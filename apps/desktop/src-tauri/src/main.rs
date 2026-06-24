// Otto desktop shell — thin Tauri 2 wrapper around the SPA + daemon supervisor.
// No business logic lives here: menus, vibrancy, notifications, and ottod
// lifecycle only.

#![cfg_attr(
    all(not(debug_assertions), target_os = "macos"),
    windows_subsystem = "windows"
)]

mod browser;
mod supervisor;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager};

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        // Forward native menu-bar clicks to the SPA (which owns all behavior).
        // Registered on the Builder — the reliable place in Tauri v2 for an
        // app-wide menu set via `app.set_menu`.
        .on_menu_event(|app, event| {
            let _ = app.emit("otto://menu", event.id().0.clone());
        })
        .invoke_handler(tauri::generate_handler![
            supervisor::daemon_status,
            supervisor::daemon_start,
            supervisor::daemon_restart,
            supervisor::daemon_install,
            set_badge_count,
            browser::browser_open,
            browser::browser_bounds,
            browser::browser_navigate,
            browser::browser_reload,
            browser::browser_show,
            browser::browser_hide,
            browser::browser_hide_all,
            browser::browser_close,
            browser::browser_close_all,
            browser::browser_devtools,
        ])
        .setup(|app| {
            let window = app.get_webview_window("main").expect("main window");

            #[cfg(target_os = "macos")]
            {
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
                let _ = apply_vibrancy(&window, NSVisualEffectMaterial::Sidebar, None, None);
            }

            build_menu(app)?;

            // Ensure the daemon is up in the background; the SPA polls /health
            // and surfaces state, so failures here are non-fatal.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let report = supervisor::ensure_daemon().await;
                let _ = handle.emit("otto://daemon-state", &report);
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Otto");
}

/// Set the macOS dock badge to the number of working agents ("" clears).
#[tauri::command]
fn set_badge_count(window: tauri::WebviewWindow, count: u32) {
    let _ = window.set_badge_count(if count == 0 { None } else { Some(count as i64) });
}

fn build_menu(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle();

    let app_menu = Submenu::with_items(
        handle,
        "Otto",
        true,
        &[
            &PredefinedMenuItem::about(handle, Some("About Otto"), None)?,
            &PredefinedMenuItem::separator(handle)?,
            &MenuItem::with_id(handle, "settings", "Settings…", true, Some("Cmd+,"))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::hide(handle, None)?,
            &PredefinedMenuItem::quit(handle, None)?,
        ],
    )?;

    let file_menu = Submenu::with_items(
        handle,
        "File",
        true,
        &[
            &MenuItem::with_id(handle, "new-session", "New Session", true, Some("Cmd+T"))?,
            &MenuItem::with_id(
                handle,
                "new-workspace",
                "New Workspace…",
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(handle)?,
            &MenuItem::with_id(handle, "close-tab", "Close Tab", true, Some("Cmd+W"))?,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        handle,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(handle, None)?,
            &PredefinedMenuItem::redo(handle, None)?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::cut(handle, None)?,
            &PredefinedMenuItem::copy(handle, None)?,
            &PredefinedMenuItem::paste(handle, None)?,
            &PredefinedMenuItem::select_all(handle, None)?,
        ],
    )?;

    let view_menu = Submenu::with_items(
        handle,
        "View",
        true,
        &[
            &MenuItem::with_id(
                handle,
                "toggle-rail",
                "Toggle Navigator",
                true,
                Some("Cmd+1"),
            )?,
            &MenuItem::with_id(
                handle,
                "toggle-panel",
                "Toggle Right Panel",
                true,
                Some("Cmd+J"),
            )?,
            &PredefinedMenuItem::separator(handle)?,
            &MenuItem::with_id(handle, "zoom-in", "Zoom In", true, Some("Cmd+="))?,
            &MenuItem::with_id(handle, "zoom-out", "Zoom Out", true, Some("Cmd+-"))?,
            &MenuItem::with_id(handle, "zoom-reset", "Actual Size", true, Some("Cmd+0"))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::fullscreen(handle, None)?,
        ],
    )?;

    let session_menu = Submenu::with_items(
        handle,
        "Session",
        true,
        &[
            &MenuItem::with_id(
                handle,
                "session-restart",
                "Restart Session",
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(handle, "session-kill", "Kill Session", true, None::<&str>)?,
        ],
    )?;

    let window_menu = Submenu::with_items(
        handle,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(handle, None)?,
            &PredefinedMenuItem::maximize(handle, None)?,
        ],
    )?;

    let help_menu = Submenu::with_items(
        handle,
        "Help",
        true,
        &[&MenuItem::with_id(
            handle,
            "walkthroughs",
            "Walkthroughs",
            true,
            None::<&str>,
        )?],
    )?;

    let menu = Menu::with_items(
        handle,
        &[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &session_menu,
            &window_menu,
            &help_menu,
        ],
    )?;
    app.set_menu(menu)?;
    // Menu-event forwarding is registered on the Builder (see main()).

    Ok(())
}
