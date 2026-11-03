//! Endpoints #11-16: workspaces CRUD + members.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{
    CreateWorkspaceReq, MemberEntry, SetMembersReq, UpdateWorkspaceReq, WorkspaceWithRole,
};
use otto_core::domain::{Workspace, WorkspaceRole, SCRATCH_WORKSPACE_ID};
use otto_core::{Error, Id};
use otto_state::WorkspacesRepo;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

fn repo(ctx: &ServerCtx) -> WorkspacesRepo {
    WorkspacesRepo::new(ctx.pool.clone())
}

/// 409 for the system-owned scratch workspace: it is never renamed, moved,
/// archived or given members through the API (`ensure_scratch` would heal it
/// at the next boot anyway). Called FIRST in every mutating workspace handler.
fn reject_system_workspace(id: &Id) -> ApiResult<()> {
    if id == SCRATCH_WORKSPACE_ID {
        return Err(ApiError(Error::Conflict(
            "the scratch workspace is system-owned".into(),
        )));
    }
    Ok(())
}

/// `GET /api/v1/workspaces` query.
#[derive(Debug, Default, serde::Deserialize)]
pub struct ListWorkspacesQuery {
    /// Also list archived (removed) workspaces. Default `false`.
    pub include_archived: Option<bool>,
}

/// `GET /api/v1/workspaces` — root sees all (as admin); others their own.
/// The system-owned scratch workspace is hidden here (user-facing lists);
/// `GET /workspaces/scratch` is its one read route.
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<ListWorkspacesQuery>,
) -> ApiResult<Json<Vec<WorkspaceWithRole>>> {
    let repo = repo(&ctx);
    let rows: Vec<(Workspace, WorkspaceRole)> = if user.is_root {
        repo.list_user_all()
            .await?
            .into_iter()
            .map(|w| (w, WorkspaceRole::Admin))
            .collect()
    } else {
        repo.list_user_for_user(&user.id).await?
    };
    // `DELETE` archives (soft delete); the UI lists every row it gets, so an
    // archived workspace left in the default list came straight back on the
    // next reload. Archived rows are opt-in.
    let include_archived = q.include_archived.unwrap_or(false);
    Ok(Json(
        rows.into_iter()
            .filter(|(w, _)| include_archived || !w.archived)
            .map(|(workspace, my_role)| WorkspaceWithRole { workspace, my_role })
            .collect(),
    ))
}

/// `GET /api/v1/workspaces/scratch` — the daemon's system-owned scratch
/// workspace (home of workspace-less sessions). Any authenticated user
/// (`Agents:View`) may read it: everyone is an implicit Editor there, so the
/// UI needs its `root_path` (the daemon `$HOME`) as the default session cwd.
pub async fn get_scratch(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<Workspace>> {
    Ok(Json(
        repo(&ctx).get(&SCRATCH_WORKSPACE_ID.to_string()).await?,
    ))
}

/// `PATCH`/`DELETE /api/v1/workspaces/scratch` — always `409`.
///
/// `/workspaces/scratch` is a STATIC route, so axum matches it ahead of
/// `/workspaces/{id}` for this exact path and would answer `405 Method Not
/// Allowed` for any method the static route does not register — never reaching
/// {@link update}/{@link archive} and their `reject_system_workspace` guard.
/// Registering both methods here keeps the documented answer (api.md #16a:
/// "PATCH/DELETE /workspaces/scratch and member edits → 409") true, from the
/// same single source: the guard itself. The body, if any, is ignored.
pub async fn reject_scratch_edit(CurrentUser(_user): CurrentUser) -> ApiResult<StatusCode> {
    reject_system_workspace(&SCRATCH_WORKSPACE_ID.to_string())?;
    // Unreachable — the guard above always rejects the scratch id.
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/workspaces` — creator becomes admin member.
pub async fn create(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateWorkspaceReq>,
) -> ApiResult<Json<Workspace>> {
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("workspace name must not be empty".into()).into());
    }
    if req.root_path.trim().is_empty() {
        return Err(Error::Invalid("workspace root_path must not be empty".into()).into());
    }
    // Expand ~ and make sure the directory exists — agent CLIs are spawned
    // with this as cwd, and a missing dir silently falls back to $HOME.
    let root_path = otto_core::paths::expand_home_strict(req.root_path.trim())
        .map_err(Error::Invalid)?
        .to_string_lossy()
        .into_owned();
    std::fs::create_dir_all(&root_path)
        .map_err(|e| Error::Invalid(format!("cannot create workspace directory: {e}")))?;
    let ws = repo(&ctx)
        .create(req.name.trim(), &root_path, &user.id)
        .await?;
    Ok(Json(ws))
}

/// `PATCH /api/v1/workspaces/{id}` — workspace admin.
pub async fn update(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateWorkspaceReq>,
) -> ApiResult<Json<Workspace>> {
    reject_system_workspace(&id)?;
    require_ws_role(&ctx, &user, &id, WorkspaceRole::Admin).await?;
    if let Some(patch) = &req.settings_patch {
        if req.settings.is_some() {
            return Err(Error::Invalid(
                "settings and settings_patch are mutually exclusive".into(),
            )
            .into());
        }
        WorkspacesRepo::validate_settings_patch(patch)?;
    }
    let ws = repo(&ctx)
        .update(
            &id,
            req.name.as_deref(),
            req.root_path.as_deref(),
            req.settings.as_ref(),
            req.archived,
        )
        .await?;
    if let Some(patch) = &req.settings_patch {
        return Ok(Json(repo(&ctx).patch_settings(&id, patch).await?));
    }
    Ok(Json(ws))
}

/// `DELETE /api/v1/workspaces/{id}` — archives (soft delete).
pub async fn archive(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    reject_system_workspace(&id)?;
    require_ws_role(&ctx, &user, &id, WorkspaceRole::Admin).await?;
    repo(&ctx).update(&id, None, None, None, Some(true)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/workspaces/{id}/members` — workspace admin.
pub async fn members(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<MemberEntry>>> {
    require_ws_role(&ctx, &user, &id, WorkspaceRole::Admin).await?;
    list_members(&ctx, &id).await.map(Json)
}

/// `PUT /api/v1/workspaces/{id}/members` — full replacement of the list.
pub async fn set_members(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SetMembersReq>,
) -> ApiResult<Json<Vec<MemberEntry>>> {
    reject_system_workspace(&id)?;
    require_ws_role(&ctx, &user, &id, WorkspaceRole::Admin).await?;
    let repo = repo(&ctx);
    // Existence check (404 for unknown workspace before mutating membership).
    repo.get(&id).await?;

    repo.replace_members(&id, &req.members).await?;
    list_members(&ctx, &id).await.map(Json)
}

async fn list_members(
    ctx: &ServerCtx,
    ws_id: &Id,
) -> Result<Vec<MemberEntry>, crate::error::ApiError> {
    Ok(repo(ctx)
        .members(ws_id)
        .await?
        .into_iter()
        .map(|m| MemberEntry {
            user_id: m.user_id,
            username: m.username,
            display_name: m.display_name,
            role: m.role,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn atomic_settings_patch_validates_before_writes_and_keeps_root_access() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = crate::test_support::mem_pool().await;
        let ctx = ServerCtx::for_tests(&pool, tmp.path().to_path_buf()).await;
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("patch-root", "x", "Root", true)
            .await
            .unwrap();
        let ws = repo(&ctx)
            .create("Original", "/tmp", &user.id)
            .await
            .unwrap();
        for body in [
            serde_json::json!({"name":"Wrong", "settings":{}, "settings_patch":{"notes":"x"}}),
            serde_json::json!({"name":"Wrong", "settings_patch":{"context":null}}),
            serde_json::json!({"name":"Wrong", "settings_patch":[]}),
        ] {
            let request = serde_json::from_value(body).unwrap();
            let error = update(
                Path(ws.id.clone()),
                State(ctx.clone()),
                CurrentUser(user.clone()),
                Json(request),
            )
            .await
            .unwrap_err();
            assert!(matches!(error.0, Error::Invalid(_)));
            assert_eq!(repo(&ctx).get(&ws.id).await.unwrap().name, "Original");
        }
        let request =
            serde_json::from_value(serde_json::json!({"settings_patch":{"notes":"root can save"}}))
                .unwrap();
        let Json(saved) = update(Path(ws.id), State(ctx), CurrentUser(user), Json(request))
            .await
            .unwrap();
        assert_eq!(saved.settings["notes"], "root can save");
    }

    /// Regression: `DELETE /workspaces/{id}` archives the row, and the list
    /// kept returning it — the sidebar shows every listed workspace, so a
    /// removed workspace came back on the next reload. Archived rows are now
    /// only listed on request (`?include_archived=true`), for root and for a
    /// plain member alike.
    #[tokio::test]
    async fn archived_workspace_leaves_the_default_list() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = crate::test_support::mem_pool().await;
        let ctx = ServerCtx::for_tests(&pool, tmp.path().to_path_buf()).await;
        let users = otto_state::UsersRepo::new(pool.clone());
        let root = users.create("del-root", "x", "Root", true).await.unwrap();
        let member = users.create("del-member", "x", "M", false).await.unwrap();
        let kept = repo(&ctx).create("kept", "/tmp", &member.id).await.unwrap();
        let gone = repo(&ctx).create("gone", "/tmp", &member.id).await.unwrap();

        let status = archive(
            Path(gone.id.clone()),
            State(ctx.clone()),
            CurrentUser(member.clone()),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);

        for user in [&root, &member] {
            let ids = |include_archived| {
                let ctx = ctx.clone();
                let user = (*user).clone();
                async move {
                    let Json(rows) = list(
                        State(ctx),
                        CurrentUser(user),
                        Query(ListWorkspacesQuery { include_archived }),
                    )
                    .await
                    .unwrap();
                    rows.into_iter().map(|r| r.workspace.id).collect::<Vec<_>>()
                }
            };
            let listed = ids(None).await;
            assert!(listed.contains(&kept.id), "{}: {listed:?}", user.username);
            assert!(
                !listed.contains(&gone.id),
                "{}: an archived workspace must leave the default list",
                user.username
            );
            assert_eq!(ids(Some(false)).await, listed);
            // Opt-in keeps the archived row reachable (restore / audit).
            assert!(ids(Some(true)).await.contains(&gone.id));
        }
    }

    #[test]
    fn reject_system_workspace_is_409_for_scratch_and_ok_otherwise() {
        let err = reject_system_workspace(&SCRATCH_WORKSPACE_ID.to_string()).unwrap_err();
        assert!(matches!(err.0, Error::Conflict(_)), "{err:?}");
        assert!(reject_system_workspace(&otto_core::new_id()).is_ok());
        // Case matters: only the exact well-known id is system-owned.
        assert!(reject_system_workspace(&"Scratch".to_string()).is_ok());
    }
}
