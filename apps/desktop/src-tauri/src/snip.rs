// Snip integration: the system-wide capture shortcut + the editor window.
//
// The shell contributes exactly three things to the snipping feature (all
// business logic lives in the daemon + SPA): (1) a global shortcut — the
// `snip` entry of the shortcuts.rs registry (persisted chord, ⌘⌃⇧2 default);
// (2) on fire, an `otto://menu` emit with id `"snip"` to exactly ONE window
// (the SPA's existing menu bridge runs `startSnip()` there — the webview holds
// the bearer token, the Rust side deliberately has none); (3) the
// `open_snip_window` command that mints a `w<N>` editor window pre-routed to
// `#/snip/<id>` via an injected `__OTTO_ROUTE__`.

use tauri::{AppHandle, Emitter, Manager};

pub const DEFAULT_SHORTCUT: &str = "Cmd+Ctrl+Shift+2";

/// Snip ids are daemon-generated ULIDs; the id is embedded in a window
/// initialization script, so anything but plain ASCII alphanumerics is
/// rejected outright (JS-injection guard — same rule as the daemon's).
fn snip_id_ok(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Route the trigger to exactly ONE window: focused first (the menu-event
/// pattern), else `main`, else any real window — a global shortcut usually
/// fires while Otto is NOT focused, so the fallbacks matter. The assistant bar
/// and the tray popover never qualify: their pages don't run the snip flow.
pub fn emit_snip(app: &AppHandle) {
    let wins = app.webview_windows();
    let target = wins
        .iter()
        .find(|(l, w)| crate::windows::is_app_window(l) && w.is_focused().unwrap_or(false))
        .or_else(|| wins.get_key_value("main"))
        .or_else(|| wins.iter().find(|(l, _)| crate::windows::is_app_window(l)))
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

/// The snip chord lives in the shortcuts.rs registry now; these two commands
/// stay for the Snipping settings page (SnipSettings.svelte).
#[tauri::command]
pub fn snip_get_shortcut(app: AppHandle) -> String {
    crate::shortcuts::accel_for(&app, "snip")
}

#[tauri::command]
pub fn snip_set_shortcut(app: AppHandle, accel: String) -> Result<(), String> {
    crate::shortcuts::set(&app, "snip", &accel)
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
