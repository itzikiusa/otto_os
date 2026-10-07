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
    ApiTokenInfo, CreateApiTokenReq, CreateApiTokenResp, LoginReq, LoginResp, MeResp, Problem,
};
use otto_core::{Error, Id};
use otto_rbac::AuthRepo;
use otto_state::{NewAuditEntry, UsersRepo};

use crate::auth::{BearerToken, CurrentAuthContext, CurrentUser};
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
/// The client IP is the host guard's tunnel-aware
/// [`otto_sessions::share_throttle::ClientIp`]: the socket peer, except behind
/// the documented Cloudflare tunnel (`docs/remote-access-runbook.md`), where
/// every client is `127.0.0.1` and `CF-Connecting-IP` — trusted ONLY for a
/// loopback peer that named the `share_base_url` host — identifies it (S8-07).
/// `X-Forwarded-For` / `X-Real-IP` are never honoured (an attacker would
/// rotate them to dodge the lockout).
///
/// The desktop app (a loopback peer addressing a loopback name) is exempt from
/// the GLOBAL per-username lock: otherwise anyone reaching the public hostname
/// could keep `root` locked out of its own Mac indefinitely. It still has its
/// own per-client key.
pub async fn login(
    State(ctx): State<ServerCtx>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    client: Option<Extension<otto_sessions::share_throttle::ClientIp>>,
    Json(req): Json<LoginReq>,
) -> Response {
    let (ip, local) = client.map_or((peer.ip(), false), |c| (c.0.ip, c.0.local));
    handle_login(&ctx, login_throttle::global(), Some(ip), local, req).await
}

/// Core login flow, parameterized over the attempt store and peer IP so it is
/// unit-testable (see `tests/auth_security.rs`). Checks BOTH the per-client and
/// the global-per-username keys, records a failure to both, and clears both on
/// success (the legitimate-user happy path).
async fn handle_login(
    ctx: &ServerCtx,
    attempts: &AttemptStore,
    peer: Option<IpAddr>,
    local: bool,
    req: LoginReq,
) -> Response {
    let ip_key = login_throttle::ip_key(peer, &req.username);
    let user_key = login_throttle::username_key(&req.username);
    // Username-independent per-client key (S8-303) — remote clients only: the
    // desktop shares 127.0.0.1 with every local tool and must not lock itself.
    let client_key = peer.filter(|_| !local).map(login_throttle::client_key);
    // The keys whose lock refuses this attempt: the desktop skips the global
    // username lock (see `login`), every other client is held to all three.
    let mut gating: Vec<&str> = vec![ip_key.as_str()];
    if !local {
        gating.push(user_key.as_str());
    }
    if let Some(k) = &client_key {
        gating.push(k.as_str());
    }

    // Either key being locked rejects the attempt; report the longer wait.
    if let Some(retry_after) = attempts.max_locked(&gating) {
        return too_many_requests(retry_after);
    }
    // Fail closed (S8-303): a flood filled the tally map and this remote
    // attempt's failures could not be counted — refuse it rather than let it
    // guess unthrottled. Never the desktop (a remote flood must not lock the
    // owner out of their own Mac).
    if !local && attempts.untracked_while_full(&gating) {
        return too_many_requests(login_throttle::SATURATED_RETRY);
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
        Err(LoginFailure::Busy) => {
            let body = Problem {
                code: "busy".to_string(),
                message: "too many sign-in checks in flight; try again in a moment".to_string(),
            };
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [("retry-after", "1".to_string())],
                Json(body),
            )
                .into_response()
        }
        Err(LoginFailure::Denied { known_user }) => {
            // A REAL account's keys are pinned: they are tracked even when a
            // junk-username flood has filled the map (a bounded set).
            if known_user {
                attempts.record_failure_pinned(&ip_key);
                attempts.record_failure_pinned(&user_key);
            } else {
                attempts.record_failure(&ip_key);
                attempts.record_failure(&user_key);
            }
            if let Some(k) = &client_key {
                attempts.record_failure(k);
            }
            // Re-check so the attempt that *crosses* either threshold is itself
            // answered with the lockout, not a bare 401.
            let locked = attempts.max_locked(&gating);
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
        Err(LoginFailure::Other(e)) => e.into_response(),
    }
}

/// Why [`try_login`] did not issue a token.
enum LoginFailure {
    /// Unknown user, bad password or disabled account (tallied as a failure).
    /// `known_user` = the username exists (its throttle keys are pinned).
    Denied {
        known_user: bool,
    },
    /// The argon2 verify bound is saturated (S8-303) — 503, not a failure.
    Busy,
    Other(ApiError),
}

impl From<Error> for LoginFailure {
    fn from(e: Error) -> Self {
        LoginFailure::Other(ApiError(e))
    }
}

/// Credential check shared by `login`; `Denied` for unknown user, bad password,
/// or disabled account (so the caller can tally failures). argon2 runs off the
/// async workers behind the shared verify bound (S8-303).
async fn try_login(ctx: &ServerCtx, req: &LoginReq) -> Result<LoginResp, LoginFailure> {
    let record = match UsersRepo::new(ctx.pool.clone())
        .get_by_username(&req.username)
        .await
    {
        Ok(record) => Some(record),
        Err(Error::NotFound(_)) => None,
        Err(e) => return Err(e.into()),
    };
    // Pay the same argon2 cost for an unknown user as for a known one
    // (S8-10): returning before the hash leaked which usernames exist.
    let hash = record
        .as_ref()
        .map_or(dummy_password_hash(), |r| r.password_hash.as_str());
    let password_ok = match otto_rbac::verify_password_bounded(&req.password, hash).await {
        Ok(Ok(ok)) => ok,
        // The dummy hash can't fail to parse in practice; never 500 on it.
        Ok(Err(_)) if record.is_none() => false,
        Ok(Err(e)) => return Err(e.into()),
        Err(otto_rbac::VerifySaturated) => return Err(LoginFailure::Busy),
    };
    let Some(record) = record else {
        return Err(LoginFailure::Denied { known_user: false });
    };
    // `disabled` is checked AFTER the verify, so a disabled account costs the
    // same as an enabled one.
    if record.user.disabled || !password_ok {
        return Err(LoginFailure::Denied { known_user: true });
    }

    let token = AuthRepo::new(ctx.pool.clone())
        .issue(&record.user.id)
        .await?;
    Ok(LoginResp {
        token,
        user: record.user,
    })
}

/// A real argon2 hash (same parameters as every stored password) that no
/// password matches, verified against for an unknown username (S8-10).
fn dummy_password_hash() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| otto_rbac::hash_password(&otto_core::new_id()).unwrap_or_default())
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
///
/// Returns both the effective and real identities so the UI can render the
/// impersonation banner correctly after a page reload. The `user` field is the
/// *effective* user (the identity authorisation runs against), preserving
/// backward compatibility for any caller that only reads `user`.
pub async fn me(auth: CurrentAuthContext) -> Json<MeResp> {
    let effective = auth.effective_user().clone();
    let real = auth.real_user().clone();
    let impersonating = real.id != effective.id;
    Json(MeResp {
        user: effective,
        real_user: real,
        impersonating,
    })
}

/// `POST /api/v1/auth/tokens` — mint a long-lived API token for the caller.
/// The raw secret is returned exactly once (only its hash is stored). Use it as
/// `Authorization: Bearer <token>` on every route, or as `?token=<token>` on
/// the WebSocket endpoints.
///
/// **Impersonation guardrail (Task 5.2):** an impersonated request (one whose
/// bearer is an impersonation token — `real_user != effective_user`) may NOT
/// mint a PAT. Otherwise an admin acting-as a user could forge a long-lived
/// credential *as that user*, escaping the short-TTL, fully-audited overlay.
/// → 403. (The same guard will later cover share-link minting.)
pub async fn create_token(
    State(ctx): State<ServerCtx>,
    auth: CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateApiTokenReq>,
) -> ApiResult<Json<CreateApiTokenResp>> {
    if auth.real_user().id != auth.effective_user().id {
        return Err(ApiError(Error::Forbidden(
            "an impersonated session cannot mint API tokens".into(),
        )));
    }
    // An agent session's own credential (or an MCP one) must not mint a
    // personal token: that token would carry no session binding and pass
    // every human-only check (UI-control results/grants, goal-loop
    // decisions) — an agent could launder itself into "the user".
    if !crate::ui_bridge::is_human(&auth.0) {
        return Err(ApiError(Error::Forbidden(
            "an agent session or MCP credential cannot mint API tokens".into(),
        )));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::login_throttle::{CLIENT_FAILURE_THRESHOLD, FAILURE_THRESHOLD};
    use crate::routes::browser::tests::{mem_pool, test_ctx};
    use axum::{body::Body, http::Request, routing::post, Router};
    use tower::ServiceExt;

    async fn fixture() -> (tempfile::TempDir, ServerCtx, String) {
        let tmp = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        let username = format!("quality-login-{}", otto_core::new_id());
        let hash = otto_rbac::hash_password("correct fixture password").unwrap();
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES(?,?,?,'Login fixture',0,?)")
            .bind(otto_core::new_id()).bind(&username).bind(hash)
            .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
        let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
        (tmp, ctx, username)
    }

    fn credentials(username: &str, correct: bool) -> LoginReq {
        LoginReq {
            username: username.into(),
            password: if correct {
                "correct fixture password"
            } else {
                "wrong"
            }
            .into(),
        }
    }

    async fn assert_locked(response: Response) {
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry: u64 = response.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=login_throttle::LOCKOUT_DURATION.as_secs()).contains(&retry));
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(problem["code"], "too_many_requests");
    }

    #[tokio::test]
    async fn quality_login_handler_rotating_peers_lock_username_but_desktop_can_recover() {
        let (_tmp, ctx, username) = fixture().await;
        let attempts = AttemptStore::default();
        for i in 0..FAILURE_THRESHOLD {
            let peer = IpAddr::from([203, 0, 113, i as u8 + 1]);
            let response = handle_login(
                &ctx,
                &attempts,
                Some(peer),
                false,
                credentials(&username, false),
            )
            .await;
            if i + 1 == FAILURE_THRESHOLD {
                assert_locked(response).await;
            } else {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
        // A fresh client with the correct password is gated before verification.
        assert_locked(
            handle_login(
                &ctx,
                &attempts,
                Some(IpAddr::from([198, 51, 100, 99])),
                false,
                credentials(&username, true),
            )
            .await,
        )
        .await;
        // A tunnel's loopback socket alone is insufficient for the exemption.
        let local = Some(IpAddr::from([127, 0, 0, 1]));
        assert_locked(
            handle_login(&ctx, &attempts, local, false, credentials(&username, true)).await,
        )
        .await;
        let response =
            handle_login(&ctx, &attempts, local, true, credentials(&username, true)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 16384)
            .await
            .unwrap();
        let login: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let authenticated = AuthRepo::new(ctx.pool.clone())
            .authenticate(login["token"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(authenticated.effective_user.username, username);
        assert!(attempts
            .check_locked(&login_throttle::username_key(&username))
            .is_none());
        // Desktop still has its own IP+username budget.
        for i in 0..FAILURE_THRESHOLD {
            let response =
                handle_login(&ctx, &attempts, local, true, credentials(&username, false)).await;
            if i + 1 == FAILURE_THRESHOLD {
                assert_locked(response).await;
            } else {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
    }

    #[tokio::test]
    async fn quality_login_handler_client_bucket_stops_username_spraying() {
        let (_tmp, ctx, username) = fixture().await;
        let attempts = AttemptStore::default();
        let peer = Some(IpAddr::from([198, 51, 100, 98]));
        for i in 0..CLIENT_FAILURE_THRESHOLD {
            let response = handle_login(
                &ctx,
                &attempts,
                peer,
                false,
                credentials(&format!("{username}-{i}"), false),
            )
            .await;
            if i + 1 == CLIENT_FAILURE_THRESHOLD {
                assert_locked(response).await;
            } else {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
        assert_locked(
            handle_login(&ctx, &attempts, peer, false, credentials(&username, true)).await,
        )
        .await;
        assert_eq!(
            handle_login(
                &ctx,
                &attempts,
                Some(IpAddr::from([198, 51, 100, 97])),
                false,
                credentials(&username, true)
            )
            .await
            .status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn quality_login_router_ignores_spoofed_forwarding_headers_and_scopes_local_exemption() {
        let (_tmp, ctx, username) = fixture().await;
        otto_state::SettingsRepo::new(ctx.pool.clone())
            .put(
                "share_base_url",
                &serde_json::json!("https://quality-login.invalid"),
            )
            .await
            .unwrap();
        let app = Router::new()
            .route("/auth/login", post(login))
            .with_state(ctx.clone())
            .layer(axum::middleware::from_fn_with_state(
                crate::host_guard::HostGuardState::new(ctx.pool.clone()),
                crate::host_guard::host_guard_with_settings,
            ));
        let peer = IpAddr::from([198, 51, 100, 237]);
        let request = |peer: IpAddr, host: &str, spoof: &str, correct: bool| {
            let mut req = Request::builder().method("POST").uri("/auth/login")
                .header("host", host).header("content-type", "application/json")
                .header("x-forwarded-for", spoof).header("x-real-ip", spoof)
                .body(Body::from(serde_json::json!({"username":username,"password":if correct {"correct fixture password"} else {"wrong"}}).to_string())).unwrap();
            req.extensions_mut()
                .insert(ConnectInfo(SocketAddr::new(peer, 12345)));
            req
        };
        for i in 0..FAILURE_THRESHOLD {
            let response = app
                .clone()
                .oneshot(request(
                    peer,
                    "quality-login.invalid",
                    &format!("203.0.113.{}", i + 1),
                    false,
                ))
                .await
                .unwrap();
            if i + 1 == FAILURE_THRESHOLD {
                assert_locked(response).await;
            } else {
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            }
        }
        // This checks actual extraction/bookkeeping, so a global username lock
        // cannot mask a regression that starts trusting either spoofed header.
        assert!(login_throttle::global()
            .check_locked(&login_throttle::ip_key(Some(peer), &username))
            .is_some());
        let loopback = IpAddr::from([127, 0, 0, 1]);
        assert_locked(
            app.clone()
                .oneshot(request(
                    loopback,
                    "quality-login.invalid",
                    "127.0.0.1",
                    true,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            app.oneshot(request(loopback, "localhost", "198.51.100.237", true))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        login_throttle::global().clear(&login_throttle::ip_key(Some(peer), &username));
        login_throttle::global().clear(&login_throttle::client_key(peer));
        login_throttle::global().clear(&login_throttle::username_key(&username));
    }
}
