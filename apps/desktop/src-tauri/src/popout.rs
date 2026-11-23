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
/// Serializes read-modify-write persistence on `popouts.json` across debounced
/// async `schedule_save` tasks and main-thread `on_close_requested` calls.
static STORE_LOCK: Mutex<()> = Mutex::new(());
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static SAVE_GEN: AtomicU64 = AtomicU64::new(0);
static STORE_TMP_SEQ: AtomicU64 = AtomicU64::new(1);

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

fn load_store_at(path: &std::path::Path) -> Store {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn unique_store_tmp(path: &std::path::Path) -> PathBuf {
    let seq = STORE_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("json.tmp.{}.{seq}", std::process::id()))
}

fn save_store_at(path: &std::path::Path, s: &Store) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = unique_store_tmp(path);
    if fs::write(&tmp, serde_json::to_vec_pretty(s).unwrap_or_default()).is_ok()
        && fs::rename(&tmp, path).is_err()
    {
        let _ = fs::remove_file(&tmp);
    }
}

fn remember_at(path: &std::path::Path, route: &str, frame: WinFrame) {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut s = load_store_at(path);
    s.frames.insert(route.to_string(), frame);
    s.order.retain(|r| r != route);
    s.order.push(route.to_string());
    while s.order.len() > MAX_REMEMBERED {
        let old = s.order.remove(0);
        s.frames.remove(&old);
    }
    save_store_at(path, &s);
}

fn remember(route: &str, frame: WinFrame) {
    remember_at(&store_path(), route, frame);
}

fn remembered(route: &str) -> Option<WinFrame> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    load_store_at(&store_path()).frames.get(route).cloned()
}

/// Persist a pop-out's current frame under its route.
fn save_frame(app: &AppHandle, label: &str) {
    let Some(route) = with_open(|o| o.get(label).cloned()) else {
        return;
    };
    if let Some(frame) = app.get_window(label).as_ref().and_then(live_frame) {
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
            .windows()
            .values()
            .find(|w| crate::windows::is_app_window(w.label()) && w.is_focused().unwrap_or(false))
            .and_then(live_frame)
            .or_else(|| app.get_window("main").as_ref().and_then(live_frame));
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
        .background_throttling(crate::NO_THROTTLE)
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
        if let Some(view) = app.get_webview(win.label()) {
            let _ = view.eval(format!("location.hash='#/{r}'"));
        }
    }
    Ok(())
}

/// Top-level Otto app routes admissible via external `otto://` deep links.
const DEEP_LINK_MODULES: &[&str] = &[
    "home",
    "agents",
    "threads",
    "rooms",
    "loops",
    "workflows",
    "scheduled-tasks",
    "personal-agents",
    "ide",
    "git",
    "product",
    "design",
    "canvas",
    "vault",
    "skills-eval",
    "skills-lab",
    "connections",
    "database",
    "brokers",
    "aws",
    "kubernetes",
    "containers",
    "api",
    "browser",
    "mcp",
    "insights",
    "usage",
    "settings",
    "plugins",
    "help",
    "walkthroughs",
    "snip",
];

/// Reserved sub-route prefixes under `design/` that must not be rewritten to
/// `design/a/<id>`.
const DESIGN_SUBROUTES: &[&str] = &[
    "a", "artifact", "p", "studio", "story", "brand", "learned", "spatial",
];

/// Read-only query keys permitted on external `otto://` deep links.
/// All mutating or action-triggering query params (e.g. `ide?collab=`,
/// `ide?root=`, `ide?scratches=`, `browser?url=`, `browser?device=`) are
/// rejected because external links arrive without user confirmation.
fn deep_link_query_allowed(module: &str, key: &str) -> bool {
    matches!((module, key), ("design", "version" | "from" | "tab"))
}

fn contains_blocked_percent_encoding(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") || lower.contains("%25")
}

/// Normalize canonical `otto://story/<id>` and `otto://design/<id>[@sel]`
/// reference paths into Otto's internal UI hash routes.
fn canonicalize_deep_link_path(path: &str) -> Result<String, &'static str> {
    if let Some(rest) = path.strip_prefix("story/") {
        if rest.is_empty() || rest.contains('/') || rest.contains("..") {
            return Err("invalid story deep link");
        }
        return Ok(format!("product/{rest}"));
    }
    if path == "story" {
        return Err("story deep link requires an id");
    }
    if let Some(rest) = path.strip_prefix("design/") {
        if !rest.is_empty() && !rest.contains('/') && !DESIGN_SUBROUTES.contains(&rest) {
            let id = rest.split('@').next().unwrap_or("");
            if id.is_empty() || id.contains("..") {
                return Err("invalid design deep link");
            }
            return Ok(format!("design/a/{id}"));
        }
    }
    Ok(path.to_string())
}

/// Parse and strictly validate a macOS `otto://` deep-link URL into an
/// internal hash route (or `None` to simply bring Otto's window forward).
///
/// Admitted forms:
///   * `otto://open/<route>[?<read-only-query>]` → `Some("<route>[?<query>]")`
///   * `otto://<module>[/<rest>][?<read-only-query>]` → `Some("<module>[/<rest>][?<query>]")`
///   * `otto://story/<id>` → `Some("product/<id>")`
///   * `otto://design/<id>[@<selector>]` → `Some("design/a/<id>")`
///   * `otto://open`, `otto://open/`, `otto://` → `None` (focus only)
///
/// Every non-empty route is gated by `DEEP_LINK_MODULES`, a strict read-only
/// query allowlist, and `route_ok`: foreign schemes, unknown modules,
/// credentials, ports, fragments, traversal (`..` or encoded `%2e`/`%2f`/`%5c`/`%25`),
/// mutating query params, control characters, quotes/backslashes, share views
/// (`s/...`), and panel routes (`bar`/`tray`) are all rejected.
pub fn parse_deep_link_url(url: &tauri::Url) -> Result<Option<String>, &'static str> {
    if !url.scheme().eq_ignore_ascii_case("otto") {
        return Err("unsupported scheme");
    }
    if url.as_str().chars().any(|c| c.is_control()) {
        return Err("control character in url");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("credentials not permitted");
    }
    if url.port().is_some() {
        return Err("port not permitted");
    }
    if url.fragment().is_some() {
        return Err("fragment not permitted");
    }

    let path = url.path().trim_start_matches('/');
    let raw_path = match url.host_str() {
        Some(host) if host.eq_ignore_ascii_case("open") => path.to_string(),
        Some(host) if !host.is_empty() => {
            if path.is_empty() {
                host.to_string()
            } else {
                format!("{host}/{path}")
            }
        }
        _ => path.to_string(),
    };

    if raw_path.is_empty() {
        if url.query().is_some() {
            return Err("query without route");
        }
        return Ok(None);
    }

    if contains_blocked_percent_encoding(&raw_path) {
        return Err("encoded traversal or separator rejected");
    }

    let normalized_path = normalize_route(&raw_path);
    let mut route = canonicalize_deep_link_path(&normalized_path)?;
    let raw_head = route.split('/').next().unwrap_or("");
    let head = *DEEP_LINK_MODULES
        .iter()
        .find(|&&m| m == raw_head)
        .ok_or("unknown module route")?;

    if let Some(q) = url.query() {
        if !q.is_empty() {
            if contains_blocked_percent_encoding(q) {
                return Err("encoded traversal or separator rejected");
            }
            for pair in q.split('&') {
                let (key, val) = pair
                    .split_once('=')
                    .ok_or("query parameter not permitted")?;
                if key.is_empty() || val.is_empty() || !deep_link_query_allowed(head, key) {
                    return Err("query parameter not permitted");
                }
            }
            route.push('?');
            route.push_str(q);
        }
    }

    if !route_ok(&route) {
        return Err("route rejected by validator");
    }
    Ok(Some(route))
}

/// Handle macOS `RunEvent::Opened` URLs routed to Otto via `CFBundleURLTypes`.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn handle_open_urls(app: &AppHandle, urls: &[tauri::Url]) {
    for url in urls {
        match parse_deep_link_url(url) {
            Ok(route) => {
                if let Err(e) = open_in_otto(app.clone(), route) {
                    crate::lifecycle::event(&format!("deep link failed: {e}"));
                }
                break;
            }
            Err(why) => {
                crate::lifecycle::event(&format!(
                    "deep link rejected (scheme={}): {why}",
                    url.scheme()
                ));
            }
        }
    }
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

    #[test]
    fn concurrent_remember_writes_do_not_lose_routes_or_leave_temp_files() {
        let dir = std::env::temp_dir().join(format!("otto-popout-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("popouts.json");

        assert_ne!(unique_store_tmp(&path), unique_store_tmp(&path));

        std::thread::scope(|s| {
            for i in 0..8_u32 {
                let p = path.clone();
                s.spawn(move || {
                    remember_at(
                        &p,
                        &format!("git/{i}"),
                        WinFrame {
                            label: format!("popout-{i}"),
                            x: i as i32 * 10,
                            y: 20,
                            w: 900,
                            h: 700,
                            fullscreen: false,
                        },
                    );
                });
            }
        });

        let loaded = load_store_at(&path);
        assert_eq!(loaded.frames.len(), 8);
        for i in 0..8_u32 {
            assert!(loaded.frames.contains_key(&format!("git/{i}")));
        }
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deep_link_accepts_otto_open_and_module_urls() {
        let cases = [
            ("otto://open/git/01H/pr/42", Some("git/01H/pr/42")),
            ("otto://git/01H/pr/42", Some("git/01H/pr/42")),
            (
                "otto://open/design/artifact/01J?version=3",
                Some("design/artifact/01J?version=3"),
            ),
            ("otto://story/STORY-42", Some("product/STORY-42")),
            ("otto://open/story/STORY-42", Some("product/STORY-42")),
            ("otto://design/01J", Some("design/a/01J")),
            ("otto://design/01J@v3", Some("design/a/01J")),
            ("otto://design/01J@approved", Some("design/a/01J")),
            (
                "otto://open/design/01J?version=3",
                Some("design/a/01J?version=3"),
            ),
            ("otto://design", Some("design")),
            ("otto://design/brand", Some("design/brand")),
            ("otto://design/learned", Some("design/learned")),
            ("otto://design/a/01J", Some("design/a/01J")),
            ("otto://mcp/activity", Some("mcp/activity")),
            ("otto://settings/appearance", Some("settings/appearance")),
            ("otto://home", Some("home")),
            ("otto://open", None),
            ("otto://open/", None),
            ("otto://", None),
        ];
        for (raw, expected) in cases {
            let u = tauri::Url::parse(raw).expect("valid url");
            assert_eq!(
                parse_deep_link_url(&u),
                Ok(expected.map(str::to_string)),
                "url {raw}"
            );
        }
    }

    #[test]
    fn deep_link_rejects_unsafe_schemes_traversal_injection_and_special_routes() {
        for raw in [
            "https://evil.example/git/1",
            "http://127.0.0.1:7700/home",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "otto://open/../etc/passwd",
            "otto://open/git/..%2fsecret",
            "otto://open/git/.%2e/secret",
            "otto://open/git/%2e./secret",
            "otto://open/git/%252e%252e/secret",
            "otto://open/git/a%2fb",
            "otto://open/git/a%5cb",
            "otto://open/design/artifact/01J?version=%252e",
            "otto://ide?collab=room1&token=abc",
            "otto://ide?root=/Users/victim/.ssh&file=id_rsa",
            "otto://ide?scratches=new",
            "otto://browser?url=https://evil.example&mode=live&device=simulator:ios:1",
            "otto://open/browser?url=http://localhost:3000",
            "otto://open/design/artifact/01J?evil=1",
            "otto://story",
            "otto://design/@bad",
            "otto://open/bar",
            "otto://open/tray",
            "otto://bar",
            "otto://tray/status",
            "otto://open/s/guest-token",
            "otto://s/guest-token",
            "otto://user:pass@open/home",
            "otto://open:8080/home",
            "otto://open/git/x';alert(1)//",
            "otto://open/home#smuggled",
            "otto://open?only=query",
        ] {
            let u = tauri::Url::parse(raw).expect("parseable url");
            assert!(
                parse_deep_link_url(&u).is_err(),
                "expected {raw:?} to be rejected"
            );
        }
    }
}
