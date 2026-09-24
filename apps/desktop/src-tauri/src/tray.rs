//! Menu-bar (tray) item + its popover.
//!
//! The status item shows Otto's ✦ glyph in three states:
//!   * idle      — template glyph (tinted by macOS for light/dark menu bars);
//!   * running   — template glyph + a dot (work is running);
//!   * needs you — amber glyph + dot (an approval or question is waiting).
//!
//! The Rust side holds no bearer token, so it can't ask the daemon itself: the
//! popover page (`#/tray`, loaded once in a hidden window at startup) polls
//! the existing endpoints and reports counts through `tray_set_status`.
//!
//! Left click toggles a 360×520 popover anchored under the icon — a
//! non-activating panel (panel.rs) with native popover vibrancy that hides
//! when it loses key status. Right click opens a small native menu (Open
//! Otto / Ask Otto / Settings / Quit) whose ids main.rs handles.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

pub const TRAY_ID: &str = "otto";
pub const POPOVER_LABEL: &str = "otto-tray";
const POPOVER_W: f64 = 360.0;
const POPOVER_H: f64 = 520.0;
/// Gap between the menu bar and the popover's top edge.
const POPOVER_GAP: f64 = 6.0;
/// A click on the icon while the popover is open first resigns the popover's
/// key status (which hides it), then delivers the click — without this guard
/// that click would immediately reopen it.
const REOPEN_GUARD_MS: u64 = 300;

/// Tray menu ids (handled in main.rs `on_menu_event`).
pub const MENU_OPEN: &str = "tray-open";
pub const MENU_ASK: &str = "tray-ask";
pub const MENU_SETTINGS: &str = "tray-settings";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Idle,
    Running,
    NeedsYou,
}

impl Glyph {
    fn from_counts(running: u32, needs_you: u32) -> Self {
        if needs_you > 0 {
            Glyph::NeedsYou
        } else if running > 0 {
            Glyph::Running
        } else {
            Glyph::Idle
        }
    }
}

static CURRENT: Mutex<Option<Glyph>> = Mutex::new(None);
static LAST_BLUR_HIDE_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---- glyph rasterizer ----------------------------------------------------
//
// 36×36 RGBA = an 18pt status-bar icon at @2x (tray-icon sizes the NSImage to
// 18pt tall). Drawn procedurally so there is no asset to keep in sync: a
// four-point sparkle (a p<1 superellipse) plus an optional
// status dot, 4×4 supersampled for anti-aliasing.

const ICON_PX: u32 = 36;
/// Amber for "needs you" — visible on both light and dark menu bars (a
/// non-template image is drawn as-is, so it can't use the system tint).
const AMBER: [u8; 3] = [0xF5, 0xA6, 0x23];

fn coverage(px: u32, py: u32, inside: impl Fn(f64, f64) -> bool) -> f64 {
    const N: u32 = 4;
    let mut hits = 0;
    for sy in 0..N {
        for sx in 0..N {
            let x = px as f64 + (sx as f64 + 0.5) / N as f64;
            let y = py as f64 + (sy as f64 + 0.5) / N as f64;
            if inside(x, y) {
                hits += 1;
            }
        }
    }
    hits as f64 / (N * N) as f64
}

/// Render one glyph state. Template variants are black-on-transparent (macOS
/// recolours them); the needs-you variant is amber and drawn as-is.
pub fn render(glyph: Glyph) -> Vec<u8> {
    // Superellipse |x|^p + |y|^p ≤ r^p with p < 1 is a concave four-point
    // star; p = 0.6 keeps the arms thick enough to read at 18pt.
    const P: f64 = 0.6;
    let (cx, cy, r) = (16.0_f64, 19.0_f64, 15.0_f64);
    let rp = r.powf(P);
    let star = move |x: f64, y: f64| {
        let dx = (x - cx).abs();
        let dy = (y - cy).abs();
        dx.powf(P) + dy.powf(P) <= rp
    };
    let (dot_x, dot_y, dot_r) = (29.5_f64, 6.5_f64, 5.0_f64);
    let dot = move |x: f64, y: f64| (x - dot_x).powi(2) + (y - dot_y).powi(2) <= dot_r * dot_r;
    // A thin transparent ring around the dot keeps it legible where it
    // overlaps the star's top-right point.
    let ring = move |x: f64, y: f64| {
        (x - dot_x).powi(2) + (y - dot_y).powi(2) <= (dot_r + 1.5) * (dot_r + 1.5)
    };
    let with_dot = glyph != Glyph::Idle;
    let rgb = if glyph == Glyph::NeedsYou {
        AMBER
    } else {
        [0, 0, 0]
    };
    let mut buf = Vec::with_capacity((ICON_PX * ICON_PX * 4) as usize);
    for py in 0..ICON_PX {
        for px in 0..ICON_PX {
            let mut a = coverage(px, py, star);
            if with_dot {
                let knock = coverage(px, py, ring);
                a *= 1.0 - knock;
                a = a.max(coverage(px, py, dot));
            }
            buf.extend_from_slice(&[rgb[0], rgb[1], rgb[2], (a * 255.0).round() as u8]);
        }
    }
    buf
}

fn icon(glyph: Glyph) -> Image<'static> {
    Image::new_owned(render(glyph), ICON_PX, ICON_PX)
}

// ---- tray item -----------------------------------------------------------

pub fn init(app: &AppHandle) {
    if let Err(e) = build_tray(app) {
        eprintln!("menu-bar item unavailable: {e}");
    }
    if let Err(e) = build_popover(app) {
        eprintln!("menu-bar popover unavailable: {e}");
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, MENU_OPEN, "Open Otto", true, None::<&str>)?,
            &MenuItem::with_id(app, MENU_ASK, "Ask Otto…", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, MENU_SETTINGS, "Settings…", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            // Same id as the app menu's Quit: main.rs runs the snapshot-then-
            // exit path for it.
            &MenuItem::with_id(app, "quit", "Quit Otto", true, None::<&str>)?,
        ],
    )?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(Glyph::Idle))
        .icon_as_template(true)
        .tooltip("Otto")
        .menu(&menu)
        // Left click = popover; the menu is the right-click affordance.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                toggle_popover(tray.app_handle(), Some(rect));
            }
        })
        .build(app)?;
    *CURRENT.lock().unwrap_or_else(|p| p.into_inner()) = Some(Glyph::Idle);
    Ok(())
}

fn set_glyph(app: &AppHandle, glyph: Glyph, tooltip: &str) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let _ = tray.set_tooltip(Some(tooltip));
    let mut cur = CURRENT.lock().unwrap_or_else(|p| p.into_inner());
    if *cur == Some(glyph) {
        return;
    }
    // Atomic icon + template swap (no flicker between the two calls).
    if tray
        .set_icon_with_as_template(Some(icon(glyph)), glyph != Glyph::NeedsYou)
        .is_ok()
    {
        *cur = Some(glyph);
    }
}

fn tooltip_for(running: u32, needs_you: u32) -> String {
    match (running, needs_you) {
        (0, 0) => "Otto".to_string(),
        (r, 0) => format!("Otto — {r} running"),
        (0, n) => format!("Otto — {n} need{} you", if n == 1 { "s" } else { "" }),
        (r, n) => format!(
            "Otto — {n} need{} you · {r} running",
            if n == 1 { "s" } else { "" }
        ),
    }
}

/// Counts from the popover page's poll → glyph + tooltip.
#[tauri::command]
pub fn tray_set_status(app: AppHandle, running: u32, needs_you: u32) {
    set_glyph(
        &app,
        Glyph::from_counts(running, needs_you),
        &tooltip_for(running, needs_you),
    );
}

// ---- popover -------------------------------------------------------------

fn build_popover(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window(POPOVER_LABEL) {
        return Ok(w);
    }
    let win = WebviewWindowBuilder::new(
        app,
        POPOVER_LABEL,
        WebviewUrl::App("index.html#/tray".into()),
    )
    .title("Otto")
    .inner_size(POPOVER_W, POPOVER_H)
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
    .initialization_script(format!("window.__OTTO_WIN__='{POPOVER_LABEL}';"))
    .build()?;
    let for_main = win.clone();
    let _ = app.run_on_main_thread(move || {
        crate::panel::make_nonactivating_panel(&for_main);
        #[cfg(target_os = "macos")]
        {
            use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
            let _ = apply_vibrancy(
                &for_main,
                NSVisualEffectMaterial::Popover,
                Some(NSVisualEffectState::Active),
                Some(12.0),
            );
        }
    });
    Ok(win)
}

/// Top-left for a popover centred under the icon, clamped into the icon's
/// screen (logical points). `icon` = (x, y, w, h) logical; `screen` = (x, y,
/// w, h) logical of the monitor holding the icon.
fn popover_origin(icon: (f64, f64, f64, f64), screen: (f64, f64, f64, f64)) -> (f64, f64) {
    let (ix, iy, iw, ih) = icon;
    let (sx, sy, sw, sh) = screen;
    let x = (ix + iw / 2.0 - POPOVER_W / 2.0)
        .min(sx + sw - POPOVER_W - 8.0)
        .max(sx + 8.0);
    let y = (iy + ih + POPOVER_GAP)
        .min(sy + sh - POPOVER_H - 8.0)
        .max(sy);
    (x, y)
}

/// The tray reports its rect in physical px of the menu-bar screen; convert
/// with THAT screen's scale factor and clamp the popover into it.
fn position_popover(app: &AppHandle, win: &tauri::WebviewWindow, rect: tauri::Rect) {
    let monitors = app.available_monitors().unwrap_or_default();
    let (ppos, psize) = match (rect.position, rect.size) {
        (tauri::Position::Physical(p), tauri::Size::Physical(s)) => {
            ((p.x as f64, p.y as f64), (s.width as f64, s.height as f64))
        }
        (tauri::Position::Logical(p), tauri::Size::Logical(s)) => {
            // Already points: place directly on the first matching screen.
            let screen = monitors
                .iter()
                .map(logical_rect)
                .find(|&(x, y, w, h)| p.x >= x && p.x < x + w && p.y >= y && p.y < y + h)
                .unwrap_or((p.x - POPOVER_W, p.y, POPOVER_W * 3.0, POPOVER_H * 2.0));
            let (x, y) = popover_origin((p.x, p.y, s.width, s.height), screen);
            let _ = win.set_position(LogicalPosition::new(x, y));
            return;
        }
        // Mixed units never happen in practice; fall back to physical math.
        (pos, size) => {
            let p = pos.to_physical::<f64>(1.0);
            let s = size.to_physical::<f64>(1.0);
            ((p.x, p.y), (s.width, s.height))
        }
    };
    let monitor = monitors.iter().find(|m| {
        let p = m.position();
        let s = m.size();
        ppos.0 >= p.x as f64
            && ppos.0 < p.x as f64 + s.width as f64
            && ppos.1 >= p.y as f64
            && ppos.1 < p.y as f64 + s.height as f64
    });
    let Some(m) = monitor.or_else(|| monitors.first()) else {
        return;
    };
    let scale = m.scale_factor();
    let icon = (
        ppos.0 / scale,
        ppos.1 / scale,
        psize.0 / scale,
        psize.1 / scale,
    );
    let (x, y) = popover_origin(icon, logical_rect(m));
    let _ = win.set_position(LogicalPosition::new(x, y));
}

fn logical_rect(m: &tauri::Monitor) -> (f64, f64, f64, f64) {
    let scale = m.scale_factor();
    let p = m.position();
    let s = m.size();
    (
        p.x as f64 / scale,
        p.y as f64 / scale,
        s.width as f64 / scale,
        s.height as f64 / scale,
    )
}

pub fn show_popover(app: &AppHandle, rect: Option<tauri::Rect>) {
    let Ok(win) = build_popover(app) else { return };
    let rect = rect.or_else(|| {
        app.tray_by_id(TRAY_ID)
            .and_then(|t| t.rect().ok().flatten())
    });
    if let Some(r) = rect {
        position_popover(app, &win, r);
    }
    crate::bar::hide(app);
    let for_main = win.clone();
    let _ = app.run_on_main_thread(move || crate::panel::show_without_activation(&for_main));
    // The page refreshes its lists immediately instead of waiting a poll tick.
    let _ = app.emit_to(POPOVER_LABEL, "otto://tray-shown", ());
}

pub fn hide_popover(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(POPOVER_LABEL) {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        }
    }
}

fn toggle_popover(app: &AppHandle, rect: Option<tauri::Rect>) {
    let visible = app
        .get_webview_window(POPOVER_LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if visible {
        hide_popover(app);
        return;
    }
    if now_ms().saturating_sub(LAST_BLUR_HIDE_MS.load(Ordering::SeqCst)) < REOPEN_GUARD_MS {
        return; // this click is the one that just dismissed it
    }
    show_popover(app, rect);
}

/// The popover lost key status (click elsewhere / another app): dismiss it,
/// like a native menu-bar popover.
pub fn on_blur(app: &AppHandle) {
    if app
        .get_webview_window(POPOVER_LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
    {
        LAST_BLUR_HIDE_MS.store(now_ms(), Ordering::SeqCst);
        hide_popover(app);
    }
}

#[tauri::command]
pub fn tray_popover_hide(app: AppHandle) {
    hide_popover(&app);
}

#[derive(Serialize)]
pub struct TrayInfo {
    /// False when the status item couldn't be created (the page then knows
    /// its counts go nowhere).
    pub has_tray: bool,
}

#[tauri::command]
pub fn tray_info(app: AppHandle) -> TrayInfo {
    TrayInfo {
        has_tray: app.tray_by_id(TRAY_ID).is_some(),
    }
}

/// Used by the tray menu's "Ask Otto…" and by the page's quick-ask row.
pub fn ask(app: &AppHandle) {
    hide_popover(app);
    crate::bar::show(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(buf: &[u8], x: u32, y: u32) -> u8 {
        buf[((y * ICON_PX + x) * 4 + 3) as usize]
    }

    #[test]
    fn glyph_buffers_have_the_right_size_and_shape() {
        for g in [Glyph::Idle, Glyph::Running, Glyph::NeedsYou] {
            let buf = render(g);
            assert_eq!(buf.len(), (ICON_PX * ICON_PX * 4) as usize);
            // Star centre is solid; a far corner is empty.
            assert_eq!(alpha_at(&buf, 16, 19), 255);
            assert_eq!(alpha_at(&buf, 1, 34), 0);
        }
        // Only running / needs-you carry the status dot.
        assert_eq!(alpha_at(&render(Glyph::Idle), 30, 6), 0);
        assert_eq!(alpha_at(&render(Glyph::Running), 30, 6), 255);
        assert_eq!(alpha_at(&render(Glyph::NeedsYou), 30, 6), 255);
    }

    #[test]
    fn needs_you_is_amber_and_templates_are_black() {
        let amber = render(Glyph::NeedsYou);
        let i = ((19 * ICON_PX + 16) * 4) as usize;
        assert_eq!(&amber[i..i + 3], &AMBER);
        let idle = render(Glyph::Idle);
        assert_eq!(&idle[i..i + 3], &[0, 0, 0]);
    }

    #[test]
    fn counts_pick_the_glyph() {
        assert_eq!(Glyph::from_counts(0, 0), Glyph::Idle);
        assert_eq!(Glyph::from_counts(3, 0), Glyph::Running);
        assert_eq!(Glyph::from_counts(3, 1), Glyph::NeedsYou);
        assert_eq!(Glyph::from_counts(0, 2), Glyph::NeedsYou);
        assert_eq!(tooltip_for(0, 1), "Otto — 1 needs you");
        assert_eq!(tooltip_for(2, 3), "Otto — 3 need you · 2 running");
    }

    #[test]
    fn popover_sits_under_the_icon_and_stays_on_screen() {
        let screen = (0.0, 0.0, 1512.0, 982.0);
        // Icon mid-bar: centred under it, just below the menu bar.
        let (x, y) = popover_origin((700.0, 0.0, 24.0, 24.0), screen);
        assert_eq!(x, 712.0 - POPOVER_W / 2.0);
        assert_eq!(y, 24.0 + POPOVER_GAP);
        // Icon at the far right edge: clamped inside the screen.
        let (x, _) = popover_origin((1500.0, 0.0, 24.0, 24.0), screen);
        assert!(x + POPOVER_W <= 1512.0);
        // Second monitor to the right: stays on that monitor.
        let (x, _) = popover_origin(
            (1512.0 + 5.0, 0.0, 24.0, 24.0),
            (1512.0, 0.0, 1920.0, 1080.0),
        );
        assert!(x >= 1512.0);
    }
}
