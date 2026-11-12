//! A quit that always finishes: graceful first, then a hard deadline.
//!
//! `packaging/deploy.sh` quits the app (AppleScript, then SIGTERM) and gives
//! each attempt a few seconds; "app did not exit" meant a quit that started
//! but never finished — a wedged main thread never ran the exit request, or
//! the process hung after `applicationWillTerminate` (tao's `LoopDestroyed`,
//! Tauri's cleanup, `exit()`'s atexit work). Nothing bounded either. Now:
//!
//! - every explicit quit (menu Quit, `app.exit`, the last app window closing,
//!   AppKit's `terminate:` from an AppleScript quit) [`arm`]s a one-shot
//!   deadline: if the process is still alive [`DEADLINE`] later it logs and
//!   `_exit`s;
//! - SIGTERM / SIGINT / SIGHUP request the same graceful `app.exit(0)` (so the
//!   window set is still snapshotted) under that deadline; a second signal,
//!   or one before the app is up, takes the signal's default action at once.
//!
//! Closing a window that is not the last one never arms anything.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// How long a requested quit may take before the process is ended. Short
/// enough to fit inside deploy.sh's waits (6 s after AppleScript, 5 s after
/// SIGTERM); a healthy quit (snapshot + teardown) takes well under a second.
pub const DEADLINE: Duration = Duration::from_secs(3);

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static ARMED: AtomicBool = AtomicBool::new(false);

/// Make the running app reachable from the signal thread (call in `setup`).
pub fn register(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
}

/// What a termination signal does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnSignal {
    /// Ask the app to quit (window snapshot + teardown) under [`DEADLINE`].
    Graceful,
    /// The signal's default action now: no app to ask yet, or a quit is
    /// already under way and this is the sender insisting.
    Default,
}

/// Graceful only the first time, and only once the app exists.
pub fn on_signal(app_ready: bool, already_quitting: bool) -> OnSignal {
    if app_ready && !already_quitting {
        OnSignal::Graceful
    } else {
        OnSignal::Default
    }
}

/// Start the one-shot quit deadline (later calls are no-ops).
pub fn arm(reason: &str) {
    let reason = reason.to_string();
    arm_with(&ARMED, DEADLINE, move || {
        crate::lifecycle::clean_exit(&format!(
            "exit forced: still running {}s after {reason}",
            DEADLINE.as_secs()
        ));
        // `_exit`, not `exit`: the hang may be in the atexit work itself.
        signal_hook::low_level::exit(0);
    });
}

/// [`arm`] with an injectable flag, delay and end — returns whether this call
/// started the timer.
fn arm_with(flag: &AtomicBool, after: Duration, end: impl FnOnce() + Send + 'static) -> bool {
    if flag.swap(true, Ordering::SeqCst) {
        return false;
    }
    std::thread::Builder::new()
        .name("otto-quit-deadline".into())
        .spawn(move || {
            std::thread::sleep(after);
            end();
        })
        .is_ok()
}

/// The signal thread's handler: graceful quit under the deadline, or the
/// default action (the pre-existing behaviour).
pub fn handle_signal(sig: i32, name: &str) {
    match on_signal(APP.get().is_some(), ARMED.load(Ordering::SeqCst)) {
        OnSignal::Graceful => {
            crate::lifecycle::durable(&format!(
                "{name} received — quitting (forced after {}s)",
                DEADLINE.as_secs()
            ));
            arm(name);
            if let Some(app) = APP.get() {
                app.exit(0);
            }
        }
        OnSignal::Default => {
            crate::lifecycle::durable(&format!("{name} received — terminating (default action)"));
            let _ = signal_hook::low_level::emulate_default_handler(sig);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_first_signal_quits_gracefully_and_a_repeat_or_early_one_does_not() {
        assert_eq!(on_signal(true, false), OnSignal::Graceful);
        assert_eq!(on_signal(true, true), OnSignal::Default, "sender insists");
        assert_eq!(on_signal(false, false), OnSignal::Default, "app not up yet");
        assert_eq!(on_signal(false, true), OnSignal::Default);
    }

    #[test]
    fn the_deadline_fires_once_however_often_it_is_armed() {
        let flag = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        assert!(arm_with(&flag, Duration::from_millis(10), move || {
            let _ = tx.send("first");
        }));
        assert!(!arm_with(&flag, Duration::from_millis(10), move || {
            let _ = tx2.send("second");
        }));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok("first"));
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "a second arm never starts a second timer"
        );
    }

    #[test]
    fn the_deadline_waits_before_ending() {
        let flag = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();
        let started = std::time::Instant::now();
        arm_with(&flag, Duration::from_millis(50), move || {
            let _ = tx.send(started.elapsed());
        });
        let waited = rx.recv_timeout(Duration::from_secs(5)).expect("fired");
        assert!(waited >= Duration::from_millis(50), "{waited:?}");
    }

    #[test]
    fn the_deadline_fits_inside_deploys_wait_after_sigterm() {
        // deploy.sh polls 10 × 0.5 s after `kill -TERM`.
        const { assert!(DEADLINE.as_secs() < 5) };
    }
}
