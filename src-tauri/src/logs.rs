//! 앱 로그: 메모리 링 버퍼 + 파일(~/Library/Logs/<identifier>/nfd2nfc.log) + 프런트엔드 이벤트.
//!
//! 원본 nfd2nfc는 별도 데몬이라 macOS unified log(`log show`)를 거쳐야 했다.
//! 이 앱은 감시자가 같은 프로세스에 있으므로 이벤트를 UI로 바로 보낸다.
//! 앱을 다시 시작해도 기록이 남도록 파일에도 저장한다.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

const MAX_ENTRIES: usize = 5_000;
const ROTATE_BYTES: u64 = 4 * 1024 * 1024;
pub const LOG_EVENT: &str = "log-entry";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// 변환 성공 같은 핵심 이벤트
    Convert,
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Convert => "CONVERT",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "CONVERT" => Level::Convert,
            "INFO" => Level::Info,
            "WARN" => Level::Warn,
            "ERROR" => Level::Error,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// `YYYY-MM-DD HH:MM:SS`
    pub ts: String,
    pub level: Level,
    pub message: String,
}

pub struct LogStore {
    app: AppHandle,
    entries: Mutex<VecDeque<LogEntry>>,
    file: Mutex<Option<File>>,
    path: PathBuf,
}

impl LogStore {
    pub fn new(app: AppHandle, dir: PathBuf) -> Self {
        let path = dir.join("nfd2nfc.log");
        let _ = fs::create_dir_all(&dir);
        rotate_if_large(&path);
        let entries = load_tail(&path);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        Self {
            app,
            entries: Mutex::new(entries),
            file: Mutex::new(file),
            path,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn push(&self, level: Level, message: impl Into<String>) {
        let entry = LogEntry {
            ts: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            level,
            message: message.into(),
        };
        if let Ok(mut f) = self.file.lock() {
            if let Some(f) = f.as_mut() {
                // 메시지에 개행이 섞여도 한 줄 = 한 항목 형식을 유지
                let msg = entry.message.replace(['\n', '\t'], " ");
                let _ = writeln!(f, "{}\t{}\t{}", entry.ts, level.as_str(), msg);
            }
        }
        if let Ok(mut q) = self.entries.lock() {
            if q.len() >= MAX_ENTRIES {
                q.pop_front();
            }
            q.push_back(entry.clone());
        }
        let _ = self.app.emit(LOG_EVENT, &entry);
    }

    pub fn info(&self, m: impl Into<String>) {
        self.push(Level::Info, m)
    }
    pub fn warn(&self, m: impl Into<String>) {
        self.push(Level::Warn, m)
    }
    pub fn error(&self, m: impl Into<String>) {
        self.push(Level::Error, m)
    }
    pub fn convert(&self, m: impl Into<String>) {
        self.push(Level::Convert, m)
    }

    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.entries
            .lock()
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut q) = self.entries.lock() {
            q.clear();
        }
        if let Ok(mut f) = self.file.lock() {
            *f = File::create(&self.path).ok();
        }
    }
}

fn rotate_if_large(path: &Path) {
    if fs::metadata(path)
        .map(|m| m.len() > ROTATE_BYTES)
        .unwrap_or(false)
    {
        let _ = fs::rename(path, path.with_extension("log.old"));
    }
}

fn load_tail(path: &Path) -> VecDeque<LogEntry> {
    let mut q = VecDeque::with_capacity(MAX_ENTRIES);
    let Ok(file) = File::open(path) else {
        return q;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let mut parts = line.splitn(3, '\t');
        let (Some(ts), Some(level), Some(msg)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let Some(level) = Level::parse(level) else {
            continue;
        };
        if q.len() >= MAX_ENTRIES {
            q.pop_front();
        }
        q.push_back(LogEntry {
            ts: ts.to_string(),
            level,
            message: msg.to_string(),
        });
    }
    q
}
