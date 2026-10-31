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
//! The `/ingest/*` hook routes are driven with a real swarm agent session and
//! its minted token, so the token check is the only thing between a forged
//! post and a write (test-quality-6w1-02).
//!
//! The Telegram sender allow-list is pinned in `otto-channels`
//! (`a_strangers_text_starts_no_session_and_is_listed_rejected`), and
//! `verify_ingest_token` by its own unit test in `otto-sessions`.
use otto_core::domain::{Channel, ConnectionKind, Environment};
use otto_server::ServerCtx;
use otto_state::{NewConnection, NewPlugin, UsersRepo};
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

    let channel = |ws: &str| format!("{}/webhooks/{ws}", f.origin);
    let body = json!({"text": "do it"});

    for (label, url_on, url_off) in [("channel", channel(&ws), channel(&off))] {
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

    // The right key passes the gate: the route reaches its next check.
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

    // Nothing was started anywhere.
    for w in [&ws, &off] {
        assert!(f.ctx.manager.list_by_workspace(w).await.unwrap().is_empty());
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

/// What the ingest routes can write, read straight from the stores: the
/// session's activity trail and whether usage was recorded at all.
#[derive(Debug, PartialEq)]
struct Written {
    trail: usize,
    usage_recorded: bool,
}

async fn written(f: &Fixture, sid: &otto_core::Id) -> Written {
    Written {
        trail: otto_state::ActivityRepo::new(f.ctx.pool.clone())
            .list_trail(sid, 50)
            .await
            .unwrap()
            .len(),
        // Nothing else records usage in this fixture; `record` stamps the
        // freshness clock even with the ClickHouse writer off.
        usage_recorded: f.ctx.usage.sampler_wanted(0),
    }
}

/// test-quality-6w1-01/-02: every `/ingest/*` route that takes a session's
/// hook token. Each route is refused with no token, an empty token and ANOTHER
/// live session's token, then accepted with the session's own token: the
/// positive case is what makes the negatives count.
#[tokio::test]
async fn ingest_routes_write_only_with_the_sessions_own_token() {
    let (f, root) = fixture().await;
    let ws = f
        .ctx
        .workspaces
        .create("A", f._tmp.path().to_str().unwrap(), &root.id)
        .await
        .unwrap();
    let sessions = otto_state::SessionsRepo::new(f.ctx.pool.clone());
    let session = |title: &str, meta: serde_json::Value| otto_state::NewSession {
        workspace_id: ws.id.clone(),
        kind: otto_core::domain::SessionKind::Agent,
        provider: "claude".into(),
        title: title.into(),
        cwd: f._tmp.path().to_string_lossy().to_string(),
        provider_session_id: None,
        connection_id: None,
        created_by: root.id.clone(),
        meta,
    };
    let own = sessions
        .create(session("po", json!({"workspace_id": ws.id})))
        .await
        .unwrap();
    let other = sessions.create(session("other", json!({}))).await.unwrap();
    // Minted the way a spawn mints them (the `OTTO_INGEST_TOKEN` env).
    let own_token = f.ctx.manager.mint_ingest_token(&own.id);
    let other_token = f.ctx.manager.mint_ingest_token(&other.id);
    assert_ne!(own_token, other_token);

    let routes = [
        (
            "usage",
            json!({"model": "claude-sonnet-4", "input_tokens": 10}),
        ),
        ("claude", json!({"hook_event_name": "SessionStart"})),
        ("codex", json!({"type": "agent-turn-complete"})),
    ];
    let post = |route: &str, body: &serde_json::Value, token: Option<&str>| {
        let mut rb = f
            .client
            .post(format!("{}/ingest/{route}", f.origin))
            .header("x-otto-session", own.id.as_str())
            .json(body);
        if let Some(t) = token {
            rb = rb.header("x-otto-token", t);
        }
        rb.send()
    };
    let before = written(&f, &own.id).await;
    assert_eq!(
        before,
        Written {
            trail: 0,
            usage_recorded: false,
        }
    );
    for (route, body) in &routes {
        for (label, token) in [
            ("no token", None),
            ("an empty token", Some("")),
            ("another session's token", Some(other_token.as_str())),
        ] {
            let status = post(route, body, token).await.unwrap().status();
            assert_eq!(status, 204, "{route} with {label}");
            assert_eq!(
                written(&f, &own.id).await,
                before,
                "{route} with {label} wrote something"
            );
        }
    }

    // The session's own token: every route writes. The two hook routes both
    // write only to the trail, so each is pinned on its own: the trail grows
    // by exactly one line per accepted post (test-quality-6w2-01). A shared
    // `trail > 0` would be met by either route alone.
    for (route, body) in &routes {
        let trail_before = written(&f, &own.id).await.trail;
        let status = post(route, body, Some(own_token.as_str()))
            .await
            .unwrap()
            .status();
        assert_eq!(status, 204, "{route} with the session's own token");
        if matches!(*route, "claude" | "codex") {
            assert_eq!(
                written(&f, &own.id).await.trail,
                trail_before + 1,
                "{route} with the session's own token appends one trail line"
            );
        }
    }
    let after = written(&f, &own.id).await;
    assert_eq!(after.trail, 2, "one trail line per hook route");
    assert!(after.usage_recorded, "usage was recorded");
    f.server.abort();
}

/// test-quality-6-01/6w1-01: all four `/plugin-host/*` routes authenticate
/// the sidecar with `auth_plugin` before doing anything. `/agents/run` is the
/// dangerous one (a headless agent CLI with the caller's prompt and cwd); its
/// body points at a missing cwd, so a request that slipped past the gate would
/// fail at the spawn (a 500), never run an agent, and still fail this test.
#[tokio::test]
async fn plugin_host_routes_need_an_enabled_plugins_bearer_token() {
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
    let missing_cwd = f._tmp.path().join("no-such-dir");
    let run_body = json!({"prompt": "x", "cwd": missing_cwd.to_string_lossy()});
    let routes = [
        "repos",
        "jira/accounts",
        "jira/credentials?account=nope",
        "agents/run",
    ];
    let send = |route: &'static str, auth: Option<&'static str>| {
        let url = format!("{}/plugin-host/{route}", f.origin);
        let mut rb = if route == "agents/run" {
            f.client.post(url).json(&run_body)
        } else {
            f.client.get(url)
        };
        if let Some(a) = auth {
            rb = rb.header("authorization", a);
        }
        rb.send()
    };
    for route in routes {
        for (label, auth) in [
            ("no Authorization", None),
            ("an empty bearer", Some("Bearer ")),
            ("a disabled plugin's token", Some("Bearer plugin-token")),
            ("an unknown token", Some("Bearer other")),
        ] {
            let status = send(route, auth).await.unwrap().status();
            assert_eq!(status, 401, "{route} with {label}");
        }
    }
    assert!(
        otto_state::SessionsRepo::new(f.ctx.pool.clone())
            .list_all()
            .await
            .unwrap()
            .is_empty(),
        "no session was created"
    );
    repo.set_enabled("fixture", true).await.unwrap();
    // The enabled plugin's token passes the gate on the read routes (the
    // agent route is not run for real).
    for route in routes.iter().take(3).copied() {
        assert_ne!(
            send(route, Some("Bearer plugin-token"))
                .await
                .unwrap()
                .status(),
            401,
            "{route}: the enabled plugin's token passes the gate"
        );
    }
    f.server.abort();
}
