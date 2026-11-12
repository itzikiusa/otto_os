//! Native in-app browser: one child webview **per tab**, overlaid on the right
//! panel's Browser tab. Unlike an `<iframe>`, a real webview ignores
//! `X-Frame-Options` / CSP `frame-ancestors` and can load `http://localhost`, so
//! it behaves like a normal browser. The SPA owns the tab strip and drives each
//! tab's webview through these commands, keeping the ACTIVE tab's bounds in sync
//! with the panel rect and hiding the rest. Requires Tauri's `unstable`
//! multiwebview feature.
//!
//! Each tab webview is labelled `otto-browser-<tabId>` where `<tabId>` is an
//! opaque id minted by the SPA.
//!
//! Two interceptors are installed per webview at creation:
//!   * `on_new_window` — a `window.open()` / `target=_blank` is denied (no OS
//!     popup floating over the app) and surfaced as `otto://browser-new-tab` so
//!     the SPA opens a real in-app tab and focuses it.
//!   * `on_navigation` — every committed navigation is pushed to the SPA as
//!     `otto://browser-url` so the address bar tracks in-page link clicks.
//!     This REPLACES polling `webview.url()`, which panics inside wry when the
//!     page hasn't committed a load yet (`unwrap` on a nil `WKWebView.URL`).
//!     That panic was doubly fatal: these commands run inside WebKit's
//!     `extern "C"` URL-scheme handler (so a panic can't unwind and aborts), AND
//!     `webview_getter!` holds the shared `window_id` mutex across the call, so
//!     the unwind POISONS it — making every later `set_position`/`set_size`
//!     panic too. Never calling `url()` avoids the panic (and the poison) at the
//!     source; `catch_unwind` below is belt-and-suspenders for any other wry
//!     panic so it degrades to a no-op instead of crashing the app.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};
use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

/// Label prefix for every browser-tab webview (`otto-browser-<tabId>`).
const PREFIX: &str = "otto-browser-";

/// Emitted (payload = requested URL) when a tab asks to open a new window.
const NEW_TAB_EVENT: &str = "otto://browser-new-tab";
/// Emitted (payload = `[tabId, url]`) on every committed navigation in a tab.
const URL_EVENT: &str = "otto://browser-url";

fn label(id: &str) -> String {
    format!("{PREFIX}{id}")
}

/// The in-app browser's OWN WebKit data store (cookies, cache, site storage),
/// apart from the default store Otto's own window uses — so "Clear browsing
/// data" can never sign Otto itself out. One store for every tab of every
/// workspace and window. Any 16 bytes name a store; fixed so it persists.
const BROWSER_DATA_STORE: [u8; 16] = *b"otto-browser-v1\0";

/// `WKWebsiteDataStore(forIdentifier:)` needs macOS 14. Below it wry silently
/// falls back to the DEFAULT store (wry `wkwebview/mod.rs`), i.e. Otto's own:
/// tabs then share it as before, and clearing is refused.
fn own_data_store() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
            .ok()
            .and_then(|p| macos_major(&p))
            .is_some_and(|v| v >= 14)
    })
}

/// The major version in a `SystemVersion.plist` body (`ProductVersion`).
fn macos_major(plist: &str) -> Option<u32> {
    let after = plist.split("<key>ProductVersion</key>").nth(1)?;
    let v = after.split("<string>").nth(1)?.split('<').next()?;
    v.trim().split('.').next()?.parse().ok()
}

/// `builder` on the browser's own data store, where macOS has one.
fn with_browser_store<R: tauri::Runtime>(
    builder: tauri::webview::WebviewBuilder<R>,
) -> tauri::webview::WebviewBuilder<R> {
    if own_data_store() {
        builder.data_store_identifier(BROWSER_DATA_STORE)
    } else {
        builder
    }
}

/// Browser tabs are for web pages only: `http(s)` plus `about:blank`. Anything
/// else (`file:`, `tauri:`, `asset:`, custom app schemes, `javascript:`…) is
/// refused — a tab must never become a way to read local files or reach an
/// app-internal / OS URL handler.
fn allowed_tab_url(u: &tauri::Url) -> bool {
    match u.scheme() {
        "http" | "https" => true,
        "about" => u.as_str() == "about:blank",
        _ => false,
    }
}

/// The URL each tab webview was last sent to (`browser_open` /
/// `browser_navigate`) or last navigated to by the page (`on_navigation`), by
/// label. Tracked here because `webview.url()` panics before a first load
/// commits (see the module docs).
fn shown_urls() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    static SHOWN: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    SHOWN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Record (`Some`) or forget (`None`) what webview `lbl` shows.
fn set_shown(lbl: &str, url: Option<&tauri::Url>) {
    match url {
        Some(u) => shown_urls().insert(lbl.to_string(), u.to_string()),
        None => shown_urls().remove(lbl),
    };
}

/// Whether `browser_open` must navigate webview `lbl` to `url` — not when it
/// already shows it (both sides are parsed, so `https://a.example` and
/// `https://a.example/` are the same page). A re-mounted Browser view
/// re-opening its tab must not reload the page and lose its scroll, form and
/// app state; an explicit reload is `browser_reload`.
fn needs_navigation(lbl: &str, url: &tauri::Url) -> bool {
    shown_urls().get(lbl).map(String::as_str) != Some(url.as_str())
}

fn parse_url(url: &str) -> Result<tauri::Url, String> {
    let u: tauri::Url = url.parse().map_err(|e| format!("bad url '{url}': {e}"))?;
    if !allowed_tab_url(&u) {
        return Err(format!(
            "unsupported url scheme '{}:' (http/https only)",
            u.scheme()
        ));
    }
    Ok(u)
}

/// Open (or reposition + show, navigating only when it shows another URL —
/// see [`needs_navigation`]) the webview for tab `id` at the given panel rect. Coordinates are logical (CSS) pixels relative to the
/// window's top-left, matching `getBoundingClientRect()` in the SPA.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // tauri command: flat invoke args (id + url + rect)
pub fn browser_open(
    app: tauri::AppHandle,
    window: tauri::Window,
    id: String,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let parsed = parse_url(&url)?;
    let lbl = label(&id);
    // Existing tab → reposition, navigate, show. Guard against any wry panic so a
    // transient failure degrades to a no-op rather than aborting the whole app.
    if let Some(wv) = app.get_webview(&lbl) {
        let navigate = needs_navigation(&lbl, &parsed);
        return catch_unwind(AssertUnwindSafe(move || -> Result<(), String> {
            let _ = wv.set_position(LogicalPosition::new(x, y));
            let _ = wv.set_size(LogicalSize::new(width.max(1.0), height.max(1.0)));
            if navigate {
                wv.navigate(parsed.clone()).map_err(|e| e.to_string())?;
                set_shown(&lbl, Some(&parsed));
            }
            let _ = wv.show();
            Ok(())
        }))
        .unwrap_or_else(|_| Err("browser webview panicked while navigating".into()));
    }
    // Anchor the tab to the window the SPA invoked us from (multi-window: each
    // window's Browser panel owns its own child webviews), and emit its events
    // to THAT window only — a broadcast would open the tab / move the address
    // bar in every window's Browser panel at once.
    let win_label = window.label().to_string();
    let app_nav = app.clone();
    let id_nav = id.clone();
    let lbl_nav = win_label.clone();
    let app_new = app.clone();
    let lbl_seen = lbl.clone();
    set_shown(&lbl, Some(&parsed));
    let builder = tauri::webview::WebviewBuilder::new(&lbl, WebviewUrl::External(parsed));
    let builder = with_browser_store(builder)
        .transparent(false)
        // Track in-page navigation for the address bar (never calls url()).
        .on_navigation(move |u: &tauri::Url| {
            // A page can't navigate the tab off the web (file:, app schemes…).
            if !allowed_tab_url(u) {
                return false;
            }
            set_shown(&lbl_seen, Some(u));
            let _ = app_nav.emit_to(lbl_nav.as_str(), URL_EVENT, (id_nav.clone(), u.to_string()));
            true
        })
        // Deny OS popups; ask the SPA to open a real in-app tab instead.
        .on_new_window(move |u: tauri::Url, _features: NewWindowFeatures| {
            if allowed_tab_url(&u) {
                let _ = app_new.emit_to(win_label.as_str(), NEW_TAB_EVENT, u.to_string());
            }
            NewWindowResponse::Deny
        });
    window
        .add_child(
            builder,
            LogicalPosition::new(x, y),
            LogicalSize::new(width.max(1.0), height.max(1.0)),
        )
        .map_err(|e| {
            set_shown(&lbl, None);
            e.to_string()
        })?;
    Ok(())
}

/// Reposition/resize tab `id`'s webview to track the panel.
#[tauri::command]
pub fn browser_bounds(app: tauri::AppHandle, id: String, x: f64, y: f64, width: f64, height: f64) {
    if let Some(wv) = app.get_webview(&label(&id)) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _ = wv.set_position(LogicalPosition::new(x, y));
            let _ = wv.set_size(LogicalSize::new(width.max(1.0), height.max(1.0)));
        }));
    }
}

/// Run `js` in tab `id`'s webview and resolve with its stringified result
/// (JSON-encoded, matching wry's `eval_with_callback` contract). Used by the
/// overlay-injection bridge (marks/notes over a live page) — the promise
/// resolves once WebKit's `evaluateJavaScript` completion handler fires, so a
/// caller can `await` a value straight out of the page.
#[tauri::command]
pub async fn browser_eval(app: tauri::AppHandle, id: String, js: String) -> Result<String, String> {
    let Some(wv) = app.get_webview(&label(&id)) else {
        return Err(format!("no live webview for tab {id}"));
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = std::sync::Mutex::new(Some(tx));
    let dispatch = catch_unwind(AssertUnwindSafe(|| {
        wv.eval_with_callback(js, move |result: String| {
            if let Some(tx) = tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = tx.send(result);
            }
        })
    }));
    match dispatch {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => return Err("browser webview panicked while evaluating".into()),
    }
    await_eval_result(rx, EVAL_TIMEOUT).await
}

/// How long an eval may take before the call gives up. WebKit only fires the
/// completion handler when the page's JS turn finishes — a hung or busy page
/// (or a discarded web process) never fires it, and the SPA's overlay/login
/// polls would otherwise stack one never-resolving IPC call per tick.
const EVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

async fn await_eval_result(
    rx: tokio::sync::oneshot::Receiver<String>,
    limit: std::time::Duration,
) -> Result<String, String> {
    match tokio::time::timeout(limit, rx).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(_)) => Err("eval callback never fired".to_string()),
        Err(_) => Err(format!("eval timed out after {} ms", limit.as_millis())),
    }
}

/// Navigate tab `id`'s webview to a new URL.
#[tauri::command]
pub fn browser_navigate(app: tauri::AppHandle, id: String, url: String) -> Result<(), String> {
    if let Some(wv) = app.get_webview(&label(&id)) {
        let parsed = parse_url(&url)?;
        set_shown(&label(&id), Some(&parsed));
        return catch_unwind(AssertUnwindSafe(move || {
            wv.navigate(parsed).map_err(|e| e.to_string())
        }))
        .unwrap_or_else(|_| Err("browser webview panicked while navigating".into()));
    }
    Ok(())
}

/// Reload tab `id`'s current page.
#[tauri::command]
pub fn browser_reload(app: tauri::AppHandle, id: String) {
    if let Some(wv) = app.get_webview(&label(&id)) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _ = wv.eval("location.reload()");
        }));
    }
}

#[tauri::command]
pub fn browser_show(app: tauri::AppHandle, id: String) {
    if let Some(wv) = app.get_webview(&label(&id)) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _ = wv.show();
        }));
    }
}

#[tauri::command]
pub fn browser_hide(app: tauri::AppHandle, id: String) {
    if let Some(wv) = app.get_webview(&label(&id)) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _ = wv.hide();
        }));
    }
}

/// Hide every browser-tab webview OF THE INVOKING WINDOW — used when an SPA
/// overlay (palette, modal, context menu) opens over the panel, or when the
/// Browser tab isn't visible. Scoped per window so window A's palette doesn't
/// blank window B's browser.
#[tauri::command]
pub fn browser_hide_all(app: tauri::AppHandle, window: tauri::Window) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        for (lbl, wv) in app.webviews() {
            if lbl.starts_with(PREFIX) && wv.window().label() == window.label() {
                let _ = wv.hide();
            }
        }
    }));
}

#[tauri::command]
pub fn browser_close(app: tauri::AppHandle, id: String) {
    set_shown(&label(&id), None);
    if let Some(wv) = app.get_webview(&label(&id)) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _ = wv.close();
        }));
    }
}

/// Toggle the web inspector (DevTools — console, network, elements, …) for tab
/// `id`, opened **DETACHED** (its own window). Tauri's `open_devtools()` shows it
/// docked, which resizes the in-panel child webview to make room — that fought
/// our bounds-sync (constant flicker) and made the webview spread over the
/// session. Detaching pops the inspector into a separate window, so the panel
/// webview is never touched. Enabled in release via the tauri `devtools` feature.
#[tauri::command]
pub fn browser_devtools(app: tauri::AppHandle, id: String) {
    let Some(wv) = app.get_webview(&label(&id)) else {
        return;
    };
    let close = wv.is_devtools_open();
    let _ = wv.with_webview(move |pw| {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(target_os = "macos")]
            unsafe {
                use objc2::runtime::AnyObject;
                let raw = pw.inner();
                if raw.is_null() {
                    return;
                }
                // WKWebView → private `_inspector` (the WKInspector). Same access
                // wry uses for open/close; `detach` is its sibling selector.
                let wk: *mut AnyObject = raw.cast();
                let inspector: *mut AnyObject = objc2::msg_send![wk, _inspector];
                if inspector.is_null() {
                    return;
                }
                if close {
                    let _: () = objc2::msg_send![inspector, close];
                } else {
                    let _: () = objc2::msg_send![inspector, show];
                    let _: () = objc2::msg_send![inspector, detach];
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = pw;
        }));
    });
}

/// Close (destroy) the invoking window's browser-tab webviews — used when the
/// panel unmounts (per-window, same scoping rationale as `browser_hide_all`).
#[tauri::command]
pub fn browser_close_all(app: tauri::AppHandle, window: tauri::Window) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        for (lbl, wv) in app.webviews() {
            if lbl.starts_with(PREFIX) && wv.window().label() == window.label() {
                set_shown(&lbl, None);
                let _ = wv.close();
            }
        }
    }));
}

/// Clear the in-app browser's cookies, cache and site storage — every tab of
/// every workspace (one shared store), signing the person out of the sites
/// they opened in it. Otto's own window is a different store and is never
/// touched; on macOS before 14 (no separate store) this is refused instead.
/// WebKit clears asynchronously; a page already loaded keeps its in-memory
/// state until it reloads.
#[tauri::command]
pub fn browser_clear_data(app: tauri::AppHandle, window: tauri::Window) -> Result<(), String> {
    if !own_data_store() {
        return Err(
            "Clearing browsing data needs macOS 14 or later — before it the browser \
                    shares its storage with Otto itself."
                .into(),
        );
    }
    // Every tab (and a helper still closing) shares the store: any one clears it all.
    if let Some((_, wv)) = app
        .webviews()
        .into_iter()
        .find(|(l, _)| l.starts_with(PREFIX) || l == CLEAR_LABEL)
    {
        return catch_unwind(AssertUnwindSafe(|| wv.clear_all_browsing_data()))
            .map_err(|_| "browser webview panicked while clearing".to_string())?
            .map_err(|e| e.to_string());
    }
    // No tab open: a hidden blank webview on the same store does it, then goes.
    let blank = tauri::Url::parse("about:blank").map_err(|e| e.to_string())?;
    let builder = with_browser_store(tauri::webview::WebviewBuilder::new(
        CLEAR_LABEL,
        WebviewUrl::External(blank),
    ));
    let wv = window
        .add_child(
            builder,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|e| e.to_string())?;
    let _ = wv.hide();
    let cleared = wv.clear_all_browsing_data().map_err(|e| e.to_string());
    // Let WebKit's removal start before the webview goes.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let _ = wv.close();
    });
    cleared
}

/// The hidden helper webview of [`browser_clear_data`] (not a tab: no `PREFIX`).
const CLEAR_LABEL: &str = "otto-clear-browser-data";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_store_is_its_own_and_needs_macos_14() {
        assert_eq!(BROWSER_DATA_STORE.len(), 16);
        assert!(!CLEAR_LABEL.starts_with(PREFIX));
        let plist = |v: &str| {
            format!("<dict>\n\t<key>ProductName</key>\n\t<string>macOS</string>\n\t<key>ProductVersion</key>\n\t<string>{v}</string>\n</dict>")
        };
        assert_eq!(macos_major(&plist("14.5")), Some(14));
        assert_eq!(macos_major(&plist("26.1")), Some(26));
        assert_eq!(macos_major(&plist("13.6.7")), Some(13));
        assert_eq!(macos_major("<dict></dict>"), None);
    }

    #[test]
    fn tabs_accept_only_web_urls() {
        for ok in [
            "https://example.com/",
            "http://127.0.0.1:3000/x",
            "about:blank",
        ] {
            assert!(parse_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "tauri://localhost/",
            "asset://localhost/x",
            "javascript:alert(1)",
            "data:text/html,<b>x</b>",
            "about:srcdoc",
            "otto://browser-new-tab",
            "vscode://file/x",
        ] {
            assert!(parse_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn open_navigates_only_to_a_url_the_webview_does_not_show() {
        let lbl = label("test-needs-navigation");
        let url = |u: &str| parse_url(u).unwrap();
        assert!(needs_navigation(&lbl, &url("https://a.example")));
        set_shown(&lbl, Some(&url("https://a.example")));
        // Normalised: the same page with and without the root slash.
        assert!(!needs_navigation(&lbl, &url("https://a.example/")));
        assert!(needs_navigation(&lbl, &url("https://a.example/next")));
        // The page navigated itself (on_navigation records it).
        set_shown(&lbl, Some(&url("https://a.example/next")));
        assert!(!needs_navigation(&lbl, &url("https://a.example/next")));
        // Closed: the next open creates and loads it again.
        set_shown(&lbl, None);
        assert!(needs_navigation(&lbl, &url("https://a.example/next")));
    }

    #[tokio::test]
    async fn eval_resolves_with_the_page_result() {
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        tx.send("\"ok\"".into()).unwrap();
        assert_eq!(
            await_eval_result(rx, EVAL_TIMEOUT).await.as_deref(),
            Ok("\"ok\"")
        );
    }

    #[tokio::test]
    async fn a_stalled_eval_times_out_instead_of_hanging() {
        let (_tx, rx) = tokio::sync::oneshot::channel::<String>();
        let err = await_eval_result(rx, std::time::Duration::from_millis(30))
            .await
            .unwrap_err();
        assert!(err.contains("timed out"), "{err}");
        assert_eq!(EVAL_TIMEOUT.as_secs(), 5);
    }

    #[tokio::test]
    async fn a_dropped_callback_is_an_error() {
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        drop(tx);
        assert!(await_eval_result(rx, EVAL_TIMEOUT).await.is_err());
    }
}
