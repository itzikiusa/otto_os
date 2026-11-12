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
//! - the old tip is kept on a backup branch `otto-backup/<b>-<short sha>`
//!   (also in the reflog) BEFORE the branch moves;
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
}

/// Routes owned by this module.
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new().route("/repos/{id}/branch/update-to-upstream", post(update::<S>))
}

/// The backup branch for `branch` at `tip`.
pub(crate) fn backup_name(branch: &str, tip: &str) -> String {
    let short: String = tip.chars().take(10).collect();
    format!("otto-backup/{branch}-{short}")
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

async fn move_to_upstream(git: &LocalGit, branch: &str, ahead: u32) -> Result<String> {
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
    let backup = backup_name(branch, &tip);
    // Create-only (`""` = must not exist yet); an existing backup at the same
    // tip is the same backup from an earlier attempt.
    let backup_ref = format!("refs/heads/{backup}");
    let created = git
        .run(&["update-ref", &backup_ref, &tip, ""])
        .await
        .is_ok();
    if !created {
        let at = git
            .run(&["rev-parse", "--verify", "--quiet", &backup_ref])
            .await?;
        if at.trim() != tip {
            return Err(Error::Conflict(format!(
                "{backup} already exists at another commit — nothing was changed"
            )));
        }
    }
    if let Err(e) = git.run(&["reset", "--keep", &upstream]).await {
        // Refused: nothing moved, so the backup made for this attempt goes
        // too (compare-and-delete — only if it still points at `tip`).
        if created {
            let _ = git.run(&["update-ref", "-d", &backup_ref, &tip]).await;
        }
        return Err(reset_refusal(e));
    }
    Ok(backup)
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
    let backup = move_to_upstream(&git, req.branch.trim(), req.ahead).await?;
    drop(guard);
    Ok(Json(UpdateToUpstreamResp {
        status: git.status().await?,
        backup,
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

        let backup = move_to_upstream(&local, "feat", 1).await.unwrap();
        assert_eq!(
            git(&clone, &["rev-parse", "feat"]),
            git(&clone, &["rev-parse", "origin/feat"])
        );
        assert_eq!(git(&clone, &["rev-parse", &backup]), old, "old tip kept");
        assert_eq!(std::fs::read_to_string(clone.join("a.txt")).unwrap(), "3\n");
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
            backup_name("feat/x", "0123456789abcdef"),
            "otto-backup/feat/x-0123456789"
        );
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
