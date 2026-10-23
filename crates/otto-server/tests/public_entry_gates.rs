//! The unauthenticated entry points refuse a missing or wrong credential
//! (test-quality-6-01). These routes sit in `public_routes()`, outside the
//! bearer chokepoint, and each validates its own credential in the handler —
//! the helpers were unit-tested, but deleting the check that wires one in
//! passed every test. Each case here sends the real request and asserts both
//! the status and that nothing was created.
//!
//! A request with the right credential is shown to PASS the gate by reaching
//! the handler's next check (a body/onboarding refusal), not by a 202: a
//! fixture with a channel bridge would spawn a real agent CLI.
//!
//! The Telegram sender allow-list is pinned in `otto-channels`
//! (`a_strangers_text_starts_no_session_and_is_listed_rejected`), and
//! `verify_ingest_token` by its own unit test in `otto-sessions`.
use otto_core::domain::{Channel, ConnectionKind, Environment};
use otto_server::ServerCtx;
use otto_state::{NewConnection, NewPlugin, NewSwarm, UsersRepo};
use serde_json::json;

const KEY: &str = "right-webhook-key";

struct Fixture {
    ctx: ServerCtx,
    origin: String,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    _tmp: tempfile::TempDir,
}

async fn fixture() -> (Fixture, otto_core::domain::User) {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let root = UsersRepo::new(pool.clone())
        .create("root", "unused", "Root", true)
        .await
        .unwrap();
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        Fixture {
            ctx,
            origin,
            client: reqwest::Client::new(),
            server,
            _tmp: tmp,
        },
        root,
    )
}

/// A workspace whose webhook integration is `enabled` with [`KEY`] configured.
async fn webhook_workspace(
    f: &Fixture,
    owner: &otto_core::Id,
    name: &str,
    enabled: bool,
) -> String {
    let root = f._tmp.path().to_str().unwrap();
    let ws = f.ctx.workspaces.create(name, root, owner).await.unwrap();
    f.ctx
        .integrations_store
        .upsert(
            &ws.id,
            Channel::Webhook,
            enabled,
            None,
            None,
            "",
            false,
            "",
            "",
            "claude",
        )
        .await
        .unwrap();
    f.ctx
        .secrets
        .put(&format!("chan-bot-{}-webhook", ws.id), KEY)
        .unwrap();
    ws.id
}

/// The three key variants every webhook route must refuse, then the right key.
fn keyed(rb: reqwest::RequestBuilder, key: Option<&str>) -> reqwest::RequestBuilder {
    match key {
        Some(k) => rb.header("x-otto-webhook-key", k),
        None => rb,
    }
}

#[tokio::test]
async fn webhook_routes_refuse_a_missing_or_wrong_key_and_a_disabled_integration() {
    let (f, root) = fixture().await;
    let ws = webhook_workspace(&f, &root.id, "A", true).await;
    let off = webhook_workspace(&f, &root.id, "Off", false).await;
    let other = webhook_workspace(&f, &root.id, "B", true).await;
    let swarm = |ws: String| NewSwarm {
        workspace_id: ws,
        name: "team".into(),
        description: String::new(),
        preset_slug: None,
        config: json!({}),
        max_total_runs: None,
        max_cost_usd: None,
        max_runtime_secs: None,
        max_attempts: None,
        created_by: root.id.clone(),
    };
    let own_swarm = f
        .ctx
        .swarm_repo
        .create_swarm(swarm(ws.clone()))
        .await
        .unwrap();
    let off_swarm = f
        .ctx
        .swarm_repo
        .create_swarm(swarm(off.clone()))
        .await
        .unwrap();
    let other_swarm = f.ctx.swarm_repo.create_swarm(swarm(other)).await.unwrap();

    let channel = |ws: &str| format!("{}/webhooks/{ws}", f.origin);
    let run = |ws: &str| format!("{}/webhooks/{ws}/run", f.origin);
    let swarm_url = |ws: &str, sid: &str| format!("{}/webhooks/swarm/{ws}/{sid}", f.origin);
    let body = json!({"text": "do it", "seed_text": "do it", "goal": "ship it"});

    for (label, url_on, url_off) in [
        ("channel", channel(&ws), channel(&off)),
        ("run", run(&ws), run(&off)),
        (
            "swarm",
            swarm_url(&ws, &own_swarm.id),
            swarm_url(&off, &off_swarm.id),
        ),
    ] {
        for key in [None, Some("wrong-key"), Some("")] {
            let r = keyed(f.client.post(&url_on), key)
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 401, "{label} with key {key:?}");
        }
        // `Authorization: Bearer <wrong>` is the second credential slot.
        let r = f
            .client
            .post(&url_on)
            .bearer_auth("wrong-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401, "{label} with a wrong bearer");
        // Disabling the integration stops the key, though it is still stored.
        let r = keyed(f.client.post(&url_off), Some(KEY))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "{label} on a disabled integration");
    }

    // The right key passes the gate: each route reaches its next check.
    let r = keyed(f.client.post(channel(&ws)), Some(KEY))
        .json(&json!({"text": "  "}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        400,
        "channel: past the key, the body is checked"
    );
    let r = f
        .client
        .post(channel(&ws))
        .bearer_auth(KEY)
        .json(&json!({"text": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503, "channel: the bearer slot works too");
    let r = keyed(f.client.post(run(&ws)), Some(KEY))
        .json(&json!({"seed_text": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 503, "run: past the key, onboarding is checked");
    let r = keyed(f.client.post(swarm_url(&ws, &own_swarm.id)), Some(KEY))
        .json(&json!({"goal": " "}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400, "swarm: past the key, the goal is checked");
    // A's key never reaches B's swarm.
    let r = keyed(f.client.post(swarm_url(&ws, &other_swarm.id)), Some(KEY))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "another workspace's swarm");

    // Nothing was started anywhere.
    for w in [&ws, &off] {
        assert!(f.ctx.manager.list_by_workspace(w).await.unwrap().is_empty());
        assert!(f
            .ctx
            .runs
            .list_by_workspace(w, 10)
            .await
            .unwrap()
            .is_empty());
    }
    for s in [&own_swarm, &off_swarm, &other_swarm] {
        assert!(
            f.ctx
                .swarm_repo
                .list_projects(&s.id)
                .await
                .unwrap()
                .is_empty(),
            "no swarm project for {}",
            s.id
        );
    }
    f.server.abort();
}

#[tokio::test]
async fn db_assist_query_needs_the_assist_key_and_stays_read_only() {
    let (f, root) = fixture().await;
    let conn = otto_state::ConnectionsRepo::new(f.ctx.pool.clone())
        .create(NewConnection {
            workspace_id: None,
            name: "DB".into(),
            kind: ConnectionKind::Mysql,
            // Port 1: a driver call would fail loudly, never reach a server.
            params: json!({"host": "127.0.0.1", "port": 1}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    f.ctx.db_assist.lock().unwrap().insert(
        "aid-1".into(),
        otto_server::db_assist::DbAssistEntry {
            dir: f._tmp.path().join("assist"),
            key: "assist-key".into(),
            session_id: "s".into(),
            connection_id: conn.id.clone(),
            workspace_id: "w".into(),
            provider: "claude".into(),
            user_id: root.id.clone(),
            node: None,
        },
    );
    let url = format!("{}/db-assist/aid-1/query", f.origin);
    let select = json!({"sql": "SELECT 1"});
    for key in [None, Some(""), Some("wrong")] {
        let mut rb = f.client.post(&url).json(&select);
        if let Some(k) = key {
            rb = rb.header("x-assist-key", k);
        }
        assert_eq!(rb.send().await.unwrap().status(), 401, "key {key:?}");
    }
    let r = f
        .client
        .post(format!("{}/db-assist/no-such/query", f.origin))
        .header("x-assist-key", "assist-key")
        .json(&select)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "an unknown assist");
    // The right key: a write is refused before any driver call.
    let r = f
        .client
        .post(&url)
        .header("x-assist-key", "assist-key")
        .json(&json!({"sql": "UPDATE users SET admin = 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .starts_with("read-only"),
        "{body}"
    );
    f.server.abort();
}

#[tokio::test]
async fn swarm_board_ingest_without_a_valid_session_token_writes_nothing() {
    let (f, root) = fixture().await;
    let ws = f
        .ctx
        .workspaces
        .create("A", f._tmp.path().to_str().unwrap(), &root.id)
        .await
        .unwrap();
    let swarm = f
        .ctx
        .swarm_repo
        .create_swarm(NewSwarm {
            workspace_id: ws.id.clone(),
            name: "team".into(),
            description: String::new(),
            preset_slug: None,
            config: json!({}),
            max_total_runs: None,
            max_cost_usd: None,
            max_runtime_secs: None,
            max_attempts: None,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let url = format!("{}/ingest/swarm/board", f.origin);
    let post = json!({"body": "I am the leader now", "kind": "message"});
    let cases: [(&str, Option<&str>, Option<&str>); 4] = [
        ("no session, no token", None, None),
        ("no token", Some("sess-1"), None),
        ("empty token", Some("sess-1"), Some("")),
        // No token was minted for this session in this daemon.
        ("a guessed token", Some("sess-1"), Some("deadbeef")),
    ];
    for (label, sid, token) in cases {
        let mut rb = f.client.post(&url).json(&post);
        if let Some(s) = sid {
            rb = rb.header("x-otto-session", s);
        }
        if let Some(t) = token {
            rb = rb.header("x-otto-token", t);
        }
        let status = rb.send().await.unwrap().status();
        assert_eq!(status, 204, "{label}: refused silently");
    }
    assert!(f
        .ctx
        .swarm_repo
        .list_board(&swarm.id, None, None, 50)
        .await
        .unwrap()
        .is_empty());
    f.server.abort();
}

#[tokio::test]
async fn plugin_host_credentials_need_an_enabled_plugins_bearer_token() {
    let (f, _root) = fixture().await;
    let repo = f.ctx.plugins.repo();
    repo.upsert(NewPlugin {
        slug: "fixture".into(),
        name: "Fixture".into(),
        icon: "box".into(),
        version: "1".into(),
        description: String::new(),
        source: f._tmp.path().to_string_lossy().into(),
        exec: vec!["/bin/false".into()],
        ui_dir: None,
        health: "/health".into(),
        token: "plugin-token".into(),
    })
    .await
    .unwrap();
    let url = format!("{}/plugin-host/jira/credentials?account=nope", f.origin);
    let send = |auth: Option<&'static str>| {
        let mut rb = f.client.get(&url);
        if let Some(a) = auth {
            rb = rb.header("authorization", a);
        }
        rb.send()
    };
    assert_eq!(send(None).await.unwrap().status(), 401, "no Authorization");
    assert_eq!(
        send(Some("Bearer ")).await.unwrap().status(),
        401,
        "empty bearer"
    );
    assert_eq!(
        send(Some("Bearer plugin-token")).await.unwrap().status(),
        401,
        "a disabled plugin's token"
    );
    assert_eq!(
        send(Some("Bearer other")).await.unwrap().status(),
        401,
        "an unknown token"
    );
    repo.set_enabled("fixture", true).await.unwrap();
    assert_ne!(
        send(Some("Bearer plugin-token")).await.unwrap().status(),
        401,
        "the enabled plugin's token passes the gate"
    );
    f.server.abort();
}
