//! Workspace MCP-server config CRUD. Users manage the MCP servers that get
//! merged into a workspace's `.mcp.json` when an agent session spawns there
//! (alongside Otto's own managed browser entry — see `otto-sessions::mcp`).
//!
//! Nothing here is auto-enabled: `enabled` defaults to `false` on create, and a
//! server is only written to `.mcp.json` once the user flips it on and a session
//! then spawns. Reads = `ws viewer`, mutations = `ws editor`. Item routes
//! resolve the owning workspace from the row.
//!
//! Secret env values never touch the row: they live in the macOS Keychain
//! under the SAME per-server blob the MCP Control Plane uses (`mcp-{id}`,
//! shape `{"env":{…},"headers":{…}}` — the two surfaces share `mcp_servers`
//! rows, so they must share the secret convention too). GET returns key names
//! only; values are resolved exclusively when `.mcp.json` is rendered at
//! agent spawn (see [`DbMcpServerProvider`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{CreateMcpServerReq, UpdateMcpServerReq};
use otto_core::domain::{McpServer, WorkspaceRole};
use otto_core::hooks::{McpServerProvider, McpServerSpec};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id};
use otto_state::{McpServersRepo, NewMcpServer};
use serde_json::{json, Value};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

fn repo(ctx: &ServerCtx) -> McpServersRepo {
    McpServersRepo::new(ctx.pool.clone())
}

/// Keychain ref for a server's secret blob — the Control Plane's convention
/// (`otto_mcp::McpService::secret_ref`), duplicated here to avoid a crate
/// dependency for one format string. The blob is `{"env":{…},"headers":{…}}`.
fn secret_ref(id: &str) -> String {
    format!("mcp-{id}")
}

/// Read a server's secret blob; absent/corrupt → empty object.
fn load_secret_blob(secrets: &dyn SecretStore, id: &str) -> Value {
    secrets
        .get(&secret_ref(id))
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

/// Replace the blob's `env` part (preserving `headers`, which the Control
/// Plane owns) and persist secret metadata onto the row. An EMPTY value for a
/// key keeps the currently stored value (the UI's "KEY= keeps it" sentinel —
/// stored values are never echoed back, so the client can't resend them). An
/// empty combined blob deletes the Keychain entry and clears the row's ref.
async fn write_secret_env(
    secrets: &dyn SecretStore,
    repo: &McpServersRepo,
    id: &str,
    secret_env: &BTreeMap<String, String>,
) -> ApiResult<McpServer> {
    let mut blob = load_secret_blob(secrets, id);
    let previous = blob
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut merged: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in secret_env {
        if !v.is_empty() {
            merged.insert(k.clone(), v.clone());
        } else if let Some(prev) = previous.get(k).and_then(Value::as_str) {
            merged.insert(k.clone(), prev.to_string());
        }
        // empty value with nothing stored → key dropped (can't keep nothing)
    }
    let secret_env = &merged;
    blob["env"] = serde_json::to_value(secret_env).unwrap_or_else(|_| json!({}));
    let headers_empty = blob
        .get("headers")
        .and_then(Value::as_object)
        .is_none_or(|h| h.is_empty());
    let keys: Vec<String> = secret_env.keys().cloned().collect();
    let sref = secret_ref(id);
    if secret_env.is_empty() && headers_empty {
        secrets.delete(&sref)?;
        Ok(repo.set_secret_meta(&id.to_string(), None, &keys).await?)
    } else {
        secrets.put(&sref, &blob.to_string())?;
        Ok(repo
            .set_secret_meta(&id.to_string(), Some(&sref), &keys)
            .await?)
    }
}

/// `GET /api/v1/workspaces/{id}/mcp-servers` — ws viewer.
pub async fn list(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<McpServer>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_for_ws(&ws_id).await?))
}

/// `POST /api/v1/workspaces/{id}/mcp-servers` — ws editor. `enabled` defaults
/// off; a server is never auto-enabled.
pub async fn create(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateMcpServerReq>,
) -> ApiResult<Json<McpServer>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("mcp server name must not be empty".into()).into());
    }
    if req.command.trim().is_empty() {
        return Err(Error::Invalid("mcp server command must not be empty".into()).into());
    }
    // A key can't be both plaintext and secret: the secret wins, the row copy
    // is dropped.
    let mut env = req.env;
    for k in req.secret_env.keys() {
        env.remove(k);
    }
    let repo = repo(&ctx);
    let mut server = repo
        .create(NewMcpServer {
            workspace_id: ws_id,
            name: name.to_string(),
            command: req.command.trim().to_string(),
            args: req.args,
            env,
            enabled: req.enabled,
            created_by: user.id,
        })
        .await?;
    if !req.secret_env.is_empty() {
        server = write_secret_env(ctx.secrets.as_ref(), &repo, &server.id, &req.secret_env).await?;
    }
    Ok(Json(server))
}

/// `PATCH /api/v1/mcp-servers/{id}` — ws editor (workspace resolved from the row).
pub async fn update(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateMcpServerReq>,
) -> ApiResult<Json<McpServer>> {
    let repo = repo(&ctx);
    let existing = repo.get(&id).await?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    let name = match req.name.as_deref().map(str::trim) {
        Some("") => return Err(Error::Invalid("mcp server name must not be empty".into()).into()),
        other => other,
    };
    let command = match req.command.as_deref().map(str::trim) {
        Some("") => {
            return Err(Error::Invalid("mcp server command must not be empty".into()).into())
        }
        other => other,
    };
    // Same plaintext/secret exclusivity as create, against the effective
    // secret-key set (the incoming replacement, else the stored one).
    let effective_secret_keys: Vec<String> = match &req.secret_env {
        Some(m) => m.keys().cloned().collect(),
        None => existing.secret_env_keys.clone(),
    };
    let env = req.env.map(|mut m| {
        for k in &effective_secret_keys {
            m.remove(k);
        }
        m
    });
    let mut server = repo
        .update(
            &id,
            name,
            command,
            req.args.as_deref(),
            env.as_ref(),
            req.enabled,
        )
        .await?;
    if let Some(secret_env) = &req.secret_env {
        server = write_secret_env(ctx.secrets.as_ref(), &repo, &id, secret_env).await?;
    }
    Ok(Json(server))
}

/// `DELETE /api/v1/mcp-servers/{id}` — ws editor (workspace resolved from the row).
pub async fn delete(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let repo = repo(&ctx);
    let existing = repo.get(&id).await?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    repo.delete(&id).await?;
    // Best-effort: drop the server's Keychain blob with it.
    let _ = ctx.secrets.delete(&secret_ref(&id));
    Ok(StatusCode::NO_CONTENT)
}

/// `McpServerProvider` backed by the SQLite repo: resolves a workspace's enabled
/// servers for the session manager to merge into `.mcp.json` at spawn. Sync trait
/// over an async repo, so it blocks briefly on the current Tokio runtime — fine
/// for the handful of rows per workspace, and best-effort (errors → empty).
///
/// Keychain-backed secret env values are resolved HERE, at merge time — the
/// rendered `.mcp.json` on disk still contains real values (the agent CLI
/// needs them; the file is user-local and out-of-tree), but Otto's DB no
/// longer does. That residual is documented in the connections feature guide.
#[derive(Clone)]
pub struct DbMcpServerProvider {
    pool: otto_state::SqlitePool,
    secrets: Arc<dyn SecretStore>,
}

impl DbMcpServerProvider {
    pub fn new(pool: otto_state::SqlitePool, secrets: Arc<dyn SecretStore>) -> Self {
        Self { pool, secrets }
    }
}

impl McpServerProvider for DbMcpServerProvider {
    fn enabled_servers(&self, workspace_id: &str) -> Vec<McpServerSpec> {
        let repo = McpServersRepo::new(self.pool.clone());
        let ws = workspace_id.to_string();
        // Bridge the async repo onto the calling thread without holding the
        // runtime: spawn the query and block this thread on its result.
        let servers = std::thread::scope(|s| {
            s.spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()
                    .and_then(|rt| rt.block_on(repo.list_enabled(&ws)).ok())
            })
            .join()
            .ok()
            .flatten()
        });
        match servers {
            Some(rows) => rows
                .into_iter()
                .map(|r| {
                    let env = if r.secret_env_keys.is_empty() {
                        r.env
                    } else {
                        let blob = load_secret_blob(self.secrets.as_ref(), &r.id);
                        merge_secret_env(r.env, &r.secret_env_keys, &blob)
                    };
                    McpServerSpec {
                        name: r.name,
                        command: r.command,
                        args: r.args,
                        env,
                    }
                })
                .collect(),
            None => Vec::new(),
        }
    }
}

/// Overlay the Keychain blob's `env` values (for the row's declared secret
/// keys) onto the non-secret env — the map that lands in `.mcp.json`.
fn merge_secret_env(
    mut env: BTreeMap<String, String>,
    secret_env_keys: &[String],
    blob: &Value,
) -> BTreeMap<String, String> {
    if let Some(senv) = blob.get("env").and_then(Value::as_object) {
        for key in secret_env_keys {
            if let Some(v) = senv.get(key).and_then(Value::as_str) {
                env.insert(key.clone(), v.to_string());
            }
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_state::NewMcpServer;

    struct MemStore(std::sync::Mutex<BTreeMap<String, String>>);
    impl SecretStore for MemStore {
        fn put(&self, k: &str, v: &str) -> otto_core::Result<()> {
            self.0.lock().unwrap().insert(k.into(), v.into());
            Ok(())
        }
        fn get(&self, k: &str) -> otto_core::Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(k).cloned())
        }
        fn delete(&self, k: &str) -> otto_core::Result<()> {
            self.0.lock().unwrap().remove(k);
            Ok(())
        }
    }

    async fn mk_repo() -> (otto_state::SqlitePool, McpServersRepo, Id, Id) {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let ws = otto_core::new_id();
        let user = otto_core::new_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, '', ?)")
            .bind(&user)
            .bind(format!("u-{user}"))
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'ws', '/tmp', ?)")
            .bind(&ws)
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();
        (pool.clone(), McpServersRepo::new(pool), ws, user)
    }

    #[tokio::test]
    async fn secret_env_blob_write_keep_and_clear() {
        let (_pool, repo, ws, user) = mk_repo().await;
        let store = MemStore(std::sync::Mutex::new(BTreeMap::new()));

        let server = repo
            .create(NewMcpServer {
                workspace_id: ws.clone(),
                name: "linear".into(),
                command: "npx".into(),
                args: vec![],
                env: BTreeMap::from([("REGION".to_string(), "eu".to_string())]),
                enabled: false,
                created_by: user,
            })
            .await
            .unwrap();

        // Write a secret: value in the store, key name on the row, env untouched.
        let s = write_secret_env(
            &store,
            &repo,
            &server.id,
            &BTreeMap::from([("API_TOKEN".to_string(), "tk-9".to_string())]),
        )
        .await
        .unwrap();
        assert_eq!(s.secret_env_keys, vec!["API_TOKEN".to_string()]);
        assert_eq!(s.secret_ref.as_deref(), Some(secret_ref(&server.id).as_str()));
        assert_eq!(s.env.get("REGION").map(String::as_str), Some("eu"));
        let blob = store.get(&secret_ref(&server.id)).unwrap().unwrap();
        assert!(blob.contains("tk-9"));

        // Empty value = keep-sentinel: the stored value survives a re-save.
        let s = write_secret_env(
            &store,
            &repo,
            &server.id,
            &BTreeMap::from([("API_TOKEN".to_string(), String::new())]),
        )
        .await
        .unwrap();
        assert_eq!(s.secret_env_keys, vec!["API_TOKEN".to_string()]);
        assert!(store.get(&secret_ref(&server.id)).unwrap().unwrap().contains("tk-9"));

        // Clearing the set deletes the Keychain entry and the row's ref.
        let s = write_secret_env(&store, &repo, &server.id, &BTreeMap::new())
            .await
            .unwrap();
        assert!(s.secret_env_keys.is_empty());
        assert!(s.secret_ref.is_none());
        assert!(store.get(&secret_ref(&server.id)).unwrap().is_none());
    }

    #[tokio::test]
    async fn provider_merge_resolves_secret_env_for_mcp_json() {
        let (_pool, repo, ws, user) = mk_repo().await;
        let store = MemStore(std::sync::Mutex::new(BTreeMap::new()));

        let server = repo
            .create(NewMcpServer {
                workspace_id: ws.clone(),
                name: "jira".into(),
                command: "npx".into(),
                args: vec![],
                env: BTreeMap::from([("BASE".to_string(), "https://x".to_string())]),
                enabled: true,
                created_by: user,
            })
            .await
            .unwrap();
        write_secret_env(
            &store,
            &repo,
            &server.id,
            &BTreeMap::from([("TOKEN".to_string(), "s3cr3t".to_string())]),
        )
        .await
        .unwrap();

        // The row (list_enabled) carries no secret value…
        let rows = repo.list_enabled(&ws).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].env.get("TOKEN").is_none());
        assert_eq!(rows[0].secret_env_keys, vec!["TOKEN".to_string()]);

        // …and the merge (what the provider renders into .mcp.json) does.
        let blob = load_secret_blob(&store, &server.id);
        let env = merge_secret_env(rows[0].env.clone(), &rows[0].secret_env_keys, &blob);
        assert_eq!(env.get("BASE").map(String::as_str), Some("https://x"));
        assert_eq!(env.get("TOKEN").map(String::as_str), Some("s3cr3t"));
    }
}
