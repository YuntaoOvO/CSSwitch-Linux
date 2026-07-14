//! System tray: minimize to tray on close, right-click to quit + stop daemon.
//! Dynamically switches icon based on daemon health status:
//!   - tray-idle.png  (gray)   — proxy not running
//!   - tray-active.png (green) — proxy healthy
//!   - tray-error.png  (amber) — proxy running but unresponsive

use std::process::Command;

use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};



pub fn setup_tray<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let show_item = MenuItemBuilder::with_id("show", "显示面板").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "终止代理并退出").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&show_item)
        .separator()
        .item(&quit_item)
        .build()?;

    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .expect("Failed to load tray icon");

    let _tray = TrayIconBuilder::with_id("csswitch-tray")
        .icon(icon)
        .tooltip("CSSwitch")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => quit_with_daemon_stop(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                toggle_main_window(app);
            }
        })
        .build(app)?;

    Ok(())
}

/// Called after proxy daemon start/stop/status changes.
/// Reads daemon health and updates tray icon accordingly.
pub fn refresh_tray_icon<R: Runtime>(app: &AppHandle<R>) {
    let tray = match app.tray_by_id("csswitch-tray") {
        Some(t) => t,
        None => return,
    };

    let icon_bytes: &[u8] = if daemon_healthy() {
        include_bytes!("../icons/tray-active.png")
    } else if daemon_running() {
        include_bytes!("../icons/tray-error.png")
    } else {
        include_bytes!("../icons/tray-idle.png")
    };

    let icon = match Image::from_bytes(icon_bytes) {
        Ok(icon) => icon,
        Err(_) => return,
    };

    let _ = tray.set_icon(Some(icon));
}

/// Check if daemon process is running (has PID file + process alive).
fn daemon_running() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    let pid_path = std::path::PathBuf::from(home).join(".csswitch").join("daemon.pid");
    match std::fs::read_to_string(&pid_path) {
        Ok(pid_str) => {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                if pid == std::process::id() as i32 {
                    return true;
                }
                std::path::PathBuf::from(format!("/proc/{}", pid)).exists()
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Check if proxy daemon is healthy (process alive + HTTP health check passing).
fn daemon_healthy() -> bool {
    if !daemon_running() {
        return false;
    }
    let dir = crate::config::default_dir();
    let cfg = match crate::config::load_from(&dir) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if cfg.secret.is_empty() {
        return false;
    }
    crate::proc::http_health(cfg.proxy_port, Some(&cfg.secret), 500)
}

#[tauri::command]
pub fn update_tray_icon(app: tauri::AppHandle) {
    refresh_tray_icon(&app);
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn toggle_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

fn quit_with_daemon_stop<R: Runtime>(app: &AppHandle<R>) {
    let _ = Command::new("csswitch").args(["daemon", "stop"]).output();
    app.exit(0);
}
