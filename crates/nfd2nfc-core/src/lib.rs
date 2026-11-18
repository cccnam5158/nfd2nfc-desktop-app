//! nfd2nfc-core
//!
//! macOS가 파일명을 NFD(자소 분해형)로 저장하는 탓에 Windows·SMB·zip으로 넘어갈 때
//! 한글 파일명이 `ㅎㅏㄴㄱㅡㄹ`처럼 깨지는 문제를 해결하기 위한 핵심 로직.
//!
//! 설계와 일부 기법(F_GETPATH로 실제 디스크 이름 조회, watch/ignore 규칙 평가)은
//! MIT 라이선스인 [elgar328/nfd2nfc](https://github.com/elgar328/nfd2nfc)를 참고했다.

pub mod config;
pub mod fs_util;
pub mod normalizer;
pub mod scan;
pub mod zipper;

pub use scan::{scan_forms, FormSample, FormScan};

pub use config::{
    effective_action, evaluate, Action, ActiveEntry, Config, ConfigError, EntryStatus, Mode,
    PathEntry, StatusKind,
};
pub use fs_util::{abbreviate_home, expand_tilde, nfc_path, nfc_string};
pub use normalizer::{
    actual_file_name, detect_form, normalize, normalize_path, EntryError, NormalizeError, Rename,
    Report, Scope, Target, UnicodeForm,
};
pub use zipper::{create_nfc_zip, default_zip_path, is_junk_name, ZipError, ZipOptions, ZipReport};
