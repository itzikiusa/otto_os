//! `/phone/*` REST routes (contract: docs/contracts/api.md → *Phone access*).
//!
//! Who may call what:
//!
//! * **Owner routes** (status, devices) — a person's own credential: not an
//!   agent session / MCP / share token, not a paired phone's device token, not
//!   an impersonation overlay.
//! * **On-the-Mac routes** (serve on/off, create / read / approve / deny a
//!   pairing) — the owner rule AND, when the daemon can see the peer, a
//!   request from this Mac itself (`PeerKind::Local`): a password login from
//!   a phone cannot approve phones. Serve on/off additionally needs root.
//! * **Phone routes** (`/phone/pair/claim`, `/phone/pair/poll`) — public, but
//!   only from a tailnet peer (`PeerKind::Tailnet`); rate-limited per peer.
//!   The code travels in the POST body, so it never lands in a URL log.
//! * `/phone/device/me` — any credential: "am I a paired phone?" / unpair me.
//!
//! Audit: `phone.serve.{on,off}`, `phone.pair.{create,claim,refused,approve,deny}`,
//! `phone.device.{paired,rename,revoke,revoke_all,unpair}`. Codes, secrets and
//! tokens are never written to the audit log or tracing.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::{root_authority, AuthContext};
use otto_core::domain::User;
use otto_core::peer::{PeerKind, RequestPeer};
use otto_core::phone::{
    validate_next_route, PairClaimResult, PairPollResult, PairingCreated, PairingView,
    PhoneAccessStatus, PhoneDevice,
};
use otto_core::{Error, Id};
use otto_rbac::devices::{DevicesRepo, PollOutcome};
use otto_state::audit::NewAuditEntry;
use serde::Deserialize;
use serde_json::json;

use crate::throttle::Throttle;
use crate::{tailscale, PhoneCtx};

/// Error → JSON `Problem`, plus a 429 for the throttle.
pub enum PhoneErr {
    Api(Error),
    RateLimited,
}

impl From<Error> for PhoneErr {
    fn from(e: Error) -> Self {
        PhoneErr::Api(e)
    }
}

impl IntoResponse for PhoneErr {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            PhoneErr::RateLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited".to_string(),
                "too many attempts — wait a few minutes".to_string(),
            ),
            PhoneErr::Api(e) => {
                let status = match &e {
                    Error::NotFound(_) => StatusCode::NOT_FOUND,
                    Error::Unauthorized => StatusCode::UNAUTHORIZED,
                    Error::Forbidden(_) => StatusCode::FORBIDDEN,
                    Error::Conflict(_) => StatusCode::CONFLICT,
                    Error::Invalid(_) => StatusCode::BAD_REQUEST,
                    Error::Upstream(_) => StatusCode::BAD_GATEWAY,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                (status, e.code().to_string(), e.to_string())
            }
        };
        (status, Json(Problem { code, message })).into_response()
    }
}

type PResult<T> = Result<T, PhoneErr>;

/// Owner routes: `/phone/status`, `/phone/serve`, pairings, devices, device/me.
pub fn protected_router<S: PhoneCtx>() -> Router<S> {
    Router::new()
        .route("/phone/status", get(status::<S>))
        .route("/phone/serve", post(serve::<S>))
        .route("/phone/pairings", post(create_pairing::<S>))
        .route("/phone/pairings/{id}", get(get_pairing::<S>))
        .route("/phone/pairings/{id}/approve", post(approve::<S>))
        .route("/phone/pairings/{id}/deny", post(deny::<S>))
        .route("/phone/devices", get(list_devices::<S>))
        .route(
            "/phone/devices/{id}",
            patch(rename_device::<S>).delete(revoke_device::<S>),
        )
        .route("/phone/devices/revoke-all", post(revoke_all::<S>))
        .route(
            "/phone/device/me",
            get(device_me::<S>).delete(unpair_me::<S>),
        )
}

/// Phone-side public routes (no bearer): claim + poll.
pub fn public_router<S: PhoneCtx>() -> Router<S> {
    Router::new()
        .route("/phone/pair/claim", post(claim::<S>))
        .route("/phone/pair/poll", post(poll::<S>))
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

/// A person's own, non-phone, non-impersonated credential.
fn require_owner<'a, S: PhoneCtx>(ctx: &S, auth: &'a AuthContext) -> PResult<&'a User> {
    if auth.device_id.is_some() {
        return Err(Error::Forbidden(
            "a paired phone cannot manage Phone access — use Otto on your Mac".into(),
        )
        .into());
    }
    if !ctx.is_human(auth) {
        return Err(Error::Forbidden(
            "an agent, MCP or share credential cannot manage Phone access — \
             a person signed in to Otto must do this"
                .into(),
        )
        .into());
    }
    if auth.real_user.id != auth.effective_user.id {
        return Err(
            Error::Forbidden("Phone access is not available while impersonating".into()).into(),
        );
    }
    Ok(&auth.effective_user)
}

/// The owner rule, on this Mac (when the peer is known).
fn require_on_mac<'a, S: PhoneCtx>(ctx: &S, auth: &'a AuthContext) -> PResult<&'a User> {
    let user = require_owner(ctx, auth)?;
    if otto_core::auth::request_peer().is_some_and(|p| p.kind != PeerKind::Local) {
        return Err(Error::Forbidden(
            "pairing phones and changing Phone access is done in Otto on your Mac".into(),
        )
        .into());
    }
    Ok(user)
}

fn on_mac() -> bool {
    otto_core::auth::request_peer().is_none_or(|p| p.kind == PeerKind::Local)
}

/// A public phone route's peer: the tailnet only (fail closed when unknown).
fn tailnet_peer() -> PResult<RequestPeer> {
    match otto_core::auth::request_peer() {
        Some(p) if p.kind == PeerKind::Tailnet => {
            if Throttle::global().allow(p.ip) {
                Ok(p)
            } else {
                Err(PhoneErr::RateLimited)
            }
        }
        _ => Err(Error::Forbidden(
            "pairing only works from a device on your tailnet (open the link over Tailscale)"
                .into(),
        )
        .into()),
    }
}

fn entry(
    user: Option<&Id>,
    action: &str,
    target: Option<&str>,
    detail: serde_json::Value,
) -> NewAuditEntry {
    NewAuditEntry {
        user_id: user.cloned(),
        action: action.into(),
        target: target.map(str::to_string),
        detail: Some(detail),
        ip: otto_core::auth::request_peer().map(|p| p.ip.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Owner / on-the-Mac handlers
// ---------------------------------------------------------------------------

async fn status<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
) -> PResult<Json<PhoneAccessStatus>> {
    let user = require_owner(&ctx, &auth)?;
    let mut st = tailscale::status(ctx.tailscale().as_ref()).await;
    st.can_manage = root_authority(user) && on_mac();
    Ok(Json(st))
}

#[derive(Deserialize)]
struct ServeReq {
    enabled: bool,
}

async fn serve<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
    Json(req): Json<ServeReq>,
) -> PResult<Json<PhoneAccessStatus>> {
    let user = require_on_mac(&ctx, &auth)?;
    if !root_authority(user) {
        return Err(Error::Forbidden("turning Phone access on or off requires root".into()).into());
    }
    let cli = ctx.tailscale();
    let res = if req.enabled {
        tailscale::enable(cli.as_ref()).await
    } else {
        tailscale::disable(cli.as_ref()).await
    };
    let action = if req.enabled {
        "phone.serve.on"
    } else {
        "phone.serve.off"
    };
    match res {
        Ok(mut st) => {
            let url = st.serving.as_ref().map(|s| s.url.clone());
            ctx.audit(entry(
                Some(&user.id),
                action,
                url.as_deref(),
                json!({ "ok": true }),
            ))
            .await;
            st.can_manage = true;
            Ok(Json(st))
        }
        Err(e) => {
            ctx.audit(entry(
                Some(&user.id),
                action,
                None,
                json!({ "ok": false, "error": e }),
            ))
            .await;
            Err(Error::Upstream(e).into())
        }
    }
}

#[derive(Deserialize, Default)]
struct CreatePairingReq {
    #[serde(default)]
    next: Option<String>,
}

async fn create_pairing<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
    body: Option<Json<CreatePairingReq>>,
) -> PResult<Json<PairingCreated>> {
    let user = require_on_mac(&ctx, &auth)?;
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let next = match req.next.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        None => None,
        Some(n) => Some(
            validate_next_route(n)
                .ok_or_else(|| Error::Invalid("next must be a session or thread route".into()))?,
        ),
    };
    let st = tailscale::status(ctx.tailscale().as_ref()).await;
    let serving = st.serving.ok_or_else(|| {
        Error::Conflict("Phone access is off — turn it on in Settings → Phone access".into())
    })?;
    let repo = DevicesRepo::new(ctx.pool());
    let (id, code, expires_at) = repo.create_pairing(&user.id, next.as_deref()).await?;
    let mut url = format!("{}/#/pair/{code}", serving.url.trim_end_matches('/'));
    if let Some(n) = next.as_deref().filter(|n| !n.is_empty()) {
        url.push('/');
        url.push_str(n);
    }
    ctx.audit(entry(
        Some(&user.id),
        "phone.pair.create",
        Some(&id),
        json!({ "next": next }),
    ))
    .await;
    Ok(Json(PairingCreated {
        id,
        code,
        url,
        expires_at,
    }))
}

async fn get_pairing<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> PResult<Json<PairingView>> {
    let user = require_on_mac(&ctx, &auth)?;
    Ok(Json(
        DevicesRepo::new(ctx.pool()).pairing(&user.id, &id).await?,
    ))
}

async fn decide<S: PhoneCtx>(
    ctx: S,
    auth: AuthContext,
    id: Id,
    approve: bool,
) -> PResult<Json<PairingView>> {
    let user = require_on_mac(&ctx, &auth)?;
    let view = DevicesRepo::new(ctx.pool())
        .decide(&user.id, &id, approve)
        .await?;
    let action = if approve {
        "phone.pair.approve"
    } else {
        "phone.pair.deny"
    };
    ctx.audit(entry(
        Some(&user.id),
        action,
        Some(&id),
        json!({ "device_label": view.device_label, "peer_ip": view.peer_ip }),
    ))
    .await;
    Ok(Json(view))
}

async fn approve<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> PResult<Json<PairingView>> {
    decide(ctx, auth, id, true).await
}

async fn deny<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> PResult<Json<PairingView>> {
    decide(ctx, auth, id, false).await
}

async fn list_devices<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
) -> PResult<Json<Vec<PhoneDevice>>> {
    let user = require_owner(&ctx, &auth)?;
    Ok(Json(
        DevicesRepo::new(ctx.pool()).list(&user.id, None).await?,
    ))
}

#[derive(Deserialize)]
struct RenameReq {
    name: String,
}

async fn rename_device<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
    Json(req): Json<RenameReq>,
) -> PResult<Json<PhoneDevice>> {
    let user = require_owner(&ctx, &auth)?;
    let d = DevicesRepo::new(ctx.pool())
        .rename(&user.id, &id, &req.name)
        .await?;
    ctx.audit(entry(
        Some(&user.id),
        "phone.device.rename",
        Some(&id),
        json!({ "name": d.name }),
    ))
    .await;
    Ok(Json(d))
}

async fn revoke_device<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> PResult<StatusCode> {
    let user = require_owner(&ctx, &auth)?;
    if !DevicesRepo::new(ctx.pool()).revoke(&user.id, &id).await? {
        return Err(Error::NotFound("device not found".into()).into());
    }
    ctx.audit(entry(
        Some(&user.id),
        "phone.device.revoke",
        Some(&id),
        json!({}),
    ))
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_all<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
) -> PResult<Json<serde_json::Value>> {
    let user = require_owner(&ctx, &auth)?;
    let n = DevicesRepo::new(ctx.pool()).revoke_all(&user.id).await?;
    ctx.audit(entry(
        Some(&user.id),
        "phone.device.revoke_all",
        None,
        json!({ "revoked": n }),
    ))
    .await;
    Ok(Json(json!({ "revoked": n })))
}

async fn device_me<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
) -> PResult<Json<serde_json::Value>> {
    let device = match &auth.device_id {
        Some(id) => {
            DevicesRepo::new(ctx.pool())
                .device(&auth.real_user.id, id, Some(id))
                .await?
        }
        None => None,
    };
    Ok(Json(json!({ "device": device })))
}

async fn unpair_me<S: PhoneCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthContext>,
) -> PResult<StatusCode> {
    let id = auth
        .device_id
        .clone()
        .ok_or_else(|| Error::Invalid("this credential is not a paired phone".into()))?;
    DevicesRepo::new(ctx.pool())
        .revoke(&auth.real_user.id, &id)
        .await?;
    ctx.audit(entry(
        Some(&auth.real_user.id),
        "phone.device.unpair",
        Some(&id),
        json!({}),
    ))
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Public phone handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ClaimReq {
    code: String,
}

async fn claim<S: PhoneCtx>(
    State(ctx): State<S>,
    headers: HeaderMap,
    Json(req): Json<ClaimReq>,
) -> PResult<Json<PairClaimResult>> {
    let peer = tailnet_peer()?;
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(400).collect::<String>());
    let label = crate::ua::device_label(ua.as_deref().unwrap_or(""));
    let code = req.code.trim();
    let well_formed = code.len() == 64 && code.bytes().all(|b| b.is_ascii_hexdigit());
    let res = if well_formed {
        DevicesRepo::new(ctx.pool())
            .claim(code, &label, ua.as_deref(), &peer.ip.to_string())
            .await
    } else {
        Err(Error::NotFound("invalid or expired pairing code".into()))
    };
    match res {
        Ok(c) => {
            ctx.audit(entry(
                None,
                "phone.pair.claim",
                Some(&c.pairing_id),
                json!({ "device_label": label }),
            ))
            .await;
            Ok(Json(PairClaimResult {
                pairing_id: c.pairing_id,
                poll_secret: c.poll_secret,
                confirm_code: c.confirm_code,
                expires_at: c.expires_at,
            }))
        }
        Err(e @ Error::NotFound(_)) => {
            Throttle::global().fail(peer.ip);
            ctx.audit(entry(
                None,
                "phone.pair.refused",
                None,
                json!({ "device_label": label }),
            ))
            .await;
            Err(e.into())
        }
        Err(e) => Err(e.into()),
    }
}

#[derive(Deserialize)]
struct PollReq {
    pairing_id: Id,
    poll_secret: String,
}

async fn poll<S: PhoneCtx>(
    State(ctx): State<S>,
    Json(req): Json<PollReq>,
) -> PResult<Json<PairPollResult>> {
    let peer = tailnet_peer()?;
    let res = DevicesRepo::new(ctx.pool())
        .poll(&req.pairing_id, &req.poll_secret)
        .await;
    let out = match res {
        Err(e @ Error::NotFound(_)) => {
            Throttle::global().fail(peer.ip);
            return Err(e.into());
        }
        Err(e) => return Err(e.into()),
        Ok(o) => o,
    };
    let simple = |s: &str| PairPollResult {
        status: s.into(),
        token: None,
        next: None,
        device: None,
    };
    Ok(Json(match out {
        PollOutcome::Pending => simple("pending"),
        PollOutcome::Denied => simple("denied"),
        PollOutcome::Expired => simple("expired"),
        PollOutcome::Approved {
            token,
            next,
            device,
        } => {
            ctx.audit(entry(
                None,
                "phone.device.paired",
                Some(&device.id),
                json!({ "pairing_id": req.pairing_id, "name": device.name }),
            ))
            .await;
            PairPollResult {
                status: "approved".into(),
                token: Some(token),
                next,
                device: Some(*device),
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tailscale::fake::FakeCli;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use otto_core::auth::BoxFuture;
    use otto_state::DbPool;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[derive(Clone)]
    struct Ctx {
        pool: DbPool,
        cli: Arc<FakeCli>,
    }

    impl PhoneCtx for Ctx {
        fn pool(&self) -> DbPool {
            self.pool.clone()
        }
        fn tailscale(&self) -> Arc<dyn tailscale::TailscaleCli> {
            self.cli.clone()
        }
        fn is_human(&self, auth: &AuthContext) -> bool {
            auth.managed_session_id.is_none() && !auth.mcp_only && auth.scope.is_none()
        }
        fn audit(&self, entry: NewAuditEntry) -> BoxFuture<'_, ()> {
            Box::pin(async move {
                // Codes / secrets / tokens must never reach the audit log.
                let text = serde_json::to_string(&entry.detail).unwrap();
                assert!(
                    !text.contains("poll_secret") && !text.contains("token\""),
                    "{text}"
                );
                let _ = otto_state::AuditRepo::new(self.pool.clone())
                    .insert(entry)
                    .await;
            })
        }
    }

    const SERVED: &str = r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"mac.tail1.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:7700"}}}}}"#;

    async fn setup(serve: &str) -> (Ctx, otto_core::domain::User) {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let user = otto_core::domain::User {
            id: otto_core::new_id(),
            username: "owner".into(),
            display_name: "Owner".into(),
            is_root: true,
            disabled: false,
            created_at: chrono::Utc::now(),
        };
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, 'owner', 'h', 'Owner', 1, ?)")
            .bind(&user.id)
            .bind(user.created_at.to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        // Keep the daemon port the fake answers for stable.
        let ctx = Ctx {
            pool: pool.into(),
            cli: Arc::new(FakeCli::new(
                &serve.replace("7700", &tailscale::daemon_port().to_string()),
            )),
        };
        (ctx, user)
    }

    fn peer(kind: PeerKind, ip: &str) -> RequestPeer {
        RequestPeer {
            ip: ip.parse().unwrap(),
            kind,
        }
    }

    /// Run one request through the owner router as `auth`, from `p`.
    async fn call(
        ctx: &Ctx,
        auth: Option<AuthContext>,
        p: Option<RequestPeer>,
        method: &str,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let app = protected_router::<Ctx>()
            .merge(public_router::<Ctx>())
            .with_state(ctx.clone());
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header(
                "user-agent",
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1) Version/18.1 Mobile Safari/604.1",
            )
            .body(Body::from(body.to_string()))
            .unwrap();
        if let Some(a) = auth {
            req.extensions_mut().insert(a);
        }
        let fut = app.oneshot(req);
        let resp = match p {
            Some(p) => otto_core::auth::with_request_peer(p, fut).await,
            None => fut.await,
        }
        .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    fn person(u: &otto_core::domain::User) -> AuthContext {
        AuthContext::automation(u, "test")
    }

    fn local() -> Option<RequestPeer> {
        Some(peer(PeerKind::Local, "127.0.0.1"))
    }

    #[tokio::test]
    async fn full_pairing_flow_lands_on_next() {
        let (ctx, u) = setup(SERVED).await;
        let (s, created) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "POST",
            "/phone/pairings",
            json!({"next": "threads/T1"}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{created}");
        let code = created["code"].as_str().unwrap().to_string();
        assert_eq!(
            created["url"].as_str().unwrap(),
            format!("https://mac.tail1.ts.net/#/pair/{code}/threads/T1")
        );
        let id = created["id"].as_str().unwrap().to_string();
        let phone = Some(peer(PeerKind::Tailnet, "100.111.7.125"));
        let (s, claimed) = call(
            &ctx,
            None,
            phone.clone(),
            "POST",
            "/phone/pair/claim",
            json!({"code": code}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{claimed}");
        // The Mac sees the same confirmation code + the phone's label/ip.
        let (_, view) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "GET",
            &format!("/phone/pairings/{id}"),
            json!({}),
        )
        .await;
        assert_eq!(view["status"], "claimed");
        assert_eq!(view["confirm_code"], claimed["confirm_code"]);
        assert_eq!(view["device_label"], "iPhone · Safari");
        assert_eq!(view["peer_ip"], "100.111.7.125");
        let poll_body =
            json!({"pairing_id": claimed["pairing_id"], "poll_secret": claimed["poll_secret"]});
        let (_, p) = call(
            &ctx,
            None,
            phone.clone(),
            "POST",
            "/phone/pair/poll",
            poll_body.clone(),
        )
        .await;
        assert_eq!(p["status"], "pending");
        let (s, _) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "POST",
            &format!("/phone/pairings/{id}/approve"),
            json!({}),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        let (_, p) = call(
            &ctx,
            None,
            phone.clone(),
            "POST",
            "/phone/pair/poll",
            poll_body.clone(),
        )
        .await;
        assert_eq!(p["status"], "approved");
        assert_eq!(p["next"], "threads/T1");
        let token = p["token"].as_str().unwrap();
        assert_eq!(token.len(), 64);
        // The device token signs the phone in.
        let dev = otto_rbac::AuthRepo::new(ctx.pool.clone());
        let dctx =
            otto_core::auth::with_request_peer(phone.clone().unwrap(), dev.authenticate(token))
                .await
                .unwrap();
        assert!(dctx.device_id.is_some());
        // device/me for the phone; null for the Mac.
        let (_, me) = call(
            &ctx,
            Some(dctx.clone()),
            phone.clone(),
            "GET",
            "/phone/device/me",
            json!({}),
        )
        .await;
        assert_eq!(me["device"]["current"], true);
        let (_, me) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "GET",
            "/phone/device/me",
            json!({}),
        )
        .await;
        assert!(me["device"].is_null());
        // A replayed poll gets no second token.
        let (_, p) = call(&ctx, None, phone, "POST", "/phone/pair/poll", poll_body).await;
        assert!(p.get("token").is_none());
    }

    #[tokio::test]
    async fn phone_cannot_approve_create_or_manage() {
        let (ctx, u) = setup(SERVED).await;
        let mut dev = person(&u);
        dev.device_id = Some("D1".into());
        let phone = Some(peer(PeerKind::Tailnet, "100.111.7.125"));
        for (m, uri) in [
            ("POST", "/phone/pairings"),
            ("POST", "/phone/pairings/X/approve"),
            ("POST", "/phone/serve"),
            ("GET", "/phone/devices"),
            ("GET", "/phone/status"),
            ("POST", "/phone/devices/revoke-all"),
        ] {
            let (s, _) = call(
                &ctx,
                Some(dev.clone()),
                phone.clone(),
                m,
                uri,
                json!({"enabled": true}),
            )
            .await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{m} {uri}");
        }
        // A password login from the phone (human, not a device) still cannot
        // create or approve pairings: that happens on the Mac.
        let (s, _) = call(
            &ctx,
            Some(person(&u)),
            phone,
            "POST",
            "/phone/pairings",
            json!({}),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn agent_share_and_mcp_credentials_cannot_create_or_approve() {
        let (ctx, u) = setup(SERVED).await;
        let mut agent = person(&u);
        agent.managed_session_id = Some("S1".into());
        let mut mcp = person(&u);
        mcp.mcp_only = true;
        let mut share = person(&u);
        share.scope = Some(otto_core::auth::SessionScope {
            session_id: "S1".into(),
            role: otto_core::domain::WorkspaceRole::Editor,
            otp_pending: false,
        });
        let mut imp = person(&u);
        imp.real_user.id = "admin".into();
        for a in [agent, mcp, share, imp] {
            let (s, _) = call(
                &ctx,
                Some(a.clone()),
                local(),
                "POST",
                "/phone/pairings",
                json!({}),
            )
            .await;
            assert_eq!(s, StatusCode::FORBIDDEN);
            let (s, _) = call(
                &ctx,
                Some(a),
                local(),
                "POST",
                "/phone/pairings/X/approve",
                json!({}),
            )
            .await;
            assert_eq!(s, StatusCode::FORBIDDEN);
        }
    }

    #[tokio::test]
    async fn non_tailnet_peers_cannot_claim() {
        let (ctx, u) = setup(SERVED).await;
        let (_, created) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "POST",
            "/phone/pairings",
            json!({}),
        )
        .await;
        let code = created["code"].as_str().unwrap();
        for p in [
            None,
            Some(peer(PeerKind::Other, "192.168.1.5")),
            Some(peer(PeerKind::Other, "203.0.113.9")),
            Some(peer(PeerKind::Local, "127.0.0.1")),
        ] {
            let (s, _) = call(
                &ctx,
                None,
                p,
                "POST",
                "/phone/pair/claim",
                json!({"code": code}),
            )
            .await;
            assert_eq!(s, StatusCode::FORBIDDEN);
        }
        // The code is still claimable from the tailnet afterwards.
        let (s, _) = call(
            &ctx,
            None,
            Some(peer(PeerKind::Tailnet, "100.64.0.7")),
            "POST",
            "/phone/pair/claim",
            json!({"code": code}),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn pairing_needs_serving_and_a_valid_next() {
        let (ctx, u) = setup("{}").await;
        let (s, _) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "POST",
            "/phone/pairings",
            json!({}),
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT);
        let (ctx, u) = setup(SERVED).await;
        let (s, _) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "POST",
            "/phone/pairings",
            json!({"next": "settings/users"}),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "POST",
            "/phone/pairings",
            json!({"next": "https://evil.example"}),
        )
        .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn bad_codes_are_throttled() {
        let (ctx, _) = setup(SERVED).await;
        let p = Some(peer(PeerKind::Tailnet, "100.99.99.99"));
        for _ in 0..crate::throttle::CLAIM_FAILURES {
            let (s, _) = call(
                &ctx,
                None,
                p.clone(),
                "POST",
                "/phone/pair/claim",
                json!({"code": "ab"}),
            )
            .await;
            assert_eq!(s, StatusCode::NOT_FOUND);
        }
        let (s, _) = call(
            &ctx,
            None,
            p,
            "POST",
            "/phone/pair/claim",
            json!({"code": "ab"}),
        )
        .await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn serve_toggle_is_root_on_mac_only() {
        let (ctx, mut u) = setup("{}").await;
        u.is_root = false;
        let (s, _) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "POST",
            "/phone/serve",
            json!({"enabled": true}),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        u.is_root = true;
        let (s, st) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "POST",
            "/phone/serve",
            json!({"enabled": true}),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{st}");
        assert_eq!(st["serving"]["https"], true);
        let (s, st) = call(
            &ctx,
            Some(person(&u)),
            local(),
            "POST",
            "/phone/serve",
            json!({"enabled": false}),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        assert!(st["serving"].is_null());
    }
}
