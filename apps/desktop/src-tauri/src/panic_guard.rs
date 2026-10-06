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
//!   The log is capped ([`MAX_LOG_BYTES`]; the full file rotates to
//!   `desktop-panic.log.1`, so the oldest entry — usually the root cause — is
//!   kept one generation), and a panic repeating at the same SITE within
//!   [`REPEAT_WINDOW`] is counted, not re-logged, whatever its message
//!   ("index 7 out of bounds", coordinates vary per frame): a contained bug
//!   in a per-frame handler (`Resized`/`Moved`) must neither grow the file
//!   without bound nor capture a backtrace on the main thread every frame.
//! - [`guard`] runs an event callback under `catch_unwind`, so a bug in one
//!   handler is logged and skipped instead of aborting the app.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Past this size the panic log is rotated to `.1` before the next write.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// A panic at the same site within this window is only counted.
const REPEAT_WINDOW: Duration = Duration::from_secs(60);

/// Distinct messages remembered per site (the count saturates here).
const MAX_DISTINCT: usize = 64;

/// One site's throttle state.
struct Site {
    /// When it was last fully logged.
    last: Instant,
    /// Repeats suppressed since.
    suppressed: u64,
    /// Hashes of the distinct messages among those repeats (capped).
    distinct: HashSet<u64>,
}

/// What a suppressed run looked like, reported on the next full entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Suppressed {
    repeats: u64,
    /// Distinct messages among `repeats` (≤ [`MAX_DISTINCT`]).
    distinct: usize,
}

/// Per-SITE throttle (keyed by location only: a per-frame panic whose
/// message varies must not slip past it).
#[derive(Default)]
struct RepeatFilter {
    seen: HashMap<String, Site>,
}

fn message_hash(msg: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    msg.hash(&mut h);
    h.finish()
}

impl RepeatFilter {
    /// `Some(s)` → log in full (`s` = what was suppressed since the last full
    /// entry at this site); `None` → a repeat inside the window, skip it.
    fn admit(&mut self, site: &str, msg: &str, now: Instant) -> Option<Suppressed> {
        match self.seen.get_mut(site) {
            Some(s) if now.duration_since(s.last) < REPEAT_WINDOW => {
                s.suppressed += 1;
                if s.distinct.len() < MAX_DISTINCT {
                    s.distinct.insert(message_hash(msg));
                }
                None
            }
            Some(s) => {
                let out = Suppressed {
                    repeats: s.suppressed,
                    distinct: s.distinct.len(),
                };
                s.last = now;
                s.suppressed = 0;
                s.distinct.clear();
                Some(out)
            }
            None => {
                // Bounded: a pathological flood of DISTINCT sites resets it.
                if self.seen.len() >= 256 {
                    self.seen.clear();
                }
                self.seen.insert(
                    site.to_string(),
                    Site {
                        last: now,
                        suppressed: 0,
                        distinct: HashSet::new(),
                    },
                );
                Some(Suppressed {
                    repeats: 0,
                    distinct: 0,
                })
            }
        }
    }
}

static REPEATS: Mutex<Option<RepeatFilter>> = Mutex::new(None);

/// [`RepeatFilter::admit`] on the process-wide filter (poison-tolerant: a
/// panic must never silence the panic log).
fn admit_site(site: &str, msg: &str) -> Option<Suppressed> {
    REPEATS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(RepeatFilter::default)
        .admit(site, msg, Instant::now())
}

/// Rotate `path` to `<path>.1` (replacing an older `.1`) when it has grown
/// past `max` bytes — truncating would discard the OLDEST entry, usually the
/// root-cause panic. Returns the old size.
fn cap_log(path: &Path, max: u64) -> Option<u64> {
    let len = std::fs::metadata(path).ok()?.len();
    if len <= max {
        return None;
    }
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    std::fs::rename(path, PathBuf::from(rotated)).ok()?;
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
            let _ = writeln!(
                f,
                "=== log rotated to desktop-panic.log.1 (was {was} bytes)\n"
            );
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
        let admitted = admit_site(&at, &msg);
        if let Some(suppressed) = admitted {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let bt = std::backtrace::Backtrace::force_capture();
            let repeats = if suppressed.repeats > 0 {
                format!(
                    " · {} repeat(s) at this site since the last entry ({} distinct message(s))",
                    suppressed.repeats, suppressed.distinct
                )
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
            let admit = admit_site(&format!("contained\u{0}{what}"), "");
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
    fn repeats_at_a_site_are_counted_not_relogged_whatever_the_message() {
        let mut f = RepeatFilter::default();
        let t0 = Instant::now();
        let fresh = Suppressed {
            repeats: 0,
            distinct: 0,
        };
        assert_eq!(f.admit("a.rs:1", "index 0 out of bounds", t0), Some(fresh));
        // S10-308: per-frame panics with VARYING messages are still repeats.
        for i in 1..=5u64 {
            let msg = format!("index {} out of bounds", i % 3);
            assert_eq!(
                f.admit("a.rs:1", &msg, t0 + Duration::from_millis(16 * i)),
                None
            );
        }
        // A different site is its own entry.
        assert_eq!(f.admit("b.rs:2", "boom", t0), Some(fresh));
        // After the window: logged again, carrying the suppressed count and
        // how many distinct messages hid in it.
        assert_eq!(
            f.admit("a.rs:1", "x", t0 + REPEAT_WINDOW + Duration::from_secs(1)),
            Some(Suppressed {
                repeats: 5,
                distinct: 3
            })
        );
    }

    #[test]
    fn panic_log_rotates_past_the_cap_keeping_the_oldest_entry() {
        let dir = std::env::temp_dir().join(format!("otto-panic-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("desktop-panic.log");
        let rotated = dir.join("desktop-panic.log.1");
        let mut first = b"=== root cause\n".to_vec();
        first.extend(vec![b'x'; 2048]);
        std::fs::write(&path, &first).unwrap();
        assert_eq!(cap_log(&path, 4096), None);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), first.len() as u64);
        assert_eq!(cap_log(&path, 1024), Some(first.len() as u64));
        assert!(!path.exists(), "the full log moved aside");
        assert_eq!(std::fs::read(&rotated).unwrap(), first, "oldest entry kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
