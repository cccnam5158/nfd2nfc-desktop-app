//! 메뉴바(트레이) 아이콘. 창을 닫아도 앱은 여기 남아 감시를 계속한다.
//!
//! 상태는 두 곳에 표시한다.
//! - 아이콘: 감시 중이면 앱 아이콘 오른쪽 아래에 녹색 점, 중지면 회색으로 흐린 아이콘
//! - 메뉴 첫 줄: 🟢/🔴 컬러 이모지. 비활성(disabled) 메뉴 항목은 macOS가 회색으로 그리므로
//!   활성 항목으로 두고, 누르면 창을 연다.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};

use crate::state::{self, AppState, WatcherStatus};

const TRAY_ID: &str = "main-tray";

pub struct TrayHandles {
    status: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    tray: TrayIcon<Wry>,
    icon_running: Option<Image<'static>>,
    icon_no_access: Option<Image<'static>>,
    icon_stopped: Option<Image<'static>>,
}

#[derive(Clone, Copy)]
enum IconState {
    Running,
    /// 감시 중이지만 전체 디스크 접근 권한이 없어 보호 폴더 이벤트를 못 받을 수 있음
    NoAccess,
    Stopped,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "🔴 감시 중지됨", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "감시 시작", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "NFD2NFC 열기…", true, Some("CmdOrCtrl+O"))?;
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

    let base = app.default_window_icon().cloned();
    let icon_running = base.as_ref().map(|b| status_icon(b, IconState::Running));
    let icon_no_access = base.as_ref().map(|b| status_icon(b, IconState::NoAccess));
    let icon_stopped = base.as_ref().map(|b| status_icon(b, IconState::Stopped));

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("NFD2NFC — 감시 중지됨")
        .icon_as_template(false)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => {
                let running = state::watcher_status(app).running;
                if let Err(e) = state::set_watching(app, !running, true) {
                    app.state::<AppState>().logs.error(e);
                }
            }
            "status" | "open" => crate::show_main_window(app),
            "quit" => {
                state::shutdown(app);
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = icon_stopped.clone() {
        builder = builder.icon(icon);
    }
    let tray = builder.build(app)?;

    *app.state::<AppState>().tray.lock().expect("tray lock") = Some(TrayHandles {
        status,
        toggle,
        tray,
        icon_running,
        icon_no_access,
        icon_stopped,
    });
    Ok(())
}

pub fn refresh(app: &AppHandle, status: &WatcherStatus) {
    let state = app.state::<AppState>();
    let Ok(guard) = state.tray.lock() else { return };
    let Some(h) = guard.as_ref() else {
        return;
    };
    let no_access = status.running && crate::fda::check() == Some(false);
    let (label, toggle, tooltip, icon) = if no_access {
        (
            "🟡 감시 중 · 전체 디스크 접근 권한 필요".to_string(),
            "감시 중지",
            "NFD2NFC — 권한이 없어 일부 폴더 감시가 동작하지 않을 수 있습니다".to_string(),
            &h.icon_no_access,
        )
    } else if status.running {
        (
            format!("🟢 감시 중 · 폴더 {}개", status.watched),
            "감시 중지",
            format!("NFD2NFC — 감시 중 (폴더 {}개)", status.watched),
            &h.icon_running,
        )
    } else {
        (
            "🔴 감시 중지됨".to_string(),
            "감시 시작",
            "NFD2NFC — 감시 중지됨".to_string(),
            &h.icon_stopped,
        )
    };
    let _ = h.status.set_text(label);
    let _ = h.toggle.set_text(toggle);
    let _ = h.tray.set_tooltip(Some(tooltip));
    if let Some(icon) = icon {
        let _ = h.tray.set_icon(Some(icon.clone()));
    }
}

/// 앱 아이콘에 상태 점을 합성한다.
/// - 감시 중: 원본 색 + 오른쪽 아래 녹색 점(흰 테두리)
/// - 권한 없음: 주황색 점
/// - 중지: 회색조로 바꾸고 투명도를 낮춰 흐리게
fn status_icon(base: &Image<'_>, state: IconState) -> Image<'static> {
    let (w, h) = (base.width(), base.height());
    let mut px = base.rgba().to_vec();

    let dot: [u8; 3] = match state {
        IconState::Running => [52, 199, 89],
        IconState::NoAccess => [255, 159, 10],
        IconState::Stopped => {
            for p in px.chunks_exact_mut(4) {
                let l = (0.30 * p[0] as f32 + 0.59 * p[1] as f32 + 0.11 * p[2] as f32) as u8;
                p[0] = l;
                p[1] = l;
                p[2] = l;
                p[3] = (p[3] as f32 * 0.55) as u8;
            }
            return Image::new_owned(px, w, h);
        }
    };

    // 점 크기는 아이콘 크기에 비례 (메뉴바에서 18pt로 줄어도 보이도록 크게)
    let size = w.min(h) as f32;
    let r = size * 0.19;
    let ring = r + size * 0.05;
    let cx = w as f32 - ring - size * 0.04;
    let cy = h as f32 - ring - size * 0.04;
    const WHITE: [u8; 3] = [255, 255, 255];

    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            // 가장자리 1px 안티앨리어싱
            let ring_cov = (ring + 0.5 - d).clamp(0.0, 1.0);
            if ring_cov <= 0.0 {
                continue;
            }
            let dot_cov = (r + 0.5 - d).clamp(0.0, 1.0);
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                let fg = dot[c] as f32 * dot_cov + WHITE[c] as f32 * (1.0 - dot_cov);
                px[i + c] = (fg * ring_cov + px[i + c] as f32 * (1.0 - ring_cov)) as u8;
            }
            px[i + 3] = (255.0 * ring_cov + px[i + 3] as f32 * (1.0 - ring_cov)) as u8;
        }
    }
    Image::new_owned(px, w, h)
}
