//! Share-link mint / list / revoke endpoints — mobile plan Task 1.9.
//!
//! ## Endpoints
//! - `POST /api/v1/sessions/{id}/share`    — mint a scoped share token (owner).
//! - `GET  /api/v1/sessions/{id}/shares`   — list live shares for a session (owner).
//! - `DELETE /api/v1/auth/shares/{share_id}` — revoke one share by id (owner).
//! - `POST /api/v1/auth/shares/revoke-all` — revoke all the caller's shares (owner).
//! - `GET  /api/v1/share/whoami`           — a share guest's own session + role.
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
//! `url = format!("{origin}/#/s/{session_id}/{token}")` where `origin` is the
//! operator-configured public domain (`share_base_url` in settings) when set,
//! else derived from the `Host` request header (defaults to a relative
//! `/#/s/{session_id}/{token}` when unavailable). The same `origin` is used to
//! build the link emailed alongside an OTP code. When neither the configured
//! domain nor the Host is reachable from another device (the desktop app always
//! calls 127.0.0.1) and the network listener is actually serving (the port
//! `ottod` bound — never the saved setting, which applies only after a restart
//! and may have failed TLS setup), the LAN listener address
//! (`https://<lan-ip>:<port>`) is used instead. `reach` classifies the final
//! origin — `remote` / `lan` (private or link-local: same Wi-Fi only, behind a
//! self-signed certificate) / `local` (loopback) — and `reachable_remotely` is
//! `reach != local`. An emailed OTP share is refused with 409 unless its origin
//! is `remote` or a Public link domain is configured: a recipient is almost
//! never on the owner's network (S20-303).
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
    CreateShareReq, CreateShareResp, ExtendShareReq, ListSharesResp, ShareReach, VerifyShareReq,
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
    /// Send the 6-digit `otp` to `to`, including the `share_url` (the ready-to-open
    /// link) when it is non-empty so the recipient can open the session straight
    /// from the email. Errors surface to the share-mint caller (so a broken sender
    /// fails the mint loudly rather than minting a share no one can redeem).
    fn send_otp<'a>(
        &'a self,
        to: &'a str,
        otp: &'a str,
        share_url: &'a str,
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
        share_url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move {
            let body = if share_url.is_empty() {
                // No link available (e.g. an extend re-send) — code-only body.
                format!(
                    "Your Otto access code is: {otp}\n\n\
                     Enter it on the share link to view the session. \
                     The code expires in 10 minutes. If you didn't expect this, ignore this email."
                )
            } else {
                format!(
                    "You've been invited to view an Otto session.\n\n\
                     Open: {share_url}\n\n\
                     Access code: {otp}\n\n\
                     The code expires in 10 minutes. If you didn't expect this, ignore this email."
                )
            };
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
    let scheme =
        if host.starts_with("127.") || host.starts_with("localhost") || host.starts_with("[::1]") {
            "http"
        } else {
            "https"
        };

    format!("{scheme}://{host}")
}

/// The origin a share link is built on, plus whether a device OTHER than this
/// Mac can open it (S20-01: a loopback link + a "scan on your phone" QR is a
/// dead end, so the UI warns instead and the OTP path refuses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareOrigin {
    pub origin: String,
    pub reachable_remotely: bool,
    /// Who can open the origin (S20-303).
    pub reach: ShareReach,
    /// The origin is the operator-configured Public link domain.
    pub configured: bool,
}

/// Host part of an origin (`scheme://host[:port]` or a bare `host[:port]`),
/// without the port; IPv6 brackets are kept off. Empty for an empty origin.
fn origin_host(origin: &str) -> &str {
    let rest = origin.split_once("://").map_or(origin, |(_, r)| r);
    let authority = rest.split(['/', '#', '?']).next().unwrap_or_default();
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split(']').next().unwrap_or_default();
    }
    authority.split(':').next().unwrap_or_default()
}

/// True when another device could resolve `origin` to this Mac: non-empty and
/// not loopback / unspecified / `localhost`.
pub fn origin_is_remote(origin: &str) -> bool {
    let host = origin_host(origin).trim().to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") {
        return false;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => !(ip.is_loopback() || ip.is_unspecified()),
        Err(_) => true,
    }
}

/// Classify who can open `origin`: `local` (loopback/empty), `lan` (an RFC
/// 1918 / link-local / IPv6 ULA address or an mDNS `.local` name — only
/// devices on this Mac's network) or `remote`.
pub fn origin_reach(origin: &str) -> ShareReach {
    if !origin_is_remote(origin) {
        return ShareReach::Local;
    }
    let host = origin_host(origin).trim().to_ascii_lowercase();
    if host.ends_with(".local") {
        return ShareReach::Lan;
    }
    let private = match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_private() || v4.is_link_local(),
        Ok(std::net::IpAddr::V6(v6)) => {
            let first = v6.segments()[0];
            // fe80::/10 link-local, fc00::/7 unique-local.
            (first & 0xffc0) == 0xfe80 || (first & 0xfe00) == 0xfc00
        }
        Err(_) => false,
    };
    if private {
        ShareReach::Lan
    } else {
        ShareReach::Remote
    }
}

/// Pick the share origin: the configured public domain, else a remote request
/// Host, else the LAN listener's address, else the (loopback/empty) Host.
pub fn resolve_share_origin(
    configured: Option<String>,
    host_origin: String,
    lan_origin: Option<String>,
) -> ShareOrigin {
    let configured = configured
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());
    let is_configured = configured.is_some();
    let origin = configured
        .or_else(|| origin_is_remote(&host_origin).then(|| host_origin.clone()))
        .or(lan_origin)
        .unwrap_or(host_origin);
    let reach = origin_reach(&origin);
    ShareOrigin {
        reachable_remotely: reach != ShareReach::Local,
        reach,
        configured: is_configured,
        origin,
    }
}

/// Whether a share link on `o` may be EMAILED: the recipient is almost never on
/// the owner's network, so only a routable origin — or one the operator
/// configured as the Public link domain — qualifies (S20-303).
pub fn emailable(o: &ShareOrigin) -> bool {
    o.reach == ShareReach::Remote || (o.configured && o.reach != ShareReach::Local)
}

/// `https://<lan-ip>:<port>` of the network (TLS, 0.0.0.0) listener when it is
/// ACTUALLY serving — `bound_port` is what `ottod` bound
/// ([`crate::transport::network_listener_port`]), not the saved setting, which
/// only applies after a restart and is ignored when TLS setup or the bind
/// failed (S20-303) — and this Mac has a non-loopback primary address.
fn lan_listener_origin(bound_port: Option<u16>, ip: Option<std::net::IpAddr>) -> Option<String> {
    let port = bound_port?;
    let ip = ip?;
    Some(match ip {
        std::net::IpAddr::V4(v4) => format!("https://{v4}:{port}"),
        std::net::IpAddr::V6(v6) => format!("https://[{v6}]:{port}"),
    })
}

/// The address the OS would route outbound traffic from. A UDP `connect` only
/// selects a route — no packet is sent.
fn primary_lan_ip() -> Option<std::net::IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?; // TEST-NET-1: never contacted
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
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

    // Guard 1b (S1-02): a share token carries no session binding, so an
    // agent's credential minting one could attach to a sibling terminal (and,
    // through a tunnel, hand out a remote shell) with no person involved.
    crate::auth::require_human(&auth.0)?;

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
    let role = WorkspaceRole::parse(&req.role)
        .ok_or_else(|| ApiError(Error::Invalid(format!("unknown role '{}'", req.role))))?;
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

    // Resolve the base origin ONCE: prefer the operator-configured public domain
    // (`share_base_url` in settings — so links work remotely, not just at the
    // request Host which is 127.0.0.1 for the desktop app / a tunneled daemon),
    // then a non-loopback request Host, then the LAN TLS listener's address when
    // the network listener is on, and only then the (loopback) Host itself.
    let settings = otto_state::SettingsRepo::new(ctx.pool.clone());
    let configured = settings
        .get("share_base_url")
        .await?
        .and_then(|v| v.as_str().map(str::to_string));
    let host_origin = origin_from_headers(&headers);
    let lan_origin = match crate::transport::network_listener_port() {
        // The caller already reached us remotely — no need to probe.
        Some(_) if origin_is_remote(&host_origin) => None,
        Some(port) => lan_listener_origin(Some(port), primary_lan_ip()),
        None => None,
    };
    let resolved = resolve_share_origin(configured, host_origin, lan_origin);

    // An emailed OTP share is useless when its link points at loopback or a
    // private LAN address: the recipient would get a dead link and a code they
    // cannot redeem. Refuse BEFORE minting (nothing to revoke) with a fixable
    // message.
    if recipient.is_some() && !emailable(&resolved) {
        let why = if resolved.reach == ShareReach::Lan {
            "this link only works on this Mac's Wi-Fi network — set a Public link domain \
             (Settings → Sharing) before emailing a share"
        } else {
            "this link would only work on this Mac — set a Public link domain \
             (Settings → Sharing) before emailing a share"
        };
        return Err(ApiError(Error::Conflict(why.into())));
    }
    let ShareOrigin {
        origin,
        reachable_remotely,
        reach,
        ..
    } = resolved;

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
            &origin,
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

    // Build the share URL from the resolved origin (configured domain or Host).
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

    Ok(Json(CreateShareResp {
        token,
        url,
        info,
        reachable_remotely,
        reach,
    }))
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
        &ctx.secrets,
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
    secrets: &std::sync::Arc<dyn otto_core::secrets::SecretStore>,
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
    let app_password = otto_core::secrets::get_async(secrets, &sender.secret_ref)
        .await?
        .ok_or_else(|| {
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
    share_origin: &str,
) -> ApiResult<(String, otto_core::api::ShareInfo)> {
    let (token, otp, info) = repo
        .issue_share_otp_token(
            owner_id,
            session_id,
            role,
            duration_secs,
            label,
            recipient_email,
        )
        .await?;

    // The ready-to-open link, emailed alongside the code so the guest can open the
    // session directly (empty `share_origin` ⇒ a relative URL, handled by the body).
    let share_url = format!("{share_origin}/#/s/{session_id}/{token}");

    if let Err(e) = mailer.send_otp(recipient_email, &otp, &share_url).await {
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
    let (otp, recipient, owner_id) = repo.extend_share_otp(token).await?.ok_or_else(|| {
        ApiError(Error::Invalid(
            "this share is not extendable (only email-OTP shares can be extended)".into(),
        ))
    })?;
    // Defensive: the row's owner must be the owner we resolved the sender for, so
    // we never email via a different user's sender than the share belongs to.
    debug_assert_eq!(&owner_id, expected_owner);

    // Email the fresh code to the STORED recipient — never an address from the
    // request (there is none). No share_url here (the guest already has the link
    // from the first email) → code-only body.
    mailer.send_otp(&recipient, &otp, "").await.map_err(|e| {
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
    client: Option<axum::Extension<otto_sessions::share_throttle::ClientIp>>,
    Json(req): Json<ExtendShareReq>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Tunnel-aware client IP (host guard) — the raw peer is 127.0.0.1 for
    // every visitor behind the Cloudflare tunnel (S8-02).
    let ip = client.map_or(peer.ip(), |c| c.0.ip);

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

    // 1b. Resolve the share BEFORE spending anything on it (S8-306): junk
    //     tokens must not consume (or saturate) the per-share budget. `None` ⇒
    //     not an OTP share ⇒ 400 + a throttle failure (so this can't be used to
    //     probe which tokens are extendable). A share past its lifetime (403)
    //     or verified and still open (409, S8-307) is refused here too.
    let share_id = match repo.extendable_share_id(&req.token).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            otto_sessions::share_throttle::global().record_failure(ip);
            return ApiError(Error::Invalid(
                "this share is not extendable (only email-OTP shares can be extended)".into(),
            ))
            .into_response();
        }
        Err(e) => return ApiError(e).into_response(),
    };

    // 1c. Per-share extend budget (S8-06), keyed on the real share id: every
    //     extend de-verifies the guest and emails the recipient from the
    //     owner's sender, so a link holder must not be able to do it at will.
    if !extend_budget_ok(&share_id) {
        let body = otto_core::api::Problem {
            code: "too_many_requests".to_string(),
            message: format!(
                "this share was extended {EXTEND_MAX_PER_WINDOW} times in the last hour; \
                 try again later"
            ),
        };
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", EXTEND_WINDOW.as_secs().to_string())],
            Json(body),
        )
            .into_response();
    }

    // 2. Re-issue the OTP (re-pends the share + fresh ≤12h window). The recipient
    //    and owner come from the DB row — NEVER from the request. `None` ⇒ the
    //    share changed under us (revoked) ⇒ 400.
    let (otp, recipient, owner_id) = match repo.extend_share_otp(&req.token).await {
        Ok(Some(v)) => v,
        Ok(None) => {
            extend_budget_refund(&share_id);
            return ApiError(Error::Invalid(
                "this share is not extendable (only email-OTP shares can be extended)".into(),
            ))
            .into_response();
        }
        Err(e) => {
            extend_budget_refund(&share_id);
            return ApiError(e).into_response();
        }
    };

    // 3. Resolve the SHARE OWNER's verified sender and email the fresh code to the
    //    LOCKED `recipient` (read from the row above). 400 when the owner no longer
    //    has a verified sender.
    let mailer = match gmail_mailer_for(&ctx, &owner_id).await {
        Ok(m) => m,
        Err(e) => {
            extend_budget_refund(&share_id);
            return e.into_response();
        }
    };
    // No share_url on extend (the guest already has the link) → code-only body.
    // A failed send gives the slot back: the guest never got a code.
    if let Err(e) = mailer.send_otp(&recipient, &otp, "").await {
        extend_budget_refund(&share_id);
        return ApiError(Error::Upstream(format!(
            "failed to re-email the access code to {recipient}: {e}"
        )))
        .into_response();
    }

    // Success: audit the extension. The IP's failure tally is deliberately NOT
    // cleared (S8-06) — a successful extend says nothing about the caller's
    // earlier failed guesses.
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

/// Extends one share may take per [`EXTEND_WINDOW`] (S8-06).
const EXTEND_MAX_PER_WINDOW: usize = 3;
/// Sliding window for [`EXTEND_MAX_PER_WINDOW`].
const EXTEND_WINDOW: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Live keys the extend budget tracks before evicting the stalest one.
const EXTEND_BUDGET_MAX_KEYS: usize = 10_000;

type ExtendSlots = std::collections::HashMap<Id, Vec<std::time::Instant>>;

fn extend_slots() -> std::sync::MutexGuard<'static, ExtendSlots> {
    static SLOTS: std::sync::OnceLock<std::sync::Mutex<ExtendSlots>> = std::sync::OnceLock::new();
    SLOTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Take one slot of the per-share extend budget, keyed on the SHARE ID — only
/// called once the token resolved to a real, extendable OTP share (S8-306),
/// so junk tokens never reach it. In-memory and per-process like the IP
/// throttle. A full map evicts the entry whose newest slot is oldest instead
/// of refusing every share (only real shares get here, so it can't be
/// flooded with junk).
fn extend_budget_ok(share_id: &Id) -> bool {
    let mut map = extend_slots();
    let now = std::time::Instant::now();
    map.retain(|_, v| {
        v.retain(|t| now.duration_since(*t) < EXTEND_WINDOW);
        !v.is_empty()
    });
    if map.len() >= EXTEND_BUDGET_MAX_KEYS && !map.contains_key(share_id) {
        let stalest = map
            .iter()
            .min_by_key(|(_, v)| v.last().copied())
            .map(|(k, _)| k.clone());
        if let Some(k) = stalest {
            map.remove(&k);
        }
    }
    let slots = map.entry(share_id.clone()).or_default();
    if slots.len() >= EXTEND_MAX_PER_WINDOW {
        return false;
    }
    slots.push(now);
    true
}

/// Give back the slot [`extend_budget_ok`] just took (the extend failed before
/// a code reached the guest — e.g. the mail send failed).
fn extend_budget_refund(share_id: &Id) {
    if let Some(v) = extend_slots().get_mut(share_id) {
        v.pop();
    }
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
    client: Option<axum::Extension<otto_sessions::share_throttle::ClientIp>>,
    Json(req): Json<VerifyShareReq>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Tunnel-aware client IP (host guard) — the raw peer is 127.0.0.1 for
    // every visitor behind the Cloudflare tunnel (S8-02).
    let ip = client.map_or(peer.ip(), |c| c.0.ip);

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

    // 2. Verify the code (single-use; clears otp_hash on success). Every miss
    //    also counts on the share itself and burns the code after
    //    `SHARE_OTP_MAX_FAILURES` (S8-301) — the hard bound IP rotation can't
    //    dodge. argon2 runs off the async workers behind a shared bound.
    use otto_rbac::tokens::ShareOtpOutcome;
    let outcome = match AuthRepo::new(ctx.pool.clone())
        .verify_share_otp_outcome(&req.token, &req.otp)
        .await
    {
        Ok(v) => v,
        Err(e) => return ApiError(e).into_response(),
    };

    if outcome == ShareOtpOutcome::Busy {
        let body = otto_core::api::Problem {
            code: "busy".to_string(),
            message: "too many sign-in checks in flight; try again in a moment".to_string(),
        };
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [("retry-after", "1".to_string())],
            Json(body),
        )
            .into_response();
    }

    if outcome != ShareOtpOutcome::Verified {
        // Wrong / expired / already-used code → record a failure and reject 401.
        otto_sessions::share_throttle::global().record_failure(ip);
        let burned = outcome == ShareOtpOutcome::Burned;
        ctx.audit(NewAuditEntry {
            user_id: None,
            action: if burned {
                "share.verify.burned".into()
            } else {
                "share.verify.fail".into()
            },
            target: None,
            detail: None,
            ip: Some(ip.to_string()),
        })
        .await;
        if burned {
            let body = otto_core::api::Problem {
                code: "otp_burned".to_string(),
                message: "too many wrong codes — this code no longer works; request a new one"
                    .to_string(),
            };
            return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
        }
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

/// One row of `GET /api/v1/auth/shares`: a live link plus its session's
/// title (None when the session is gone), so the table reads as sessions.
#[derive(Debug, serde::Serialize)]
pub struct MyShare {
    #[serde(flatten)]
    pub share: otto_core::api::ShareInfo,
    pub session_title: Option<String>,
}

/// `GET /api/v1/auth/shares`
///
/// List ALL of the caller's live (non-revoked, non-expired) share links across
/// sessions, newest first — Settings → Sharing's "Active links". Self-owned
/// (Exempt in policy, like `/auth/tokens`): only the caller's own links.
pub async fn list_my_shares(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<MyShare>>> {
    let shares = AuthRepo::new(ctx.pool.clone())
        .list_shares_for_user(&user.id)
        .await?;
    let mut out = Vec::with_capacity(shares.len());
    for share in shares {
        let session_title = ctx
            .manager
            .get(&share.session_id)
            .await
            .ok()
            .map(|s| s.title);
        out.push(MyShare {
            share,
            session_title,
        });
    }
    Ok(Json(out))
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
        detail: session_id_opt
            .as_ref()
            .map(|sid| serde_json::json!({ "session_id": sid })),
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

/// `GET /api/v1/share/whoami` — what a share token is: its pinned session and
/// capped role (`viewer` | `editor`). The guest page (`#/s/{id}/{token}`) reads
/// it to decide whether the terminal takes input; before this it had to assume
/// read-only, so an Editor link could never type. Only a scoped (share) token
/// may call it — the feature guard admits it for any verified share scope — and
/// anything else gets 400 (a normal bearer has no share role to report).
pub async fn share_whoami(
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<serde_json::Value>> {
    let Some(scope) = auth.scope.as_ref() else {
        return Err(ApiError(Error::Invalid("not a share token".into())));
    };
    Ok(Json(serde_json::json!({
        "session_id": scope.session_id,
        "role": scope.role.as_str(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S8-06: a share may be extended at most `EXTEND_MAX_PER_WINDOW` times
    /// per window; other shares keep their own budget.
    #[test]
    fn extend_budget_caps_each_share_independently() {
        let a = format!("budget-test-a-{}", otto_core::new_id());
        let b = format!("budget-test-b-{}", otto_core::new_id());
        for _ in 0..EXTEND_MAX_PER_WINDOW {
            assert!(extend_budget_ok(&a));
        }
        assert!(!extend_budget_ok(&a), "the next extend must be refused");
        assert!(extend_budget_ok(&b), "another share is unaffected");
    }

    #[test]
    fn origin_is_remote_rejects_loopback_and_empty() {
        for o in [
            "",
            "http://127.0.0.1:7700",
            "http://localhost:5173",
            "http://[::1]:7700",
            "https://0.0.0.0:7701",
            "http://app.localhost",
        ] {
            assert!(!origin_is_remote(o), "{o} must not count as remote");
        }
        for o in [
            "https://otto.example.com",
            "https://192.168.1.20:7700",
            "https://[fe80::1]:7700",
            "otto.example.com",
        ] {
            assert!(origin_is_remote(o), "{o} must count as remote");
        }
    }

    #[test]
    fn configured_domain_wins_and_is_trimmed() {
        let o = resolve_share_origin(
            Some(" https://otto.example.com/ ".into()),
            "http://127.0.0.1:7700".into(),
            Some("https://192.168.1.20:7700".into()),
        );
        assert_eq!(o.origin, "https://otto.example.com");
        assert!(o.reachable_remotely);
    }

    #[test]
    fn loopback_host_falls_back_to_lan_listener() {
        let o = resolve_share_origin(
            Some("   ".into()),
            "http://127.0.0.1:7700".into(),
            Some("https://192.168.1.20:7700".into()),
        );
        assert_eq!(o.origin, "https://192.168.1.20:7700");
        assert!(o.reachable_remotely);
    }

    #[test]
    fn loopback_host_without_listener_is_flagged_unreachable() {
        let o = resolve_share_origin(None, "http://127.0.0.1:7700".into(), None);
        assert_eq!(o.origin, "http://127.0.0.1:7700");
        assert!(!o.reachable_remotely);
        let empty = resolve_share_origin(None, String::new(), None);
        assert!(!empty.reachable_remotely);
    }

    #[test]
    fn remote_host_is_kept_over_lan_listener() {
        let o = resolve_share_origin(
            None,
            "https://192.168.1.30:7700".into(),
            Some("https://10.0.0.2:7700".into()),
        );
        assert_eq!(o.origin, "https://192.168.1.30:7700");
        assert!(o.reachable_remotely);
    }

    #[test]
    fn lan_listener_origin_requires_a_bound_listener() {
        let ip: std::net::IpAddr = "192.168.1.20".parse().unwrap();
        // Setting on but nothing bound (before the restart / failed TLS): no
        // LAN origin, so the link stays honest about being local (S20-303).
        assert_eq!(lan_listener_origin(None, Some(ip)), None);
        assert_eq!(lan_listener_origin(Some(7700), None), None);
        assert_eq!(
            lan_listener_origin(Some(7701), Some(ip)).as_deref(),
            Some("https://192.168.1.20:7701")
        );
        let v6: std::net::IpAddr = "fe80::1".parse().unwrap();
        assert_eq!(
            lan_listener_origin(Some(7700), Some(v6)).as_deref(),
            Some("https://[fe80::1]:7700")
        );
    }

    #[test]
    fn origin_reach_separates_lan_from_remote() {
        for o in [
            "http://127.0.0.1:7700",
            "",
            "http://localhost:7700",
            "http://[::1]:7700",
        ] {
            assert_eq!(origin_reach(o), ShareReach::Local, "{o}");
        }
        for o in [
            "https://192.168.1.20:7700",
            "https://10.0.0.2:7700",
            "https://172.16.4.1:7700",
            "https://169.254.3.3:7700",
            "https://[fe80::1]:7700",
            "https://[fd12:3456::1]:7700",
            "https://my-mac.local:7700",
        ] {
            assert_eq!(origin_reach(o), ShareReach::Lan, "{o}");
        }
        for o in [
            "https://otto.example.com",
            "https://203.0.113.9:7700",
            "https://[2001:db8::1]",
        ] {
            assert_eq!(origin_reach(o), ShareReach::Remote, "{o}");
        }
    }

    #[test]
    fn emailed_shares_refuse_lan_unless_a_domain_is_configured() {
        // The LAN listener fallback: phone-on-Wi-Fi only → not emailable.
        let lan = resolve_share_origin(
            None,
            "http://127.0.0.1:7700".into(),
            Some("https://192.168.1.20:7700".into()),
        );
        assert_eq!(lan.reach, ShareReach::Lan);
        assert!(lan.reachable_remotely);
        assert!(!emailable(&lan));
        // Loopback: never.
        let local = resolve_share_origin(None, "http://127.0.0.1:7700".into(), None);
        assert!(!emailable(&local));
        // A public domain: yes.
        let public = resolve_share_origin(
            Some("https://otto.example.com".into()),
            "http://127.0.0.1:7700".into(),
            None,
        );
        assert_eq!(public.reach, ShareReach::Remote);
        assert!(emailable(&public));
        // An operator-configured private domain is their explicit choice.
        let configured_lan = resolve_share_origin(
            Some("https://192.168.1.20:7700".into()),
            "http://127.0.0.1:7700".into(),
            None,
        );
        assert_eq!(configured_lan.reach, ShareReach::Lan);
        assert!(emailable(&configured_lan));
    }
}
