//! Endpoints #4-6: login, logout, me. Plus API token management (#87-89).
//!
//! Brute-force throttling for `/auth/login` lives in [`crate::login_throttle`];
//! this module wires the real socket peer (never a forwarding header) and the
//! global per-username tally into the handler (audit S5).

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use otto_core::api::{
    ApiTokenInfo, CreateApiTokenReq, CreateApiTokenResp, LoginReq, LoginResp, Problem,
};
use otto_core::domain::User;
use otto_core::{Error, Id};
use otto_rbac::AuthRepo;
use otto_state::{NewAuditEntry, UsersRepo};

use crate::auth::{BearerToken, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::login_throttle::{self, AttemptStore};
use crate::state::ServerCtx;

/// Build the 429 response for a locked-out key, with a `Retry-After` header.
fn too_many_requests(retry_after: Duration) -> Response {
    let secs = retry_after.as_secs().max(1);
    let body = Problem {
        code: "too_many_requests".to_string(),
        message: "too many failed login attempts; try again later".to_string(),
    };
    (
        StatusCode::TOO_MANY_REQUESTS,
        [("retry-after", secs.to_string())],
        Json(body),
    )
        .into_response()
}

/// `POST /api/v1/auth/login` — 401 on unknown user, bad password or disabled;
/// 429 once this client (real socket peer + username) OR this username globally
/// has failed too many times in the window (S5). The username-only tally is the
/// part that survives IP / forwarding-header rotation.
///
/// We extract `ConnectInfo<SocketAddr>` (wired up in ottod via
/// `into_make_service_with_connect_info`) for the real peer IP and deliberately
/// ignore `X-Forwarded-For` / `X-Real-IP`: no trusted proxy sits in front, so
/// honoring them would let an attacker rotate the header to dodge the lockout.
pub async fn login(
    State(ctx): State<ServerCtx>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<LoginReq>,
) -> Response {
    handle_login(&ctx, login_throttle::global(), Some(peer.ip()), req).await
}

/// Core login flow, parameterized over the attempt store and peer IP so it is
/// unit-testable (see `tests/auth_security.rs`). Checks BOTH the per-client and
/// the global-per-username keys, records a failure to both, and clears both on
/// success (the legitimate-user happy path).
async fn handle_login(
    ctx: &ServerCtx,
    attempts: &AttemptStore,
    peer: Option<IpAddr>,
    req: LoginReq,
) -> Response {
    let ip_key = login_throttle::ip_key(peer, &req.username);
    let user_key = login_throttle::username_key(&req.username);

    // Either key being locked rejects the attempt; report the longer wait.
    if let Some(retry_after) = attempts.max_locked(&[&ip_key, &user_key]) {
        return too_many_requests(retry_after);
    }

    let ip = peer.map(|p| p.to_string());
    match try_login(ctx, &req).await {
        Ok(resp) => {
            attempts.clear(&ip_key);
            attempts.clear(&user_key);
            // Audit the successful authentication (the acting user is now known).
            ctx.audit(NewAuditEntry {
                user_id: Some(resp.user.id.clone()),
                action: "login.success".into(),
                target: Some(req.username.clone()),
                detail: None,
                ip,
            })
            .await;
            Json(resp).into_response()
        }
        Err(ApiError(Error::Unauthorized)) => {
            attempts.record_failure(&ip_key);
            attempts.record_failure(&user_key);
            // Re-check so the attempt that *crosses* either threshold is itself
            // answered with the lockout, not a bare 401.
            let locked = attempts.max_locked(&[&ip_key, &user_key]);
            // No acting user on a failed login (the username may not even exist),
            // so user_id is None; the attempted username is the target.
            ctx.audit(NewAuditEntry {
                user_id: None,
                action: if locked.is_some() {
                    "login.lockout".into()
                } else {
                    "login.failure".into()
                },
                target: Some(req.username.clone()),
                detail: None,
                ip,
            })
            .await;
            if let Some(retry_after) = locked {
                too_many_requests(retry_after)
            } else {
                ApiError(Error::Unauthorized).into_response()
            }
        }
        Err(e) => e.into_response(),
    }
}

/// Credential check shared by `login`; returns `Error::Unauthorized` for unknown
/// user, bad password, or disabled account (so the caller can tally failures).
async fn try_login(ctx: &ServerCtx, req: &LoginReq) -> ApiResult<LoginResp> {
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
    Ok(LoginResp {
        token,
        user: record.user,
    })
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

/// `POST /api/v1/auth/tokens` — mint a long-lived API token for the caller.
/// The raw secret is returned exactly once (only its hash is stored). Use it as
/// `Authorization: Bearer <token>` on every route, or as `?token=<token>` on
/// the WebSocket endpoints.
pub async fn create_token(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateApiTokenReq>,
) -> ApiResult<Json<CreateApiTokenResp>> {
    let label = req
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (token, info) = AuthRepo::new(ctx.pool.clone())
        .issue_api_token(&user.id, label)
        .await?;
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "token.mint".into(),
        target: Some(info.id.clone()),
        detail: info
            .label
            .clone()
            .map(|l| serde_json::json!({ "label": l })),
        ip: None,
    })
    .await;
    Ok(Json(CreateApiTokenResp { token, info }))
}

/// `GET /api/v1/auth/tokens` — list the caller's API tokens (never the secret).
pub async fn list_tokens(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiTokenInfo>>> {
    let tokens = AuthRepo::new(ctx.pool.clone())
        .list_api_tokens(&user.id)
        .await?;
    Ok(Json(tokens))
}

/// `DELETE /api/v1/auth/tokens/{id}` — revoke one of the caller's API tokens.
pub async fn revoke_token(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let deleted = AuthRepo::new(ctx.pool.clone())
        .revoke_api_token(&user.id, &id)
        .await?;
    if deleted {
        ctx.audit(NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: "token.revoke".into(),
            target: Some(id.clone()),
            detail: None,
            ip: None,
        })
        .await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(Error::NotFound("api token".into()).into())
    }
}
