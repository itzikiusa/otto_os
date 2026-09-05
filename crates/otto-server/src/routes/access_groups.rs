//! Root-managed group memberships and reusable operation presets.
use super::resource_access::{actor, AccessCtx};
use crate::auth::{require_root, CurrentAuthContext};
use crate::error::ApiResult;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::access::{AccessGroup, AccessRole, ResourceKind};
use otto_core::Id;
use otto_state::resource_access::ResourceAccessRepo;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct GroupInput {
    pub name: String,
    pub description: Option<String>,
}
#[derive(Deserialize)]
pub struct RoleInput {
    pub name: String,
    pub description: Option<String>,
    pub kind: ResourceKind,
    pub operations: Vec<String>,
    #[serde(default)]
    pub grantable_operations: Vec<String>,
}

pub async fn list_groups<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<Vec<AccessGroup>>> {
    require_root(&auth.effective_user)?;
    Ok(Json(
        ResourceAccessRepo::new(ctx.access_pool())
            .list_groups()
            .await?,
    ))
}
pub async fn create_group<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<GroupInput>,
) -> ApiResult<Json<AccessGroup>> {
    require_root(&auth.effective_user)?;
    Ok(Json(
        ResourceAccessRepo::new(ctx.access_pool())
            .create_group(&req.name, req.description.as_deref(), &actor(&auth))
            .await?,
    ))
}
pub async fn update_group<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<GroupInput>,
) -> ApiResult<Json<AccessGroup>> {
    require_root(&auth.effective_user)?;
    Ok(Json(
        ResourceAccessRepo::new(ctx.access_pool())
            .update_group(&id, &req.name, req.description.as_deref(), &actor(&auth))
            .await?,
    ))
}
pub async fn delete_group<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    require_root(&auth.effective_user)?;
    ResourceAccessRepo::new(ctx.access_pool())
        .delete_group(&id, &actor(&auth))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn members<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<Id>>> {
    require_root(&auth.effective_user)?;
    Ok(Json(
        ResourceAccessRepo::new(ctx.access_pool())
            .group_members(&id)
            .await?,
    ))
}
pub async fn add_member<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((id, uid)): Path<(Id, Id)>,
) -> ApiResult<StatusCode> {
    require_root(&auth.effective_user)?;
    ResourceAccessRepo::new(ctx.access_pool())
        .add_group_member(&id, &uid, &actor(&auth))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn remove_member<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((id, uid)): Path<(Id, Id)>,
) -> ApiResult<StatusCode> {
    require_root(&auth.effective_user)?;
    ResourceAccessRepo::new(ctx.access_pool())
        .remove_group_member(&id, &uid, &actor(&auth))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn list_roles<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<Vec<AccessRole>>> {
    require_root(&auth.effective_user)?;
    Ok(Json(
        ResourceAccessRepo::new(ctx.access_pool())
            .list_roles()
            .await?,
    ))
}
pub async fn create_role<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<RoleInput>,
) -> ApiResult<Json<AccessRole>> {
    require_root(&auth.effective_user)?;
    Ok(Json(
        ResourceAccessRepo::new(ctx.access_pool())
            .create_role(
                &req.name,
                req.description.as_deref(),
                req.kind,
                &req.operations,
                &req.grantable_operations,
                &actor(&auth),
            )
            .await?,
    ))
}
pub async fn update_role<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<RoleInput>,
) -> ApiResult<Json<AccessRole>> {
    require_root(&auth.effective_user)?;
    Ok(Json(
        ResourceAccessRepo::new(ctx.access_pool())
            .update_role(
                &id,
                &req.name,
                req.description.as_deref(),
                req.kind,
                &req.operations,
                &req.grantable_operations,
                &actor(&auth),
            )
            .await?,
    ))
}
pub async fn delete_role<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    require_root(&auth.effective_user)?;
    ResourceAccessRepo::new(ctx.access_pool())
        .delete_role(&id, &actor(&auth))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
