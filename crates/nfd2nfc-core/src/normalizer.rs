use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use unicode_normalization::{is_nfc, is_nfd, UnicodeNormalization};

use crate::fs_util::nfc_string;

/// 변환 방향
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    /// NFD → NFC (Windows 호환, 기본값)
    Nfc,
    /// NFC → NFD (역변환)
    Nfd,
}

impl Target {
    pub fn convert(self, name: &str) -> String {
        match self {
            Target::Nfc => name.nfc().collect(),
            Target::Nfd => name.nfd().collect(),
        }
    }

    pub fn needs_conversion(self, name: &str) -> bool {
        match self {
            Target::Nfc => !is_nfc(name),
            Target::Nfd => !is_nfd(name),
        }
    }
}

/// 파일명의 유니코드 정규화 형태 (UI 배지용)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UnicodeForm {
    /// ASCII만 포함 — 정규화와 무관
    Ascii,
    Nfc,
    Nfd,
    /// 조합형과 분해형이 섞여 있음
    Mixed,
}

pub fn detect_form(name: &str) -> UnicodeForm {
    if name.is_ascii() {
        return UnicodeForm::Ascii;
    }
    match (is_nfc(name), is_nfd(name)) {
        (true, false) => UnicodeForm::Nfc,
        (false, true) => UnicodeForm::Nfd,
        // 한자처럼 두 형태가 동일한 문자열은 NFC로 취급 (변환 불필요)
        (true, true) => UnicodeForm::Nfc,
        (false, false) => UnicodeForm::Mixed,
    }
}

/// 변환 범위
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// 선택한 항목의 이름만
    Name,
    /// 폴더 바로 아래 항목만 (폴더 자신은 제외)
    Children,
    /// 자신 + 모든 하위 항목
    Recursive,
}

#[derive(Debug, thiserror::Error)]
pub enum NormalizeError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("이름 충돌: 같은 이름의 다른 항목이 이미 있습니다 — {0}")]
    Conflict(String),
    #[error("이 볼륨은 NFC 파일명을 유지하지 않습니다 (HFS+ 등)")]
    Unsupported,
    #[error("UTF-8이 아닌 파일명은 건너뜁니다")]
    NonUtf8,
}

impl NormalizeError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, NormalizeError::Io(e) if e.kind() == io::ErrorKind::NotFound)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rename {
    pub from: PathBuf,
    pub to: PathBuf,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct Report {
    pub renamed: Vec<Rename>,
    pub errors: Vec<EntryError>,
    /// 검사한 항목 수
    pub scanned: usize,
}

impl Report {
    pub fn merge(&mut self, other: Report) {
        self.renamed.extend(other.renamed);
        self.errors.extend(other.errors);
        self.scanned += other.scanned;
    }

    fn error(&mut self, path: &Path, err: impl ToString) {
        self.errors.push(EntryError {
            path: path.to_path_buf(),
            message: err.to_string(),
        });
    }
}

/// `fcntl(F_GETPATH)`로 열린 파일의 실제 디스크 경로를 얻는다.
///
/// 이벤트·사용자 입력으로 받은 경로 문자열은 디스크에 저장된 정규화 형태와 다를 수 있다.
/// APFS는 정규화를 구분하지 않으므로 어떤 형태로 열어도 같은 파일이 열린다.
/// 따라서 열린 fd에 커널이 기억하는 이름을 물어보면 디스크에 저장된 형태를 정확히 알 수 있다.
#[cfg(target_os = "macos")]
fn fcntl_path(path: &Path) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;

    // O_SYMLINK: 심볼릭 링크를 따라가지 않고 링크 자체를 연다.
    // O_NONBLOCK: FIFO 등을 열 때 블로킹되지 않게 한다.
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_SYMLINK | libc::O_NONBLOCK)
        .open(path)?;
    let mut buf = vec![0u8; libc::PATH_MAX as usize + 1];
    // SAFETY: buf는 MAXPATHLEN 이상 크기이며 fd는 file이 살아있는 동안 유효하다.
    let ret = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) };
    if ret == -1 {
        return Err(io::Error::last_os_error());
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf.truncate(len);
    Ok(PathBuf::from(OsString::from_vec(buf)))
}

/// 디스크에 실제로 저장된 파일명(정규화 형태 그대로)을 반환한다.
pub fn actual_file_name(path: &Path) -> io::Result<OsString> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "파일명이 없는 경로"))?;
    let want = nfc_string(&name.to_string_lossy());

    // 1) O(1): F_GETPATH
    if let Some(n) = fast_actual_name(path, &want) {
        return Ok(n);
    }

    // 2) 폴백: 부모 디렉터리를 훑어 NFC 기준으로 같은 이름을 찾는다. (읽기 권한 없는 파일 등)
    let parent = parent_or_cwd(path);
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let n = entry.file_name();
        if nfc_string(&n.to_string_lossy()) == want {
            return Ok(n);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "파일을 찾을 수 없습니다",
    ))
}

/// 하드 링크는 F_GETPATH가 다른 링크의 이름을 돌려줄 수 있으므로 NFC 비교로 검증한다.
#[cfg(target_os = "macos")]
fn fast_actual_name(path: &Path, want: &str) -> Option<OsString> {
    let real = fcntl_path(path).ok()?;
    let real_name = real.file_name()?;
    (nfc_string(&real_name.to_string_lossy()) == want).then(|| real_name.to_os_string())
}

#[cfg(not(target_os = "macos"))]
fn fast_actual_name(_path: &Path, _want: &str) -> Option<OsString> {
    None
}

fn parent_or_cwd(path: &Path) -> &Path {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    }
}

/// `to`가 이미 존재하는데 `from`과 다른 파일(inode)이면 충돌로 본다.
///
/// APFS에서는 NFD 이름과 NFC 이름이 같은 파일을 가리키므로 충돌이 아니다.
/// 반면 정규화를 구분하는 볼륨에서는 두 이름이 서로 다른 파일일 수 있다.
/// 이런 경우 그대로 `rename`하면 기존 파일을 덮어쓰게 되므로 반드시 막아야 한다.
fn check_collision(from: &Path, to: &Path) -> Result<(), NormalizeError> {
    match fs::symlink_metadata(to) {
        Ok(tm) => {
            let fm = fs::symlink_metadata(from)?;
            if (tm.dev(), tm.ino()) != (fm.dev(), fm.ino()) {
                return Err(NormalizeError::Conflict(
                    to.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ));
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// `parent` 안의 `actual_name` 항목을 `target` 형태로 바꾼다.
/// 바꿀 필요가 없으면 `Ok(None)`.
fn normalize_entry(
    parent: &Path,
    actual_name: &OsStr,
    target: Target,
    dry_run: bool,
) -> Result<Option<Rename>, NormalizeError> {
    let Some(name) = actual_name.to_str() else {
        return Err(NormalizeError::NonUtf8);
    };
    if !target.needs_conversion(name) {
        return Ok(None);
    }
    let new_name = target.convert(name);
    let from = parent.join(actual_name);
    let to = parent.join(&new_name);
    let is_dir = fs::symlink_metadata(&from)?.is_dir();

    if !dry_run {
        check_collision(&from, &to)?;
        fs::rename(&from, &to)?;
        // HFS+처럼 이름을 강제로 NFD로 저장하는 볼륨이면 rename이 효과가 없다.
        // 감시자가 같은 파일을 무한 반복 처리하지 않도록 오류로 알린다.
        if let Ok(after) = actual_file_name(&to) {
            if target.needs_conversion(&after.to_string_lossy()) {
                return Err(NormalizeError::Unsupported);
            }
        }
    }
    Ok(Some(Rename { from, to, is_dir }))
}

/// 단일 경로의 이름(마지막 구성 요소)만 변환한다.
pub fn normalize_path(path: &Path, target: Target) -> Result<Option<Rename>, NormalizeError> {
    normalize_path_inner(path, target, false)
}

fn normalize_path_inner(
    path: &Path,
    target: Target,
    dry_run: bool,
) -> Result<Option<Rename>, NormalizeError> {
    let actual = actual_file_name(path)?;
    normalize_entry(parent_or_cwd(path), &actual, target, dry_run)
}

/// 경로를 `scope` 범위만큼 변환한다. 개별 항목의 오류는 모아서 보고하고 계속 진행한다.
///
/// - 심볼릭 링크 폴더 안으로는 들어가지 않는다.
/// - 다른 파일 시스템(마운트된 볼륨) 경계는 넘지 않는다.
/// - `dry_run`이면 실제로 바꾸지 않고 바뀔 목록만 만든다.
pub fn normalize(path: &Path, scope: Scope, target: Target, dry_run: bool) -> Report {
    let mut report = Report::default();
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            report.error(path, e);
            return report;
        }
    };
    let is_real_dir = meta.is_dir();

    let mut root = path.to_path_buf();
    if !is_real_dir || matches!(scope, Scope::Name | Scope::Recursive) {
        report.scanned += 1;
        match normalize_path_inner(path, target, dry_run) {
            Ok(Some(r)) => {
                if !dry_run {
                    root = r.to.clone();
                }
                report.renamed.push(r);
            }
            Ok(None) => {}
            Err(e) => report.error(path, e),
        }
    }
    if !is_real_dir || scope == Scope::Name {
        return report;
    }

    let root_dev = meta.dev();
    let recursive = scope == Scope::Recursive;
    let mut queue = VecDeque::from([root]);

    while let Some(dir) = queue.pop_front() {
        // 순회 중에 이름을 바꾸면 readdir 결과가 흔들릴 수 있어 먼저 목록을 고정한다.
        let entries: Vec<_> = match fs::read_dir(&dir) {
            Ok(rd) => rd.filter_map(Result::ok).collect(),
            Err(e) => {
                report.error(&dir, e);
                continue;
            }
        };
        for entry in entries {
            report.scanned += 1;
            let name = entry.file_name();
            let mut child = dir.join(&name);
            match normalize_entry(&dir, &name, target, dry_run) {
                Ok(Some(r)) => {
                    if !dry_run {
                        child = r.to.clone();
                    }
                    report.renamed.push(r);
                }
                Ok(None) => {}
                Err(e) => report.error(&child, e),
            }
            if !recursive {
                continue;
            }
            // DirEntry::file_type()은 심볼릭 링크를 따라가지 않는다.
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                let same_fs = fs::symlink_metadata(&child)
                    .map(|m| m.dev() == root_dev)
                    .unwrap_or(false);
                if same_fs {
                    queue.push_back(child);
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const NFC: &str = "한글파일.txt";

    fn nfd(s: &str) -> String {
        s.nfd().collect()
    }

    #[test]
    fn detects_forms() {
        assert_eq!(detect_form("hello.txt"), UnicodeForm::Ascii);
        assert_eq!(detect_form(NFC), UnicodeForm::Nfc);
        assert_eq!(detect_form(&nfd(NFC)), UnicodeForm::Nfd);
        let mixed = format!("{}{}", "한", nfd("글"));
        assert_eq!(detect_form(&mixed), UnicodeForm::Mixed);
        assert_eq!(detect_form("漢字"), UnicodeForm::Nfc);
    }

    #[test]
    fn converts_both_ways() {
        assert_ne!(nfd(NFC), NFC);
        assert!(Target::Nfc.needs_conversion(&nfd(NFC)));
        assert_eq!(Target::Nfc.convert(&nfd(NFC)), NFC);
        assert!(!Target::Nfc.needs_conversion(NFC));
        assert_eq!(Target::Nfd.convert(NFC), nfd(NFC));
    }

    fn disk_name(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn normalizes_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(nfd(NFC));
        fs::write(&p, b"x").unwrap();

        let r = normalize_path(&p, Target::Nfc).unwrap().expect("renamed");
        assert!(!r.is_dir);
        assert_eq!(disk_name(tmp.path()), vec![NFC.to_string()]);
        // 두 번째는 할 일 없음
        assert!(normalize_path(&tmp.path().join(NFC), Target::Nfc)
            .unwrap()
            .is_none());
    }

    #[test]
    fn normalizes_recursively_and_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().join(nfd("폴더"));
        let sub = top.join(nfd("하위"));
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join(nfd("문서.txt")), b"x").unwrap();
        fs::write(top.join("plain.txt"), b"x").unwrap();

        let dry = normalize(&top, Scope::Recursive, Target::Nfc, true);
        assert_eq!(dry.renamed.len(), 3, "{dry:?}");
        assert!(dry.errors.is_empty());
        assert_eq!(
            disk_name(tmp.path()),
            vec![nfd("폴더")],
            "dry run must not touch disk"
        );

        let children = normalize(&top, Scope::Children, Target::Nfc, false);
        assert_eq!(children.renamed.len(), 1); // 하위 폴더 이름만

        let rep = normalize(&top, Scope::Recursive, Target::Nfc, false);
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
        assert_eq!(rep.renamed.len(), 2); // 폴더 자신 + 문서.txt
        assert_eq!(disk_name(tmp.path()), vec!["폴더".to_string()]);
        assert_eq!(
            disk_name(&tmp.path().join("폴더/하위")),
            vec!["문서.txt".to_string()]
        );
    }
}
