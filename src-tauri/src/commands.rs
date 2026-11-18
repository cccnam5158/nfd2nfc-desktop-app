//! 프런트엔드에서 `invoke()`로 호출하는 커맨드.
//!
//! Tauri 2에서 동기 커맨드는 메인 스레드에서 실행된다. 파일 시스템을 오래 다루는 작업은
//! `async` + `spawn_blocking`으로 돌려야 UI가 멈추지 않는다.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;

use nfd2nfc_core::{
    abbreviate_home, create_nfc_zip, default_zip_path, detect_form, evaluate, expand_tilde,
    is_junk_name, nfc_string, normalize, scan_forms, EntryStatus, FormScan, PathEntry, Report,
    Scope, Target, UnicodeForm, ZipOptions, ZipReport,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::logs::LogEntry;
use crate::state::{self, AppState, WatcherStatus};

type CmdResult<T> = Result<T, String>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

// ───────────────────────── 상태 ─────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    watcher: WatcherStatus,
    full_disk_access: Option<bool>,
    session_converted: u64,
    version: String,
    config_path: String,
    log_path: String,
}

#[tauri::command]
pub async fn get_status(app: AppHandle) -> CmdResult<AppStatus> {
    let state = app.state::<AppState>();
    Ok(AppStatus {
        watcher: state::watcher_status(&app),
        full_disk_access: crate::fda::check(),
        session_converted: state.session_converted.load(Ordering::Relaxed),
        version: app.package_info().version.to_string(),
        config_path: abbreviate_home(&state.config_path),
        log_path: abbreviate_home(state.logs.path()),
    })
}

#[tauri::command]
pub async fn set_watching(app: AppHandle, enabled: bool) -> CmdResult<WatcherStatus> {
    state::set_watching(&app, enabled, true)
}

#[tauri::command]
pub async fn restart_watcher(app: AppHandle) -> CmdResult<WatcherStatus> {
    state::set_watching(&app, true, true)
}

// ───────────────────────── 감시 폴더 설정 ─────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    #[serde(flatten)]
    entry: PathEntry,
    /// `~` 축약 + NFC 표시 경로
    display: String,
    status: EntryStatus,
}

fn entry_views(entries: Vec<PathEntry>) -> Vec<EntryView> {
    let (statuses, _) = evaluate(&entries);
    entries
        .into_iter()
        .zip(statuses)
        .map(|(entry, status)| EntryView {
            display: abbreviate_home(&expand_tilde(&entry.path)),
            entry,
            status,
        })
        .collect()
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> CmdResult<Vec<EntryView>> {
    let paths = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .paths
        .clone();
    blocking(move || entry_views(paths)).await
}

/// 저장하지 않은 편집 상태의 규칙들을 평가한다. (UI 미리보기용)
#[tauri::command]
pub async fn evaluate_entries(entries: Vec<PathEntry>) -> CmdResult<Vec<EntryView>> {
    blocking(move || entry_views(entries)).await
}

#[tauri::command]
pub async fn save_config(app: AppHandle, entries: Vec<PathEntry>) -> CmdResult<Vec<EntryView>> {
    let state = app.state::<AppState>();
    {
        let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
        let mut next = cfg.clone();
        // 홈 경로는 `~`로 저장해 사람이 읽기 쉽게 한다.
        next.paths = entries
            .iter()
            .map(|e| PathEntry {
                path: abbreviate_home(&expand_tilde(&e.path)),
                ..e.clone()
            })
            .collect();
        next.save(&state.config_path).map_err(|e| e.to_string())?;
        *cfg = next;
    }
    state
        .logs
        .info(format!("설정 저장 — 규칙 {}개", entries.len()));
    state::restart_if_running(&app)?;
    let paths = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .paths
        .clone();
    blocking(move || entry_views(paths)).await
}

/// 처음 실행할 때 추천하는 폴더 (존재하는 것만)
#[tauri::command]
pub fn suggested_folders() -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return vec![];
    };
    ["Desktop", "Downloads", "Documents"]
        .iter()
        .map(|d| home.join(d))
        .filter(|p| p.is_dir())
        .map(|p| abbreviate_home(&p))
        .collect()
}

// ───────────────────────── 탐색기 ─────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirItem {
    /// 표시용 이름 (NFC)
    name: String,
    /// 디스크에 저장된 그대로의 이름 — 코드 포인트 분석용
    raw_name: String,
    path: String,
    is_dir: bool,
    is_symlink: bool,
    form: UnicodeForm,
    /// 파일 크기 (폴더는 None)
    size: Option<u64>,
    /// 수정 시각 (Unix 초)
    modified: Option<i64>,
}

#[derive(Serialize)]
pub struct Crumb {
    name: String,
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    path: String,
    display: String,
    parent: Option<String>,
    /// 경로 조각 (홈 아래면 `~`부터, 아니면 `/`부터)
    crumbs: Vec<Crumb>,
    /// 현재 폴더 자신의 이름 형태와 원본 이름
    form: UnicodeForm,
    raw_name: String,
    items: Vec<DirItem>,
}

fn crumbs_for(dir: &Path) -> Vec<Crumb> {
    let home = dirs::home_dir();
    let mut crumbs = Vec::new();
    let mut cur = Some(dir);
    while let Some(p) = cur {
        let is_home = home.as_deref() == Some(p);
        let name = if is_home {
            "~".to_string()
        } else {
            match p.file_name() {
                Some(n) => nfc_string(&n.to_string_lossy()),
                None => "/".to_string(),
            }
        };
        crumbs.push(Crumb {
            name,
            path: p.to_string_lossy().into_owned(),
        });
        if is_home {
            break;
        }
        cur = p.parent();
    }
    crumbs.reverse();
    crumbs
}

fn unix_secs(t: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    let d = t.ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(d.as_secs() as i64)
}

#[tauri::command]
pub async fn list_dir(path: Option<String>, show_hidden: bool) -> CmdResult<DirListing> {
    blocking(move || {
        let dir = match path {
            Some(p) if !p.is_empty() => expand_tilde(&p),
            _ => dirs::home_dir().ok_or("홈 폴더를 찾을 수 없습니다")?,
        };
        let rd = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", abbreviate_home(&dir)))?;
        let mut items: Vec<DirItem> = rd
            .filter_map(Result::ok)
            .filter_map(|e| {
                let raw = e.file_name().to_string_lossy().into_owned();
                if is_junk_name(&raw) || (!show_hidden && raw.starts_with('.')) {
                    return None;
                }
                let is_symlink = e.file_type().map(|t| t.is_symlink()).unwrap_or(false);
                // 심볼릭 링크 폴더도 탐색할 수 있도록 여기서는 링크를 따라간다.
                let meta = fs::metadata(e.path()).ok();
                let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                Some(DirItem {
                    form: detect_form(&raw),
                    name: nfc_string(&raw),
                    path: e.path().to_string_lossy().into_owned(),
                    size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
                    modified: meta.as_ref().and_then(|m| unix_secs(m.modified())),
                    raw_name: raw,
                    is_dir,
                    is_symlink,
                })
            })
            .collect();
        items.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        let own_name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(DirListing {
            display: abbreviate_home(&dir),
            parent: dir.parent().map(|p| p.to_string_lossy().into_owned()),
            crumbs: crumbs_for(&dir),
            form: detect_form(&own_name),
            raw_name: own_name,
            path: dir.to_string_lossy().into_owned(),
            items,
        })
    })
    .await?
}

/// 탐색기 상세 패널: 폴더 아래 전체의 NFC/NFD 분포와 NFD 항목 표본
#[tauri::command]
pub async fn scan_tree(path: String) -> CmdResult<FormScan> {
    const MAX_ENTRIES: usize = 200_000;
    const MAX_SAMPLES: usize = 500;
    blocking(move || scan_forms(Path::new(&path), MAX_ENTRIES, MAX_SAMPLES)).await
}

// ───────────────────────── 변환 · 압축 ─────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzedItem {
    path: String,
    name: String,
    display: String,
    is_dir: bool,
    form: UnicodeForm,
    /// 자신 + 하위에서 NFC가 아닌 이름의 개수
    nfd_count: usize,
    /// 검사한 항목 수
    scanned: usize,
    error: Option<String>,
}

#[tauri::command]
pub async fn analyze_paths(paths: Vec<String>) -> CmdResult<Vec<AnalyzedItem>> {
    blocking(move || {
        paths
            .into_iter()
            .map(|p| {
                let path = PathBuf::from(&p);
                let raw = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.clone());
                let meta = fs::symlink_metadata(&path);
                let report = normalize(&path, Scope::Recursive, Target::Nfc, true);
                AnalyzedItem {
                    name: nfc_string(&raw),
                    display: abbreviate_home(&path),
                    is_dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                    form: detect_form(&raw),
                    nfd_count: report.renamed.len(),
                    scanned: report.scanned,
                    error: meta.err().map(|e| e.to_string()),
                    path: p,
                }
            })
            .collect()
    })
    .await
}

#[tauri::command]
pub async fn convert_paths(
    app: AppHandle,
    paths: Vec<String>,
    scope: Scope,
    target: Target,
    dry_run: bool,
) -> CmdResult<Report> {
    let report = blocking(move || {
        let mut total = Report::default();
        for p in paths {
            total.merge(normalize(Path::new(&p), scope, target, dry_run));
        }
        total
    })
    .await?;

    if !dry_run {
        let state = app.state::<AppState>();
        let arrow = match target {
            Target::Nfc => "NFD→NFC",
            Target::Nfd => "NFC→NFD",
        };
        for r in &report.renamed {
            state
                .logs
                .convert(format!("[수동 {arrow}] {}", abbreviate_home(&r.to)));
        }
        for e in &report.errors {
            state.logs.error(format!(
                "변환 실패: {} — {}",
                abbreviate_home(&e.path),
                e.message
            ));
        }
        state
            .session_converted
            .fetch_add(report.renamed.len() as u64, Ordering::Relaxed);
    }
    Ok(report)
}

#[tauri::command]
pub fn suggest_zip_path(paths: Vec<String>) -> Option<String> {
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    default_zip_path(&inputs).map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn create_zip(
    app: AppHandle,
    paths: Vec<String>,
    output: String,
    include_hidden: bool,
) -> CmdResult<ZipReport> {
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let mut output = PathBuf::from(output);
    if output.extension().map(|e| e != "zip").unwrap_or(true) {
        output.set_extension("zip");
    }
    let report = blocking(move || {
        create_nfc_zip(&inputs, &output, &ZipOptions { include_hidden }).map_err(|e| e.to_string())
    })
    .await??;
    app.state::<AppState>().logs.info(format!(
        "압축 완료: {} (파일 {}개, 이름 정규화 {}개)",
        abbreviate_home(&report.output),
        report.files,
        report.normalized_names
    ));
    Ok(report)
}

// ───────────────────────── 로그 · 기타 ─────────────────────────

#[tauri::command]
pub fn get_logs(state: State<'_, AppState>) -> Vec<LogEntry> {
    state.logs.snapshot()
}

#[tauri::command]
pub fn clear_logs(state: State<'_, AppState>) {
    state.logs.clear();
}

fn open(args: &[&str]) -> CmdResult<()> {
    Command::new("/usr/bin/open")
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reveal_in_finder(path: String) -> CmdResult<()> {
    open(&["-R", &path])
}

#[tauri::command]
pub fn open_log_folder(state: State<'_, AppState>) -> CmdResult<()> {
    open(&["-R", &state.logs.path().to_string_lossy()])
}

#[tauri::command]
pub fn open_fda_settings() -> CmdResult<()> {
    open(&["x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"])
}
