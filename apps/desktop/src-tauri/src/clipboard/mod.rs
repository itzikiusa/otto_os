//! Clipboard history: a Raycast-style panel of what you copied, summoned by a
//! global shortcut (⌘⌃⇧V by default, the `clipboard` id in shortcuts.rs).
//!
//! PRIVACY BOUNDARY — everything lives in this desktop shell. Capture
//! (mac.rs), storage and search never touch the daemon: the shell has no
//! bearer token and no HTTP surface, so the history can't reach the daemon's
//! API, MCP, agents, sessions, the phone/remote surface or telemetry. Its
//! content is readable only by the panel window: every command that returns
//! content checks the caller is `otto-clipboard` (Tauri lets any LOCAL window
//! invoke app commands, so the label check is the gate — the main window,
//! which agents can drive, gets status and settings only). The panel page
//! never signs in to the daemon (App.svelte), and nothing here logs content.
//!
//! Off until the person turns it on in the panel or Settings; turning it off
//! deletes the history. Stored in `<app data>/clipboard/` (0700; files 0600):
//! `history.json`, `settings.json`, `images/<id>.png` + `<id>.thumb.png`,
//! and small app icons in `icons/`.

pub mod history;
mod mac;

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use history::{Entry, Kind, PastePlan};
use objc2::MainThreadMarker;
use objc2_app_kit::NSPasteboard;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "otto-clipboard";
pub const DEFAULT_SHORTCUT: &str = "Cmd+Ctrl+Shift+V";
/// changeCount polling: macOS has no pasteboard-changed notification.
const POLL: Duration = Duration::from_millis(750);
const PANEL_W: f64 = 760.0;
const PANEL_H: f64 = 480.0;
/// What a list row carries of an entry's text (the preview asks for the rest).
const ROW_TEXT: usize = 300;

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct Settings {
    enabled: bool,
    paused: bool,
}

struct Store {
    dir: PathBuf,
    settings: Settings,
    entries: Vec<Entry>,
    access: mac::Access,
    /// The pasteboard's changeCount last seen — tracked even while off or
    /// paused, so what was copied meanwhile is never read later.
    change_count: isize,
    /// The app Return pastes into (frontmost when the panel opened).
    target: Option<String>,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn with_store<R>(f: impl FnOnce(&mut Store) -> R) -> Result<R, String> {
    let mut g = STORE.lock().unwrap_or_else(|p| p.into_inner());
    g.as_mut()
        .map(f)
        .ok_or_else(|| "clipboard history is unavailable".to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Run `f` on the main thread (AppKit) and wait for it; direct when already
/// there (sync commands run on the main thread).
fn on_main<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    if MainThreadMarker::new().is_some() {
        return Some(f());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .ok()?;
    rx.recv().ok()
}

// ---- storage ---------------------------------------------------------------

/// Owner-only, atomic (temp + rename).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(bytes)?;
    fs::rename(&tmp, path)
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

fn image_path(dir: &Path, id: &str, thumb: bool) -> PathBuf {
    dir.join("images").join(if thumb {
        format!("{id}.thumb.png")
    } else {
        format!("{id}.png")
    })
}

fn save(s: &Store) {
    let ok =
        serde_json::to_vec(&s.entries).map(|raw| write_private(&s.dir.join("history.json"), &raw));
    let ok2 = serde_json::to_vec(&s.settings)
        .map(|raw| write_private(&s.dir.join("settings.json"), &raw));
    if !matches!((ok, ok2), (Ok(Ok(())), Ok(Ok(())))) {
        eprintln!("clipboard history: could not save"); // never the content
    }
}

fn forget_images(dir: &Path, gone: &[Entry]) {
    for e in gone.iter().filter(|e| e.kind == Kind::Image) {
        let _ = fs::remove_file(image_path(dir, &e.id, false));
        let _ = fs::remove_file(image_path(dir, &e.id, true));
    }
}

/// `sips` (always on macOS) converts/shrinks images to PNG; outputs 0600.
fn sips_png(src: &Path, dst: &Path, max_px: Option<u32>) -> bool {
    let mut cmd = Command::new("/usr/bin/sips");
    cmd.args(["-s", "format", "png"]);
    if let Some(px) = max_px {
        cmd.args(["-Z", &px.to_string()]);
    }
    let ok = cmd
        .arg(src)
        .arg("--out")
        .arg(dst)
        .output()
        .is_ok_and(|o| o.status.success());
    ok && fs::set_permissions(dst, fs::Permissions::from_mode(0o600)).is_ok()
}

pub fn init(app: &AppHandle) {
    let Ok(dir) = app.path().app_data_dir().map(|d| d.join("clipboard")) else {
        eprintln!("clipboard history unavailable: no app data dir");
        return;
    };
    if private_dir(&dir.join("images"))
        .and_then(|_| private_dir(&dir))
        .is_err()
    {
        eprintln!("clipboard history unavailable: cannot create its folder");
        return;
    }
    let settings: Settings = read_json(&dir.join("settings.json"));
    let mut entries: Vec<Entry> = if settings.enabled {
        read_json(&dir.join("history.json"))
    } else {
        Vec::new()
    };
    forget_images(&dir, &history::prune(&mut entries, now_ms()));
    let change_count = NSPasteboard::generalPasteboard().changeCount(); // setup runs on main
    *STORE.lock().unwrap_or_else(|p| p.into_inner()) = Some(Store {
        dir,
        settings,
        entries,
        access: mac::Access::Allowed,
        change_count,
        target: None,
    });
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL);
        poll_once(&app);
    });
}

// ---- capture ---------------------------------------------------------------

fn poll_once(app: &AppHandle) {
    let Ok((capture, last)) =
        with_store(|s| (s.settings.enabled && !s.settings.paused, s.change_count))
    else {
        return;
    };
    let tick = on_main(app, move || {
        let pb = NSPasteboard::generalPasteboard();
        let (count, access) = (pb.changeCount(), mac::access(&pb));
        let read = (capture && count != last && access == mac::Access::Allowed)
            .then(|| (mac::read(&pb), mac::frontmost()));
        (count, access, read)
    });
    let Some((count, access, read)) = tick else {
        return;
    };
    let _ = with_store(|s| {
        s.change_count = count;
        s.access = access;
    });
    if let Some((snap, source)) = read {
        if capture_snapshot(snap, source).unwrap_or(false) {
            let _ = app.emit_to(LABEL, "otto://clipboard-changed", ());
        }
    }
}

/// Turn one read into an entry (storing a new image's files) and record it.
fn capture_snapshot(snap: mac::Snapshot, source: Option<history::Source>) -> Result<bool, String> {
    let dir = with_store(|s| s.dir.clone())?;
    let now = now_ms();
    let base = |kind: Kind, id: String, text: String, size: u64| Entry {
        id,
        kind,
        text,
        width: 0,
        height: 0,
        size,
        source: source.clone(),
        first_ms: now,
        last_ms: now,
        count: 1,
        pinned: false,
    };
    let entry = if !snap.files.is_empty() {
        let text = snap.files.join("\n");
        base(
            Kind::File,
            history::content_id(Kind::File, text.as_bytes()),
            text.clone(),
            text.len() as u64,
        )
    } else if let Some(text) = snap.text {
        let kind = history::text_kind(&text);
        base(
            kind,
            history::content_id(kind, text.as_bytes()),
            text.clone(),
            text.len() as u64,
        )
    } else if let Some(img) = snap.image {
        let id = history::content_id(Kind::Image, &img.bytes);
        let (full, thumb) = (image_path(&dir, &id, false), image_path(&dir, &id, true));
        if !full.exists() {
            let stored = if img.png {
                write_private(&full, &img.bytes).is_ok()
            } else {
                let tiff = full.with_extension("tiff");
                let ok = write_private(&tiff, &img.bytes).is_ok() && sips_png(&tiff, &full, None);
                let _ = fs::remove_file(&tiff);
                ok
            };
            if !stored || !sips_png(&full, &thumb, Some(160)) {
                let _ = fs::remove_file(&full);
                return Ok(false);
            }
        }
        let size = fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
        Entry {
            width: img.width,
            height: img.height,
            ..base(Kind::Image, id, String::new(), size)
        }
    } else {
        return Ok(false); // skipped by policy, or nothing we record
    };
    with_store(|s| {
        history::record(&mut s.entries, entry);
        let gone = history::prune(&mut s.entries, now);
        forget_images(&s.dir, &gone);
        save(s);
        true
    })
}

// ---- the panel -------------------------------------------------------------

fn build(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Ok(w);
    }
    let win =
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html#/clipboard".into()))
            .title("Clipboard History")
            .inner_size(PANEL_W, PANEL_H)
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
    });
    Ok(win)
}

/// Centred on the screen under the cursor, in its upper third.
fn place(app: &AppHandle, win: &tauri::WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|c| app.monitor_from_point(c.x, c.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let (s, wa) = (m.scale_factor(), m.work_area());
    let (x, y) = (wa.position.x as f64 / s, wa.position.y as f64 / s);
    let (w, h) = (wa.size.width as f64 / s, wa.size.height as f64 / s);
    let _ = win.set_position(LogicalPosition::new(
        x + (w - PANEL_W) / 2.0,
        y + (h - PANEL_H) / 3.0,
    ));
}

pub fn show(app: &AppHandle) {
    let Ok(win) = build(app) else { return };
    crate::bar::hide(app);
    crate::tray::hide_popover(app);
    place(app, &win);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        // Non-activating: the app in front stays in front, so Return can
        // paste straight back into it.
        let target = mac::frontmost().map(|s| s.name);
        let _ = with_store(|s| s.target = target);
        crate::panel::show_without_activation(&win);
        let _ = handle.emit_to(LABEL, "otto://clipboard-shown", ());
    });
}

pub fn hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.hide();
    }
}

pub fn toggle(app: &AppHandle) {
    match app
        .get_webview_window(LABEL)
        .and_then(|w| w.is_visible().ok())
    {
        Some(true) => hide(app),
        _ => show(app),
    }
}

/// Lost key status (a click elsewhere): dismiss, deferred like the tray's.
pub fn on_blur(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || hide(&handle));
}

// ---- commands: status + settings (any Otto window; no content) -------------

#[derive(Serialize)]
pub struct Status {
    enabled: bool,
    paused: bool,
    access: mac::Access,
    /// Accessibility granted (Return pastes; else it copies only).
    accessibility: bool,
    count: usize,
    target: Option<String>,
}

fn status() -> Result<Status, String> {
    let accessibility = mac::accessibility(false);
    with_store(|s| Status {
        enabled: s.settings.enabled,
        paused: s.settings.paused,
        access: s.access,
        accessibility,
        count: s.entries.len(),
        target: s.target.clone(),
    })
}

fn clear(s: &mut Store) {
    let gone = std::mem::take(&mut s.entries);
    forget_images(&s.dir, &gone);
}

#[tauri::command]
pub fn clipboard_status() -> Result<Status, String> {
    status()
}

/// Turn capture on/off (off deletes the history) or pause/resume it.
#[tauri::command]
pub fn clipboard_set(enabled: Option<bool>, paused: Option<bool>) -> Result<Status, String> {
    with_store(|s| {
        if let Some(on) = enabled {
            s.settings = Settings {
                enabled: on,
                paused: false,
            };
            if !on {
                clear(s);
            }
        }
        if let Some(p) = paused {
            s.settings.paused = p;
        }
        save(s);
    })?;
    status()
}

#[tauri::command]
pub fn clipboard_clear() -> Result<Status, String> {
    with_store(|s| {
        clear(s);
        save(s);
    })?;
    status()
}

/// The System Settings pane a permission lives in (fixed URLs only).
#[tauri::command]
pub fn clipboard_open_settings(pane: String) -> Result<(), String> {
    let anchor = match pane.as_str() {
        "accessibility" => {
            mac::accessibility(true); // lists Otto in the pane
            "Privacy_Accessibility"
        }
        "paste" => "Privacy_Pasteboard",
        _ => return Err("unknown settings pane".into()),
    };
    Command::new("/usr/bin/open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{anchor}"
        ))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clipboard_hide(app: AppHandle) {
    hide(&app);
}

// ---- commands: content (the panel window only) -----------------------------

fn panel_only(webview: &tauri::Webview) -> Result<(), String> {
    if webview.label() == LABEL {
        Ok(())
    } else {
        Err("clipboard history is readable only in its own panel".into())
    }
}

fn find(id: &str) -> Result<(Entry, PathBuf), String> {
    with_store(|s| {
        s.entries
            .iter()
            .find(|e| e.id == id)
            .cloned()
            .map(|e| (e, s.dir.clone()))
    })?
    .ok_or_else(|| "that entry is gone".to_string())
}

/// Newest first, filtered; each row's text cut to its first characters.
#[tauri::command]
pub fn clipboard_list(
    webview: tauri::Webview,
    query: String,
    kind: Option<Kind>,
) -> Result<Vec<Entry>, String> {
    panel_only(&webview)?;
    with_store(|s| {
        s.entries
            .iter()
            .filter(|e| history::matches(e, &query, kind))
            .map(|e| Entry {
                text: e.text.chars().take(ROW_TEXT).collect(),
                ..e.clone()
            })
            .collect()
    })
}

#[tauri::command]
pub fn clipboard_text(webview: tauri::Webview, id: String) -> Result<String, String> {
    panel_only(&webview)?;
    Ok(find(&id)?.0.text)
}

#[tauri::command]
pub fn clipboard_image(
    webview: tauri::Webview,
    id: String,
    thumb: bool,
) -> Result<tauri::ipc::Response, String> {
    panel_only(&webview)?;
    let (e, dir) = find(&id)?;
    fs::read(image_path(&dir, &e.id, thumb))
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

/// A source app's icon as a 64 px PNG, cached in `icons/`.
#[tauri::command]
pub async fn clipboard_icon(
    app: AppHandle,
    webview: tauri::Webview,
    bundle_id: String,
) -> Result<tauri::ipc::Response, String> {
    panel_only(&webview)?;
    if bundle_id.is_empty()
        || !bundle_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err("bad bundle id".into());
    }
    let icons = with_store(|s| s.dir.join("icons"))?;
    let png = icons.join(format!("{bundle_id}.png"));
    if !png.exists() {
        let id = bundle_id.clone();
        let tiff = on_main(&app, move || mac::app_icon_tiff(&id))
            .flatten()
            .ok_or("no icon")?;
        let tmp = icons.join(format!("{bundle_id}.tiff"));
        let ok = private_dir(&icons).is_ok()
            && write_private(&tmp, &tiff).is_ok()
            && sips_png(&tmp, &png, Some(64));
        let _ = fs::remove_file(&tmp);
        if !ok {
            return Err("no icon".into());
        }
    }
    fs::read(&png)
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

/// Put an entry on the clipboard; with `paste`, also hide the panel and send
/// ⌘V to the app in front. Resolves whether it pasted (false = copied only:
/// no Accessibility permission, or a plain copy was asked for).
#[tauri::command]
pub fn clipboard_paste(
    app: AppHandle,
    webview: tauri::Webview,
    id: String,
    paste: bool,
) -> Result<bool, String> {
    panel_only(&webview)?;
    let (e, dir) = find(&id)?;
    let png = (e.kind == Kind::Image)
        .then(|| fs::read(image_path(&dir, &e.id, false)).ok())
        .flatten();
    let plan = if paste {
        history::paste_plan(mac::accessibility(false))
    } else {
        PastePlan::CopyOnly
    };
    let written = on_main(&app, move || {
        mac::write(&NSPasteboard::generalPasteboard(), &e, png.as_deref())
    });
    if written != Some(true) {
        return Err("macOS refused the clipboard write".into());
    }
    if plan == PastePlan::CopyOnly {
        return Ok(false);
    }
    hide(&app);
    std::thread::spawn(|| {
        // Let the panel order out so the target window is key again.
        std::thread::sleep(Duration::from_millis(150));
        mac::send_cmd_v();
    });
    Ok(true)
}

#[tauri::command]
pub fn clipboard_pin(webview: tauri::Webview, id: String, pinned: bool) -> Result<(), String> {
    panel_only(&webview)?;
    with_store(|s| {
        if let Some(e) = s.entries.iter_mut().find(|e| e.id == id) {
            e.pinned = pinned;
        }
        save(s);
    })
}

#[tauri::command]
pub fn clipboard_delete(webview: tauri::Webview, id: String) -> Result<(), String> {
    panel_only(&webview)?;
    with_store(|s| {
        let (gone, keep): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut s.entries)
            .into_iter()
            .partition(|e| e.id == id);
        s.entries = keep;
        forget_images(&s.dir, &gone);
        save(s);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_files_and_folders_are_owner_only() {
        let dir = std::env::temp_dir().join(format!("otto-clip-{}", std::process::id()));
        private_dir(&dir).unwrap();
        let f = dir.join("history.json");
        write_private(&f, b"[]").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&f), 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The panel gets its own capability with only event listening: never
    /// the main windows' `core:default`, and the main capability never
    /// covers the panel.
    #[test]
    fn the_panel_capability_is_listen_only() {
        let read = |name: &str| -> serde_json::Value {
            let path = format!("{}/capabilities/{name}", env!("CARGO_MANIFEST_DIR"));
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
        };
        let cap = read("clipboard.json");
        assert_eq!(cap["windows"], serde_json::json!([LABEL]));
        assert_eq!(
            cap["permissions"],
            serde_json::json!(["core:event:allow-listen", "core:event:allow-unlisten"])
        );
        assert!(cap.get("remote").is_none());
        let main = read("default.json");
        let windows = main["windows"].as_array().unwrap();
        assert!(!windows
            .iter()
            .any(|w| w == LABEL || w == "otto-*" || w == "*"));
    }
}
