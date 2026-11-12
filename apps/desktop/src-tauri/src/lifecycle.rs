//! Durable lifecycle log for the desktop shell: `~/Library/Logs/Otto/desktop.log`.
//!
//! A GUI app's stderr goes nowhere, and an app killed outright (SIGKILL,
//! memory pressure) leaves no crash report — the 2026-11-03 "the UI stopped"
//! left no trace at all. This log records, one line per event:
//!
//! - **launch** (version, pid) and the previous run's fate: a `desktop.running`
//!   marker is written at launch and removed on a clean exit, so a marker found
//!   at the next launch means the previous run died — logged with that run's
//!   last event;
//! - window close requests and destroys (label, user close vs programmatic
//!   destroy, app windows remaining), every `app.exit` with its reason, and
//!   `RunEvent::ExitRequested` / `Exit`;
//! - SIGTERM / SIGINT / SIGHUP (then a deadline-bounded quit), panics,
//!   contained panics, WebContent process crashes, uncaught NSExceptions;
//! - a heartbeat every [`HEARTBEAT`], bounding when a silent death happened.
//!
//! Lines are `<UTC time> pid=<pid> <event>`, written whole through an
//! `O_APPEND` file; events on an exit or crash path are fsynced. The file is
//! capped at [`MAX_LOG_BYTES`] and rotates to `desktop.log.1`. Nothing but
//! window labels and fixed event text is recorded — no paths, URLs or content.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Past this size the log rotates to `desktop.log.1` before the next write.
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// How often a live app writes `alive`.
pub const HEARTBEAT: Duration = Duration::from_secs(15 * 60);
/// The tail read for "the previous run's last event".
const TAIL_BYTES: u64 = 4096;

/// One log + its run marker (separate files in one directory).
pub struct Log {
    path: PathBuf,
    marker: PathBuf,
    max: u64,
    lock: Mutex<()>,
}

static LOG: OnceLock<Log> = OnceLock::new();

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix time (days → civil date, Howard Hinnant).
fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn now() -> String {
    iso_utc(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

impl Log {
    pub fn new(dir: &Path, max: u64) -> Self {
        Self {
            path: dir.join("desktop.log"),
            marker: dir.join("desktop.running"),
            max,
            lock: Mutex::new(()),
        }
    }

    /// Append one line (`sync`: fsync it — exit / crash paths).
    pub fn write(&self, event: &str, sync: bool) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        self.rotate();
        // One line per event; a newline inside the text would forge one.
        let event = event.replace(['\n', '\r'], " ");
        let line = format!("{} pid={} {event}\n", now(), std::process::id());
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = f.write_all(line.as_bytes());
            if sync {
                let _ = f.sync_all();
            }
        }
    }

    /// Move a full log aside (replacing an older `.1`).
    fn rotate(&self) {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return;
        };
        if meta.len() <= self.max {
            return;
        }
        let mut rotated = self.path.as_os_str().to_owned();
        rotated.push(".1");
        let _ = std::fs::rename(&self.path, PathBuf::from(rotated));
    }

    /// The last non-empty line of the log (bounded tail read).
    fn last_line(&self) -> Option<String> {
        let mut f = File::open(&self.path).ok()?;
        let len = f.metadata().ok()?.len();
        f.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
            .ok()?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        String::from_utf8_lossy(&buf)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .map(str::to_string)
    }

    /// Launch: report an unclean previous run (marker still present), then
    /// arm the marker for this run. Returns the unclean-exit note, if any.
    pub fn start(&self, version: &str) -> Option<String> {
        let note = std::fs::read_to_string(&self.marker).ok().map(|run| {
            let last = self
                .last_line()
                .unwrap_or_else(|| "(log empty)".to_string());
            format!(
                "previous run ({}) ended without a clean exit; last event: {last}",
                run.trim()
            )
        });
        if let Some(n) = &note {
            self.write(n, false);
        }
        self.write(&format!("launch v{version}"), true);
        if let Some(dir) = self.marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(
            &self.marker,
            format!("pid={} started={}", std::process::id(), now()),
        );
        note
    }

    /// Clean exit: log it (fsynced) and disarm the marker.
    pub fn finish(&self, event: &str) {
        self.write(event, true);
        let _ = std::fs::remove_file(&self.marker);
    }
}

/// Open the real log and record the launch. Call first in `main`.
pub fn init() {
    if cfg!(test) {
        return; // never touch the user's real log from unit tests
    }
    let Some(dir) = dirs::home_dir().map(|h| h.join("Library/Logs/Otto")) else {
        return;
    };
    let log = LOG.get_or_init(|| Log::new(&dir, MAX_LOG_BYTES));
    log.start(env!("CARGO_PKG_VERSION"));
}

/// Record an event (no fsync).
pub fn event(what: &str) {
    if let Some(l) = LOG.get() {
        l.write(what, false);
    }
}

/// Record an event on an exit / crash path (fsynced).
pub fn durable(what: &str) {
    if let Some(l) = LOG.get() {
        l.write(what, true);
    }
}

/// The process is exiting cleanly: fsync the line and clear the marker.
pub fn clean_exit(what: &str) {
    if let Some(l) = LOG.get() {
        l.finish(what);
    }
}

/// Log SIGTERM / SIGINT / SIGHUP from a thread (signal-hook's self-pipe keeps
/// the handler async-signal-safe), then quit: gracefully under a hard
/// deadline once the app is up, else the signal's DEFAULT action
/// (`quit::handle_signal`).
pub fn watch_signals() {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGTERM, SIGINT, SIGHUP]) else {
        event("signal watch unavailable");
        return;
    };
    let _ = std::thread::Builder::new()
        .name("otto-signals".into())
        .spawn(move || {
            for sig in signals.forever() {
                let name = signal_hook::low_level::signal_name(sig).unwrap_or("signal");
                crate::quit::handle_signal(sig, name);
            }
        });
}

/// Write `alive` every [`HEARTBEAT`] with the app-window count from `count`.
pub fn heartbeat(count: impl Fn() -> usize + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("otto-heartbeat".into())
        .spawn(move || loop {
            std::thread::sleep(HEARTBEAT);
            event(&format!("alive · app windows={}", count()));
        });
}

/// Labels with a close request in flight, so a `Destroyed` can tell a
/// requested close from a programmatic destroy (`destroy()` skips
/// CloseRequested).
static CLOSING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A `CloseRequested` (title-bar ×, ⌘⇧W, or `close()` from code — Tauri
/// routes both through it); `remaining` = app windows left if it closes.
pub fn close_requested(label: &str, remaining: usize) {
    if let Ok(mut c) = CLOSING.lock() {
        if !c.iter().any(|l| l == label) {
            c.push(label.to_string());
        }
    }
    event(&format!(
        "close requested label={label} · app windows remaining={remaining}"
    ));
}

/// A `Destroyed`: after a close request, or a programmatic `destroy()`.
pub fn destroyed(label: &str, remaining: usize) {
    let requested = CLOSING
        .lock()
        .map(|mut c| {
            let had = c.iter().any(|l| l == label);
            c.retain(|l| l != label);
            had
        })
        .unwrap_or(false);
    let how = if requested {
        "after close request"
    } else {
        "programmatic destroy"
    };
    event(&format!(
        "window destroyed label={label} ({how}) · app windows remaining={remaining}"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("otto-lifecycle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn lines(log: &Log) -> Vec<String> {
        std::fs::read_to_string(&log.path)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn a_clean_exit_leaves_no_unclean_note_at_the_next_launch() {
        let d = tmp("clean");
        let log = Log::new(&d, MAX_LOG_BYTES);
        assert_eq!(log.start("1.0"), None, "first launch ever");
        assert!(log.marker.exists(), "marker armed while running");
        log.finish("exit (clean)");
        assert!(!log.marker.exists(), "marker cleared on a clean exit");
        assert_eq!(log.start("1.0"), None);
        let l = lines(&log);
        assert!(
            l[0].ends_with(" launch v1.0") && l[0].contains(" pid="),
            "{l:?}"
        );
        assert!(l[1].ends_with(" exit (clean)"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_run_that_died_is_reported_with_its_last_event() {
        let d = tmp("unclean");
        let log = Log::new(&d, MAX_LOG_BYTES);
        log.start("1.0");
        log.write("close requested label=w2 (user) app windows=2", false);
        // No finish(): the process "died". Next launch:
        let note = log.start("1.1").expect("unclean exit detected");
        assert!(note.starts_with("previous run (pid="), "{note}");
        assert!(note.contains("started=") && note.contains("ended without a clean exit"));
        assert!(
            note.ends_with("close requested label=w2 (user) app windows=2"),
            "{note}"
        );
        let l = lines(&log);
        assert!(l[2].contains("previous run"), "{l:?}");
        assert!(l[3].ends_with("launch v1.1"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_log_rotates_past_its_cap_and_lines_stay_whole() {
        let d = tmp("rotate");
        let log = Log::new(&d, 200);
        for i in 0..20 {
            log.write(&format!("event {i}\nforged line"), false);
        }
        let rotated = d.join("desktop.log.1");
        assert!(rotated.exists(), "rotated aside");
        let cur = std::fs::metadata(&log.path).unwrap().len();
        assert!(cur <= 200 + 100, "current log stays near the cap: {cur}");
        for l in lines(&log) {
            assert!(l.contains(" pid=") && !l.starts_with("forged"), "{l}");
        }
        assert!(log.last_line().unwrap().ends_with("event 19 forged line"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn utc_timestamps_are_civil_dates() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso_utc(1_793_728_136), "2026-11-03T17:48:56Z");
    }
}
