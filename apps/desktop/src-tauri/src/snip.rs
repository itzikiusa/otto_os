// Snip integration: the system-wide capture shortcut + the editor window.
//
// The shell contributes exactly three things to the snipping feature (all
// business logic lives in the daemon + SPA): (1) a global shortcut registered
// via tauri-plugin-global-shortcut, persisted in `<app-config>/snip.json`;
// (2) on fire, an `otto://menu` emit with id `"snip"` to exactly ONE window
// (the SPA's existing menu bridge runs `startSnip()` there — the webview holds
// the bearer token, the Rust side deliberately has none); (3) the
// `open_snip_window` command that mints a `w<N>` editor window pre-routed to
// `#/snip/<id>` via an injected `__OTTO_ROUTE__`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub const DEFAULT_SHORTCUT: &str = "Cmd+Ctrl+Shift+2";

#[derive(Serialize, Deserialize, Default)]
struct SnipConfig {
    /// Global-shortcut accelerator; empty string = disabled.
    shortcut: Option<String>,
}

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("snip.json"))
}

fn load_shortcut(app: &AppHandle) -> String {
    config_path(app)
        .and_then(|p| fs::read(p).ok())
        .and_then(|raw| serde_json::from_slice::<SnipConfig>(&raw).ok())
        .and_then(|c| c.shortcut)
        .unwrap_or_else(|| DEFAULT_SHORTCUT.to_string())
}

fn save_shortcut(app: &AppHandle, accel: &str) {
    let Some(path) = config_path(app) else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let cfg = SnipConfig { shortcut: Some(accel.to_string()) };
    if let Ok(raw) = serde_json::to_vec_pretty(&cfg) {
        let _ = fs::write(path, raw);
    }
}

/// Snip ids are daemon-generated ULIDs; the id is embedded in a window
/// initialization script, so anything but plain ASCII alphanumerics is
/// rejected outright (JS-injection guard — same rule as the daemon's).
fn snip_id_ok(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Route the trigger to exactly ONE window: focused first (the menu-event
/// pattern), else `main`, else any real window — a global shortcut usually
/// fires while Otto is NOT focused, so the fallbacks matter.
fn emit_snip(app: &AppHandle) {
    let wins = app.webview_windows();
    let target = wins
        .iter()
        .find(|(l, w)| !l.starts_with("otto-browser-") && w.is_focused().unwrap_or(false))
        .or_else(|| wins.get_key_value("main"))
        .or_else(|| wins.iter().find(|(l, _)| !l.starts_with("otto-browser-")))
        .map(|(l, _)| l.clone());
    match target {
        Some(label) => {
            let _ = app.emit_to(label.as_str(), "otto://menu", "snip".to_string());
        }
        None => {
            let _ = app.emit("otto://menu", "snip".to_string());
        }
    }
}

/// (Re)register the global shortcut. Empty accel = disabled. Unregisters all
/// previous snip shortcuts first (this module owns every global shortcut in
/// the app today).
pub fn register(app: &AppHandle, accel: &str) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    if accel.is_empty() {
        return Ok(());
    }
    let sc: Shortcut = accel
        .parse()
        .map_err(|e| format!("invalid shortcut {accel:?}: {e}"))?;
    gs.on_shortcut(sc, |app, _sc, event| {
        if event.state() == ShortcutState::Pressed {
            emit_snip(app);
        }
    })
    .map_err(|e| format!("could not register {accel:?}: {e}"))
}

/// Startup: register the persisted (or default) chord. Non-fatal on failure —
/// e.g. another app holds the chord; the in-app triggers still work and the
/// Settings page surfaces the error on change.
pub fn init(app: &AppHandle) {
    let accel = load_shortcut(app);
    if let Err(e) = register(app, &accel) {
        eprintln!("snip: global shortcut unavailable: {e}");
    }
}

#[tauri::command]
pub fn snip_get_shortcut(app: AppHandle) -> String {
    load_shortcut(&app)
}

#[tauri::command]
pub fn snip_set_shortcut(app: AppHandle, accel: String) -> Result<(), String> {
    register(&app, &accel)?;
    save_shortcut(&app, &accel);
    Ok(())
}

/// Open a dedicated editor window for a snip, pre-routed to `#/snip/<id>`.
#[tauri::command]
pub fn open_snip_window(app: AppHandle, snip_id: String) -> Result<(), String> {
    if !snip_id_ok(&snip_id) {
        return Err("invalid snip id".into());
    }
    crate::windows::create_snip_window(&app, &snip_id)
}

#[cfg(test)]
mod tests {
    use super::snip_id_ok;

    #[test]
    fn snip_id_guards_injection() {
        assert!(snip_id_ok("01J8ZX0N7Q2R4T6V8X0Z2B4D6F"));
        assert!(!snip_id_ok("abc'); alert(1);//"));
        assert!(!snip_id_ok("../../etc"));
        assert!(!snip_id_ok("short"));
        assert!(!snip_id_ok(""));
    }
}
