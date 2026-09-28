//! One live local side document per normal host. Only the side WKWebView is
//! reparented; primary detach changes the host's frame, preserving both DOMs.
use crate::panes_policy as policy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, PhysicalPosition,
    PhysicalSize, Webview, WebviewBuilder, WebviewUrl, Window, WindowBuilder,
};

// Only isolated debug examples install this managed state. It is deliberately
// absent from the IPC surface and release builds; fixture credentials never
// enter the installed app's persistent WKWebsiteDataStore.
#[cfg(debug_assertions)]
pub struct ProbeIsolation {
    pub initialization_script: String,
}

pub const PREFIX: &str = "otto-pane-window-";
static PAIRS: LazyLock<Mutex<HashMap<String, Pair>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
// Never hold PAIRS while dispatching native operations: their window events
// can re-enter Rust on the AppKit thread. Commands are serialized separately.
static ACTION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Attached,
    Side,
    Primary,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
impl Bounds {
    fn validate(&self) -> Result<(), String> {
        if policy::valid_bounds(self.x, self.y, self.width, self.height) {
            Ok(())
        } else {
            Err("Invalid pane bounds".into())
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneState {
    host: String,
    child: String,
    mode: Mode,
    always_on_top: bool,
    fullscreen: bool,
    visible: bool,
}
#[derive(Clone)]
struct Saved {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    fullscreen: bool,
    top: bool,
    maximized: bool,
    scale: f64,
}
#[derive(Clone)]
struct Pair {
    host: String,
    child: String,
    mode: Mode,
    bounds: Bounds,
    visible: bool,
    saved: Option<Saved>,
    always_on_top: bool,
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn local_url(url: &tauri::Url) -> bool {
    policy::local_document(url.scheme(),url.host_str(),url.path(),url.fragment())
        && url.port().is_none() && url.username().is_empty() && url.password().is_none()
}
fn local(view: &Webview) -> Result<(), String> {
    let url = view.url().map_err(|_| "Cannot validate pane caller".to_string())?;
    if local_url(&url) { Ok(()) } else { Err("Panes require an Otto local document".into()) }
}

fn pair_for(view: &Webview) -> Result<Pair, String> {
    local(view)?;
    PAIRS
        .lock()
        .unwrap()
        .values()
        .find(|p| policy::owns(&p.host, &p.child, view.label()))
        .cloned()
        .ok_or_else(|| "No pane belongs to this document".into())
}
fn host_only(view: &Webview, pair: &Pair) -> Result<(), String> {
    if view.label() == pair.host {
        Ok(())
    } else {
        Err("This pane action belongs to its host".into())
    }
}
fn host_window(app: &AppHandle, pair: &Pair) -> Result<Window, String> {
    app.get_window(&pair.host)
        .ok_or_else(|| "Pane host is closed".into())
}
fn child_view(app: &AppHandle, pair: &Pair) -> Result<Webview, String> {
    app.get_webview(&pair.child)
        .ok_or_else(|| "Pane document is closed".into())
}
fn companion_label(pair: &Pair) -> String {
    format!("{PREFIX}{}", pair.host)
}
fn detached_window(app: &AppHandle, pair: &Pair) -> Result<Window, String> {
    match pair.mode {
        Mode::Attached => Err("Detach a pane first".into()),
        Mode::Primary => host_window(app, pair),
        Mode::Side => app
            .get_window(&companion_label(pair))
            .ok_or_else(|| "Detached pane is closed".into()),
    }
}
fn state(app: &AppHandle, pair: &Pair) -> PaneState {
    let win = detached_window(app, pair).ok();
    PaneState {
        host: pair.host.clone(),
        child: pair.child.clone(),
        mode: pair.mode,
        always_on_top: pair.mode != Mode::Attached && pair.always_on_top,
        fullscreen: win
            .as_ref()
            .is_some_and(|w| w.is_fullscreen().unwrap_or(false)),
        visible: pair.mode != Mode::Attached || pair.visible,
    }
}
fn emit(app: &AppHandle, pair: &Pair) -> PaneState {
    let value = state(app, pair);
    for label in [&pair.host, &pair.child] {
        let _ = app.emit_to(EventTarget::webview(label), "otto://pane-state", &value);
    }
    value
}
fn put(pair: &Pair) {
    PAIRS
        .lock()
        .unwrap()
        .insert(pair.host.clone(), pair.clone());
}
/// On macOS Tauri 2.11 reports the first webview's bounds as inner_size
/// after reparent into a Window built without children. Recover the actual
/// decorated content rectangle from native inner/outer positions instead.
fn content_size(win: &Window) -> Result<PhysicalSize<u32>, String> {
    if !win.label().starts_with(PREFIX) { return win.inner_size().map_err(err); }
    let outer=win.outer_size().map_err(err)?;
    let origin=win.outer_position().map_err(err)?;
    let content=win.inner_position().map_err(err)?;
    let border=(content.x-origin.x).max(0) as u32;
    let top=(content.y-origin.y).max(0) as u32;
    Ok(PhysicalSize::new(outer.width.saturating_sub(border*2).max(1),outer.height.saturating_sub(top+border).max(1)))
}
fn layout_child(app: &AppHandle, pair: &Pair) -> Result<(), String> {
    let child = child_view(app, pair)?;
    child.set_auto_resize(false).map_err(err)?;
    if pair.mode == Mode::Attached {
        child
            .set_position(LogicalPosition::new(pair.bounds.x, pair.bounds.y))
            .map_err(err)?;
        child
            .set_size(LogicalSize::new(pair.bounds.width, pair.bounds.height))
            .map_err(err)?;
        if pair.visible {
            child.show().map_err(err)?;
        } else {
            child.hide().map_err(err)?;
        }
    } else {
        let win = child.window();
        child
            .set_position(LogicalPosition::new(0.0, 0.0))
            .map_err(err)?;
        child
            .set_size(content_size(&win)?)
            .map_err(err)?;
        child.show().map_err(err)?;
    }
    Ok(())
}
fn route_ok(route: &str) -> bool {
    crate::popout::route_ok(route)
        && !matches!(route.split(['/', '?']).next(), Some("room" | "snip"))
        && !route.split('/').next().unwrap_or("").contains('%')
}
#[tauri::command]
pub async fn pane_open(
    app: AppHandle,
    webview: Webview,
    route: String,
    bounds: Bounds,
) -> Result<PaneState, String> {
    let _guard = ACTION.lock().await;
    local(&webview)?;
    if !policy::host_label(webview.label()) || webview.window().label() != webview.label() {
        return Err("Only a normal Otto host can open a pane".into());
    }
    bounds.validate()?;
    if !route_ok(&route) {
        return Err("Invalid pane route".into());
    }
    let existing = PAIRS.lock().unwrap().get(webview.label()).cloned();
    if let Some(pair) = existing {
        return Ok(state(&app, &pair));
    }
    let host = webview.label().to_string();
    let child = format!("otto-pane-{host}");
    let pair = Pair {
        host: host.clone(),
        child: child.clone(),
        mode: Mode::Attached,
        bounds,
        visible: true,
        saved: None,
        always_on_top: false,
    };
    let init = format!(
        "window.__OTTO_NATIVE_PANE__={{host:{}}};window.__OTTO_WIN__={};",
        serde_json::to_string(&host).unwrap(),
        serde_json::to_string(&host).unwrap()
    );
    let builder = WebviewBuilder::new(
        &child,
        WebviewUrl::App(format!("index.html?embed=1#/{route}").into()),
    )
    .initialization_script(init)
    .disable_drag_drop_handler()
    .on_navigation(local_url)
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny);
    #[cfg(debug_assertions)]
    let builder = if let Some(probe) = app.try_state::<ProbeIsolation>() {
        builder
            .incognito(true)
            .initialization_script(&probe.initialization_script)
    } else {
        builder
    };
    // Register before loading: the guest's ready command must find its owner.
    put(&pair);
    if let Err(e) = webview.window().add_child(
        builder,
        LogicalPosition::new(pair.bounds.x, pair.bounds.y),
        LogicalSize::new(pair.bounds.width, pair.bounds.height),
    ) {
        PAIRS.lock().unwrap().remove(&host);
        return Err(err(e));
    }
    Ok(emit(&app, &pair))
}
#[tauri::command]
pub async fn pane_layout(
    app: AppHandle,
    webview: Webview,
    bounds: Bounds,
    visible: bool,
) -> Result<(), String> {
    let _guard = ACTION.lock().await;
    let mut pair = pair_for(&webview)?;
    host_only(&webview, &pair)?;
    bounds.validate()?;
    let changed = pair.visible != visible;
    pair.bounds = bounds;
    pair.visible = visible;
    if pair.mode == Mode::Attached {
        layout_child(&app, &pair)?;
    }
    put(&pair);
    if changed {
        emit(&app, &pair);
    }
    Ok(())
}
#[tauri::command]
pub async fn pane_state(app: AppHandle, webview: Webview) -> Result<Option<PaneState>, String> {
    local(&webview)?;
    if !policy::host_label(webview.label())
        && !PAIRS
            .lock()
            .unwrap()
            .values()
            .any(|p| p.child == webview.label())
    {
        return Err("Not a pane owner".into());
    }
    Ok(pair_for(&webview).ok().map(|p| state(&app, &p)))
}
fn text_field(v: &Value, key: &str, max: usize) -> bool {
    v.get(key)
        .and_then(Value::as_str)
        .is_some_and(|s| s.len() <= max)
}
fn relay_ok(message: &Value, guest: bool) -> bool {
    if message.get("ns").and_then(Value::as_str) != Some("otto-side")
        || serde_json::to_vec(message).map_or(true, |v| v.len() > 2_097_152)
    {
        return false;
    }
    let Some(kind) = message.get("type").and_then(Value::as_str) else {
        return false;
    };
    match (guest, kind) {
        (true, "ready" | "route" | "open-in-main") | (false, "navigate") => message
            .get("route")
            .and_then(Value::as_str)
            .is_some_and(route_ok),
        (true, "close" | "swap" | "promote" | "focus") => true,
        (true, "key") => {
            text_field(message, "action", 64)
                && message
                    .get("index")
                    .is_none_or(|v| v.as_i64().is_some_and(|n| (0..=10000).contains(&n)))
        }
        (_, "workspace") | (false, "run-command") => text_field(message, "id", 256),
        (false, "host") => {
            text_field(message, "primary", 256)
                && message.get("padTraffic").is_some_and(Value::is_boolean)
        }
        (false, "menu") => text_field(message, "id", 64),
        (true, "commands") => message
            .get("list")
            .and_then(Value::as_array)
            .is_some_and(|list| {
                list.len() <= 2000
                    && list.iter().all(|c| {
                        text_field(c, "id", 256)
                            && text_field(c, "title", 256)
                            && ["group", "detail", "keywords", "shortcut"].iter().all(|k| {
                                c.get(k)
                                    .is_none_or(|v| v.as_str().is_some_and(|s| s.len() <= 512))
                            })
                    })
            }),
        (true, "open-external") => message.get("url").and_then(Value::as_str).is_some_and(|s| {
            s.len() <= 2048
                && s.parse::<tauri::Url>()
                    .is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
        }),
        _ => false,
    }
}
#[tauri::command]
pub async fn pane_to_host(app: AppHandle, webview: Webview, message: Value) -> Result<(), String> {
    let pair = pair_for(&webview)?;
    if webview.label() != pair.child || !relay_ok(&message, true) {
        return Err("Invalid pane guest message".into());
    }
    app.emit_to(
        EventTarget::webview(&pair.host),
        "otto://pane-to-host",
        message,
    )
    .map_err(err)
}
#[tauri::command]
pub async fn pane_to_guest(app: AppHandle, webview: Webview, message: Value) -> Result<(), String> {
    let pair = pair_for(&webview)?;
    host_only(&webview, &pair)?;
    if !relay_ok(&message, false) {
        return Err("Invalid pane host message".into());
    }
    app.emit_to(
        EventTarget::webview(&pair.child),
        "otto://pane-to-guest",
        message,
    )
    .map_err(err)
}
fn fit_window(
    app: &AppHandle,
    win: &Window,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    source_scale: f64,
) -> Result<(), String> {
    let position = position.to_logical::<f64>(source_scale);
    let size = size.to_logical::<f64>(source_scale);
    let mut frame = crate::windows::WinFrame {
        label: win.label().into(),
        x: position.x.round() as i32,
        y: position.y.round() as i32,
        w: size.width.round() as u32,
        h: size.height.round() as u32,
        fullscreen: false,
    };
    let monitors = app
        .available_monitors()
        .map_err(err)?
        .iter()
        .map(|m| {
            let area = m.work_area();
            let p = area.position.to_logical::<f64>(m.scale_factor());
            let s = area.size.to_logical::<f64>(m.scale_factor());
            (
                p.x.round() as i32,
                p.y.round() as i32,
                s.width.round() as u32,
                s.height.round() as u32,
            )
        })
        .collect::<Vec<_>>();
    crate::windows::clamp_frame(&mut frame, &monitors);
    crate::windows::fit_frame(&mut frame, &monitors);
    win.set_position(LogicalPosition::new(frame.x as f64, frame.y as f64))
        .map_err(err)?;
    win.set_size(LogicalSize::new(frame.w as f64, frame.h as f64))
        .map_err(err)
}
fn restore_host(app: &AppHandle, pair: &Pair) -> Result<(), String> {
    if let Some(saved) = &pair.saved {
        let host = host_window(app, pair)?;
        host.set_fullscreen(false).map_err(err)?;
        host.unmaximize().map_err(err)?;
        fit_window(app, &host, saved.position, saved.size, saved.scale)?;
        host.set_always_on_top(saved.top).map_err(err)?;
        host.set_min_size(Some(LogicalSize::new(980.0, 640.0)))
            .map_err(err)?;
        if saved.maximized {
            host.maximize().map_err(err)?;
        }
        if saved.fullscreen {
            host.set_fullscreen(true).map_err(err)?;
        }
    }
    Ok(())
}
fn return_pair(app: &AppHandle, pair: &mut Pair) -> Result<(), String> {
    if pair.mode == Mode::Attached {
        let layout = layout_child(app, pair);
        if let Some(empty) = app.get_window(&companion_label(pair)) {
            empty.destroy().map_err(err)?;
        }
        return layout;
    }
    let child = child_view(app, pair)?;
    let companion = app.get_window(&companion_label(pair));
    if let Some(w) = &companion {
        w.set_fullscreen(false).map_err(err)?;
    }
    child.reparent(&host_window(app, pair)?).map_err(err)?;
    if let Err(e) = restore_host(app, pair) {
        if let Some(w) = &companion {
            let _ = child.reparent(w);
        }
        return Err(e);
    }
    pair.mode = Mode::Attached;
    pair.always_on_top = false;
    pair.saved = None;
    put(pair);
    let layout_result = layout_child(app, pair);
    let close_result = companion
        .map(|w| w.destroy().map_err(err))
        .unwrap_or(Ok(()));
    emit(app, pair);
    layout_result.and(close_result)
}
#[tauri::command]
pub async fn pane_detach(
    app: AppHandle,
    webview: Webview,
    pane: Mode,
) -> Result<PaneState, String> {
    let _guard = ACTION.lock().await;
    let mut pair = pair_for(&webview)?;
    if pane == Mode::Attached {
        return Err("Choose side or primary pane".into());
    }
    if pair.mode == pane {
        return Ok(state(&app, &pair));
    }
    if pair.mode != Mode::Attached {
        return Err("Return the detached pane first".into());
    }
    let host = host_window(&app, &pair)?;
    let saved = Saved {
        position: host.outer_position().map_err(err)?,
        size: host.inner_size().map_err(err)?,
        fullscreen: host.is_fullscreen().map_err(err)?,
        top: host.is_always_on_top().map_err(err)?,
        maximized: host.is_maximized().map_err(err)?,
        scale: host.scale_factor().map_err(err)?,
    };
    pair.mode = pane;
    pair.saved = Some(saved.clone());
    // Publish ownership before creation or any native mutation delivers close.
    put(&pair);
    let companion = match WindowBuilder::new(&app, companion_label(&pair))
        .title("Otto — Side pane")
        .inner_size(900.0, 680.0)
        .min_inner_size(360.0, 300.0)
        .visible(false)
        .build()
    {
        Ok(window) => window,
        Err(e) => {
            pair.mode = Mode::Attached;
            pair.saved = None;
            put(&pair);
            return Err(err(e));
        }
    };
    if let Err(e) = child_view(&app, &pair)?.reparent(&companion) {
        let rollback = return_pair(&app, &mut pair);
        return Err(match rollback {
            Ok(()) => err(e),
            Err(recovery) => format!("{e}; return failed: {recovery}"),
        });
    }
    let transition = (|| -> Result<(), String> {
        if pane == Mode::Primary {
            fit_window(&app, &companion, saved.position, saved.size, saved.scale)?;
            host.set_fullscreen(false).map_err(err)?;
            host.unmaximize().map_err(err)?;
            host.set_min_size(Some(LogicalSize::new(480.0, 360.0)))
                .map_err(err)?;
            let scale = host.scale_factor().map_err(err)?;
            fit_window(
                &app,
                &host,
                PhysicalPosition::new(saved.position.x + 48, saved.position.y + 48),
                PhysicalSize::new((900.0 * scale) as u32, (680.0 * scale) as u32),
                saved.scale,
            )?;
        } else {
            fit_window(
                &app,
                &companion,
                PhysicalPosition::new(saved.position.x + 48, saved.position.y + 48),
                PhysicalSize::new((900.0 * saved.scale) as u32, (680.0 * saved.scale) as u32),
                saved.scale,
            )?;
        }
        layout_child(&app, &pair)?;
        companion.show().map_err(err)?;
        detached_window(&app, &pair)?.set_focus().map_err(err)?;
        Ok(())
    })();
    if let Err(e) = transition {
        let rollback = return_pair(&app, &mut pair);
        return Err(match rollback {
            Ok(()) => e,
            Err(recovery) => format!("{e}; return failed: {recovery}"),
        });
    }
    put(&pair);
    Ok(emit(&app, &pair))
}
#[tauri::command]
pub async fn pane_return(app: AppHandle, webview: Webview) -> Result<PaneState, String> {
    let _guard = ACTION.lock().await;
    let mut pair = pair_for(&webview)?;
    return_pair(&app, &mut pair)?;
    Ok(state(&app, &pair))
}
#[tauri::command]
pub async fn pane_focus(app: AppHandle, webview: Webview, pane: Mode) -> Result<(), String> {
    let _guard = ACTION.lock().await;
    let pair = pair_for(&webview)?;
    let target = match pane {
        Mode::Side => child_view(&app, &pair)?,
        Mode::Primary => app.get_webview(&pair.host).ok_or("Pane host is closed")?,
        Mode::Attached => return Err("Choose side or primary pane".into()),
    };
    target.window().unminimize().map_err(err)?;
    target.window().set_focus().map_err(err)?;
    target.set_focus().map_err(err)
}
#[derive(Serialize)]
pub struct PaneMonitor {
    id: usize,
    name: String,
    current: bool,
}
#[tauri::command]
pub async fn pane_monitors(app: AppHandle, webview: Webview) -> Result<Vec<PaneMonitor>, String> {
    let pair = pair_for(&webview)?;
    let win = detached_window(&app, &pair).or_else(|_| host_window(&app, &pair))?;
    let current = win.current_monitor().map_err(err)?;
    Ok(app
        .available_monitors()
        .map_err(err)?
        .iter()
        .enumerate()
        .map(|(id, m)| PaneMonitor {
            id,
            name: m
                .name()
                .cloned()
                .unwrap_or_else(|| format!("Display {}", id + 1)),
            current: current
                .as_ref()
                .is_some_and(|c| c.position() == m.position()),
        })
        .collect())
}
#[tauri::command]
pub async fn pane_window_action(
    app: AppHandle,
    webview: Webview,
    action: String,
    monitor: Option<usize>,
) -> Result<PaneState, String> {
    let _guard = ACTION.lock().await;
    let mut pair = pair_for(&webview)?;
    let win = detached_window(&app, &pair)?;
    match action.as_str() {
        "toggle-top" => {
            // Tao schedules the NSWindow level asynchronously. Remember the
            // successful request instead of immediately reading the old level.
            let next = !pair.always_on_top;
            win.set_always_on_top(next).map_err(err)?;
            pair.always_on_top = next;
            put(&pair);
        }
        "toggle-fullscreen" => win
            .set_fullscreen(!win.is_fullscreen().map_err(err)?)
            .map_err(err)?,
        "maximize" => {
            win.set_fullscreen(false).map_err(err)?;
            if win.is_maximized().map_err(err)? {
                win.unmaximize().map_err(err)?
            } else {
                win.maximize().map_err(err)?
            }
        }
        "move-display" => {
            let monitors = app.available_monitors().map_err(err)?;
            let m = monitors
                .get(monitor.ok_or("Choose a display")?)
                .ok_or("Display is no longer connected")?;
            win.set_fullscreen(false).map_err(err)?;
            win.unmaximize().map_err(err)?;
            // macOS interprets position/size using the window's CURRENT
            // display scale. Use global logical coordinates for cross-display
            // moves, otherwise a 2x→1x move lands at half the requested x/y.
            let current_scale = win.scale_factor().map_err(err)?;
            let size = content_size(&win)?.to_logical::<f64>(current_scale);
            let outer = win
                .outer_size()
                .map_err(err)?
                .to_logical::<f64>(current_scale);
            let area = m.work_area();
            let screen = area.size.to_logical::<f64>(m.scale_factor());
            let chrome = (outer.height - size.height).max(0.0);
            win.set_size(LogicalSize::new(
                size.width.min(screen.width),
                size.height.min((screen.height - chrome).max(1.0)),
            ))
            .map_err(err)?;
            win.set_position(area.position.to_logical::<f64>(m.scale_factor()))
                .map_err(err)?;
        }
        _ => return Err("Unknown pane window action".into()),
    }
    Ok(emit(&app, &pair))
}
#[tauri::command]
pub async fn pane_close(app: AppHandle, webview: Webview) -> Result<(), String> {
    let _guard = ACTION.lock().await;
    let mut pair = pair_for(&webview)?;
    host_only(&webview, &pair)?;
    return_pair(&app, &mut pair)?;
    child_view(&app, &pair)?.close().map_err(err)?;
    PAIRS.lock().unwrap().remove(&pair.host);
    app.emit_to(
        EventTarget::webview(&pair.host),
        "otto://pane-state",
        Value::Null,
    )
    .map_err(err)
}
/// Close of either detached surface means Return. Event callback must not block
/// AppKit while a command is waiting for a native operation on the same thread.
pub fn intercept_close(app: &AppHandle, label: &str) -> bool {
    if crate::windows::is_quitting() {
        return false;
    }
    let pair = PAIRS
        .lock()
        .unwrap()
        .values()
        .find(|p| p.mode != Mode::Attached && (p.host == label || companion_label(p) == label))
        .cloned();
    if let Some(mut pair) = pair {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _guard = ACTION.lock().await;
            if let Some(current) = PAIRS.lock().unwrap().get(&pair.host).cloned() {
                pair = current;
            }
            if let Err(e) = return_pair(&app, &mut pair) {
                eprintln!("Could not return pane: {e}");
            }
        });
        true
    } else {
        false
    }
}
pub fn resized(app: &AppHandle, label: &str) {
    let pair = PAIRS
        .lock()
        .unwrap()
        .values()
        .find(|p| companion_label(p) == label || p.host == label)
        .cloned();
    if let Some(pair) = pair {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _guard = ACTION.lock().await;
            let current = PAIRS.lock().unwrap().get(&pair.host).cloned();
            if let Some(pair) = current {
                let _ = layout_child(&app, &pair);
                emit(&app, &pair);
            }
        });
    }
}
pub fn destroyed(app: &AppHandle, label: &str) {
    let pair = PAIRS.lock().unwrap().remove(label);
    if let Some(pair) = pair {
        if let Some(win) = app.get_window(&companion_label(&pair)) {
            let _ = win.destroy();
        }
    }
}
/// Persist original host geometry during a detach, including direct OS Quit.
/// This avoids dispatching reparent synchronously inside AppKit exit callbacks.
pub fn original_frame(label: &str) -> Option<crate::windows::WinFrame> {
    let pairs = PAIRS.lock().unwrap();
    let p = pairs.get(label)?;
    let s = p.saved.as_ref()?;
    Some(crate::windows::WinFrame {
        label: label.into(),
        x: s.position.x,
        y: s.position.y,
        w: s.size.width,
        h: s.size.height,
        fullscreen: s.fullscreen,
    })
}
/// Map a companion's physical window back to its one logical host. For an
/// attached child, its focus message already tracks which pane owns the menu.
pub fn menu_host(app: &AppHandle, label: &str) -> String {
    let pair = PAIRS
        .lock()
        .unwrap()
        .values()
        .find(|p| companion_label(p) == label)
        .cloned();
    if let Some(pair) = pair {
        let _ = app.emit_to(
            EventTarget::webview(&pair.host),
            "otto://pane-to-host",
            json!({"ns":"otto-side","type":"focus"}),
        );
        pair.host
    } else {
        label.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_urls_reject_remote_ports_credentials_and_guest_routes() {
        assert!(local_url(&"tauri://localhost/index.html?embed=1#/agents".parse().unwrap()));
        for url in ["https://tauri.localhost:4443/", "tauri://user@localhost/", "tauri://localhost/#/room/guest", "https://example.com/"] {
            assert!(!local_url(&url.parse().unwrap()), "{url}");
        }
    }
    #[test]
    fn relay_direction_size_and_shape_are_enforced() {
        assert!(relay_ok(
            &json!({"ns":"otto-side","type":"ready","route":"agents"}),
            true
        ));
        assert!(!relay_ok(
            &json!({"ns":"otto-side","type":"navigate","route":"agents"}),
            true
        ));
        assert!(!relay_ok(
            &json!({"ns":"otto-side","type":"ready","route":"room/x"}),
            true
        ));
        assert!(!relay_ok(
            &json!({"ns":"otto-side","type":"commands","list":[{"id":"x"}]}),
            true
        ));
        assert!(!relay_ok(
            &json!({"ns":"otto-side","type":"key","action":"x","index":-1}),
            true
        ));
        assert!(!relay_ok(
            &json!({"ns":"otto-side","type":"open-external","url":"file:///etc/passwd"}),
            true
        ));
        assert!(!relay_ok(
            &json!({"ns":"otto-side","type":"workspace","id":"x".repeat(257)}),
            false
        ));
    }
}
