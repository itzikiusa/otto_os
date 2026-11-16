//! Muse Code session-id discovery for a terminal session.
//!
//! Muse mints its own session id and takes no `--session-id`; each session
//! dir `sessions/YYYY/MM/DD/<id>/` holds a `.session.lock` naming the
//! process that owns it (`pid=<n>`). The PTY child IS that process, so its
//! pid identifies the session exactly — no cwd or timing guess. The id only
//! attributes usage: Muse has no resume argv (Threads resumes over ACP).

use std::path::{Path, PathBuf};

/// `~/.local/share/muse`.
pub(crate) fn muse_root() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".local")
        .join("share")
        .join("muse")
}

/// The id of the session whose `.session.lock` names `pid`, among the two
/// newest day dirs (a session started just before midnight).
pub(crate) fn session_for_pid(root: &Path, pid: u32) -> Option<String> {
    let want = format!("pid={pid}");
    let mut days = vec![root.join("sessions")];
    for _ in 0..3 {
        days = days.iter().flat_map(|d| sorted_dirs(d)).collect();
    }
    days.iter().rev().take(2).find_map(|day| {
        sorted_dirs(day).into_iter().find_map(|session| {
            let lock = std::fs::read_to_string(session.join(".session.lock")).ok()?;
            lock.lines()
                .any(|l| l.trim() == want)
                .then(|| session.file_name()?.to_str().map(str::to_owned))
                .flatten()
        })
    })
}

/// Sub-directories of `dir` by name (Muse's dot-named bookkeeping skipped).
#[allow(clippy::disallowed_methods)] // called via tokio::task::spawn_blocking
fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_naming_our_pid_is_our_session() {
        let root = tempfile::tempdir().unwrap();
        let lock = |day: &str, id: &str, pid: u32| {
            let dir = root.path().join("sessions").join(day).join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(".session.lock"),
                format!("pid={pid}\nhost=mac.local\n"),
            )
            .unwrap();
        };
        lock("2026/11/15", "yesterday", 41);
        lock("2026/11/16", "other", 4242);
        lock("2026/11/16", "ours", 42);
        lock("2026/10/01", "too-old", 7);
        assert_eq!(session_for_pid(root.path(), 42).as_deref(), Some("ours"));
        assert_eq!(
            session_for_pid(root.path(), 41).as_deref(),
            Some("yesterday")
        );
        // `pid=4242` must not match pid 424, and old days are not walked.
        assert_eq!(session_for_pid(root.path(), 424), None);
        assert_eq!(session_for_pid(root.path(), 7), None);
    }
}
