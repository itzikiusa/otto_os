//! Repository hooks Otto's own git may skip (see `local::HOOK_VERBS`): a
//! `core.hooksPath` inside the work tree is editable by any agent working
//! there, so daemon-initiated commits / pushes run without it.
use std::path::PathBuf;

use crate::local::LocalGit;

/// Does a `core.hooksPath` value point inside the work tree `toplevel`?
/// Relative values resolve against the work tree root (git's rule), so they
/// are inside unless they climb out with `..`; `~/` expands to `$HOME`.
pub(crate) fn hooks_path_inside(hooks: &str, toplevel: Option<&str>) -> bool {
    let Some(top) = toplevel else {
        return false;
    };
    let top = std::fs::canonicalize(top).unwrap_or_else(|_| PathBuf::from(top));
    let raw = if let Some(rest) = hooks.strip_prefix("~/") {
        match std::env::var_os("HOME") {
            Some(h) => PathBuf::from(h).join(rest),
            None => return false,
        }
    } else {
        PathBuf::from(hooks)
    };
    let abs = if raw.is_absolute() {
        raw
    } else {
        top.join(raw)
    };
    // Resolve `..` / symlinks where the dir exists; else normalise lexically.
    let resolved = std::fs::canonicalize(&abs).unwrap_or_else(|_| {
        let mut out = PathBuf::new();
        for c in abs.components() {
            match c {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                other => out.push(other),
            }
        }
        out
    });
    resolved.starts_with(&top)
}

impl LocalGit {
    /// Will this handle's commits / pushes skip the repo's hooks? True for a
    /// daemon-initiated handle in a repo whose `core.hooksPath` lies inside the
    /// work tree (husky / lefthook) — callers surface it on their run, since
    /// those hooks (commit-msg, pre-commit, pre-push) are then left to CI.
    pub async fn worktree_hooks_skipped(&self) -> bool {
        !self.person_initiated && self.hooks_path_in_worktree().await
    }
}
