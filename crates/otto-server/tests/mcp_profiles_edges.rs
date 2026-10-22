//! The MCP profile's two server edges through the REAL daemon router
//! (test-quality-5-10). The pure pieces are unit-tested in their crates
//! (`check_mcp_profiles_patch`, `ProfiledTools`); this pins the wiring:
//!
//! - `PATCH /sessions/{id}` with a CHANGED `meta.mcp_profiles`: the session's
//!   own agent credential is refused (it would widen its own tool list), the
//!   owner may set it, and an agent's round-trip that leaves the value as it
//!   is still saves (computing `agent` as false fails the first assertion);
//! - `POST /mcp/http?profile=database` narrows `tools/list` and refuses a
//!   `tools/call` outside the profile, while the same request without
//!   `?profile=` lists the tool (reading another query key fails it).

use chrono::Utc;
use otto_rbac::AuthRepo;
use otto_state::{DbPool, NewSession, SessionsRepo};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

struct Daemon {
    base: String,
    http: reqwest::Client,
    /// The session owner's own login token.
    human: String,
    /// The session's managed (agent) token.
    agent: String,
    sid: String,
    _tmp: tempfile::TempDir,
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
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES ('alice', 'alice', 'x', 'alice', 1, ?)",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .expect("seed user");
    let cwd = tmp.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES ('ws1', 'ws1', ?, '{}', 0, ?)",
    )
    .bind(cwd.to_string_lossy().into_owned())
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .expect("seed workspace");
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1', 'alice', 'admin')",
    )
    .execute(&pool)
    .await
    .expect("member");
    let session = SessionsRepo::new(pool.clone())
        .create(NewSession {
            workspace_id: "ws1".into(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "claude".into(),
            title: "Profiled".into(),
            cwd: cwd.to_string_lossy().into_owned(),
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
        .issue_session_api_token(&"alice".to_string(), &session.id)
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
        http: reqwest::Client::new(),
        human,
        agent,
        sid: session.id,
        _tmp: tmp,
    }
}

impl Daemon {
    async fn send(&self, method: &str, token: &str, path: &str, body: Value) -> (u16, Value) {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let resp = self
            .http
            .request(m, format!("{}/api/v1{path}", self.base))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn patch_profiles(&self, token: &str, profiles: Value) -> (u16, Value) {
        let path = format!("/sessions/{}", self.sid);
        self.send(
            "PATCH",
            token,
            &path,
            json!({ "meta": { "mcp_profiles": profiles } }),
        )
        .await
    }

    /// One JSON-RPC call on `/mcp/http` (with `query`, e.g. `?profile=…`).
    async fn rpc(&self, query: &str, method: &str, params: Value) -> Value {
        let (st, body) = self
            .send(
                "POST",
                &self.human,
                &format!("/mcp/http{query}"),
                json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
            )
            .await;
        assert_eq!(st, 200, "{method}{query}: {body}");
        body
    }

    async fn tool_names(&self, query: &str) -> Vec<String> {
        let body = self.rpc(query, "tools/list", json!({})).await;
        body["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools/list{query}: {body}"))
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_a_person_changes_a_sessions_mcp_profile() {
    let d = boot().await;
    // The session's own agent credential may not widen (or set) its profile.
    let (st, body) = d.patch_profiles(&d.agent, json!(["full"])).await;
    assert_eq!(st, 403, "agent change: {body}");
    // The owner picks it.
    let (st, body) = d.patch_profiles(&d.human, json!(["full"])).await;
    assert_eq!(st, 200, "owner change: {body}");
    assert_eq!(body["meta"]["mcp_profiles"], json!(["full"]), "{body}");
    // An agent PATCH that carries the value unchanged is not a change.
    let (st, body) = d.patch_profiles(&d.agent, json!(["full"])).await;
    assert_eq!(st, 200, "agent round-trip: {body}");
    // …but a different value from the agent still is.
    let (st, body) = d.patch_profiles(&d.agent, json!(["database"])).await;
    assert_eq!(st, 403, "agent narrowing: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_profile_query_narrows_the_http_transport() {
    let d = boot().await;
    let (st, body) = d
        .send(
            "PATCH",
            &d.human,
            "/mcp/otto-server",
            json!({ "enabled": true, "tools": ["k8s_logs"] }),
        )
        .await;
    assert_eq!(st, 200, "enable k8s_logs: {body}");

    // No profile: every enabled tool, k8s_logs included.
    let all = d.tool_names("").await;
    assert!(all.iter().any(|n| n == "otto.k8s_logs"), "{all:?}");

    // `?profile=database`: no Kubernetes tool is listed…
    let narrowed = d.tool_names("?profile=database").await;
    assert!(
        !narrowed.iter().any(|n| n.starts_with("otto.k8s_")),
        "{narrowed:?}"
    );
    // …and calling one is refused with the profile denial, before it runs.
    let body = d
        .rpc(
            "?profile=database",
            "tools/call",
            json!({ "name": "otto.k8s_logs", "arguments": {} }),
        )
        .await;
    let text = body.to_string();
    assert!(
        text.contains("is not in the MCP profile"),
        "expected the profile denial: {text}"
    );
}
