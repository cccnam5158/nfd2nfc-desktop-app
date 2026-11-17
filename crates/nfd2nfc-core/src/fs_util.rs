use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

/// 문자열을 NFC로 정규화한다.
pub fn nfc_string(s: &str) -> String {
    s.nfc().collect()
}

/// 경로 전체를 NFC로 정규화한 "비교용 키"를 만든다.
///
/// 설정 파일에 적힌 경로와 FSEvents가 보고하는 경로는 정규화 형식이 다를 수 있다.
/// 바이트 단위의 `starts_with` 비교가 어긋나지 않도록 양쪽을 모두 NFC로 맞춘 뒤 비교한다.
pub fn nfc_path(p: &Path) -> PathBuf {
    PathBuf::from(nfc_string(&p.to_string_lossy()))
}

/// `~` 또는 `~/...`를 홈 디렉터리로 펼친다.
pub fn expand_tilde(raw: &str) -> PathBuf {
    let raw = raw.trim();
    if raw == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

/// 홈 디렉터리 접두어를 `~`로 줄여 표시용 문자열을 만든다. (항상 NFC)
pub fn abbreviate_home(p: &Path) -> String {
    let s = nfc_string(&p.to_string_lossy());
    if let Some(home) = dirs::home_dir() {
        let home = nfc_string(&home.to_string_lossy());
        if s == home {
            return "~".into();
        }
        if let Some(rest) = s.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_roundtrip() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("~/Downloads"), home.join("Downloads"));
        assert_eq!(abbreviate_home(&home.join("Downloads")), "~/Downloads");
        assert_eq!(expand_tilde("/tmp"), PathBuf::from("/tmp"));
    }
}
