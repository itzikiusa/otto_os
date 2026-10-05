//! Personal-agent permission policy — end-to-end through the REAL daemon
//! router (auth middleware + feature guard + the governed
//! `POST /mcp/otto-tools/invoke`), playing agent sessions exactly as
//! `ottod mcp-tools` presents them.
//!
//! What it pins:
//! - a READ-ONLY session (proactive / read-only schedule) is denied every
//!   mutating governed tool even when an auto-approve rule covers it, still
//!   reads, and its own token is refused non-GET feature routes directly;
//! - the SENSITIVE-ACTION gate files an approval even when an auto-approve
//!   rule covers the tool;
//! - an agent's enforceable custom rule forces an approval for matching calls
//!   only.
//!
//! Harness copied from `mcp_auto_approve.rs` (kept separate so the two suites
//! evolve independently).

use std::sync::Arc;

use chrono::Utc;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_rbac::{AuthRepo, RbacRoleChecker};
use otto_server::ServerCtx;
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{
    AgentAutonomy, AgentRule, ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, DbPool,
    GitStore, IntegrationsRepo, IssuesRepo, NewPersonalAgent, NewSession, PersonalAgentsRepo,
    ProductRepo, ReviewsRepo, SessionsRepo, SkillEvalsRepo, SwarmRepo, WorkspacesRepo,
};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Harness (mirrors ui_control.rs: a real base_url + listener)
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

async fn seed_workspace(pool: &DbPool, ws_id: &str, admin: &str) {
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES (?, 'ws', '/tmp', '{}', 0, ?)",
    )
    .bind(ws_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed workspace");
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, 'admin')",
    )
    .bind(ws_id)
    .bind(admin)
    .execute(pool)
    .await
    .expect("member");
}

async fn test_ctx(pool: &DbPool, base_url: String, tmp: &std::path::Path) -> ServerCtx {
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
        producer: Arc::new(otto_improve::RealProposalProducer::new(
            orchestrator.clone(),
        )),
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
    let usage = otto_usage::UsageEngine::start(
        otto_usage::UsageConfig {
            enabled: false, // This fixture does not exercise metrics or start ClickHouse.
            ..Default::default()
        },
        tmp.join("usage"),
    )
    .await;
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
        rooms: Default::default(),
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
        telemetry: None,
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
    human: String,
    http: reqwest::Client,
    pool: DbPool,
    _tmp: tempfile::TempDir,
}

impl Daemon {
    async fn send(
        &self,
        method: &str,
        token: &str,
        path: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        let url = format!("{}/api/v1{path}", self.base);
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let mut req = self.http.request(m, url).bearer_auth(token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// An agent session in ws1 with `meta`, and its managed token.
    async fn agent_session(&self, meta: Value) -> String {
        let s = SessionsRepo::new(self.pool.clone())
            .create(NewSession {
                workspace_id: "ws1".into(),
                kind: otto_core::domain::SessionKind::Agent,
                provider: "claude".into(),
                title: "Agent".into(),
                cwd: self._tmp.path().to_string_lossy().to_string(),
                provider_session_id: None,
                connection_id: None,
                created_by: "alice".into(),
                meta,
            })
            .await
            .unwrap();
        let (token, _) = AuthRepo::new(self.pool.clone())
            .issue_session_api_token(&"alice".to_string(), &s.id)
            .await
            .unwrap();
        token
    }

    async fn invoke(&self, token: &str, tool: &str, args: Value) -> Value {
        let (st, body) = self
            .send(
                "POST",
                token,
                "/mcp/otto-tools/invoke",
                Some(json!({"tool": format!("otto.{tool}"), "arguments": args})),
            )
            .await;
        assert_eq!(st, 200, "invoke {tool}: {body}");
        body
    }

    async fn rule_for(&self, tool: &str) {
        let (st, body) = self
            .send(
                "POST",
                &self.human,
                "/mcp/auto-approve",
                Some(json!({"scope": "global", "target_kind": "tool", "target": format!("otto.{tool}"),
                            "name": format!("auto {tool}")})),
            )
            .await;
        assert_eq!(st, 201, "rule {tool}: {body}");
    }

    async fn pending(&self) -> Vec<Value> {
        let (st, body) = self
            .send("GET", &self.human, "/mcp/approvals?status=pending", None)
            .await;
        assert_eq!(st, 200, "approvals: {body}");
        body.as_array().cloned().unwrap_or_default()
    }

    /// A personal agent in ws1 with these rules (enforcement server-derived).
    async fn agent_with_rules(&self, rules: &[&str]) -> String {
        let repo = PersonalAgentsRepo::new(self.pool.clone());
        let a = repo
            .create(NewPersonalAgent::defaults("ws1".into(), "Scout".into()))
            .await
            .unwrap();
        let cfg = AgentAutonomy {
            rules: rules
                .iter()
                .enumerate()
                .map(|(i, t)| AgentRule {
                    id: format!("r{i}"),
                    text: (*t).into(),
                    enforce: otto_server::personal_agent_policy::derive_enforcement(t),
                })
                .collect(),
            ..Default::default()
        };
        repo.save_autonomy(&a.id, &cfg).await.unwrap();
        a.id
    }
}

async fn boot(tools: &[&str]) -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let pool = file_pool(tmp.path()).await;
    seed_user(&pool, "alice", true).await;
    seed_workspace(&pool, "ws1", "alice").await;
    let repo_dir = tmp.path().join("demo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    sqlx::query(
        "INSERT INTO repos (id, workspace_id, name, path, remote_url, provider, created_at)
         VALUES ('repo1', 'ws1', 'demo', ?, NULL, NULL, ?)",
    )
    .bind(repo_dir.to_string_lossy().to_string())
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .expect("seed repo");
    let (human, _) = AuthRepo::new(pool.clone())
        .issue_api_token(&"alice".to_string(), Some("ui"))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let ctx = test_ctx(&pool, base.clone(), tmp.path()).await;
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let d = Daemon {
        base,
        human,
        http: reqwest::Client::new(),
        pool,
        _tmp: tmp,
    };
    let (st, body) = d
        .send(
            "PATCH",
            &d.human.clone(),
            "/mcp/otto-server",
            Some(json!({"enabled": true, "tools": tools})),
        )
        .await;
    assert_eq!(st, 200, "enable tools: {body}");
    d
}

fn pr_args(title: &str) -> Value {
    json!({"repo_id": "repo1", "title": title, "description": "Body",
           "source_branch": "fix/report", "target_branch": "main"})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_only_session_is_refused_mutations_at_the_tool_and_http_layers() {
    let d = boot(&["create_pr", "comment_pr", "list_repos", "room_post"]).await;
    // Even an operator's auto-approve rule cannot re-open a write.
    d.rule_for("create_pr").await;
    let agent = d.agent_with_rules(&[]).await;
    let token = d
        .agent_session(
            json!({"personal_agent": agent, "read_only": true, "agent_mode": "proactive"}),
        )
        .await;

    let env = d.invoke(&token, "create_pr", pr_args("Fix")).await;
    assert_eq!(env["decision"], "denied", "{env}");
    assert_eq!(env["executed"], false, "{env}");
    assert!(
        env["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("read-only"),
        "{env}"
    );
    let env = d
        .invoke(&token, "room_post", json!({"room_id": "r", "text": "hi"}))
        .await;
    assert_eq!(env["decision"], "denied", "a send is refused too: {env}");
    assert!(
        d.pending().await.is_empty(),
        "nothing may be queued for approval"
    );

    // Reads still work.
    let env = d.invoke(&token, "list_repos", json!({})).await;
    assert_ne!(env["decision"], "denied", "{env}");

    // The session token's direct (native stdio) writes are refused by the guard…
    let (st, body) = d
        .send(
            "POST",
            &token,
            "/agent-rooms/r1/messages",
            Some(json!({"text": "hi"})),
        )
        .await;
    assert_eq!(st, 403, "{body}");
    assert!(body.to_string().contains("read-only"), "{body}");
    let (st, _) = d
        .send(
            "PATCH",
            &token,
            &format!("/personal-agents/{agent}"),
            Some(json!({"name": "x"})),
        )
        .await;
    assert_eq!(st, 403);
    // …while its reads pass.
    let (st, body) = d
        .send("GET", &token, &format!("/personal-agents/{agent}"), None)
        .await;
    assert_eq!(st, 200, "{body}");

    // The live activity feed shows the blocked calls.
    let (st, act) = d
        .send(
            "GET",
            &d.human,
            &format!("/personal-agents/{agent}/activity"),
            None,
        )
        .await;
    assert_eq!(st, 200, "{act}");
    assert!(
        act["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["kind"] == "blocked" && i["tool"] == "create_pr"),
        "{act}"
    );

    // A DIRECTED session of the same agent keeps normal governance (rule ⇒ runs).
    let directed = d
        .agent_session(json!({"personal_agent": agent, "agent_mode": "directed"}))
        .await;
    let env = d.invoke(&directed, "create_pr", pr_args("Fix")).await;
    assert_eq!(env["executed"], true, "{env}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sensitive_actions_need_approval_even_with_an_auto_approve_rule() {
    let d = boot(&["test_integration", "create_pr"]).await;
    d.rule_for("test_integration").await;
    let token = d.agent_session(json!({})).await;
    let env = d
        .invoke(
            &token,
            "test_integration",
            json!({"workspace_id": "ws1", "channel": "slack"}),
        )
        .await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    assert_eq!(env["executed"], false);
    let pending = d.pending().await;
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0]["risk_label"], "sensitive", "{pending:?}");
    assert!(
        pending[0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("sensitive action"),
        "{pending:?}"
    );
    // A credential-shaped argument on an otherwise auto-approved tool is gated too.
    d.rule_for("create_pr").await;
    let mut args = pr_args("Fix");
    args["api_key"] = json!("sk-123");
    let env = d.invoke(&token, "create_pr", args).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    // …and without it the rule applies as before.
    let env = d.invoke(&token, "create_pr", pr_args("Fix")).await;
    assert_eq!(env["executed"], true, "{env}");
    // Unattended automation (a scheduled-task / workflow session) is not parked
    // on a sensitive approval: the operator's auto-approve rule decides.
    for source in ["scheduled_task", "workflow"] {
        let auto = d.agent_session(json!({ "source": source })).await;
        let mut args = pr_args("Fix");
        args["api_key"] = json!("sk-123");
        let env = d.invoke(&auto, "create_pr", args).await;
        assert_eq!(env["executed"], true, "{source}: {env}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_rule_forces_approval_only_for_matching_targets() {
    let d = boot(&["create_pr"]).await;
    d.rule_for("create_pr").await;
    let agent = d.agent_with_rules(&["Ask before touching prod"]).await;
    let token = d.agent_session(json!({"personal_agent": agent})).await;
    let env = d.invoke(&token, "create_pr", pr_args("prod hotfix")).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    let pending = d.pending().await;
    assert_eq!(pending[0]["risk_label"], "agent_rule", "{pending:?}");
    let env = d.invoke(&token, "create_pr", pr_args("docs typo")).await;
    assert_eq!(env["executed"], true, "{env}");
}

/// Perf N2/N4 budgets through the real router.
/// - `GET …/activity?after_seq=N&runs=false` is a constant number of
///   statements whatever the ring holds: approvals in ONE `IN` statement, no
///   run history, no per-item reads.
/// - The guard's read-only check and the governed call's session + autonomy
///   reads are cached: repeating them reads neither `sessions` nor
///   `personal_agent_autonomy` again.
/// - A cursor from another daemon process (wrong `epoch`, or ahead of the
///   counter) is answered in full with `reset: true`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activity_and_guard_reads_stay_within_budget() {
    let d = boot(&["create_pr", "list_repos"]).await;
    d.rule_for("create_pr").await;
    let agent = d.agent_with_rules(&["Ask before touching prod"]).await;
    let token = d.agent_session(json!({"personal_agent": agent})).await;
    let path = format!("/personal-agents/{agent}/activity");
    let probe = d.pool.statement_probe();

    // One waiting approval + one plain call.
    let env = d.invoke(&token, "create_pr", pr_args("prod 0")).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    d.invoke(&token, "list_repos", json!({})).await;
    let (st, first) = d.send("GET", &d.human, &path, None).await;
    assert_eq!(st, 200, "{first}");
    let epoch = first["epoch"].as_str().unwrap().to_string();
    assert_eq!(first["reset"], false);
    let cursor = first["seq"].as_u64().unwrap();

    let measure = |stmts: Vec<String>| {
        let approvals = stmts.iter().filter(|q| q.contains("mcp_approvals")).count();
        let runs = stmts
            .iter()
            .filter(|q| q.contains("personal_agent_runs"))
            .count();
        (stmts.len(), approvals, runs)
    };
    let incremental = format!("{path}?after_seq={cursor}&epoch={epoch}&runs=false");
    probe.reset();
    let (st, small) = d.send("GET", &d.human, &incremental, None).await;
    assert_eq!(st, 200, "{small}");
    let small_stmts = probe.take();
    let (n_small, ap_small, runs_small) = measure(small_stmts.clone());
    assert_eq!(ap_small, 1, "approvals in one statement: {small_stmts:?}");
    assert_eq!(runs_small, 1, "the running run only: {small_stmts:?}");
    assert!(small["runs"].is_null());
    assert!(small["items"].as_array().unwrap().is_empty());

    // Grow the ring: 15 more waiting approvals and 40 plain calls.
    for i in 1..=15 {
        d.invoke(&token, "create_pr", pr_args(&format!("prod {i}")))
            .await;
    }
    for _ in 0..40 {
        d.invoke(&token, "list_repos", json!({})).await;
    }
    probe.reset();
    let (st, big) = d.send("GET", &d.human, &incremental, None).await;
    assert_eq!(st, 200, "{big}");
    let big_stmts = probe.take();
    let (n_big, ap_big, runs_big) = measure(big_stmts.clone());
    assert_eq!(ap_big, 1, "still one approvals statement: {big_stmts:?}");
    assert_eq!(runs_big, 1, "{big_stmts:?}");
    assert_eq!(
        n_big, n_small,
        "statement count independent of ring size: {small_stmts:?} vs {big_stmts:?}"
    );
    assert_eq!(big["approvals"].as_array().unwrap().len(), 16);
    assert!(big["items"].as_array().unwrap().len() >= 55);

    // The guard + governed path: repeated calls re-read neither the session
    // row nor the agent's autonomy (cached; perf N4).
    probe.reset();
    for _ in 0..5 {
        d.invoke(&token, "list_repos", json!({})).await;
    }
    let calls = probe.take();
    assert!(
        !calls.iter().any(|q| q.contains("personal_agent_autonomy")),
        "autonomy cached: {calls:?}"
    );
    let ro = d
        .agent_session(json!({"personal_agent": agent, "read_only": true}))
        .await;
    let (st, _) = d
        .send(
            "POST",
            &ro,
            "/agent-rooms/r1/messages",
            Some(json!({"text": "x"})),
        )
        .await;
    assert_eq!(st, 403);
    probe.reset();
    for _ in 0..5 {
        let (st, _) = d
            .send(
                "POST",
                &ro,
                "/agent-rooms/r1/messages",
                Some(json!({"text": "x"})),
            )
            .await;
        assert_eq!(st, 403, "still refused from the cache");
    }
    let guard = probe.take();
    assert!(
        !guard
            .iter()
            .any(|q| q.contains("SELECT * FROM sessions WHERE id")),
        "read-only flag cached: {guard:?}"
    );

    // A cursor from a previous daemon process → full answer, reset.
    for stale in [
        format!("{path}?after_seq={cursor}&epoch=previous-boot&runs=false"),
        format!("{path}?after_seq=999999999&runs=false"),
    ] {
        let (st, ans) = d.send("GET", &d.human, &stale, None).await;
        assert_eq!(st, 200, "{ans}");
        assert_eq!(ans["reset"], true, "{stale}: {ans}");
        assert_eq!(ans["epoch"], epoch.as_str());
        assert!(ans["items"].as_array().unwrap().len() >= 55, "{ans}");
    }
}

/// Perf N2 (swarm W7 budget, reusing this file's full-daemon harness): an
/// ACTIVE swarm with nothing ready and no events costs nothing between
/// safety ticks. Its coordinator ticks when started and once for the
/// start's own status event (≤ 6 statements each), then parks on the
/// swarm's bell: no swarm statement runs for several MIN_GAPs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_idle_swarm_coordinator_issues_no_statements_between_safety_ticks() {
    let d = boot(&[]).await;
    let swarm = SwarmRepo::new(d.pool.clone())
        .create_swarm(otto_state::NewSwarm {
            workspace_id: "ws1".into(),
            name: "Idle".into(),
            description: String::new(),
            preset_slug: None,
            config: json!({}),
            // A budget, so the tick also reads spend.
            max_total_runs: Some(100),
            max_cost_usd: None,
            max_runtime_secs: None,
            max_attempts: None,
            created_by: "alice".into(),
        })
        .await
        .unwrap();
    let probe = d.pool.statement_probe();
    let (st, body) = d
        .send(
            "POST",
            &d.human,
            &format!("/workspaces/ws1/swarm/swarms/{}/start", swarm.id),
            None,
        )
        .await;
    assert_eq!(st, 200, "{body}");
    // The first tick runs at start; the start's own `swarm_status` event
    // rings the bell once more (after MIN_GAP). One tick of an idle swarm
    // with a budget: swarm + spend + active count + ready tasks.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    probe.reset();
    tokio::time::sleep(otto_server::swarm_wake::MIN_GAP + std::time::Duration::from_millis(1000))
        .await;
    let tick: Vec<String> = probe
        .take()
        .into_iter()
        .filter(|q| q.contains("swarm"))
        .collect();
    assert!(
        tick.len() <= 6,
        "an idle tick is at most 6 statements: {tick:?}"
    );
    // Then nothing: no event, and the safety tick is a minute away.
    tokio::time::sleep(
        otto_server::swarm_wake::MIN_GAP * 2 + std::time::Duration::from_millis(500),
    )
    .await;
    let idle: Vec<String> = probe
        .take()
        .into_iter()
        .filter(|q| q.contains("swarm"))
        .collect();
    assert!(
        idle.is_empty(),
        "an idle coordinator parks until an event or the {:?} safety tick: {idle:?}",
        otto_server::swarm_wake::SAFETY_TICK
    );
    assert!(
        otto_server::swarm_wake::has_bell(&swarm.id),
        "parked on its bell"
    );
}
