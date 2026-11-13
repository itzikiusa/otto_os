//! "Update to the PR's version" for a checked-out PR branch whose remote was
//! force-pushed — route merged into `crate::http::router`.
//!
//! A PR's "Check out" switches to the local branch and never pulls; after a
//! force-push that local branch has DIVERGED from `origin/<b>` (the old
//! commits on one side, the rewritten ones on the other), and a fast-forward
//! pull cannot fix it. `POST /repos/{id}/branch/update-to-upstream`
//! `{branch, ahead}` moves the checked-out branch to its upstream, and is
//! never silent and never loses work:
//! - only the CHECKED-OUT branch, and only when it has an upstream;
//! - `ahead` is the local-only commit count the person confirmed; any other
//!   count now is a 409 "changed — re-check" (someone committed meanwhile);
//! - the old tip is kept on a backup branch `otto-backup/<b>-<yyyymmdd>-<short
//!   sha>` (also in the reflog) BEFORE the branch moves; Otto never removes one;
//! - the repo lock does not cover a commit made in a terminal meanwhile: the
//!   move starts only while `HEAD` is still the backed-up tip (else 409,
//!   nothing changed), and a commit that slips in between that check and the
//!   reset is backed up too (`also_kept`, from `ORIG_HEAD`);
//! - `git reset --keep`: uncommitted changes stay, and a change the move would
//!   overwrite makes git refuse with nothing moved (409 with git's words).

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Extension, Json, Router};
use otto_core::api::RepoStatusResp;
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};

use crate::branch_sync::is_syncable_branch_name;
use crate::http::{repo_ctx, repo_lock, ApiResult, GitCtx};
use crate::local::LocalGit;

/// Body of `POST /repos/{id}/branch/update-to-upstream`.
#[derive(Debug, Deserialize)]
pub struct UpdateToUpstreamReq {
    pub branch: String,
    /// The local-only commit count the confirm named.
    pub ahead: u32,
}

/// The answer: the fresh status and where the old tip was kept.
#[derive(Debug, Serialize)]
pub struct UpdateToUpstreamResp {
    pub status: RepoStatusResp,
    pub backup: String,
    /// A second backup branch: a commit made on the branch while it was
    /// being updated (an agent in a terminal), kept rather than left only in
    /// the reflog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub also_kept: Option<String>,
}

/// Routes owned by this module.
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new().route("/repos/{id}/branch/update-to-upstream", post(update::<S>))
}

/// The backup branch for `branch` at `tip`, made on `day` (`YYYYMMDD`):
/// unique per commit, and it says when. Never removed by Otto.
pub(crate) fn backup_name(branch: &str, day: &str, tip: &str) -> String {
    let short: String = tip.chars().take(10).collect();
    format!("otto-backup/{branch}-{day}-{short}")
}

/// Today in the person's time zone, as a backup name carries it.
fn today() -> String {
    chrono::Local::now().format("%Y%m%d").to_string()
}

/// A `git reset --keep` refusal ("Entry '…' not uptodate. Cannot merge.")
/// is the person's own uncommitted work in the way — a 409, not an outage.
fn reset_refusal(e: Error) -> Error {
    match e {
        Error::Upstream(m) => Error::Conflict(format!(
            "your uncommitted changes would be overwritten — commit or stash them first ({m})"
        )),
        other => other,
    }
}

/// Where the branch moved and what was kept.
#[derive(Debug)]
struct Moved {
    backup: String,
    also_kept: Option<String>,
}

/// Create-only `refs/heads/<name>` at `at` (`""` = must not exist yet).
/// `true` = made now; `false` = it already pointed at `at` (the same backup
/// from an earlier attempt).
async fn keep_at(git: &LocalGit, name: &str, at: &str) -> Result<bool> {
    let full = format!("refs/heads/{name}");
    if git.run(&["update-ref", &full, at, ""]).await.is_ok() {
        return Ok(true);
    }
    let cur = git
        .run(&["rev-parse", "--verify", "--quiet", &full])
        .await?;
    if cur.trim() != at {
        return Err(Error::Conflict(format!(
            "{name} already exists at another commit — nothing was changed"
        )));
    }
    Ok(false)
}

/// Undo a backup made for an attempt that moved nothing (compare-and-delete:
/// only while it still points at `tip`).
async fn drop_backup(git: &LocalGit, name: &str, tip: &str, created: bool) {
    if created {
        let _ = git
            .run(&["update-ref", "-d", &format!("refs/heads/{name}"), tip])
            .await;
    }
}

async fn move_to_upstream(git: &LocalGit, branch: &str, ahead: u32) -> Result<Moved> {
    if !is_syncable_branch_name(branch) {
        return Err(Error::Invalid(format!("not a branch name: {branch:?}")));
    }
    if git.current_branch().await? != branch {
        return Err(Error::Conflict(format!(
            "{branch} is not the checked-out branch — check it out first"
        )));
    }
    let local = format!("refs/heads/{branch}");
    let tip = git
        .run(&["rev-parse", "--verify", "--quiet", &local])
        .await?
        .trim()
        .to_string();
    // The upstream as `branch.<b>.remote/merge` names it (what "Check out"'s
    // `checkout -b <b> --track origin/<b>` sets). Not `refs/heads/<b>@{upstream}`:
    // git resolves `@{upstream}` only after a SHORT branch name, and the short
    // form could be shadowed by a tag of the same name.
    let no_upstream = || Error::Conflict(format!("{branch} has no upstream branch to update to"));
    let upstream_ref = git
        .run(&["for-each-ref", "--format=%(upstream)", &local])
        .await?
        .trim()
        .to_string();
    if upstream_ref.is_empty() {
        return Err(no_upstream());
    }
    let upstream = git
        .run(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{upstream_ref}^{{commit}}"),
        ])
        .await
        .map_err(|_| no_upstream())?
        .trim()
        .to_string();
    let now: u32 = git
        .run(&["rev-list", "--count", &format!("{upstream}..{tip}")])
        .await?
        .trim()
        .parse()
        .map_err(|_| Error::Internal("rev-list --count: not a number".into()))?;
    if now != ahead {
        return Err(Error::Conflict(format!(
            "{branch} changed — re-check: it now has {now} commit(s) the pull request doesn't (you confirmed {ahead})"
        )));
    }
    let day = today();
    let backup = backup_name(branch, &day, &tip);
    let created = keep_at(git, &backup, &tip).await?;
    finish_move(git, branch, &day, &tip, &upstream, backup, created).await
}

/// Move the checked-out branch from the backed-up `tip` to `upstream`. Otto's
/// repo lock does not cover a commit made in a terminal meanwhile, so the
/// move starts only while `HEAD` is still `tip` (else nothing changes: 409),
/// and whatever the reset did move away from (`ORIG_HEAD`) is backed up too
/// should a commit slip in between that check and the reset.
async fn finish_move(
    git: &LocalGit,
    branch: &str,
    day: &str,
    tip: &str,
    upstream: &str,
    backup: String,
    created: bool,
) -> Result<Moved> {
    let head = git
        .run(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .await?
        .trim()
        .to_string();
    if head != tip {
        drop_backup(git, &backup, tip, created).await;
        return Err(Error::Conflict(format!(
            "{branch} changed — re-check: a commit landed on it while it was being updated; nothing was changed"
        )));
    }
    if let Err(e) = git.run(&["reset", "--keep", upstream]).await {
        // Refused: nothing moved, so the backup made for this attempt goes too.
        drop_backup(git, &backup, tip, created).await;
        return Err(reset_refusal(e));
    }
    let also_kept = keep_what_moved(git, branch, day, tip).await;
    Ok(Moved { backup, also_kept })
}

/// After the reset: a second backup at `ORIG_HEAD` (what the reset moved
/// away from) when that is not the backed-up `tip`.
async fn keep_what_moved(git: &LocalGit, branch: &str, day: &str, tip: &str) -> Option<String> {
    let orig = git
        .run(&["rev-parse", "--verify", "--quiet", "ORIG_HEAD^{commit}"])
        .await
        .ok()?;
    let orig = orig.trim();
    if orig.is_empty() || orig == tip {
        return None;
    }
    let name = backup_name(branch, day, orig);
    match keep_at(git, &name, orig).await {
        Ok(_) => Some(name),
        Err(e) => {
            tracing::warn!(
                branch,
                commit = orig,
                "could not back up a commit made during the update: {e}"
            );
            None
        }
    }
}

async fn update<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpdateToUpstreamReq>,
) -> ApiResult<Json<UpdateToUpstreamResp>> {
    let lock = repo_lock(&id);
    let guard = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let moved = move_to_upstream(&git, req.branch.trim(), req.ahead).await?;
    drop(guard);
    Ok(Json(UpdateToUpstreamResp {
        status: git.status().await?,
        backup: moved.backup,
        also_kept: moved.also_kept,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn identity(dir: &Path) {
        git(dir, &["config", "user.name", "Test"]);
        git(dir, &["config", "user.email", "test@example.invalid"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
    }

    /// `origin` with `feat`, a clone that checked `feat` out, then a
    /// force-push on origin (the tip amended): the clone is 1 ahead, 1 behind.
    fn force_pushed() -> (tempfile::TempDir, std::path::PathBuf, LocalGit) {
        let root = tempfile::tempdir().unwrap();
        let origin = root.path().join("origin");
        std::fs::create_dir(&origin).unwrap();
        git(&origin, &["init", "-b", "main"]);
        identity(&origin);
        std::fs::write(origin.join("a.txt"), "1\n").unwrap();
        git(&origin, &["add", "."]);
        git(&origin, &["commit", "-m", "base"]);
        git(&origin, &["checkout", "-b", "feat"]);
        std::fs::write(origin.join("a.txt"), "2\n").unwrap();
        git(&origin, &["commit", "-am", "feat v1"]);
        // The clone's default branch is main, so `feat` exists only as
        // origin/feat until "Check out" creates it.
        git(&origin, &["checkout", "-q", "main"]);
        let clone = root.path().join("clone");
        git(
            root.path(),
            &["clone", "-q", origin.to_str().unwrap(), "clone"],
        );
        identity(&clone);
        // Exactly how Otto's "Check out" creates a local branch for a remote one.
        git(
            &clone,
            &["checkout", "-q", "-b", "feat", "--track", "origin/feat"],
        );
        git(&origin, &["checkout", "-q", "feat"]);
        std::fs::write(origin.join("a.txt"), "3\n").unwrap();
        git(&origin, &["commit", "-q", "-a", "--amend", "-m", "feat v2"]);
        git(&clone, &["fetch", "-q"]);
        let local = LocalGit::new(&clone);
        (root, clone, local)
    }

    #[tokio::test]
    async fn a_force_pushed_branch_moves_to_its_upstream_and_keeps_a_backup() {
        let (_root, clone, local) = force_pushed();
        let old = git(&clone, &["rev-parse", "feat"]);
        let wrong = move_to_upstream(&local, "feat", 2)
            .await
            .expect_err("the confirmed count must match");
        assert!(matches!(wrong, Error::Conflict(_)), "{wrong}");
        assert_eq!(git(&clone, &["rev-parse", "feat"]), old, "nothing moved");

        let moved = move_to_upstream(&local, "feat", 1).await.unwrap();
        assert_eq!(
            git(&clone, &["rev-parse", "feat"]),
            git(&clone, &["rev-parse", "origin/feat"])
        );
        assert_eq!(
            git(&clone, &["rev-parse", &moved.backup]),
            old,
            "old tip kept"
        );
        assert_eq!(moved.backup, backup_name("feat", &today(), &old));
        assert_eq!(moved.also_kept, None, "nothing slipped in");
        assert_eq!(std::fs::read_to_string(clone.join("a.txt")).unwrap(), "3\n");
    }

    /// A commit made after the tip was backed up (an agent in a terminal)
    /// stops the move: nothing changes, and this attempt's backup goes.
    #[tokio::test]
    async fn a_commit_landing_after_the_backup_moves_nothing() {
        let (_root, clone, local) = force_pushed();
        let old = git(&clone, &["rev-parse", "feat"]);
        let upstream = git(&clone, &["rev-parse", "origin/feat"]);
        let backup = backup_name("feat", "20261113", &old);
        assert!(keep_at(&local, &backup, &old).await.unwrap());
        git(
            &clone,
            &["commit", "-q", "--allow-empty", "-m", "agent's commit"],
        );
        let sneaky = git(&clone, &["rev-parse", "feat"]);
        let err = finish_move(&local, "feat", "20261113", &old, &upstream, backup, true)
            .await
            .expect_err("HEAD moved");
        assert!(err.to_string().contains("changed — re-check"), "{err}");
        assert_eq!(git(&clone, &["rev-parse", "feat"]), sneaky, "nothing moved");
        assert_eq!(git(&clone, &["branch", "--list", "otto-backup/*"]), "");
    }

    /// A commit that slips in between the check and the reset is what the
    /// reset moves away from (`ORIG_HEAD`): it gets its own backup.
    #[tokio::test]
    async fn a_commit_the_reset_moved_away_from_gets_its_own_backup() {
        let (_root, clone, local) = force_pushed();
        let old = git(&clone, &["rev-parse", "feat"]);
        git(
            &clone,
            &["commit", "-q", "--allow-empty", "-m", "agent's commit"],
        );
        let sneaky = git(&clone, &["rev-parse", "feat"]);
        git(&clone, &["reset", "-q", "--keep", "origin/feat"]);
        let kept = keep_what_moved(&local, "feat", "20261113", &old)
            .await
            .expect("a second backup");
        assert_eq!(kept, backup_name("feat", "20261113", &sneaky));
        assert_eq!(git(&clone, &["rev-parse", &kept]), sneaky);
        // Asked again: the same backup, not a second copy.
        assert_eq!(
            keep_what_moved(&local, "feat", "20261113", &old).await,
            Some(kept)
        );
        // Nothing else moved away: no second backup.
        assert_eq!(
            keep_what_moved(&local, "feat", "20261113", &sneaky).await,
            None
        );
    }

    #[tokio::test]
    async fn uncommitted_work_in_the_way_refuses_and_moves_nothing() {
        let (_root, clone, local) = force_pushed();
        let old = git(&clone, &["rev-parse", "feat"]);
        std::fs::write(clone.join("a.txt"), "mine\n").unwrap();
        let err = move_to_upstream(&local, "feat", 1)
            .await
            .expect_err("reset --keep refuses");
        assert!(matches!(err, Error::Conflict(_)), "{err}");
        // Refused by the reset itself — not by an earlier precondition.
        assert!(!err.to_string().contains("no upstream"), "{err}");
        assert_eq!(
            git(&clone, &["branch", "--list", "otto-backup/*"]),
            "",
            "a refused move leaves no backup branch behind"
        );
        assert_eq!(git(&clone, &["rev-parse", "feat"]), old);
        assert_eq!(
            std::fs::read_to_string(clone.join("a.txt")).unwrap(),
            "mine\n"
        );
    }

    #[tokio::test]
    async fn only_the_checked_out_branch_is_moved() {
        let (_root, _clone, local) = force_pushed();
        let err = move_to_upstream(&local, "main", 0)
            .await
            .expect_err("main is not checked out");
        assert!(
            err.to_string().contains("not the checked-out branch"),
            "{err}"
        );
    }

    #[test]
    fn backup_names_carry_the_branch_and_a_short_tip() {
        assert_eq!(
            backup_name("feat/x", "20261113", "0123456789abcdef"),
            "otto-backup/feat/x-20261113-0123456789"
        );
        assert_eq!(today().len(), 8);
    }

    #[test]
    fn a_reset_refusal_is_a_conflict_about_local_changes() {
        let e = reset_refusal(Error::Upstream("Entry 'a.rs' not uptodate".into()));
        assert!(
            matches!(e, Error::Conflict(ref m) if m.contains("uncommitted")),
            "{e}"
        );
        assert!(matches!(
            reset_refusal(Error::Invalid("x".into())),
            Error::Invalid(_)
        ));
    }
}
