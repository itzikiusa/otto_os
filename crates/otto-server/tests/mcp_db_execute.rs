//! `otto.query_db` — the governed database WRITER — end-to-end through the
//! REAL daemon router on a loopback port (production auth middleware + feature
//! guard + the governed `POST /mcp/otto-tools/invoke` choke point + the
//! Database Explorer's own `/db/query` route), playing an Otto agent session
//! calling `otto_query_db` exactly as `ottod mcp-tools` proxies it.
//!
//! What it pins:
//! - a write asks a person first (approval card, nothing executes), and the
//!   approved retry runs on a writable connection the user owns;
//! - the irreversible guardrail: a Database category rule never covers it, a
//!   per-tool rule with `allow_irreversible` does;
//! - a read-only or production connection refuses the write — after the
//!   approval — with a message naming the connection and the setting;
//! - `otto.query_db_readonly` still refuses a write on a writable connection.
//!
//! The connections are Redis profiles on a dead loopback port: a statement
//! that passes every gate fails fast at connect, one refused never connects.

use chrono::Utc;
use otto_core::domain::{ConnectionKind, Environment};
use otto_core::Id;
use otto_rbac::AuthRepo;
use otto_state::{ConnectionsRepo, DbPool, NewConnection, NewSession, SessionsRepo};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

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

struct Daemon {
    base: String,
    /// alice's own (human) API token — root, workspace admin.
    human: String,
    /// The managed token of alice's agent session in ws1.
    agent: String,
    http: reqwest::Client,
    /// Dev, not read-only: the agent may write once a person approves.
    writable: Id,
    /// Dev, `read_only` set in the connection's settings.
    read_only: Id,
    /// Production environment.
    prod: Id,
    _tmp: tempfile::TempDir,
}

async fn seed_conn(pool: &DbPool, name: &str, environment: Environment, read_only: bool) -> Id {
    ConnectionsRepo::new(pool.clone())
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: name.into(),
            kind: ConnectionKind::Redis,
            params: json!({"host": "127.0.0.1", "port": 1}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment,
            read_only,
            created_by: "alice".into(),
        })
        .await
        .expect("seed connection")
        .id
}

async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let pool = file_pool(tmp.path()).await;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES ('alice', 'alice', 'x', 'alice', 1, ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES ('ws1', 'ws', '/tmp', '{}', 0, ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1', 'alice', 'admin')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let writable = seed_conn(&pool, "cache", Environment::Dev, false).await;
    let read_only = seed_conn(&pool, "cache-ro", Environment::Dev, true).await;
    let prod = seed_conn(&pool, "cache-prod", Environment::Prod, false).await;
    let s = SessionsRepo::new(pool.clone())
        .create(NewSession {
            workspace_id: "ws1".into(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "claude".into(),
            title: "Fix the data".into(),
            cwd: tmp.path().to_string_lossy().to_string(),
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
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut ctx = otto_server::ServerCtx::for_tests(&pool, tmp.path()).await;
    ctx.base_url = base.clone();
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let d = Daemon {
        base,
        human,
        agent,
        http: reqwest::Client::new(),
        writable,
        read_only,
        prod,
        _tmp: tmp,
    };
    // The operator exposes both database query tools (off by default).
    let (st, body) = d
        .send(
            "PATCH",
            &d.human,
            "/mcp/otto-server",
            Some(json!({"enabled": true, "tools": ["query_db", "query_db_readonly"]})),
        )
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

    async fn allow_irreversible_query_db(&self) {
        let (st, rule) = self
            .send(
                "POST",
                &self.human,
                "/mcp/auto-approve",
                Some(
                    json!({"scope": "global", "target_kind": "tool", "target": "query_db",
                            "allow_irreversible": true}),
                ),
            )
            .await;
        assert_eq!(st, 201, "{rule}");
    }
}

fn write_args(conn: &Id) -> Value {
    json!({"connection_id": conn, "statement": "SET otto:test 1"})
}

/// The error text of an executed-but-failed governed call.
fn call_error(env: &Value) -> String {
    assert_eq!(env["executed"], true, "{env}");
    assert_eq!(env["is_error"], true, "{env}");
    env["content"]["error"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_asks_a_person_then_runs_on_a_writable_connection() {
    let d = boot().await;
    let env = d.agent_invoke("query_db", write_args(&d.writable)).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    assert_eq!(env["executed"], false);
    let id = env["approval_id"].as_str().unwrap().to_string();
    let (_, pending) = d
        .send("GET", &d.human, "/mcp/approvals?status=pending", None)
        .await;
    let card = &pending.as_array().unwrap()[0];
    assert_eq!(card["tool"], "otto.query_db");
    assert!(
        card["detail"].as_str().unwrap().contains("SET otto:test 1"),
        "the approver sees the statement: {card}"
    );

    let (st, body) = d
        .send(
            "POST",
            &d.human,
            &format!("/mcp/approvals/{id}/decide"),
            Some(json!({"approved": true})),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    // The approved retry passes every gate (Editor role, resource access,
    // write-guard) and reaches the driver — which fails at the dead port.
    let err = call_error(&d.agent_invoke("query_db", write_args(&d.writable)).await);
    assert!(
        !err.contains("write_blocked") && !err.contains("read-only") && !err.contains("403"),
        "a writable connection must not refuse the write: {err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guarded_connection_refuses_the_write_and_names_the_setting() {
    let d = boot().await;
    // A Database category rule never covers the irreversible writer…
    let (st, body) = d
        .send(
            "POST",
            &d.human,
            "/mcp/auto-approve",
            Some(json!({"scope": "global", "target_kind": "category", "target": "Database"})),
        )
        .await;
    assert_eq!(st, 201, "{body}");
    let env = d.agent_invoke("query_db", write_args(&d.read_only)).await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    // …a per-tool rule with the second toggle does — and the connection's
    // own guard still refuses, whatever the approval said.
    d.allow_irreversible_query_db().await;
    // New connections are access-enforced, so the enforced gate answers first
    // (the legacy write-guard's agent message says the same; see the
    // otto-dbviewer `mcp_query` suite).
    let err = call_error(&d.agent_invoke("query_db", write_args(&d.read_only)).await);
    assert!(err.contains("read-only connection 'cache-ro'"), "{err}");
    assert!(
        err.contains("turn off Read-only in the connection's settings"),
        "{err}"
    );
    let err = call_error(&d.agent_invoke("query_db", write_args(&d.prod)).await);
    assert!(err.contains("review_required: "), "{err}");
    assert!(err.contains("production connection 'cache-prod'"), "{err}");
    assert!(
        err.contains("set an environment other than Production in the connection's settings"),
        "{err}"
    );
    // A read on the guarded connection is not a write: it reaches the driver.
    let err = call_error(
        &d.agent_invoke(
            "query_db",
            json!({"connection_id": d.read_only, "statement": "GET otto:test"}),
        )
        .await,
    );
    assert!(!err.contains("write_blocked"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_read_only_tool_still_refuses_a_write() {
    let d = boot().await;
    let env = d
        .agent_invoke("query_db_readonly", write_args(&d.writable))
        .await;
    // Not approval-gated (a read tool), and its own route refuses the write.
    assert_ne!(env["decision"], "pending_approval", "{env}");
    let err = call_error(&env);
    assert!(err.contains("mcp_read_only: "), "{err}");
}
