//! The assistant bar: a floating glass pill summoned over any app by the
//! `assistant` global shortcut (⌥Space by default).
//!
//! One hidden window (`otto-bar`) is built at startup so the first summon is
//! instant. It loads the SPA's chromeless `#/bar` route (FloatingBar.svelte),
//! is transparent + undecorated + always on top, converted to a
//! non-activating `NSPanel` (panel.rs) with native HUD vibrancy behind the
//! page, and joins every Space. Geometry is owned HERE, not by CSS:
//!   * it opens horizontally centred near the bottom of the screen under the
//!     cursor (the "active" screen);
//!   * the page reports its content height via `bar_resize`, and the window
//!     grows UPWARD from that bottom anchor (the thread peeks above the pill),
//!     capped at `MAX_H` and clamped to the screen's work area;
//!   * `Esc` in the page calls `bar_hide`; a second ⌥Space also hides.
//!
//! Events to the bar page: `otto://bar-shown` (focus the input),
//! `otto://bar-hidden`, and `otto://assistant-voice` `{pressed: bool}` for the
//! hold-to-talk chord.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
};

pub const LABEL: &str = "otto-bar";

/// Default / min / max geometry, logical points. The pill itself is at most
/// 720 wide (layout.md §7); the window adds no margin around it.
const DEFAULT_W: f64 = 680.0;
const MIN_W: f64 = 360.0;
const MAX_W: f64 = 720.0;
/// Collapsed height: the pill alone.
pub const PILL_H: f64 = 56.0;
const MIN_H: f64 = 40.0;
/// Pill + compact thread, before the screen clamp.
const MAX_H: f64 = 560.0;
/// Gap between the pill's bottom edge and the bottom of the work area (above
/// the Dock when it's visible).
const BOTTOM_GAP: f64 = 96.0;
/// Keep this much of the work area free above an expanded bar.
const TOP_GAP: f64 = 24.0;

/// Where the bar sits while shown: the bottom-centre anchor and the work area
/// it must stay inside (all logical points, desktop coordinates).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchor {
    center_x: f64,
    bottom: f64,
    area_top: f64,
    area_left: f64,
    area_right: f64,
}

#[derive(Clone, Copy, Debug)]
struct State {
    anchor: Option<Anchor>,
    width: f64,
    height: f64,
    radius: f64,
}

static STATE: Mutex<State> = Mutex::new(State {
    anchor: None,
    width: DEFAULT_W,
    height: PILL_H,
    radius: PILL_H / 2.0,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Build the hidden bar window (startup). Failure is non-fatal: the shortcut
/// then just logs, and every in-app entry point still works.
pub fn init(app: &AppHandle) {
    if let Err(e) = build(app) {
        eprintln!("assistant bar unavailable: {e}");
    }
}

fn build(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Ok(w);
    }
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html#/bar".into()))
        .title("Otto Assistant")
        .inner_size(DEFAULT_W, PILL_H)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .accept_first_mouse(true)
        .disable_drag_drop_handler()
        .initialization_script(format!("window.__OTTO_WIN__='{LABEL}';"))
        .build()?;
    let for_main = win.clone();
    let _ = app.run_on_main_thread(move || {
        crate::panel::make_nonactivating_panel(&for_main);
        apply_glass(&for_main, PILL_H / 2.0);
    });
    Ok(win)
}

/// HUD vibrancy behind the (transparent) page, rounded to `radius`. Re-applied
/// when the shape changes: the pill is fully round, the expanded thread uses a
/// smaller radius. State `Active` keeps the material lit although a
/// non-activating panel never makes Otto the active app.
fn apply_glass(win: &tauri::WebviewWindow, radius: f64) {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{
            apply_vibrancy, clear_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
        };
        let _ = clear_vibrancy(win);
        let _ = apply_vibrancy(
            win,
            NSVisualEffectMaterial::HudWindow,
            Some(NSVisualEffectState::Active),
            Some(radius),
        );
    }
}

/// Bottom-centre anchor on the screen under the cursor (else the primary).
fn anchor_for_cursor(app: &AppHandle) -> Option<Anchor> {
    let monitors = app.available_monitors().ok()?;
    let cursor = app.cursor_position().ok();
    let monitor = cursor
        .and_then(|c| {
            monitors.iter().find(|m| {
                let p = m.position();
                let s = m.size();
                c.x >= p.x as f64
                    && c.x < p.x as f64 + s.width as f64
                    && c.y >= p.y as f64
                    && c.y < p.y as f64 + s.height as f64
            })
        })
        .or_else(|| monitors.first())?;
    let scale = monitor.scale_factor();
    let wa = monitor.work_area();
    let left = wa.position.x as f64 / scale;
    let top = wa.position.y as f64 / scale;
    let width = wa.size.width as f64 / scale;
    let height = wa.size.height as f64 / scale;
    Some(Anchor {
        center_x: left + width / 2.0,
        bottom: top + height - BOTTOM_GAP,
        area_top: top,
        area_left: left,
        area_right: left + width,
    })
}

/// Fit a requested (w, h) to the anchor's screen: width into [MIN_W, MAX_W]
/// and the screen width; height into [MIN_H, MAX_H] and the room above the
/// anchor. Returns the frame's (x, y, w, h), top-left origin.
fn fit(anchor: &Anchor, w: f64, h: f64) -> (f64, f64, f64, f64) {
    let screen_w = (anchor.area_right - anchor.area_left - 16.0).max(MIN_W);
    let w = w.clamp(MIN_W, MAX_W).min(screen_w);
    let room = (anchor.bottom - anchor.area_top - TOP_GAP).max(MIN_H);
    let h = h.clamp(MIN_H, MAX_H).min(room);
    let x = (anchor.center_x - w / 2.0)
        .max(anchor.area_left + 8.0)
        .min(anchor.area_right - 8.0 - w);
    (x, anchor.bottom - h, w, h)
}

fn place(win: &tauri::WebviewWindow, frame: (f64, f64, f64, f64)) {
    let (x, y, w, h) = frame;
    // Size first: `setContentSize:` keeps the bottom-left origin, then the
    // top-left move lands the frame exactly — both are queued on the main
    // thread in this order.
    let _ = win.set_size(LogicalSize::new(w, h));
    let _ = win.set_position(LogicalPosition::new(x, y));
}

#[derive(Clone, Serialize)]
struct Shown {
    width: f64,
    height: f64,
}

pub fn show(app: &AppHandle) {
    let Ok(win) = build(app) else { return };
    let anchor = anchor_for_cursor(app);
    let (w, h) = {
        let mut st = state();
        st.anchor = anchor.or(st.anchor);
        (st.width, st.height)
    };
    if let Some(a) = anchor {
        place(&win, fit(&a, w, h));
    }
    let for_main = win.clone();
    let _ = app.run_on_main_thread(move || crate::panel::show_without_activation(&for_main));
    let _ = app.emit_to(
        LABEL,
        "otto://bar-shown",
        Shown {
            width: w,
            height: h,
        },
    );
    // The popover and the bar never stack: summoning one dismisses the other.
    crate::tray::hide_popover(app);
}

pub fn hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LABEL) {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
            let _ = app.emit_to(LABEL, "otto://bar-hidden", ());
        }
    }
}

pub fn is_visible(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// ⌥Space: a second press hides (Spotlight semantics).
pub fn toggle(app: &AppHandle) {
    if is_visible(app) {
        hide(app);
    } else {
        show(app);
    }
}

#[derive(Clone, Serialize)]
struct Voice {
    pressed: bool,
}

/// Hold-to-talk chord: show the bar on press, then forward press/release to
/// the page (the voice helper arrives in a later PR; the page decides).
pub fn voice(app: &AppHandle, pressed: bool) {
    if pressed && !is_visible(app) {
        show(app);
    }
    let _ = app.emit_to(LABEL, "otto://assistant-voice", Voice { pressed });
}

// ---- commands (the bar page + in-app entry points) ----

#[tauri::command]
pub fn bar_show(app: AppHandle) {
    show(&app);
}

#[tauri::command]
pub fn bar_hide(app: AppHandle) {
    hide(&app);
}

#[tauri::command]
pub fn bar_toggle(app: AppHandle) {
    toggle(&app);
}

/// The applied frame after clamping, so the page can tell it was capped.
#[derive(Serialize)]
pub struct BarFrame {
    pub width: f64,
    pub height: f64,
    pub radius: f64,
}

/// Resize the bar to its content, growing upward from the bottom anchor.
/// `height`/`width` are logical px (CSS px); `radius` reshapes the glass
/// (default: fully round while at pill height, 18 when expanded).
#[tauri::command]
pub fn bar_resize(
    app: AppHandle,
    height: f64,
    width: Option<f64>,
    radius: Option<f64>,
) -> Result<BarFrame, String> {
    if !height.is_finite() || width.is_some_and(|w| !w.is_finite()) {
        return Err("bar size must be finite".into());
    }
    let win = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "assistant bar window is not available".to_string())?;
    let (anchor, prev_radius, w_req) = {
        let st = state();
        (st.anchor, st.radius, width.unwrap_or(st.width))
    };
    let anchor = match anchor.or_else(|| anchor_for_cursor(&app)) {
        Some(a) => a,
        None => return Err("no screen to place the bar on".into()),
    };
    let (x, y, w, h) = fit(&anchor, w_req, height);
    let r = radius
        .filter(|r| r.is_finite())
        .map(|r| r.clamp(0.0, h / 2.0))
        .unwrap_or(if h <= PILL_H + 0.5 { h / 2.0 } else { 18.0 });
    {
        let mut st = state();
        st.anchor = Some(anchor);
        st.width = w;
        st.height = h;
        st.radius = r;
    }
    place(&win, (x, y, w, h));
    if (r - prev_radius).abs() > 0.5 {
        let for_main = win.clone();
        let _ = app.run_on_main_thread(move || apply_glass(&for_main, r));
    }
    Ok(BarFrame {
        width: w,
        height: h,
        radius: r,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> Anchor {
        // 1512×944 work area at the origin (MacBook-ish), bottom gap applied.
        Anchor {
            center_x: 756.0,
            bottom: 944.0 - BOTTOM_GAP,
            area_top: 0.0,
            area_left: 0.0,
            area_right: 1512.0,
        }
    }

    #[test]
    fn fit_centres_and_grows_upward_from_the_anchor() {
        let a = anchor();
        let (x, y, w, h) = fit(&a, DEFAULT_W, PILL_H);
        assert_eq!((w, h), (DEFAULT_W, PILL_H));
        assert_eq!(x, 756.0 - DEFAULT_W / 2.0);
        assert_eq!(y + h, a.bottom, "bottom edge pinned to the anchor");
        let (_, y2, _, h2) = fit(&a, DEFAULT_W, 400.0);
        assert_eq!(h2, 400.0);
        assert_eq!(y2 + h2, a.bottom, "expanding keeps the bottom edge");
    }

    #[test]
    fn fit_caps_height_and_width() {
        let a = anchor();
        let (_, _, w, h) = fit(&a, 5000.0, 5000.0);
        assert_eq!(w, MAX_W);
        assert_eq!(h, MAX_H);
        let (_, _, w, h) = fit(&a, 10.0, 1.0);
        assert_eq!((w, h), (MIN_W, MIN_H));
    }

    #[test]
    fn fit_clamps_to_a_short_screen() {
        // A 400pt-tall work area: the bar may not climb past its top.
        let a = Anchor {
            center_x: 400.0,
            bottom: 400.0 - BOTTOM_GAP,
            area_top: 0.0,
            area_left: 0.0,
            area_right: 800.0,
        };
        let (_, y, _, h) = fit(&a, DEFAULT_W, MAX_H);
        assert!(
            y >= a.area_top + TOP_GAP - 0.001,
            "top edge stays on screen"
        );
        assert_eq!(y + h, a.bottom);
    }

    #[test]
    fn fit_keeps_a_narrow_screen_fully_visible() {
        let a = Anchor {
            center_x: 200.0,
            bottom: 600.0,
            area_top: 0.0,
            area_left: 0.0,
            area_right: 400.0,
        };
        let (x, _, w, _) = fit(&a, DEFAULT_W, PILL_H);
        assert!(x >= 0.0 && x + w <= 400.0);
    }
}
