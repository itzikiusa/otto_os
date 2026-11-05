//! Phone access through the REAL daemon router (docs/features/phone-access.md):
//! a paired phone's device token is the owner's credential minus credential
//! minting / listing, administration and Phone access itself; it only works
//! from the tailnet or this Mac; the public claim route refuses non-tailnet
//! peers; agent credentials can never create or approve a pairing; revoking a
//! device closes its live sockets.
//!
//! The daemon is served with `ConnectInfo`, so the host guard classifies each
//! request's peer exactly as production does. A `tailscale serve` request is
//! simulated the way tailscaled sends it: from 127.0.0.1, with the phone's
//! `Host` (`*.ts.net`) and `X-Forwarded-For` set to the tailnet address.

use std::net::SocketAddr;
use std::time::Duration;

use chrono::Utc;
use futures_util::StreamExt;
use otto_rbac::devices::{DevicesRepo, PollOutcome};
use otto_rbac::AuthRepo;
use otto_server::ServerCtx;
use otto_state::{DbPool, NewSession, SessionsRepo};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio_tungstenite::tungstenite::Message;

async fn file_pool(dir: &std::path::Path) -> DbPool {
    let opts = SqliteConnectOptions::new()
        .filename(dir.join("otto.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .expect("connect sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

async fn seed(pool: &DbPool) {
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES ('alice', 'alice', 'x', 'Alice', 1, ?)",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES ('ws1', 'ws', '/tmp', '{}', 0, ?)",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1', 'alice', 'admin')",
    )
    .execute(pool)
    .await
    .unwrap();
}

struct Daemon {
    base: String,
    ws_base: String,
    human: String,
    agent: String,
    pool: DbPool,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

const TS_HOST: &str = "mac.tail0test.ts.net";
const PHONE_IP: &str = "100.111.7.125";

async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let pool = file_pool(tmp.path()).await;
    seed(&pool).await;
    let s = SessionsRepo::new(pool.clone())
        .create(NewSession {
            workspace_id: "ws1".into(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "claude".into(),
            title: "agent".into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "alice".into(),
            meta: json!({}),
        })
        .await
        .unwrap();
    let repo = AuthRepo::new(pool.clone());
    let (human, _) = repo
        .issue_api_token(&"alice".to_string(), Some("ui"))
        .await
        .unwrap();
    let (agent, _) = repo
        .issue_session_api_token(&"alice".to_string(), &s.id)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let mut ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
    ctx.base_url = base.clone();
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Daemon {
        base,
        ws_base: format!("ws://{addr}"),
        human,
        agent,
        pool,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

/// How a request reaches the daemon.
#[derive(Clone, Copy)]
enum Via {
    /// The desktop app on this Mac.
    Mac,
    /// `tailscale serve` from a tailnet phone.
    Tailnet,
    /// A non-tailnet client behind a loopback proxy (a public address).
    Public,
    /// A Funnel request (public, even with a tailnet-looking address).
    Funnel,
}

impl Daemon {
    async fn send(
        &self,
        via: Via,
        method: reqwest::Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (u16, Value) {
        let mut req = self
            .http
            .request(method, format!("{}/api/v1{path}", self.base));
        req = match via {
            Via::Mac => req,
            Via::Tailnet => req
                .header("host", TS_HOST)
                .header("x-forwarded-for", PHONE_IP),
            Via::Public => req
                .header("host", TS_HOST)
                .header("x-forwarded-for", "203.0.113.7"),
            Via::Funnel => req
                .header("host", TS_HOST)
                .header("x-forwarded-for", PHONE_IP)
                .header("tailscale-funnel-request", "?1"),
        };
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let v = resp.json().await.unwrap_or(Value::Null);
        (status, v)
    }

    /// Pair a phone through the repo (the tailscale half is otto-phone's own
    /// suite); returns (device token, device id).
    async fn paired_phone(&self) -> (String, String) {
        let repo = DevicesRepo::new(self.pool.clone());
        let user = "alice".to_string();
        let (id, code, _) = repo.create_pairing(&user, None).await.unwrap();
        let c = repo
            .claim(&code, "iPhone · Safari", None, PHONE_IP)
            .await
            .unwrap();
        repo.decide(&user, &id, true).await.unwrap();
        match repo.poll(&id, &c.poll_secret).await.unwrap() {
            PollOutcome::Approved { token, device, .. } => (token, device.id),
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn device_token_does_normal_app_work_but_not_credentials_or_admin() {
    let d = boot().await;
    let (token, _) = d.paired_phone().await;
    use reqwest::Method as M;
    for path in [
        "/auth/me",
        "/workspaces",
        "/workspaces/ws1/sessions",
        "/phone/device/me",
    ] {
        let (s, v) = d.send(Via::Tailnet, M::GET, path, Some(&token), None).await;
        assert_eq!(s, 200, "GET {path}: {v}");
    }
    let (_, me) = d
        .send(Via::Tailnet, M::GET, "/phone/device/me", Some(&token), None)
        .await;
    assert_eq!(me["device"]["current"], true);
    for (m, path, body) in [
        (M::GET, "/auth/tokens", None),
        (M::POST, "/auth/tokens", Some(json!({"label": "x"}))),
        (M::GET, "/mcp/tokens", None),
        (M::POST, "/phone/pairings", Some(json!({}))),
        (M::POST, "/phone/pairings/P1/approve", None),
        (M::POST, "/phone/serve", Some(json!({"enabled": false}))),
        (M::GET, "/phone/devices", None),
        (
            M::POST,
            "/users",
            Some(json!({"username": "eve", "password": "x"})),
        ),
        (M::PATCH, "/settings", Some(json!({}))),
    ] {
        let (s, v) = d
            .send(Via::Tailnet, m.clone(), path, Some(&token), body)
            .await;
        assert_eq!(s, 403, "{m} {path} must be refused for a phone: {v}");
    }
}

#[tokio::test]
async fn device_token_only_works_from_the_tailnet_or_this_mac() {
    let d = boot().await;
    let (token, _) = d.paired_phone().await;
    use reqwest::Method as M;
    assert_eq!(
        d.send(Via::Mac, M::GET, "/auth/me", Some(&token), None)
            .await
            .0,
        200
    );
    assert_eq!(
        d.send(Via::Tailnet, M::GET, "/auth/me", Some(&token), None)
            .await
            .0,
        200
    );
    assert_eq!(
        d.send(Via::Public, M::GET, "/auth/me", Some(&token), None)
            .await
            .0,
        401
    );
    assert_eq!(
        d.send(Via::Funnel, M::GET, "/auth/me", Some(&token), None)
            .await
            .0,
        401
    );
    // The owner's normal token is not peer-restricted (unchanged behaviour).
    assert_eq!(
        d.send(Via::Public, M::GET, "/auth/me", Some(&d.human), None)
            .await
            .0,
        200
    );
    // last_ip records the tailnet address.
    let ip: Option<String> = sqlx::query_scalar("SELECT last_ip FROM phone_devices")
        .fetch_one(&d.pool)
        .await
        .unwrap();
    assert_eq!(ip.as_deref(), Some(PHONE_IP));
}

#[tokio::test]
async fn claim_route_refuses_non_tailnet_peers() {
    let d = boot().await;
    let repo = DevicesRepo::new(d.pool.clone());
    let (_, code, _) = repo
        .create_pairing(&"alice".to_string(), None)
        .await
        .unwrap();
    use reqwest::Method as M;
    for via in [Via::Mac, Via::Public, Via::Funnel] {
        let (s, _) = d
            .send(
                via,
                M::POST,
                "/phone/pair/claim",
                None,
                Some(json!({"code": code})),
            )
            .await;
        assert_eq!(s, 403);
    }
    let (s, v) = d
        .send(
            Via::Tailnet,
            M::POST,
            "/phone/pair/claim",
            None,
            Some(json!({"code": code})),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["confirm_code"].as_str().unwrap().len(), 6);
    // Single use.
    let (s, _) = d
        .send(
            Via::Tailnet,
            M::POST,
            "/phone/pair/claim",
            None,
            Some(json!({"code": code})),
        )
        .await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn agent_credentials_cannot_create_or_approve_pairings() {
    let d = boot().await;
    use reqwest::Method as M;
    let (s, _) = d
        .send(
            Via::Mac,
            M::POST,
            "/phone/pairings",
            Some(&d.agent),
            Some(json!({})),
        )
        .await;
    assert_eq!(s, 403);
    let (s, _) = d
        .send(
            Via::Mac,
            M::POST,
            "/phone/pairings/P1/approve",
            Some(&d.agent),
            None,
        )
        .await;
    assert_eq!(s, 403);
    let (s, _) = d
        .send(Via::Mac, M::GET, "/phone/devices", Some(&d.agent), None)
        .await;
    assert_eq!(s, 403);
}

#[tokio::test]
async fn revoking_a_device_closes_its_event_socket() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let d = boot().await;
    let (token, device_id) = d.paired_phone().await;
    let mut req = format!("{}/ws/events", d.ws_base)
        .into_client_request()
        .unwrap();
    req.headers_mut().insert(
        "sec-websocket-protocol",
        format!("otto-bearer, {token}").parse().unwrap(),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("a paired phone opens the event socket");
    let (s, _) = d
        .send(
            Via::Mac,
            reqwest::Method::DELETE,
            &format!("/phone/devices/{device_id}"),
            Some(&d.human),
            None,
        )
        .await;
    assert_eq!(s, 204);
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(msg) = ws.next().await {
            match msg {
                Ok(Message::Close(frame)) => return frame.map(|f| u16::from(f.code)),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
        None
    })
    .await
    .expect("the revoked phone's socket must close within seconds");
    assert_eq!(closed, Some(4401));
    let (s, _) = d
        .send(
            Via::Tailnet,
            reqwest::Method::GET,
            "/auth/me",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(s, 401, "the revoked device token is dead");
}
