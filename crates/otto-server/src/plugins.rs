//! Runtime (out-of-process) custom plugins: supervisor + reverse-proxy + scoped
//! host-API + iframe asset server + management.
//!
//! A plugin is an external program (any language) under the plugins home with an
//! `otto-plugin.json` manifest. When **enabled**, Otto spawns it on a loopback
//! port and reverse-proxies `/api/v1/plugins/<slug>/*` to it (behind the auth +
//! RBAC feature-guard). The sidecar calls back into the token-authenticated host
//! API (`/api/v1/plugin-host/*`) for capabilities (repos, Jira creds, agents). Its
//! iframe UI is served from `/plugins/<slug>/ui/*`.
//!
//! Trust model: plugins are user-installed local processes with a scoped token to
//! the host API. Install is an admin action; plugins are disabled by default;
//! disable/remove kills the process. See docs/plugins/AUTHORING.md.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path as AxPath, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};

use otto_core::auth::AuthUser;
use otto_core::{new_id, Error};
use otto_state::{NewPlugin, PluginRecord, PluginsRepo};

use crate::auth::{require_root, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

// ===========================================================================
// Manifest
// ===========================================================================

/// `otto-plugin.json` on disk.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifestFile {
    pub slug: String,
    pub name: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// argv to run the sidecar (from the plugin dir); it must bind $OTTO_PLUGIN_PORT.
    pub exec: Vec<String>,
    /// iframe assets dir (relative to the plugin dir). None = no UI.
    #[serde(default)]
    pub ui: Option<String>,
    #[serde(default)]
    pub health: Option<String>,
}

fn default_icon() -> String {
    "box".into()
}

fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && s.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
}

fn load_manifest(dir: &Path) -> Result<PluginManifestFile, String> {
    let path = dir.join("otto-plugin.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let m: PluginManifestFile =
        serde_json::from_str(&raw).map_err(|e| format!("parse otto-plugin.json: {e}"))?;
    if !valid_slug(&m.slug) {
        return Err(format!("invalid slug '{}' (use ^[a-z][a-z0-9-]*$)", m.slug));
    }
    if m.exec.is_empty() {
        return Err("manifest `exec` must not be empty".into());
    }
    Ok(m)
}

// ===========================================================================
// Supervisor
// ===========================================================================

struct RunningPlugin {
    port: u16,
    child: Child,
}

/// Supervises enabled plugin sidecars (spawn/stop) and tracks their ports for the
/// reverse proxy.
pub struct PluginManager {
    repo: PluginsRepo,
    plugins_home: PathBuf,
    data_dir: PathBuf,
    /// Base URL the sidecar uses to call back (`…/api/v1/plugin-host`).
    host_api_base: String,
    running: Mutex<HashMap<String, RunningPlugin>>,
    http: reqwest::Client,
}

impl PluginManager {
    pub fn new(
        repo: PluginsRepo,
        plugins_home: PathBuf,
        data_dir: PathBuf,
        host_api_base: String,
    ) -> Self {
        Self {
            repo,
            plugins_home,
            data_dir,
            host_api_base,
            running: Mutex::new(HashMap::new()),
            http: reqwest::Client::new(),
        }
    }

    pub fn repo(&self) -> &PluginsRepo {
        &self.repo
    }

    pub fn plugins_home(&self) -> &Path {
        &self.plugins_home
    }

    /// Spawn every enabled plugin (best-effort; logs per-plugin failures).
    pub async fn start_enabled(&self) {
        let enabled = match self.repo.list_enabled().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("plugins: list_enabled failed: {e}");
                return;
            }
        };
        for p in enabled {
            if let Err(e) = self.spawn(&p).await {
                tracing::warn!(plugin = %p.slug, "plugins: spawn failed: {e}");
            }
        }
    }

    /// Spawn one plugin's sidecar on a fresh loopback port. Replaces any existing.
    pub async fn spawn(&self, p: &PluginRecord) -> Result<u16, String> {
        self.stop(&p.slug).await;
        let port = free_port().ok_or("no free loopback port")?;
        let data_dir = self.data_dir.join("plugins").join(&p.slug);
        let _ = std::fs::create_dir_all(&data_dir);

        let mut cmd = Command::new(&p.exec[0]);
        cmd.args(&p.exec[1..]);
        cmd.current_dir(&p.source);
        cmd.env("OTTO_PLUGIN_SLUG", &p.slug);
        cmd.env("OTTO_PLUGIN_PORT", port.to_string());
        cmd.env("OTTO_PLUGIN_TOKEN", &p.token);
        cmd.env("OTTO_HOST_API", &self.host_api_base);
        cmd.env("OTTO_PLUGIN_DATA_DIR", &data_dir);
        cmd.kill_on_drop(true);
        let child = cmd
            .spawn()
            .map_err(|e| format!("spawn `{}`: {e}", p.exec.join(" ")))?;

        self.running
            .lock()
            .unwrap()
            .insert(p.slug.clone(), RunningPlugin { port, child });

        // Best-effort readiness wait (don't fail the spawn if slow).
        let health = if p.health.is_empty() { "/health" } else { &p.health };
        let url = format!("http://127.0.0.1:{port}{health}");
        for _ in 0..20 {
            if self
                .http
                .get(&url)
                .timeout(Duration::from_millis(500))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        tracing::info!(plugin = %p.slug, port, "plugins: sidecar started");
        Ok(port)
    }

    /// Stop a running plugin (kills the child). No-op if not running.
    pub async fn stop(&self, slug: &str) {
        let running = self.running.lock().unwrap().remove(slug);
        if let Some(mut rp) = running {
            let _ = rp.child.start_kill();
            let _ = rp.child.wait().await;
            tracing::info!(plugin = %slug, "plugins: sidecar stopped");
        }
    }

    fn port_of(&self, slug: &str) -> Option<u16> {
        self.running.lock().unwrap().get(slug).map(|r| r.port)
    }
}

fn free_port() -> Option<u16> {
    let l = TcpListener::bind("127.0.0.1:0").ok()?;
    l.local_addr().ok().map(|a| a.port())
}

// ===========================================================================
// Routers
// ===========================================================================

/// API routes (mounted under `/api/v1`, behind auth + feature-guard):
/// public list, the slug-gated reverse proxy, and the admin management surface.
pub fn api_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/plugins", get(list_enabled))
        .route("/plugins/{slug}", any(proxy))
        .route("/plugins/{slug}/{*rest}", any(proxy))
        .route("/plugin-admin", get(admin_list))
        .route("/plugin-admin/install", post(install))
        .route("/plugin-admin/{slug}/enable", post(enable))
        .route("/plugin-admin/{slug}/disable", post(disable))
        .route("/plugin-admin/{slug}", delete(remove))
}

/// Scoped host API (merged into the PUBLIC routes — sidecar-token auth, NOT user
/// auth). Each handler validates the `OTTO_PLUGIN_TOKEN` bearer.
pub fn host_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/plugin-host/repos", get(host_repos))
        .route("/plugin-host/jira/accounts", get(host_jira_accounts))
        .route("/plugin-host/jira/credentials", get(host_jira_credentials))
        .route("/plugin-host/agents/run", post(host_agents_run))
}

/// Root-mounted iframe asset server: `/plugins/{slug}/ui/{*path}` (public static).
pub fn asset_router(ctx: ServerCtx) -> Router {
    Router::new()
        .route("/plugins/{slug}/ui", get(asset))
        .route("/plugins/{slug}/ui/{*path}", get(asset))
        .with_state(ctx)
}

// ===========================================================================
// Handlers — public list + reverse proxy
// ===========================================================================

#[derive(Serialize)]
struct PluginNav {
    slug: String,
    name: String,
    icon: String,
    has_ui: bool,
}

/// `GET /api/v1/plugins` — enabled plugins (slug/name/icon) for the UI sidebar.
/// Any authed user; the UI filters by `canPlugin`. (Exempt in policy.)
async fn list_enabled(State(ctx): State<ServerCtx>) -> ApiResult<Json<Vec<PluginNav>>> {
    let list = ctx.plugins.repo().list_enabled().await.map_err(ApiError)?;
    Ok(Json(
        list.into_iter()
            .map(|p| PluginNav {
                slug: p.slug,
                name: p.name,
                icon: p.icon,
                has_ui: p.ui_dir.is_some(),
            })
            .collect(),
    ))
}

/// Reverse-proxy `/api/v1/plugins/{slug}/*` to the sidecar. The auth +
/// feature-guard already authorized this (slug-keyed). Forwards method, query,
/// body, content-type, and the caller identity (X-Otto-User).
async fn proxy(State(ctx): State<ServerCtx>, req: Request) -> Response {
    let path = req.uri().path().to_string();
    let Some((slug, rest)) = split_plugin_path(&path) else {
        return (StatusCode::BAD_REQUEST, "bad plugin path").into_response();
    };
    let Some(port) = ctx.plugins.port_of(&slug) else {
        return (StatusCode::BAD_GATEWAY, "plugin not running").into_response();
    };
    let user = req.extensions().get::<AuthUser>().map(|u| u.0.clone());
    let method = req.method().clone();
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let ctype = req
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let body = match axum::body::to_bytes(req.into_body(), 16 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "body too large").into_response(),
    };

    let url = format!("http://127.0.0.1:{port}/{rest}{query}");
    let rmethod = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut rb = ctx.plugins.http.request(rmethod, &url).body(body.to_vec());
    if let Some(ct) = ctype {
        rb = rb.header("content-type", ct);
    }
    if let Some(u) = user {
        rb = rb.header("x-otto-user", u.id.as_str()).header("x-otto-user-name", u.display_name);
    }

    match rb.send().await {
        Ok(resp) => {
            let status = resp.status();
            let ct = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_string();
            let bytes = resp.bytes().await.unwrap_or_default();
            Response::builder()
                .status(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY))
                .header("content-type", ct)
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(e) => (StatusCode::BAD_GATEWAY, format!("plugin proxy: {e}")).into_response(),
    }
}

/// Split `…/plugins/<slug>/<rest>` (with or without the `/api/v1` nest prefix).
fn split_plugin_path(path: &str) -> Option<(String, String)> {
    let after = path.split("/plugins/").nth(1)?;
    let mut it = after.splitn(2, '/');
    let slug = it.next()?.to_string();
    let rest = it.next().unwrap_or("").to_string();
    if slug.is_empty() {
        return None;
    }
    Some((slug, rest))
}

// ===========================================================================
// Handlers — management (root only)
// ===========================================================================

/// `GET /api/v1/plugin-admin` — full installed-plugin list (admin).
async fn admin_list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<PluginRecord>>> {
    require_root(&user)?;
    Ok(Json(ctx.plugins.repo().list().await.map_err(ApiError)?))
}

#[derive(Deserialize)]
struct InstallReq {
    /// A local directory path or a git URL.
    source: String,
}

/// `POST /api/v1/plugin-admin/install` — install from a local path or git URL.
/// Copies/clones into the plugins home, reads the manifest, registers it (disabled).
async fn install(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<InstallReq>,
) -> ApiResult<Json<PluginRecord>> {
    require_root(&user)?;
    let home = ctx.plugins.plugins_home().to_path_buf();
    std::fs::create_dir_all(&home).ok();

    // Resolve the source into home/<slug>.
    let src = req.source.trim();
    let dir = if src.starts_with("http://") || src.starts_with("https://") || src.ends_with(".git")
    {
        let name = src
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("plugin")
            .trim_end_matches(".git")
            .to_string();
        let dest = home.join(&name);
        let out = Command::new("git")
            .args(["clone", "--depth", "1", src])
            .arg(&dest)
            .output()
            .await
            .map_err(|e| ApiError(Error::Internal(format!("git clone: {e}"))))?;
        if !out.status.success() {
            return Err(ApiError(Error::Invalid(format!(
                "git clone failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ))));
        }
        dest
    } else {
        let srcp = PathBuf::from(src);
        let m = load_manifest(&srcp).map_err(|e| ApiError(Error::Invalid(e)))?;
        let dest = home.join(&m.slug);
        if srcp.canonicalize().ok() != dest.canonicalize().ok() {
            copy_dir(&srcp, &dest).map_err(|e| ApiError(Error::Internal(e)))?;
        }
        dest
    };

    let m = load_manifest(&dir).map_err(|e| ApiError(Error::Invalid(e)))?;
    let rec = ctx
        .plugins
        .repo()
        .upsert(NewPlugin {
            slug: m.slug.clone(),
            name: m.name,
            icon: m.icon,
            version: m.version,
            description: m.description,
            source: dir.to_string_lossy().to_string(),
            exec: m.exec,
            ui_dir: m.ui,
            health: m.health.unwrap_or_else(|| "/health".into()),
            token: new_id(),
        })
        .await
        .map_err(ApiError)?;
    ctx.audit(otto_state::NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "plugin.installed".into(),
        target: Some(otto_core::Id::from(rec.slug.as_str())),
        detail: Some(serde_json::json!({ "source": src })),
        ip: None,
    })
    .await;
    Ok(Json(rec))
}

/// `POST /api/v1/plugin-admin/{slug}/enable` — enable + spawn.
async fn enable(
    State(ctx): State<ServerCtx>,
    AxPath(slug): AxPath<String>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<PluginRecord>> {
    require_root(&user)?;
    let rec = ctx
        .plugins
        .repo()
        .get(&slug)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("plugin {slug}"))))?;
    ctx.plugins.repo().set_enabled(&slug, true).await.map_err(ApiError)?;
    let enabled = PluginRecord { enabled: true, ..rec };
    if let Err(e) = ctx.plugins.spawn(&enabled).await {
        // Roll back the enabled flag if the process won't start.
        let _ = ctx.plugins.repo().set_enabled(&slug, false).await;
        return Err(ApiError(Error::Internal(format!("spawn: {e}"))));
    }
    Ok(Json(enabled))
}

/// `POST /api/v1/plugin-admin/{slug}/disable` — disable + stop.
async fn disable(
    State(ctx): State<ServerCtx>,
    AxPath(slug): AxPath<String>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_root(&user)?;
    ctx.plugins.repo().set_enabled(&slug, false).await.map_err(ApiError)?;
    ctx.plugins.stop(&slug).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/plugin-admin/{slug}` — stop + unregister (files are kept).
async fn remove(
    State(ctx): State<ServerCtx>,
    AxPath(slug): AxPath<String>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_root(&user)?;
    ctx.plugins.stop(&slug).await;
    ctx.plugins.repo().delete(&slug).await.map_err(ApiError)?;
    ctx.audit(otto_state::NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "plugin.removed".into(),
        target: Some(otto_core::Id::from(slug.as_str())),
        detail: None,
        ip: None,
    })
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ===========================================================================
// Handlers — iframe assets (public static)
// ===========================================================================

#[derive(Deserialize)]
struct AssetPath {
    slug: String,
    #[serde(default)]
    path: String,
}

async fn asset(State(ctx): State<ServerCtx>, AxPath(p): AxPath<AssetPath>) -> Response {
    let rec = match ctx.plugins.repo().get(&p.slug).await {
        Ok(Some(r)) if r.enabled => r,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let Some(ui_dir) = rec.ui_dir else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let base = PathBuf::from(&rec.source).join(&ui_dir);
    let rel = if p.path.is_empty() { "index.html" } else { &p.path };
    // Contain the path within the ui dir (no traversal).
    let target = base.join(rel);
    let (Ok(c_base), Ok(c_target)) = (base.canonicalize(), target.canonicalize()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !c_target.starts_with(&c_base) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match std::fs::read(&c_target) {
        Ok(bytes) => {
            let ct = mime_for(&c_target);
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", ct)
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn mime_for(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

// ===========================================================================
// Handlers — scoped host API (sidecar-token auth)
// ===========================================================================

/// Authenticate a sidecar by its `Authorization: Bearer <OTTO_PLUGIN_TOKEN>`.
async fn auth_plugin(ctx: &ServerCtx, headers: &HeaderMap) -> Result<PluginRecord, Response> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if token.is_empty() {
        return Err((StatusCode::UNAUTHORIZED, "missing plugin token").into_response());
    }
    match ctx.plugins.repo().find_enabled_by_token(token).await {
        Ok(Some(rec)) => Ok(rec),
        Ok(None) => Err((StatusCode::UNAUTHORIZED, "invalid plugin token").into_response()),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()),
    }
}

#[derive(Serialize)]
struct HostRepo {
    id: String,
    name: String,
    path: String,
    remote_url: Option<String>,
}

async fn host_repos(State(ctx): State<ServerCtx>, headers: HeaderMap) -> Response {
    if let Err(r) = auth_plugin(&ctx, &headers).await {
        return r;
    }
    match ctx.git_store.list_all_repos().await {
        Ok(repos) => Json(
            repos
                .into_iter()
                .map(|r| HostRepo {
                    id: r.id.to_string(),
                    name: r.name,
                    path: r.path,
                    remote_url: r.remote_url,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Serialize)]
struct HostJiraAccount {
    id: String,
    label: String,
    base_url: String,
    email: String,
}

async fn host_jira_accounts(State(ctx): State<ServerCtx>, headers: HeaderMap) -> Response {
    if let Err(r) = auth_plugin(&ctx, &headers).await {
        return r;
    }
    match ctx.issues_store.list_all_accounts().await {
        Ok(accts) => Json(
            accts
                .into_iter()
                .map(|a| HostJiraAccount {
                    id: a.id.to_string(),
                    label: a.label,
                    base_url: a.base_url,
                    email: a.email,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct AccountQuery {
    account: String,
}

#[derive(Serialize)]
struct HostJiraCreds {
    base_url: String,
    email: String,
    token: String,
}

/// Returns the Jira credentials (incl. token) for an account. The plugin is
/// trusted user-installed code; the token never leaves the loopback host API.
async fn host_jira_credentials(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Query(q): Query<AccountQuery>,
) -> Response {
    if let Err(r) = auth_plugin(&ctx, &headers).await {
        return r;
    }
    let account = match ctx.issues_store.get_account(&otto_core::Id::from(q.account.as_str())).await {
        Ok(a) => a,
        Err(e) => return (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    };
    let token = match ctx.secrets.get(&account.token_ref) {
        Ok(Some(t)) => t,
        Ok(None) => return (StatusCode::NOT_FOUND, "token missing").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    Json(HostJiraCreds {
        base_url: account.base_url,
        email: account.email,
        token,
    })
    .into_response()
}

#[derive(Deserialize)]
struct AgentRunReq {
    prompt: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Serialize)]
struct AgentRunResp {
    text: String,
}

async fn host_agents_run(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<AgentRunReq>,
) -> Response {
    if let Err(r) = auth_plugin(&ctx, &headers).await {
        return r;
    }
    let cwd = req.cwd.unwrap_or_else(|| ".".into());
    match ctx
        .orchestrator
        .run_agent(&req.prompt, &cwd, req.model.as_deref(), Duration::from_secs(180))
        .await
    {
        Ok(text) => Json(AgentRunResp { text }).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Recursively copy a directory (used to install a local plugin into the home,
/// and to stage review-lens skills into a shared `--add-dir` bundle — see
/// `modules::stage_review_skills`).
pub(crate) fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("mkdir {}: {e}", dest.display()))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("read {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        // Skip VCS + heavy build dirs.
        if matches!(name.to_str(), Some(".git") | Some("node_modules") | Some("target")) {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| format!("copy {}: {e}", from.display()))?;
        }
    }
    Ok(())
}
