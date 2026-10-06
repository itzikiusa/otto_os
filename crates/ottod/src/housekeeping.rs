//! Boot-time housekeeping for the data dir: the unclean-shutdown marker and a
//! deliberately narrow sweep of files that earlier versions left behind.
//!
//! The data dir is USER DATA, so the sweep matches only exact, known-dead
//! patterns and never recurses:
//! - `bin/ottod.resigned.*` older than [`RESIGNED_MAX_AGE`] (re-sign leftovers
//!   from manual codesign repairs);
//! - `<name>-shm` / `<name>-wal` in the data dir whose `<name>` (a `*.bak`
//!   SQLite copy) no longer exists — an orphaned sidecar is never readable;
//! - a `state.db` of exactly 0 bytes (an aborted create from an old build;
//!   any non-empty file is kept).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// `bin/ottod.resigned.*` files older than this are swept.
pub const RESIGNED_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

/// launchd appends the daemon's stderr to one file forever; once it passes
/// this size the next boot truncates it (the daemon's own log rotates daily).
pub const STDERR_LOG_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// Marker present while a daemon runs; left behind by a crash or SIGKILL.
pub fn running_marker(data_dir: &Path) -> PathBuf {
    data_dir.join("ottod.running")
}

/// Write the running marker. Returns the PREVIOUS marker's contents when one
/// was still present — i.e. the last run never reached a clean exit.
pub fn mark_running(data_dir: &Path, pid: u32, now: SystemTime) -> Option<String> {
    let path = running_marker(data_dir);
    let previous = std::fs::read_to_string(&path).ok();
    let started = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Err(e) = std::fs::write(&path, format!("pid={pid} started_unix={started}\n")) {
        tracing::warn!("cannot write {}: {e}", path.display());
    }
    previous.map(|s| s.trim().to_string())
}

/// Remove the running marker on a clean exit.
pub fn clear_running(data_dir: &Path) {
    let _ = std::fs::remove_file(running_marker(data_dir));
}

/// Remove the dead files described in the module docs. Returns what was
/// removed (for the log). Every failure is skipped silently: best-effort.
// Sync by design: ottod runs it on the blocking pool after the listener starts.
#[allow(clippy::disallowed_methods)]
pub fn sweep_data_dir(data_dir: &Path, now: SystemTime) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let mut remove = |p: PathBuf| {
        if std::fs::remove_file(&p).is_ok() {
            removed.push(p);
        }
    };

    // 1. Stale re-signed daemon copies.
    if let Ok(rd) = std::fs::read_dir(data_dir.join("bin")) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("ottod.resigned.") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let old = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age > RESIGNED_MAX_AGE);
            if old {
                remove(entry.path());
            }
        }
    }

    // 2. Orphaned SQLite sidecars of `*.bak` copies, data dir root only.
    if let Ok(rd) = std::fs::read_dir(data_dir) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(main) = name
                .strip_suffix("-shm")
                .or_else(|| name.strip_suffix("-wal"))
            else {
                continue;
            };
            if !main.ends_with(".bak") {
                continue;
            }
            let is_file = entry.metadata().map(|m| m.is_file()).unwrap_or(false);
            if is_file && !data_dir.join(main).exists() {
                remove(entry.path());
            }
        }
    }

    // 3. A zero-byte `state.db` (exactly 0 bytes — anything else is kept).
    let state = data_dir.join("state.db");
    if std::fs::symlink_metadata(&state).is_ok_and(|m| m.is_file() && m.len() == 0) {
        remove(state);
    }

    removed
}

/// Truncate launchd's append-only stderr log once it outgrows
/// [`STDERR_LOG_MAX_BYTES`]. launchd opens it O_APPEND, so truncating in place
/// is safe while the file is held open. Returns the size it was cut from.
pub fn cap_stderr_log(path: &Path, max: u64) -> Option<u64> {
    let len = std::fs::metadata(path).ok()?.len();
    if len <= max {
        return None;
    }
    let f = std::fs::OpenOptions::new().write(true).open(path).ok()?;
    f.set_len(0).ok()?;
    Some(len)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // sync test scaffolding
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ottod-housekeeping-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        dir
    }

    #[test]
    fn sweep_removes_only_the_exact_dead_patterns() {
        let dir = scratch("sweep");
        let w = |rel: &str, body: &str| std::fs::write(dir.join(rel), body).unwrap();
        // Re-signed copies: one old (judged with a clock 8 days ahead), and
        // look-alikes that must survive.
        w("bin/ottod.resigned.1", "x");
        w("bin/ottod", "live daemon");
        w("bin/ottod.prev", "previous daemon");
        w("bin/ottod.bak.123", "deploy backup");
        // Orphaned vs owned sidecars.
        w("otto.db.bak-shm", "");
        w("otto.db.bak-wal", "");
        w("keep.db.bak", "main");
        w("keep.db.bak-wal", "owned sidecar");
        // The LIVE database's sidecars are never touched, main present or not.
        w("otto.db", "live");
        w("otto.db-wal", "live wal");
        w("gone.db-shm", "not a .bak sidecar");
        // Zero-byte state.db goes; a non-empty one stays (tested below).
        w("state.db", "");

        let now = SystemTime::now();
        // Same clock: nothing is old enough yet.
        let removed = sweep_data_dir(&dir, now);
        assert!(!removed.contains(&dir.join("bin/ottod.resigned.1")));
        let later = now + RESIGNED_MAX_AGE + Duration::from_secs(86400);
        let removed: std::collections::HashSet<_> = sweep_data_dir(&dir, later)
            .into_iter()
            .chain(removed)
            .collect();
        let expected: std::collections::HashSet<_> = [
            "bin/ottod.resigned.1",
            "otto.db.bak-shm",
            "otto.db.bak-wal",
            "state.db",
        ]
        .iter()
        .map(|r| dir.join(r))
        .collect();
        assert_eq!(removed, expected);
        for kept in [
            "bin/ottod",
            "bin/ottod.prev",
            "bin/ottod.bak.123",
            "keep.db.bak",
            "keep.db.bak-wal",
            "otto.db",
            "otto.db-wal",
            "gone.db-shm",
        ] {
            assert!(dir.join(kept).exists(), "{kept} must survive");
        }

        std::fs::write(dir.join("state.db"), "data").unwrap();
        assert!(sweep_data_dir(&dir, later).is_empty());
        assert!(dir.join("state.db").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn running_marker_reports_an_unclean_previous_run() {
        let dir = scratch("marker");
        let now = SystemTime::now();
        assert_eq!(mark_running(&dir, 1, now), None, "first boot is clean");
        clear_running(&dir);
        assert_eq!(mark_running(&dir, 2, now), None, "clean exit cleared it");
        // No clear → the next boot sees the previous run's marker.
        let prev = mark_running(&dir, 3, now).expect("unclean");
        assert!(prev.starts_with("pid=2 "), "{prev}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stderr_log_is_capped_in_place() {
        let dir = scratch("stderr");
        let log = dir.join("ottod.stderr.log");
        assert_eq!(cap_stderr_log(&log, 10), None, "missing file is fine");
        std::fs::write(&log, "small").unwrap();
        assert_eq!(cap_stderr_log(&log, 10), None);
        std::fs::write(&log, "much more than ten bytes").unwrap();
        assert_eq!(cap_stderr_log(&log, 10), Some(24));
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
