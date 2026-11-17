use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;

use nfd2nfc_core::{evaluate, Config};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::logs::LogStore;
use crate::tray::TrayHandles;
use crate::watcher::{self, WatcherHandle};

pub const STATUS_EVENT: &str = "watcher-status";

pub struct AppState {
    pub config_path: PathBuf,
    pub config: Mutex<Config>,
    pub watcher: Mutex<Option<WatcherHandle>>,
    pub logs: LogStore,
    pub session_converted: AtomicU64,
    pub tray: Mutex<Option<TrayHandles>>,
}

impl AppState {
    pub fn new(app: &AppHandle) -> tauri::Result<Self> {
        let config_path = app.path().app_config_dir()?.join("config.toml");
        let log_dir = app.path().app_log_dir()?;
        let logs = LogStore::new(app.clone(), log_dir);
        let config = match Config::load(&config_path) {
            Ok(c) => c,
            Err(e) => {
                logs.error(format!("{e} — 기본 설정으로 시작합니다"));
                Config::default()
            }
        };
        Ok(Self {
            config_path,
            config: Mutex::new(config),
            watcher: Mutex::new(None),
            logs,
            session_converted: AtomicU64::new(0),
            tray: Mutex::new(None),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherStatus {
    pub running: bool,
    /// 실제로 FSEvents에 등록된 감시 폴더 수
    pub watched: usize,
}

pub fn watcher_status(app: &AppHandle) -> WatcherStatus {
    let state = app.state::<AppState>();
    let guard = state.watcher.lock().expect("watcher lock");
    WatcherStatus {
        running: guard.is_some(),
        watched: guard.as_ref().map(|w| w.watched).unwrap_or(0),
    }
}

/// 감시 상태를 바꾸고(필요하면 재시작), 설정·트레이·UI에 반영한다.
///
/// `remember`가 true면 다음 실행 때도 이 상태로 시작하도록 설정에 저장한다.
pub fn set_watching(app: &AppHandle, enabled: bool, remember: bool) -> Result<WatcherStatus, String> {
    let state = app.state::<AppState>();
    {
        let mut guard = state.watcher.lock().map_err(|e| e.to_string())?;
        if let Some(old) = guard.take() {
            old.stop();
        }
        if enabled {
            let paths = state.config.lock().map_err(|e| e.to_string())?.paths.clone();
            let (_, active) = evaluate(&paths);
            let handle = watcher::start(app.clone(), active)?;
            if handle.watched == 0 {
                state
                    .logs
                    .warn("감시할 폴더가 없습니다. ‘감시 폴더’ 탭에서 폴더를 추가하세요.");
            } else {
                state
                    .logs
                    .info(format!("감시 시작 — 폴더 {}개", handle.watched));
            }
            *guard = Some(handle);
        } else {
            state.logs.info("감시 중지");
        }
    }

    if remember {
        let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
        if cfg.watcher_enabled != enabled {
            cfg.watcher_enabled = enabled;
            cfg.save(&state.config_path).map_err(|e| e.to_string())?;
        }
    }

    let status = watcher_status(app);
    crate::tray::refresh(app, &status);
    let _ = app.emit(STATUS_EVENT, &status);
    Ok(status)
}

/// 앱 종료 시 감시 스레드를 정리한다.
pub fn shutdown(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let handle = match state.watcher.lock() {
        Ok(mut guard) => guard.take(),
        Err(_) => None,
    };
    if let Some(h) = handle {
        h.stop();
    }
}

/// 감시 중일 때만 새 설정으로 다시 시작한다.
pub fn restart_if_running(app: &AppHandle) -> Result<WatcherStatus, String> {
    if watcher_status(app).running {
        set_watching(app, true, false)
    } else {
        Ok(watcher_status(app))
    }
}
