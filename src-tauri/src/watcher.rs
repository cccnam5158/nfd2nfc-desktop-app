//! FSEvents(notify) 기반 폴더 감시 → NFD 이름이 보이면 즉시 NFC로 변환.
//!
//! 흐름
//! 1. notify 콜백이 이벤트를 std mpsc 채널로 보낸다.
//! 2. 워커 스레드가 50ms 동안 경로를 모은다. 이때 이름이 NFC가 아니고 감시 규칙에 해당하는 경로만 남긴다.
//! 3. 모은 경로를 얕은 경로부터 처리한다. 부모 폴더 이름부터 바뀐다.
//! 4. 같은 경로가 반복해서 실패하면 잠시 건너뛴다. HFS+ 같은 볼륨에서 무한 루프를 막기 위해서다.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use nfd2nfc_core::{
    abbreviate_home, effective_action, nfc_path, normalize_path, Action, ActiveEntry, Mode, Target,
};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

const DEBOUNCE: Duration = Duration::from_millis(50);
const FAILURE_COOLDOWN: Duration = Duration::from_secs(30);
pub const CONVERTED_EVENT: &str = "watcher-converted";

pub struct WatcherHandle {
    watcher: Option<RecommendedWatcher>,
    worker: Option<JoinHandle<()>>,
    pub watched: usize,
}

impl WatcherHandle {
    /// 감시자를 내려놓으면 채널 송신부가 drop되고, 워커는 Disconnected를 받아 종료한다.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        drop(self.watcher.take());
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn start(app: AppHandle, active: Vec<ActiveEntry>) -> Result<WatcherHandle, String> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .map_err(|e| format!("감시자를 만들 수 없습니다: {e}"))?;

    let state = app.state::<AppState>();
    let mut watched = 0;
    for entry in active.iter().filter(|e| e.action == Action::Watch) {
        let mode = match entry.mode {
            Mode::Recursive => RecursiveMode::Recursive,
            Mode::Children => RecursiveMode::NonRecursive,
        };
        match watcher.watch(&entry.canonical, mode) {
            Ok(()) => watched += 1,
            Err(e) => state.logs.error(format!(
                "감시 등록 실패: {} — {e}",
                abbreviate_home(&entry.canonical)
            )),
        }
    }

    let worker_app = app.clone();
    let worker = thread::Builder::new()
        .name("nfd2nfc-watcher".into())
        .spawn(move || worker_loop(worker_app, rx, active))
        .map_err(|e| format!("감시 스레드를 시작할 수 없습니다: {e}"))?;

    Ok(WatcherHandle {
        watcher: Some(watcher),
        worker: Some(worker),
        watched,
    })
}

fn worker_loop(app: AppHandle, rx: mpsc::Receiver<notify::Result<Event>>, active: Vec<ActiveEntry>) {
    let mut pending: HashSet<PathBuf> = HashSet::new();
    let mut failures: HashMap<PathBuf, Instant> = HashMap::new();
    let mut last_flush = Instant::now();

    loop {
        match rx.recv_timeout(DEBOUNCE) {
            Ok(Ok(event)) => {
                for path in event.paths {
                    if is_candidate(&path, &active) {
                        pending.insert(path);
                    }
                }
            }
            Ok(Err(e)) => app
                .state::<AppState>()
                .logs
                .warn(format!("감시 이벤트 오류: {e}")),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        // 이벤트가 쉬지 않고 들어와도 주기적으로 처리되도록 경과 시간 기준으로 비운다.
        if !pending.is_empty() && last_flush.elapsed() >= DEBOUNCE {
            flush(&app, &mut pending, &mut failures);
            last_flush = Instant::now();
        }
    }
}

/// 빠른 1차 필터: 이름이 NFC가 아니고, 해당 경로에 적용되는 규칙이 watch인가?
fn is_candidate(path: &std::path::Path, active: &[ActiveEntry]) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    if !Target::Nfc.needs_conversion(&name.to_string_lossy()) {
        return false;
    }
    effective_action(&nfc_path(path), active) == Some(Action::Watch)
}

fn flush(app: &AppHandle, pending: &mut HashSet<PathBuf>, failures: &mut HashMap<PathBuf, Instant>) {
    let state = app.state::<AppState>();
    let mut paths: Vec<PathBuf> = pending.drain().collect();
    paths.sort_by_key(|p| p.components().count());
    failures.retain(|_, t| t.elapsed() < FAILURE_COOLDOWN);

    for path in paths {
        let key = nfc_path(&path);
        if failures.contains_key(&key) {
            continue;
        }
        match normalize_path(&path, Target::Nfc) {
            Ok(Some(r)) => {
                let count = state.session_converted.fetch_add(1, Ordering::Relaxed) + 1;
                state.logs.convert(format!(
                    "{} {}",
                    if r.is_dir { "📁" } else { "📄" },
                    abbreviate_home(&r.to)
                ));
                let _ = app.emit(CONVERTED_EVENT, count);
            }
            Ok(None) => {}
            // 이벤트를 처리하기 전에 파일이 삭제되거나 다시 옮겨진 경우: 정상 상황
            Err(e) if e.is_not_found() => {}
            Err(e) => {
                state
                    .logs
                    .error(format!("변환 실패: {} — {e}", abbreviate_home(&path)));
                failures.insert(key, Instant::now());
            }
        }
    }
}
