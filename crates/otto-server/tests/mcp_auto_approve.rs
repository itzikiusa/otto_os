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
use otto_core::Id;
use otto_rbac::AuthRepo;
use otto_server::ServerCtx;
use otto_state::{DbPool, NewSession, SessionsRepo};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// Harness (mirrors ui_control.rs: a real base_url + listener)
// ---------------------------------------------------------------------------

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
    let mut ctx = ServerCtx::for_tests(pool, tmp).await;
    ctx.base_url = base_url;
    ctx
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
    /// The daemon's database (query-budget probes: `DbPool::op_count`).
    pool: DbPool,
    /// The daemon's event bus (stands in for ottod's approval-change hook).
    events: broadcast::Sender<otto_core::event::Event>,
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
    let (human, _) = auth
        .issue_api_token(&"alice".to_string(), Some("ui"))
        .await
        .unwrap();
    let (agent, _) = auth
        .issue_session_api_token(&"alice".to_string(), &s.id)
        .await
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let ctx = test_ctx(&pool, base.clone(), tmp.path()).await;
    let events = ctx.events.clone();
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
        pool: pool.clone(),
        events,
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

    /// The agent session's governed call — what stdio `otto_<tool>` proxies to.
    async fn agent_invoke(&self, tool: &str, args: Value) -> Value {
        let (st, body) = self
            .send(
                "POST",
                &self.agent,
                "/mcp/otto-tools/invoke",
                Some(json!({"tool": format!("otto.{tool}"), "arguments": args})),
            )
            .await;
        assert_eq!(st, 200, "invoke {tool}: {body}");
        body
    }

    async fn rule(&self, body: Value) -> (u16, Value) {
        self.send("POST", &self.human, "/mcp/auto-approve", Some(body))
            .await
    }

    async fn pending_approvals(&self) -> Vec<Value> {
        let (st, body) = self
            .send("GET", &self.human, "/mcp/approvals?status=pending", None)
            .await;
        assert_eq!(st, 200, "approvals: {body}");
        body.as_array().cloned().unwrap_or_default()
    }

    /// Newest audit row for an otto.* tool.
    async fn last_audit(&self, tool: &str) -> Value {
        let (st, body) = self
            .send(
                "GET",
                &self.human,
                &format!("/mcp/audit?tool=otto.{tool}"),
                None,
            )
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
    assert_eq!(
        d.last_audit("create_pr").await["decision"],
        "pending_approval"
    );
    // The card names the requesting session; the audit row carries the
    // session and the call's resolved workspace.
    assert_eq!(pending[0]["requested_by_session_id"], d.sid.as_str());
    let audit = d.last_audit("create_pr").await;
    assert_eq!(audit["caller_session_id"], d.sid.as_str(), "{audit}");
    assert_eq!(audit["workspace_id"], "ws1", "{audit}");
    // A retry while it is still waiting reuses the same card.
    let again = d.agent_invoke("create_pr", pr_args()).await;
    assert_eq!(again["decision"], "pending_approval", "{again}");
    assert_eq!(again["approval_id"], env["approval_id"], "{again}");
    assert_eq!(d.pending_approvals().await.len(), 1, "no duplicate card");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_approved_create_pr_runs_without_an_approval_and_is_audited() {
    let d = boot().await;
    let (st, rule) = d
        .rule(
            json!({"scope": "global", "target_kind": "tool", "target": "otto.create_pr",
                     "name": "Agents open PRs"}),
        )
        .await;
    assert_eq!(st, 201, "{rule}");
    assert_eq!(rule["target"], "create_pr");

    let env = d.agent_invoke("create_pr", pr_args()).await;
    // It went past the gate and EXECUTED (the self-call to the git route then
    // fails — the seeded repo has no remote — which is irrelevant here).
    assert_eq!(env["executed"], true, "{env}");
    assert_ne!(env["decision"], "pending_approval");
    assert_eq!(env["auto_approved_by"]["name"], "Agents open PRs", "{env}");
    assert!(
        d.pending_approvals().await.is_empty(),
        "no approval may be enqueued"
    );

    let row = d.last_audit("create_pr").await;
    assert_eq!(row["decision"], "auto_approved", "{row}");
    let reason = row["decision_reason"].as_str().unwrap_or_default();
    assert!(
        reason.starts_with("auto-approved by policy 'Agents open PRs'"),
        "{reason}"
    );
    assert!(row["approval_id"].is_null());

    // The status catalog shows it as auto-approved everywhere.
    let (_, status) = d.send("GET", &d.human, "/mcp/otto-server", None).await;
    let tool = status["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "otto.create_pr")
        .unwrap()
        .clone();
    assert_eq!(tool["approval_exempt"], true, "{tool}");
    assert_eq!(tool["auto_approved_by"][0]["id"], rule["id"]);

    // Deleting the rule restores the gate.
    let id = rule["id"].as_str().unwrap();
    let (st, _) = d
        .send("DELETE", &d.human, &format!("/mcp/auto-approve/{id}"), None)
        .await;
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
    // Both the session's Git rule and the workspace's comment_pr rule cover
    // it: the most specific scope (the session) is the one recorded…
    let env = d
        .agent_invoke(
            "comment_pr",
            json!({"repo_id": "demo", "number": 7, "body": "LGTM"}),
        )
        .await;
    assert_eq!(env["auto_approved_by"]["id"], mine["id"], "{env}");
    // …and without it the workspace rule (resolved via the repo's workspace).
    let (st, _) = d
        .send(
            "DELETE",
            &d.human,
            &format!("/mcp/auto-approve/{}", mine["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(st, 204);
    let env = d
        .agent_invoke(
            "comment_pr",
            json!({"repo_id": "demo", "number": 8, "body": "LGTM"}),
        )
        .await;
    assert_eq!(env["auto_approved_by"]["id"], ws_rule["id"], "{env}");

    // Validation: a duplicate is a 409; a global rule with a workspace is a 400.
    let (st, _) = d
        .rule(json!({"scope": "workspace", "workspace_id": "ws2", "target_kind": "category", "target": "Git"}))
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
    let env = d
        .agent_invoke("merge_pr", json!({"repo_id": "repo1", "number": 3}))
        .await;
    assert_eq!(env["decision"], "pending_approval", "{env}");

    // A per-tool rule without the second toggle is refused…
    let (st, body) = d
        .rule(json!({"scope": "global", "target_kind": "tool", "target": "merge_pr"}))
        .await;
    assert_eq!(st, 400, "{body}");
    // …a category rule can never carry it…
    let (st, _) = d
        .rule(
            json!({"scope": "workspace", "workspace_id": "ws1", "target_kind": "category",
                     "target": "Git", "allow_irreversible": true}),
        )
        .await;
    assert_eq!(st, 400);
    // …and the legacy compat shim cannot add it either.
    let (st, _) = d
        .send(
            "PATCH",
            &d.human,
            "/mcp/otto-server",
            Some(json!({"approval_exempt_tools": ["merge_pr"]})),
        )
        .await;
    assert_eq!(st, 400);

    // With the explicit acknowledgement it is auto-approved.
    let (st, rule) = d
        .rule(json!({"scope": "global", "target_kind": "tool", "target": "merge_pr", "allow_irreversible": true}))
        .await;
    assert_eq!(st, 201, "{rule}");
    let env = d
        .agent_invoke("merge_pr", json!({"repo_id": "repo1", "number": 4}))
        .await;
    assert_eq!(env["executed"], true, "{env}");
    let row = d.last_audit("merge_pr").await;
    assert_eq!(row["decision"], "auto_approved");
    assert!(
        row["decision_reason"]
            .as_str()
            .unwrap()
            .contains("irreversible allowed"),
        "{row}"
    );

    // Only a person may change rules: the agent session's own credential
    // (which authorizes as its root owner) cannot auto-approve its calls —
    // neither through the rules API nor the compat shim.
    let (st, _) = d
        .send(
            "POST",
            &d.agent,
            "/mcp/auto-approve",
            Some(json!({"scope": "global", "target_kind": "tool", "target": "comment_pr"})),
        )
        .await;
    assert_eq!(st, 403, "agent minted a rule");
    let id = rule["id"].as_str().unwrap();
    let (st, _) = d
        .send(
            "PATCH",
            &d.agent,
            &format!("/mcp/auto-approve/{id}"),
            Some(json!({"enabled": false})),
        )
        .await;
    assert_eq!(st, 403);
    let (st, _) = d
        .send("DELETE", &d.agent, &format!("/mcp/auto-approve/{id}"), None)
        .await;
    assert_eq!(st, 403);
    let (st, _) = d
        .send(
            "PATCH",
            &d.agent,
            "/mcp/otto-server",
            Some(json!({"approval_exempt_tools": ["comment_pr"]})),
        )
        .await;
    assert_eq!(st, 403);
    // Reading them is fine.
    let (st, _) = d.send("GET", &d.agent, "/mcp/auto-approve", None).await;
    assert_eq!(st, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compat_shim_and_disabling_a_tool_drop_its_per_tool_rules() {
    let d = boot().await;
    let (st, status) = d
        .send(
            "PATCH",
            &d.human,
            "/mcp/otto-server",
            Some(json!({"approval_exempt_tools": ["otto.create_pr"]})),
        )
        .await;
    assert_eq!(st, 200, "{status}");
    assert_eq!(status["approval_exempt_tools"], json!(["create_pr"]));
    let (_, list) = d.send("GET", &d.human, "/mcp/auto-approve", None).await;
    assert_eq!(list["rules"].as_array().unwrap().len(), 1, "{list}");
    assert!(list["categories"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["category"] == "Git"));

    // Disabling create_pr drops its rule: re-enabling starts gated.
    let (st, _) = d
        .send(
            "PATCH",
            &d.human,
            "/mcp/otto-server",
            Some(json!({"tools": ["comment_pr", "list_repos"]})),
        )
        .await;
    assert_eq!(st, 200);
    let (_, list) = d.send("GET", &d.human, "/mcp/auto-approve", None).await;
    assert!(list["rules"].as_array().unwrap().is_empty(), "{list}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_credential_cannot_decide_approvals_but_its_owner_can() {
    let d = boot().await;
    let env = d.agent_invoke("create_pr", pr_args()).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    let id = env["approval_id"].as_str().unwrap().to_string();

    // The agent session that raised it cannot approve (or deny) it.
    let (st, body) = d
        .send(
            "POST",
            &d.agent,
            &format!("/mcp/approvals/{id}/decide"),
            Some(json!({"approved": true})),
        )
        .await;
    assert_eq!(st, 403, "agent approved its own request: {body}");
    let (st, _) = d
        .send(
            "POST",
            &d.agent,
            &format!("/mcp/approvals/{id}/decide"),
            Some(json!({"approved": false})),
        )
        .await;
    assert_eq!(st, 403);
    assert_eq!(d.pending_approvals().await.len(), 1, "still pending");

    // The human owner approves their own agent's request — the normal flow —
    // and the agent's identical retry then executes on that approval.
    let (st, body) = d
        .send(
            "POST",
            &d.human,
            &format!("/mcp/approvals/{id}/decide"),
            Some(json!({"approved": true})),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["status"], "approved");
    let env = d.agent_invoke("create_pr", pr_args()).await;
    assert_eq!(env["executed"], true, "{env}");
    assert_eq!(d.last_audit("create_pr").await["decision"], "approved");
}

// ---------------------------------------------------------------------------
// Performance guards (perf/10-mcp F2/F4/F5/F8/F9) — and proof the governance
// semantics the caches sit under are unchanged.
// ---------------------------------------------------------------------------

/// The light enabled read answers names only (+ the caller session's UI
/// grant), agrees with the full status, and is reachable by the agent token.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn light_enabled_read_matches_the_status_and_reports_the_grant() {
    let d = boot().await;
    let (st, light) = d
        .send("GET", &d.agent, "/mcp/otto-server/enabled", None)
        .await;
    assert_eq!(st, 200, "{light}");
    let (_, full) = d.send("GET", &d.human, "/mcp/otto-server", None).await;
    let from_status: Vec<&str> = full["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["enabled"] == json!(true))
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let light_names: Vec<&str> = light["enabled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert_eq!(light_names, from_status);
    assert_eq!(light["outward_enabled"], json!(true));
    assert_eq!(light["ui_granted"], json!(false));
    let bytes_light = serde_json::to_vec(&light).unwrap().len();
    let bytes_full = serde_json::to_vec(&full).unwrap().len();
    assert!(
        bytes_light * 10 < bytes_full,
        "the light read must be a small fraction of the status ({bytes_light} vs {bytes_full})"
    );
    // The human grants UI control → the agent's next read says so.
    let (st, body) = d
        .send(
            "POST",
            &d.human,
            &format!("/sessions/{}/ui-control", d.sid),
            Some(json!({"enabled": true})),
        )
        .await;
    assert_eq!(st, 200, "grant: {body}");
    let (_, light) = d
        .send("GET", &d.agent, "/mcp/otto-server/enabled", None)
        .await;
    assert_eq!(light["ui_granted"], json!(true), "{light}");
    // A person's own credential is not a session: never granted.
    let (_, human) = d
        .send("GET", &d.human, "/mcp/otto-server/enabled", None)
        .await;
    assert_eq!(human["ui_granted"], json!(false));
}

/// Settings are cached for the governed path, but an operator's change is
/// effective on the very next call (write-through invalidation): turning the
/// dangerous-approval gate off / on flips `create_pr` between running and
/// asking at once, and a disabled tool disappears from the next read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_setting_changes_apply_to_the_next_call() {
    let d = boot().await;
    // Warm the cache with the gate ON.
    let env = d.agent_invoke("create_pr", pr_args()).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    let (st, body) = d
        .send(
            "PUT",
            &d.human,
            "/settings",
            Some(json!({"mcp_require_approval_dangerous": false})),
        )
        .await;
    assert!(st == 200 || st == 204, "settings: {st} {body}");
    let env = d
        .agent_invoke(
            "create_pr",
            json!({"repo_id": "repo1", "title": "Other", "description": "B",
            "source_branch": "fix/other", "target_branch": "main"}),
        )
        .await;
    assert_ne!(
        env["decision"], "pending_approval",
        "the gate was turned off — the next call must not ask: {env}"
    );
    let (st, _) = d
        .send(
            "PUT",
            &d.human,
            "/settings",
            Some(json!({"mcp_require_approval_dangerous": true})),
        )
        .await;
    assert!(st == 200 || st == 204);
    let env = d
        .agent_invoke(
            "create_pr",
            json!({"repo_id": "repo1", "title": "Third", "description": "C",
            "source_branch": "fix/third", "target_branch": "main"}),
        )
        .await;
    assert_eq!(env["decision"], "pending_approval", "gate back on: {env}");
    // The enable list: dropping a tool is visible to the very next read.
    let (st, body) = d
        .send(
            "PATCH",
            &d.human,
            "/mcp/otto-server",
            Some(json!({"tools": ["comment_pr", "merge_pr", "list_repos"]})),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    let (_, light) = d
        .send("GET", &d.agent, "/mcp/otto-server/enabled", None)
        .await;
    assert!(
        !light["enabled"]
            .as_array()
            .unwrap()
            .contains(&json!("otto.create_pr")),
        "disabling is visible at once: {light}"
    );
}

/// An approval decision resumes the waiting call as soon as the change event
/// fires — not on the next 1 s poll (and not on the 5 s fallback).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_approval_resumes_the_waiting_call_on_the_change_event() {
    let d = boot().await;
    let d = Arc::new(d);
    let waiter = {
        let d = d.clone();
        tokio::spawn(async move {
            let (st, body) = d
                .send(
                    "POST",
                    &d.agent,
                    "/mcp/otto-tools/invoke",
                    Some(json!({"tool": "otto.create_pr", "arguments": pr_args(),
                                "wait_seconds": 10})),
                )
                .await;
            (st, body, std::time::Instant::now())
        })
    };
    // Wait until the card exists, then decide it as the human owner.
    let id = loop {
        if let Some(a) = d.pending_approvals().await.first() {
            break a["id"].as_str().unwrap().to_string();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (st, body) = d
        .send(
            "POST",
            &d.human,
            &format!("/mcp/approvals/{id}/decide"),
            Some(json!({"approved": true})),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    // ottod's approval-change hook publishes this; the test router has no
    // hook installed, so publish it the same way.
    let decided = std::time::Instant::now();
    let _ = d.events.send(otto_core::event::Event::McpApprovalChanged {
        approval_id: Some(id.clone()),
        workspace_id: Some("ws1".into()),
        status: "approved".into(),
    });
    let (st, env, done) = waiter.await.unwrap();
    assert_eq!(st, 200, "{env}");
    // It RAN on the approval (the fixture repo has no git provider, so the
    // PR route itself errors — after execution, not as a pending/denied gate).
    assert_eq!(env["executed"], true, "the waiting call ran: {env}");
    let latency = done.saturating_duration_since(decided);
    assert!(
        latency < std::time::Duration::from_millis(500),
        "resumed {latency:?} after the decision (event-driven wake expected)"
    );
    assert_eq!(d.last_audit("create_pr").await["decision"], "approved");
}

/// Statement budget for one governed READ call (dry-run, so the self-call's
/// own route is not measured): the whole request — token auth, the feature
/// guard, the governed pipeline and its audit insert. Before: the settings
/// reads alone were 4–5 per call, plus the status catalog the bridge fetched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_governed_read_call_stays_within_its_statement_budget() {
    let d = boot().await;
    let call = || {
        d.send(
            "POST",
            &d.agent,
            "/mcp/otto-tools/invoke",
            Some(json!({"tool": "otto.list_repos", "arguments": {}, "dry_run": true})),
        )
    };
    let (st, env) = call().await; // warm caches (auth, settings)
    assert_eq!(st, 200, "{env}");
    assert_eq!(env["decision"], "dry_run", "{env}");
    let before = d.pool.op_count();
    let (st, env) = call().await;
    assert_eq!(st, 200, "{env}");
    let used = d.pool.op_count() - before;
    eprintln!("governed dry-run read: {used} statements");
    assert!(
        used <= GOVERNED_READ_BUDGET,
        "{used} statements > budget {GOVERNED_READ_BUDGET}"
    );
    // And the light enabled read the bridge makes per tools/list.
    let before = d.pool.op_count();
    let (st, _) = d
        .send("GET", &d.agent, "/mcp/otto-server/enabled", None)
        .await;
    assert_eq!(st, 200);
    let used = d.pool.op_count() - before;
    eprintln!("enabled read: {used} statements");
    assert!(
        used <= ENABLED_READ_BUDGET,
        "{used} > {ENABLED_READ_BUDGET}"
    );
}
/// Measured 4 (token auth + guard + pipeline + audit insert, settings cached).
const GOVERNED_READ_BUDGET: u64 = 8;
/// Measured 3.
const ENABLED_READ_BUDGET: u64 = 5;

/// `GET /mcp/audit` over 200 rows from 2 downstream servers: visibility is
/// memoized per (server, tool), so the page costs a fixed handful of
/// statements instead of ~5–8 per row.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_audit_list_does_not_query_per_row() {
    let d = boot().await;
    let reg = otto_state::McpRegistryRepo::new(d.pool.clone());
    let mut servers = Vec::new();
    for name in ["alpha", "beta"] {
        let s = reg
            .create(otto_state::NewServerRow {
                workspace_id: "ws1".into(),
                name: name.into(),
                transport: "stdio".into(),
                command: "true".into(),
                args: vec![],
                env: Default::default(),
                url: None,
                description: None,
                headers: Default::default(),
                secret_ref: None,
                secret_env_keys: vec![],
                secret_header_keys: vec![],
                injection_risk: "low".into(),
                default_tool_access: "allow".into(),
                enabled: true,
                created_by: "alice".into(),
            })
            .await
            .unwrap();
        servers.push(s);
    }
    let log = otto_state::McpCallLogRepo::new(d.pool.clone());
    for i in 0..200 {
        let s = &servers[i % 2];
        log.insert(otto_state::NewCallLog {
            workspace_id: Some("ws1".into()),
            server_id: Some(s.id.clone()),
            server_name: Some(s.name.clone()),
            tool: format!("tool_{}", i % 3),
            direction: "outbound".into(),
            caller_user_id: Some("alice".into()),
            decision: "allowed".into(),
            args_redacted_json: "{}".into(),
            ok: true,
            ..Default::default()
        })
        .await
        .unwrap();
    }
    let (one_row, all_rows) = ("/mcp/audit?limit=1", "/mcp/audit?limit=200");
    let _ = d.send("GET", &d.human, one_row, None).await; // warm auth
    let before = d.pool.op_count();
    let (st, rows) = d.send("GET", &d.human, one_row, None).await;
    assert_eq!(st, 200, "{rows}");
    let one = d.pool.op_count() - before;
    let before = d.pool.op_count();
    let (st, rows) = d.send("GET", &d.human, all_rows, None).await;
    assert_eq!(st, 200, "{rows}");
    assert_eq!(rows.as_array().unwrap().len(), 200);
    let many = d.pool.op_count() - before;
    eprintln!("audit list: 1 row = {one} statements, 200 rows = {many}");
    // A fixed cost per distinct server (row + live policy), nothing per row:
    // measured 5 (1 row) vs 7 (200 rows); per-row checks cost ~600.
    assert!(
        many <= 9,
        "200 rows cost {many} statements vs {one} for 1 — visibility is per row again"
    );
}

/// The Enforced variant of [`the_audit_list_does_not_query_per_row`]
/// (perf2/10-mcp R1): under an Enforced policy every distinct (server, tool)
/// pair used to re-read the caller's MCP capability, workspace role and
/// access groups — 3 statements × 80 pairs here. A NON-root caller (root
/// short-circuits those reads) now pays them once per request, so a 200-row
/// page over 2 servers × 40 tools and the stats table both stay fixed-cost.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_enforced_audit_page_and_stats_read_the_caller_once() {
    use otto_core::access::{AccessActor, AccessMode, AccessPolicy, AccessRule};
    use otto_core::access::{ResourceKind, RuleEffect, SubjectKind};
    let d = boot().await;
    // bob: a plain ws1 editor with MCP View — the reads root skips.
    seed_user(&d.pool, "bob", false).await;
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1', 'bob', 'editor')")
        .execute(&d.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_feature_grants (user_id, feature, capability) VALUES ('bob', 'mcp', 'view')")
        .execute(&d.pool)
        .await
        .unwrap();
    let (bob, _) = AuthRepo::new(d.pool.clone())
        .issue_api_token(&"bob".to_string(), Some("ui"))
        .await
        .unwrap();
    let reg = otto_state::McpRegistryRepo::new(d.pool.clone());
    let access = otto_state::ResourceAccessRepo::new(d.pool.clone());
    let mut servers = Vec::new();
    for name in ["alpha", "beta"] {
        let s = reg
            .create(otto_state::NewServerRow {
                workspace_id: "ws1".into(),
                name: name.into(),
                transport: "stdio".into(),
                command: "true".into(),
                args: vec![],
                env: Default::default(),
                url: None,
                description: None,
                headers: Default::default(),
                secret_ref: None,
                secret_env_keys: vec![],
                secret_header_keys: vec![],
                injection_risk: "low".into(),
                default_tool_access: "allow".into(),
                enabled: true,
                created_by: "alice".into(),
            })
            .await
            .unwrap();
        // Enforced: bob may discover every tool except tool_39.
        let current = access
            .get_policy(ResourceKind::McpServer, &s.id)
            .await
            .unwrap();
        let rule = |effect, children: Option<Vec<String>>| AccessRule {
            id: otto_core::new_id(),
            subject_kind: SubjectKind::User,
            subject_id: "bob".into(),
            effect,
            operations: vec!["discover".into()],
            children,
            grantable_operations: vec![],
            credential_connection_id: None,
        };
        access
            .put_policy(
                &AccessPolicy {
                    kind: ResourceKind::McpServer,
                    resource_id: s.id.clone(),
                    mode: AccessMode::Enforced,
                    revision: current.revision,
                    rules: vec![
                        rule(RuleEffect::Allow, None),
                        rule(RuleEffect::Deny, Some(vec!["tool_39".into()])),
                    ],
                },
                current.revision,
                &AccessActor {
                    real_user_id: "alice".into(),
                    effective_user_id: None,
                },
            )
            .await
            .unwrap();
        servers.push(s);
    }
    let log = otto_state::McpCallLogRepo::new(d.pool.clone());
    for i in 0..200 {
        let s = &servers[i % 2];
        log.insert(otto_state::NewCallLog {
            workspace_id: Some("ws1".into()),
            server_id: Some(s.id.clone()),
            server_name: Some(s.name.clone()),
            // 2 servers × 40 tools = 80 distinct (server, tool) pairs.
            tool: format!("tool_{}", (i / 2) % 40),
            direction: "outbound".into(),
            caller_user_id: Some("bob".into()),
            decision: "allowed".into(),
            args_redacted_json: "{}".into(),
            ok: true,
            ..Default::default()
        })
        .await
        .unwrap();
    }
    let _ = d.send("GET", &bob, "/mcp/audit?limit=1", None).await; // warm auth
    let before = d.pool.op_count();
    let (st, rows) = d.send("GET", &bob, "/mcp/audit?limit=200", None).await;
    let page = d.pool.op_count() - before;
    assert_eq!(st, 200, "{rows}");
    let rows = rows.as_array().unwrap();
    // The policy really filters (tool_39 is denied on both servers: 2 × 2
    // rows of it in the 200), so the budget is not measured on a no-op.
    assert_eq!(rows.len(), 196, "tool_39 rows hidden");
    assert!(rows.iter().all(|r| r["tool"] != "tool_39"));
    let before = d.pool.op_count();
    let (st, stats) = d.send("GET", &bob, "/mcp/stats", None).await;
    let stats_cost = d.pool.op_count() - before;
    assert_eq!(st, 200, "{stats}");
    assert_eq!(stats.as_array().unwrap().len(), 78, "{stats}");
    eprintln!("enforced: audit 200 rows = {page} statements, stats 80 pairs = {stats_cost}");
    // Fixed cost: auth + ws list + the page + 2 × (server row + live policy)
    // + the caller's capability / role / groups ONCE. It was ~3 per pair (≈ 240).
    assert!(
        page <= ENFORCED_LIST_BUDGET,
        "audit page: {page} statements"
    );
    assert!(
        stats_cost <= ENFORCED_LIST_BUDGET,
        "stats: {stats_cost} statements"
    );
}
/// Measured 10 for both (was ≈ 3 per distinct pair before R1: ~250).
const ENFORCED_LIST_BUDGET: u64 = 12;

/// R6: statement budgets on the MUTATING paths, the whole request measured
/// (token auth, guard, pipeline, auto-approve resolution / approval
/// creation, audit, and — when executed — the self-call to the git route,
/// which fails early here: the fixture repo has no provider).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutating_calls_stay_within_their_statement_budgets() {
    let d = boot().await;
    // Warm the token/settings caches with a read.
    let _ = d.agent_invoke("list_repos", json!({})).await;
    // 1. No rule: the call files a pending approval (and returns at once).
    let call = |args: Value| {
        d.send(
            "POST",
            &d.agent,
            "/mcp/otto-tools/invoke",
            Some(json!({"tool": "otto.create_pr", "arguments": args, "wait_seconds": 0})),
        )
    };
    let before = d.pool.op_count();
    let (st, env) = call(pr_args()).await;
    let pending = d.pool.op_count() - before;
    assert_eq!(st, 200, "{env}");
    assert_eq!(env["decision"], "pending_approval", "{env}");
    // 2. An auto-approve rule covers it: resolved, executed, audited.
    let (st, rule) = d
        .rule(
            json!({"scope": "global", "target_kind": "tool", "target": "otto.create_pr",
                     "name": "Agents open PRs"}),
        )
        .await;
    assert_eq!(st, 201, "{rule}");
    let mut args = pr_args();
    args["title"] = json!("Another fix");
    let _ = call(args.clone()).await; // warm the rules read path
    args["title"] = json!("A third fix");
    let before = d.pool.op_count();
    let (st, env) = call(args).await;
    let auto = d.pool.op_count() - before;
    assert_eq!(st, 200, "{env}");
    assert_eq!(env["executed"], true, "{env}");
    assert_eq!(env["auto_approved_by"]["name"], "Agents open PRs", "{env}");
    eprintln!("mutating create_pr: pending={pending} auto_approved+executed={auto} statements");
    assert!(pending <= PENDING_CALL_BUDGET, "pending call: {pending}");
    assert!(
        auto <= AUTO_APPROVED_CALL_BUDGET,
        "auto-approved call: {auto}"
    );
}
/// Measured 17: auth, session row, repo resolution, the auto-approve rules
/// read, the usable / still-pending approval lookups, the approval insert +
/// its re-read, the decision read and the audit insert — fixed per call.
const PENDING_CALL_BUDGET: u64 = 19;
/// Measured 14, INCLUDING the self-call's own route.
const AUTO_APPROVED_CALL_BUDGET: u64 = 16;

/// R7: the badge count equals the list's length (same visibility) and costs
/// a fixed handful of statements.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pending_count_matches_the_list() {
    let d = boot().await;
    for title in ["One", "Two", "Three"] {
        let mut args = pr_args();
        args["title"] = json!(title);
        let env = d.agent_invoke("create_pr", args).await;
        assert_eq!(env["decision"], "pending_approval", "{env}");
    }
    let listed = d.pending_approvals().await.len();
    let (st, body) = d
        .send("GET", &d.human, "/mcp/approvals/count?status=pending", None)
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["count"], json!(listed), "{body}");
    assert_eq!(listed, 3);
    let before = d.pool.op_count();
    let _ = d
        .send("GET", &d.human, "/mcp/approvals/count?status=pending", None)
        .await;
    let used = d.pool.op_count() - before;
    assert!(used <= 5, "count: {used} statements");
}

/// R7: the stdio bridge's audit append goes through the daemon — accepted
/// only from a session credential, stamped with THAT session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_bridge_audit_append_is_session_bound() {
    let d = boot().await;
    let (st, body) = d
        .send(
            "POST",
            &d.agent,
            "/mcp/tool-calls",
            Some(json!({"tool": "otto_list_repos", "arguments": {"token": "s3cret-value"}, "ok": true, "rows": 2})),
        )
        .await;
    assert_eq!(st, 204, "{body}");
    let row: (Option<String>, Option<String>, String, String, i64) = sqlx::query_as(
        "SELECT session_id, workspace_id, tool, args_json, rows FROM mcp_tool_calls",
    )
    .fetch_one(&d.pool)
    .await
    .unwrap();
    assert_eq!(row.0.as_deref(), Some(d.sid.as_str()));
    assert_eq!(row.1.as_deref(), Some("ws1"), "the session's workspace");
    assert_eq!(row.2, "otto_list_repos");
    assert!(!row.3.contains("s3cret-value"), "redacted: {}", row.3);
    assert_eq!(row.4, 2);
    // A person's own credential has no session to stamp: refused.
    let (st, _) = d
        .send(
            "POST",
            &d.human,
            "/mcp/tool-calls",
            Some(json!({"tool": "x", "ok": true})),
        )
        .await;
    assert_eq!(st, 403);
}

/// R4: a UI tool's governed reply carries the calling session's grant, so
/// the bridge needs no follow-up enabled read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_ui_tool_reply_carries_the_session_grant() {
    let d = boot().await;
    let (st, env) = d
        .send(
            "POST",
            &d.agent,
            "/mcp/otto-tools/invoke",
            Some(json!({"tool": "otto.ui_state", "arguments": {}, "dry_run": true})),
        )
        .await;
    assert_eq!(st, 200, "{env}");
    assert_eq!(env["ui_granted"], json!(false), "{env}");
    // A non-UI tool's envelope is unchanged.
    let (_, env) = d
        .send(
            "POST",
            &d.agent,
            "/mcp/otto-tools/invoke",
            Some(json!({"tool": "otto.list_repos", "arguments": {}, "dry_run": true})),
        )
        .await;
    assert!(env.get("ui_granted").is_none(), "{env}");
}
