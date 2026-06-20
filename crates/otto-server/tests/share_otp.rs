//! Email-OTP gate for share links (mobile plan Tasks 7.2 + 7.3).
//!
//! A share link's recipient must redeem an emailed one-time code before the
//! scoped token reaches ANYTHING — so a leaked link alone is useless. These
//! tests prove the gate end-to-end through the **real** code paths:
//!
//! - **Mint + email (7.2):** [`otto_server::routes::share::mint_otp_share`] with an
//!   injected capturing mailer → an OTP is generated and "emailed", and the minted
//!   share authenticates as `scope.otp_pending == true`. No real SMTP is touched.
//! - **Pre-verify deny (7.3):** the same `feature_guard` production layers, fed the
//!   real `AuthContext` from `AuthRepo::authenticate`, returns **403 on GET the
//!   session** while OTP-pending.
//! - **Verify → single-use → allow (7.3):** the correct OTP flips `verified_at`,
//!   clears the pending flag (GET allowed), and the same code cannot be reused.
//! - **Expired OTP rejects; plain (no-recipient) share has no gate.**
//! - **Policy:** `/share/verify` is Exempt (public; the token is the auth).
//!
//! The full `ServerCtx`-backed `verify_share` HTTP handler (IP throttle + audit)
//! is not assembled here — building a `ServerCtx` couples to ~30 subsystems — but
//! every behaviour it relies on (`verify_share_otp` single-use/expiry and the
//! `ShareThrottle` wiring) is covered at the repo / throttle layer.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::{from_fn, from_fn_with_state, Next};
use axum::routing::get;
use axum::Router;
use chrono::Utc;
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::domain::WorkspaceRole;
use otto_core::{new_id, Error, Id};
use otto_rbac::AuthRepo;
use otto_server::feature_guard::feature_guard;
use otto_core::secrets::SecretStore;
use otto_server::routes::share::{extend_otp_share, mint_otp_share, resolve_verified_sender, OtpMailer};
use otto_state::{EmailSendersRepo, GrantsRepo, SqlitePool};
use std::collections::HashMap;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tower::ServiceExt; // for `oneshot`

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async fn mem_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

async fn seed_user(pool: &SqlitePool, username: &str) -> Id {
    let id = new_id();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, ?, ?, 0, ?)",
    )
    .bind(&id)
    .bind(username)
    .bind("hash")
    .bind(username)
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

/// A capturing mailer: records `(to, otp)` instead of sending real email, so the
/// test can assert the OTP that was "emailed" WITHOUT touching SMTP.
#[derive(Default, Clone)]
struct CaptureMailer {
    sent: Arc<Mutex<Vec<(String, String)>>>,
}

impl OtpMailer for CaptureMailer {
    fn send_otp<'a>(
        &'a self,
        to: &'a str,
        otp: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        let sent = self.sent.clone();
        let to = to.to_string();
        let otp = otp.to_string();
        Box::pin(async move {
            sent.lock().unwrap().push((to, otp));
            Ok(())
        })
    }
}

/// Minimal in-memory SecretStore (mirrors the MemSecrets pattern used by
/// `email_sender_storage.rs`) so the no-real-Keychain sender path is testable.
#[derive(Default)]
struct MemSecrets {
    map: Mutex<HashMap<String, String>>,
}
impl SecretStore for MemSecrets {
    fn put(&self, key: &str, value: &str) -> Result<(), Error> {
        self.map.lock().unwrap().insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<String>, Error> {
        Ok(self.map.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<(), Error> {
        self.map.lock().unwrap().remove(key);
        Ok(())
    }
}

/// A mailer that always fails — proves a delivery failure revokes the share.
struct FailingMailer;
impl OtpMailer for FailingMailer {
    fn send_otp<'a>(
        &'a self,
        _to: &'a str,
        _otp: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async move { Err(Error::Upstream("smtp down".into())) })
    }
}

// ---------------------------------------------------------------------------
// feature-guard harness (mirrors share_scope_guard.rs): real authenticate →
// real guard, GET the session.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct TestState {
    grants: GrantsRepo,
}
impl otto_server::feature_guard::HasGrants for TestState {
    fn grants(&self) -> GrantsRepo {
        self.grants.clone()
    }
}

fn app(pool: SqlitePool, ctx: AuthContext) -> Router {
    let state = TestState {
        grants: GrantsRepo::new(pool),
    };
    async fn ok() -> &'static str {
        "ok"
    }
    let protected = Router::new().route("/sessions/{id}", get(ok));
    let protected = protected.route_layer(from_fn_with_state(
        state.clone(),
        feature_guard::<TestState>,
    ));
    let ctx = Arc::new(ctx);
    let protected = protected.layer(from_fn(move |mut req: Request, next: Next| {
        let ctx = ctx.clone();
        async move {
            req.extensions_mut()
                .insert(AuthUser(ctx.effective_user.clone()));
            req.extensions_mut().insert((*ctx).clone());
            next.run(req).await
        }
    }));
    Router::new().nest("/api/v1", protected).with_state(state)
}

async fn status(app: &Router, method: Method, path: &str) -> StatusCode {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap().status()
}

// ---------------------------------------------------------------------------
// Policy: /share/verify is Exempt (public; the token is the auth).
// ---------------------------------------------------------------------------

#[test]
fn share_verify_is_exempt() {
    use otto_server::policy::{policy_for, PolicyDecision};
    assert_eq!(
        policy_for(&Method::POST, "/api/v1/share/verify"),
        PolicyDecision::Exempt,
        "POST /share/verify must be Exempt (token-in-body is the auth)"
    );
}

// ---------------------------------------------------------------------------
// 7.2: mint with recipient emails the OTP and the share is OTP-pending.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mint_otp_share_emails_code_and_is_pending() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner").await;
    let mailer = CaptureMailer::default();

    let (token, info) = mint_otp_share(
        &repo,
        &owner,
        &Id::from("S1"),
        WorkspaceRole::Viewer,
        3600,
        Some("for guest".into()),
        "guest@example.com",
        &mailer,
    )
    .await
    .expect("mint otp share");
    assert_eq!(info.session_id, Id::from("S1"));

    // The OTP was "emailed" to the recipient, and it's a 6-digit code.
    let sent = mailer.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "exactly one OTP email is sent");
    assert_eq!(sent[0].0, "guest@example.com", "to the locked recipient");
    assert_eq!(sent[0].1.len(), 6, "the OTP is 6 digits");

    // The minted share authenticates as OTP-pending.
    let ctx = repo.authenticate(&token).await.expect("auth share");
    assert!(
        ctx.scope.unwrap().otp_pending,
        "a freshly-minted OTP share must be pending"
    );
}

/// If delivery fails the just-minted share is revoked (no dangling OTP-pending
/// capability whose code nobody received).
#[tokio::test]
async fn mint_otp_share_revokes_on_delivery_failure() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner").await;

    let err = mint_otp_share(
        &repo,
        &owner,
        &Id::from("S1"),
        WorkspaceRole::Viewer,
        3600,
        None,
        "guest@example.com",
        &FailingMailer,
    )
    .await;
    assert!(err.is_err(), "a delivery failure must fail the mint");
    // No live share remains for the session.
    assert!(
        repo.list_shares_for_session(&Id::from("S1"))
            .await
            .unwrap()
            .is_empty(),
        "the share must be revoked after a delivery failure"
    );
}

// ---------------------------------------------------------------------------
// 7.3: before verify → 403 on GET session; after verify → 200; single-use.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_session_denied_until_otp_verified() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner").await;
    let mailer = CaptureMailer::default();

    let (token, _info) = mint_otp_share(
        &repo,
        &owner,
        &Id::from("S1"),
        WorkspaceRole::Viewer,
        3600,
        None,
        "guest@example.com",
        &mailer,
    )
    .await
    .unwrap();
    let otp = mailer.sent.lock().unwrap()[0].1.clone();

    // Pre-verify: even GET the session (the one allow-listed read) is 403.
    let ctx = repo.authenticate(&token).await.unwrap();
    let pending_app = app(pool.clone(), ctx);
    assert_eq!(
        status(&pending_app, Method::GET, "/api/v1/sessions/S1").await,
        StatusCode::FORBIDDEN,
        "an OTP-pending share is denied even GET its own session"
    );

    // Wrong OTP → not verified (and stays pending).
    let wrong = if otp == "000000" { "111111" } else { "000000" };
    assert!(!repo.verify_share_otp(&token, wrong).await.unwrap());
    assert!(
        repo.authenticate(&token).await.unwrap().scope.unwrap().otp_pending,
        "a wrong OTP must leave the share pending"
    );

    // Correct OTP → verified; verified_at is set; GET now allowed.
    assert!(repo.verify_share_otp(&token, &otp).await.unwrap());
    let verified_at: Option<i64> =
        sqlx::query_scalar("SELECT verified_at FROM auth_sessions WHERE token_hash = ?")
            .bind(otto_rbac::tokens::token_hash(&token))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(verified_at.is_some(), "verified_at must be set on success");

    let ctx = repo.authenticate(&token).await.unwrap();
    assert!(!ctx.scope.unwrap().otp_pending, "verified share is no longer pending");
    let verified_app = app(pool.clone(), repo.authenticate(&token).await.unwrap());
    assert_eq!(
        status(&verified_app, Method::GET, "/api/v1/sessions/S1").await,
        StatusCode::OK,
        "a verified share may GET its session"
    );

    // Single-use: the same code cannot be redeemed again.
    assert!(
        !repo.verify_share_otp(&token, &otp).await.unwrap(),
        "the OTP is single-use"
    );
}

/// An expired OTP cannot be redeemed (otp_expires_at in the past).
#[tokio::test]
async fn expired_otp_is_rejected() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner").await;
    let mailer = CaptureMailer::default();

    let (token, _info) = mint_otp_share(
        &repo,
        &owner,
        &Id::from("S1"),
        WorkspaceRole::Viewer,
        3600,
        None,
        "guest@example.com",
        &mailer,
    )
    .await
    .unwrap();
    let otp = mailer.sent.lock().unwrap()[0].1.clone();

    // Backdate the OTP expiry.
    let past = (Utc::now() - chrono::Duration::minutes(1)).timestamp();
    sqlx::query("UPDATE auth_sessions SET otp_expires_at = ? WHERE token_hash = ?")
        .bind(past)
        .bind(otto_rbac::tokens::token_hash(&token))
        .execute(&pool)
        .await
        .unwrap();

    assert!(
        !repo.verify_share_otp(&token, &otp).await.unwrap(),
        "an expired OTP must be rejected"
    );
}

// ---------------------------------------------------------------------------
// 7.2: minting an OTP share requires a VERIFIED email sender → else 400.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn otp_mint_requires_verified_sender() {
    let pool = mem_pool().await;
    let senders = EmailSendersRepo::new(pool.clone());
    let secrets = MemSecrets::default();
    let owner = seed_user(&pool, "owner").await;

    // (a) No sender at all → 400 Invalid.
    let err = resolve_verified_sender(&senders, &secrets, &owner)
        .await
        .expect_err("no sender must error");
    assert!(matches!(err.0, Error::Invalid(_)), "got {:?}", err.0);

    // (b) Unverified sender (verified_at NULL) → still 400.
    senders
        .upsert(&owner, "owner@gmail.com", "email-sender-ref")
        .await
        .unwrap();
    secrets.put("email-sender-ref", "app-pw").unwrap();
    let err = resolve_verified_sender(&senders, &secrets, &owner)
        .await
        .expect_err("unverified sender must error");
    assert!(matches!(err.0, Error::Invalid(_)), "got {:?}", err.0);

    // (c) Verified sender + secret present → resolves to (address, password).
    senders.set_verified(&owner, Utc::now()).await.unwrap();
    let (addr, pw) = resolve_verified_sender(&senders, &secrets, &owner)
        .await
        .expect("verified sender resolves");
    assert_eq!(addr, "owner@gmail.com");
    assert_eq!(pw, "app-pw");
}

// ---------------------------------------------------------------------------
// 7.4: extend re-emails a FRESH OTP to the LOCKED original recipient ONLY.
//
// The locked-recipient property is the security crux: extend takes NO email in
// its request (its signature is `(repo, owner, token, mailer)`), so there is no
// code path by which it can send anywhere but the address stored on the row. The
// destination asserted below is the recipient captured at MINT time, proven to be
// the one extend re-emails — never an attacker-supplied address.
// ---------------------------------------------------------------------------

/// Extending an OTP share re-emails a fresh code to the ORIGINAL recipient only,
/// re-pends the share (verified_at cleared), the new code differs from the old,
/// the NEW code re-verifies (≤12h window) and the OLD code no longer works.
#[tokio::test]
async fn extend_emails_fresh_otp_to_locked_recipient_and_re_pends() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner").await;
    let mailer = CaptureMailer::default();

    // Mint, redeem, and verify the share once (the normal happy path).
    let (token, _info) = mint_otp_share(
        &repo,
        &owner,
        &Id::from("S1"),
        WorkspaceRole::Viewer,
        3600,
        Some("for guest".into()),
        "guest@example.com",
        &mailer,
    )
    .await
    .expect("mint otp share");
    let old_otp = mailer.sent.lock().unwrap()[0].1.clone();
    assert!(repo.verify_share_otp(&token, &old_otp).await.unwrap());
    assert!(
        !repo.authenticate(&token).await.unwrap().scope.unwrap().otp_pending,
        "verified share is not pending before extend"
    );

    // EXTEND — note the call carries NO email; the destination is read from the
    // share row. The handler signature is the proof: it cannot redirect delivery.
    extend_otp_share(&repo, &owner, &token, &mailer)
        .await
        .expect("extend otp share");

    // A SECOND email went out, and it went to the SAME locked recipient.
    let sent = mailer.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2, "extend sends exactly one more OTP email");
    assert_eq!(
        sent[1].0, "guest@example.com",
        "extend re-emails the LOCKED original recipient, never elsewhere"
    );
    let new_otp = sent[1].1.clone();
    assert_eq!(new_otp.len(), 6, "the fresh OTP is 6 digits");
    assert_ne!(new_otp, old_otp, "the fresh OTP differs from the old one");

    // The share is OTP-pending again (verified_at was cleared on extend).
    assert!(
        repo.authenticate(&token).await.unwrap().scope.unwrap().otp_pending,
        "after extend the share must be OTP-pending again (re-verify required)"
    );

    // The OLD code no longer works; the NEW code re-verifies and re-opens ≤12h.
    assert!(
        !repo.verify_share_otp(&token, &old_otp).await.unwrap(),
        "the old OTP must not work after extend"
    );
    assert!(
        repo.verify_share_otp(&token, &new_otp).await.unwrap(),
        "the fresh OTP re-verifies the share"
    );
    assert!(
        !repo.authenticate(&token).await.unwrap().scope.unwrap().otp_pending,
        "the share re-opens after re-verifying with the fresh code"
    );

    // The new window end is in the future and within 12h of now (≤12h per window).
    let max_expires_at: Option<i64> =
        sqlx::query_scalar("SELECT max_expires_at FROM auth_sessions WHERE token_hash = ?")
            .bind(otto_rbac::tokens::token_hash(&token))
            .fetch_one(&pool)
            .await
            .unwrap();
    let now = Utc::now().timestamp();
    let window = max_expires_at.expect("max_expires_at is set") - now;
    assert!(window > 0, "the extended window is in the future");
    assert!(window <= 12 * 60 * 60, "each granted window stays ≤12h");
}

/// Extending a share that has NO recipient_email (a plain, non-OTP share) is a
/// `400` — only OTP shares are extendable.
#[tokio::test]
async fn extend_rejects_plain_non_otp_share() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner").await;
    let mailer = CaptureMailer::default();

    // A plain scoped share (no recipient_email → no OTP gate).
    let (token, _info) = repo
        .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
        .await
        .unwrap();

    let err = extend_otp_share(&repo, &owner, &token, &mailer)
        .await
        .expect_err("extending a plain share must error");
    assert!(
        matches!(err.0, Error::Invalid(_)),
        "extending a non-OTP share is a 400 Invalid, got {:?}",
        err.0
    );
    // Nothing was emailed.
    assert!(
        mailer.sent.lock().unwrap().is_empty(),
        "no OTP is emailed when there is no recipient to lock onto"
    );
}

/// Locked-recipient proof at the repository layer: `extend_share_otp` returns the
/// destination it read from the row — there is no parameter by which a caller can
/// influence WHERE the code goes. The address it hands back equals the one stored
/// at mint, regardless of anything the caller might wish.
#[tokio::test]
async fn extend_destination_is_read_from_row_not_request() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner").await;
    let mailer = CaptureMailer::default();

    let (token, _info) = mint_otp_share(
        &repo,
        &owner,
        &Id::from("S1"),
        WorkspaceRole::Viewer,
        3600,
        None,
        "original@example.com",
        &mailer,
    )
    .await
    .unwrap();

    // The repo-layer extend takes ONLY the token — no email argument exists. It
    // hands back the destination it READ from the row (recipient) plus the share's
    // owner (so the route can resolve the owner's verified sender).
    let (new_otp, recipient, row_owner) = repo
        .extend_share_otp(&token)
        .await
        .expect("extend lookup")
        .expect("an OTP share is extendable");
    assert_eq!(
        recipient, "original@example.com",
        "the destination is the STORED recipient, read from the row"
    );
    assert_eq!(row_owner, owner, "the owner is read from the row");
    assert_eq!(new_otp.len(), 6, "a fresh 6-digit OTP is generated");
}

// ---------------------------------------------------------------------------
// Policy: /share/extend is Exempt (public; the share token is the auth).
// ---------------------------------------------------------------------------

#[test]
fn share_extend_is_exempt() {
    use otto_server::policy::{policy_for, PolicyDecision};
    assert_eq!(
        policy_for(&Method::POST, "/api/v1/share/extend"),
        PolicyDecision::Exempt,
        "POST /share/extend must be Exempt (token-in-body is the auth)"
    );
}

// ---------------------------------------------------------------------------
// Backward compat: a plain share (no recipient) has no OTP gate.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plain_share_has_no_otp_gate() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let owner = seed_user(&pool, "owner").await;

    let (token, _info) = repo
        .issue_share_token(&owner, &Id::from("S1"), WorkspaceRole::Viewer, 3600, None)
        .await
        .unwrap();
    let ctx = repo.authenticate(&token).await.unwrap();
    assert!(
        !ctx.scope.unwrap().otp_pending,
        "a plain share is never OTP-pending"
    );
    // GET the session is immediately allowed (no verify step).
    let plain_app = app(pool, repo.authenticate(&token).await.unwrap());
    assert_eq!(
        status(&plain_app, Method::GET, "/api/v1/sessions/S1").await,
        StatusCode::OK,
        "a plain share may GET its session without any OTP step"
    );
}
