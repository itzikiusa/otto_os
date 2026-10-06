//! Isolated PR-head checkouts for review runs: a throwaway linked worktree of
//! the PR's source branch (or the host's PR head ref for fork PRs), torn down
//! when the run ends.

use otto_core::Id;

/// A throwaway checkout of a PR's head, created for a review run.
pub struct PrWorktree {
    pub path: String,
    pub branch: String,
    /// The `refs/otto/pr-review/*` ref fetched to build this tree (fork PRs),
    /// deleted on teardown — the worktree branch holds the commit while the
    /// review runs (S2-309).
    pub review_ref: Option<String>,
}

/// Materialize a PR's head into an isolated linked worktree (best-effort).
/// The checkout must be THE PR's head: when the host told us `head_sha`, every
/// candidate is verified against it before it is used —
/// 1. `origin/<source>` (same-repo PRs) — only if it resolves to `head_sha`.
///    A fork PR whose branch name also exists upstream (fork `main` → `main`)
///    would otherwise check out ORIGIN's branch, and a failed fetch / a branch
///    that moved since the diff was read would check out a stale tip;
/// 2. the host's PR head ref / the head sha via
///    [`otto_git::LocalGit::fetch_pr_head`] (fork PRs) — again verified;
/// 3. `head_sha` itself when the commit is local.
///
/// Without a `head_sha` (nothing to verify against) `origin/<source>` is used
/// as before. `None` ⇒ no matching checkout could be built and the caller
/// must not claim one. `suffix` keeps concurrent retries apart. `git_token` is
/// the repo's git credential (otto-server resolves it), used only so the
/// fetch can reach a private origin.
pub async fn materialize_pr_worktree(
    repo: &otto_core::domain::Repo,
    review_id: &Id,
    pr_number: u64,
    source: Option<&str>,
    head_sha: Option<&str>,
    suffix: &str,
    git_token: Option<String>,
) -> Option<PrWorktree> {
    let git = otto_git::LocalGit::new(&repo.path);
    if let Err(e) = git.fetch(git_token.clone()).await {
        tracing::warn!(review = %review_id, "fetch before review failed: {e}; using cached refs");
    }
    let head_sha = head_sha.map(str::trim).filter(|s| !s.is_empty());
    let wt_path = std::env::temp_dir()
        .join(format!("otto-review-wt-{review_id}{suffix}"))
        .to_string_lossy()
        .into_owned();
    let wt_branch = format!("otto-review-{review_id}{suffix}");
    let _ = git.worktree_remove(&wt_path).await; // clear any stale tree

    // `true` when `r` is the PR head (or there is no head to check against).
    let is_head = |r: String| {
        let git = &git;
        async move {
            match head_sha {
                None => true,
                Some(h) => git
                    .rev_parse(&r)
                    .await
                    .is_ok_and(|full| sha_matches(&full, h)),
            }
        }
    };
    let mut candidates: Vec<String> = Vec::new();
    if let Some(src) = source.filter(|s| !s.is_empty()) {
        let base = format!("origin/{src}");
        if is_head(base.clone()).await {
            candidates.push(base);
        } else {
            tracing::warn!(review = %review_id, "{base} is not the PR head; using the PR head ref");
        }
    }
    // A `refs/otto/pr-review/*` ref this call fetched: owned by the tree it
    // builds (deleted on teardown), or deleted right here when none is built.
    let mut review_ref: Option<String> = None;
    if candidates.is_empty() {
        match git.fetch_pr_head(pr_number, head_sha, git_token).await {
            Ok(h) if is_head(h.clone()).await => {
                if h.starts_with("refs/otto/pr-review/") {
                    review_ref = Some(h.clone());
                }
                candidates.push(h)
            }
            Ok(h) => {
                tracing::warn!(review = %review_id, "PR head ref {h} is not the reviewed head");
                if h.starts_with("refs/otto/pr-review/") {
                    let _ = git.delete_pr_review_ref(&h).await;
                }
            }
            Err(e) => tracing::warn!(review = %review_id, "PR head unavailable: {e}"),
        }
    }
    if candidates.is_empty() {
        // The ref may lag the host; the commit itself may still be local.
        if let Some(h) = head_sha {
            if is_head(h.to_string()).await {
                candidates.push(h.to_string());
            }
        }
    }
    for head in candidates {
        match git.worktree_add(&wt_path, &wt_branch, &head).await {
            Ok(()) => {
                tracing::info!(review = %review_id, "reviewing PR #{pr_number} head ({head}) in isolated worktree");
                return Some(PrWorktree {
                    path: wt_path,
                    branch: wt_branch,
                    review_ref,
                });
            }
            Err(e) => {
                tracing::warn!(review = %review_id, "worktree of PR head {head} failed: {e}")
            }
        }
    }
    if let Some(r) = review_ref {
        let _ = git.delete_pr_review_ref(&r).await;
    }
    tracing::warn!(review = %review_id, "no checkout of PR #{pr_number}'s head; reviewing in repo path (partial)");
    None
}

/// Does the resolved full sha `full` name the host's `head` (which Bitbucket
/// abbreviates to 12 chars)? A prefix match of at least 7 hex chars.
pub fn sha_matches(full: &str, head: &str) -> bool {
    let (full, head) = (full.trim(), head.trim());
    head.len() >= 7 && head.len() <= full.len() && full[..head.len()].eq_ignore_ascii_case(head)
}

/// Tear down a [`PrWorktree`] (best-effort; leaves no litter in the user's
/// branch list). The branch is review-only, so force-delete is safe.
pub async fn teardown_pr_worktree(repo_path: &str, wt: PrWorktree) {
    let git = otto_git::LocalGit::new(repo_path);
    let _ = git.worktree_remove(&wt.path).await;
    let _ = git.delete_branch(&wt.branch, true).await;
    if let Some(r) = wt.review_ref.as_deref() {
        let _ = git.delete_pr_review_ref(r).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha_matches_full_and_abbreviated_heads_only() {
        let full = "0123456789abcdef0123456789abcdef01234567";
        assert!(sha_matches(full, full));
        assert!(sha_matches(full, "0123456789ab")); // Bitbucket's 12-char form
        assert!(sha_matches(full, "0123456789AB"));
        assert!(!sha_matches(full, "0123")); // too short to mean anything
        assert!(!sha_matches(full, "ffff456789ab"));
        assert!(!sha_matches("0123456", full));
    }

    fn sh(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A fork PR whose source branch is `main`: `origin/main` exists but is
    /// NOT the PR head, so the checkout must be the head sha, never origin's.
    #[tokio::test]
    async fn fork_pr_from_main_checks_out_the_head_sha_not_origin_main() {
        let tmp = tempfile::tempdir().unwrap();
        let upstream = tmp.path().join("upstream");
        std::fs::create_dir(&upstream).unwrap();
        sh(&upstream, &["init", "-q", "-b", "main"]);
        std::fs::write(upstream.join("f.txt"), "base\n").unwrap();
        sh(&upstream, &["add", "."]);
        sh(&upstream, &["commit", "-qm", "base"]);
        let clone = tmp.path().join("clone");
        sh(
            tmp.path(),
            &[
                "clone",
                "-q",
                upstream.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        // The fork's head: a commit only present locally (Bitbucket-style —
        // no pull/merge-request ref on the host).
        sh(&clone, &["checkout", "-q", "-b", "fork-main"]);
        std::fs::write(clone.join("f.txt"), "fork change\n").unwrap();
        sh(&clone, &["commit", "-qam", "fork"]);
        let fork_head = sh(&clone, &["rev-parse", "HEAD"]);
        sh(&clone, &["checkout", "-q", "main"]);

        let repo = otto_core::domain::Repo {
            id: "r".into(),
            workspace_id: "w".into(),
            name: "n".into(),
            path: clone.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
            created_at: chrono::Utc::now(),
            forge: None,
        };
        let id = otto_core::new_id();
        let wt = materialize_pr_worktree(
            &repo,
            &id,
            7,
            Some("main"),
            Some(&fork_head[..12]),
            "-t",
            None,
        )
        .await
        .expect("a checkout of the PR head");
        let at = sh(std::path::Path::new(&wt.path), &["rev-parse", "HEAD"]);
        assert_eq!(
            at, fork_head,
            "must review the fork's head, not origin/main"
        );
        teardown_pr_worktree(&repo.path, wt).await;
    }

    /// S2-309: the per-head `refs/otto/pr-review/<n>-<sha12>` ref a fork PR's
    /// checkout is fetched into is deleted on teardown (it pinned the fork's
    /// objects against gc forever).
    #[tokio::test]
    async fn teardown_deletes_the_fetched_pr_review_ref() {
        let tmp = tempfile::tempdir().unwrap();
        let upstream = tmp.path().join("upstream");
        std::fs::create_dir(&upstream).unwrap();
        sh(&upstream, &["init", "-q", "-b", "main"]);
        std::fs::write(upstream.join("f.txt"), "base\n").unwrap();
        sh(&upstream, &["add", "."]);
        sh(&upstream, &["commit", "-qm", "base"]);
        // The PR head lives only under the host's `refs/pull/7/head`.
        sh(&upstream, &["checkout", "-q", "-b", "pr"]);
        std::fs::write(upstream.join("f.txt"), "pr change\n").unwrap();
        sh(&upstream, &["commit", "-qam", "pr"]);
        let head = sh(&upstream, &["rev-parse", "HEAD"]);
        sh(&upstream, &["update-ref", "refs/pull/7/head", &head]);
        sh(&upstream, &["checkout", "-q", "main"]);
        sh(&upstream, &["branch", "-qD", "pr"]);
        let clone = tmp.path().join("clone");
        sh(
            tmp.path(),
            &[
                "clone",
                "-q",
                upstream.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        let repo = otto_core::domain::Repo {
            id: "r".into(),
            workspace_id: "w".into(),
            name: "n".into(),
            path: clone.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
            created_at: chrono::Utc::now(),
            forge: None,
        };
        let id = otto_core::new_id();
        let wt = materialize_pr_worktree(&repo, &id, 7, None, Some(&head), "-r", None)
            .await
            .expect("a checkout of the PR head");
        let review_ref = wt.review_ref.clone().expect("fetched into a review ref");
        assert!(review_ref.starts_with("refs/otto/pr-review/7-"));
        let refs = sh(&clone, &["for-each-ref", "refs/otto/"]);
        assert!(refs.contains(&review_ref), "{refs}");
        teardown_pr_worktree(&repo.path, wt).await;
        let refs = sh(&clone, &["for-each-ref", "refs/otto/"]);
        assert!(refs.is_empty(), "ref left behind: {refs}");
    }
}
