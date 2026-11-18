mod commands;
mod fda;
mod logs;
mod state;
mod tray;
mod watcher;

use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

use crate::state::AppState;

/// 로그인 시 자동 실행될 때 넘기는 인자 — 창 없이 메뉴바에서만 시작한다.
const MINIMIZED_ARG: &str = "--minimized";

/// 창을 띄울 때는 Dock 아이콘과 앱 메뉴가 보이는 일반 앱으로 전환한다.
pub fn show_main_window(app: &AppHandle) {
    set_dock_visible(app, true);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 창을 숨기면 Dock 아이콘도 숨겨 메뉴바 전용 앱처럼 동작한다.
fn hide_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    set_dock_visible(app, false);
}

fn set_dock_visible(app: &AppHandle, visible: bool) {
    #[cfg(target_os = "macos")]
    {
        use tauri::ActivationPolicy;
        let policy = if visible {
            ActivationPolicy::Regular
        } else {
            ActivationPolicy::Accessory
        };
        let _ = app.set_activation_policy(policy);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, visible);
}

/// 자동 실행(LaunchAgent)에는 "켤 당시의 실행 파일 경로"가 기록된다.
/// 앱을 /Applications로 옮기거나 업데이트한 뒤에도 올바른 경로로 실행되도록 시작할 때 다시 기록한다.
/// 개발 빌드가 설치본의 설정을 덮어쓰지 않도록 릴리스 빌드에서만 수행한다.
fn refresh_autostart_path(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if al.is_enabled().unwrap_or(false) {
        let _ = al.enable();
    }
}

pub fn run() {
    let app = tauri::Builder::default()
        // LaunchAgent는 .app이 아니라 실행 파일을 직접 띄우므로, 중복 실행을 직접 막아야 한다.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_ARG]),
        ))
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(AppState::new(&handle)?);
            tray::create(&handle)?;

            let state = handle.state::<AppState>();
            state
                .logs
                .info(format!("NFD2NFC {} 시작", handle.package_info().version));
            let enabled = state
                .config
                .lock()
                .map(|c| c.watcher_enabled)
                .unwrap_or(false);
            if let Err(e) = state::set_watching(&handle, enabled, false) {
                state.logs.error(e);
            }

            refresh_autostart_path(&handle);
            // 권한 없이 실행되면 macOS가 보호 폴더의 FSEvents를 조용히 걸러 내므로, 진단용으로 남긴다
            state.logs.info(match fda::check() {
                Some(true) => "전체 디스크 접근 권한: 허용됨",
                Some(false) => "전체 디스크 접근 권한: 허용 안 됨 — 데스크탑·문서·다운로드·클라우드 폴더 감시가 동작하지 않을 수 있습니다",
                None => "전체 디스크 접근 권한: 확인할 수 없음",
            });

            if std::env::args().any(|a| a == MINIMIZED_ARG) {
                set_dock_visible(&handle, false);
            } else {
                show_main_window(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 닫기 버튼(⌘W)은 창만 숨긴다. 감시는 메뉴바에서 계속된다.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    hide_main_window(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::set_watching,
            commands::restart_watcher,
            commands::get_config,
            commands::evaluate_entries,
            commands::save_config,
            commands::suggested_folders,
            commands::list_dir,
            commands::scan_tree,
            commands::analyze_paths,
            commands::convert_paths,
            commands::suggest_zip_path,
            commands::create_zip,
            commands::get_logs,
            commands::clear_logs,
            commands::reveal_in_finder,
            commands::open_log_folder,
            commands::open_fda_settings,
            commands::relaunch_app,
        ])
        .build(tauri::generate_context!())
        .expect("NFD2NFC 앱을 초기화할 수 없습니다");

    app.run(|app, event| match event {
        // Dock 아이콘 클릭 시 숨겨 둔 창을 다시 연다.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main_window(app),
        RunEvent::Exit => state::shutdown(app),
        _ => {}
    });
}
