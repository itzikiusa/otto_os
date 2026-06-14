//! Native in-app browser: a child webview overlaid on the right panel's Browser
//! tab. Unlike an `<iframe>`, a real webview ignores `X-Frame-Options` /
//! CSP `frame-ancestors` and can load `http://localhost`, so it behaves like a
//! normal browser. The SPA drives it through these commands and keeps its bounds
//! in sync with the panel rect. Requires Tauri's `unstable` multiwebview feature.

use tauri::{LogicalPosition, LogicalSize, Manager, WebviewUrl};

/// Single reusable child webview label.
const LABEL: &str = "otto-browser";
const MAIN: &str = "main";

fn parse_url(url: &str) -> Result<tauri::Url, String> {
    url.parse().map_err(|e| format!("bad url '{url}': {e}"))
}

/// Open (or re-navigate + reposition + show) the browser webview at the given
/// panel rect. Coordinates are logical (CSS) pixels relative to the window's
/// top-left, matching `getBoundingClientRect()` in the SPA.
#[tauri::command]
pub fn browser_open(
    app: tauri::AppHandle,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let parsed = parse_url(&url)?;
    if let Some(wv) = app.get_webview(LABEL) {
        let _ = wv.set_position(LogicalPosition::new(x, y));
        let _ = wv.set_size(LogicalSize::new(width.max(1.0), height.max(1.0)));
        wv.navigate(parsed).map_err(|e| e.to_string())?;
        let _ = wv.show();
        return Ok(());
    }
    let window = app.get_window(MAIN).ok_or("main window not found")?;
    let builder = tauri::webview::WebviewBuilder::new(LABEL, WebviewUrl::External(parsed))
        .transparent(false);
    window
        .add_child(
            builder,
            LogicalPosition::new(x, y),
            LogicalSize::new(width.max(1.0), height.max(1.0)),
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Reposition/resize the browser webview to track the panel.
#[tauri::command]
pub fn browser_bounds(app: tauri::AppHandle, x: f64, y: f64, width: f64, height: f64) {
    if let Some(wv) = app.get_webview(LABEL) {
        let _ = wv.set_position(LogicalPosition::new(x, y));
        let _ = wv.set_size(LogicalSize::new(width.max(1.0), height.max(1.0)));
    }
}

/// Navigate the existing browser webview to a new URL.
#[tauri::command]
pub fn browser_navigate(app: tauri::AppHandle, url: String) -> Result<(), String> {
    if let Some(wv) = app.get_webview(LABEL) {
        wv.navigate(parse_url(&url)?).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Reload the current page.
#[tauri::command]
pub fn browser_reload(app: tauri::AppHandle) {
    if let Some(wv) = app.get_webview(LABEL) {
        let _ = wv.eval("location.reload()");
    }
}

#[tauri::command]
pub fn browser_show(app: tauri::AppHandle) {
    if let Some(wv) = app.get_webview(LABEL) {
        let _ = wv.show();
    }
}

#[tauri::command]
pub fn browser_hide(app: tauri::AppHandle) {
    if let Some(wv) = app.get_webview(LABEL) {
        let _ = wv.hide();
    }
}

#[tauri::command]
pub fn browser_close(app: tauri::AppHandle) {
    if let Some(wv) = app.get_webview(LABEL) {
        let _ = wv.close();
    }
}

/// Current URL of the browser webview (for the address bar to track link clicks).
#[tauri::command]
pub fn browser_current_url(app: tauri::AppHandle) -> Option<String> {
    app.get_webview(LABEL)
        .and_then(|wv| wv.url().ok())
        .map(|u| u.to_string())
}
