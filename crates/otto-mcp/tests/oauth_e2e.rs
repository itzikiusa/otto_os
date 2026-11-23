//! OAuth on the governed path against a local fake server: the access token
//! rides on the governed client only, a 401 refreshes it once (rotation kept)
//! and retries the SAME call, a revoked grant and an unsigned server both say
//! "Needs sign-in", and sign-out deletes the tokens.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use otto_core::secrets::{get_async, put_async, SecretStore};
use otto_core::{new_id, Result};
use otto_mcp::oauth::{token_key, Tokens};
use otto_mcp::{InvokeCtx, InvokeOutcome, McpService};
use otto_state::{DbPool, NewServerRow};
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Trivial in-memory secret store (the pipeline only resolves secrets for servers
/// that declare them; this test uses none, but the service still needs a store).
#[derive(Default)]
struct MemSecrets(Mutex<HashMap<String, String>>);
impl SecretStore for MemSecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

async fn pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let p = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&p)
        .await
        .unwrap();
    p.into()
}

async fn seed_ws(pool: &DbPool) -> (String, String) {
    let user = new_id();
    let ws = new_id();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, 'u', 'x', 'U', 0, ?)")
        .bind(&user).bind(&now).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'w', '/tmp', ?)",
    )
    .bind(&ws)
    .bind(&now)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?, 'mcp', 'edit')",
    )
    .bind(&user)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id,user_id,role) VALUES (?, ?, 'editor')",
    )
    .bind(&ws)
    .bind(&user)
    .execute(pool)
    .await
    .unwrap();
    (ws, user)
}

/// A remote MCP server that takes exactly one bearer token (`accept`) and
/// its token endpoint (refresh `rt-1` → `at-2` / `rt-2`).
async fn fake(accept: Arc<Mutex<String>>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let accept = accept.clone();
            tokio::spawn(async move {
                let mut req = Vec::new();
                let mut buf = [0u8; 8192];
                let (head, body) = loop {
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    req.extend_from_slice(&buf[..n]);
                    let t = String::from_utf8_lossy(&req).to_string();
                    if let Some(end) = t.find("\r\n\r\n") {
                        let len = t[..end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if req.len() >= end + 4 + len {
                            break (t[..end].to_string(), t[end + 4..].to_string());
                        }
                    }
                };
                let auth = head
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("Authorization: ")
                            .or_else(|| l.strip_prefix("authorization: "))
                    })
                    .unwrap_or("")
                    .to_string();
                let (status, json) = if head.starts_with("POST /token") {
                    if body.contains("refresh_token=rt-1") {
                        (
                            200,
                            json!({"access_token":"at-2","refresh_token":"rt-2","expires_in":3600}),
                        )
                    } else {
                        (400, json!({"error":"invalid_grant"}))
                    }
                } else if auth != format!("Bearer {}", accept.lock().unwrap()) {
                    (401, json!({}))
                } else {
                    let msg: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let id = msg["id"].clone();
                    match msg["method"].as_str() {
                        Some("initialize") => (
                            200,
                            json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"fake","version":"1"}}}),
                        ),
                        Some("tools/list") => (
                            200,
                            json!({"jsonrpc":"2.0","id":id,"result":{"tools":[{"name":"list_items","description":"list items","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}}),
                        ),
                        Some("tools/call") => (
                            200,
                            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"listed"}]}}),
                        ),
                        _ => (202, json!(null)),
                    }
                };
                let body = if status == 202 {
                    String::new()
                } else {
                    json.to_string()
                };
                let extra = if status == 401 {
                    "WWW-Authenticate: Bearer\r\n"
                } else {
                    ""
                };
                let resp = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    (base, task)
}

async fn setup(
    accept: &str,
) -> (
    McpService,
    Arc<MemSecrets>,
    otto_state::McpServerDetail,
    String,
    Arc<Mutex<String>>,
    tokio::task::JoinHandle<()>,
) {
    let pool = pool().await;
    let (ws, user) = seed_ws(&pool).await;
    let accept = Arc::new(Mutex::new(accept.to_string()));
    let (base, task) = fake(accept.clone()).await;
    let secrets = Arc::new(MemSecrets::default());
    let svc = McpService::new(pool.clone(), secrets.clone());
    let server = svc
        .registry()
        .create(NewServerRow {
            workspace_id: ws.clone(),
            name: "remote".into(),
            transport: "http".into(),
            command: String::new(),
            args: vec![],
            env: Default::default(),
            url: Some(format!("{base}/mcp")),
            description: None,
            headers: Default::default(),
            secret_ref: None,
            secret_env_keys: vec![],
            secret_header_keys: vec![],
            injection_risk: "low".into(),
            default_tool_access: "allow".into(),
            enabled: true,
            created_by: user.clone(),
        })
        .await
        .unwrap();
    let repo = otto_state::ResourceAccessRepo::new(pool.clone());
    let old = repo
        .get_policy(otto_core::access::ResourceKind::McpServer, &server.id)
        .await
        .unwrap();
    let mut legacy = old.clone();
    legacy.mode = otto_core::access::AccessMode::Legacy;
    repo.put_policy(
        &legacy,
        old.revision,
        &otto_core::access::AccessActor {
            real_user_id: user.clone(),
            effective_user_id: None,
        },
    )
    .await
    .unwrap();
    (svc, secrets, server, base, accept, task)
}

fn tokens(base: &str, access: &str, refresh: &str) -> String {
    serde_json::to_string(&Tokens {
        access_token: access.into(),
        refresh_token: Some(refresh.into()),
        expires_at: Some(chrono::Utc::now().timestamp() + 3600),
        token_endpoint: format!("{base}/token"),
        client_id: "client-1".into(),
        client_secret: None,
        resource: format!("{base}/mcp"),
        auth_host: "127.0.0.1".into(),
    })
    .unwrap()
}

fn ctx(ws: &str) -> InvokeCtx {
    InvokeCtx {
        workspace_id: Some(ws.into()),
        dry_run: false,
        caller_user_id: Some("u".into()),
        caller_kind: "ui".into(),
        direction: "outbound".into(),
        caller_session_id: None,
    }
}

#[tokio::test]
async fn a_401_refreshes_once_and_the_same_call_runs_on_the_new_token() {
    let (svc, secrets, server, base, accept, task) = setup("at-1").await;
    let store: Arc<dyn SecretStore> = secrets.clone();
    put_async(
        &store,
        &token_key(&server.id),
        &tokens(&base, "at-1", "rt-1"),
    )
    .await
    .unwrap();
    svc.discover(&server.id)
        .await
        .expect("discover with the stored token");
    // The server stops taking at-1 (expired early / rotated server-side).
    *accept.lock().unwrap() = "at-2".into();
    let out = svc
        .invoke(
            &server.id,
            "list_items",
            &json!({}),
            &ctx(&server.workspace_id),
        )
        .await
        .unwrap();
    let InvokeOutcome::Executed { content, is_error } = out else {
        panic!("{out:?}")
    };
    assert!(!is_error, "{content}");
    assert_eq!(content["content"][0]["text"], "listed");
    let saved: Tokens = serde_json::from_str(
        &get_async(&store, &token_key(&server.id))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (saved.access_token.as_str(), saved.refresh_token.as_deref()),
        ("at-2", Some("rt-2")),
        "rotation kept"
    );
    task.abort();
}

#[tokio::test]
async fn a_revoked_grant_and_an_unsigned_server_say_needs_sign_in() {
    let (svc, secrets, server, base, _accept, task) = setup("at-2").await;
    // No tokens at all: the 401 marks the server.
    let probed = svc.health_check(&server.id).await.unwrap();
    assert!(
        probed
            .health_error
            .as_deref()
            .unwrap_or("")
            .starts_with("Needs sign-in"),
        "{:?}",
        probed.health_error
    );
    // Signed in: tools discovered, then the grant is revoked.
    let store: Arc<dyn SecretStore> = secrets.clone();
    put_async(
        &store,
        &token_key(&server.id),
        &tokens(&base, "at-2", "rt-revoked"),
    )
    .await
    .unwrap();
    svc.discover(&server.id).await.expect("discover signed in");
    *_accept.lock().unwrap() = "at-3".into();
    // A revoked grant: the 401's refresh fails, the server says so.
    let st = svc.oauth_status(&server.id).await;
    assert!(st.signed_in);
    let _ = svc
        .invoke(
            &server.id,
            "list_items",
            &json!({}),
            &ctx(&server.workspace_id),
        )
        .await;
    let row = svc.registry().get(&server.id).await.unwrap();
    assert!(
        row.health_error
            .as_deref()
            .unwrap_or("")
            .starts_with("Needs sign-in"),
        "{:?}",
        row.health_error
    );
    // Sign-out deletes the tokens.
    svc.oauth_sign_out(&server.id).await.unwrap();
    assert!(get_async(&store, &token_key(&server.id))
        .await
        .unwrap()
        .is_none());
    assert!(!svc.oauth_status(&server.id).await.signed_in);
    task.abort();
}
