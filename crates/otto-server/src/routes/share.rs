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

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use otto_channels::GmailSender;
use otto_core::api::{
    CreateShareReq, CreateShareResp, ExtendShareReq, ListSharesResp, VerifyShareReq,
    VerifyShareResp,
};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_rbac::{
    tokens::{SHARE_TOKEN_TTL_MAX_SECS, SHARE_TOKEN_TTL_MIN_SECS},
    AuthRepo,
};
use otto_state::{EmailSendersRepo, NewAuditEntry};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use crate::auth::{require_session_owner_or_admin, CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Default TTL (seconds) for a share link when the caller omits `ttl_secs`.
const SHARE_DEFAULT_TTL_SECS: i64 = 3600;
/// Default OTP-share session window (seconds) when the caller omits
/// `duration_secs`: 1h (clamped server-side to ≤12h).
const SHARE_OTP_DEFAULT_WINDOW_SECS: i64 = 3600;

/// Subject line of the OTP email.
const OTP_EMAIL_SUBJECT: &str = "Your Otto access code";

/// An injectable one-time-code mailer. The production path emails via the
/// owner's verified Gmail App Password sender ([`GmailMailer`]); unit tests pass
/// a capturing implementation so the OTP can be asserted WITHOUT real SMTP.
///
/// Boxed-future (not `async_trait`) to stay dependency-light and object-safe.
pub trait OtpMailer: Send + Sync {
    /// Send the 6-digit `otp` to `to`. Errors surface to the share-mint caller
    /// (so a broken sender fails the mint loudly rather than minting a share no
    /// one can redeem).
    fn send_otp<'a>(
        &'a self,
        to: &'a str,
        otp: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;
}

/// The production mailer: a configured [`GmailSender`] (the owner's address +
/// the app password read from the Keychain by the route). Sends the OTP body.
struct GmailMailer(GmailSender);

impl OtpMailer for GmailMailer {
    fn send_otp<'a>(
        &'a self,
        to: &'a str,
        otp: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let body = format!(
                "Your Otto access code is: {otp}\n\n\
                 Enter it on the share link to view the session. \
                 The code expires in 10 minutes. If you didn't expect this, ignore this email."
            );
            self.0.send(to, OTP_EMAIL_SUBJECT, &body).await
        })
    }
}

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

    let label = req
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    // Email-OTP branch (mobile plan Task 7.2): when a recipient_email is given,
    // mint an OTP-gated share and email the code via the owner's verified sender.
    let recipient = req
        .recipient_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let (token, info) = if let Some(recipient) = recipient {
        // Resolve the owner's verified Gmail sender → build the production mailer.
        let mailer = gmail_mailer_for(&ctx, &user.id).await?;
        let duration_secs = req.duration_secs.unwrap_or(SHARE_OTP_DEFAULT_WINDOW_SECS);
        mint_otp_share(
            &AuthRepo::new(ctx.pool.clone()),
            &user.id,
            &session_id,
            role,
            duration_secs,
            label,
            recipient,
            &mailer,
        )
        .await?
    } else {
        // Plain scoped share (no OTP gate) — backward compatible.
        let ttl_secs = req
            .ttl_secs
            .unwrap_or(SHARE_DEFAULT_TTL_SECS)
            .clamp(SHARE_TOKEN_TTL_MIN_SECS, SHARE_TOKEN_TTL_MAX_SECS);
        AuthRepo::new(ctx.pool.clone())
            .issue_share_token(&user.id, &session_id, role, ttl_secs, label)
            .await?
    };

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
            "otp_gated": recipient.is_some(),
        })),
        ip: None,
    })
    .await;

    Ok(Json(CreateShareResp { token, url, info }))
}

/// Resolve the owner's **verified** email sender into a production [`OtpMailer`].
///
/// Returns a clear `400` when the owner has no sender, an unverified sender, or
/// the Keychain has no app password for it — so the share-mint route tells the
/// user to "set up an email sender first" instead of silently minting a share
/// whose code can never be delivered.
async fn gmail_mailer_for(ctx: &ServerCtx, owner_id: &str) -> ApiResult<GmailMailer> {
    let (gmail_address, app_password) = resolve_verified_sender(
        &EmailSendersRepo::new(ctx.pool.clone()),
        ctx.secrets.as_ref(),
        owner_id,
    )
    .await?;
    Ok(GmailMailer(GmailSender::new(gmail_address, app_password)))
}

/// Resolve the owner's **verified** Gmail sender → `(address, app_password)`,
/// reading the password from the Keychain via `secrets`. Returns a clear `400`
/// when there is no verified sender or the secret is missing. Factored out of
/// the route so the no-sender guard is unit-testable without a full `ServerCtx`.
pub async fn resolve_verified_sender(
    senders: &EmailSendersRepo,
    secrets: &dyn otto_core::secrets::SecretStore,
    owner_id: &str,
) -> ApiResult<(String, String)> {
    let sender = senders
        .get(owner_id)
        .await?
        .filter(|s| s.verified_at.is_some())
        .ok_or_else(|| {
            ApiError(Error::Invalid(
                "set up a verified email sender first (Settings → Sharing) before creating an OTP share"
                    .into(),
            ))
        })?;
    let app_password = secrets.get(&sender.secret_ref)?.ok_or_else(|| {
        ApiError(Error::Invalid(
            "email sender app password is missing — re-configure your email sender".into(),
        ))
    })?;
    Ok((sender.gmail_address, app_password))
}

/// Mint an OTP-gated share AND deliver the code via `mailer` (mobile plan Task
/// 7.2). Factored out of the route so a unit test can inject a capturing
/// [`OtpMailer`] and assert the OTP WITHOUT touching real SMTP. The share row is
/// written first; if delivery fails the share is revoked so a code that was never
/// emailed can't leave a dangling OTP-pending share behind.
#[allow(clippy::too_many_arguments)]
pub async fn mint_otp_share(
    repo: &AuthRepo,
    owner_id: &Id,
    session_id: &Id,
    role: WorkspaceRole,
    duration_secs: i64,
    label: Option<String>,
    recipient_email: &str,
    mailer: &dyn OtpMailer,
) -> ApiResult<(String, otto_core::api::ShareInfo)> {
    let (token, otp, info) = repo
        .issue_share_otp_token(owner_id, session_id, role, duration_secs, label, recipient_email)
        .await?;

    if let Err(e) = mailer.send_otp(recipient_email, &otp).await {
        // Delivery failed — revoke the just-minted share so we don't leave an
        // OTP-pending capability whose code nobody received.
        let _ = repo.revoke_share(owner_id, &info.id).await;
        return Err(ApiError(Error::Upstream(format!(
            "failed to email the access code to {recipient_email}: {e}"
        ))));
    }

    Ok((token, info))
}

/// Re-issue a FRESH OTP for an existing OTP share AND deliver it to the LOCKED
/// original recipient (mobile plan Task 7.4 / `POST /api/v1/share/extend`).
///
/// The destination is read from the share row's immutable `recipient_email` —
/// **never from the request** (the request body carries no email). This is the
/// locked-recipient guarantee: there is no parameter by which a caller can
/// redirect the code to another mailbox. Factored out of the route so a unit test
/// can inject a capturing [`OtpMailer`] and assert the new code lands on the
/// ORIGINAL address WITHOUT real SMTP.
///
/// `repo.extend_share_otp` re-pends the share (clears `verified_at`), stores the
/// fresh `otp_hash` (~10-min expiry) and a fresh ≤12h window; this helper then
/// emails the new code to the row's recipient. A non-OTP / missing / revoked
/// share yields `400` (it is not extendable).
pub async fn extend_otp_share(
    repo: &AuthRepo,
    expected_owner: &Id,
    token: &str,
    mailer: &dyn OtpMailer,
) -> ApiResult<()> {
    // The repo reads the destination from the row — the caller cannot influence
    // WHERE the code goes. `None` ⇒ not an extendable OTP share ⇒ 400.
    let (otp, recipient, owner_id) = repo
        .extend_share_otp(token)
        .await?
        .ok_or_else(|| {
            ApiError(Error::Invalid(
                "this share is not extendable (only email-OTP shares can be extended)".into(),
            ))
        })?;
    // Defensive: the row's owner must be the owner we resolved the sender for, so
    // we never email via a different user's sender than the share belongs to.
    debug_assert_eq!(&owner_id, expected_owner);

    // Email the fresh code to the STORED recipient — never an address from the
    // request (there is none).
    mailer.send_otp(&recipient, &otp).await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "failed to re-email the access code to {recipient}: {e}"
        )))
    })?;
    Ok(())
}

/// `POST /api/v1/share/extend` — re-issue a FRESH OTP for an existing OTP share,
/// emailed to the **LOCKED original recipient ONLY** (mobile plan Task 7.4).
/// **Public / Exempt**: the `token` (the share link) is the auth, so this route
/// is reachable even after the share's window has elapsed (the share is then
/// OTP-pending). IP rate-limited via the share throttle.
///
/// Flow: IP rate-limit → load the share by the token's hash; it MUST be a
/// `kind='share'` row WITH a `recipient_email` (only OTP shares are extendable) →
/// generate a fresh OTP, clear `verified_at` (forces re-verification), set a fresh
/// ≤12h window → resolve the OWNER's verified sender and **email the code to the
/// STORED `recipient_email` ONLY** (the request body has no email field; the
/// destination is read from the DB row, never the request). Returns `{ ok: true }`;
/// the guest then re-verifies via `POST /api/v1/share/verify`.
pub async fn extend_share(
    State(ctx): State<ServerCtx>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<ExtendShareReq>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let ip = peer.ip();

    // 1. IP rate-limit BEFORE doing any work → 429 with Retry-After.
    if let Err(locked) = otto_sessions::share_throttle::global().check(ip) {
        let secs = locked.retry_after.as_secs().max(1);
        let body = otto_core::api::Problem {
            code: "too_many_requests".to_string(),
            message: "too many share-extend attempts; try again later".to_string(),
        };
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", secs.to_string())],
            Json(body),
        )
            .into_response();
    }

    let repo = AuthRepo::new(ctx.pool.clone());

    // 2. Re-issue the OTP (re-pends the share + fresh ≤12h window). The recipient
    //    and owner come from the DB row — NEVER from the request. `None` ⇒ not an
    //    extendable OTP share ⇒ 400.
    let (otp, recipient, owner_id) = match repo.extend_share_otp(&req.token).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            // Record a throttle failure so this can't be brute-forced to probe
            // which tokens are extendable OTP shares.
            otto_sessions::share_throttle::global().record_failure(ip);
            return ApiError(Error::Invalid(
                "this share is not extendable (only email-OTP shares can be extended)".into(),
            ))
            .into_response();
        }
        Err(e) => return ApiError(e).into_response(),
    };

    // 3. Resolve the SHARE OWNER's verified sender and email the fresh code to the
    //    LOCKED `recipient` (read from the row above). 400 when the owner no longer
    //    has a verified sender.
    let mailer = match gmail_mailer_for(&ctx, &owner_id).await {
        Ok(m) => m,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = mailer.send_otp(&recipient, &otp).await {
        return ApiError(Error::Upstream(format!(
            "failed to re-email the access code to {recipient}: {e}"
        )))
        .into_response();
    }

    // Success: clear the IP's failure tally and audit the extension.
    otto_sessions::share_throttle::global().clear(ip);
    ctx.audit(NewAuditEntry {
        user_id: Some(owner_id.clone()),
        action: "share.extend".into(),
        target: None,
        detail: None,
        ip: Some(ip.to_string()),
    })
    .await;

    Json(serde_json::json!({ "ok": true })).into_response()
}

/// `POST /api/v1/share/verify` — redeem an emailed OTP for a share token
/// (mobile plan Task 7.3). **Public / Exempt**: the `token` (the share link) is
/// the auth, so this route is reachable even while the share is OTP-pending.
///
/// Flow: IP rate-limit (the share throttle) → verify `otp_hash == sha256(otp)`
/// AND `otp_expires_at > now` → set `verified_at` and clear `otp_hash`
/// (single-use). A wrong/expired code records a throttle failure and returns
/// `401`. The peer IP is the real socket address (never a spoofable header).
pub async fn verify_share(
    State(ctx): State<ServerCtx>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<VerifyShareReq>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let ip = peer.ip();

    // 1. IP rate-limit BEFORE attempting verification → 429 with Retry-After.
    if let Err(locked) = otto_sessions::share_throttle::global().check(ip) {
        let secs = locked.retry_after.as_secs().max(1);
        let body = otto_core::api::Problem {
            code: "too_many_requests".to_string(),
            message: "too many failed code attempts; try again later".to_string(),
        };
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", secs.to_string())],
            Json(body),
        )
            .into_response();
    }

    // 2. Verify the code (single-use; clears otp_hash on success).
    let verified = match AuthRepo::new(ctx.pool.clone())
        .verify_share_otp(&req.token, &req.otp)
        .await
    {
        Ok(v) => v,
        Err(e) => return ApiError(e).into_response(),
    };

    if !verified {
        // Wrong / expired / already-used code → record a failure and reject 401.
        otto_sessions::share_throttle::global().record_failure(ip);
        ctx.audit(NewAuditEntry {
            user_id: None,
            action: "share.verify.fail".into(),
            target: None,
            detail: None,
            ip: Some(ip.to_string()),
        })
        .await;
        return ApiError(Error::Unauthorized).into_response();
    }

    // Success: clear the IP's failure tally and audit the redemption.
    otto_sessions::share_throttle::global().clear(ip);
    ctx.audit(NewAuditEntry {
        user_id: None,
        action: "share.verify".into(),
        target: None,
        detail: None,
        ip: Some(ip.to_string()),
    })
    .await;

    Json(VerifyShareResp { verified: true }).into_response()
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
