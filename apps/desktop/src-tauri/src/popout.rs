//! Native pop-out windows + "Open in Otto".
//!
//! `open_popout(route, title)` opens ONE route (a DB explorer tab, a terminal
//! session, a diff, a Design Hall artifact…) in a real macOS window beside the
//! main one (layout.md §6 — never an in-app window). The window matches the
//! app windows (overlay title bar, sidebar vibrancy) and loads
//! `index.html?popout=1#/<route>`: the SPA reads `?popout=1` (ui/src/lib/
//! desktop.ts `isPopout`) and drops the sidebar + status bar, drawing a slim
//! unified title strip instead. Its title reaches the page as
//! `window.__OTTO_POPOUT__ = {title}`.
//!
//! Pop-outs are per-route singletons (asking again focuses the open one) and
//! remember their frame PER ROUTE in `~/Library/Application Support/Otto/
//! popouts.json` (physical px, like windows.json). They are not part of the
//! windows.rs registry and aren't reopened on launch — they're views, not
//! workspaces.
//!
//! `open_in_otto(route?)` surfaces the main window (activating Otto, unlike
//! the panels) and navigates it: the assistant bar's "Open in Otto" and the
//! tray popover rows use it.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

use crate::windows::{clamp_frame, fit_frame, live_frame, monitors_of, WinFrame};

pub const PREFIX: &str = "popout-";
const DEFAULT_W: u32 = 1100;
const DEFAULT_H: u32 = 760;
const MIN_W: f64 = 480.0;
const MIN_H: f64 = 360.0;
/// Remembered frames kept (oldest routes are dropped past this).
const MAX_REMEMBERED: usize = 200;

/// label → route of every open pop-out.
static OPEN: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static SAVE_GEN: AtomicU64 = AtomicU64::new(0);

fn with_open<R>(f: impl FnOnce(&mut HashMap<String, String>) -> R) -> R {
    let mut g = OPEN.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

/// A route is embedded in a URL fragment and in `eval`'d JS (`open_in_otto`),
/// so only a conservative URL-safe alphabet passes — no quotes, backslashes,
/// whitespace, `<`/`>` or `#` can reach either. Share views (`s/…`, one-time
/// guest links) and the panel routes never pop out.
pub fn route_ok(route: &str) -> bool {
    const EXTRA: &[u8] = b"/_-.~%:=&?+,@";
    !route.is_empty()
        && route.len() <= 512
        && route
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || EXTRA.contains(&b))
        && !route.starts_with('/')
        && !route.contains("..")
        && route != "s"
        && !route.starts_with("s/")
        && !matches!(route.split(['/', '?']).next(), Some("bar" | "tray"))
}

/// Accept `#/x`, `/x` or `x`; return the bare `x`.
pub fn normalize_route(route: &str) -> String {
    route
        .trim()
        .trim_start_matches('#')
        .trim_start_matches('/')
        .to_string()
}

/// Window titles are user-visible only; strip control characters and cap the
/// length. JSON-encoded before it reaches the init script.
fn clean_title(title: Option<&str>) -> String {
    let t: String = title
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    let t = t.trim();
    if t.is_empty() {
        "Otto".to_string()
    } else {
        t.to_string()
    }
}

// ---- remembered frames ---------------------------------------------------

#[derive(Serialize, Deserialize, Default)]
struct Store {
    /// route → last frame (label field unused).
    #[serde(default)]
    frames: BTreeMap<String, WinFrame>,
    /// Recency order for the cap (most recent last).
    #[serde(default)]
    order: Vec<String>,
}

fn store_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Application Support/Otto/popouts.json")
}

fn load_store() -> Store {
    fs::read(store_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_store(s: &Store) {
    let path = store_path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, serde_json::to_vec_pretty(s).unwrap_or_default()).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

fn remember(route: &str, frame: WinFrame) {
    let mut s = load_store();
    s.frames.insert(route.to_string(), frame);
    s.order.retain(|r| r != route);
    s.order.push(route.to_string());
    while s.order.len() > MAX_REMEMBERED {
        let old = s.order.remove(0);
        s.frames.remove(&old);
    }
    save_store(&s);
}

fn remembered(route: &str) -> Option<WinFrame> {
    load_store().frames.get(route).cloned()
}

/// Persist a pop-out's current frame under its route.
fn save_frame(app: &AppHandle, label: &str) {
    let Some(route) = with_open(|o| o.get(label).cloned()) else {
        return;
    };
    if let Some(frame) = app.get_webview_window(label).as_ref().and_then(live_frame) {
        if !frame.fullscreen {
            remember(&route, frame);
        }
    }
}

/// Debounced save for the Moved/Resized storm (main.rs routes pop-out events
/// here instead of the windows.rs registry snapshot).
pub fn schedule_save(app: &AppHandle, label: &str) {
    let gen = SAVE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    let label = label.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if SAVE_GEN.load(Ordering::SeqCst) == gen {
            save_frame(&app, &label);
        }
    });
}

/// Close: keep the final frame, forget the label.
pub fn on_close_requested(app: &AppHandle, label: &str) {
    save_frame(app, label);
    with_open(|o| o.remove(label));
}

// ---- windows -------------------------------------------------------------

fn initial_frame(app: &AppHandle, route: &str) -> WinFrame {
    let monitors = monitors_of(app);
    let mut frame = remembered(route).unwrap_or_else(|| {
        // Cascade from the focused app window (else main), at a pop-out size.
        let base = app
            .webview_windows()
            .values()
            .find(|w| crate::windows::is_app_window(w.label()) && w.is_focused().unwrap_or(false))
            .and_then(live_frame)
            .or_else(|| app.get_webview_window("main").as_ref().and_then(live_frame));
        match base {
            Some(b) => WinFrame {
                label: String::new(),
                x: b.x + 48,
                y: b.y + 48,
                w: DEFAULT_W.min(b.w),
                h: DEFAULT_H.min(b.h),
                fullscreen: false,
            },
            None => WinFrame {
                label: String::new(),
                x: 160,
                y: 120,
                w: DEFAULT_W,
                h: DEFAULT_H,
                fullscreen: false,
            },
        }
    });
    frame.fullscreen = false;
    clamp_frame(&mut frame, &monitors);
    fit_frame(&mut frame, &monitors);
    frame
}

fn build(
    app: &AppHandle,
    label: &str,
    route: &str,
    title: &str,
    frame: &WinFrame,
) -> tauri::Result<tauri::WebviewWindow> {
    // `title` is JSON-encoded, which is a valid JS string literal (escapes
    // quotes, backslashes and U+2028/9) — no injection through the init script.
    let title_js = serde_json::to_string(title).unwrap_or_else(|_| "\"Otto\"".into());
    let url = format!("index.html?popout=1#/{route}");
    let win = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(title)
        .min_inner_size(MIN_W, MIN_H)
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .transparent(true)
        .accept_first_mouse(true)
        .disable_drag_drop_handler()
        .initialization_script(format!(
            "window.__OTTO_WIN__='{label}';window.__OTTO_POPOUT__={{title:{title_js}}};"
        ))
        .build()?;
    let _ = win.set_position(PhysicalPosition::new(frame.x, frame.y));
    let _ = win.set_size(PhysicalSize::new(frame.w, frame.h));
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
        let _ = apply_vibrancy(&win, NSVisualEffectMaterial::Sidebar, None, None);
    }
    Ok(win)
}

/// Focus an existing pop-out for `route`, if one is open.
fn focus_existing(app: &AppHandle, route: &str) -> Option<String> {
    let label = with_open(|o| {
        o.iter()
            .find(|(_, r)| r.as_str() == route)
            .map(|(l, _)| l.clone())
    })?;
    match app.get_webview_window(&label) {
        Some(win) => {
            let _ = win.unminimize();
            let _ = win.show();
            let _ = win.set_focus();
            Some(label)
        }
        None => {
            with_open(|o| o.remove(&label));
            None
        }
    }
}

/// Open (or focus) a native window showing one route. Returns its label.
#[tauri::command]
pub fn open_popout(app: AppHandle, route: String, title: Option<String>) -> Result<String, String> {
    let route = normalize_route(&route);
    if !route_ok(&route) {
        return Err(format!("route {route:?} can't open in its own window"));
    }
    if let Some(label) = focus_existing(&app, &route) {
        return Ok(label);
    }
    let title = clean_title(title.as_deref());
    let mut frame = initial_frame(&app, &route);
    let label = format!("{PREFIX}{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
    frame.label = label.clone();
    // Registered BEFORE build so the first Moved/Resized events already map
    // the label to its route.
    with_open(|o| o.insert(label.clone(), route.clone()));
    match build(&app, &label, &route, &title, &frame) {
        Ok(win) => {
            let _ = win.set_focus();
            Ok(label)
        }
        Err(e) => {
            with_open(|o| o.remove(&label));
            Err(format!("could not open window: {e}"))
        }
    }
}

/// Bring Otto's main window forward (activating the app) and optionally
/// navigate it to `route`. Hides the assistant bar + tray popover: the work
/// continues in the full window.
#[tauri::command]
pub fn open_in_otto(app: AppHandle, route: Option<String>) -> Result<(), String> {
    let route = match route.as_deref().map(normalize_route) {
        Some(r) if r.is_empty() => None,
        Some(r) if route_ok(&r) => Some(r),
        Some(r) => return Err(format!("route {r:?} is not navigable")),
        None => None,
    };
    crate::bar::hide(&app);
    crate::tray::hide_popover(&app);
    let win =
        crate::windows::primary_window(&app).ok_or_else(|| "no Otto window to open".to_string())?;
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    if let Some(r) = route {
        // Safe to splice: route_ok admits no quote, backslash or newline.
        let _ = win.eval(format!("location.hash='#/{r}'"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_guard_admits_app_routes() {
        for r in [
            "agents/01J8ZX0N7Q2R4T6V8X0Z2B4D6F",
            "database/01H/tab/3",
            "git/01H/pr/42",
            "design/artifact/01J?version=3",
            "vault/notes/My%20Note.md",
            "settings/appearance",
            "mcp/activity",
        ] {
            assert!(route_ok(r), "{r} should be allowed");
        }
    }

    #[test]
    fn route_guard_rejects_injection_and_special_routes() {
        for r in [
            "",
            "agents/x'; alert(1);//",
            "agents/x\"",
            "a\\b",
            "a b",
            "a\nb",
            "a<script>",
            "a#b",
            "/etc/passwd",
            "../x",
            "s",
            "s/01J/token",
            "bar",
            "tray",
            "tray?x=1",
        ] {
            assert!(!route_ok(r), "{r:?} must be rejected");
        }
        assert!(!route_ok(&"a".repeat(513)));
        // A module merely starting with those letters is fine.
        assert!(route_ok("settings"));
        assert!(route_ok("barista"));
    }

    #[test]
    fn normalize_strips_hash_and_slashes() {
        assert_eq!(normalize_route("#/git/1"), "git/1");
        assert_eq!(normalize_route("/git/1"), "git/1");
        assert_eq!(normalize_route("  git/1 "), "git/1");
    }

    #[test]
    fn titles_are_cleaned() {
        assert_eq!(clean_title(None), "Otto");
        assert_eq!(clean_title(Some("  ")), "Otto");
        assert_eq!(clean_title(Some("DB\n— orders")), "DB— orders");
        assert_eq!(clean_title(Some(&"x".repeat(500))).chars().count(), 120);
        // JSON encoding is what reaches the init script: one double-quoted
        // literal with inner quotes escaped.
        let js = serde_json::to_string("a'b\"c").unwrap();
        assert_eq!(js, r#""a'b\"c""#);
    }
}
