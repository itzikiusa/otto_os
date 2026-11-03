//! Routes for worktree status/removal, stash detail and the bulk branch
//! delete (engine: [`crate::gitops`]). A child of `http` so it shares the
//! repo context, locks and token helpers.

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use otto_core::api::{
    BranchDeleteItem, BranchDeleteResp, BranchDeleteResult, StashDetail, WorktreeInfo,
};
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use serde::Deserialize;

use super::{optional_token, remote_shares_origin_host, repo_ctx, repo_lock, ApiResult, GitCtx};
use crate::gitops::err_text;

#[derive(Deserialize)]
pub(super) struct WorktreeRemoveReq {
    /// Absolute worktree path, exactly as returned by GET …/worktrees.
    pub(super) path: String,
    /// Remove even though it has risks (uncommitted / untracked files,
    /// commits nothing else holds, a lock, submodules, unknown status).
    #[serde(default)]
    pub(super) force: Option<bool>,
}

/// `POST /repos/{id}/worktrees/remove` — see [`crate::LocalGit::worktree_remove_safe`].
/// Never deletes the branch. Returns the refreshed list.
pub(super) async fn repo_worktree_remove<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<WorktreeRemoveReq>,
) -> ApiResult<Json<Vec<WorktreeInfo>>> {
    let path = req.path.trim();
    if path.is_empty() {
        return Err(Error::Invalid("worktree path must not be empty".into()).into());
    }
    let lock = repo_lock(&id);
    let g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    git.worktree_remove_safe(path, req.force.unwrap_or(false))
        .await?;
    drop(g);
    Ok(Json(git.worktree_list_detailed().await?))
}

#[derive(Deserialize)]
pub(super) struct WorktreeCreateReq {
    /// The NEW branch to create and check out in the worktree.
    pub(super) branch: String,
    /// What it starts from (branch / sha); `HEAD` when absent.
    #[serde(default)]
    pub(super) base: Option<String>,
}

/// `POST /repos/{id}/worktrees` — new branch + new worktree beside the repo
/// (see [`crate::LocalGit::worktree_create`]). Returns the new worktree.
pub(super) async fn repo_worktree_create<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<WorktreeCreateReq>,
) -> ApiResult<Json<WorktreeInfo>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(
        git.worktree_create(&req.branch, req.base.as_deref())
            .await?,
    ))
}

#[derive(Deserialize)]
pub(super) struct WorktreeStatusQuery {
    path: String,
}

/// `GET /repos/{id}/worktrees/status?path=` — one worktree, freshly inspected
/// (generous status budget, submodules included) with its removal `risks`.
pub(super) async fn repo_worktree_status<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<WorktreeStatusQuery>,
) -> ApiResult<Json<WorktreeInfo>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let (mut w, admin) = git.worktree_inspect(q.path.trim()).await?;
    if admin.is_none() && !w.is_main && !w.prunable {
        w.risks
            .push("Otto can't verify it belongs to this repo, so it won't remove it".into());
    }
    Ok(Json(w))
}

/// `GET /repos/{id}/stashes/{sha}` — the stash's files and patch.
pub(super) async fn repo_stash_detail<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((id, sha)): Path<(Id, String)>,
) -> ApiResult<Json<StashDetail>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(git.stash_detail(sha.trim()).await?))
}

#[derive(Deserialize)]
pub(super) struct BranchesDeleteReq {
    pub(super) items: Vec<BranchDeleteItem>,
}

/// Most branches one bulk delete accepts.
const BULK_DELETE_MAX: usize = 200;

/// Names never deleted in bulk: the checked-out branch and the default one.
fn protected_names(current: &str, default: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = ["main", "master"].iter().map(|s| s.to_string()).collect();
    out.extend(default.map(str::to_string));
    if !current.is_empty() {
        out.push(current.to_string());
    }
    out
}

/// `POST /repos/{id}/branches/delete` — delete many branches, each locally
/// and/or on a remote. Items run in order and independently: one failure is
/// reported in its result and never stops the rest. The current and default
/// branches (and `main` / `master`) are refused. A local branch git won't
/// drop with `-d` (unmerged) comes back `needs_force`; its remote copy is
/// then left alone.
pub(super) async fn repo_branches_delete<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<BranchesDeleteReq>,
) -> ApiResult<Json<BranchDeleteResp>> {
    if req.items.is_empty() {
        return Err(Error::Invalid("no branches to delete".into()).into());
    }
    if req.items.len() > BULK_DELETE_MAX {
        return Err(
            Error::Invalid(format!("at most {BULK_DELETE_MAX} branches per request")).into(),
        );
    }
    let lock = repo_lock(&id);
    let g = lock.lock().await;
    let (repo, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let current = git.current_branch().await.unwrap_or_default();
    let default = git.default_branch().await;
    let protected = protected_names(&current, default.as_deref());
    // Resolved once, lazily: only a remote delete needs the account token.
    let mut token = None;
    let mut results = Vec::with_capacity(req.items.len());
    for item in req.items {
        let name = item.name.trim().to_string();
        let remote = item
            .remote
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_string);
        let mut r = BranchDeleteResult {
            name: name.clone(),
            remote: remote.clone(),
            local_deleted: false,
            remote_deleted: false,
            error: None,
            needs_force: false,
        };
        let want_local = item.local.unwrap_or(true);
        if name.is_empty() || (!want_local && remote.is_none()) {
            r.error = Some("nothing to delete".into());
            results.push(r);
            continue;
        }
        if want_local {
            if protected.contains(&name) {
                r.error = Some(if name == current {
                    "it is the checked-out branch".into()
                } else {
                    "the default branch is protected".into()
                });
                results.push(r);
                continue;
            }
            match git.delete_branch(&name, item.force.unwrap_or(false)).await {
                Ok(()) => r.local_deleted = true,
                Err(e) => {
                    let msg = err_text(&e);
                    r.needs_force = msg.contains("not fully merged");
                    r.error = Some(msg);
                    results.push(r);
                    continue;
                }
            }
        }
        if let Some(remote) = remote {
            let rb = item
                .remote_branch
                .as_deref()
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .unwrap_or(&name)
                .to_string();
            if protected_names("", default.as_deref()).contains(&rb) {
                r.error = Some(format!("{remote}/{rb} is a protected default branch"));
                results.push(r);
                continue;
            }
            // The bound account's token belongs to origin's host: offer it
            // only to a remote on the same host (as the single delete does).
            let tok = if remote == "origin" || remote_shares_origin_host(&git, &remote).await {
                if token.is_none() {
                    token = Some(
                        optional_token(&s, &user, &repo)
                            .await
                            .map_err(|e| err_text(&e)),
                    );
                }
                match token.clone() {
                    Some(Ok(t)) => t,
                    Some(Err(msg)) => {
                        r.error = Some(msg);
                        results.push(r);
                        continue;
                    }
                    None => None,
                }
            } else {
                None
            };
            match git.delete_remote_branch_on(&remote, &rb, tok).await {
                Ok(_) => r.remote_deleted = true,
                Err(e) => r.error = Some(err_text(&e)),
            }
        }
        results.push(r);
    }
    drop(g);
    Ok(Json(BranchDeleteResp {
        results,
        status: git.status().await?,
    }))
}

#[cfg(test)]
#[test]
fn bulk_delete_protects_current_default_main_and_master() {
    let p = protected_names("feat/x", Some("develop"));
    for name in ["main", "master", "develop", "feat/x"] {
        assert!(p.iter().any(|n| n == name), "{name} must be protected");
    }
    assert!(!p.iter().any(|n| n == "feat/y"));
    assert!(!protected_names("", None).iter().any(|n| n.is_empty()));
}
