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
use otto_state::{
    NewAgent, NewConnection, NewDiscoveryRun, NewPlugin, NewProject, NewSwarm, UsersRepo,
};
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

/// What the ingest routes can write, read straight from the stores: board
/// rows, product stories, design attachments, the discovery report, the
/// session's activity trail and whether usage was recorded at all.
#[derive(Debug, PartialEq)]
struct Written {
    board: usize,
    stories: i64,
    attachments: i64,
    report: Option<String>,
    trail: usize,
    usage_recorded: bool,
}

async fn written(
    f: &Fixture,
    swarm: &otto_core::Id,
    run: &otto_core::Id,
    sid: &otto_core::Id,
) -> Written {
    // Fixed statements (sqlx 0.9 refuses an unaudited `format!` query).
    let count = |sql: &'static str| {
        let pool = f.ctx.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    Written {
        board: f
            .ctx
            .swarm_repo
            .list_board(swarm, None, None, 50)
            .await
            .unwrap()
            .len(),
        stories: count("SELECT COUNT(*) FROM product_stories").await,
        attachments: count("SELECT COUNT(*) FROM product_attachments").await,
        report: f
            .ctx
            .discovery_repo
            .get(run)
            .await
            .unwrap()
            .unwrap()
            .report_md,
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
/// hook token. The session is a REAL swarm agent session (swarm, agent,
/// project and a discovery run all exist), so once the token check passes
/// each handler has everything it needs to write — the 204s alone prove
/// nothing (accepted and refused posts both return 204). Each route is
/// refused with no token, an empty token and ANOTHER live session's token,
/// then accepted with the session's own token: the positive case is what
/// makes the negatives count.
#[tokio::test]
async fn ingest_routes_write_only_with_the_sessions_own_token() {
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
    let agent = f
        .ctx
        .swarm_repo
        .create_agent(NewAgent {
            swarm_id: swarm.id.clone(),
            workspace_id: ws.id.clone(),
            name: "po".into(),
            title: "Product owner".into(),
            reports_to: None,
            provider: "claude".into(),
            model: None,
            soul_name: None,
            soul_md: None,
            specialization: String::new(),
            scope_md: String::new(),
            skills: json!([]),
            schedule: None,
            cwd_mode: None,
            avatar: String::new(),
            order_idx: 0,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let project = f
        .ctx
        .swarm_repo
        .create_project(NewProject {
            swarm_id: swarm.id.clone(),
            workspace_id: ws.id.clone(),
            name: "p".into(),
            description: String::new(),
            repo_path: None,
            goal_md: None,
            story_id: None,
            order_idx: 0,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let story = f
        .ctx
        .product
        .create_draft(&ws.id, &root.id, Some("Under discovery"))
        .await
        .unwrap()
        .story;
    let run = f
        .ctx
        .discovery_repo
        .create(NewDiscoveryRun {
            story_id: story.id.clone(),
            workspace_id: ws.id.clone(),
            swarm_id: swarm.id.clone(),
            project_id: project.id.clone(),
            brief_md: String::new(),
            created_by: root.id.clone(),
        })
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
        .create(session(
            "po",
            json!({
                "swarm_id": swarm.id,
                "agent_id": agent.id,
                "workspace_id": ws.id,
                "project_id": project.id,
            }),
        ))
        .await
        .unwrap();
    let other = sessions.create(session("other", json!({}))).await.unwrap();
    // Minted the way a spawn mints them (the `OTTO_INGEST_TOKEN` env).
    let own_token = f.ctx.manager.mint_ingest_token(&own.id);
    let other_token = f.ctx.manager.mint_ingest_token(&other.id);
    assert_ne!(own_token, other_token);

    let routes = [
        ("swarm/board", json!({"body": "I am the leader now"})),
        (
            "swarm/product",
            json!({"title": "Plan", "body_md": "# forged"}),
        ),
        (
            "swarm/mockup",
            json!({"title": "Screen", "content": "<p>x</p>"}),
        ),
        (
            "swarm/discovery-report",
            json!({"report_md": "forged report"}),
        ),
        (
            "usage",
            json!({"model": "claude-sonnet-4", "input_tokens": 10}),
        ),
        ("claude", json!({"hook_event_name": "SessionStart"})),
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
    let before = written(&f, &swarm.id, &run.id, &own.id).await;
    assert_eq!(
        before,
        Written {
            board: 0,
            stories: 1,
            attachments: 0,
            report: None,
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
                written(&f, &swarm.id, &run.id, &own.id).await,
                before,
                "{route} with {label} wrote something"
            );
        }
    }

    // The session's own token: every route writes.
    for (route, body) in &routes {
        let status = post(route, body, Some(own_token.as_str()))
            .await
            .unwrap()
            .status();
        assert_eq!(status, 204, "{route} with the session's own token");
    }
    let after = written(&f, &swarm.id, &run.id, &own.id).await;
    assert_eq!(after.board, 1, "exactly one board row");
    assert!(after.stories > before.stories, "a product draft was filed");
    assert_eq!(after.attachments, 1, "one mockup attachment");
    assert_eq!(after.report.as_deref(), Some("forged report"));
    assert!(after.trail > 0, "the hook landed in the activity trail");
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
