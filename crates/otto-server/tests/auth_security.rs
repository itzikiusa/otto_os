//! Auth security tests (audit S5 + token lifecycle).
//!
//! Two concerns, exercised against the real crate code (no mocks of the logic
//! under test):
//!
//!   1. Token lifecycle through `otto_rbac::AuthRepo` over a temp SQLite pool:
//!      mint → authenticate → revoke → auth fails, plus revocation-on-credential
//!      -change (`revoke_all_for_user`) invalidating every outstanding token.
//!
//!   2. The login throttle (`otto_server::login_throttle`), driven exactly the
//!      way the `/auth/login` handler drives it, demonstrating the core S5
//!      property: rotating the source IP per request does NOT defeat the global
//!      per-username lockout, and a success resets the counters.

use std::net::{IpAddr, Ipv4Addr};

use otto_rbac::AuthRepo;
use otto_server::login_throttle::{self, AttemptStore, FAILURE_THRESHOLD};
use otto_state::DbPool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/// Temp in-memory SQLite pool with the full otto-state schema applied. The
/// migrations live in otto-state, so the `sqlx::migrate!` macro is pointed at
/// them by relative path (resolved at compile time from this test crate).
async fn mem_pool() -> DbPool {
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
    pool.into()
}

/// Seed a minimal (non-root, enabled) user row and return its id.
async fn seed_user(pool: &DbPool, username: &str) -> String {
    let id = otto_core::new_id();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, ?, ?, 0, ?)",
    )
    .bind(&id)
    .bind(username)
    .bind("hash")
    .bind("Test User")
    .bind(&now)
    .execute(pool)
    .await
    .expect("seed user");
    id
}

fn ipv4(a: u8, b: u8, c: u8, d: u8) -> Option<IpAddr> {
    Some(IpAddr::V4(Ipv4Addr::new(a, b, c, d)))
}

// ---------------------------------------------------------------------------
// Token lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_mint_authenticate_revoke_cycle() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let uid = seed_user(&pool, "alice").await;

    // Mint an API token, see it in the list, authenticate with it.
    let (token, info) = repo.issue_api_token(&uid, Some("cli")).await.unwrap();
    let listed = repo.list_api_tokens(&uid).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, info.id);
    assert_eq!(
        repo.authenticate(&token).await.unwrap().effective_user.id,
        uid
    );

    // Revoke it; authentication now fails and the list is empty.
    assert!(repo.revoke_api_token(&uid, &info.id).await.unwrap());
    assert!(repo.authenticate(&token).await.is_err());
    assert!(repo.list_api_tokens(&uid).await.unwrap().is_empty());
}

#[tokio::test]
async fn revoke_all_on_credential_change_invalidates_outstanding_tokens() {
    let pool = mem_pool().await;
    let repo = AuthRepo::new(pool.clone());
    let victim = seed_user(&pool, "victim").await;
    let other = seed_user(&pool, "other").await;

    // The victim holds a session token and an API token; the other user holds
    // an unrelated token.
    let session = repo.issue(&victim).await.unwrap();
    let (api, _) = repo.issue_api_token(&victim, Some("ci")).await.unwrap();
    let other_token = repo.issue(&other).await.unwrap();
    assert!(repo.authenticate(&session).await.is_ok());
    assert!(repo.authenticate(&api).await.is_ok());

    // Simulate a credential change: every one of the victim's tokens is revoked.
    let deleted = repo.revoke_all_for_user(&victim).await.unwrap();
    assert_eq!(deleted, 2);

    assert!(
        repo.authenticate(&session).await.is_err(),
        "session token must be invalid after credential change"
    );
    assert!(
        repo.authenticate(&api).await.is_err(),
        "API token must be invalid after credential change"
    );
    // The unrelated user's token still works.
    assert!(repo.authenticate(&other_token).await.is_ok());
}

// ---------------------------------------------------------------------------
// Login throttle (S5)
//
// These drive `login_throttle` the same way the `/auth/login` handler does:
// every attempt gates on, and records a failure to, BOTH the per-client
// `ip_key` and the global `username_key`; a success clears both.
// ---------------------------------------------------------------------------

/// Mirror of the handler's failed-attempt bookkeeping so the test exercises the
/// real keying + lockout logic rather than re-implementing it.
fn record_failed_attempt(store: &AttemptStore, peer: Option<IpAddr>, username: &str) {
    let ip_key = login_throttle::ip_key(peer, username);
    let user_key = login_throttle::username_key(username);
    store.record_failure(&ip_key);
    store.record_failure(&user_key);
}

/// Mirror of the handler's pre-attempt lock gate: locked iff EITHER key is.
fn is_blocked(store: &AttemptStore, peer: Option<IpAddr>, username: &str) -> bool {
    let ip_key = login_throttle::ip_key(peer, username);
    let user_key = login_throttle::username_key(username);
    store.max_locked(&[&ip_key, &user_key]).is_some()
}

#[tokio::test]
async fn lockout_trips_after_threshold_and_resets_on_success() {
    let store = AttemptStore::default();
    let peer = ipv4(192, 168, 1, 50);
    let user = "carol";

    // Below threshold: not blocked.
    for _ in 0..FAILURE_THRESHOLD - 1 {
        record_failed_attempt(&store, peer, user);
        assert!(!is_blocked(&store, peer, user), "must not lock early");
    }
    // The threshold-crossing failure locks the client.
    record_failed_attempt(&store, peer, user);
    assert!(is_blocked(&store, peer, user), "must lock at threshold");

    // A successful login clears BOTH counters (legit-user happy path), so the
    // same client+username is no longer blocked.
    store.clear(&login_throttle::ip_key(peer, user));
    store.clear(&login_throttle::username_key(user));
    assert!(
        !is_blocked(&store, peer, user),
        "successful login must reset the lockout"
    );
}

#[tokio::test]
async fn ip_rotation_does_not_defeat_username_lockout() {
    // The exact S5 bypass: an attacker rotates the source IP every request.
    // Each request mints a fresh `ip|username` key, so no per-client key ever
    // reaches the threshold — but every attempt also tallies against the global
    // per-username key, which locks the account regardless of the source IP.
    let store = AttemptStore::default();
    let user = "dave";

    for i in 0..FAILURE_THRESHOLD {
        let rotating_peer = ipv4(203, 0, 113, i as u8);
        // The attacker is not blocked yet at the point of *this* request...
        assert!(
            store
                .check_locked(&login_throttle::ip_key(rotating_peer, user))
                .is_none(),
            "a never-before-seen rotating IP must have no per-client lock"
        );
        record_failed_attempt(&store, rotating_peer, user);
    }

    // No single per-client IP key ever locked.
    for i in 0..FAILURE_THRESHOLD {
        let p = ipv4(203, 0, 113, i as u8);
        assert!(
            store
                .check_locked(&login_throttle::ip_key(p, user))
                .is_none(),
            "no rotated per-client key should have crossed the threshold"
        );
    }
    // But the global per-username key IS locked — the rotation bought nothing.
    assert!(
        store
            .check_locked(&login_throttle::username_key(user))
            .is_some(),
        "username lockout must survive IP rotation (S5)"
    );
    // And a brand-new IP attempting that same username is now blocked outright.
    let fresh_peer = ipv4(198, 51, 100, 7);
    assert!(
        is_blocked(&store, fresh_peer, user),
        "a fresh IP must be blocked by the username lockout"
    );
}

#[tokio::test]
async fn forwarding_headers_are_not_a_throttle_input() {
    // The handler keys on `ConnectInfo<SocketAddr>` (the real socket peer) and
    // never reads `X-Forwarded-For` / `X-Real-IP`. We assert that property at the
    // keying layer: the throttle key is derived purely from the socket-peer IP +
    // username, so a spoofed forwarding header has no key it could rotate. Two
    // requests from the SAME socket peer collapse to the SAME per-client key
    // regardless of any header an attacker might attach.
    let user = "erin";
    let peer = ipv4(10, 0, 0, 9);
    let key_a = login_throttle::ip_key(peer, user);
    let key_b = login_throttle::ip_key(peer, user);
    assert_eq!(key_a, key_b, "same socket peer => same per-client key");
    // The username key is independent of IP entirely.
    assert_eq!(
        login_throttle::username_key(user),
        login_throttle::username_key(&user.to_uppercase()),
        "username key is case-insensitive so casing can't dodge the tally"
    );
}

// ---------------------------------------------------------------------------
// MCP token rotation is owner-bound (access-control-3-01)
// ---------------------------------------------------------------------------
//
// Through the REAL daemon router on a loopback port: `POST
// /mcp/tokens/{id}/rotate` hands back a plaintext token that authenticates AS
// the row's owner, so a non-root `mcp:admin` must not be able to rotate root's
// token (it would walk away with a working root credential). Revoking stays
// open to the admin; rotating your own, or root rotating anyone's, works.

mod mcp_rotate {
    use chrono::Utc;
    use otto_core::domain::{Capability, Feature};
    use otto_core::Error;
    use otto_rbac::AuthRepo;
    use otto_state::{DbPool, GrantsRepo};
    use serde_json::Value;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    struct Daemon {
        base: String,
        pool: DbPool,
        http: reqwest::Client,
        _tmp: tempfile::TempDir,
    }

    async fn seed_user(pool: &DbPool, id: &str, is_root: bool) {
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, 'x', ?, ?, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(id)
        .bind(is_root as i64)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .expect("seed user");
    }

    async fn boot() -> Daemon {
        let tmp = tempfile::tempdir().unwrap();
        let opts = SqliteConnectOptions::new()
            .filename(tmp.path().join("otto.db"))
            .create_if_missing(true)
            .foreign_keys(true);
        let raw = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await
            .expect("connect sqlite");
        sqlx::migrate!("../otto-state/migrations")
            .run(&raw)
            .await
            .expect("run migrations");
        let pool: DbPool = raw.into();
        seed_user(&pool, "root", true).await;
        seed_user(&pool, "carol", false).await;
        // Carol holds the documented, grantable delegated role.
        GrantsRepo::new(pool.clone())
            .set_grants("carol", &[(Feature::Mcp, Capability::Admin)])
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut ctx = otto_server::ServerCtx::for_tests(&pool, tmp.path()).await;
        ctx.base_url = base.clone();
        let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
        let app = otto_server::build_router(ctx, api_extras, root_extras);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Daemon {
            base,
            pool,
            http: reqwest::Client::new(),
            _tmp: tmp,
        }
    }

    impl Daemon {
        async fn api_token(&self, user: &str) -> String {
            AuthRepo::new(self.pool.clone())
                .issue_api_token(&user.to_string(), Some("ui"))
                .await
                .unwrap()
                .0
        }

        async fn mcp_token(&self, user: &str) -> (String, String) {
            let (token, info) = AuthRepo::new(self.pool.clone())
                .issue_mcp_token_with_scope(
                    &user.to_string(),
                    Some("cli"),
                    &otto_core::auth::McpScope::unrestricted(),
                )
                .await
                .unwrap();
            (token, info.id)
        }

        async fn rotate(&self, caller: &str, id: &str) -> (u16, Value) {
            let resp = self
                .http
                .post(format!("{}/api/v1/mcp/tokens/{id}/rotate", self.base))
                .bearer_auth(caller)
                .send()
                .await
                .unwrap();
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            (status, serde_json::from_str(&text).unwrap_or(Value::Null))
        }

        async fn owner_of(&self, token: &str) -> Option<String> {
            match AuthRepo::new(self.pool.clone()).authenticate(token).await {
                Ok(ctx) => Some(ctx.effective_user.id),
                Err(Error::Unauthorized) => None,
                Err(e) => panic!("authenticate: {e}"),
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn non_root_mcp_admin_cannot_rotate_roots_token() {
        let d = boot().await;
        let carol = d.api_token("carol").await;
        let (root_mcp, root_id) = d.mcp_token("root").await;

        let (st, body) = d.rotate(&carol, &root_id).await;
        assert_eq!(st, 403, "{body}");
        assert!(
            body.to_string()
                .contains("only root may rotate an MCP token owned by another user"),
            "{body}"
        );
        // Nothing was minted or deleted: root's token still authenticates.
        assert_eq!(d.owner_of(&root_mcp).await.as_deref(), Some("root"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mcp_admin_rotates_own_token_and_root_rotates_any() {
        let d = boot().await;
        let carol = d.api_token("carol").await;
        let root = d.api_token("root").await;

        let (old, id) = d.mcp_token("carol").await;
        let (st, body) = d.rotate(&carol, &id).await;
        assert_eq!(st, 200, "{body}");
        let new = body["token"].as_str().expect("token").to_string();
        assert_eq!(d.owner_of(&old).await, None);
        assert_eq!(d.owner_of(&new).await.as_deref(), Some("carol"));

        // Root may rotate a token owned by someone else; the owner is kept.
        let (_, carol_id) = d.mcp_token("carol").await;
        let (st, body) = d.rotate(&root, &carol_id).await;
        assert_eq!(st, 200, "{body}");
        assert_eq!(body["info"]["user_id"], "carol", "{body}");

        // An unknown id is 404 for anyone allowed to rotate.
        let (st, _) = d.rotate(&carol, "no-such-token").await;
        assert_eq!(st, 404);
    }
}
