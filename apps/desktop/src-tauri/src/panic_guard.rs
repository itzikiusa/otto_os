//! Panic visibility + containment for the desktop shell.
//!
//! The shell's callbacks run inside AppKit's Objective-C event dispatch
//! (`NSApplication sendEvent:` → tao → our run/menu handlers). A Rust panic
//! can't unwind through those frames, so it becomes `panic_cannot_unwind` →
//! `abort()` — the whole app dies, and because a GUI app's stderr goes
//! nowhere, the panic message is lost (the crash report only shows the abort).
//!
//! - [`install`] appends every panic (message, location, thread, backtrace) to
//!   `~/Library/Logs/Otto/desktop-panic.log` before the default hook runs.
//!   The log is capped ([`MAX_LOG_BYTES`]), and a panic repeating at the same
//!   site within [`REPEAT_WINDOW`] is counted, not re-logged: a contained bug
//!   in a per-frame handler (`Resized`/`Moved`) must neither grow the file
//!   without bound nor capture a backtrace on the main thread every frame.
//! - [`guard`] runs an event callback under `catch_unwind`, so a bug in one
//!   handler is logged and skipped instead of aborting the app.

use std::collections::HashMap;
use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Past this size the panic log is truncated before the next write.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// A panic at the same site + message within this window is only counted.
const REPEAT_WINDOW: Duration = Duration::from_secs(60);

/// Per-site throttle for identical panics.
#[derive(Default)]
struct RepeatFilter {
    /// site key → (when it was last fully logged, repeats suppressed since).
    seen: HashMap<String, (Instant, u64)>,
}

impl RepeatFilter {
    /// `Some(n)` → log in full (`n` = repeats suppressed since the last full
    /// entry); `None` → a repeat inside the window, skip it.
    fn admit(&mut self, key: &str, now: Instant) -> Option<u64> {
        match self.seen.get_mut(key) {
            Some((last, suppressed)) if now.duration_since(*last) < REPEAT_WINDOW => {
                *suppressed += 1;
                None
            }
            Some((last, suppressed)) => {
                let n = *suppressed;
                *last = now;
                *suppressed = 0;
                Some(n)
            }
            None => {
                // Bounded: a pathological flood of DISTINCT sites resets it.
                if self.seen.len() >= 256 {
                    self.seen.clear();
                }
                self.seen.insert(key.to_string(), (now, 0));
                Some(0)
            }
        }
    }
}

static REPEATS: Mutex<Option<RepeatFilter>> = Mutex::new(None);

/// [`RepeatFilter::admit`] on the process-wide filter (poison-tolerant: a
/// panic must never silence the panic log).
fn admit_site(key: &str) -> Option<u64> {
    REPEATS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(RepeatFilter::default)
        .admit(key, Instant::now())
}

/// Truncate `path` when it has grown past `max` bytes. Returns the old size.
fn cap_log(path: &Path, max: u64) -> Option<u64> {
    let len = std::fs::metadata(path).ok()?.len();
    if len <= max {
        return None;
    }
    let f = std::fs::OpenOptions::new().write(true).open(path).ok()?;
    f.set_len(0).ok()?;
    Some(len)
}

fn log_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library/Logs/Otto/desktop-panic.log"))
}

pub(crate) fn append(line: &str) {
    if cfg!(test) {
        return; // never write the user's real log from unit tests
    }
    let Some(path) = log_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let truncated = cap_log(&path, MAX_LOG_BYTES);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        if let Some(was) = truncated {
            let _ = writeln!(f, "=== log truncated (was {was} bytes)\n");
        }
        let _ = f.write_all(line.as_bytes());
    }
}

/// Install the logging panic hook. Keeps the default hook (stderr) after it.
pub fn install() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let at = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "?".into());
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".into());
        let admitted = admit_site(&format!("{at}\u{0}{msg}"));
        if let Some(suppressed) = admitted {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let bt = std::backtrace::Backtrace::force_capture();
            let repeats = if suppressed > 0 {
                format!(" · {suppressed} identical repeat(s) since the last entry")
            } else {
                String::new()
            };
            append(&format!(
                "=== panic at unix {ts} · v{} · thread '{}'{repeats}\n{msg}\n  at {at}\n{bt}\n\n",
                env!("CARGO_PKG_VERSION"),
                thread.name().unwrap_or("<unnamed>"),
            ));
            // The default hook prints to stderr (nowhere for a GUI app) — and
            // would do it every frame; only for admitted entries.
            default(info);
        }
    }));
}

/// Run an AppKit-dispatched callback; a panic is logged (by the hook) and
/// contained here instead of aborting the process at the FFI boundary.
pub fn guard<R>(what: &str, f: impl FnOnce() -> R) -> Option<R> {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => Some(r),
        Err(_) => {
            // One line per admitted panic, not per frame.
            let admit = admit_site(&format!("contained\u{0}{what}"));
            if admit.is_some() {
                append(&format!(
                    "=== contained: panic in {what} callback (skipped)\n\n"
                ));
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_contains_a_panic_and_passes_values_through() {
        assert_eq!(guard("test", || 7), Some(7));
        let prev = panic::take_hook();
        panic::set_hook(Box::new(|_| {})); // keep test output quiet
        let r: Option<()> = guard("test", || panic!("boom"));
        panic::set_hook(prev);
        assert!(r.is_none());
    }

    #[test]
    fn identical_panics_are_counted_not_relogged() {
        let mut f = RepeatFilter::default();
        let t0 = Instant::now();
        assert_eq!(f.admit("a.rs:1\u{0}boom", t0), Some(0));
        for i in 1..=5 {
            assert_eq!(
                f.admit("a.rs:1\u{0}boom", t0 + Duration::from_millis(16 * i)),
                None
            );
        }
        // A different site is its own entry.
        assert_eq!(f.admit("b.rs:2\u{0}boom", t0), Some(0));
        // After the window: logged again, carrying the suppressed count.
        assert_eq!(
            f.admit(
                "a.rs:1\u{0}boom",
                t0 + REPEAT_WINDOW + Duration::from_secs(1)
            ),
            Some(5)
        );
    }

    #[test]
    fn panic_log_is_truncated_past_the_cap() {
        let path = std::env::temp_dir().join(format!("otto-panic-cap-{}", std::process::id()));
        std::fs::write(&path, vec![b'x'; 2048]).unwrap();
        assert_eq!(cap_log(&path, 4096), None);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 2048);
        assert_eq!(cap_log(&path, 1024), Some(2048));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        let _ = std::fs::remove_file(&path);
    }
}
