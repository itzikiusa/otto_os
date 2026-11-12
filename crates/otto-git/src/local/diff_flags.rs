//! Diff-side helpers split out of `local.rs` (LOC ratchet): the rename-limit
//! warning probe and the "are these commits here?" check the PR diff's
//! local whitespace-ignoring fallback needs.

use super::{GitCmd, LocalGit};

/// git's `-l<N>` overflow warning ("exhaustive/inexact rename detection was
/// skipped due to too many files"): past [`super::RENAMES`]' limit some
/// renames come back as a delete + an add. Recorded so the diff can say so
/// (`DiffResp.renames_incomplete`).
pub(super) fn note_renames(stderr: &str, ri: &std::sync::atomic::AtomicBool) {
    if stderr.contains("rename detection was skipped") {
        ri.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A full or abbreviated hex object name — nothing git could read as an
/// option or a rev expression.
fn is_sha(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl LocalGit {
    /// True when every one of `shas` names a commit in this clone (no fetch
    /// is attempted). Anything that is not a hex object name is "absent".
    pub async fn has_commits(&self, shas: &[&str]) -> bool {
        for sha in shas {
            if !is_sha(sha) {
                return false;
            }
            let spec = format!("{sha}^{{commit}}");
            let found = matches!(
                self.exec(&GitCmd::read(&["rev-parse", "-q", "--verify", &spec]), None)
                    .await,
                Ok((true, ..))
            );
            if !found {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::is_sha;

    #[test]
    fn only_hex_object_names_pass() {
        assert!(is_sha("deadbeef"));
        assert!(is_sha(&"a".repeat(40)));
        assert!(!is_sha("abc"));
        assert!(!is_sha("--output=x"));
        assert!(!is_sha("HEAD"));
        assert!(!is_sha("main"));
    }
}
