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
//! - [`guard`] runs an event callback under `catch_unwind`, so a bug in one
//!   handler is logged and skipped instead of aborting the app.

use std::io::Write;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;

fn log_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library/Logs/Otto/desktop-panic.log"))
}

fn append(line: &str) {
    if cfg!(test) {
        return; // never write the user's real log from unit tests
    }
    let Some(path) = log_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
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
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let bt = std::backtrace::Backtrace::force_capture();
        append(&format!(
            "=== panic at unix {ts} · v{} · thread '{}'\n{msg}\n  at {at}\n{bt}\n\n",
            env!("CARGO_PKG_VERSION"),
            thread.name().unwrap_or("<unnamed>"),
        ));
        default(info);
    }));
}

/// Run an AppKit-dispatched callback; a panic is logged (by the hook) and
/// contained here instead of aborting the process at the FFI boundary.
pub fn guard<R>(what: &str, f: impl FnOnce() -> R) -> Option<R> {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => Some(r),
        Err(_) => {
            append(&format!("=== contained: panic in {what} callback (skipped)\n\n"));
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
}
