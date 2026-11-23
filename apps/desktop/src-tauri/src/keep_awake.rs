//! Keep awake: the menu-bar item's "caffeinate" — keep this Mac (and its
//! display) awake for a while, or until turned off.
//!
//! It holds the same two power assertions `caffeinate -d -i` holds
//! (`PreventUserIdleDisplaySleep` + `PreventUserIdleSystemSleep`), taken by
//! THIS process through IOKit. powerd ties an assertion to the process that
//! took it, so a quit or a crash releases it with no cleanup of ours — no
//! child process to orphan. It lives in the shell, not the daemon: the tray
//! menu is here, the daemon outlives the app under launchd (an assertion it
//! held would survive a quit), and the shell needs no bearer token for it.
//!
//! State is in memory only (off after a relaunch). A timed run ends on a
//! ticker that checks the WALL clock, so a forced sleep (lid closed) can't
//! push the end out; the ticker also refreshes the menu-bar countdown.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// The `keep-awake` global shortcut's default (shortcuts.rs; C for
/// caffeinate): on indefinitely when off, off when on.
pub const DEFAULT_SHORTCUT: &str = "Cmd+Ctrl+Shift+C";
/// Tray menu ids (main.rs routes every `keep-awake-*` id here).
pub const MENU_PREFIX: &str = "keep-awake-";
pub const MENU_CUSTOM: &str = "keep-awake-custom";
/// The name `pmset -g assertions` shows next to Otto's pid.
const ASSERTION_NAME: &str = "Otto keep awake";
/// Longest gap between two countdown refreshes.
const TICK: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct KeepAwake {
    pub active: bool,
    /// Epoch ms when it turns itself off; `None` = until turned off.
    pub until_ms: Option<u64>,
}

struct Held {
    ids: Vec<u32>,
    until_ms: Option<u64>,
    gen: u64,
}

static HELD: Mutex<Option<Held>> = Mutex::new(None);
static GEN: AtomicU64 = AtomicU64::new(0);

fn held() -> std::sync::MutexGuard<'static, Option<Held>> {
    HELD.lock().unwrap_or_else(|p| p.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

mod power {
    use std::ffi::{c_char, c_void, CString};

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            s: *const c_char,
            encoding: u32,
        ) -> *mut c_void;
        fn CFRelease(cf: *mut c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: *const c_void,
            level: u32,
            name: *const c_void,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    const UTF8: u32 = 0x0800_0100;
    const LEVEL_ON: u32 = 255;
    /// What `caffeinate -d -i` asserts.
    const KINDS: [&str; 2] = ["PreventUserIdleDisplaySleep", "PreventUserIdleSystemSleep"];

    fn cf(s: &str) -> *mut c_void {
        let c = CString::new(s).expect("no NUL in a constant");
        // SAFETY: a valid NUL-terminated UTF-8 C string; the caller releases.
        unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8) }
    }

    /// Take both assertions, or none (a half-taken pair is released).
    pub fn take(name: &str) -> Result<Vec<u32>, String> {
        let name = cf(name);
        let mut ids = Vec::new();
        let mut err = None;
        for kind in KINDS {
            let k = cf(kind);
            let mut id = 0u32;
            // SAFETY: both CFStrings are live; `id` is a valid out-pointer.
            let rc = unsafe { IOPMAssertionCreateWithName(k, LEVEL_ON, name, &mut id) };
            // SAFETY: created above, released once.
            unsafe { CFRelease(k) };
            if rc == 0 {
                ids.push(id);
            } else {
                err = Some(format!("macOS refused the {kind} assertion ({rc:#x})"));
                break;
            }
        }
        // SAFETY: created above, released once.
        unsafe { CFRelease(name) };
        match err {
            Some(e) => {
                release(&ids);
                Err(e)
            }
            None => Ok(ids),
        }
    }

    pub fn release(ids: &[u32]) {
        for &id in ids {
            // SAFETY: ids came from IOPMAssertionCreateWithName, released once.
            unsafe { IOPMAssertionRelease(id) };
        }
    }
}

pub fn state() -> KeepAwake {
    match &*held() {
        Some(h) => KeepAwake {
            active: true,
            until_ms: h.until_ms,
        },
        None => KeepAwake::default(),
    }
}

/// Take the assertions (replacing a running keep-awake); returns the run's
/// generation for the ticker.
fn begin(minutes: Option<u32>, now: u64) -> Result<u64, String> {
    if minutes == Some(0) {
        return Err("pick at least one minute".into());
    }
    let ids = power::take(ASSERTION_NAME)?;
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let until_ms = minutes.map(|m| now + u64::from(m) * 60_000);
    if let Some(old) = held().replace(Held { ids, until_ms, gen }) {
        power::release(&old.ids);
    }
    Ok(gen)
}

/// Release the assertions — only those of run `gen` when given (a timer that
/// fires after a restart must not end the new run). True when it released.
fn end(gen: Option<u64>) -> bool {
    let mut h = held();
    if h.as_ref().is_some_and(|h| gen.is_none_or(|g| h.gen == g)) {
        if let Some(old) = h.take() {
            power::release(&old.ids);
            return true;
        }
    }
    false
}

fn expired(until_ms: Option<u64>, now: u64) -> bool {
    until_ms.is_some_and(|u| now >= u)
}

/// "42m", "1h", "1h 5m" — minutes rounded UP, so it never reads 0m early.
fn fmt_left(ms: u64) -> String {
    let m = ms.div_ceil(60_000).max(1);
    match (m / 60, m % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// The menu-bar title beside the glyph: time left, ∞ until turned off.
pub fn title(st: KeepAwake, now: u64) -> Option<String> {
    if !st.active {
        return None;
    }
    Some(match st.until_ms {
        Some(u) => fmt_left(u.saturating_sub(now)),
        None => "∞".to_string(),
    })
}

/// Tell the tray (title, and its menu when on/off flipped) and every window
/// (the status bar item, ⌘K).
fn publish(app: &AppHandle, flipped: bool) {
    crate::tray::show_keep_awake(app, flipped);
    let _ = app.emit("otto://keep-awake", state());
}

pub fn start(app: &AppHandle, minutes: Option<u32>) -> Result<KeepAwake, String> {
    let gen = begin(minutes, now_ms())?;
    publish(app, true);
    if minutes.is_some() {
        let app = app.clone();
        std::thread::spawn(move || loop {
            let until = match &*held() {
                Some(h) if h.gen == gen => h.until_ms,
                _ => return,
            };
            let now = now_ms();
            if expired(until, now) {
                if end(Some(gen)) {
                    publish(&app, true);
                }
                return;
            }
            let left = Duration::from_millis(until.unwrap_or(now).saturating_sub(now));
            std::thread::sleep(left.min(TICK));
            crate::tray::show_keep_awake(&app, false);
        });
    }
    Ok(state())
}

pub fn stop(app: &AppHandle) -> KeepAwake {
    if end(None) {
        publish(app, true);
    }
    state()
}

/// The global shortcut.
pub fn toggle(app: &AppHandle) {
    if state().active {
        stop(app);
    } else if let Err(e) = start(app, None) {
        crate::lifecycle::durable(&format!("keep awake failed: {e}"));
    }
}

/// A tray menu click. "Custom time" needs a text field, so it opens Otto and
/// asks the main window (its ⌘K / menu bridge prompts and calls back).
pub fn on_menu(app: &AppHandle, id: &str) {
    let minutes = match id {
        "keep-awake-15" => Some(15),
        "keep-awake-60" => Some(60),
        "keep-awake-120" => Some(120),
        "keep-awake-on" => None,
        "keep-awake-off" => {
            stop(app);
            return;
        }
        MENU_CUSTOM => {
            let _ = crate::popout::open_in_otto(app.clone(), None);
            if let Some(win) = crate::windows::primary_window(app) {
                let _ = app.emit_to(
                    tauri::EventTarget::webview(win.label()),
                    "otto://menu",
                    MENU_CUSTOM.to_string(),
                );
            }
            return;
        }
        _ => return,
    };
    if let Err(e) = start(app, minutes) {
        crate::lifecycle::durable(&format!("keep awake failed: {e}"));
    }
}

#[tauri::command]
pub fn keep_awake_state() -> KeepAwake {
    state()
}

/// `minutes: null` = until turned off.
#[tauri::command]
pub fn keep_awake_start(app: AppHandle, minutes: Option<u32>) -> Result<KeepAwake, String> {
    start(&app, minutes)
}

#[tauri::command]
pub fn keep_awake_stop(app: AppHandle) -> KeepAwake {
    stop(&app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn countdown_reads_in_whole_minutes_rounded_up() {
        assert_eq!(fmt_left(1), "1m");
        assert_eq!(fmt_left(15 * 60_000), "15m");
        assert_eq!(fmt_left(15 * 60_000 - 1), "15m");
        assert_eq!(fmt_left(60 * 60_000), "1h");
        assert_eq!(fmt_left(65 * 60_000), "1h 5m");
        assert_eq!(fmt_left(0), "1m", "never 0m while still on");
    }

    #[test]
    fn title_shows_only_while_on() {
        assert_eq!(title(KeepAwake::default(), 0), None);
        let on = KeepAwake {
            active: true,
            until_ms: None,
        };
        assert_eq!(title(on, 0).as_deref(), Some("∞"));
        let timed = KeepAwake {
            active: true,
            until_ms: Some(1_000 + 42 * 60_000),
        };
        assert_eq!(title(timed, 1_000).as_deref(), Some("42m"));
    }

    #[test]
    fn a_timed_run_expires_at_its_deadline_and_indefinite_never_does() {
        assert!(!expired(None, u64::MAX));
        assert!(!expired(Some(100), 99));
        assert!(expired(Some(100), 100));
        assert!(expired(Some(100), 5_000));
    }

    /// Our assertions as `pmset -g assertions` lists them for `pid`.
    fn pmset_lines(pid: u32) -> Vec<String> {
        let out = Command::new("/usr/bin/pmset")
            .args(["-g", "assertions"])
            .output()
            .expect("pmset runs");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains(&format!("pid {pid}(")) && l.contains(ASSERTION_NAME))
            .map(str::to_string)
            .collect()
    }

    fn wait_for(mut ok: impl FnMut() -> bool) -> bool {
        for _ in 0..50 {
            if ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// The real mechanism, checked against powerd: start → both assertions
    /// listed for this pid; a restart replaces (never stacks) them; stop →
    /// gone; a stale timer generation can't end a newer run.
    #[test]
    fn assertions_show_in_pmset_and_release_on_stop() {
        let pid = std::process::id();
        let gen = begin(Some(15), now_ms()).expect("assertions taken");
        let lines = pmset_lines(pid);
        println!("pmset while on:\n{}", lines.join("\n"));
        assert!(lines
            .iter()
            .any(|l| l.contains("PreventUserIdleDisplaySleep")));
        assert!(lines
            .iter()
            .any(|l| l.contains("PreventUserIdleSystemSleep")));
        assert!(state().active && state().until_ms.is_some());

        let gen2 = begin(None, now_ms()).expect("restart");
        assert_eq!(pmset_lines(pid).len(), 2, "a restart replaces the pair");
        assert!(!end(Some(gen)), "an old timer leaves the new run alone");
        assert_eq!(state().until_ms, None);

        assert!(end(Some(gen2)));
        assert!(pmset_lines(pid).is_empty(), "stop releases both");
        assert!(!state().active);
        assert!(begin(Some(0), now_ms()).is_err());
    }

    /// Crash: a child process takes the assertions and is SIGKILLed; powerd
    /// drops them with it (no orphan, nothing to clean up).
    #[test]
    fn a_killed_process_leaves_no_assertion_behind() {
        if std::env::var("OTTO_KEEP_AWAKE_CHILD").is_ok() {
            return; // never recurse when run as the child
        }
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "keep_awake::tests::hold_until_killed",
                "--ignored",
            ])
            .env("OTTO_KEEP_AWAKE_CHILD", "1")
            .spawn()
            .expect("child test process");
        let pid = child.id();
        assert!(wait_for(|| pmset_lines(pid).len() == 2), "child holds both");
        println!(
            "pmset with the child alive:\n{}",
            pmset_lines(pid).join("\n")
        );
        child.kill().expect("SIGKILL");
        let _ = child.wait();
        assert!(
            wait_for(|| pmset_lines(pid).is_empty()),
            "released on crash"
        );
    }

    #[test]
    #[ignore = "helper: run only as the crash test's child"]
    fn hold_until_killed() {
        if std::env::var("OTTO_KEEP_AWAKE_CHILD").is_err() {
            return;
        }
        begin(None, now_ms()).expect("child takes the assertions");
        std::thread::sleep(Duration::from_secs(30));
    }
}
