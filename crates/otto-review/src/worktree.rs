//! Isolated PR-head checkouts for review runs: a throwaway linked worktree of
//! the PR's source branch (or the host's PR head ref for fork PRs), torn down
//! when the run ends.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use otto_core::Id;

const WT_DIR_PREFIX: &str = "otto-review-wt-";
const WT_BRANCH_PREFIX: &str = "otto-review-";
const PR_REVIEW_REF_PREFIX: &str = "refs/otto/pr-review/";

/// Worktree directory names currently materialized by THIS daemon process,
/// mapped to their canonical repository path. Keeps concurrent reviews (and
/// parallel tests) from ever sweeping each other's live worktree or refs.
fn active_worktrees() -> &'static Mutex<HashMap<String, PathBuf>> {
    static ACTIVE: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn canonical_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn register_live_worktree(dir_name: &str, repo_path: &Path) {
    active_worktrees()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(dir_name.to_string(), canonical_or_self(repo_path));
}

fn unregister_live_worktree(dir_name: &str) {
    active_worktrees()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(dir_name);
}

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
    git_token: Option<otto_git::transport::TransportCredential>,
) -> Option<PrWorktree> {
    let git = otto_git::LocalGit::new(&repo.path);
    if let Err(e) = git.fetch(git_token.clone()).await {
        tracing::warn!(review = %review_id, "fetch before review failed: {e}; using cached refs");
    }
    let head_sha = head_sha.map(str::trim).filter(|s| !s.is_empty());
    let wt_dir_name = format!("{WT_DIR_PREFIX}{review_id}{suffix}");
    let wt_path = std::env::temp_dir()
        .join(&wt_dir_name)
        .to_string_lossy()
        .into_owned();
    let wt_branch = format!("{WT_BRANCH_PREFIX}{review_id}{suffix}");
    register_live_worktree(&wt_dir_name, Path::new(&repo.path));
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
    unregister_live_worktree(&wt_dir_name);
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
    if let Some(dir_name) = Path::new(&wt.path).file_name().and_then(|n| n.to_str()) {
        unregister_live_worktree(dir_name);
    }
}

/// Summary of orphaned PR review artifacts cleaned up by
/// [`sweep_orphaned_worktrees`] / [`sweep_orphaned_worktrees_in`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OrphanSweepReport {
    pub worktrees_removed: usize,
    pub branches_deleted: usize,
    pub refs_deleted: usize,
}

/// Does `rest` (the suffix after `otto-review-wt-` or `otto-review-`) belong
/// to one of `active_review_ids`? Matches exact `<id>` or `<id>-<suffix>`
/// (such as `-sum`, `-r1`, `-t`) without accidentally matching an unrelated id
/// that merely shares a prefix.
fn matches_active_review(rest: &str, active_review_ids: &HashSet<String>) -> bool {
    active_review_ids.iter().any(|id| {
        !id.is_empty()
            && (rest == id.as_str()
                || rest
                    .strip_prefix(id.as_str())
                    .is_some_and(|suffix| suffix.starts_with('-')))
    })
}

/// Best-effort discovery of a linked worktree's parent repository path from
/// its `.git` pointer file (`gitdir: <repo>/.git/worktrees/<name>`). Returns
/// `None` if the dir is corrupt, not a linked worktree, or the parent repo no
/// longer exists.
pub(crate) async fn repo_path_from_worktree(wt_dir: &Path) -> Option<PathBuf> {
    let dot_git = wt_dir.join(".git");
    let meta = tokio::fs::symlink_metadata(&dot_git).await.ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > 8192 {
        return None;
    }
    let text = tokio::fs::read_to_string(&dot_git).await.ok()?;
    let raw_gitdir = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))?
        .trim();
    if raw_gitdir.is_empty() {
        return None;
    }
    let gitdir = {
        let p = PathBuf::from(raw_gitdir);
        if p.is_absolute() {
            p
        } else {
            wt_dir.join(p)
        }
    };
    let common = {
        let commondir_file = gitdir.join("commondir");
        if let Ok(rel) = tokio::fs::read_to_string(&commondir_file).await {
            let rel = rel.trim();
            if !rel.is_empty() {
                let joined = if Path::new(rel).is_absolute() {
                    PathBuf::from(rel)
                } else {
                    gitdir.join(rel)
                };
                tokio::fs::canonicalize(&joined).await.ok()
            } else {
                None
            }
        } else if gitdir.parent().and_then(|p| p.file_name())
            == Some(std::ffi::OsStr::new("worktrees"))
        {
            gitdir
                .parent()
                .and_then(|p| p.parent())
                .map(Path::to_path_buf)
        } else {
            None
        }
    }?;

    let repo = if common.file_name() == Some(std::ffi::OsStr::new(".git")) {
        common.parent()?.to_path_buf()
    } else {
        common
    };
    tokio::fs::metadata(&repo)
        .await
        .ok()
        .filter(|m| m.is_dir())
        .map(|_| repo)
}

/// Sweep orphaned `$TMPDIR/otto-review-wt-*` directories (not belonging to any
/// active review) and prune stale `otto-review-*` branches and
/// `refs/otto/pr-review/*` refs in discovered repositories (plus any
/// `extra_repo_paths`). Safe to call on boot or while live reviews run.
pub async fn sweep_orphaned_worktrees(
    active_review_ids: &HashSet<String>,
    extra_repo_paths: &[PathBuf],
) -> OrphanSweepReport {
    sweep_orphaned_worktrees_in(&std::env::temp_dir(), active_review_ids, extra_repo_paths).await
}

/// Same as [`sweep_orphaned_worktrees`], scoped to `base_dir` (for isolated
/// unit tests).
pub async fn sweep_orphaned_worktrees_in(
    base_dir: &Path,
    active_review_ids: &HashSet<String>,
    extra_repo_paths: &[PathBuf],
) -> OrphanSweepReport {
    let mut report = OrphanSweepReport::default();
    let live_snapshot: HashMap<String, PathBuf> = active_worktrees()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    let mut repos_to_prune: BTreeSet<PathBuf> = BTreeSet::new();
    for repo in extra_repo_paths {
        if let Ok(meta) = tokio::fs::metadata(repo).await {
            if meta.is_dir() {
                repos_to_prune.insert(
                    tokio::fs::canonicalize(repo)
                        .await
                        .unwrap_or_else(|_| repo.clone()),
                );
            }
        }
    }

    if let Ok(mut entries) = tokio::fs::read_dir(base_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(rest) = name.strip_prefix(WT_DIR_PREFIX) else {
                continue;
            };
            if live_snapshot.contains_key(&name) || matches_active_review(rest, active_review_ids) {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = tokio::fs::symlink_metadata(&path).await else {
                continue;
            };
            // Never follow symlinks or touch non-directory entries.
            if meta.file_type().is_symlink() || !meta.is_dir() {
                continue;
            }

            if let Some(repo_path) = repo_path_from_worktree(&path).await {
                let canon_repo = tokio::fs::canonicalize(&repo_path)
                    .await
                    .unwrap_or(repo_path);
                repos_to_prune.insert(canon_repo.clone());
                let git = otto_git::LocalGit::new(&canon_repo);
                let _ = git.worktree_remove(&path.to_string_lossy()).await;
            }
            if tokio::fs::symlink_metadata(&path).await.is_ok() {
                if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(
                            path = %path.display(),
                            "review orphan sweep: remove_dir_all failed: {e}"
                        );
                        continue;
                    }
                }
            }
            if tokio::fs::symlink_metadata(&path).await.is_err() {
                report.worktrees_removed += 1;
            }
        }
    }

    for repo_path in repos_to_prune {
        prune_repo_review_refs(&repo_path, active_review_ids, &live_snapshot, &mut report).await;
    }
    report
}

async fn list_refs_with_prefix(repo_path: &Path, format: &str, prefix: &str) -> Vec<String> {
    let mut cmd = otto_git::hardened_command();
    cmd.arg("-C")
        .arg(repo_path)
        .arg("for-each-ref")
        .arg(format!("--format={format}"))
        .arg(prefix)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    match cmd.output().await {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

async fn prune_repo_review_refs(
    repo_path: &Path,
    active_review_ids: &HashSet<String>,
    live_snapshot: &HashMap<String, PathBuf>,
    report: &mut OrphanSweepReport,
) {
    let git = otto_git::LocalGit::new(repo_path);
    let _ = git.worktree_prune().await;

    for branch in
        list_refs_with_prefix(repo_path, "%(refname:short)", "refs/heads/otto-review-*").await
    {
        let Some(rest) = branch.strip_prefix(WT_BRANCH_PREFIX) else {
            continue;
        };
        let expected_dir = format!("{WT_DIR_PREFIX}{rest}");
        if live_snapshot.contains_key(&expected_dir)
            || matches_active_review(rest, active_review_ids)
        {
            continue;
        }
        if git.delete_branch(&branch, true).await.is_ok() {
            report.branches_deleted += 1;
        }
    }

    // `refs/otto/pr-review/<n>-<sha12>` does not carry a review id, so only
    // prune them when no review is active and no in-process worktree targets
    // this repository.
    let repo_has_live = live_snapshot.values().any(|p| p == repo_path);
    if active_review_ids.is_empty() && !repo_has_live {
        for r in list_refs_with_prefix(repo_path, "%(refname)", PR_REVIEW_REF_PREFIX).await {
            if r.starts_with(PR_REVIEW_REF_PREFIX) && git.delete_pr_review_ref(&r).await.is_ok() {
                report.refs_deleted += 1;
            }
        }
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

    #[allow(clippy::disallowed_methods)] // test helper: a blocking git call is fine here
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

    #[tokio::test]
    async fn sweep_removes_orphaned_worktrees_branches_and_refs_but_preserves_active() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_dir = tmp.path().join("repo");
        let sweep_base = tmp.path().join("tmpbase");
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::create_dir_all(&sweep_base).unwrap();

        sh(&repo_dir, &["init", "-q", "-b", "main"]);
        std::fs::write(repo_dir.join("f.txt"), "base\n").unwrap();
        sh(&repo_dir, &["add", "."]);
        sh(&repo_dir, &["commit", "-qm", "base"]);
        let head = sh(&repo_dir, &["rev-parse", "HEAD"]);

        // 1. Orphaned linked worktree + branch
        let orphan_wt = sweep_base.join("otto-review-wt-orphan1");
        sh(
            &repo_dir,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "otto-review-orphan1",
                orphan_wt.to_str().unwrap(),
                "HEAD",
            ],
        );
        // 2. Stale orphan branch whose worktree dir is already gone
        sh(&repo_dir, &["branch", "otto-review-orphan2", "HEAD"]);
        // 3. Active review's linked worktree + branch (with retry suffix)
        let active_wt = sweep_base.join("otto-review-wt-active1-r1");
        sh(
            &repo_dir,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "otto-review-active1-r1",
                active_wt.to_str().unwrap(),
                "HEAD",
            ],
        );
        // 4. Corrupt/detached orphan dir (invalid .git pointer)
        let corrupt_wt = sweep_base.join("otto-review-wt-corrupt");
        std::fs::create_dir_all(&corrupt_wt).unwrap();
        std::fs::write(corrupt_wt.join(".git"), "not a valid gitdir").unwrap();
        // 5. Unrelated user directory that must never be touched
        let unrelated = sweep_base.join("keep-me");
        std::fs::create_dir_all(&unrelated).unwrap();
        // 6. Stale PR review ref
        sh(
            &repo_dir,
            &["update-ref", "refs/otto/pr-review/42-deadbeef1234", &head],
        );

        // First sweep with `active1` still running: orphans are removed, active
        // worktree/branch and PR review ref are preserved.
        let active: HashSet<String> = ["active1".to_string()].into_iter().collect();
        let report = sweep_orphaned_worktrees_in(&sweep_base, &active, &[]).await;
        assert_eq!(report.worktrees_removed, 2, "{report:?}");
        assert_eq!(report.branches_deleted, 2, "{report:?}");
        assert_eq!(report.refs_deleted, 0, "{report:?}");
        assert!(!orphan_wt.exists());
        assert!(!corrupt_wt.exists());
        assert!(active_wt.is_dir(), "active worktree must be preserved");
        assert!(unrelated.is_dir(), "unrelated dir must be preserved");
        let branches = sh(
            &repo_dir,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
        );
        assert!(branches.contains("main"), "{branches}");
        assert!(
            branches.contains("otto-review-active1-r1"),
            "active branch must be preserved: {branches}"
        );
        assert!(!branches.contains("otto-review-orphan1"), "{branches}");
        assert!(!branches.contains("otto-review-orphan2"), "{branches}");
        let refs = sh(&repo_dir, &["for-each-ref", "refs/otto/pr-review/"]);
        assert!(
            refs.contains("refs/otto/pr-review/42-deadbeef1234"),
            "review ref must stay while a review is active: {refs}"
        );

        // Boot-like sweep with no active reviews: remaining worktree, branch,
        // and PR review ref are all cleaned up.
        let empty: HashSet<String> = HashSet::new();
        let report2 = sweep_orphaned_worktrees_in(&sweep_base, &empty, &[]).await;
        assert_eq!(report2.worktrees_removed, 1, "{report2:?}");
        assert_eq!(report2.branches_deleted, 1, "{report2:?}");
        assert_eq!(report2.refs_deleted, 1, "{report2:?}");
        assert!(!active_wt.exists());
        assert!(unrelated.is_dir());
        let branches2 = sh(
            &repo_dir,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
        );
        assert_eq!(branches2, "main");
        let refs2 = sh(&repo_dir, &["for-each-ref", "refs/otto/pr-review/"]);
        assert!(refs2.is_empty(), "stale ref left behind: {refs2}");
    }

    #[tokio::test]
    async fn sweep_handles_missing_dir_and_skips_symlinks_safely() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let empty = HashSet::new();
        assert_eq!(
            sweep_orphaned_worktrees_in(&missing, &empty, &[]).await,
            OrphanSweepReport::default()
        );

        #[cfg(unix)]
        {
            let sweep_base = tmp.path().join("base");
            let outside = tmp.path().join("outside");
            std::fs::create_dir_all(&sweep_base).unwrap();
            std::fs::create_dir_all(&outside).unwrap();
            let marker = outside.join("precious.txt");
            std::fs::write(&marker, "keep").unwrap();
            std::os::unix::fs::symlink(&outside, sweep_base.join("otto-review-wt-sym")).unwrap();

            let report = sweep_orphaned_worktrees_in(&sweep_base, &empty, &[]).await;
            assert_eq!(report, OrphanSweepReport::default());
            assert_eq!(std::fs::read_to_string(&marker).unwrap(), "keep");
        }
    }
}
