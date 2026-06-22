//! API client ("Postman" section) endpoints, workspace-scoped under
//! `/api/v1/workspaces/{wid}/api-client/...` (plus the workspace-agnostic
//! `POST /api-client/import-curl`). Editor role is required for every
//! workspace-scoped route; mutations and reads alike (the API client is a
//! collaborative authoring surface).
//!
//! Execution runs through the daemon via a shared `reqwest` client (mirroring
//! the browser proxy), so requests dodge webview CORS/CSP and get real timing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{
    ApiResponse, ApiRunResult, ApiRunStepResult, ExecuteApiReq, ImportCurlReq, ParsedCurl,
    UpsertApiAutomationReq, UpsertApiCollectionReq, UpsertApiEnvironmentReq, UpsertApiRequestReq,
};
use otto_core::domain::{
    ApiAutomation, ApiCollection, ApiEnvironment, ApiHistoryEntry, ApiRequest, Connection,
    ConnectionKind, WorkspaceRole,
};
use otto_ssh::{SshTunnel, SshTunnelConfig};
use otto_core::{Error, Id};
use otto_state::{
    ApiClientRepo, NewApiAutomation, NewApiCollection, NewApiEnvironment, NewApiHistory,
    NewApiRequest,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api_helpers::{
    collection_to_openapi, eval_assertion, json_path, parse_curl, percent_decode,
};
use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Hard cap for `GET .../history?limit=`.
const HISTORY_MAX: i64 = 500;
const HISTORY_DEFAULT: i64 = 100;
/// Execution timeout for outbound requests.
const EXECUTE_TIMEOUT: Duration = Duration::from_secs(60);

// ===========================================================================
// SSRF guard (audit S1)
// ===========================================================================

/// SSRF defense for every outbound user-URL fetch in the API-client / streaming
/// / gRPC / browser-proxy paths. The implementation now lives in the leaf
/// `otto-netguard` crate so the exact same classifier guards the Message-Brokers
/// metrics / schema-registry fetches too — one definition, no drifting copy
/// (audit S1). Re-exported here under the original `net_guard` path so existing
/// call sites stay byte-for-byte unchanged.
pub(crate) mod net_guard {
    pub(crate) use otto_netguard::{check_url, redirect_policy};
}

fn repo(ctx: &ServerCtx) -> ApiClientRepo {
    ApiClientRepo::new(ctx.pool.clone())
}

/// Shared outbound HTTP client (built once). Follows redirects, generous body
/// timeout enforced per-request via `.timeout()`.
/// Daemon-global cookie jar shared by the API client (captures Set-Cookie and
/// resends on matching requests, so login/session flows just work).
fn cookie_jar() -> std::sync::Arc<reqwest_cookie_store::CookieStoreMutex> {
    static JAR: OnceLock<std::sync::Arc<reqwest_cookie_store::CookieStoreMutex>> = OnceLock::new();
    JAR.get_or_init(|| {
        std::sync::Arc::new(reqwest_cookie_store::CookieStoreMutex::new(
            reqwest_cookie_store::CookieStore::default(),
        ))
    })
    .clone()
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("Otto-ApiClient/1.0")
            // SSRF guard: cap + re-validate each redirect hop's host so an
            // upstream 30x can't bounce us into a private/loopback address.
            .redirect(net_guard::redirect_policy())
            .cookie_provider(cookie_jar())
            .build()
            .unwrap_or_default()
    })
}

/// Build a one-off client when per-request settings deviate from the defaults
/// (disable redirects, skip TLS verification) or the request is tunnelled
/// through a SOCKS5 proxy. `None` → use the shared client. TLS verification is
/// only ever skipped when the request explicitly sets `verify_ssl=false` (never
/// the default).
fn build_settings_client(req: &ExecuteApiReq, proxy: Option<&str>) -> Option<reqwest::Client> {
    let no_redirect = req.follow_redirects == Some(false);
    let no_verify = req.verify_ssl == Some(false);
    if !no_redirect && !no_verify && proxy.is_none() {
        return None;
    }
    let mut builder = reqwest::Client::builder()
        .user_agent("Otto-ApiClient/1.0")
        .cookie_provider(cookie_jar());
    builder = if no_redirect {
        builder.redirect(reqwest::redirect::Policy::none())
    } else {
        // Redirects still follow the SSRF-guarded policy.
        builder.redirect(net_guard::redirect_policy())
    };
    if no_verify {
        builder = builder.danger_accept_invalid_certs(true);
    }
    if let Some(url) = proxy {
        // `socks5h://` so the bastion does the DNS — the target host is resolved
        // and dialled from the SSH server's network, not ours.
        if let Ok(px) = reqwest::Proxy::all(url) {
            builder = builder.proxy(px);
        }
    }
    builder.build().ok()
}

// ===========================================================================
// SSH tunnel (SOCKS5) for IP-whitelisted upstreams
// ===========================================================================

/// How long an idle cached tunnel is kept before it is torn down. Mirrors the
/// DB Explorer's tunnel cache (`otto-dbviewer`).
const TUNNEL_IDLE_TTL: Duration = Duration::from_secs(600);

struct CachedTunnel {
    tunnel: Arc<SshTunnel>,
    last_used: Instant,
}

/// Daemon-global cache of live SOCKS5 tunnels, keyed by the bastion identity
/// (`host|port|user|identity`) so distinct requests through the same bastion
/// share one `ssh` process. The child is killed on `Drop` once evicted.
fn tunnel_cache() -> &'static StdMutex<HashMap<String, CachedTunnel>> {
    static C: OnceLock<StdMutex<HashMap<String, CachedTunnel>>> = OnceLock::new();
    C.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Build an [`SshTunnelConfig`] from an `ssh`-kind connection's params. The
/// shape matches what the Connections page already stores (`host`/`port`/`user`/
/// `identity_file`); auth flows through the system `ssh` client (agent /
/// `~/.ssh/config` / identity file), so no secret is needed here.
fn ssh_config_from_conn(conn: &Connection) -> Result<SshTunnelConfig, String> {
    let p = &conn.params;
    let host = p
        .get("host")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("SSH connection '{}' has no host", conn.name))?;
    let user = p
        .get("user")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "SSH connection '{}' has no user — required to use it as an API tunnel",
                conn.name
            )
        })?;
    let port = match p.get("port") {
        None | Some(Value::Null) => 22,
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .ok_or_else(|| format!("SSH connection '{}' has an invalid port", conn.name))?,
        Some(Value::String(s)) if s.is_empty() => 22,
        Some(Value::String(s)) => s
            .parse::<u16>()
            .map_err(|_| format!("SSH connection '{}' has an invalid port", conn.name))?,
        Some(_) => return Err(format!("SSH connection '{}' has an invalid port", conn.name)),
    };
    let identity_file = p
        .get("identity_file")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(SshTunnelConfig {
        host: host.to_string(),
        port,
        user: user.to_string(),
        identity_file,
    })
}

/// Open (or reuse) a cached SOCKS5 tunnel for `cfg` and return its proxy URL
/// (`socks5h://127.0.0.1:<port>`). Dead/idle entries are evicted on access.
async fn socks_proxy_url(cfg: &SshTunnelConfig) -> Result<String, String> {
    let key = format!(
        "{}|{}|{}|{}",
        cfg.host,
        cfg.port,
        cfg.user,
        cfg.identity_file.as_deref().unwrap_or("")
    );
    // Fast path: a live cached tunnel. Evict idle/dead entries while we hold the
    // lock. The lock is never held across an await.
    {
        let mut cache = tunnel_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|_, c| c.last_used.elapsed() < TUNNEL_IDLE_TTL && c.tunnel.is_alive());
        if let Some(c) = cache.get_mut(&key) {
            c.last_used = Instant::now();
            return Ok(format!("socks5h://127.0.0.1:{}", c.tunnel.local_port()));
        }
    }
    // Slow path: open a fresh tunnel (blocks until the local SOCKS port is up),
    // then cache it. A concurrent opener racing us just replaces the entry; the
    // loser's tunnel drops (and its `ssh` child is killed) when overwritten.
    let tunnel = SshTunnel::open_socks(cfg)
        .await
        .map_err(|e| format!("SSH tunnel failed: {e}"))?;
    let port = tunnel.local_port();
    let tunnel = Arc::new(tunnel);
    {
        let mut cache = tunnel_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(
            key,
            CachedTunnel {
                tunnel,
                last_used: Instant::now(),
            },
        );
    }
    Ok(format!("socks5h://127.0.0.1:{port}"))
}

/// Resolve the SOCKS5 proxy URL for a request that opts into an SSH tunnel via
/// `ssh_connection_id`. `Ok(None)` when no tunnel is requested. The referenced
/// connection must be an `ssh`-kind profile visible to `wid` (workspace-scoped
/// or a global profile). Errors are human-readable (surfaced as a 502).
async fn resolve_socks_proxy(
    ctx: &ServerCtx,
    wid: &Id,
    ssh_connection_id: Option<&Id>,
) -> Result<Option<String>, String> {
    let Some(conn_id) = ssh_connection_id else {
        return Ok(None);
    };
    let conn = ctx
        .connections
        .get(conn_id)
        .await
        .map_err(|_| "SSH tunnel connection not found".to_string())?;
    if conn.kind != ConnectionKind::Ssh {
        return Err(format!("connection '{}' is not an SSH connection", conn.name));
    }
    // Global profiles (workspace_id = None) are visible everywhere; otherwise it
    // must belong to this workspace.
    if let Some(ws) = &conn.workspace_id {
        if ws != wid {
            return Err("SSH tunnel connection belongs to another workspace".into());
        }
    }
    let cfg = ssh_config_from_conn(&conn)?;
    Ok(Some(socks_proxy_url(&cfg).await?))
}

// ===========================================================================
// Collections
// ===========================================================================

/// `GET /workspaces/{wid}/api-client/collections`
pub async fn list_collections(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiCollection>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_collections(&wid).await?))
}

/// `POST /workspaces/{wid}/api-client/collections`
pub async fn create_collection(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiCollectionReq>,
) -> ApiResult<Json<ApiCollection>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("collection name must not be empty".into()).into());
    }
    let position = repo(&ctx).list_collections(&wid).await?.len() as i64;
    let col = repo(&ctx)
        .create_collection(NewApiCollection {
            workspace_id: wid,
            name: req.name.trim().to_string(),
            parent_id: req.parent_id,
            position,
        })
        .await?;
    Ok(Json(col))
}

/// `PATCH /workspaces/{wid}/api-client/collections/{id}`
pub async fn update_collection(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiCollectionReq>,
) -> ApiResult<Json<ApiCollection>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_collection(&id).await?.workspace_id, &wid)?;
    let col = repo
        .update_collection(&id, Some(req.name.trim()), Some(req.parent_id.as_deref()))
        .await?;
    Ok(Json(col))
}

/// `DELETE /workspaces/{wid}/api-client/collections/{id}`
pub async fn delete_collection(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_collection(&id).await?.workspace_id, &wid)?;
    repo.delete_collection(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ===========================================================================
// Requests
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct RequestsFilter {
    pub collection_id: Option<Id>,
}

/// `GET /workspaces/{wid}/api-client/requests` (?collection_id)
pub async fn list_requests(
    Path(wid): Path<Id>,
    Query(filter): Query<RequestsFilter>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiRequest>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(
        repo(&ctx)
            .list_requests(&wid, filter.collection_id.as_ref())
            .await?,
    ))
}

/// `GET /workspaces/{wid}/api-client/requests/{id}`
pub async fn get_request(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiRequest>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let req = repo(&ctx).get_request(&id).await?;
    ensure_in_workspace(&req.workspace_id, &wid)?;
    Ok(Json(req))
}

/// `POST /workspaces/{wid}/api-client/requests`
pub async fn create_request(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiRequestReq>,
) -> ApiResult<Json<ApiRequest>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("request name must not be empty".into()).into());
    }
    let position = repo(&ctx)
        .list_requests(&wid, req.collection_id.as_ref())
        .await?
        .len() as i64;
    let created = repo(&ctx)
        .create_request(req_to_new(&wid, req, position))
        .await?;
    Ok(Json(created))
}

/// `PATCH /workspaces/{wid}/api-client/requests/{id}`
pub async fn update_request(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiRequestReq>,
) -> ApiResult<Json<ApiRequest>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    let existing = repo.get_request(&id).await?;
    ensure_in_workspace(&existing.workspace_id, &wid)?;
    let updated = repo
        .update_request(&id, req_to_new(&wid, req, existing.position))
        .await?;
    Ok(Json(updated))
}

/// `DELETE /workspaces/{wid}/api-client/requests/{id}`
pub async fn delete_request(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_request(&id).await?.workspace_id, &wid)?;
    repo.delete_request(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn req_to_new(wid: &Id, req: UpsertApiRequestReq, position: i64) -> NewApiRequest {
    NewApiRequest {
        workspace_id: wid.clone(),
        collection_id: req.collection_id,
        name: req.name.trim().to_string(),
        method: req.method.to_uppercase(),
        url: req.url,
        headers: normalize_json_array(req.headers),
        query: normalize_json_array(req.query),
        body_mode: req.body_mode,
        body: req.body,
        auth: normalize_json_object(req.auth),
        ssh_connection_id: req.ssh_connection_id,
        position,
    }
}

// ===========================================================================
// Environments
// ===========================================================================

/// `GET /workspaces/{wid}/api-client/environments`
pub async fn list_environments(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiEnvironment>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_environments(&wid).await?))
}

/// `POST /workspaces/{wid}/api-client/environments`
pub async fn create_environment(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiEnvironmentReq>,
) -> ApiResult<Json<ApiEnvironment>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("environment name must not be empty".into()).into());
    }
    let env = repo(&ctx)
        .create_environment(NewApiEnvironment {
            workspace_id: wid,
            name: req.name.trim().to_string(),
            variables: normalize_json_object(req.variables),
        })
        .await?;
    Ok(Json(env))
}

/// `PATCH /workspaces/{wid}/api-client/environments/{id}`
pub async fn update_environment(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiEnvironmentReq>,
) -> ApiResult<Json<ApiEnvironment>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_environment(&id).await?.workspace_id, &wid)?;
    let vars = normalize_json_object(req.variables);
    let env = repo
        .update_environment(&id, Some(req.name.trim()), Some(&vars))
        .await?;
    Ok(Json(env))
}

/// `DELETE /workspaces/{wid}/api-client/environments/{id}`
pub async fn delete_environment(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_environment(&id).await?.workspace_id, &wid)?;
    repo.delete_environment(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /workspaces/{wid}/api-client/environments/{id}/activate`
pub async fn activate_environment(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiEnvironment>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_environment(&id).await?.workspace_id, &wid)?;
    Ok(Json(repo.set_active(&wid, &id).await?))
}

// ===========================================================================
// History
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct HistoryFilter {
    pub limit: Option<i64>,
}

/// `GET /workspaces/{wid}/api-client/history` (?limit)
pub async fn list_history(
    Path(wid): Path<Id>,
    Query(filter): Query<HistoryFilter>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiHistoryEntry>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let limit = filter
        .limit
        .unwrap_or(HISTORY_DEFAULT)
        .clamp(1, HISTORY_MAX);
    Ok(Json(repo(&ctx).list_history(&wid, limit).await?))
}

/// `DELETE /workspaces/{wid}/api-client/history`
pub async fn clear_history(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    repo(&ctx).clear_history(&wid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ===========================================================================
// OpenAPI export
// ===========================================================================

/// `GET /workspaces/{wid}/api-client/collections/{id}/openapi`
pub async fn export_openapi(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let repo = repo(&ctx);
    let collection = repo.get_collection(&id).await?;
    ensure_in_workspace(&collection.workspace_id, &wid)?;
    let requests = repo.list_requests(&wid, Some(&id)).await?;
    Ok(Json(collection_to_openapi(&collection, &requests)))
}

// ===========================================================================
// Import curl (workspace-agnostic)
// ===========================================================================

/// `POST /api-client/import-curl`
pub async fn import_curl(
    State(_ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
    Json(req): Json<ImportCurlReq>,
) -> ApiResult<Json<ParsedCurl>> {
    Ok(Json(parse_curl(&req.curl)?))
}

/// `GET /workspaces/{wid}/api-client/cookies` — list the shared cookie jar.
pub async fn list_cookies(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let jar = cookie_jar();
    let store = jar
        .lock()
        .map_err(|_| ApiError(Error::Internal("cookie jar poisoned".into())))?;
    let cookies: Vec<Value> = store
        .iter_any()
        .map(|c| {
            json!({
                "name": c.name(),
                "value": c.value(),
                "domain": c.domain().unwrap_or(""),
                "path": c.path().unwrap_or("/"),
            })
        })
        .collect();
    Ok(Json(Value::Array(cookies)))
}

/// `DELETE /workspaces/{wid}/api-client/cookies` — clear the shared cookie jar.
pub async fn clear_cookies(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let jar = cookie_jar();
    jar.lock()
        .map_err(|_| ApiError(Error::Internal("cookie jar poisoned".into())))?
        .clear();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct OAuth2TokenReq {
    grant: String,
    token_url: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    refresh_token: String,
}

/// `POST /workspaces/{wid}/api-client/oauth2/token` — perform an OAuth 2.0
/// token request (client_credentials / password / refresh_token) server-side
/// and return the token JSON. Authorization-code grant (browser redirect) is
/// not handled here.
pub async fn oauth2_token(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<OAuth2TokenReq>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;

    let mut form: Vec<(&str, &str)> = Vec::new();
    match req.grant.as_str() {
        "client_credentials" => form.push(("grant_type", "client_credentials")),
        "password" => {
            form.push(("grant_type", "password"));
            form.push(("username", &req.username));
            form.push(("password", &req.password));
        }
        "refresh_token" => {
            form.push(("grant_type", "refresh_token"));
            form.push(("refresh_token", &req.refresh_token));
        }
        other => {
            return Err(ApiError(Error::Invalid(format!(
                "unsupported grant: {other}"
            ))))
        }
    }
    if !req.scope.is_empty() {
        form.push(("scope", &req.scope));
    }
    if !req.client_id.is_empty() {
        form.push(("client_id", &req.client_id));
    }
    if !req.client_secret.is_empty() {
        form.push(("client_secret", &req.client_secret));
    }

    // SSRF guard: never let the token endpoint point at an internal address.
    net_guard::check_url(&req.token_url)
        .await
        .map_err(|m| ApiError(Error::Invalid(m)))?;

    let resp = http_client()
        .post(&req.token_url)
        .header("Accept", "application/json")
        .form(&form)
        .send()
        .await
        .map_err(|e| ApiError(Error::Upstream(describe_reqwest_error(&e))))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 400 || parsed.get("access_token").is_none() {
        let msg = parsed
            .get("error_description")
            .or_else(|| parsed.get("error"))
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| format!("token endpoint returned {status}: {text}"));
        return Err(ApiError(Error::Upstream(msg)));
    }
    Ok(Json(parsed))
}

// ===========================================================================
// Execute
// ===========================================================================

/// `POST /workspaces/{wid}/api-client/execute`
///
/// Resolves `{{var}}` placeholders from the selected (or active) environment,
/// builds and sends the request through the shared client, records a history
/// entry, and returns the response. Network failures map to a 502 `Problem`
/// (and are still recorded in history with a null status).
pub async fn execute(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ExecuteApiReq>,
) -> ApiResult<Json<ApiResponse>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);

    // Resolve the environment variable map, then layer runtime overrides
    // (from post-response scripts / chaining) on top.
    let mut vars = resolve_variables(&repo, &wid, req.environment_id.as_ref()).await?;
    if let Some(Value::Object(overrides)) = &req.vars {
        for (k, v) in overrides {
            vars.insert(k.clone(), v.clone());
        }
    }

    // Snapshot of the request as executed (post-substitution view recorded too).
    let request_snapshot = json!({
        "method": req.method,
        "url": req.url,
        "headers": req.headers,
        "query": req.query,
        "body_mode": req.body_mode,
        "body": req.body,
        "auth": req.auth,
        "environment_id": req.environment_id,
        "ssh_connection_id": req.ssh_connection_id,
    });

    // Resolve the optional SSH tunnel first; a resolution failure flows through
    // the same error/history path as a network failure below.
    let send = match resolve_socks_proxy(&ctx, &wid, req.ssh_connection_id.as_ref()).await {
        Ok(proxy) => build_and_send(&req, &vars, proxy.as_deref()).await,
        Err(msg) => Err(msg),
    };
    match send {
        Ok(resp) => {
            // Record success in history (best-effort; do not fail the request).
            let _ = repo
                .insert_history(NewApiHistory {
                    workspace_id: wid.clone(),
                    method: req.method.to_uppercase(),
                    url: substitute(&req.url, &vars),
                    status: Some(resp.status as i64),
                    duration_ms: Some(resp.duration_ms),
                    request: request_snapshot,
                    response: serde_json::to_value(&resp).unwrap_or(Value::Null),
                })
                .await;
            Ok(Json(resp))
        }
        Err(err) => {
            // Record the failure too, then surface as a 502.
            let _ = repo
                .insert_history(NewApiHistory {
                    workspace_id: wid.clone(),
                    method: req.method.to_uppercase(),
                    url: substitute(&req.url, &vars),
                    status: None,
                    duration_ms: None,
                    request: request_snapshot,
                    response: json!({ "error": err }),
                })
                .await;
            Err(ApiError(Error::Upstream(err)))
        }
    }
}

/// Resolve the variable map: explicit `environment_id`, else the workspace's
/// active environment, else empty.
async fn resolve_variables(
    repo: &ApiClientRepo,
    wid: &Id,
    environment_id: Option<&Id>,
) -> ApiResult<serde_json::Map<String, Value>> {
    let env = match environment_id {
        Some(eid) => {
            let env = repo.get_environment(eid).await?;
            ensure_in_workspace(&env.workspace_id, wid)?;
            Some(env)
        }
        None => repo.active_environment(wid).await?,
    };
    Ok(env
        .and_then(|e| e.variables.as_object().cloned())
        .unwrap_or_default())
}

/// Build the outbound request and send it, returning an `ApiResponse` or an
/// error message string (never panics).
/// One multipart/form-data field as encoded by the UI. A `file` field carries
/// the file's bytes base64-encoded in `value`.
#[derive(Deserialize)]
struct MultipartField {
    #[serde(default)]
    key: String,
    #[serde(default, rename = "type")]
    field_type: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    filename: String,
}

/// Build a reqwest multipart form from the request body. Accepts either a JSON
/// array of [`MultipartField`]s or the legacy `k=v&...` text encoding.
fn build_multipart(body: &str, vars: &serde_json::Map<String, Value>) -> reqwest::multipart::Form {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;

    let mut form = reqwest::multipart::Form::new();
    if body.trim_start().starts_with('[') {
        if let Ok(fields) = serde_json::from_str::<Vec<MultipartField>>(body) {
            for f in fields {
                let key = substitute(&f.key, vars);
                if key.trim().is_empty() {
                    continue;
                }
                if f.field_type == "file" {
                    let bytes = B64.decode(f.value.trim()).unwrap_or_default();
                    let filename = if f.filename.is_empty() {
                        "file".to_string()
                    } else {
                        f.filename.clone()
                    };
                    let mut part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
                    if let Some(mime) = guess_mime(&f.filename) {
                        // guess_mime only returns valid static MIME strings.
                        part = part.mime_str(mime).expect("valid static mime");
                    }
                    form = form.part(key, part);
                } else {
                    form = form.text(key, substitute(&f.value, vars));
                }
            }
            return form;
        }
    }
    // Legacy `k=v&...` (percent-encoded) text fields.
    for pair in body.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        };
        let key = substitute(&k, vars);
        if key.trim().is_empty() {
            continue;
        }
        form = form.text(key, substitute(&v, vars));
    }
    form
}

/// Minimal filename-extension → MIME guess for common upload types.
fn guess_mime(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "csv" => "text/csv",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "html" | "htm" => "text/html",
        _ => return None,
    })
}

async fn build_and_send(
    req: &ExecuteApiReq,
    vars: &serde_json::Map<String, Value>,
    proxy: Option<&str>,
) -> Result<ApiResponse, String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

    // --- method ---
    let url = substitute(&req.url, vars);
    // SSRF guard: resolve + classify the target host before connecting.
    net_guard::check_url(&url).await?;
    let method = reqwest::Method::from_bytes(req.method.to_uppercase().as_bytes())
        .map_err(|_| format!("invalid HTTP method '{}'", req.method))?;

    // --- query params (enabled only) ---
    let mut query: Vec<(String, String)> = Vec::new();
    for (k, v) in enabled_kv(&req.query) {
        query.push((substitute(&k, vars), substitute(&v, vars)));
    }

    // --- headers (enabled only) ---
    let mut headers = HeaderMap::new();
    for (k, v) in enabled_kv(&req.headers) {
        let name = substitute(&k, vars);
        let value = substitute(&v, vars);
        if name.trim().is_empty() {
            continue;
        }
        let hn = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("invalid header name '{name}'"))?;
        let hv = HeaderValue::from_str(&value)
            .map_err(|_| format!("invalid header value for '{name}'"))?;
        headers.insert(hn, hv);
    }

    // --- auth ---
    apply_auth(&req.auth, vars, &mut headers, &mut query)?;

    // Per-request settings: a custom client when any non-default is set or the
    // request is tunnelled, otherwise the shared pooled client.
    let custom_client = build_settings_client(req, proxy);
    let client = custom_client.as_ref().unwrap_or_else(|| http_client());
    let timeout = req
        .timeout_ms
        .map(Duration::from_millis)
        .unwrap_or(EXECUTE_TIMEOUT);
    let mut builder = client.request(method, &url).timeout(timeout);

    // --- body per body_mode ---
    match req.body_mode.as_str() {
        "none" | "" => {}
        "json" => {
            let body = substitute(&req.body, vars);
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            }
            builder = builder.body(body);
        }
        "graphql" => {
            let body = substitute(&req.body, vars);
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            }
            builder = builder.body(body);
        }
        "form" => {
            let body = substitute(&req.body, vars);
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/x-www-form-urlencoded"),
                );
            }
            builder = builder.body(body);
        }
        // multipart/form-data: the body is either a JSON array of fields
        // (`[{key,type,value,filename}]`, where a `file` field's value is the
        // base64 of the file's bytes) or the legacy `k=v&...` text encoding.
        // reqwest sets the `Content-Type: multipart/form-data; boundary=…`
        // itself, so any explicit Content-Type must be dropped.
        "multipart" => {
            let body = substitute(&req.body, vars);
            headers.remove(CONTENT_TYPE);
            builder = builder.multipart(build_multipart(&body, vars));
        }
        // raw or any other mode → send as text verbatim.
        _ => {
            let body = substitute(&req.body, vars);
            builder = builder.body(body);
        }
    }

    // Silence unused import when AUTHORIZATION is only referenced inside apply_auth.
    let _ = AUTHORIZATION;

    if !query.is_empty() {
        builder = builder.query(&query);
    }
    let header_count = headers.len();
    builder = builder.headers(headers);

    // Trace: resolved request + per-phase timing for the response "Trace" tab.
    let method_str = req.method.to_uppercase();
    let body_desc = match req.body_mode.as_str() {
        "none" | "" => String::new(),
        m => format!(", {m} body"),
    };
    let mut trace: Vec<otto_core::api::TraceStep> = vec![
        trace_step("Request", &format!("{method_str} {url}"), None, "info"),
        trace_step(
            "Sent request",
            &format!("{header_count} header(s){body_desc}"),
            None,
            "info",
        ),
    ];
    if proxy.is_some() {
        trace.push(trace_step(
            "SSH tunnel",
            "routed via SOCKS5 over SSH",
            None,
            "info",
        ));
    }

    let started = Instant::now();
    let resp = builder
        .send()
        .await
        .map_err(|e| describe_reqwest_error(&e))?;
    let ttfb_ms = started.elapsed().as_millis() as i64;
    let final_url = resp.url().to_string();
    trace.push(trace_step(
        "Waiting (TTFB)",
        "time to first response byte (includes connect + TLS)",
        Some(ttfb_ms),
        "timing",
    ));
    if final_url != url {
        trace.push(trace_step(
            "Redirected",
            &format!("→ {final_url}"),
            None,
            "redirect",
        ));
    }
    let status = resp.status();
    let status_code = status.as_u16();
    let status_text = status.canonical_reason().unwrap_or("").to_string();

    // Response headers as [{key,value}].
    let resp_headers: Vec<Value> = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            json!({
                "key": k.as_str(),
                "value": v.to_str().unwrap_or("").to_string(),
            })
        })
        .collect();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Body: keep the full bytes (base64) for preview/save and a UTF-8 (lossy)
    // text rendering capped for display. Beyond MAX_INLINE we keep neither.
    const MAX_INLINE: usize = 25 * 1024 * 1024; // 25 MB raw → base64 inlined
    const MAX_TEXT_DISPLAY: usize = 512 * 1024; // 512 KB rendered as text
    let dl_start = Instant::now();
    let bytes = resp.bytes().await.map_err(|e| describe_reqwest_error(&e))?;
    let download_ms = dl_start.elapsed().as_millis() as i64;
    let size_bytes = bytes.len() as i64;
    let duration_ms = started.elapsed().as_millis() as i64;
    trace.push(trace_step(
        "Downloaded",
        &human_bytes(bytes.len()),
        Some(download_ms),
        "timing",
    ));
    trace.push(trace_step(
        "Completed",
        &format!("{status_code} {status_text}"),
        Some(duration_ms),
        if status.is_success() {
            "success"
        } else {
            "error"
        },
    ));

    let (body, body_base64, truncated, too_large) = if bytes.len() > MAX_INLINE {
        (String::new(), String::new(), false, true)
    } else {
        use base64::engine::general_purpose::STANDARD as B64;
        use base64::Engine;
        let b64 = B64.encode(&bytes);
        let text = String::from_utf8_lossy(&bytes);
        if text.len() > MAX_TEXT_DISPLAY {
            let mut end = MAX_TEXT_DISPLAY;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            (text[..end].to_string(), b64, true, false)
        } else {
            (text.into_owned(), b64, false, false)
        }
    };

    Ok(ApiResponse {
        status: status_code,
        status_text,
        headers: Value::Array(resp_headers),
        body,
        body_base64,
        truncated,
        too_large,
        duration_ms,
        size_bytes,
        content_type,
        trace,
    })
}

fn trace_step(
    label: &str,
    detail: &str,
    ms: Option<i64>,
    level: &str,
) -> otto_core::api::TraceStep {
    otto_core::api::TraceStep {
        label: label.to_string(),
        detail: detail.to_string(),
        ms,
        level: level.to_string(),
    }
}

fn human_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

/// Apply the `auth` object to headers/query. Shapes:
/// `{type:bearer, token}` → Authorization: Bearer …
/// `{type:basic, username, password}` → Authorization: Basic base64
/// `{type:api_key, in:"header"|"query", key, value}`
fn apply_auth(
    auth: &Value,
    vars: &serde_json::Map<String, Value>,
    headers: &mut reqwest::header::HeaderMap,
    query: &mut Vec<(String, String)>,
) -> Result<(), String> {
    use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};

    let kind = auth.get("type").and_then(Value::as_str).unwrap_or("none");
    match kind {
        "bearer" => {
            let token = substitute(
                auth.get("token").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            if !token.is_empty() {
                let hv = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| "invalid bearer token".to_string())?;
                headers.insert(AUTHORIZATION, hv);
            }
        }
        "oauth2" => {
            // The token is fetched separately (POST .../oauth2/token) and stored
            // on the auth object; here we just attach it.
            let token = substitute(
                auth.get("access_token")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                vars,
            );
            let ttype = auth
                .get("token_type")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("Bearer");
            if !token.is_empty() {
                let hv = HeaderValue::from_str(&format!("{ttype} {token}"))
                    .map_err(|_| "invalid oauth2 token".to_string())?;
                headers.insert(AUTHORIZATION, hv);
            }
        }
        "basic" => {
            use base64::engine::general_purpose::STANDARD as B64;
            use base64::Engine;
            let user = substitute(
                auth.get("username").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            let pass = substitute(
                auth.get("password").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            let encoded = B64.encode(format!("{user}:{pass}"));
            let hv = HeaderValue::from_str(&format!("Basic {encoded}"))
                .map_err(|_| "invalid basic auth".to_string())?;
            headers.insert(AUTHORIZATION, hv);
        }
        "api_key" => {
            let key = substitute(auth.get("key").and_then(Value::as_str).unwrap_or(""), vars);
            let value = substitute(
                auth.get("value").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            if key.is_empty() {
                return Ok(());
            }
            let location = auth.get("in").and_then(Value::as_str).unwrap_or("header");
            if location == "query" {
                query.push((key, value));
            } else {
                let hn = HeaderName::from_bytes(key.as_bytes())
                    .map_err(|_| format!("invalid api_key header name '{key}'"))?;
                let hv = HeaderValue::from_str(&value)
                    .map_err(|_| "invalid api_key header value".to_string())?;
                headers.insert(hn, hv);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Human-friendly message for a reqwest error (timeout / DNS / connect / …).
fn describe_reqwest_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        format!("request timed out after {}s", EXECUTE_TIMEOUT.as_secs())
    } else if e.is_connect() {
        format!("connection failed: {e}")
    } else if e.is_request() {
        format!("could not build request: {e}")
    } else if e.is_body() || e.is_decode() {
        format!("failed reading response: {e}")
    } else {
        format!("request failed: {e}")
    }
}

// ===========================================================================
// Automations (collection runner)
// ===========================================================================

/// `GET /workspaces/{wid}/api-client/automations`
pub async fn list_automations(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiAutomation>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_automations(&wid).await?))
}

/// `POST /workspaces/{wid}/api-client/automations`
pub async fn create_automation(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiAutomationReq>,
) -> ApiResult<Json<ApiAutomation>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("automation name must not be empty".into()).into());
    }
    let auto = repo(&ctx)
        .create_automation(NewApiAutomation {
            workspace_id: wid,
            name: req.name.trim().to_string(),
            steps: normalize_json_array(req.steps),
        })
        .await?;
    Ok(Json(auto))
}

/// `PATCH /workspaces/{wid}/api-client/automations/{id}`
pub async fn update_automation(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiAutomationReq>,
) -> ApiResult<Json<ApiAutomation>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_automation(&id).await?.workspace_id, &wid)?;
    let steps = normalize_json_array(req.steps);
    let auto = repo
        .update_automation(&id, Some(req.name.trim()), Some(&steps))
        .await?;
    Ok(Json(auto))
}

/// `DELETE /workspaces/{wid}/api-client/automations/{id}`
pub async fn delete_automation(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_automation(&id).await?.workspace_id, &wid)?;
    repo.delete_automation(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /workspaces/{wid}/api-client/automations/{id}/run`
///
/// Runs each step in order against its saved request. The effective variable
/// map starts from the workspace's active environment and is overlaid with
/// variables extracted from prior steps (chaining). Every step is attempted —
/// a failing step is recorded but never aborts the run, and a failure to load a
/// request, send it, or evaluate assertions never panics.
pub async fn run_automation(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ApiRunResult>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);

    let automation = repo.get_automation(&id).await?;
    ensure_in_workspace(&automation.workspace_id, &wid)?;

    // Seed the chained variable map from the workspace's active environment.
    let mut vars = resolve_variables(&repo, &wid, None).await?;

    let steps = automation.steps.as_array().cloned().unwrap_or_default();
    let mut results: Vec<ApiRunStepResult> = Vec::with_capacity(steps.len());

    for step in &steps {
        results.push(run_step(&ctx, &repo, &wid, step, &mut vars).await);
    }

    let passed = results.iter().all(|s| s.ok);
    Ok(Json(ApiRunResult {
        automation_id: id,
        steps: results,
        passed,
    }))
}

/// Run one automation step against its saved request, evaluating assertions and
/// applying extractions into `vars` for later steps. Resilient: any error is
/// captured into the returned [`ApiRunStepResult`] rather than propagated.
async fn run_step(
    ctx: &ServerCtx,
    repo: &ApiClientRepo,
    wid: &Id,
    step: &Value,
    vars: &mut serde_json::Map<String, Value>,
) -> ApiRunStepResult {
    let request_id = step
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // Load the saved request (must belong to this workspace).
    let request = match load_step_request(repo, wid, &request_id).await {
        Ok(r) => r,
        Err(msg) => {
            return ApiRunStepResult {
                request_id,
                name: String::new(),
                status: None,
                duration_ms: 0,
                ok: false,
                assertions: Value::Array(Vec::new()),
                error: Some(msg),
            };
        }
    };

    // Honour the saved request's SSH tunnel choice, if any.
    let proxy = match resolve_socks_proxy(ctx, wid, request.ssh_connection_id.as_ref()).await {
        Ok(p) => p,
        Err(msg) => {
            return ApiRunStepResult {
                request_id,
                name: request.name,
                status: None,
                duration_ms: 0,
                ok: false,
                assertions: Value::Array(Vec::new()),
                error: Some(msg),
            };
        }
    };

    let exec = request_to_execute(&request);

    // Send via the shared single-request path (same reqwest logic as /execute).
    match build_and_send(&exec, vars, proxy.as_deref()).await {
        Ok(resp) => {
            // Parse the body as JSON once for json_path assertions/extraction;
            // a non-JSON body yields Null (assertions/extracts simply miss).
            let body_json = serde_json::from_str::<Value>(&resp.body).unwrap_or(Value::Null);

            // Evaluate assertions.
            let (assertions_value, all_passed) =
                eval_step_assertions(step, Some(resp.status), resp.duration_ms, &body_json);

            // Apply extractions into the chained variable map.
            apply_extractions(step, &body_json, vars);

            ApiRunStepResult {
                request_id,
                name: request.name,
                status: Some(resp.status),
                duration_ms: resp.duration_ms,
                ok: all_passed, // request succeeded; ok iff every assertion held
                assertions: assertions_value,
                error: None,
            }
        }
        Err(err) => {
            // Request failed: still evaluate assertions (against a null body and
            // unknown status) so the report is complete, but ok is false.
            let (assertions_value, _) = eval_step_assertions(step, None, 0, &Value::Null);
            ApiRunStepResult {
                request_id,
                name: request.name,
                status: None,
                duration_ms: 0,
                ok: false,
                assertions: assertions_value,
                error: Some(err),
            }
        }
    }
}

/// Load a step's saved request, validating presence and workspace ownership.
/// Returns a descriptive error string on any miss.
async fn load_step_request(
    repo: &ApiClientRepo,
    wid: &Id,
    request_id: &str,
) -> Result<ApiRequest, String> {
    if request_id.is_empty() {
        return Err("step is missing request_id".to_string());
    }
    let request = repo
        .get_request(&request_id.to_string())
        .await
        .map_err(|_| format!("request '{request_id}' not found"))?;
    if &request.workspace_id != wid {
        return Err(format!("request '{request_id}' not in this workspace"));
    }
    Ok(request)
}

/// Build an [`ExecuteApiReq`] from a saved request so the runner can reuse the
/// single-request execute path verbatim.
fn request_to_execute(request: &ApiRequest) -> ExecuteApiReq {
    ExecuteApiReq {
        method: request.method.clone(),
        url: request.url.clone(),
        headers: request.headers.clone(),
        query: request.query.clone(),
        body_mode: request.body_mode.clone(),
        body: request.body.clone(),
        auth: request.auth.clone(),
        // Variables are supplied by the runner's chained map, not an env id.
        environment_id: None,
        timeout_ms: None,
        follow_redirects: None,
        verify_ssl: None,
        vars: None,
        ssh_connection_id: request.ssh_connection_id.clone(),
    }
}

/// Evaluate a step's `assertions` array, returning the `[{desc,passed}]` JSON
/// and whether every assertion passed (vacuously true when none).
fn eval_step_assertions(
    step: &Value,
    status: Option<u16>,
    duration_ms: i64,
    body: &Value,
) -> (Value, bool) {
    let mut out: Vec<Value> = Vec::new();
    let mut all = true;
    if let Some(arr) = step.get("assertions").and_then(Value::as_array) {
        for assertion in arr {
            let r = eval_assertion(assertion, status, duration_ms, body);
            if !r.passed {
                all = false;
            }
            out.push(json!({ "desc": r.desc, "passed": r.passed }));
        }
    }
    (Value::Array(out), all)
}

/// Apply a step's `extract` list: evaluate each `path` against the JSON body and
/// store `var -> value` into the chained variable map. Missing paths are
/// skipped (the variable is simply not set).
fn apply_extractions(step: &Value, body: &Value, vars: &mut serde_json::Map<String, Value>) {
    if let Some(arr) = step.get("extract").and_then(Value::as_array) {
        for entry in arr {
            let var = entry.get("var").and_then(Value::as_str).unwrap_or("");
            let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
            if var.is_empty() {
                continue;
            }
            if let Some(found) = json_path(body, path) {
                vars.insert(var.to_string(), found.clone());
            }
        }
    }
}

// ===========================================================================
// shared helpers
// ===========================================================================

/// 404 when an entity belongs to a different workspace than the path's `wid`.
fn ensure_in_workspace(entity_ws: &Id, wid: &Id) -> Result<(), ApiError> {
    if entity_ws == wid {
        Ok(())
    } else {
        Err(ApiError(Error::NotFound("not in this workspace".into())))
    }
}

/// Replace `{{name}}` occurrences with the corresponding variable value
/// (string-coerced). Unknown placeholders are left intact.
fn substitute(input: &str, vars: &serde_json::Map<String, Value>) -> String {
    if !input.contains("{{") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = input[i + 2..].find("}}") {
                let name = input[i + 2..i + 2 + end].trim();
                if let Some(dynamic) = resolve_dynamic_var(name) {
                    out.push_str(&dynamic);
                } else if let Some(v) = vars.get(name) {
                    out.push_str(&value_to_string(v));
                } else {
                    // leave placeholder as-is
                    out.push_str(&input[i..i + 2 + end + 2]);
                }
                i = i + 2 + end + 2;
                continue;
            }
        }
        // push the current char (handle utf-8 boundaries via char iteration)
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Resolve Postman-style dynamic variables (`{{$guid}}`, `{{$timestamp}}`, …).
/// Returns `None` for anything that isn't a known dynamic name.
fn resolve_dynamic_var(name: &str) -> Option<String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    match name {
        "$guid" | "$randomUUID" => Some(uuid::Uuid::new_v4().to_string()),
        "$timestamp" => Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .to_string(),
        ),
        "$isoTimestamp" => Some(chrono::Utc::now().to_rfc3339()),
        "$randomInt" => Some((rand::random::<u32>() % 1000).to_string()),
        _ => None,
    }
}

/// Extract enabled `(key, value)` pairs from a `[{key,value,enabled}]` array.
/// A missing `enabled` field defaults to true.
fn enabled_kv(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(arr) = v.as_array() {
        for item in arr {
            let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if !enabled {
                continue;
            }
            let key = item.get("key").and_then(Value::as_str).unwrap_or("");
            if key.is_empty() {
                continue;
            }
            let value = item.get("value").and_then(Value::as_str).unwrap_or("");
            out.push((key.to_string(), value.to_string()));
        }
    }
    out
}

/// Default a JSON value to an empty array when it arrives as null (the DTOs use
/// `#[serde(default)]` so omitted fields become `Value::Null`).
fn normalize_json_array(v: Value) -> Value {
    if v.is_null() {
        Value::Array(Vec::new())
    } else {
        v
    }
}

/// Default a JSON value to an empty object when null.
fn normalize_json_object(v: Value) -> Value {
    if v.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn substitute_replaces_known_and_keeps_unknown() {
        let mut vars = serde_json::Map::new();
        vars.insert("base".into(), json!("https://api.test"));
        vars.insert("ver".into(), json!(2));
        assert_eq!(
            substitute("{{base}}/v{{ver}}/users", &vars),
            "https://api.test/v2/users"
        );
        // unknown placeholder preserved
        assert_eq!(substitute("{{missing}}/x", &vars), "{{missing}}/x");
        // no placeholders → unchanged
        assert_eq!(substitute("plain", &vars), "plain");
    }

    #[test]
    fn net_guard_blocks_internal_addresses() {
        use otto_netguard::is_blocked_ip;
        use std::net::IpAddr;
        let blocked = [
            "127.0.0.1",
            "::1",
            "10.0.0.5",
            "172.16.3.4",
            "192.168.1.1",
            "169.254.169.254", // cloud metadata
            "169.254.1.1",     // link-local
            "100.64.0.1",      // CGNAT
            "0.0.0.0",
            "::",
            "::ffff:127.0.0.1", // v4-mapped loopback
            "fd00::1",          // ULA
            "fe80::1",          // link-local v6
        ];
        for s in blocked {
            let ip: IpAddr = s.parse().unwrap();
            assert!(is_blocked_ip(ip), "{s} should be blocked");
        }
        let allowed = ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:2800:220:1::1"];
        for s in allowed {
            let ip: IpAddr = s.parse().unwrap();
            assert!(!is_blocked_ip(ip), "{s} should be allowed");
        }
    }

    #[tokio::test]
    async fn net_guard_check_url_rejects_loopback_and_schemes() {
        use super::net_guard::check_url;
        assert!(check_url("http://127.0.0.1/x").await.is_err());
        assert!(check_url("http://169.254.169.254/latest/meta-data")
            .await
            .is_err());
        assert!(check_url("http://[::1]:8080/").await.is_err());
        // Non-fetchable schemes are rejected outright.
        assert!(check_url("file:///etc/passwd").await.is_err());
        assert!(check_url("not a url").await.is_err());
    }

    #[test]
    fn enabled_kv_filters_disabled_and_empty() {
        let v = json!([
            {"key":"A","value":"1","enabled":true},
            {"key":"B","value":"2","enabled":false},
            {"key":"","value":"x","enabled":true},
            {"key":"C","value":"3"}
        ]);
        assert_eq!(
            enabled_kv(&v),
            vec![("A".into(), "1".into()), ("C".into(), "3".into())]
        );
    }
}
