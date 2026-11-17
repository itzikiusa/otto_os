//! The worktree files a checkout, pull or stash rewrote. The IDE's language servers
//! don't watch the disk, so a closed file changed under them (gopls, pyright) stays
//! stale until the host forwards these paths ([`crate::GitCtx::worktree_files_changed`]).

use std::path::PathBuf;

use crate::local::{nul_records, GitCmd, LocalGit};

/// Taken before the operation: HEAD and the tracked files dirty against it.
pub(crate) struct WorktreeBefore {
    head: Option<String>,
    dirty: Vec<String>,
}

/// Tracked files whose worktree content differs from `rev` (empty on any failure).
async fn differing_from(git: &LocalGit, rev: &str) -> Vec<String> {
    let cmd = GitCmd::read(&[
        "diff",
        "--no-ext-diff",
        "--name-only",
        "-z",
        "--end-of-options",
        rev,
        "--",
    ]);
    match git.exec_text(&cmd).await {
        Ok(out) => nul_records(&out).map(str::to_string).collect(),
        Err(_) => Vec::new(),
    }
}

pub(crate) async fn before(git: &LocalGit) -> WorktreeBefore {
    let head = git.rev_parse("HEAD").await.ok();
    let dirty = differing_from(git, "HEAD").await;
    WorktreeBefore { head, dirty }
}

/// What was dirty before (a stash save took it) plus what now differs from the old HEAD
/// (a checkout, a pull, a stash pop), as absolute paths.
pub(crate) async fn changed_since(git: &LocalGit, before: WorktreeBefore) -> Vec<PathBuf> {
    let mut names = before.dirty;
    if let Some(head) = before.head {
        names.extend(differing_from(git, &head).await);
    }
    names.sort_unstable();
    names.dedup();
    names.into_iter().map(|n| git.path().join(n)).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{before, changed_since};
    use crate::local::LocalGit;

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    #[tokio::test]
    async fn checkout_and_stash_report_the_files_they_rewrote() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "otto@test.local"]);
        git(&dir, &["config", "user.name", "Otto Test"]);
        git(&dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("map.go"), "package m\nfunc NewMap() {}\n").unwrap();
        std::fs::write(dir.join("same.go"), "package m\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "base"]);
        git(&dir, &["checkout", "-q", "-b", "renamed"]);
        std::fs::write(dir.join("map.go"), "package m\nfunc NewMap2() {}\n").unwrap();
        git(&dir, &["commit", "-q", "-am", "rename"]);
        let lg = LocalGit::new(&dir);

        let b = before(&lg).await;
        lg.checkout("main", false).await.unwrap();
        assert_eq!(changed_since(&lg, b).await, vec![dir.join("map.go")]);

        // A stash save takes the dirty file away; the pop brings it back.
        std::fs::write(dir.join("same.go"), "package m\n// wip\n").unwrap();
        let b = before(&lg).await;
        lg.stash_save().await.unwrap();
        assert_eq!(changed_since(&lg, b).await, vec![dir.join("same.go")]);
        let b = before(&lg).await;
        lg.stash_pop().await.unwrap();
        assert_eq!(changed_since(&lg, b).await, vec![dir.join("same.go")]);
    }
}
