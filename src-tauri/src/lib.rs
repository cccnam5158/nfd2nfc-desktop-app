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

pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
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

            if !std::env::args().any(|a| a == MINIMIZED_ARG) {
                show_main_window(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 닫기 버튼은 창만 숨긴다. 감시는 메뉴바에서 계속된다.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
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
            commands::analyze_paths,
            commands::convert_paths,
            commands::suggest_zip_path,
            commands::create_zip,
            commands::get_logs,
            commands::clear_logs,
            commands::reveal_in_finder,
            commands::open_log_folder,
            commands::open_fda_settings,
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
