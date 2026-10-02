//! MCP auto-approve rules — end-to-end through the REAL daemon router on a
//! loopback port (production auth middleware + feature guard + the governed
//! `POST /mcp/otto-tools/invoke` choke point), playing an Otto agent session
//! calling `otto_create_pr` exactly as `ottod mcp-tools` proxies it.
//!
//! What it pins:
//! - no rule ⇒ `create_pr` opens a pending approval and does NOT execute;
//! - a rule (tool / category, global / workspace / session) ⇒ the call
//!   executes with no approval enqueued, audited `auto_approved` with a reason
//!   naming the rule; a rule for another workspace never applies;
//! - the irreversible guardrail: a category rule never covers `merge_pr`, a
//!   per-tool rule needs `allow_irreversible`, and the compat shim refuses it;
//! - deleting the rule restores the approval gate.

use std::sync::Arc;

use chrono::Utc;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_rbac::{AuthRepo, RbacRoleChecker};
use otto_server::ServerCtx;
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{
    ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, DbPool, GitStore, IntegrationsRepo,
    IssuesRepo, NewSession, ProductRepo, ReviewsRepo, SessionsRepo, SkillEvalsRepo, SwarmRepo,
    WorkspacesRepo,
};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

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
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, 'admin')")
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
    /// alice's own (human) API token — root, MCP Admin.
    human: String,
    /// The managed token of alice's agent session `sid` in ws1 (what an Otto
    /// session's `ottod mcp-tools` presents).
    agent: String,
    sid: Id,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let pool = file_pool(tmp.path()).await;
    seed_user(&pool, "alice", true).await;
    seed_workspace(&pool, "ws1", "alice").await;
    seed_workspace(&pool, "ws2", "alice").await;
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
    let s = SessionsRepo::new(pool.clone())
        .create(NewSession {
            workspace_id: "ws1".into(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "claude".into(),
            title: "Ship the fix".into(),
            cwd: repo_dir.to_string_lossy().to_string(),
            provider_session_id: None,
            connection_id: None,
            created_by: "alice".into(),
            meta: json!({}),
        })
        .await
        .unwrap();
    let auth = AuthRepo::new(pool.clone());
    let (human, _) = auth.issue_api_token(&"alice".to_string(), Some("ui")).await.unwrap();
    let (agent, _) = auth.issue_session_api_token(&"alice".to_string(), &s.id).await.unwrap();

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
        agent,
        sid: s.id,
        http: reqwest::Client::new(),
        _tmp: tmp,
    };
    // The operator exposes the PR tools (create_pr / merge_pr are off by default).
    let (st, body) = d
        .send("PATCH", &d.human.clone(), "/mcp/otto-server",
              Some(json!({"enabled": true, "tools": ["create_pr", "comment_pr", "merge_pr", "list_repos"]})))
        .await;
    assert_eq!(st, 200, "enable tools: {body}");
    d
}

impl Daemon {
    async fn send(&self, method: &str, token: &str, path: &str, body: Option<Value>) -> (u16, Value) {
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

    /// The agent session's governed call — what stdio `otto_<tool>` proxies to.
    async fn agent_invoke(&self, tool: &str, args: Value) -> Value {
        let (st, body) = self
            .send("POST", &self.agent, "/mcp/otto-tools/invoke",
                  Some(json!({"tool": format!("otto.{tool}"), "arguments": args})))
            .await;
        assert_eq!(st, 200, "invoke {tool}: {body}");
        body
    }

    async fn rule(&self, body: Value) -> (u16, Value) {
        self.send("POST", &self.human, "/mcp/auto-approve", Some(body)).await
    }

    async fn pending_approvals(&self) -> Vec<Value> {
        let (st, body) = self.send("GET", &self.human, "/mcp/approvals?status=pending", None).await;
        assert_eq!(st, 200, "approvals: {body}");
        body.as_array().cloned().unwrap_or_default()
    }

    /// Newest audit row for an otto.* tool.
    async fn last_audit(&self, tool: &str) -> Value {
        let (st, body) = self
            .send("GET", &self.human, &format!("/mcp/audit?tool=otto.{tool}"), None)
            .await;
        assert_eq!(st, 200, "audit: {body}");
        body.as_array()
            .and_then(|rows| rows.first().cloned())
            .unwrap_or_else(|| panic!("no audit row for {tool}: {body}"))
    }
}

fn pr_args() -> Value {
    json!({"repo_id": "repo1", "title": "Fix the report", "description": "Body",
           "source_branch": "fix/report", "target_branch": "main"})
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_pr_without_a_rule_enqueues_an_approval() {
    let d = boot().await;
    let env = d.agent_invoke("create_pr", pr_args()).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    assert_eq!(env["executed"], false);
    let pending = d.pending_approvals().await;
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0]["tool"], "otto.create_pr");
    assert_eq!(d.last_audit("create_pr").await["decision"], "pending_approval");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_approved_create_pr_runs_without_an_approval_and_is_audited() {
    let d = boot().await;
    let (st, rule) = d
        .rule(json!({"scope": "global", "target_kind": "tool", "target": "otto.create_pr",
                     "name": "Agents open PRs"}))
        .await;
    assert_eq!(st, 201, "{rule}");
    assert_eq!(rule["target"], "create_pr");

    let env = d.agent_invoke("create_pr", pr_args()).await;
    // It went past the gate and EXECUTED (the self-call to the git route then
    // fails — the seeded repo has no remote — which is irrelevant here).
    assert_eq!(env["executed"], true, "{env}");
    assert_ne!(env["decision"], "pending_approval");
    assert_eq!(env["auto_approved_by"]["name"], "Agents open PRs", "{env}");
    assert!(d.pending_approvals().await.is_empty(), "no approval may be enqueued");

    let row = d.last_audit("create_pr").await;
    assert_eq!(row["decision"], "auto_approved", "{row}");
    let reason = row["decision_reason"].as_str().unwrap_or_default();
    assert!(reason.starts_with("auto-approved by policy 'Agents open PRs'"), "{reason}");
    assert!(row["approval_id"].is_null());

    // The status catalog shows it as auto-approved everywhere.
    let (_, status) = d.send("GET", &d.human, "/mcp/otto-server", None).await;
    let tool = status["tools"].as_array().unwrap().iter()
        .find(|t| t["name"] == "otto.create_pr").unwrap().clone();
    assert_eq!(tool["approval_exempt"], true, "{tool}");
    assert_eq!(tool["auto_approved_by"][0]["id"], rule["id"]);

    // Deleting the rule restores the gate.
    let id = rule["id"].as_str().unwrap();
    let (st, _) = d.send("DELETE", &d.human, &format!("/mcp/auto-approve/{id}"), None).await;
    assert_eq!(st, 204);
    let env = d.agent_invoke("create_pr", pr_args()).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_and_category_rules_apply_only_where_they_say() {
    let d = boot().await;
    // A Git-category rule for ANOTHER workspace: the PR lands in ws1 → gated.
    let (st, other) = d
        .rule(json!({"scope": "workspace", "workspace_id": "ws2", "target_kind": "category", "target": "Git"}))
        .await;
    assert_eq!(st, 201, "{other}");
    let env = d.agent_invoke("create_pr", pr_args()).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");

    // The same rule for THIS session → auto-approved, naming the session rule.
    let (st, mine) = d
        .rule(json!({"scope": "session", "session_id": d.sid, "target_kind": "category", "target": "Git"}))
        .await;
    assert_eq!(st, 201, "{mine}");
    let mut args = pr_args();
    args["title"] = json!("Another PR"); // fresh args: not the pending approval's hash
    let env = d.agent_invoke("create_pr", args).await;
    assert_eq!(env["executed"], true, "{env}");
    assert_eq!(env["auto_approved_by"]["id"], mine["id"], "{env}");
    assert_eq!(d.last_audit("create_pr").await["decision"], "auto_approved");

    // Workspace scope: the repo's workspace (ws1) — resolved from the repo.
    let (st, ws_rule) = d
        .rule(json!({"scope": "workspace", "workspace_id": "ws1", "target_kind": "tool", "target": "comment_pr"}))
        .await;
    assert_eq!(st, 201, "{ws_rule}");
    let env = d
        .agent_invoke("comment_pr", json!({"repo_id": "demo", "number": 7, "body": "LGTM"}))
        .await;
    assert_eq!(env["auto_approved_by"]["id"], ws_rule["id"], "{env}");

    // Validation: a duplicate is a 409; a global rule with a workspace is a 400.
    let (st, _) = d
        .rule(json!({"scope": "session", "session_id": d.sid, "target_kind": "category", "target": "Git"}))
        .await;
    assert_eq!(st, 409);
    let (st, _) = d
        .rule(json!({"scope": "global", "workspace_id": "ws1", "target_kind": "tool", "target": "create_pr"}))
        .await;
    assert_eq!(st, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn irreversible_merge_needs_a_per_tool_rule_and_the_second_toggle() {
    let d = boot().await;
    let (st, _) = d
        .rule(json!({"scope": "global", "target_kind": "category", "target": "Git"}))
        .await;
    assert_eq!(st, 201);
    // The Git category covers create_pr but never the irreversible merge.
    let env = d.agent_invoke("merge_pr", json!({"repo_id": "repo1", "number": 3})).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");

    // A per-tool rule without the second toggle is refused…
    let (st, body) = d.rule(json!({"scope": "global", "target_kind": "tool", "target": "merge_pr"})).await;
    assert_eq!(st, 400, "{body}");
    // …a category rule can never carry it…
    let (st, _) = d
        .rule(json!({"scope": "workspace", "workspace_id": "ws1", "target_kind": "category",
                     "target": "Git", "allow_irreversible": true}))
        .await;
    assert_eq!(st, 400);
    // …and the legacy compat shim cannot add it either.
    let (st, _) = d
        .send("PATCH", &d.human, "/mcp/otto-server", Some(json!({"approval_exempt_tools": ["merge_pr"]})))
        .await;
    assert_eq!(st, 400);

    // With the explicit acknowledgement it is auto-approved.
    let (st, rule) = d
        .rule(json!({"scope": "global", "target_kind": "tool", "target": "merge_pr", "allow_irreversible": true}))
        .await;
    assert_eq!(st, 201, "{rule}");
    let env = d.agent_invoke("merge_pr", json!({"repo_id": "repo1", "number": 4})).await;
    assert_eq!(env["executed"], true, "{env}");
    let row = d.last_audit("merge_pr").await;
    assert_eq!(row["decision"], "auto_approved");
    assert!(row["decision_reason"].as_str().unwrap().contains("irreversible allowed"), "{row}");

    // Only an admin may change rules: the agent's own credential cannot.
    let (st, _) = d
        .send("POST", &d.agent, "/mcp/auto-approve",
              Some(json!({"scope": "global", "target_kind": "tool", "target": "comment_pr"})))
        .await;
    assert!(st == 403 || st == 401, "agent minted a rule: {st}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compat_shim_and_disabling_a_tool_drop_its_per_tool_rules() {
    let d = boot().await;
    let (st, status) = d
        .send("PATCH", &d.human, "/mcp/otto-server", Some(json!({"approval_exempt_tools": ["otto.create_pr"]})))
        .await;
    assert_eq!(st, 200, "{status}");
    assert_eq!(status["approval_exempt_tools"], json!(["create_pr"]));
    let (_, list) = d.send("GET", &d.human, "/mcp/auto-approve", None).await;
    assert_eq!(list["rules"].as_array().unwrap().len(), 1, "{list}");
    assert!(list["categories"].as_array().unwrap().iter().any(|c| c["category"] == "Git"));

    // Disabling create_pr drops its rule: re-enabling starts gated.
    let (st, _) = d
        .send("PATCH", &d.human, "/mcp/otto-server", Some(json!({"tools": ["comment_pr", "list_repos"]})))
        .await;
    assert_eq!(st, 200);
    let (_, list) = d.send("GET", &d.human, "/mcp/auto-approve", None).await;
    assert!(list["rules"].as_array().unwrap().is_empty(), "{list}");
}
