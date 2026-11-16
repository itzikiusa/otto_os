//! Test-only: a temp dir that must outlive the fixture that made it.
//!
//! The manager fixtures hand back a pool over an on-disk SQLite and nothing
//! that could own the `TempDir`, so they used to `mem::forget` it. Under
//! nextest every test is its own process, so each one left its migrated
//! database (~8 MiB) in the system temp folder for good — gigabytes after a
//! few runs. The dir is kept for the rest of the process and removed when it
//! exits.

use std::path::PathBuf;
use std::sync::{Mutex, Once};

static KEPT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static HOOK: Once = Once::new();

#[allow(clippy::disallowed_methods)] // runs at process exit, off any async runtime
extern "C" fn sweep() {
    // `try_lock`: never block (or panic on a poisoned lock) inside `exit`.
    if let Ok(dirs) = KEPT.try_lock() {
        for dir in dirs.iter() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Keep `dir` alive until the process exits, then delete it.
pub fn keep_until_exit(dir: tempfile::TempDir) {
    let path = dir.keep();
    KEPT.lock().unwrap_or_else(|e| e.into_inner()).push(path);
    HOOK.call_once(|| {
        // SAFETY: `sweep` is a plain `extern "C" fn()` that only removes
        // directories this module registered; `atexit` is async-signal-unsafe
        // but is called here from ordinary test code.
        unsafe {
            libc::atexit(sweep);
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_kept_dir_stays_for_the_process_and_is_registered_for_removal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        super::keep_until_exit(dir);
        assert!(path.is_dir(), "still there while the process runs");
        assert!(super::KEPT.lock().unwrap().contains(&path));
        // What the exit hook does.
        super::sweep();
        assert!(!path.exists(), "removed by the exit sweep");
    }
}
