//! Endpoint #3: `POST /api/v1/onboarding/root` — valid only while 0 users.

use axum::extract::State;
use axum::Json;
use otto_core::api::{LoginResp, OnboardRootReq};
use otto_core::Error;
use otto_rbac::AuthRepo;
use otto_state::UsersRepo;

use crate::error::ApiResult;
use crate::state::ServerCtx;

/// Minimum root password length (mirrors the onboarding wizard hint).
const MIN_PASSWORD_LEN: usize = 10;

pub async fn onboard_root(
    State(ctx): State<ServerCtx>,
    Json(req): Json<OnboardRootReq>,
) -> ApiResult<Json<LoginResp>> {
    let users = UsersRepo::new(ctx.pool.clone());
    if users.count().await? > 0 {
        return Err(Error::Conflict("already onboarded".into()).into());
    }
    if req.password.chars().count() < MIN_PASSWORD_LEN {
        return Err(Error::Invalid(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        ))
        .into());
    }

    let hash = otto_rbac::hash_password(&req.password)?;
    let display_name = req
        .display_name
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| "Root".to_string());
    let user = users.create("root", &hash, &display_name, true).await?;
    let token = AuthRepo::new(ctx.pool.clone()).issue(&user.id).await?;

    Ok(Json(LoginResp { token, user }))
}
