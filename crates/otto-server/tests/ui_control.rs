//! Agent UI control — end-to-end through the REAL daemon router on a real
//! loopback port: the production auth middleware + feature guard, real API /
//! session tokens, `/ws/events` sockets playing the Otto windows, and the
//! governed `POST /mcp/otto-tools/invoke` playing the agent.
//!
//! What it pins (the daemon is the security boundary):
//! - an agent-session socket can never register as a window;
//! - an ungranted call raises `ui_control_requested` and waits; the agent
//!   cannot grant itself (route, PATCH, or a laundered PAT);
//! - the auto-open → side pane → dispatch path, and result auth (user +
//!   connection, human credential only);
//! - result shaping (row cap + redaction), revoke → cancel, the quiet period,
//!   the headless fallback, and `no_ui_client`.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_rbac::{AuthRepo, RbacRoleChecker};
use otto_server::ServerCtx;
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{
    ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, GitStore, IntegrationsRepo,
    IssuesRepo, NewSession, ProductRepo, ReviewsRepo, SessionsRepo, SkillEvalsRepo, SqlitePool,
    SwarmRepo, WorkspacesRepo,
};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ---------------------------------------------------------------------------
// Harness (mirrors canvas_refs_api.rs, with a real base_url + listener)
// ---------------------------------------------------------------------------

struct NoopSecrets;
impl SecretStore for NoopSecrets {
    fn put(&self, _key: &str, _value: &str) -> Result<()> {
        Err(Error::Internal("noop secrets".into()))
    }
    fn get(&self, _key: &str) -> Result<Option<String>> {
        Err(Error::Internal("noop secrets".into()))
    }
    fn delete(&self, _key: &str) -> Result<()> {
        Err(Error::Internal("noop secrets".into()))
    }
}

struct NoopSpawner;
impl otto_connections::Spawner for NoopSpawner {
    fn spawn_connection<'a>(
        &'a self,
        _ws_id: &'a Id,
        _user_id: &'a Id,
        _conn: &'a otto_core::domain::Connection,
        _spec: otto_pty::CommandSpec,
        _first_command: Option<String>,
        _title: Option<String>,
    ) -> otto_core::auth::BoxFuture<'a, Result<otto_core::domain::Session>> {
        Box::pin(async { Err(Error::Internal("noop spawner".into())) })
    }
}

async fn file_pool(dir: &std::path::Path) -> SqlitePool {
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
    pool
}

async fn seed_user(pool: &SqlitePool, id: &str, is_root: bool) {
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

async fn seed_workspace(pool: &SqlitePool, ws_id: &str, admin: &str) {
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES (?, 'ws', '/tmp', '{}', 0, ?)",
    )
    .bind(ws_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed workspace");
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, 'admin')")
        .bind(ws_id)
        .bind(admin)
        .execute(pool)
        .await
        .expect("member");
}

async fn test_ctx(pool: &SqlitePool, base_url: String, tmp: &std::path::Path) -> ServerCtx {
    let (events, _rx) = broadcast::channel(256);
    let secrets: Arc<dyn SecretStore> = Arc::new(NoopSecrets);
    let roles = Arc::new(RbacRoleChecker::new(pool.clone()));
    let manager = Arc::new(SessionManager::new(
        SessionsRepo::new(pool.clone()),
        events.clone(),
        ProviderRegistry::new(None),
    ));
    let orchestrator = Arc::new(otto_orchestrator::Orchestrator::new("claude"));
    let improve_engine = Arc::new(otto_improve::ImprovementEngine {
        improvements: otto_state::ImprovementsRepo::new(pool.clone()),
        sessions: SessionsRepo::new(pool.clone()),
        workspaces: WorkspacesRepo::new(pool.clone()),
        producer: Arc::new(otto_improve::RealProposalProducer::new(orchestrator.clone())),
        events: events.clone(),
        library_root: tmp.join("lib"),
    });
    let connections = Arc::new(otto_connections::ConnectionsService::new(
        ConnectionsRepo::new(pool.clone()),
        ConnectionSectionsRepo::new(pool.clone()),
        secrets.clone(),
    ));
    let db_explorer = Arc::new(otto_dbviewer::DbViewerService::new(
        ConnectionsRepo::new(pool.clone()),
        secrets.clone(),
        DbExplorerRepo::new(pool.clone()),
    ));
    let brokers = Arc::new(otto_brokers::BrokersService::new(
        otto_state::BrokerClustersRepo::new(pool.clone()),
        secrets.clone(),
        None,
    ));
    let mcp = Arc::new(otto_mcp::McpService::new(pool.clone(), secrets.clone()));
    let swarm_repo = SwarmRepo::new(pool.clone());
    let swarm = Arc::new(otto_swarm::SwarmService::new(swarm_repo.clone()));
    let product_repo = ProductRepo::new(pool.clone());
    let product = Arc::new(otto_product::ProductService::new(
        product_repo.clone(),
        IssuesRepo::new(pool.clone()),
        secrets.clone(),
    ));
    let usage =
        otto_usage::UsageEngine::start(otto_usage::UsageConfig::default(), tmp.join("usage")).await;
    ServerCtx {
        pool: pool.clone(),
        secrets,
        events: events.clone(),
        authenticator: Arc::new(otto_rbac::RbacAuthenticator::new(pool.clone())),
        roles,
        auth_cache: otto_rbac::AuthCache::new(),
        version: "test".into(),
        base_url,
        data_dir: tmp.to_path_buf(),
        plugins: Arc::new(otto_server::plugins::PluginManager::new(
            otto_state::PluginsRepo::new(pool.clone()),
            tmp.join("plugins"),
            tmp.to_path_buf(),
            "http://127.0.0.1:7700/api/v1/plugin-host".into(),
        )),
        manager,
        workspaces: WorkspacesRepo::new(pool.clone()),
        connections,
        db_explorer,
        db_assist: otto_server::db_assist::new_registry(),
        transcript_cache: Default::default(),
        brokers,
        mcp,
        spawner: Arc::new(NoopSpawner),
        git_store: GitStore::new(pool.clone()),
        issues_store: IssuesRepo::new(pool.clone()),
        integrations_store: IntegrationsRepo::new(pool.clone()),
        channel_bridge: None,
        wf_skip_current: Default::default(),
        reviews_store: ReviewsRepo::new(pool.clone()),
        findings_store: otto_state::ReviewFindingsRepo::new(pool.clone()),
        finding_events_store: otto_state::FindingEventsRepo::new(pool.clone()),
        repo_rules_store: otto_state::RepoRulesRepo::new(pool.clone()),
        proof_packs_store: otto_state::ReviewProofPacksRepo::new(pool.clone()),
        skill_evals_store: SkillEvalsRepo::new(pool.clone()),
        golden_tasks_store: otto_state::GoldenTasksRepo::new(pool.clone()),
        eval_matrices_store: otto_state::EvalMatricesRepo::new(pool.clone()),
        skill_eval_cancels: Default::default(),
        skill_reviews_store: otto_state::SkillReviewsRepo::new(pool.clone()),
        skill_review_cancels: Default::default(),
        review_agent_cancels: Default::default(),
        review_cancels: Default::default(),
        orchestrator,
        improve_engine,
        context_library: otto_context::Library::new(tmp.join("ctx")),
        usage,
        product,
        product_repo,
        attachment_repo: otto_state::ProductAttachmentRepo::new(pool.clone()),
        discovery_repo: otto_state::ProductDiscoveryRepo::new(pool.clone()),
        refinement_repo: otto_state::ProductRefinementRepo::new(pool.clone()),
        mockup_repo: otto_state::ProductMockupRepo::new(pool.clone()),
        discovery_chat_repo: otto_state::DiscoveryChatRepo::new(pool.clone()),
        canvas_repo: otto_state::CanvasRepo::new(pool.clone()),
        product_agent_cancels: otto_server::product_run::new_cancel_registry(),
        design_jobs: otto_server::design_blender::new_job_registry(),
        memory: Arc::new(otto_memory::MemoryService::with_defaults(pool.clone())),
        vault: Arc::new(otto_vault::VaultEngine::new(pool.clone())),
        vault_docs_runs: otto_server::vault_docs_agent::new_run_registry(),
        vault_docs_refine: otto_server::vault_docs_agent::new_refine_registry(),
        swarm,
        swarm_repo,
        swarm_coords: otto_server::swarm_runtime::new_registry(),
        swarm_run_cancels: otto_server::swarm_run::new_cancel_registry(),
        goal_loops_repo: otto_state::GoalLoopsRepo::new(pool.clone()),
        goal_loops: otto_server::goal_loop::new_registry(),
        workgraph: Arc::new(otto_workgraph::WorkGraphService::new(
            otto_state::WorkGraphRepo::new(pool.clone()),
            events.clone(),
        )),
        scheduled_tasks: otto_state::ScheduledTasksRepo::new(pool.clone()),
        proof_repo: otto_state::ProofRepo::new(pool.clone()),
        proof_locks: otto_server::proof::new_locks(),
        runs: otto_state::RunsRepo::new(pool.clone()),
        runs_engine: otto_server::run_engine::RunEngine::new(),
        browser_tabs: otto_state::BrowserTabsRepo::new(pool.clone()),
        browser_annotations: otto_state::BrowserAnnotationsRepo::new(pool.clone()),
        browser_credentials: otto_state::BrowserCredentialsRepo::new(pool.clone()),
        ui_bridge: Default::default(),
        browser: Arc::new(otto_server::routes::browser::BrowserEngineHandle::new(
            None,
            tmp.join("browser"),
        )),
    }
}

struct Daemon {
    base: String,
    ws_base: String,
    human: String,
    agent: String,
    bob: String,
    sid: Id,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

/// Boot a real daemon router: alice (root) owns session `sid` started on
/// device `dev1`; `human` is alice's own API token, `agent` the session's
/// managed token, `bob` another (non-root) user's token.
async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let pool = file_pool(tmp.path()).await;
    seed_user(&pool, "alice", true).await;
    seed_user(&pool, "bob", false).await;
    seed_workspace(&pool, "ws1", "alice").await;
    let s = SessionsRepo::new(pool.clone())
        .create(NewSession {
            workspace_id: "ws1".into(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "claude".into(),
            title: "Fix the report".into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "alice".into(),
            meta: json!({"client_id": "dev1"}),
        })
        .await
        .unwrap();
    let repo = AuthRepo::new(pool.clone());
    let (human, _) = repo.issue_api_token(&"alice".to_string(), Some("ui")).await.unwrap();
    let (agent, _) = repo.issue_session_api_token(&"alice".to_string(), &s.id).await.unwrap();
    let (bob, _) = repo.issue_api_token(&"bob".to_string(), Some("ui")).await.unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let ctx = test_ctx(&pool, base.clone(), tmp.path()).await;
    otto_server::ui_bridge::spawn_session_watch(ctx.clone());
    // The production module composition (sessions, connections, dbviewer, …).
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Daemon {
        ws_base: format!("ws://{addr}"),
        base,
        human,
        agent,
        bob,
        sid: s.id,
        http: reqwest::Client::new(),
        _tmp: tmp,
    }
}

impl Daemon {
    async fn ws(&self, token: &str) -> Ws {
        let (ws, _) = tokio_tungstenite::connect_async(format!("{}/ws/events?token={token}", self.ws_base))
            .await
            .expect("ws connect");
        ws
    }

    async fn post(&self, token: &str, path: &str, body: Value, conn: Option<&str>) -> (u16, Value) {
        let mut req = self
            .http
            .post(format!("{}/api/v1{path}", self.base))
            .bearer_auth(token)
            .json(&body);
        if let Some(c) = conn {
            req = req.header("X-Otto-Ui-Conn", c);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn patch(&self, token: &str, path: &str, body: Value) -> u16 {
        self.http
            .patch(format!("{}/api/v1{path}", self.base))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    /// The agent's governed call (what stdio `otto_ui_*` proxies to).
    fn invoke(&self, tool: &str, args: Value) -> tokio::task::JoinHandle<Value> {
        let http = self.http.clone();
        let url = format!("{}/api/v1/mcp/otto-tools/invoke", self.base);
        let token = self.agent.clone();
        let tool = format!("otto.{tool}");
        tokio::spawn(async move {
            http.post(url)
                .bearer_auth(token)
                .json(&json!({"tool": tool, "arguments": args}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        })
    }

    async fn grant(&self, enabled: bool) {
        let (st, body) = self
            .post(&self.human, &format!("/sessions/{}/ui-control", self.sid), json!({"enabled": enabled}), None)
            .await;
        assert_eq!(st, 200, "grant({enabled}): {body}");
        assert_eq!(body["meta"]["ui_control"]["enabled"], json!(enabled));
    }
}

/// Next JSON text frame whose `type` is `ty` (skipping others), or panic.
async fn next_of(ws: &mut Ws, ty: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(&t).unwrap();
                    if v["type"] == ty {
                        return v;
                    }
                }
                Some(Ok(_)) => {}
                other => panic!("socket ended waiting for {ty}: {other:?}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {ty}"))
}

/// True when a frame of type `ty` arrives within `wait`.
async fn gets(ws: &mut Ws, ty: &str, wait: Duration) -> bool {
    tokio::time::timeout(wait, next_of(ws, ty)).await.is_ok()
}

async fn hello(ws: &mut Ws, window: &str, pane: &str, module: &str, caps: &[&str]) -> String {
    let host = if pane == "side" { json!(window) } else { Value::Null };
    ws.send(Message::Text(
        json!({"type":"hello","client_id":"dev1","window_id":window,"pane":pane,
               "host_window_id":host,"route":module,"module":module,
               "focused":true,"visible":true,"capabilities":caps})
        .to_string(),
    ))
    .await
    .unwrap();
    next_of(ws, "hello_ack").await["conn_id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// The flow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn agent_drives_the_window_end_to_end() {
    let d = boot().await;

    // The catalog is served.
    let cat: Value = d
        .http
        .get(format!("{}/api/v1/ui/commands/catalog", d.base))
        .bearer_auth(&d.human)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cat["version"], 1);
    assert!(cat["commands"].as_array().unwrap().iter().any(|c| c["name"] == "db_run_query"));

    // An agent-session socket may connect (it gets events) but never becomes a window.
    let mut rogue = d.ws(&d.agent).await;
    rogue
        .send(Message::Text(
            json!({"type":"hello","client_id":"dev1","window_id":"evil","pane":"main","route":"database",
                   "module":"connections","focused":true,"visible":true,"capabilities":["db_run_query","open"]})
            .to_string(),
        ))
        .await
        .unwrap();
    assert!(!gets(&mut rogue, "hello_ack", Duration::from_millis(500)).await, "agent socket registered");

    // The real main window (showing Agents).
    let mut main = d.ws(&d.human).await;
    let main_conn = hello(&mut main, "w1", "main", "agents", &["open", "state", "focus"]).await;

    // Ungranted call → the owner is asked; the agent cannot grant itself.
    let call = d.invoke("ui_db_run_query", json!({"tab_id": "t1", "statement": "select 1"}));
    let ask = next_of(&mut main, "ui_control_requested").await;
    assert_eq!(ask["session_id"], json!(d.sid));
    assert_eq!(ask["session_title"], "Fix the report");
    assert_eq!(ask["module"], "connections");
    assert_eq!(ask["command"], "db_run_query");
    let (st, _) = d
        .post(&d.agent, &format!("/sessions/{}/ui-control", d.sid), json!({"enabled": true}), None)
        .await;
    assert_eq!(st, 403, "agent granted itself");
    assert_eq!(
        d.patch(&d.agent, &format!("/sessions/{}", d.sid), json!({"meta":{"ui_control":{"enabled":true}}})).await,
        403,
        "PATCH set the grant"
    );
    assert_eq!(
        d.patch(&d.agent, &format!("/sessions/{}", d.sid), json!({"meta":{"client_id":"other-device"}})).await,
        403,
        "PATCH moved the device"
    );
    let (st, _) = d.post(&d.agent, "/auth/tokens", json!({"label": "escape"}), None).await;
    assert_eq!(st, 403, "agent minted a human PAT");
    // Another user cannot grant it either.
    let (st, _) = d
        .post(&d.bob, &format!("/sessions/{}/ui-control", d.sid), json!({"enabled": true}), None)
        .await;
    assert_eq!(st, 403);
    d.grant(true).await;

    // Nothing shows Connections → the daemon opens it in the side pane first.
    let open = next_of(&mut main, "ui_command").await;
    assert_eq!(open["command"], "open");
    assert_eq!(open["args"], json!({"module":"connections","route":"database","placement":"side"}));
    assert_eq!(open["agent"]["title"], "Fix the report");
    assert_eq!(open["agent"]["provider"], "claude");
    let mut side = d.ws(&d.human).await;
    let side_conn = hello(&mut side, "w1", "side", "connections", &["db_run_query"]).await;
    let (st, _) = d
        .post(&d.human, &format!("/ui/commands/{}/result", open["id"].as_str().unwrap()),
              json!({"ok": true, "result": {"pane": "side"}}), Some(&main_conn))
        .await;
    assert_eq!(st, 204);

    // The command reaches the side pane — and only a human, on that
    // connection, as the owner, can answer it.
    let cmd = next_of(&mut side, "ui_command").await;
    assert_eq!(cmd["command"], "db_run_query");
    assert_eq!(cmd["args"], json!({"tab_id":"t1","statement":"select 1"}));
    assert_eq!(cmd["session_id"], json!(d.sid));
    assert!(cmd["deadline_ms"].as_u64().unwrap() >= 1000);
    let id = cmd["id"].as_str().unwrap().to_string();
    let rows: Vec<Value> = (0..250).map(|i| json!([i])).collect();
    let reply = json!({"ok": true, "result": {"rows": rows, "statement": "select 1", "hint": "key AKIAIOSFODNN7EXAMPLE"}});
    let path = format!("/ui/commands/{id}/result");
    assert_eq!(d.post(&d.agent, &path, reply.clone(), Some(&side_conn)).await.0, 403, "agent forged a result");
    assert_eq!(d.post(&d.bob, &path, reply.clone(), Some(&side_conn)).await.0, 403, "another user answered");
    assert_eq!(d.post(&d.human, &path, reply.clone(), Some(&main_conn)).await.0, 403, "wrong window answered");
    assert_eq!(d.post(&d.human, &path, reply.clone(), None).await.0, 403, "no connection header");
    assert_eq!(d.post(&d.human, &path, reply.clone(), Some(&side_conn)).await.0, 204);
    assert_eq!(d.post(&d.human, &path, reply, Some(&side_conn)).await.0, 404, "answered twice");

    let out = call.await.unwrap();
    assert_eq!(out["decision"], "allowed", "{out}");
    let content = &out["content"];
    assert_eq!(content["ui_visible"], true);
    assert_eq!(content["rows"].as_array().unwrap().len(), 200);
    assert_eq!(content["rows_truncated"], 250);
    assert!(!content["hint"].as_str().unwrap().contains("AKIAIOSFODNN7EXAMPLE"), "{content}");

    // Now Connections shows → direct dispatch; a human-confirm progress + a
    // Cancel reach the agent as `cancelled_by_user`.
    let call = d.invoke("ui_db_run_query", json!({"tab_id": "t1", "statement": "delete from t"}));
    let cmd = next_of(&mut side, "ui_command").await;
    let id = cmd["id"].as_str().unwrap().to_string();
    let (st, _) = d
        .post(&d.human, &format!("/ui/commands/{id}/progress"), json!({"note":"confirm write","awaiting_human":true}), Some(&side_conn))
        .await;
    assert_eq!(st, 204);
    assert_eq!(
        d.post(&d.agent, &format!("/ui/commands/{id}/progress"), json!({"note":"x"}), Some(&side_conn)).await.0,
        403
    );
    let (st, _) = d
        .post(&d.human, &format!("/ui/commands/{id}/result"),
              json!({"ok": false, "error": {"code": "cancelled_by_user", "message": "Cancelled"}}), Some(&side_conn))
        .await;
    assert_eq!(st, 204);
    let out = call.await.unwrap();
    assert_eq!(out["decision"], "error");
    assert_eq!(out["code"], "cancelled_by_user");
    assert_eq!(out["executed"], true);

    // Bad arguments never leave the daemon.
    let out = d.invoke("ui_db_run_query", json!({"sql": "x"})).await.unwrap();
    assert_eq!(out["code"], "invalid_args", "{out}");
    assert!(!gets(&mut side, "ui_command", Duration::from_millis(300)).await);

    // Stop (revoke) cancels the in-flight command at once.
    let call = d.invoke("ui_db_run_query", json!({"tab_id": "t1"}));
    let cmd = next_of(&mut side, "ui_command").await;
    d.grant(false).await;
    let cancel = next_of(&mut side, "ui_command_cancel").await;
    assert_eq!(cancel["id"], cmd["id"]);
    assert_eq!(cancel["reason"], "revoked");
    let out = call.await.unwrap();
    assert_eq!(out["code"], "cancelled_by_user", "{out}");

    // Right after a Stop: pending_grant at once, without asking again.
    let started = std::time::Instant::now();
    let out = d.invoke("ui_db_run_query", json!({"tab_id": "t1"})).await.unwrap();
    assert_eq!(out["code"], "pending_grant", "{out}");
    assert_eq!(out["executed"], false);
    assert!(out["content"]["error"].as_str().unwrap().contains("hasn't allowed UI control"));
    assert!(started.elapsed() < Duration::from_secs(5), "quiet period re-asked / waited");
    assert!(!gets(&mut main, "ui_control_requested", Duration::from_millis(300)).await);
    // …and the user can re-grant any time.
    d.grant(true).await;

    // An unchanged round-trip of the server-owned keys is accepted and dropped.
    assert_eq!(
        d.patch(&d.human, &format!("/sessions/{}", d.sid), json!({"meta":{"client_id":"dev1","note":"x"}})).await,
        200
    );

    // Every window closes: reads fall back headless, the rest is no_ui_client.
    drop(main);
    drop(side);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let out = d.invoke("ui_state", json!({})).await.unwrap();
    assert_eq!(out["decision"], "allowed", "{out}");
    assert_eq!(out["content"]["ui_visible"], false);
    assert!(out["content"]["documents"].as_array().unwrap().is_empty());
    let out = d.invoke("ui_focus", json!({})).await.unwrap();
    assert_eq!(out["code"], "no_ui_client", "{out}");
    let out = d.invoke("ui_db_run_query", json!({"tab_id": "t1"})).await.unwrap();
    assert_eq!(out["code"], "no_ui_client", "{out}");
    let out = d.invoke("ui_db_list_connections", json!({})).await.unwrap();
    assert_eq!(out["content"]["ui_visible"], false, "{out}");
    assert!(out["content"]["items"].is_array(), "{out}");
}

/// A person's own token (no session) cannot drive a UI; neither can another
/// user's session reach this user's windows.
#[tokio::test]
async fn only_session_credentials_drive_and_only_their_owners_windows() {
    let d = boot().await;
    let out: Value = d
        .http
        .post(format!("{}/api/v1/mcp/otto-tools/invoke", d.base))
        .bearer_auth(&d.human)
        .json(&json!({"tool": "otto.ui_state", "arguments": {}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // A plain token hits the outward master switch first (off by default).
    assert_eq!(out["decision"], "denied", "{out}");

    // Bob's window never receives alice's session's commands.
    let mut bobs = d.ws(&d.bob).await;
    let _ = hello(&mut bobs, "wb", "main", "connections", &["db_run_query", "open", "state"]).await;
    d.grant(true).await;
    let out = d.invoke("ui_db_run_query", json!({"tab_id": "t"})).await.unwrap();
    assert_eq!(out["code"], "no_ui_client", "{out}");
    assert!(!gets(&mut bobs, "ui_command", Duration::from_millis(300)).await);
}
