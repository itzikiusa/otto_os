//! Multi-window lifecycle: a persisted *window registry* (labels + frames) that
//! recreates the previous window set on launch, so relaunching Otto reopens the
//! same windows in the same places. The SPA side keys its per-window layout
//! state off `window.__OTTO_WIN__` (injected below); sessions stay daemon-owned,
//! so a window only ever restores *references* (tabs/routes), never processes.
//!
//! Semantics (macOS-conventional):
//!   * closing a NON-last window forgets it (removed from the registry);
//!   * quitting (Cmd+Q) — or closing the LAST window, which exits the app —
//!     snapshots the whole set, restored on next launch.
//!
//! The registry lives beside the supervisor's data dir
//! (`~/Library/Application Support/Otto/windows.json`) and is written atomically
//! (temp file + rename) so a crash mid-write can't corrupt it; a missing or
//! corrupt file degrades to "just the main window", never a startup failure.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

/// One window's persisted placement (physical pixels, like tauri reports them).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WinFrame {
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    #[serde(default)]
    pub fullscreen: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Registry {
    /// Monotonic id for secondary labels (`w2`, `w3`, …) — never reused, so a
    /// closed window's namespaced SPA state can be GC'd without aliasing.
    pub next_id: u32,
    pub windows: Vec<WinFrame>,
}

/// In-memory registry (single writer: the main thread's window events).
static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);
/// Set on the quit path so per-window CloseRequested bookkeeping doesn't
/// "forget" windows that are only closing because the app is exiting.
static QUITTING: AtomicBool = AtomicBool::new(false);
/// Generation counter for debounced frame saves (drag emits Moved storms).
static SAVE_GEN: AtomicU64 = AtomicU64::new(0);

pub fn registry_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("Library/Application Support/Otto/windows.json")
}

/// Load a registry; missing/corrupt → `Default` (never an error).
pub fn load(path: &Path) -> Registry {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Atomic save: write a sibling temp file, then rename over the target.
pub fn save(path: &Path, reg: &Registry) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(reg).unwrap_or_default())?;
    fs::rename(&tmp, path)
}

/// Clamp a frame onto the available monitors (each `(x, y, w, h)` in physical
/// px). A frame whose title-bar region intersects no monitor (unplugged screen)
/// is recentred on the first monitor; sizes are left alone (the OS clips).
pub fn clamp_frame(frame: &mut WinFrame, monitors: &[(i32, i32, u32, u32)]) {
    if monitors.is_empty() {
        return;
    }
    let visible = monitors.iter().any(|&(mx, my, mw, mh)| {
        // "Grabbable": the frame's top strip overlaps this monitor's rect.
        let (mx2, my2) = (mx + mw as i32, my + mh as i32);
        frame.x + (frame.w as i32) > mx + 40
            && frame.x < mx2 - 40
            && frame.y >= my - 20
            && frame.y < my2 - 40
    });
    if !visible {
        let (mx, my, mw, mh) = monitors[0];
        frame.x = mx + (mw.saturating_sub(frame.w) / 2) as i32;
        frame.y = my + (mh.saturating_sub(frame.h) / 2) as i32;
    }
}

fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    let mut guard = REGISTRY.lock().unwrap_or_else(|p| p.into_inner());
    let reg = guard.get_or_insert_with(|| load(&registry_path()));
    f(reg)
}

fn persist() {
    let snapshot = with_registry(|r| r.clone());
    if let Err(e) = save(&registry_path(), &snapshot) {
        eprintln!("windows.json save failed: {e}");
    }
}

pub fn mark_quitting() {
    QUITTING.store(true, Ordering::SeqCst);
}

pub fn is_quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

fn monitors_of(app: &tauri::AppHandle) -> Vec<(i32, i32, u32, u32)> {
    app.available_monitors()
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    let p = m.position();
                    let s = m.size();
                    (p.x, p.y, s.width, s.height)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Current physical frame of a live window (None while minimized/gone).
fn live_frame(win: &tauri::WebviewWindow) -> Option<WinFrame> {
    let pos = win.outer_position().ok()?;
    let size = win.outer_size().ok()?;
    Some(WinFrame {
        label: win.label().to_string(),
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
        fullscreen: win.is_fullscreen().unwrap_or(false),
    })
}

/// Write every live window's frame into the registry (+ disk). Used on quit and
/// as the debounced Moved/Resized handler.
pub fn snapshot_all(app: &tauri::AppHandle) {
    let frames: Vec<WinFrame> = app
        .webview_windows()
        .values()
        .filter(|w| !w.label().starts_with("otto-browser-"))
        .filter_map(live_frame)
        .collect();
    with_registry(|reg| {
        for f in frames {
            match reg.windows.iter_mut().find(|w| w.label == f.label) {
                Some(slot) => *slot = f,
                None => reg.windows.push(f),
            }
        }
    });
    persist();
}

/// Debounced snapshot: coalesce the Moved/Resized event storm into one save
/// ~400ms after the last event.
pub fn schedule_snapshot(app: &tauri::AppHandle) {
    let gen = SAVE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if SAVE_GEN.load(Ordering::SeqCst) == gen && !is_quitting() {
            snapshot_all(&app);
        }
    });
}

/// A window is being closed by the user (not by quit): forget it — unless it is
/// the LAST window, in which case its close IS the quit gesture on macOS-lite
/// semantics (tauri exits when the last window closes), so snapshot instead.
pub fn on_close_requested(app: &tauri::AppHandle, label: &str) {
    if is_quitting() || label.starts_with("otto-browser-") {
        return;
    }
    let real_windows = app
        .webview_windows()
        .keys()
        .filter(|l| !l.starts_with("otto-browser-"))
        .count();
    if real_windows <= 1 {
        mark_quitting();
        snapshot_all(app);
        return;
    }
    with_registry(|reg| reg.windows.retain(|w| w.label != label));
    persist();
}

/// Builder options shared by every Otto window — parity with the `main` window
/// declared in tauri.conf.json, plus the per-window id the SPA keys state off.
fn build_window(app: &tauri::AppHandle, frame: &WinFrame) -> tauri::Result<tauri::WebviewWindow> {
    let win = WebviewWindowBuilder::new(app, &frame.label, WebviewUrl::App("index.html".into()))
        .title("Otto")
        .min_inner_size(980.0, 640.0)
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .transparent(true)
        .accept_first_mouse(true)
        .disable_drag_drop_handler()
        .initialization_script(format!("window.__OTTO_WIN__='{}';", frame.label))
        .build()?;
    let _ = win.set_position(PhysicalPosition::new(frame.x, frame.y));
    let _ = win.set_size(PhysicalSize::new(frame.w, frame.h));
    if frame.fullscreen {
        let _ = win.set_fullscreen(true);
    }
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
        let _ = apply_vibrancy(&win, NSVisualEffectMaterial::Sidebar, None, None);
    }
    Ok(win)
}

/// Startup: restore the main window's frame and recreate every secondary
/// window recorded in the registry (clamped back on-screen if displays changed).
pub fn restore(app: &tauri::AppHandle) {
    let monitors = monitors_of(app);
    let entries = with_registry(|reg| reg.windows.clone());
    for mut frame in entries {
        if frame.label == "main" {
            if let Some(main) = app.get_webview_window("main") {
                clamp_frame(&mut frame, &monitors);
                let _ = main.set_position(PhysicalPosition::new(frame.x, frame.y));
                let _ = main.set_size(PhysicalSize::new(frame.w, frame.h));
                if frame.fullscreen {
                    let _ = main.set_fullscreen(true);
                }
            }
            continue;
        }
        clamp_frame(&mut frame, &monitors);
        if let Err(e) = build_window(app, &frame) {
            eprintln!("could not restore window {}: {e}", frame.label);
            with_registry(|reg| reg.windows.retain(|w| w.label != frame.label));
        }
    }
    // Make sure the registry knows main (first launch has no file at all).
    if let Some(main) = app.get_webview_window("main") {
        if let Some(f) = live_frame(&main) {
            with_registry(|reg| {
                if !reg.windows.iter().any(|w| w.label == "main") {
                    reg.windows.push(f);
                }
                if reg.next_id < 2 {
                    reg.next_id = 2;
                }
            });
        }
    }
    persist();
}

/// `File → New Window`: mint the next label, cascade from the focused window,
/// record it, create it.
pub fn create_new_window(app: &tauri::AppHandle) {
    let monitors = monitors_of(app);
    let base = app
        .webview_windows()
        .values()
        .find(|w| w.is_focused().unwrap_or(false) && !w.label().starts_with("otto-browser-"))
        .and_then(live_frame)
        .or_else(|| app.get_webview_window("main").as_ref().and_then(live_frame));
    let mut frame = base.unwrap_or(WinFrame {
        label: String::new(),
        x: 120,
        y: 120,
        w: 1280,
        h: 800,
        fullscreen: false,
    });
    frame.label = with_registry(|reg| {
        let n = reg.next_id.max(2);
        reg.next_id = n + 1;
        format!("w{n}")
    });
    frame.x += 24;
    frame.y += 24;
    frame.fullscreen = false;
    clamp_frame(&mut frame, &monitors);
    match build_window(app, &frame) {
        Ok(_) => {
            with_registry(|reg| reg.windows.push(frame));
            persist();
        }
        Err(e) => eprintln!("could not create window: {e}"),
    }
}

/// Live window labels for the SPA's stale-state GC (`windows_registry` command).
#[tauri::command]
pub fn windows_registry() -> Vec<String> {
    with_registry(|reg| reg.windows.iter().map(|w| w.label.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("otto-winreg-{}", std::process::id()));
        let path = dir.join("windows.json");
        let reg = Registry {
            next_id: 4,
            windows: vec![
                WinFrame {
                    label: "main".into(),
                    x: 10,
                    y: 20,
                    w: 1280,
                    h: 800,
                    fullscreen: false,
                },
                WinFrame {
                    label: "w2".into(),
                    x: 40,
                    y: 60,
                    w: 1000,
                    h: 700,
                    fullscreen: true,
                },
            ],
        };
        save(&path, &reg).expect("save");
        assert_eq!(load(&path), reg);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_or_corrupt_defaults() {
        let dir = std::env::temp_dir().join(format!("otto-winreg-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("nope.json");
        assert_eq!(load(&missing), Registry::default());
        let corrupt = dir.join("bad.json");
        std::fs::write(&corrupt, b"{not json").unwrap();
        assert_eq!(load(&corrupt), Registry::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clamp_keeps_visible_frame_and_recentres_lost_one() {
        let monitors = vec![(0, 0, 2560u32, 1440u32)];
        let mut ok = WinFrame {
            label: "w2".into(),
            x: 100,
            y: 100,
            w: 1280,
            h: 800,
            fullscreen: false,
        };
        let before = ok.clone();
        clamp_frame(&mut ok, &monitors);
        assert_eq!(ok, before, "on-screen frame must be untouched");

        // Window living on a monitor that no longer exists (far right).
        let mut lost = WinFrame {
            label: "w3".into(),
            x: 5000,
            y: 100,
            w: 1280,
            h: 800,
            fullscreen: false,
        };
        clamp_frame(&mut lost, &monitors);
        assert_eq!(lost.x, (2560 - 1280) / 2);
        assert_eq!(lost.y, (1440 - 800) / 2);

        // No monitor info → leave alone rather than guess.
        let mut untouched = WinFrame {
            label: "w4".into(),
            x: 5000,
            y: 100,
            w: 100,
            h: 100,
            fullscreen: false,
        };
        let before = untouched.clone();
        clamp_frame(&mut untouched, &[]);
        assert_eq!(untouched, before);
    }

    /// Guard: the capability file must grant IPC to every real Otto window
    /// (`main`, `w2`, `w3`, …) or drag/maximize/menu/zoom silently die in
    /// secondary windows — and must NEVER cover the embedded-browser windows,
    /// which host remote web content.
    #[test]
    fn capability_covers_secondary_windows_but_not_browser() {
        // '*'-only glob, mirroring how tauri matches capability window patterns.
        fn glob_match(pat: &str, s: &str) -> bool {
            match pat.split_once('*') {
                None => pat == s,
                Some((pre, rest)) => {
                    assert!(!rest.contains('*'), "extend glob_match for {pat}");
                    s.strip_prefix(pre)
                        .is_some_and(|tail| tail.ends_with(rest) && tail.len() >= rest.len())
                }
            }
        }
        let raw = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/capabilities/default.json"
        ))
        .expect("capability file");
        let cap: serde_json::Value = serde_json::from_slice(&raw).expect("valid json");
        let patterns: Vec<String> = cap["windows"]
            .as_array()
            .expect("windows array")
            .iter()
            .map(|v| v.as_str().expect("string pattern").to_string())
            .collect();
        let covered = |label: &str| patterns.iter().any(|p| glob_match(p, label));
        assert!(covered("main"), "main window must keep IPC");
        assert!(covered("w2"), "first secondary window needs IPC");
        assert!(covered("w34"), "all minted w<N> labels need IPC");
        assert!(
            !covered("otto-browser-1"),
            "embedded-browser windows host REMOTE content and must never get IPC"
        );
        // Drag + double-click-maximize both go through JS IPC (windowDrag.ts).
        let perms: Vec<&str> = cap["permissions"]
            .as_array()
            .expect("permissions array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(perms.contains(&"core:window:allow-start-dragging"));
        assert!(perms.contains(&"core:window:allow-toggle-maximize"));
    }
}
