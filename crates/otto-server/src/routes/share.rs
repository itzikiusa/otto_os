//! Share-link mint / list / revoke endpoints — mobile plan Task 1.9.
//!
//! ## Endpoints
//! - `POST /api/v1/sessions/{id}/share`    — mint a scoped share token (owner).
//! - `GET  /api/v1/sessions/{id}/shares`   — list live shares for a session (owner).
//! - `DELETE /api/v1/auth/shares/{share_id}` — revoke one share by id (owner).
//! - `POST /api/v1/auth/shares/revoke-all` — revoke all the caller's shares (owner).
//!
//! ## Guards (mint)
//! The caller must:
//! 1. Own the session (or be a workspace Admin): `require_session_owner_or_admin`.
//! 2. NOT be impersonated (`real != effective`) — an impersonation overlay must
//!    not be able to forge a long-lived capability on behalf of the true owner.
//! 3. NOT hold a scoped (share) token — a guest cannot mint sub-shares.
//!
//! Both checks mirror the PAT-mint guard in `auth_routes.rs:188-192`.
//!
//! ## URL construction
//! `url = format!("{origin}/#/s/{session_id}/{token}")` where `origin` is
//! derived from the `Host` request header (defaults to a relative
//! `/#/s/{session_id}/{token}` when unavailable).
//!
//! ## Eviction on revoke
//! After revoking a share, `SessionManager::evict(&session_id)` is called so
//! any still-attached viewer receives a `{"type":"terminated"}` frame and the
//! WS closes immediately (Task 4.1 eviction signal).
//!
//! ## Audit
//! `share.mint` and `share.revoke` entries are written via `ctx.audit`.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use otto_core::api::{CreateShareReq, CreateShareResp, ListSharesResp};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_rbac::{
    tokens::{SHARE_TOKEN_TTL_MAX_SECS, SHARE_TOKEN_TTL_MIN_SECS},
    AuthRepo,
};
use otto_state::NewAuditEntry;

use crate::auth::{require_session_owner_or_admin, CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Default TTL (seconds) for a share link when the caller omits `ttl_secs`.
const SHARE_DEFAULT_TTL_SECS: i64 = 3600;

/// Derive the base origin (`scheme://host`) from the request's `Host` header so
/// the returned `url` points back to the caller's actual domain. Falls back to
/// an empty string (yielding a relative URL `/#/s/…`) when the header is absent
/// or malformed — this is defensive and will be fixed automatically once the
/// SPA constructs the link client-side.
fn origin_from_headers(headers: &HeaderMap) -> String {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    if host.is_empty() {
        return String::new();
    }

    // If the host already looks like a full URL (e.g. a forwarded `X-Forwarded-Proto`
    // is unavailable here) default to https as the safe assumption for a publicly-
    // exposed tunnel. For loopback (local dev / testing) use http.
    let scheme = if host.starts_with("127.") || host.starts_with("localhost") || host.starts_with("[::1]") {
        "http"
    } else {
        "https"
    };

    format!("{scheme}://{host}")
}

/// `POST /api/v1/sessions/{id}/share`
///
/// Mint a scoped share-link token bound to the session. The raw token is
/// returned exactly once; the `url` field is the ready-to-share fragment URL
/// (`<origin>/#/s/<session_id>/<token>`).
pub async fn mint_share(
    State(ctx): State<ServerCtx>,
    Path(session_id): Path<Id>,
    auth: CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Json(req): Json<CreateShareReq>,
) -> ApiResult<Json<CreateShareResp>> {
    // Guard 1: block impersonated requests (real != effective user).
    // An impersonation overlay must not mint capabilities on behalf of the true owner.
    if auth.real_user().id != auth.effective_user().id {
        return Err(ApiError(Error::Forbidden(
            "an impersonated session cannot mint share links".into(),
        )));
    }

    // Guard 2: block scoped (share) tokens from minting sub-shares.
    if auth.0.scope.is_some() {
        return Err(ApiError(Error::Forbidden(
            "a share token cannot mint further share links".into(),
        )));
    }

    // Load the session (404 when absent) and enforce ownership.
    let session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    require_session_owner_or_admin(&ctx, &user, &session).await?;

    // Parse the requested role (reject "admin").
    let role = WorkspaceRole::parse(&req.role).ok_or_else(|| {
        ApiError(Error::Invalid(format!("unknown role '{}'", req.role)))
    })?;
    if role == WorkspaceRole::Admin {
        return Err(ApiError(Error::Forbidden(
            "a share link cannot grant Admin role".into(),
        )));
    }

    // Clamp TTL to [MIN, MAX], default 3600.
    let ttl_secs = req
        .ttl_secs
        .unwrap_or(SHARE_DEFAULT_TTL_SECS)
        .clamp(SHARE_TOKEN_TTL_MIN_SECS, SHARE_TOKEN_TTL_MAX_SECS);

    let label = req
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let (token, info) = AuthRepo::new(ctx.pool.clone())
        .issue_share_token(&user.id, &session_id, role, ttl_secs, label)
        .await?;

    // Build the share URL from the request's Host header.
    let origin = origin_from_headers(&headers);
    let url = format!("{origin}/#/s/{session_id}/{token}");

    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "share.mint".into(),
        target: Some(session_id.clone()),
        detail: Some(serde_json::json!({
            "share_id": info.id,
            "role": req.role,
            "ttl_secs": ttl_secs,
        })),
        ip: None,
    })
    .await;

    Ok(Json(CreateShareResp { token, url, info }))
}

/// `GET /api/v1/sessions/{id}/shares`
///
/// List all live (non-revoked, non-expired) share tokens for the session.
/// The caller must own the session or be a workspace admin.
pub async fn list_shares(
    State(ctx): State<ServerCtx>,
    Path(session_id): Path<Id>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ListSharesResp>> {
    let session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    require_session_owner_or_admin(&ctx, &user, &session).await?;

    let shares = AuthRepo::new(ctx.pool.clone())
        .list_shares_for_session(&session_id)
        .await?;

    Ok(Json(ListSharesResp { shares }))
}

/// `DELETE /api/v1/auth/shares/{share_id}`
///
/// Revoke one of the caller's share tokens by id. After revocation, calls
/// `SessionManager::evict` on the share's session so any attached viewer is
/// dropped immediately. Returns 204.
pub async fn revoke_share(
    State(ctx): State<ServerCtx>,
    Path(share_id): Path<String>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let repo = AuthRepo::new(ctx.pool.clone());

    // Look up the session this share is pinned to (for eviction after revoke).
    let session_id_opt = repo.share_session_id(&user.id, &share_id).await?;

    // Revoke the share (owner-scoped; idempotent).
    repo.revoke_share(&user.id, &share_id).await?;

    // Evict attached viewers for the share's session (if we found one).
    if let Some(session_id) = &session_id_opt {
        ctx.manager.evict(session_id);
    }

    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "share.revoke".into(),
        target: Some(share_id.clone()),
        detail: session_id_opt.as_ref().map(|sid| {
            serde_json::json!({ "session_id": sid })
        }),
        ip: None,
    })
    .await;

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/auth/shares/revoke-all`
///
/// Revoke ALL of the caller's live share tokens and evict any attached viewers
/// for those sessions. Returns 204.
pub async fn revoke_all_shares(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let repo = AuthRepo::new(ctx.pool.clone());

    // Revoke all shares and collect the session ids to evict.
    let session_ids = repo.revoke_all_shares_for_user(&user.id).await?;

    // Evict viewers for every affected session.
    for session_id in &session_ids {
        ctx.manager.evict(session_id);
    }

    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "share.revoke".into(),
        target: None,
        detail: Some(serde_json::json!({
            "scope": "all",
            "session_count": session_ids.len(),
        })),
        ip: None,
    })
    .await;

    Ok(StatusCode::NO_CONTENT)
}
