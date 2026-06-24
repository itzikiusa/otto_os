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

use std::panic::{catch_unwind, AssertUnwindSafe};
use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

/// Label prefix for every browser-tab webview (`otto-browser-<tabId>`).
const PREFIX: &str = "otto-browser-";
const MAIN: &str = "main";

/// Emitted (payload = requested URL) when a tab asks to open a new window.
const NEW_TAB_EVENT: &str = "otto://browser-new-tab";
/// Emitted (payload = `[tabId, url]`) on every committed navigation in a tab.
const URL_EVENT: &str = "otto://browser-url";

fn label(id: &str) -> String {
    format!("{PREFIX}{id}")
}

fn parse_url(url: &str) -> Result<tauri::Url, String> {
    url.parse().map_err(|e| format!("bad url '{url}': {e}"))
}

/// Open (or re-navigate + reposition + show) the webview for tab `id` at the
/// given panel rect. Coordinates are logical (CSS) pixels relative to the
/// window's top-left, matching `getBoundingClientRect()` in the SPA.
#[tauri::command]
pub fn browser_open(
    app: tauri::AppHandle,
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
        return catch_unwind(AssertUnwindSafe(move || -> Result<(), String> {
            let _ = wv.set_position(LogicalPosition::new(x, y));
            let _ = wv.set_size(LogicalSize::new(width.max(1.0), height.max(1.0)));
            wv.navigate(parsed).map_err(|e| e.to_string())?;
            let _ = wv.show();
            Ok(())
        }))
        .unwrap_or_else(|_| Err("browser webview panicked while navigating".into()));
    }
    let window = app.get_window(MAIN).ok_or("main window not found")?;
    let app_nav = app.clone();
    let id_nav = id.clone();
    let app_new = app.clone();
    let builder = tauri::webview::WebviewBuilder::new(&lbl, WebviewUrl::External(parsed))
        .transparent(false)
        // Track in-page navigation for the address bar (never calls url()).
        .on_navigation(move |u: &tauri::Url| {
            let _ = app_nav.emit(URL_EVENT, (id_nav.clone(), u.to_string()));
            true
        })
        // Deny OS popups; ask the SPA to open a real in-app tab instead.
        .on_new_window(move |u: tauri::Url, _features: NewWindowFeatures| {
            let _ = app_new.emit(NEW_TAB_EVENT, u.to_string());
            NewWindowResponse::Deny
        });
    window
        .add_child(
            builder,
            LogicalPosition::new(x, y),
            LogicalSize::new(width.max(1.0), height.max(1.0)),
        )
        .map_err(|e| e.to_string())?;
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

/// Navigate tab `id`'s webview to a new URL.
#[tauri::command]
pub fn browser_navigate(app: tauri::AppHandle, id: String, url: String) -> Result<(), String> {
    if let Some(wv) = app.get_webview(&label(&id)) {
        let parsed = parse_url(&url)?;
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

/// Hide EVERY browser-tab webview — used when an SPA overlay (palette, modal,
/// context menu) opens over the panel, or when the Browser tab isn't visible.
#[tauri::command]
pub fn browser_hide_all(app: tauri::AppHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        for (lbl, wv) in app.webviews() {
            if lbl.starts_with(PREFIX) {
                let _ = wv.hide();
            }
        }
    }));
}

#[tauri::command]
pub fn browser_close(app: tauri::AppHandle, id: String) {
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

/// Close (destroy) every browser-tab webview — used when the panel unmounts.
#[tauri::command]
pub fn browser_close_all(app: tauri::AppHandle) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        for (lbl, wv) in app.webviews() {
            if lbl.starts_with(PREFIX) {
                let _ = wv.close();
            }
        }
    }));
}
