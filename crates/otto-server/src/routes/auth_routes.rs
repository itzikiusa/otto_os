//! Endpoints #4-6: login, logout, me.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use otto_core::api::{LoginReq, LoginResp};
use otto_core::domain::User;
use otto_core::Error;
use otto_rbac::AuthRepo;
use otto_state::UsersRepo;

use crate::auth::{BearerToken, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// `POST /api/v1/auth/login` — 401 on unknown user, bad password or disabled.
pub async fn login(
    State(ctx): State<ServerCtx>,
    Json(req): Json<LoginReq>,
) -> ApiResult<Json<LoginResp>> {
    let record = UsersRepo::new(ctx.pool.clone())
        .get_by_username(&req.username)
        .await
        .map_err(|e| match e {
            Error::NotFound(_) => Error::Unauthorized,
            other => other,
        })?;

    if record.user.disabled || !otto_rbac::verify_password(&req.password, &record.password_hash)? {
        return Err(Error::Unauthorized.into());
    }

    let token = AuthRepo::new(ctx.pool.clone())
        .issue(&record.user.id)
        .await?;
    Ok(Json(LoginResp {
        token,
        user: record.user,
    }))
}

/// `POST /api/v1/auth/logout` — revokes the presented token.
pub async fn logout(
    State(ctx): State<ServerCtx>,
    Extension(BearerToken(token)): Extension<BearerToken>,
) -> ApiResult<StatusCode> {
    AuthRepo::new(ctx.pool.clone()).revoke(&token).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/auth/me`
pub async fn me(CurrentUser(user): CurrentUser) -> Json<User> {
    Json(user)
}
