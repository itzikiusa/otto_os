//! Session RAM/resumability lifecycle helpers.
//!
//! Two background concerns, both driven from ottod:
//!
//! 1. **Suspend** an idle, unattached, resumable LIVE session — release its
//!    RAM-holding PTY without losing the conversation (it stays resumable via
//!    `--resume`). Lives on [`crate::manager::SessionManager`]; the decision
//!    logic that picks which sessions to suspend is [`should_suspend`].
//! 2. **Prune** non-live sessions whose provider-side transcript no longer
//!    exists (un-resumable) — the existence checks here ([`Resumability`] /
//!    [`claude_transcript_exists`]) tell the manager when a row is safe to
//!    delete. We only ever prune what we can *positively confirm* is gone.
//!
//! Path encoding for claude (verified against a live `~/.claude/projects`):
//! the project directory is the session cwd with every `/`, `.` and `_`
//! replaced by `-`, and the transcript file is `<provider_session_id>.jsonl`
//! inside it. e.g. cwd `/Users/dev/project` →
//! `~/.claude/projects/-Users-dev-project/<sid>.jsonl`.

use std::path::{Path, PathBuf};

/// Outcome of a transcript existence check for a non-live session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resumability {
    /// Provider transcript is positively present → keep the session.
    Exists,
    /// Provider transcript is positively gone → safe to prune the row.
    Gone,
    /// Cannot determine (unknown provider, no session id, no HOME, …) →
    /// KEEP the session. We never prune what we can't verify.
    Unknown,
}

/// Encode a session cwd into the claude `~/.claude/projects` directory name:
/// every `/`, `.` and `_` becomes `-`. (Verified: `/Users/dev/project`
/// → `-Users-dev-project`; a leading `/` yields the leading `-`.)
pub fn claude_project_dir_name(cwd: &str) -> String {
    cwd.chars()
        .map(|c| match c {
            '/' | '.' | '_' => '-',
            other => other,
        })
        .collect()
}

/// Resolve the expected claude transcript path for a session under `home`,
/// using the exact cwd→dir encoding.
pub fn claude_transcript_path(home: &Path, cwd: &str, provider_session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(claude_project_dir_name(cwd))
        .join(format!("{provider_session_id}.jsonl"))
}

/// Does a claude transcript for `provider_session_id` exist under `home`?
///
/// Primary check: the exact cwd-encoded path. Fallback (covers cwd-encoding
/// edge cases, e.g. the session was created from a slightly different cwd
/// string): scan every immediate subdirectory of `~/.claude/projects` for a
/// `<provider_session_id>.jsonl`. Returns `Exists`/`Gone`; never `Unknown`
/// (the caller decides `Unknown` when it has no home/session id).
pub fn claude_transcript_exists(
    home: &Path,
    cwd: &str,
    provider_session_id: &str,
) -> Resumability {
    // Fast path: exact encoded location.
    if claude_transcript_path(home, cwd, provider_session_id).is_file() {
        return Resumability::Exists;
    }
    // Fallback: the file is named exactly `<sid>.jsonl`; look in every project
    // directory. This makes a positive "exists" reliable even if the stored
    // cwd doesn't encode 1:1 to the on-disk project dir.
    let projects = home.join(".claude").join("projects");
    let target = format!("{provider_session_id}.jsonl");
    if let Ok(entries) = std::fs::read_dir(&projects) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            if entry.path().join(&target).is_file() {
                return Resumability::Exists;
            }
        }
    }
    Resumability::Gone
}

/// Decide whether a non-live agent session's conversation can still be resumed
/// by its provider, by checking the on-disk transcript.
///
/// - `claude`: existence of `<sid>.jsonl` under `~/.claude/projects`.
/// - any other provider (incl. `codex`, which Otto doesn't resume so never
///   stores a `provider_session_id`): `Unknown` — we can't verify, so keep.
///
/// `home` is the user's home dir (`$HOME`); `provider_session_id` is the
/// provider-side id used for resume.
pub fn check_resumability(
    home: &Path,
    provider: &str,
    cwd: &str,
    provider_session_id: &str,
) -> Resumability {
    match provider {
        "claude" => claude_transcript_exists(home, cwd, provider_session_id),
        _ => Resumability::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_dir_encoding_matches_claude() {
        // Verified against a real ~/.claude/projects entry.
        assert_eq!(
            claude_project_dir_name("/Users/dev/project"),
            "-Users-dev-project"
        );
        // Dots become dashes; consecutive separators stay (each maps 1:1).
        assert_eq!(
            claude_project_dir_name("/Users/x/.config/my.app"),
            "-Users-x--config-my-app"
        );
    }

    #[test]
    fn transcript_path_is_under_projects() {
        let home = Path::new("/home/u");
        let p = claude_transcript_path(home, "/Users/dev/project", "abc-123");
        assert_eq!(
            p,
            Path::new("/home/u/.claude/projects/-Users-dev-project/abc-123.jsonl")
        );
    }

    #[test]
    fn exists_via_exact_path() {
        let home = tempfile::tempdir().unwrap();
        let cwd = "/Users/dev/project";
        let sid = "11111111-2222-3333-4444-555555555555";
        let dir = home
            .path()
            .join(".claude")
            .join("projects")
            .join(claude_project_dir_name(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{sid}.jsonl")), b"{}").unwrap();

        assert_eq!(
            claude_transcript_exists(home.path(), cwd, sid),
            Resumability::Exists
        );
        assert_eq!(
            check_resumability(home.path(), "claude", cwd, sid),
            Resumability::Exists
        );
    }

    #[test]
    fn exists_via_fallback_scan_when_cwd_encoding_differs() {
        let home = tempfile::tempdir().unwrap();
        let sid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        // File lives in a project dir that does NOT match the cwd we pass in.
        let dir = home
            .path()
            .join(".claude")
            .join("projects")
            .join("-some-other-encoded-dir");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{sid}.jsonl")), b"{}").unwrap();

        assert_eq!(
            claude_transcript_exists(home.path(), "/a/mismatched/cwd", sid),
            Resumability::Exists
        );
    }

    #[test]
    fn gone_when_no_transcript() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".claude").join("projects")).unwrap();
        assert_eq!(
            claude_transcript_exists(home.path(), "/Users/x/proj", "no-such-id"),
            Resumability::Gone
        );
        assert_eq!(
            check_resumability(home.path(), "claude", "/Users/x/proj", "no-such-id"),
            Resumability::Gone
        );
    }

    #[test]
    fn gone_when_projects_dir_missing() {
        let home = tempfile::tempdir().unwrap();
        // No ~/.claude/projects at all.
        assert_eq!(
            claude_transcript_exists(home.path(), "/Users/x/proj", "id"),
            Resumability::Gone
        );
    }

    #[test]
    fn unknown_provider_is_unverifiable() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            check_resumability(home.path(), "codex", "/Users/x/proj", "id"),
            Resumability::Unknown
        );
        assert_eq!(
            check_resumability(home.path(), "shell", "/Users/x/proj", "id"),
            Resumability::Unknown
        );
    }
}
