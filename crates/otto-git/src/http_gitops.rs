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
use serde::{Deserialize, Serialize};

use super::{optional_token, remote_shares_origin_host, repo_ctx, repo_lock, ApiResult, GitCtx};
use crate::gitops::err_text;
use crate::transport::TransportCredential;
use crate::worktree_ready::{run_setup, Prepared, SETUP_KEY};

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

/// A new worktree and what it got ([`crate::worktree_ready`]).
#[derive(Serialize)]
pub(super) struct WorktreeCreated {
    #[serde(flatten)]
    info: WorktreeInfo,
    prepared: Prepared,
}

/// `POST /repos/{id}/worktrees` — new branch + new worktree beside the repo
/// (see [`crate::LocalGit::worktree_create`]), given what the main
/// checkout's `.worktreeinclude` names. Returns the new worktree.
pub(super) async fn repo_worktree_create<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<WorktreeCreateReq>,
) -> ApiResult<Json<WorktreeCreated>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let info = git
        .worktree_create(&req.branch, req.base.as_deref())
        .await?;
    // From the MAIN checkout (its real .env / node_modules), even when the
    // new worktree is made from another worktree's repo.
    let main = main_checkout(&git).await;
    let mut prepared = crate::LocalGit::new(&main)
        .prepare_worktree(std::path::Path::new(&info.path))
        .await;
    prepared.setup = setup_of(&s, &main).await.map(|(name, _)| name);
    Ok(Json(WorktreeCreated { info, prepared }))
}

/// The main checkout of `git`'s repository (itself when it is the main one).
async fn main_checkout(git: &crate::LocalGit) -> std::path::PathBuf {
    let main = git
        .worktree_list()
        .await
        .ok()
        .and_then(|l| l.into_iter().find(|w| w.is_main));
    main.map_or_else(|| git.path().to_path_buf(), |w| w.path.into())
}

/// The worktree setup command (`(name, command)`) of the repository whose
/// main checkout is `main`, if set.
async fn setup_of<S: GitCtx>(s: &S, main: &std::path::Path) -> Option<(String, String)> {
    let all = otto_state::SettingsRepo::new(s.store().pool().clone())
        .get(SETUP_KEY)
        .await
        .ok()??;
    let e = all.get(main.to_str()?)?;
    let command = e.get("command")?.as_str()?.trim().to_string();
    let name = e
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&command)
        .to_string();
    (!command.is_empty()).then_some((name, command))
}

#[derive(Deserialize)]
pub(super) struct WorktreeSetupReq {
    /// The new worktree (absolute, as GET …/worktrees lists it; not the main checkout).
    path: String,
}

/// `POST /repos/{id}/worktrees/setup` — run the repo's setup command in one
/// of its worktrees (a person's request only: it runs a shell command).
/// Streams NDJSON: `{"line": …}` with its latest output line while it runs
/// (at most every second), then `{"result": SetupResult}` (or `{"error": …}`).
/// It runs to its end even when the caller goes away.
pub(super) async fn repo_worktree_setup<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<WorktreeSetupReq>,
) -> ApiResult<axum::response::Response> {
    if !otto_core::auth::request_is_person() {
        return Err(Error::Forbidden("only a person can run a worktree's setup".into()).into());
    }
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    let want = std::fs::canonicalize(&req.path).map_err(|_| Error::NotFound("worktree".into()))?;
    let found = git.worktree_list().await?.into_iter().any(|w| {
        !w.is_main && std::fs::canonicalize(&w.path).ok().as_deref() == Some(want.as_path())
    });
    if !found {
        return Err(Error::NotFound("worktree".into()).into());
    }
    let (name, command) = setup_of(&s, &main_checkout(&git).await)
        .await
        .ok_or_else(|| Error::NotFound("worktree setup".into()))?;
    let (tx, lines) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (done_tx, done) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = done_tx.send(run_setup(&name, &command, &want, Some(tx)).await);
    });
    let ndjson = |v: serde_json::Value| axum::body::Bytes::from(format!("{v}\n"));
    // Every progress line, then (once the sender is gone) the result.
    let body =
        futures_util::stream::unfold((lines, Some(done)), move |(mut lines, done)| async move {
            if let Some(l) = lines.recv().await {
                return Some((
                    Ok::<_, std::convert::Infallible>(ndjson(serde_json::json!({ "line": l }))),
                    (lines, done),
                ));
            }
            let last = match done?.await {
                Ok(Ok(r)) => serde_json::json!({ "result": r }),
                Ok(Err(e)) => serde_json::json!({ "error": e.to_string() }),
                Err(_) => serde_json::json!({ "error": "the setup stopped unexpectedly" }),
            };
            Some((Ok(ndjson(last)), (lines, None)))
        });
    Ok(axum::response::Response::builder()
        .header(
            axum::http::header::CONTENT_TYPE,
            "application/x-ndjson; charset=utf-8",
        )
        .body(axum::body::Body::from_stream(body))
        .map_err(|e| Error::Internal(e.to_string()))?)
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
    // Locals go in one `git branch -d` (and one `-D` for forced ones); the
    // remotes after them, one push per remote.
    let mut locals: [Vec<(usize, String)>; 2] = [Vec::new(), Vec::new()];
    let mut remote_wants = Vec::new();
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
            locals[usize::from(item.force.unwrap_or(false))].push((results.len(), name.clone()));
        }
        if let Some(remote) = remote {
            let rb = item
                .remote_branch
                .as_deref()
                .map(str::trim)
                .filter(|b| !b.is_empty())
                .unwrap_or(&name)
                .to_string();
            remote_wants.push((results.len(), remote, rb));
        }
        results.push(r);
    }
    for (force, batch) in locals.iter().enumerate() {
        if batch.is_empty() {
            continue;
        }
        let names: Vec<String> = batch.iter().map(|(_, n)| n.clone()).collect();
        for ((i, _), res) in batch
            .iter()
            .zip(git.delete_branches(&names, force == 1).await)
        {
            match res {
                Ok(()) => results[*i].local_deleted = true,
                Err(e) => {
                    let msg = err_text(&e);
                    results[*i].needs_force = msg.contains("not fully merged");
                    results[*i].error = Some(msg);
                }
            }
        }
    }
    // One push per remote for all its branches; if that push fails (one ref
    // already gone, a protected branch…) each is retried on its own, so the
    // per-item results stay exact. A local that wasn't deleted keeps its
    // remote copy.
    type RemoteBatch = (String, Option<TransportCredential>, Vec<(usize, String)>);
    let mut by_remote: Vec<RemoteBatch> = Vec::new();
    for (i, remote, rb) in remote_wants {
        if results[i].error.is_some() {
            continue;
        }
        if protected_names("", default.as_deref()).contains(&rb) {
            results[i].error = Some(format!("{remote}/{rb} is a protected default branch"));
            continue;
        }
        if let Some((.., items)) = by_remote.iter_mut().find(|(r, ..)| *r == remote) {
            items.push((i, rb));
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
                    results[i].error = Some(msg);
                    continue;
                }
                None => None,
            }
        } else {
            None
        };
        by_remote.push((remote, tok, vec![(i, rb)]));
    }
    for (remote, tok, items) in by_remote {
        let names: Vec<String> = items.iter().map(|(_, n)| n.clone()).collect();
        if items.len() > 1
            && git
                .delete_remote_branches_on(&remote, &names, tok.clone())
                .await
                .is_ok()
        {
            for (i, _) in &items {
                results[*i].remote_deleted = true;
            }
            continue;
        }
        for (i, rb) in items {
            match git.delete_remote_branch_on(&remote, &rb, tok.clone()).await {
                Ok(_) => results[i].remote_deleted = true,
                Err(e) => results[i].error = Some(err_text(&e)),
            }
        }
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
