//! Windows에서 압축을 풀어도 한글 파일명이 깨지지 않는 zip 생성.
//!
//! Finder의 "압축하기"는 파일명을 NFD 그대로 넣는다. 그래서 Windows 탐색기에서 풀면
//! 자소가 분리된 이름이 나온다. 여기서는 다음 세 가지를 처리한다.
//! 1. 모든 경로 구성 요소를 NFC로 정규화한다.
//! 2. UTF-8 플래그(general purpose bit 11)를 설정한다. zip 크레이트가 ASCII가 아닌 이름에 자동으로 설정한다.
//! 3. `.DS_Store`, `._*`, `__MACOSX` 같은 macOS 전용 부산물을 제외한다.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufWriter};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, Timelike};
use serde::Serialize;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::fs_util::nfc_string;

#[derive(Debug, Clone, Default)]
pub struct ZipOptions {
    /// 숨김 파일(`.`으로 시작)도 포함할지 여부. `.DS_Store` 등은 항상 제외된다.
    pub include_hidden: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipReport {
    pub output: PathBuf,
    pub files: usize,
    pub dirs: usize,
    /// NFD → NFC로 바뀌어 들어간 항목 수
    pub normalized_names: usize,
    /// 제외된 항목 수 (macOS 부산물, 심볼릭 링크, 숨김 파일 등)
    pub skipped: usize,
    /// 원본 파일 크기 합계
    pub bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ZipError {
    #[error("압축할 항목이 없습니다")]
    Empty,
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("zip 생성 실패: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("폴더 탐색 실패: {0}")]
    Walk(#[from] walkdir::Error),
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ZipError + '_ {
    move |source| ZipError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// 항상 제외하는 macOS 전용 부산물
pub fn is_junk_name(name: &str) -> bool {
    name == ".DS_Store"
        || name == "__MACOSX"
        || name == "Icon\r"
        || name == ".localized"
        || name.starts_with("._")
}

fn should_skip(name: &str, opts: &ZipOptions) -> bool {
    is_junk_name(name) || (!opts.include_hidden && name.starts_with('.'))
}

fn zip_time(t: SystemTime) -> zip::DateTime {
    let local: DateTime<Local> = t.into();
    zip::DateTime::from_date_and_time(
        local.year().clamp(1980, 2107) as u16,
        local.month() as u8,
        local.day() as u8,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    )
    .unwrap_or_default()
}

/// 같은 NFC 이름이 두 번 나오면 (예: 정규화를 구분하는 볼륨) ` (2)`를 붙여 피한다.
fn unique_name(name: String, used: &mut HashSet<String>) -> String {
    if used.insert(name.clone()) {
        return name;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && !name[..i].ends_with('/') => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    let (stem, slash) = match stem.strip_suffix('/') {
        Some(s) => (s, "/"),
        None => (stem, ""),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){ext}{slash}"))
        .find(|candidate| used.insert(candidate.clone()))
        .expect("infinite iterator")
}

/// `inputs`(파일 또는 폴더 여러 개)를 `output` zip으로 만든다.
/// 작업 중에는 `.part` 임시 파일에 쓰고, 성공하면 최종 이름으로 바꾼다.
pub fn create_nfc_zip(
    inputs: &[PathBuf],
    output: &Path,
    opts: &ZipOptions,
) -> Result<ZipReport, ZipError> {
    if inputs.is_empty() {
        return Err(ZipError::Empty);
    }
    let part = output.with_extension("zip.part");
    let result = write_zip(inputs, output, &part, opts);
    match result {
        Ok(report) => {
            fs::rename(&part, output).map_err(io_err(output))?;
            Ok(report)
        }
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

fn write_zip(
    inputs: &[PathBuf],
    output: &Path,
    part: &Path,
    opts: &ZipOptions,
) -> Result<ZipReport, ZipError> {
    let file = File::create(part).map_err(io_err(part))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let mut used = HashSet::new();
    let mut report = ZipReport {
        output: output.to_path_buf(),
        files: 0,
        dirs: 0,
        normalized_names: 0,
        skipped: 0,
        bytes: 0,
    };
    // 출력 파일 자신을 다시 압축하지 않도록
    let output_nfc = nfc_string(&output.to_string_lossy());

    for input in inputs {
        let base = input.parent().unwrap_or(Path::new("/"));
        let walker = WalkDir::new(input)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| {
                // 최상위 항목은 사용자가 명시적으로 고른 것이므로 숨김 여부와 관계없이 포함
                e.depth() == 0 || !should_skip(&e.file_name().to_string_lossy(), opts)
            });

        for entry in walker {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type();
            if ft.is_symlink() || !(ft.is_file() || ft.is_dir()) {
                report.skipped += 1;
                continue;
            }
            let raw_rel = path.strip_prefix(base).unwrap_or(path);
            let raw_rel = raw_rel.to_string_lossy();
            if nfc_string(&path.to_string_lossy()) == output_nfc
                || nfc_string(&path.to_string_lossy()) == nfc_string(&part.to_string_lossy())
            {
                continue;
            }
            let rel = nfc_string(&raw_rel);
            if rel != raw_rel {
                report.normalized_names += 1;
            }
            let meta = entry.metadata().map_err(|e| ZipError::Io {
                path: path.to_path_buf(),
                source: e.into(),
            })?;
            let mut options = SimpleFileOptions::default()
                .unix_permissions(meta.permissions().mode() & 0o777)
                .last_modified_time(zip_time(meta.modified().unwrap_or(SystemTime::now())));

            if ft.is_dir() {
                let name = unique_name(format!("{rel}/"), &mut used);
                zip.add_directory(name, options)?;
                report.dirs += 1;
            } else {
                options = options
                    .compression_method(CompressionMethod::Deflated)
                    .large_file(meta.len() >= u32::MAX as u64);
                let name = unique_name(rel, &mut used);
                zip.start_file(name, options)?;
                let mut src = File::open(path).map_err(io_err(path))?;
                report.bytes += io::copy(&mut src, &mut zip).map_err(io_err(path))?;
                report.files += 1;
            }
        }
    }
    let mut writer = zip.finish()?;
    io::Write::flush(&mut writer).map_err(io_err(part))?;
    Ok(report)
}

/// 기본 출력 경로: 첫 항목과 같은 폴더에 `<이름>.zip` (여러 개면 `아카이브.zip`).
/// 이미 있으면 ` 2`, ` 3`…을 붙인다.
pub fn default_zip_path(inputs: &[PathBuf]) -> Option<PathBuf> {
    let first = inputs.first()?;
    let dir = first.parent()?.to_path_buf();
    let stem = if inputs.len() == 1 {
        let name = first.file_name()?.to_string_lossy();
        let name = nfc_string(&name);
        if first.is_dir() {
            name
        } else {
            Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(name)
        }
    } else {
        "아카이브".to_string()
    };
    let candidate = dir.join(format!("{stem}.zip"));
    if !candidate.exists() {
        return Some(candidate);
    }
    (2..10_000)
        .map(|n| dir.join(format!("{stem} {n}.zip")))
        .find(|p| !p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_normalization::UnicodeNormalization;

    #[test]
    fn zip_names_are_nfc_and_junk_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("사진모음".nfd().collect::<String>());
        fs::create_dir_all(folder.join("하위".nfd().collect::<String>())).unwrap();
        fs::write(
            folder
                .join("하위".nfd().collect::<String>())
                .join("가족.jpg".nfd().collect::<String>()),
            b"jpeg",
        )
        .unwrap();
        fs::write(folder.join(".DS_Store"), b"junk").unwrap();
        fs::write(folder.join("._가족.jpg"), b"junk").unwrap();
        fs::write(folder.join(".hidden"), b"h").unwrap();

        let inputs = vec![folder.clone()];
        let out = default_zip_path(&inputs).unwrap();
        assert_eq!(out.file_name().unwrap().to_string_lossy(), "사진모음.zip");

        let rep = create_nfc_zip(&inputs, &out, &ZipOptions::default()).unwrap();
        assert_eq!(rep.files, 1);
        assert_eq!(rep.dirs, 2);
        assert_eq!(rep.normalized_names, 3);
        assert!(rep.skipped == 0);

        let mut ar = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        let mut names: Vec<String> = (0..ar.len())
            .map(|i| ar.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["사진모음/", "사진모음/하위/", "사진모음/하위/가족.jpg"]
        );
        assert!(!out.with_extension("zip.part").exists());

        // 같은 이름이 이미 있으면 번호가 붙는다
        let next = default_zip_path(&inputs).unwrap();
        assert_eq!(
            next.file_name().unwrap().to_string_lossy(),
            "사진모음 2.zip"
        );
    }

    #[test]
    fn unique_names() {
        let mut used = HashSet::new();
        assert_eq!(unique_name("a.txt".into(), &mut used), "a.txt");
        assert_eq!(unique_name("a.txt".into(), &mut used), "a (2).txt");
        assert_eq!(unique_name("d/".into(), &mut used), "d/");
        assert_eq!(unique_name("d/".into(), &mut used), "d (2)/");
    }
}
