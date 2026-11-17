//! Full Disk Access(전체 디스크 접근 권한) 감지.
//!
//! FDA가 있어야만 읽을 수 있는 위치를 몇 군데 열어 본다. 이런 위치는 접근해도 TCC 팝업이 뜨지 않는다.
//! - 하나라도 열리면 → 권한 있음
//! - 전부 거부되면 → 권한 없음
//! - 확인할 대상이 하나도 없으면 → 알 수 없음

use std::fs;
use std::io::ErrorKind;

const PROBES: &[&str] = &[
    "Library/Containers/com.apple.stocks",
    "Library/Safari/CloudTabs.db",
    "Library/Messages/chat.db",
    "Library/Suggestions/snippets.db",
];

pub fn check() -> Option<bool> {
    let home = dirs::home_dir()?;
    let mut denied = false;
    for probe in PROBES {
        let path = home.join(probe);
        let result = match fs::metadata(&path) {
            Ok(m) if m.is_dir() => fs::read_dir(&path).map(|_| ()),
            Ok(_) => fs::File::open(&path).map(|_| ()),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => return Some(true),
            Err(e) if e.kind() == ErrorKind::PermissionDenied => denied = true,
            Err(_) => {}
        }
    }
    denied.then_some(false)
}
