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
use otto_core::hooks::McpServerSpec;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id};
use otto_sessions::mcp::McpServerSource;
use otto_state::{McpServersRepo, NewMcpServer};
use serde_json::{json, Value};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

fn repo(ctx: &ServerCtx) -> McpServersRepo {
    McpServersRepo::new(ctx.pool.clone())
}

async fn resource_check(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    id: &Id,
    operation: &str,
) -> ApiResult<()> {
    let server = ctx.mcp.registry().get(id).await?;
    if !ctx
        .mcp
        .resource_allowed(&server, user, "discover", None)
        .await?
    {
        return Err(Error::NotFound("MCP server".into()).into());
    }
    if !ctx
        .mcp
        .resource_allowed(&server, user, operation, None)
        .await?
    {
        return Err(Error::Forbidden("MCP resource operation denied".into()).into());
    }
    Ok(())
}

/// Keychain ref for a server's secret blob — the Control Plane's convention
/// (`otto_mcp::McpService::secret_ref`), duplicated here to avoid a crate
/// dependency for one format string. The blob is `{"env":{…},"headers":{…}}`.
fn secret_ref(id: &str) -> String {
    format!("mcp-{id}")
}

/// A stored secret blob from a Keychain lookup. ONLY a missing entry
/// (`Ok(None)`) means "no secrets" (`{}`); a failed read (a locked or denied
/// Keychain) or a blob that doesn't parse is an `Err` (PC-6). Folding those to
/// `{}` launched the server without its token and, on a PATCH, rewrote the
/// blob without the stored values (or deleted it outright). Pure — tested.
fn parse_secret_blob(lookup: otto_core::Result<Option<String>>, id: &str) -> Result<Value, String> {
    match lookup {
        Ok(None) => Ok(json!({})),
        Ok(Some(raw)) => serde_json::from_str::<Value>(&raw)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| format!("the stored secrets of mcp server {id} don't parse")),
        Err(e) => Err(format!("Keychain read for mcp server {id} failed: {e}")),
    }
}

/// Read a server's secret blob (see [`parse_secret_blob`]).
#[allow(clippy::disallowed_methods)] // sync McpProvider path: enabled_servers is blocking by contract (already blocks on a bridge thread)
fn load_secret_blob(secrets: &dyn SecretStore, id: &str) -> Result<Value, String> {
    parse_secret_blob(secrets.get(&secret_ref(id)), id)
}

/// [`load_secret_blob`] off the runtime thread, for the routes.
async fn read_secret_blob(secrets: &Arc<dyn SecretStore>, id: &str) -> Result<Value, String> {
    parse_secret_blob(
        otto_core::secrets::get_async(secrets, &secret_ref(id)).await,
        id,
    )
}

/// `PATCH /mcp-servers/{id}` error: an ordinary [`ApiError`], or the Keychain
/// refusing to read the server's stored secrets — 503 "Keychain unavailable"
/// with NOTHING written (PC-6), since the "KEY= keeps it" sentinels can't be
/// resolved without the stored values.
#[derive(Debug)]
pub enum UpdateError {
    Api(crate::error::ApiError),
    KeychainUnavailable,
}

impl From<crate::error::ApiError> for UpdateError {
    fn from(e: crate::error::ApiError) -> Self {
        UpdateError::Api(e)
    }
}

impl From<Error> for UpdateError {
    fn from(e: Error) -> Self {
        UpdateError::Api(e.into())
    }
}

impl axum::response::IntoResponse for UpdateError {
    fn into_response(self) -> axum::response::Response {
        match self {
            UpdateError::Api(e) => e.into_response(),
            UpdateError::KeychainUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(otto_core::api::Problem {
                    code: "keychain_unavailable".to_string(),
                    message: "Keychain unavailable".to_string(),
                }),
            )
                .into_response(),
        }
    }
}

/// Replace the stored `blob`'s `env` part (preserving `headers`, which the
/// Control Plane owns) and persist secret metadata onto the row. An EMPTY
/// value for a key keeps the currently stored value (the UI's "KEY= keeps it"
/// sentinel — stored values are never echoed back, so the client can't resend
/// them). An empty combined blob deletes the Keychain entry and clears the
/// row's ref. The caller reads `blob` first ([`read_secret_blob`]) so a failed
/// read refuses the request before anything is written, and holds
/// `McpService::lock_secrets` from that read until this returns, so the blob
/// can't change in between.
async fn write_secret_env(
    secrets: &std::sync::Arc<dyn SecretStore>,
    repo: &McpServersRepo,
    id: &str,
    mut blob: Value,
    secret_env: &BTreeMap<String, String>,
) -> ApiResult<McpServer> {
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
        otto_core::secrets::delete_async(secrets, &sref).await?;
        Ok(repo.set_secret_meta(&id.to_string(), None, &keys).await?)
    } else {
        otto_core::secrets::put_async(secrets, &sref, &blob.to_string()).await?;
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
    let mut visible = Vec::new();
    for mut server in repo(&ctx).list_for_ws(&ws_id).await? {
        if resource_check(&ctx, &user, &server.id, "discover")
            .await
            .is_err()
        {
            continue;
        }
        if resource_check(&ctx, &user, &server.id, "configure")
            .await
            .is_err()
        {
            server.command.clear();
            server.args.clear();
            server.env.clear();
            server.secret_ref = None;
            server.secret_env_keys.clear();
        }
        visible.push(server);
    }
    Ok(Json(visible))
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
    if !otto_core::auth::root_authority(&user) {
        return Err(Error::Forbidden(
            "only the owner can attach MCP credentials or commands".into(),
        )
        .into());
    }
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("mcp server name must not be empty".into()).into());
    }
    if req.command.trim().is_empty() {
        return Err(Error::Invalid("mcp server command must not be empty".into()).into());
    }
    // An env key a session's `sh` env file can't assign would leave the
    // server silently out of every session: refuse it here, with the rule.
    if let Some(msg) = otto_mcp::service::env_key_error(req.env.keys().chain(req.secret_env.keys()))
    {
        return Err(Error::Invalid(msg).into());
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
            created_by: user.id.clone(),
        })
        .await?;
    if !req.secret_env.is_empty() {
        // A brand-new id has no stored blob to merge with.
        server =
            write_secret_env(&ctx.secrets, &repo, &server.id, json!({}), &req.secret_env).await?;
    }
    let operations: Vec<String> =
        otto_core::access::operations_for(otto_core::access::ResourceKind::McpServer)
            .iter()
            .map(|s| s.to_string())
            .collect();
    otto_state::ResourceAccessRepo::new(ctx.pool.clone())
        .initialize_owner_policy(
            otto_core::access::ResourceKind::McpServer,
            &server.id,
            &user.id,
            &operations,
            &[],
            &otto_core::access::AccessActor {
                real_user_id: user.id.clone(),
                effective_user_id: None,
            },
        )
        .await?;
    Ok(Json(server))
}

/// `PATCH /api/v1/mcp-servers/{id}` — ws editor (workspace resolved from the row).
pub async fn update(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateMcpServerReq>,
) -> Result<Json<McpServer>, UpdateError> {
    let repo = repo(&ctx);
    let existing = repo.get(&id).await?;
    resource_check(&ctx, &user, &id, "configure").await?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    if !otto_core::auth::root_authority(&user)
        && (req.command.as_ref().is_some_and(|v| v != &existing.command)
            || req.args.as_ref().is_some_and(|v| v != &existing.args)
            || req.env.as_ref().is_some_and(|v| v != &existing.env)
            || req.secret_env.is_some())
    {
        return Err(Error::Forbidden(
            "only the owner can change MCP credentials or commands".into(),
        )
        .into());
    }
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
    // Only the maps this save supplies: a stored key never blocks another edit.
    let supplied_env = req.env.iter().chain(req.secret_env.iter());
    if let Some(msg) = otto_mcp::service::env_key_error(supplied_env.flat_map(|m| m.keys())) {
        return Err(Error::Invalid(msg).into());
    }
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
    // The per-server secret lock, held from this read through
    // `set_secret_meta` in `write_secret_env`: the MCP Control Plane writes the
    // `headers` half of the same blob under it, so neither save can revert
    // the other's map (CA-5-03).
    let _secrets_guard = match &req.secret_env {
        Some(_) => Some(ctx.mcp.lock_secrets(&id).await),
        None => None,
    };
    // Read the stored secrets BEFORE any write: a failed read is a 503 that
    // leaves the row and the Keychain exactly as they were.
    let stored_blob = match &req.secret_env {
        Some(_) => match read_secret_blob(&ctx.secrets, &id).await {
            Ok(blob) => Some(blob),
            Err(e) => {
                tracing::warn!(server = %id, "mcp server PATCH refused: {e}");
                return Err(UpdateError::KeychainUnavailable);
            }
        },
        None => None,
    };
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
    if let (Some(secret_env), Some(blob)) = (&req.secret_env, stored_blob) {
        server = write_secret_env(&ctx.secrets, &repo, &id, blob, secret_env).await?;
    }
    // Disabled or renamed: the old name's launcher env files (which hold the
    // resolved secrets) no longer back any entry; drop them in every cwd now
    // rather than waiting for a spawn in each.
    if (existing.enabled && !server.enabled) || existing.name != server.name {
        sweep_env_files(existing.name).await;
    }
    Ok(Json(server))
}

/// [`otto_sessions::mcp::sweep_launcher_env_files`] off the runtime thread.
async fn sweep_env_files(name: String) {
    let _ =
        tokio::task::spawn_blocking(move || otto_sessions::mcp::sweep_launcher_env_files(&name))
            .await;
}

/// `DELETE /api/v1/mcp-servers/{id}` — ws editor (workspace resolved from the row).
pub async fn delete(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let repo = repo(&ctx);
    let existing = repo.get(&id).await?;
    resource_check(&ctx, &user, &id, "configure").await?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    repo.delete(&id).await?;
    // Best-effort: drop the server's Keychain blob with it, and the launcher
    // env files that carry its resolved values.
    let _ = otto_core::secrets::delete_async(&ctx.secrets, &secret_ref(&id)).await;
    sweep_env_files(existing.name).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `McpServerSource` backed by the SQLite repo: resolves a workspace's enabled
/// servers for the session manager to reconcile into `.mcp.json` at spawn. Sync
/// trait over an async repo, so it blocks briefly on a bridge thread — fine for
/// the handful of rows per workspace. A lookup failure is returned as `Err`,
/// never as an empty list: "nothing enabled" REMOVES every managed user entry
/// from the cwd-shared launcher files, so the session manager must be able to
/// tell the two apart (it preserves the entries and records the error instead).
///
/// Keychain-backed secret env values are resolved HERE, at merge time. They
/// never reach `.mcp.json` (or agy's / grok's launcher): the entry execs
/// through `/bin/sh` with the env in a 0600 `<tmp>/otto-mcp/launch-*.env`
/// file, which reconcile deletes once the server leaves the launcher and the
/// delete/disable routes sweep. Otto's DB holds none of the values. The
/// mechanism is documented in the connections feature guide.
#[derive(Clone)]
pub struct DbMcpServerProvider {
    pool: otto_state::DbPool,
    secrets: Arc<dyn SecretStore>,
}

impl DbMcpServerProvider {
    pub fn new(pool: otto_state::DbPool, secrets: Arc<dyn SecretStore>) -> Self {
        Self { pool, secrets }
    }
}

impl McpServerSource for DbMcpServerProvider {
    fn enabled_servers(&self, workspace_id: &str) -> Result<Vec<McpServerSpec>, String> {
        let repo = McpServersRepo::new(self.pool.clone());
        let ws = workspace_id.to_string();
        // Bridge the async repo onto the calling thread without holding the
        // runtime: spawn the query and block this thread on its result. (The
        // trait is sync by design — `block_in_place` would panic on the
        // current-thread runtimes tests run under, so the bridge thread stays.)
        let servers = std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("build runtime: {e}"))?;
                rt.block_on(repo.list_enabled(&ws))
                    .map_err(|e| format!("list enabled mcp servers: {e}"))
            })
            .join()
            .unwrap_or_else(|_| Err("mcp server lookup thread panicked".into()))
        });
        // "no servers enabled" (Ok, possibly empty) vs "lookup failed" (Err):
        // only the latter is a fault, and it must reach the logs AND the caller.
        // A Keychain read that fails is a lookup failure too: launching the
        // server without its token would surface as confusing downstream auth
        // errors, so reconcile keeps the entries it has instead (PC-6).
        let specs = servers.and_then(|rows| {
            rows.into_iter()
                .map(|r| {
                    let env = if r.secret_env_keys.is_empty() {
                        r.env
                    } else {
                        let blob = load_secret_blob(self.secrets.as_ref(), &r.id)?;
                        merge_secret_env(r.env, &r.secret_env_keys, &blob)
                    };
                    Ok(McpServerSpec {
                        name: r.name,
                        command: r.command,
                        args: r.args,
                        env,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        });
        match specs {
            Ok(specs) => Ok(specs),
            Err(e) => {
                tracing::warn!(workspace = %workspace_id, "mcp servers unavailable for spawn: {e}");
                Err(e)
            }
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
#[allow(clippy::disallowed_methods)] // tests: plain sync fs / process / secret store is fine
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

    async fn mk_repo() -> (otto_state::DbPool, McpServersRepo, Id, Id) {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = otto_state::DbPool::from(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap(),
        );
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let ws = otto_core::new_id();
        let user = otto_core::new_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, '', ?)",
        )
        .bind(&user)
        .bind(format!("u-{user}"))
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'ws', '/tmp', ?)",
        )
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
        let store: std::sync::Arc<dyn SecretStore> =
            std::sync::Arc::new(MemStore(std::sync::Mutex::new(BTreeMap::new())));

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
            load_secret_blob(store.as_ref(), &server.id).unwrap(),
            &BTreeMap::from([("API_TOKEN".to_string(), "tk-9".to_string())]),
        )
        .await
        .unwrap();
        assert_eq!(s.secret_env_keys, vec!["API_TOKEN".to_string()]);
        assert_eq!(
            s.secret_ref.as_deref(),
            Some(secret_ref(&server.id).as_str())
        );
        assert_eq!(s.env.get("REGION").map(String::as_str), Some("eu"));
        let blob = store.get(&secret_ref(&server.id)).unwrap().unwrap();
        assert!(blob.contains("tk-9"));

        // Empty value = keep-sentinel: the stored value survives a re-save.
        let s = write_secret_env(
            &store,
            &repo,
            &server.id,
            load_secret_blob(store.as_ref(), &server.id).unwrap(),
            &BTreeMap::from([("API_TOKEN".to_string(), String::new())]),
        )
        .await
        .unwrap();
        assert_eq!(s.secret_env_keys, vec!["API_TOKEN".to_string()]);
        assert!(store
            .get(&secret_ref(&server.id))
            .unwrap()
            .unwrap()
            .contains("tk-9"));

        // Clearing the set deletes the Keychain entry and the row's ref.
        let blob = load_secret_blob(store.as_ref(), &server.id).unwrap();
        let s = write_secret_env(&store, &repo, &server.id, blob, &BTreeMap::new())
            .await
            .unwrap();
        assert!(s.secret_env_keys.is_empty());
        assert!(s.secret_ref.is_none());
        assert!(store.get(&secret_ref(&server.id)).unwrap().is_none());
    }

    #[tokio::test]
    async fn provider_merge_resolves_secret_env_for_mcp_json() {
        let (pool, repo, ws, user) = mk_repo().await;
        let store: std::sync::Arc<dyn SecretStore> =
            std::sync::Arc::new(MemStore(std::sync::Mutex::new(BTreeMap::new())));

        let server = repo
            .create(NewMcpServer {
                workspace_id: ws.clone(),
                name: "jira".into(),
                command: "npx".into(),
                args: vec![],
                env: BTreeMap::from([("BASE".to_string(), "https://x".to_string())]),
                enabled: true,
                created_by: user.clone(),
            })
            .await
            .unwrap();
        write_secret_env(
            &store,
            &repo,
            &server.id,
            json!({}),
            &BTreeMap::from([("TOKEN".to_string(), "s3cr3t".to_string())]),
        )
        .await
        .unwrap();

        assert!(
            repo.list_enabled(&ws).await.unwrap().is_empty(),
            "governed servers never render raw credentials"
        );
        let access = otto_state::ResourceAccessRepo::new(pool);
        let old = access
            .get_policy(otto_core::access::ResourceKind::McpServer, &server.id)
            .await
            .unwrap();
        let mut legacy = old.clone();
        legacy.mode = otto_core::access::AccessMode::Legacy;
        access
            .put_policy(
                &legacy,
                old.revision,
                &otto_core::access::AccessActor {
                    real_user_id: user,
                    effective_user_id: None,
                },
            )
            .await
            .unwrap();
        // The legacy row (list_enabled) carries no secret value…
        let rows = repo.list_enabled(&ws).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].env.contains_key("TOKEN"));
        assert_eq!(rows[0].secret_env_keys, vec!["TOKEN".to_string()]);

        // …and the merge (what the provider renders into .mcp.json) does.
        let blob = load_secret_blob(store.as_ref(), &server.id).unwrap();
        let env = merge_secret_env(rows[0].env.clone(), &rows[0].secret_env_keys, &blob);
        assert_eq!(env.get("BASE").map(String::as_str), Some("https://x"));
        assert_eq!(env.get("TOKEN").map(String::as_str), Some("s3cr3t"));
    }

    /// A Keychain that refuses every read (locked after sleep, access denied).
    struct DeniedStore;
    impl SecretStore for DeniedStore {
        fn put(&self, _: &str, _: &str) -> otto_core::Result<()> {
            panic!("nothing may be written after a failed read")
        }
        fn get(&self, _: &str) -> otto_core::Result<Option<String>> {
            Err(Error::Internal("errSecInteractionNotAllowed".into()))
        }
        fn delete(&self, _: &str) -> otto_core::Result<()> {
            panic!("nothing may be deleted after a failed read")
        }
    }

    #[test]
    fn only_a_missing_entry_means_no_secrets() {
        assert_eq!(parse_secret_blob(Ok(None), "s").unwrap(), json!({}));
        assert_eq!(
            parse_secret_blob(Ok(Some(r#"{"env":{"T":"v"}}"#.into())), "s").unwrap()["env"]["T"],
            "v"
        );
        assert!(parse_secret_blob(Ok(Some("not json".into())), "s").is_err());
        assert!(parse_secret_blob(Ok(Some("[1]".into())), "s").is_err());
        assert!(parse_secret_blob(Err(Error::Internal("locked".into())), "s").is_err());
    }

    #[tokio::test]
    async fn keychain_read_failure_is_a_lookup_failure_not_no_secrets() {
        let (pool, repo, ws, user) = mk_repo().await;
        let server = repo
            .create(NewMcpServer {
                workspace_id: ws.clone(),
                name: "jira".into(),
                command: "npx".into(),
                args: vec![],
                env: BTreeMap::new(),
                enabled: true,
                created_by: user.clone(),
            })
            .await
            .unwrap();
        repo.set_secret_meta(
            &server.id,
            Some(&secret_ref(&server.id)),
            &["TOKEN".to_string()],
        )
        .await
        .unwrap();
        // A legacy (ungoverned) row reaches the merge, as in the test above.
        let access = otto_state::ResourceAccessRepo::new(pool.clone());
        let old = access
            .get_policy(otto_core::access::ResourceKind::McpServer, &server.id)
            .await
            .unwrap();
        let mut legacy = old.clone();
        legacy.mode = otto_core::access::AccessMode::Legacy;
        access
            .put_policy(
                &legacy,
                old.revision,
                &otto_core::access::AccessActor {
                    real_user_id: user,
                    effective_user_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(repo.list_enabled(&ws).await.unwrap().len(), 1);
        let denied: Arc<dyn SecretStore> = Arc::new(DeniedStore);
        // The route's read refuses (→ 503, before any write)…
        assert!(read_secret_blob(&denied, &server.id).await.is_err());
        // …and the spawn-time provider reports a lookup failure instead of
        // launching the server without its token, so reconcile keeps entries.
        let provider = DbMcpServerProvider::new(pool, denied);
        let res = tokio::task::spawn_blocking(move || provider.enabled_servers(&ws))
            .await
            .unwrap();
        assert!(res.is_err(), "launched without its token: {res:?}");
    }

    #[test]
    fn keychain_unavailable_is_a_503() {
        use axum::response::IntoResponse;
        let resp = UpdateError::KeychainUnavailable.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
