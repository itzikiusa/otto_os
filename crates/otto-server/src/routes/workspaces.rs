//! Endpoints #11-16: workspaces CRUD + members.

use axum::extract::{Path, State};
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

/// `GET /api/v1/workspaces` — root sees all (as admin); others their own.
/// The system-owned scratch workspace is hidden here (user-facing lists);
/// `GET /workspaces/scratch` is its one read route.
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
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
    Ok(Json(
        rows.into_iter()
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
    let root_path = expand_home(req.root_path.trim());
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
    let ws = repo(&ctx)
        .update(
            &id,
            req.name.as_deref(),
            req.root_path.as_deref(),
            req.settings.as_ref(),
            req.archived,
        )
        .await?;
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

    let current = repo.members(&id).await?;
    for member in &current {
        if !req.members.iter().any(|m| m.user_id == member.user_id) {
            repo.remove_member(&id, &member.user_id).await?;
        }
    }
    for entry in &req.members {
        repo.set_member(&id, &entry.user_id, entry.role).await?;
    }
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

/// Expand a leading `~/` to the user's home directory.
fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_system_workspace_is_409_for_scratch_and_ok_otherwise() {
        let err = reject_system_workspace(&SCRATCH_WORKSPACE_ID.to_string()).unwrap_err();
        assert!(matches!(err.0, Error::Conflict(_)), "{err:?}");
        assert!(reject_system_workspace(&otto_core::new_id()).is_ok());
        // Case matters: only the exact well-known id is system-owned.
        assert!(reject_system_workspace(&"Scratch".to_string()).is_ok());
    }
}
