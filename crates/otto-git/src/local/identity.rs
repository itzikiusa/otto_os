//! A checkout's branch identity — what review lineage is keyed to. Split
//! out of `local.rs` (LOC ratchet).

use super::LocalGit;

impl LocalGit {
    /// The branch a checkout's code belongs to, for keying work to it (review
    /// lineage). [`Self::current_branch`]'s `"HEAD"` for a detached checkout
    /// is not a name: every detached checkout would share it. A workflow
    /// checks a PR out as `--detach origin/<branch>`, so a detached HEAD is
    /// named by the ONE remote branch that points at it; none or several ⇒
    /// `None` (unknown, never a guess).
    pub async fn branch_identity(&self) -> Option<String> {
        if let Some(name) = self.symbolic_head().await {
            return Some(name);
        }
        let out = self
            .run_read(&[
                "for-each-ref",
                "--points-at=HEAD",
                "--format=%(refname:lstrip=3)",
                "refs/remotes",
            ])
            .await
            .ok()?;
        crate::parse::sole_remote_branch(&out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sh(dir: &Path, args: &[&str]) -> String {
        let mut cmd = std::process::Command::new("git");
        super::super::test_hermetic(&mut cmd);
        let out = cmd
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn repo(branch: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh(&dir, &["init", "-q", "-b", branch]);
        sh(&dir, &["config", "user.email", "otto@test.local"]);
        sh(&dir, &["config", "user.name", "Otto Test"]);
        sh(&dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        sh(&dir, &["add", "."]);
        sh(&dir, &["commit", "-qm", "init"]);
        (tmp, dir)
    }

    fn commit(dir: &Path, file: &str) {
        std::fs::write(dir.join(file), file).unwrap();
        sh(dir, &["add", file]);
        sh(dir, &["commit", "-qm", file]);
    }

    /// A workflow's `checkout --detach origin/<branch>` is named by that
    /// branch, not "HEAD" (which every detached review used to share, so
    /// one PR's review inherited every other PR's findings).
    #[tokio::test]
    async fn branch_identity_names_a_detached_pr_checkout_by_its_remote_branch() {
        let (_tmp, dir) = repo("main");
        let git = LocalGit::new(dir.to_str().unwrap());
        assert_eq!(git.branch_identity().await.as_deref(), Some("main"));

        commit(&dir, "pr.txt");
        let head = sh(&dir, &["rev-parse", "HEAD"]);
        sh(
            &dir,
            &["update-ref", "refs/remotes/origin/feature/GS-1-x", &head],
        );
        sh(&dir, &["update-ref", "refs/remotes/origin/main", "HEAD~1"]);
        sh(
            &dir,
            &["checkout", "-q", "--detach", "origin/feature/GS-1-x"],
        );
        assert_eq!(git.current_branch().await.unwrap(), "HEAD");
        assert_eq!(
            git.branch_identity().await.as_deref(),
            Some("feature/GS-1-x")
        );

        // Two different remote branches at HEAD: unknown, never a guess.
        sh(&dir, &["update-ref", "refs/remotes/origin/other", &head]);
        assert_eq!(git.branch_identity().await, None);
    }
}
