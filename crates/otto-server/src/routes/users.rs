//! Endpoints #7-10: users CRUD (root only).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{CreateUserReq, UpdateUserReq};
use otto_core::domain::User;
use otto_core::{Error, Id};
use otto_rbac::AuthRepo;
use otto_state::UsersRepo;

use crate::auth::{require_root, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// `GET /api/v1/users`
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<User>>> {
    require_root(&user)?;
    Ok(Json(UsersRepo::new(ctx.pool.clone()).list().await?))
}

/// `POST /api/v1/users` — 409 on duplicate username.
pub async fn create(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateUserReq>,
) -> ApiResult<Json<User>> {
    require_root(&user)?;
    if req.username.trim().is_empty() {
        return Err(Error::Invalid("username must not be empty".into()).into());
    }
    // Enforce the shared minimum-password policy (same rule as onboarding).
    otto_rbac::validate_password(&req.password)?;
    let hash = otto_rbac::hash_password(&req.password)?;
    let display_name = req.display_name.unwrap_or_else(|| req.username.clone());
    let created = UsersRepo::new(ctx.pool.clone())
        .create(req.username.trim(), &hash, &display_name, false)
        .await?;
    Ok(Json(created))
}

/// `PATCH /api/v1/users/{id}`
pub async fn update(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateUserReq>,
) -> ApiResult<Json<User>> {
    require_root(&user)?;
    let repo = UsersRepo::new(ctx.pool.clone());
    let target = repo.get(&id).await?;
    if target.is_root && req.disabled == Some(true) {
        return Err(Error::Invalid("the root user cannot be disabled".into()).into());
    }
    let password_hash = match &req.password {
        // Enforce the shared minimum-password policy on any password change.
        Some(p) => {
            otto_rbac::validate_password(p)?;
            Some(otto_rbac::hash_password(p)?)
        }
        None => None,
    };
    let updated = repo
        .update(
            &id,
            req.display_name.as_deref(),
            password_hash.as_deref(),
            req.disabled,
        )
        .await?;

    // A changed credential or a disabled account must invalidate every
    // outstanding token for this user (login sessions + API tokens), so a
    // leaked/old token can't keep working after the change.
    if password_hash.is_some() || req.disabled == Some(true) {
        AuthRepo::new(ctx.pool.clone())
            .revoke_all_for_user(&id)
            .await?;
    }

    Ok(Json(updated))
}

/// `DELETE /api/v1/users/{id}` — soft delete (disables); 400 for root user.
pub async fn remove(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_root(&user)?;
    let repo = UsersRepo::new(ctx.pool.clone());
    let target = repo.get(&id).await?;
    if target.is_root {
        return Err(Error::Invalid("the root user cannot be disabled".into()).into());
    }
    repo.update(&id, None, None, Some(true)).await?;
    // Disabling the account invalidates all of its outstanding tokens.
    AuthRepo::new(ctx.pool.clone())
        .revoke_all_for_user(&id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
