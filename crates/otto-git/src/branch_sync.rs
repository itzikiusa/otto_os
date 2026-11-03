//! Make the branches a run needs resolvable BEFORE it starts: fetch `origin`,
//! then bring each named local branch up to its remote-tracking tip where that
//! is provably safe. A PR (Slack-triggered workflow, review, draft) names its
//! head and base by branch NAME; a clone that never fetched a branch created
//! on the host — or still holds a stale copy of it — otherwise resolves
//! nothing (the run fails) or the wrong commit (the diff is against an old
//! base).
//!
//! Safety (load-bearing): this never forces a ref, never touches a working
//! tree and never discards a local commit.
//! - no local branch → one is created at `origin/<b>`, tracking it;
//! - local branch strictly BEHIND `origin/<b>` and checked out nowhere →
//!   fast-forwarded with a compare-and-swap `update-ref`;
//! - behind but checked out in some worktree → left alone (moving the ref
//!   would desync that tree's index); callers use `origin/<b>`;
//! - local ahead → left alone; the local branch already contains the remote;
//! - diverged → left alone and logged; callers use `origin/<b>`.

use crate::local::{upstream_err, AskPass, LocalGit, SpawnClass};
use crate::transport::TransportCredential;

/// What [`LocalGit::sync_branch_with_origin`] found / did for one branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchSyncOutcome {
    /// Local branch already at `origin/<b>`.
    UpToDate,
    /// No local branch existed; created at `origin/<b>`, tracking it.
    Created,
    /// Local branch was behind and checked out nowhere; fast-forwarded.
    FastForwarded,
    /// Local branch is behind but checked out in a worktree — not moved.
    CheckedOutBehind,
    /// Local branch has commits `origin/<b>` lacks (and contains it).
    LocalAhead,
    /// Local and remote both have commits the other lacks — not moved.
    Diverged,
    /// Only a local branch exists (never pushed, or the fetch failed).
    LocalOnly,
    /// Only `origin/<b>` exists and a local branch could not be created.
    RemoteOnly,
    /// Neither a local branch nor `origin/<b>` exists.
    Missing,
    /// Not a branch name that is safe to pass to git.
    Invalid,
}

/// One branch's sync result. `resolved` is the FULL ref name callers should
/// read the branch's commit from (`refs/heads/<b>` or
/// `refs/remotes/origin/<b>`), `None` when nothing resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSync {
    pub branch: String,
    pub outcome: BranchSyncOutcome,
    pub resolved: Option<String>,
}

/// [`LocalGit::fetch_and_sync_branches`]'s result: the fetch error (the sync
/// still ran on the cached remote-tracking refs) plus one entry per branch.
#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub fetch_error: Option<String>,
    pub branches: Vec<BranchSync>,
}

impl SyncReport {
    /// The ref to use for `branch`, if it resolved.
    pub fn resolved(&self, branch: &str) -> Option<&str> {
        self.branches
            .iter()
            .find(|b| b.branch == branch)
            .and_then(|b| b.resolved.as_deref())
    }
}

/// A branch name that is safe to splice into a ref / refspec: no option-like
/// prefix, no refspec/glob metacharacters, no whitespace or control chars, no
/// `..`. `git` itself still validates the full ref name.
pub fn is_syncable_branch_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.contains("..")
        && !name.contains("@{")
        && name != "@"
        && !name.starts_with("refs/")
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || ":^~?*[\\+".contains(c))
}

/// 7–40 hex chars: an (abbreviated) commit id, never fetched as a branch.
pub(crate) fn looks_like_sha(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

impl LocalGit {
    /// Fetch `origin` (all branches, pruned) and then
    /// [`Self::sync_branch_with_origin`] every name in `branches` (blank and
    /// duplicate names are skipped). A failed fetch is reported, not fatal:
    /// the sync then works from the refs already cached locally.
    pub async fn fetch_and_sync_branches(
        &self,
        branches: &[&str],
        token: Option<TransportCredential>,
    ) -> SyncReport {
        let fetch_error = match self.fetch(token).await {
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(
                    repo = %self.path().display(),
                    "fetch before branch sync failed: {e}; using cached remote refs"
                );
                Some(e.to_string())
            }
        };
        let mut seen: Vec<&str> = Vec::new();
        let mut out = Vec::new();
        for b in branches.iter().map(|b| b.trim()) {
            let b = b.strip_prefix("origin/").unwrap_or(b);
            if b.is_empty() || seen.contains(&b) {
                continue;
            }
            seen.push(b);
            out.push(self.sync_branch_with_origin(b).await);
        }
        SyncReport {
            fetch_error,
            branches: out,
        }
    }

    /// Fetch ONE branch from `origin` into its remote-tracking ref
    /// (`refs/remotes/origin/<branch>`) — the cheap repair when a single named
    /// branch is missing (a base created on the host after the last fetch).
    /// Writes only that remote-tracking ref: never a local branch, HEAD or
    /// working tree. SSH runs in BatchMode so it can never prompt.
    pub async fn fetch_origin_branch(
        &self,
        branch: &str,
        token: Option<TransportCredential>,
    ) -> otto_core::Result<()> {
        if !is_syncable_branch_name(branch) {
            return Err(otto_core::Error::Invalid(format!(
                "not a fetchable branch name: {branch}"
            )));
        }
        Self::guard_ref(branch)?;
        let spec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
        let askpass = match token {
            Some(t) => Some(AskPass::new(&t)?),
            None => None,
        };
        let mut envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
        envs.push((
            "GIT_SSH_COMMAND".to_owned(),
            "ssh -o BatchMode=yes".to_owned(),
        ));
        let (ok, stdout, stderr, code) = self
            .run_raw_class(
                &["fetch", "--no-tags", "origin", &spec],
                &envs,
                SpawnClass::Remote,
            )
            .await?;
        if ok {
            Ok(())
        } else {
            Err(upstream_err(&stderr, &stdout, code))
        }
    }

    /// True when the remote-tracking branch `origin/<branch>` exists. A
    /// probe, not a failure: `--quiet` + the raw runner keep a missing ref (the
    /// workflow reaper asking about an already-deleted `otto-wf/*` branch) out
    /// of the "git failed … code 128" warn log.
    pub async fn origin_branch_exists(&self, branch: &str) -> bool {
        if Self::guard_ref(branch).is_err() {
            return false;
        }
        let refname = format!("refs/remotes/origin/{branch}");
        matches!(
            self.run_raw(&["rev-parse", "--verify", "--quiet", &refname], &[])
                .await,
            Ok((true, _, _, _))
        )
    }

    /// [`Self::resolve_base`]'s repair: the wanted base `w` (logical name
    /// `logical`) exists neither locally nor as `origin/<logical>` — typically
    /// a PR base created on the host after this clone last fetched. Fetch THAT
    /// branch before the caller falls back to the default branch: falling
    /// back silently diffed (and reviewed / drafted a PR) against the wrong
    /// base. Only for a branch-like name — a sha or `HEAD~2` is never fetched.
    pub(crate) async fn fetch_base_if_missing(&self, w: &str, logical: &str) {
        if looks_like_sha(logical)
            || !is_syncable_branch_name(logical)
            || self.verify_commit_ref(w).await
            || self.verify_commit_ref(&format!("origin/{logical}")).await
        {
            return;
        }
        match self.fetch_origin_branch(logical, None).await {
            Ok(()) => tracing::info!(
                repo = %self.path().display(),
                "resolve_base: fetched missing base '{logical}' from origin"
            ),
            Err(e) => tracing::debug!(
                repo = %self.path().display(),
                "resolve_base: fetching base '{logical}' failed: {e}"
            ),
        }
    }

    /// Bring local `branch` in line with `refs/remotes/origin/<branch>` without
    /// any network access (see the module docs for the safety rules). Never
    /// errors: every failure degrades to an outcome with the best ref to use.
    pub async fn sync_branch_with_origin(&self, branch: &str) -> BranchSync {
        let done = |outcome, resolved: Option<String>| BranchSync {
            branch: branch.to_string(),
            outcome,
            resolved,
        };
        if !is_syncable_branch_name(branch) || Self::guard_ref(branch).is_err() {
            return done(BranchSyncOutcome::Invalid, None);
        }
        let local_ref = format!("refs/heads/{branch}");
        let remote_ref = format!("refs/remotes/origin/{branch}");
        let local = self.commit_of(&local_ref).await;
        let remote = self.commit_of(&remote_ref).await;
        let (local, remote) = match (local, remote) {
            (None, None) => return done(BranchSyncOutcome::Missing, None),
            (Some(_), None) => return done(BranchSyncOutcome::LocalOnly, Some(local_ref)),
            (None, Some(_)) => {
                // `--track` + the full remote ref: tracking is set up even
                // with `branch.autoSetupMerge=false`, and no local name can
                // shadow the start point.
                let created = self
                    .run_raw(&["branch", "--track", "--", branch, &remote_ref], &[])
                    .await;
                return match created {
                    Ok((true, ..)) => {
                        tracing::info!(
                            repo = %self.path().display(),
                            "branch sync: created local '{branch}' tracking origin/{branch}"
                        );
                        done(BranchSyncOutcome::Created, Some(local_ref))
                    }
                    // Refused (or raced with another creator): the remote
                    // ref still names the right commit.
                    _ => done(BranchSyncOutcome::RemoteOnly, Some(remote_ref)),
                };
            }
            (Some(l), Some(r)) => (l, r),
        };
        if local == remote {
            return done(BranchSyncOutcome::UpToDate, Some(local_ref));
        }
        if self.is_ancestor(&local, &remote).await {
            // Behind. A checked-out branch is never moved under its tree.
            if self.worktree_for_branch(branch).await.is_some() {
                tracing::info!(
                    repo = %self.path().display(),
                    "branch sync: '{branch}' is behind origin/{branch} but checked out — using origin/{branch}"
                );
                return done(BranchSyncOutcome::CheckedOutBehind, Some(remote_ref));
            }
            // Compare-and-swap: only moves the ref if it is still `local`.
            let msg = format!("otto: fast-forward {branch} to origin/{branch}");
            let moved = self
                .run_raw(
                    &["update-ref", "-m", &msg, &local_ref, &remote, &local],
                    &[],
                )
                .await;
            return match moved {
                Ok((true, ..)) => {
                    tracing::info!(
                        repo = %self.path().display(),
                        "branch sync: fast-forwarded '{branch}' {} → {}",
                        short(&local),
                        short(&remote)
                    );
                    done(BranchSyncOutcome::FastForwarded, Some(local_ref))
                }
                _ => done(BranchSyncOutcome::CheckedOutBehind, Some(remote_ref)),
            };
        }
        if self.is_ancestor(&remote, &local).await {
            return done(BranchSyncOutcome::LocalAhead, Some(local_ref));
        }
        tracing::warn!(
            repo = %self.path().display(),
            "branch sync: local '{branch}' ({}) diverged from origin/{branch} ({}) — leaving it, using origin/{branch}",
            short(&local),
            short(&remote)
        );
        done(BranchSyncOutcome::Diverged, Some(remote_ref))
    }

    /// Full sha of `full_ref`'s commit; `None` when it doesn't exist. Raw
    /// spawn so a missing ref (an expected answer) isn't logged as a failure.
    async fn commit_of(&self, full_ref: &str) -> Option<String> {
        let spec = format!("{full_ref}^{{commit}}");
        match self
            .run_raw(
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &spec,
                ],
                &[],
            )
            .await
        {
            Ok((true, out, ..)) => Some(out.trim().to_string()).filter(|s| !s.is_empty()),
            _ => None,
        }
    }

    /// `git merge-base --is-ancestor a b` (both are full shas from
    /// [`Self::commit_of`]).
    async fn is_ancestor(&self, a: &str, b: &str) -> bool {
        matches!(
            self.run_raw(&["merge-base", "--is-ancestor", a, b], &[])
                .await,
            Ok((true, ..))
        )
    }
}

fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

#[cfg(test)]
#[path = "branch_sync_tests.rs"]
mod tests;
