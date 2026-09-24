//! Remote live browser (daemon-owned Chromium) — REST + the viewer WebSocket.
//! Contract: `docs/contracts/api.md` "Browser — remote live view" and
//! `docs/contracts/ws.md` §1b. The engine itself is `otto_browser::live`;
//! this module is the HTTP/WS surface, the auth around it, and the
//! [`ServerLiveHooks`] that give the runtime the audit log, the event bus and
//! the MCP approvals queue.
//!
//! Auth, three layers:
//! - feature axis (`Feature::Browser`, `policy.rs`): status/list/get = View,
//!   open/close/nav/control/screenshot = Edit, settings/install = Admin;
//! - workspace axis: every by-tab route loads the tab and checks the caller's
//!   role on ITS workspace (viewer to read, editor to act) — the IDOR guard;
//! - session axis: a live session is private to its owner — only the owner,
//!   a workspace Admin, or root may see / attach to / drive it (404 otherwise,
//!   so its existence doesn't leak).
//!
//! The WS route is root-mounted and self-authenticating (like `/ws/events`):
//! bearer via `Sec-WebSocket-Protocol: otto-bearer, <token>` or `?token=`,
//! share-scoped and MCP-only tokens refused, and the grant re-checked every
//! 5 s off the socket loop.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{broadcast, mpsc};

use otto_browser::live::protocol::MAX_CLIENT_FRAME_BYTES;
use otto_browser::live::session::validate_nav_url;
use otto_browser::live::{
    ClientMsg, ControlAction, ControllerKind, InstallJob, InstallState, LiveAudit, LiveError,
    LiveHooks, LiveRuntime, LiveSession, LiveSessionInfo, LiveSettingsPatch, NavAction,
    OpenParams, OutwardAction, ScreenshotRequest, ServerMsg, ViewerOut, Viewport,
    EPHEMERAL_PROFILE, SETTINGS_KEY,
};
use otto_core::api::Problem;
use otto_core::domain::{Capability, Feature, User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::{AuditRepo, BrowserTab, GrantsRepo, NewApproval, NewAuditEntry, SettingsRepo};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::ApiError;
use crate::state::ServerCtx;

/// `/api/v1` routes (bearer-authenticated, feature-guarded).
pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/browser/live/status", get(status))
        .route("/browser/live/settings", put(put_settings))
        .route("/browser/live/install", post(install))
        .route("/workspaces/{wid}/browser/live", get(list_live))
        .route(
            "/browser/tabs/{id}/live",
            get(get_live).post(open_live).delete(close_live),
        )
        .route("/browser/tabs/{id}/live/nav", post(nav_live))
        .route("/browser/tabs/{id}/live/control", post(control_live))
        .route("/browser/tabs/{id}/live/screenshot", post(screenshot_live))
}

/// Root-mounted viewer WebSocket (self-authenticating).
pub fn ws_router(ctx: ServerCtx) -> Router {
    Router::new()
        .route("/ws/browser/{tab_id}/live", get(live_ws))
        .with_state(ctx)
}

// ---------------------------------------------------------------------------
// Hooks: audit log, event bus, MCP approvals
// ---------------------------------------------------------------------------

/// What the live runtime needs from the daemon. Holds only the pieces it
/// uses (no `ServerCtx`, which owns the runtime — no reference cycle).
pub struct ServerLiveHooks {
    pub pool: sqlx::SqlitePool,
    pub events: broadcast::Sender<Event>,
    pub mcp: Arc<otto_mcp::McpService>,
}

#[async_trait::async_trait]
impl LiveHooks for ServerLiveHooks {
    async fn audit(&self, e: LiveAudit) {
        let action = e.action;
        if let Err(err) = AuditRepo::new(self.pool.clone())
            .insert(NewAuditEntry {
                user_id: e.user_id,
                action: e.action.to_string(),
                target: Some(e.target),
                detail: Some(e.detail),
                ip: None,
            })
            .await
        {
            tracing::warn!(%action, "audit insert failed (best-effort): {err}");
        }
    }

    fn session_changed(&self, info: &LiveSessionInfo) {
        let _ = self.events.send(Event::BrowserLiveSessionUpdated {
            workspace_id: info.workspace_id.clone(),
            tab_id: info.tab_id.clone(),
            owner_id: info.owner_id.clone(),
            state: info.state.as_str().to_string(),
        });
    }

    fn install_progress(&self, job: &InstallJob) {
        let _ = self.events.send(Event::BrowserEngineInstallUpdated {
            build: job.build.as_str().to_string(),
            version: job.version.clone(),
            state: job.state.as_str().to_string(),
            received_bytes: job.received_bytes,
            total_bytes: job.total_bytes,
            error: job.error.clone(),
        });
    }

    async fn request_approval(&self, a: &OutwardAction) -> Result<String, String> {
        let mut detail = format!(
            "An agent driving the remote browser wants to {} → {} (from {}{}). \
             Approving lets this one request go through.",
            a.method,
            a.target_host,
            a.page_origin,
            if a.page_title.is_empty() {
                String::new()
            } else {
                format!(", page \"{}\"", a.page_title.chars().take(120).collect::<String>())
            }
        );
        if let Some(p) = &a.screenshot_path {
            detail.push_str(&format!("\nScreenshot before the action: {p}"));
        }
        // Host/origin/method only — never a path, query or form body.
        let args = json!({
            "tab_id": a.tab_id,
            "method": a.method,
            "target_host": a.target_host,
            "page_origin": a.page_origin,
            "screenshot_path": a.screenshot_path,
        });
        let expires = chrono::Utc::now()
            + chrono::Duration::seconds(
                otto_browser::live::session::OUTWARD_APPROVAL_TIMEOUT.as_secs() as i64,
            );
        self.mcp
            .approvals()
            .create(NewApproval {
                workspace_id: Some(a.workspace_id.clone()),
                kind: "browser_action".into(),
                server_id: None,
                server_name: Some("otto".into()),
                tool: Some("otto.browser_live".into()),
                title: a.title(),
                detail: Some(detail),
                args_redacted_json: args.to_string(),
                args_hash: None,
                risk_label: Some("outward".into()),
                requested_by: Some(a.owner_id.clone()),
                requested_by_kind: Some("agent".into()),
                expires_at: Some(expires.to_rfc3339()),
            })
            .await
            .map(|appr| appr.id)
            .map_err(|e| e.to_string())
    }

    async fn await_approval(&self, approval_id: &str, timeout: Duration) -> Option<bool> {
        let deadline = tokio::time::Instant::now() + timeout;
        let id: Id = approval_id.to_string();
        loop {
            if let Ok(a) = self.mcp.approvals().get(&id).await {
                match a.status.as_str() {
                    "approved" => {
                        // Single use: this approval covered exactly one request.
                        let _ = self.mcp.approvals().consume(&id).await;
                        return Some(true);
                    }
                    "denied" | "expired" | "cancelled" | "consumed" => return Some(false),
                    _ => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

/// The daemon's live runtime (created on first use with the stored settings).
pub async fn runtime(ctx: &ServerCtx) -> Arc<LiveRuntime> {
    ctx.browser
        .live(|| async {
            let stored = SettingsRepo::new(ctx.pool.clone())
                .get(SETTINGS_KEY)
                .await
                .ok()
                .flatten();
            let hooks: Arc<dyn LiveHooks> = Arc::new(ServerLiveHooks {
                pool: ctx.pool.clone(),
                events: ctx.events.clone(),
                mcp: ctx.mcp.clone(),
            });
            (
                otto_browser::live::LiveSettings::from_stored(stored.as_ref()),
                hooks,
            )
        })
        .await
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

fn problem(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(Problem {
            code: code.to_string(),
            message: message.into(),
        }),
    )
        .into_response()
}

/// `LiveError` → HTTP (codes per the contract).
pub fn live_err(e: LiveError) -> Response {
    let msg = e.to_string();
    match e {
        LiveError::Unsupported => problem(StatusCode::BAD_REQUEST, "unsupported_platform", msg),
        LiveError::NotInstalled => problem(StatusCode::CONFLICT, "engine_not_installed", msg),
        LiveError::Invalid(_) => problem(StatusCode::BAD_REQUEST, "invalid", msg),
        LiveError::NotFound => problem(StatusCode::NOT_FOUND, "not_found", msg),
        LiveError::Conflict(_) => problem(StatusCode::CONFLICT, "conflict", msg),
        LiveError::TooMany(_) => problem(StatusCode::TOO_MANY_REQUESTS, "too_many_sessions", msg),
        LiveError::Engine(_) => problem(StatusCode::BAD_GATEWAY, "engine_unavailable", msg),
        LiveError::Blocked(_) => problem(StatusCode::BAD_REQUEST, "blocked", msg),
        LiveError::AgentPaused => problem(StatusCode::CONFLICT, "agent_paused", msg),
        LiveError::Timeout(_) => problem(StatusCode::BAD_GATEWAY, "timeout", msg),
    }
}

fn api(e: ApiError) -> Response {
    e.into_response()
}

fn not_found() -> Response {
    live_err(LiveError::NotFound)
}

// ---------------------------------------------------------------------------
// Access helpers
// ---------------------------------------------------------------------------

/// Owner, workspace Admin, or root.
pub async fn can_see(ctx: &ServerCtx, user: &User, owner_id: &str, workspace_id: &Id) -> bool {
    user.is_root
        || user.id == owner_id
        || ctx
            .roles
            .check(user, workspace_id, WorkspaceRole::Admin)
            .await
            .is_ok()
}

async fn load_tab(ctx: &ServerCtx, id: &Id) -> Result<BrowserTab, Response> {
    ctx.browser_tabs
        .get(id)
        .await
        .map_err(|e| api(ApiError(e)))?
        .ok_or_else(|| api(ApiError(Error::NotFound(format!("browser tab {id}")))))
}

/// The tab's session when it exists and `user` may see it.
async fn visible_session(
    ctx: &ServerCtx,
    user: &User,
    tab_id: &Id,
) -> Result<Arc<LiveSession>, Response> {
    let Some(rt) = ctx.browser.live_if_started() else {
        return Err(not_found());
    };
    let Some(s) = rt.session(tab_id) else {
        return Err(not_found());
    };
    if !can_see(ctx, user, &s.owner_id, &s.workspace_id).await {
        return Err(not_found());
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// Engine: status / settings / install
// ---------------------------------------------------------------------------

/// `GET /browser/live/status`
async fn status(State(ctx): State<ServerCtx>, CurrentUser(_user): CurrentUser) -> Response {
    let rt = runtime(&ctx).await;
    Json(rt.status().await).into_response()
}

/// `PUT /browser/live/settings` — partial `BrowserLiveSettings`.
async fn put_settings(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(patch): Json<LiveSettingsPatch>,
) -> Response {
    let rt = runtime(&ctx).await;
    let next = match rt.settings().patched(&patch) {
        Ok(s) => s,
        Err(m) => return problem(StatusCode::BAD_REQUEST, "invalid", m),
    };
    let value = serde_json::to_value(&next).unwrap_or_default();
    if let Err(e) = SettingsRepo::new(ctx.pool.clone())
        .put(SETTINGS_KEY, &value)
        .await
    {
        return api(ApiError(e));
    }
    rt.set_settings(next.clone());
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "browser.live.settings".into(),
        target: None,
        detail: Some(value),
        ip: None,
    })
    .await;
    Json(next).into_response()
}

#[derive(Deserialize, Default)]
struct InstallReq {
    #[serde(default)]
    build: Option<otto_browser::live::ChromeBuild>,
}

/// `POST /browser/live/install` — `{build?}`; the only way anything is
/// ever downloaded.
async fn install(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Bytes,
) -> Response {
    let req: InstallReq = if body.iter().all(u8::is_ascii_whitespace) {
        InstallReq::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => return problem(StatusCode::BAD_REQUEST, "invalid", format!("bad body: {e}")),
        }
    };
    let rt = runtime(&ctx).await;
    let build = req.build.unwrap_or(rt.settings().build);
    match rt.start_install(build) {
        Ok(job) => {
            let already = job.state == InstallState::Installed;
            if !already {
                ctx.audit(NewAuditEntry {
                    user_id: Some(user.id.clone()),
                    action: "browser.engine.install".into(),
                    target: Some(build.as_str().to_string()),
                    detail: Some(json!({"version": job.version})),
                    ip: None,
                })
                .await;
            }
            let code = if already {
                StatusCode::OK
            } else {
                StatusCode::ACCEPTED
            };
            (code, Json(job)).into_response()
        }
        Err(e) => live_err(e),
    }
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// `GET /workspaces/{wid}/browser/live`
async fn list_live(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> Response {
    if let Err(e) = require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await {
        return api(e);
    }
    let Some(rt) = ctx.browser.live_if_started() else {
        return Json(Vec::<LiveSessionInfo>::new()).into_response();
    };
    let mut out = Vec::new();
    for s in rt.sessions_in(&wid) {
        if can_see(&ctx, &user, &s.owner_id, &s.workspace_id).await {
            out.push(s.info());
        }
    }
    Json(out).into_response()
}

/// `GET /browser/tabs/{id}/live`
async fn get_live(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> Response {
    let tab = match load_tab(&ctx, &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(e) = require_ws_role(&ctx, &user, &tab.workspace_id, WorkspaceRole::Viewer).await {
        return api(e);
    }
    match visible_session(&ctx, &user, &id).await {
        Ok(s) => Json(s.info()).into_response(),
        Err(r) => r,
    }
}

#[derive(Deserialize, Default)]
struct CreateLiveReq {
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    viewport: Option<Viewport>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

/// `POST /browser/tabs/{id}/live`
async fn open_live(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Bytes,
) -> Response {
    let req: CreateLiveReq = if body.iter().all(u8::is_ascii_whitespace) {
        CreateLiveReq::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => return problem(StatusCode::BAD_REQUEST, "invalid", format!("bad body: {e}")),
        }
    };
    match req.engine.as_deref() {
        None | Some("remote") => {}
        Some("native") => {
            return problem(
                StatusCode::BAD_REQUEST,
                "invalid",
                "a native (desktop webview) tab needs no daemon session",
            )
        }
        Some(other) => {
            return problem(
                StatusCode::BAD_REQUEST,
                "invalid",
                format!("unknown engine {other:?} (expected \"remote\")"),
            )
        }
    }
    let tab = match load_tab(&ctx, &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(e) = require_ws_role(&ctx, &user, &tab.workspace_id, WorkspaceRole::Editor).await {
        return api(e);
    }
    // An explicit URL must pass the pre-check (400); the tab's stored URL is
    // only a best-effort start page for a NEW session (a blocked one just
    // opens blank) — re-attaching never navigates unless asked to.
    let explicit = match req.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => match validate_nav_url(u).await {
            Ok(u) => Some(u),
            Err(e) => return live_err(e),
        },
        None => None,
    };
    let rt = runtime(&ctx).await;
    let reattach = rt
        .session(&tab.id)
        .is_some_and(|s| s.is_live() && s.owner_id == user.id);
    let url = if explicit.is_some() || reattach {
        explicit
    } else {
        let u = tab.url.trim();
        if u.starts_with("http://") || u.starts_with("https://") {
            validate_nav_url(u).await.ok()
        } else {
            None
        }
    };
    let params = OpenParams {
        tab_id: tab.id.clone(),
        workspace_id: tab.workspace_id.clone(),
        owner_id: user.id.clone(),
        profile: req
            .profile
            .unwrap_or_else(|| EPHEMERAL_PROFILE.to_string()),
        viewport: req.viewport.unwrap_or_default(),
        url,
    };
    let session = match rt.open(params).await {
        Ok((s, _created)) => s,
        Err(e) => return live_err(e),
    };
    if tab.mode != "live" {
        if let Err(e) = ctx.browser_tabs.set_mode(&tab.id, "live").await {
            tracing::warn!("browser live: could not flip tab {} to live: {e}", tab.id);
        } else if let Ok(Some(updated)) = ctx.browser_tabs.get(&tab.id).await {
            super::browser::publish_tab_updated(&ctx, &updated);
        }
    }
    Json(session.info()).into_response()
}

/// `DELETE /browser/tabs/{id}/live`
async fn close_live(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> Response {
    let tab = match load_tab(&ctx, &id).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Err(e) = require_ws_role(&ctx, &user, &tab.workspace_id, WorkspaceRole::Editor).await {
        return api(e);
    }
    let Some(rt) = ctx.browser.live_if_started() else {
        return StatusCode::NO_CONTENT.into_response();
    };
    if let Some(s) = rt.session(&id) {
        if !can_see(&ctx, &user, &s.owner_id, &s.workspace_id).await {
            return not_found();
        }
        rt.close(&id, "closed").await;
    }
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
struct NavReq {
    action: NavAction,
    #[serde(default)]
    url: Option<String>,
}

async fn editable_session(
    ctx: &ServerCtx,
    user: &User,
    id: &Id,
) -> Result<Arc<LiveSession>, Response> {
    let tab = load_tab(ctx, id).await?;
    require_ws_role(ctx, user, &tab.workspace_id, WorkspaceRole::Editor)
        .await
        .map_err(api)?;
    let s = visible_session(ctx, user, id).await?;
    if !s.is_live() {
        return Err(not_found());
    }
    Ok(s)
}

/// `POST /browser/tabs/{id}/live/nav`
async fn nav_live(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<NavReq>,
) -> Response {
    let s = match editable_session(&ctx, &user, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    // A person's REST navigation follows the same lock as their WS input.
    if s.controller() == ControllerKind::Agent {
        return problem(
            StatusCode::CONFLICT,
            "not_driver",
            "the agent is driving this tab — take over first",
        );
    }
    if let Err(e) = s.navigate(req.action, req.url).await {
        return live_err(e);
    }
    Json(s.info()).into_response()
}

#[derive(Deserialize)]
struct ControlReq {
    action: ControlAction,
}

/// `POST /browser/tabs/{id}/live/control`
async fn control_live(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ControlReq>,
) -> Response {
    let s = match editable_session(&ctx, &user, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    s.control(&user.id, req.action);
    Json(s.info()).into_response()
}

/// `POST /browser/tabs/{id}/live/screenshot` — the image bytes.
async fn screenshot_live(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Bytes,
) -> Response {
    let req: ScreenshotRequest = if body.iter().all(u8::is_ascii_whitespace) {
        ScreenshotRequest::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => return problem(StatusCode::BAD_REQUEST, "invalid", format!("bad body: {e}")),
        }
    };
    let s = match editable_session(&ctx, &user, &id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match s.screenshot(&req).await {
        Ok(shot) => {
            let mut resp = (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, shot.mime)],
                shot.bytes,
            )
                .into_response();
            if let Ok(v) = HeaderValue::from_str(&shot.url) {
                resp.headers_mut().insert("x-otto-page-url", v);
            }
            resp.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            );
            resp
        }
        Err(e) => live_err(e),
    }
}

// ---------------------------------------------------------------------------
// Viewer WebSocket
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// What a viewer may do, decided at the upgrade and re-checked every 5 s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerGrant {
    /// Not allowed at all (→ refuse / close).
    Denied,
    Watch,
    Drive,
}

/// Pure grant rule: visibility (owner/admin/root) + Browser ≥ View to watch;
/// ws editor + Browser ≥ Edit to drive. Share-scoped / MCP-only tokens never.
pub fn viewer_grant(
    token_restricted: bool,
    visible: bool,
    capability: Capability,
    ws_editor: bool,
) -> ViewerGrant {
    if token_restricted || !visible || capability < Capability::View {
        return ViewerGrant::Denied;
    }
    if capability >= Capability::Edit && ws_editor {
        ViewerGrant::Drive
    } else {
        ViewerGrant::Watch
    }
}

async fn grant_for(ctx: &ServerCtx, token: &str, session: &LiveSession) -> (ViewerGrant, Option<User>) {
    let Ok(auth) = ctx.authenticator.authenticate(token).await else {
        return (ViewerGrant::Denied, None);
    };
    let restricted = auth.is_scoped() || auth.mcp_only;
    let user = auth.effective_user;
    let visible = can_see(ctx, &user, &session.owner_id, &session.workspace_id).await;
    let capability = GrantsRepo::new(ctx.pool.clone())
        .capability_of(&user, Feature::Browser)
        .await
        .unwrap_or(Capability::None);
    let ws_editor = ctx
        .roles
        .check(&user, &session.workspace_id, WorkspaceRole::Editor)
        .await
        .is_ok();
    (
        viewer_grant(restricted, visible, capability, ws_editor),
        Some(user),
    )
}

/// `GET /ws/browser/{tab_id}/live`
async fn live_ws(
    ws: WebSocketUpgrade,
    Path(tab_id): Path<String>,
    Query(q): Query<TokenQuery>,
    headers: HeaderMap,
    State(ctx): State<ServerCtx>,
) -> Response {
    let sub = crate::ws_events::token_from_subprotocol(&headers);
    let used_subprotocol = sub.is_some();
    let Some(token) = sub.or(q.token) else {
        return ApiError(Error::Unauthorized).into_response();
    };
    let Ok(auth) = ctx.authenticator.authenticate(&token).await else {
        return ApiError(Error::Unauthorized).into_response();
    };
    if auth.is_scoped() || auth.mcp_only {
        return ApiError(Error::Forbidden(
            "share-scoped and MCP-only tokens cannot open the live browser".into(),
        ))
        .into_response();
    }
    let Some(rt) = ctx.browser.live_if_started() else {
        return not_found();
    };
    let Some(session) = rt.session(&tab_id).filter(|s| s.is_live()) else {
        return not_found();
    };
    let (grant, user) = grant_for(&ctx, &token, &session).await;
    let Some(user) = user else {
        return ApiError(Error::Unauthorized).into_response();
    };
    if grant == ViewerGrant::Denied {
        return not_found();
    }
    let upgrade = ws
        .max_message_size(MAX_CLIENT_FRAME_BYTES)
        .max_frame_size(MAX_CLIENT_FRAME_BYTES);
    let go = move |socket: WebSocket| serve_viewer(socket, ctx, token, user, session, grant);
    if used_subprotocol {
        upgrade
            .protocols([crate::ws_events::BEARER_SUBPROTOCOL])
            .on_upgrade(go)
    } else {
        upgrade.on_upgrade(go)
    }
}

enum Recheck {
    Revoked,
    Grant(ViewerGrant),
}

async fn serve_viewer(
    socket: WebSocket,
    ctx: ServerCtx,
    token: String,
    user: User,
    session: Arc<LiveSession>,
    grant: ViewerGrant,
) {
    let (mut sink, mut stream) = socket.split();
    let mut handle = match session.attach(&user.id, grant == ViewerGrant::Drive) {
        Ok(h) => h,
        Err(e) => {
            let _ = sink
                .send(Message::Text(
                    ServerMsg::error("engine_unavailable", e.to_string())
                        .to_json()
                        .into(),
                ))
                .await;
            let _ = sink.send(Message::Close(None)).await;
            return;
        }
    };

    // Input runs in order on its own task: a slow CDP input call (a page
    // stuck in a dialog) never stalls the outbound frame stream.
    let (in_tx, mut in_rx) = mpsc::channel::<ClientMsg>(256);
    let input_session = session.clone();
    let viewer_id = handle.id;
    let input_task = tokio::spawn(async move {
        while let Some(m) = in_rx.recv().await {
            input_session.handle_client(viewer_id, m).await;
        }
    });

    // Grant re-check every 5 s, off the socket loop.
    let (re_tx, mut re_rx) = mpsc::channel::<Recheck>(4);
    let re_ctx = ctx.clone();
    let re_session = session.clone();
    let re_token = token.clone();
    let recheck_task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.tick().await;
        loop {
            tick.tick().await;
            let (g, _) = grant_for(&re_ctx, &re_token, &re_session).await;
            let msg = if g == ViewerGrant::Denied {
                Recheck::Revoked
            } else {
                Recheck::Grant(g)
            };
            let stop = matches!(msg, Recheck::Revoked);
            if re_tx.send(msg).await.is_err() || stop {
                return;
            }
        }
    });

    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.tick().await;
    let mut current = grant;
    loop {
        tokio::select! {
            out = handle.recv() => match out {
                Some(ViewerOut::Text(t)) => {
                    if sink.send(Message::Text(t.into())).await.is_err() {
                        break;
                    }
                }
                Some(ViewerOut::Frame(f)) => {
                    if sink.send(Message::Binary(Bytes::from(f.as_ref().clone()))).await.is_err() {
                        break;
                    }
                }
                // The session closed (its `closed` frame was already queued)
                // or dropped this viewer as wedged.
                None => break,
            },
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(t))) => match ClientMsg::parse(t.as_str()) {
                    Ok(ClientMsg::Ack { seq }) => handle.handle(ClientMsg::Ack { seq }).await,
                    Ok(m) => {
                        // Input flood beyond the queue is dropped, not buffered.
                        let _ = in_tx.try_send(m);
                    }
                    Err(e) => {
                        let _ = sink
                            .send(Message::Text(ServerMsg::error("bad_frame", e).to_json().into()))
                            .await;
                    }
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            re = re_rx.recv() => match re {
                Some(Recheck::Revoked) | None => {
                    let _ = sink
                        .send(Message::Text(ServerMsg::Closed { reason: "revoked" }.to_json().into()))
                        .await;
                    break;
                }
                Some(Recheck::Grant(g)) => {
                    // A live socket's right only ever narrows (a regained
                    // Edit needs a reconnect), like the terminal stream.
                    if g == ViewerGrant::Watch && current == ViewerGrant::Drive {
                        current = g;
                        handle.set_can_drive(false);
                    }
                }
            },
            _ = ping.tick() => {
                if sink.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
        }
    }
    input_task.abort();
    recheck_task.abort();
    let _ = sink.send(Message::Close(None)).await;
    drop(handle); // detaches the viewer (releases a human's control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_grants() {
        use Capability::*;
        // Restricted tokens never, whatever else holds.
        assert_eq!(viewer_grant(true, true, Admin, true), ViewerGrant::Denied);
        // Not the owner / ws admin / root → invisible.
        assert_eq!(viewer_grant(false, false, Admin, true), ViewerGrant::Denied);
        // No Browser grant at all.
        assert_eq!(viewer_grant(false, true, None, true), ViewerGrant::Denied);
        // View grant, or not a ws editor → watch only.
        assert_eq!(viewer_grant(false, true, View, true), ViewerGrant::Watch);
        assert_eq!(viewer_grant(false, true, Edit, false), ViewerGrant::Watch);
        // Edit + editor → drive.
        assert_eq!(viewer_grant(false, true, Edit, true), ViewerGrant::Drive);
        assert_eq!(viewer_grant(false, true, Admin, true), ViewerGrant::Drive);
    }

    #[test]
    fn live_errors_map_to_contract_codes() {
        let s = |e| live_err(e).status();
        assert_eq!(s(LiveError::NotInstalled), StatusCode::CONFLICT);
        assert_eq!(s(LiveError::TooMany(6)), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(s(LiveError::NotFound), StatusCode::NOT_FOUND);
        assert_eq!(s(LiveError::Blocked("h".into())), StatusCode::BAD_REQUEST);
        assert_eq!(s(LiveError::Engine("x".into())), StatusCode::BAD_GATEWAY);
        assert_eq!(s(LiveError::Unsupported), StatusCode::BAD_REQUEST);
        assert_eq!(s(LiveError::AgentPaused), StatusCode::CONFLICT);
    }
}
