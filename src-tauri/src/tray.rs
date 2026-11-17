//! 메뉴바(트레이) 아이콘. 창을 닫아도 앱은 여기 남아 감시를 계속한다.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::state::{self, AppState, WatcherStatus};

pub struct TrayHandles {
    status: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "○ 감시 중지됨", false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "감시 시작", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "NFD2NFC 열기…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "NFD2NFC 종료", true, Some("CmdOrCtrl+Q"))?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &toggle,
            &open,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("NFD2NFC — 한글 파일명 자소 분리 해결")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => {
                let running = state::watcher_status(app).running;
                if let Err(e) = state::set_watching(app, !running, true) {
                    app.state::<AppState>().logs.error(e);
                }
            }
            "open" => crate::show_main_window(app),
            "quit" => {
                state::shutdown(app);
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    *app.state::<AppState>().tray.lock().expect("tray lock") = Some(TrayHandles { status, toggle });
    Ok(())
}

pub fn refresh(app: &AppHandle, status: &WatcherStatus) {
    let state = app.state::<AppState>();
    let Ok(guard) = state.tray.lock() else { return };
    let Some(handles) = guard.as_ref() else { return };
    let (label, toggle) = if status.running {
        (format!("● 감시 중 (폴더 {}개)", status.watched), "감시 중지")
    } else {
        ("○ 감시 중지됨".to_string(), "감시 시작")
    };
    let _ = handles.status.set_text(label);
    let _ = handles.toggle.set_text(toggle);
}
