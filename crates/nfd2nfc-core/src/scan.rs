//! 하위 트리의 파일명 정규화 상태 조회 (읽기 전용).
//!
//! `normalize(.., dry_run)`은 "무엇을 바꿀지"를 계산한다. 이 모듈은 "지금 어떤 상태인지"를
//! 형태별 개수와 NFD 항목 표본으로 집계한다. 홈 폴더처럼 거대한 트리를 고를 수도 있으므로
//! 검사 항목 수에 상한을 둔다.

use std::path::{Path, PathBuf};

use serde::Serialize;
use walkdir::WalkDir;

use crate::fs_util::nfc_string;
use crate::normalizer::{detect_form, UnicodeForm};
use crate::zipper::is_junk_name;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormSample {
    pub path: PathBuf,
    /// 검사 시작 폴더 기준 상대 경로 (표시용, NFC)
    pub relative: String,
    pub is_dir: bool,
    pub form: UnicodeForm,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormScan {
    pub scanned: usize,
    pub ascii: usize,
    pub nfc: usize,
    pub nfd: usize,
    pub mixed: usize,
    /// `max_entries`에 도달해 검사를 중단했는지
    pub truncated: bool,
    /// NFD·혼합 항목 표본 (최대 `max_samples`개)
    pub samples: Vec<FormSample>,
}

impl FormScan {
    /// 변환이 필요한 항목 수
    pub fn needs_conversion(&self) -> usize {
        self.nfd + self.mixed
    }
}

/// `root` 아래(자신 제외)를 훑어 형태별로 센다.
/// 심볼릭 링크는 따라가지 않고 다른 볼륨으로 넘어가지 않는다. `.DS_Store` 같은 부산물은 제외한다.
pub fn scan_forms(root: &Path, max_entries: usize, max_samples: usize) -> FormScan {
    let mut scan = FormScan::default();
    let walker = WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .same_file_system(true)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !is_junk_name(&e.file_name().to_string_lossy()));

    for entry in walker.filter_map(Result::ok) {
        if scan.scanned >= max_entries {
            scan.truncated = true;
            break;
        }
        scan.scanned += 1;
        let name = entry.file_name().to_string_lossy();
        let form = detect_form(&name);
        match form {
            UnicodeForm::Ascii => scan.ascii += 1,
            UnicodeForm::Nfc => scan.nfc += 1,
            UnicodeForm::Nfd => scan.nfd += 1,
            UnicodeForm::Mixed => scan.mixed += 1,
        }
        if matches!(form, UnicodeForm::Nfd | UnicodeForm::Mixed) && scan.samples.len() < max_samples
        {
            let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
            scan.samples.push(FormSample {
                path: entry.path().to_path_buf(),
                relative: nfc_string(&rel.to_string_lossy()),
                is_dir: entry.file_type().is_dir(),
                form,
            });
        }
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use unicode_normalization::UnicodeNormalization;

    #[test]
    fn counts_forms_and_caps() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let sub = root.join("폴더".nfd().collect::<String>());
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("문서.txt".nfd().collect::<String>()), b"").unwrap();
        fs::write(root.join("정상.txt"), b"").unwrap();
        fs::write(root.join("a.txt"), b"").unwrap();
        fs::write(root.join(".DS_Store"), b"").unwrap();

        let s = scan_forms(root, 100, 10);
        assert_eq!((s.scanned, s.ascii, s.nfc, s.nfd, s.mixed), (4, 1, 1, 2, 0));
        assert!(!s.truncated);
        assert_eq!(s.needs_conversion(), 2);
        let mut rels: Vec<_> = s.samples.iter().map(|x| x.relative.clone()).collect();
        rels.sort();
        assert_eq!(rels, vec!["폴더", "폴더/문서.txt"]);

        let capped = scan_forms(root, 2, 1);
        assert!(capped.truncated);
        assert_eq!(capped.scanned, 2);
        assert!(capped.samples.len() <= 1);
    }
}
