//! Sessions REST router (contract endpoints #17–#22).
//!
//! The router is generic over a context the server implements; handlers read
//! the authenticated user from `Extension<AuthUser>` (inserted by the
//! server's auth middleware).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::{CreateSessionReq, Problem, UpdateSessionReq};
use otto_core::auth::{session_owner_or_admin, AuthUser, RoleChecker};
use otto_core::domain::{Session, User, WorkspaceRole, SCRATCH_WORKSPACE_ID};
use otto_core::workref::WorkRef;
use otto_core::{Error, Id};
use otto_state::WorkspacesRepo;

use crate::manager::SessionManager;

/// Owner-or-admin gate for a single session: `Ok` iff the canonical
/// [`session_owner_or_admin`] helper allows the caller (root, the session's
/// creator, or a workspace Admin of the session's workspace). Returns a
/// `Forbidden` [`ApiErr`] otherwise. This is the chokepoint that stops one user
/// reading or controlling another user's session (#L1–#L7) — the single source
/// of truth lives in `otto_core::auth`, shared with `otto-server`.
async fn ensure_session_owner_or_admin<S: SessionsCtx>(
    ctx: &S,
    user: &User,
    session: &Session,
) -> ApiResult<()> {
    ctx.check_resource(user, session).await?;
    if session_owner_or_admin(ctx.roles().as_ref(), user, session).await {
        Ok(())
    } else {
        Err(ApiErr(Error::Forbidden(
            "not the session owner or a workspace admin".into(),
        )))
    }
}

/// Server-side context required by the sessions routes.
pub trait SessionsCtx: Clone + Send + Sync + 'static {
    /// Resource-bound sessions reauthorize on reads, control, and reconnect.
    fn check_resource<'a>(
        &'a self,
        _user: &'a User,
        _session: &'a Session,
    ) -> otto_core::auth::BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }

    /// Keep only the `sessions` [`Self::check_resource`] admits, in order.
    /// The default checks row by row; `otto-server` overrides it to load the
    /// caller once per request and memoise each (resource, op) decision.
    fn check_resources<'a>(
        &'a self,
        user: &'a User,
        sessions: Vec<Session>,
    ) -> otto_core::auth::BoxFuture<'a, Vec<Session>> {
        Box::pin(async move {
            let mut out = Vec::with_capacity(sessions.len());
            for s in sessions {
                if self.check_resource(user, &s).await.is_ok() {
                    out.push(s);
                }
            }
            out
        })
    }

    /// True when this session's terminal is bound to an external resource
    /// (k8s / AWS / a connection). Those re-authorize on a tight cadence — a
    /// revoked grant must drop the socket promptly — while a plain agent/shell
    /// terminal re-checks far less often (the check is a SQLite round-trip, and
    /// it used to sit on the keystroke path). Cheap and synchronous: the
    /// binding is entirely in the session row. Default `false` keeps test
    /// contexts on the relaxed cadence; `otto-server` answers from
    /// `resource_sessions::binding`.
    fn resource_bound(&self, _session: &Session) -> bool {
        false
    }

    fn manager(&self) -> &Arc<SessionManager>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn workspaces(&self) -> &WorkspacesRepo;
}

/// Maps `otto_core::Error` to the problem-details response (local newtype
/// because the orphan rule forbids `impl IntoResponse for otto_core::Error`).
pub(crate) struct ApiErr(pub Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        ApiErr(e)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let problem = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(problem)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

/// REST routes; the server nests this under `/api/v1` and supplies the state.
pub fn api_router<S: SessionsCtx>() -> Router<S> {
    Router::new()
        .route(
            "/workspaces/{id}/sessions",
            get(list_sessions::<S>).post(create_session::<S>),
        )
        // Cross-workspace live list (tray / all-workspaces sidebar): one query
        // instead of one `/workspaces/{id}/sessions` per workspace.
        .route("/sessions", get(list_all_sessions::<S>))
        .route(
            "/sessions/{id}",
            get(get_session::<S>)
                .patch(patch_session::<S>)
                .delete(delete_session::<S>),
        )
        .route("/sessions/{id}/restart", post(restart_session::<S>))
        .route("/sessions/{id}/resume", post(resume_session::<S>))
        .route("/sessions/{id}/archive", post(archive_session::<S>))
        .route("/sessions/{id}/unarchive", post(unarchive_session::<S>))
        .route("/sessions/{id}/kill", post(kill_session::<S>))
        // Static segment beats the `{id}` capture in axum's route priority.
        .route("/sessions/bulk", post(bulk_sessions::<S>))
        // Distinct prefix so it can't collide with `/sessions/{id}`.
        .route("/app/kill-sessions", post(kill_all_sessions::<S>))
}

/// A [`Session`] as the list/get endpoints serve it: the domain row plus the
/// manager's TRANSIENT view of it — whether a PTY is live right now and how
/// many `/ws/term` viewers are attached. Serialize-only (never persisted); the
/// UI needs it to say "closing this will kill a running process" truthfully.
#[derive(serde::Serialize)]
struct SessionOut {
    #[serde(flatten)]
    session: Session,
    live: bool,
    viewers: u32,
}

fn with_live<S: SessionsCtx>(ctx: &S, session: Session) -> SessionOut {
    let live = ctx.manager().is_live(&session.id);
    let viewers = ctx.manager().attached_count(&session.id) as u32;
    SessionOut {
        session,
        live,
        viewers,
    }
}

/// Optional filters for `GET /workspaces/{id}/sessions` and `GET /sessions`.
/// All narrowing and all applied in SQL; absent params keep today's full
/// owner-scoped list (except `GET /sessions`, where `archived` defaults to
/// `false`).
#[derive(Default, serde::Deserialize)]
struct ListSessionsQuery {
    /// `true` → only archived rows; `false` → only active rows.
    archived: Option<bool>,
    /// "agent" | "connection".
    kind: Option<String>,
    /// Match `meta.source` exactly; the literal `"none"` matches sessions with
    /// NO source (the user-opened foreground ones).
    source: Option<String>,
    /// Match the session status string ("running", "idle", "exited", …).
    status: Option<String>,
    /// Keep only the newest `limit` matching rows (1–1000; still returned
    /// oldest-first). For paging the archived history.
    limit: Option<u32>,
    /// Paging cursor: only rows created strictly before this RFC 3339 instant
    /// (the `created_at` of the oldest row of the previous page).
    before: Option<String>,
    /// `true` → only sidebar-listable rows: connections + foreground agents
    /// (`Session::is_foreground_agent`); `false` → background agents only.
    foreground: Option<bool>,
    /// Comma list of background sources to keep anyway with
    /// `foreground=true` (e.g. `channel` for the Slack/Telegram groups).
    with_sources: Option<String>,
    /// Comma list of session ids (≤ 64) — fetch-by-id for open tabs.
    ids: Option<String>,
}

/// Hard ceiling on `?limit=`.
const MAX_LIST_LIMIT: u32 = 1000;
/// Hard ceiling on `?ids=` / `?with_sources=` entries.
const MAX_LIST_IDS: usize = 64;

/// Split a comma list, trimming and dropping blanks.
fn comma_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(String::from)
        .collect()
}

impl ListSessionsQuery {
    fn into_filter(self) -> Result<otto_state::SessionListFilter, Error> {
        if let Some(k) = self.kind.as_deref() {
            if !matches!(k, "agent" | "connection") {
                // Unknown kind: the old Rust filter matched nothing.
                return Ok(otto_state::SessionListFilter {
                    limit: Some(0),
                    ..Default::default()
                });
            }
        }
        let before = match self.before {
            Some(b) => Some(
                chrono::DateTime::parse_from_rfc3339(&b)
                    .map_err(|_| Error::Invalid("before must be an RFC 3339 timestamp".into()))?
                    .with_timezone(&chrono::Utc)
                    .to_rfc3339(),
            ),
            None => None,
        };
        let ids = match self.ids.as_deref() {
            Some(raw) => {
                let ids = comma_list(raw);
                if ids.len() > MAX_LIST_IDS {
                    return Err(Error::Invalid(format!(
                        "ids accepts at most {MAX_LIST_IDS} entries"
                    )));
                }
                Some(ids)
            }
            None => None,
        };
        let with_sources = self
            .with_sources
            .as_deref()
            .map(comma_list)
            .unwrap_or_default();
        if with_sources.len() > MAX_LIST_IDS {
            return Err(Error::Invalid("too many with_sources".into()));
        }
        Ok(otto_state::SessionListFilter {
            archived: self.archived,
            kind: self.kind,
            status: self.status,
            source: self.source,
            limit: self.limit.map(|l| l.clamp(1, MAX_LIST_LIMIT)),
            before,
            foreground: self.foreground,
            with_sources,
            ids,
        })
    }
}

/// POST /app/kill-sessions — terminate every live PTY. Called by the desktop
/// app on quit so no agent processes are left running.
///
/// Root-only (#L8): any authenticated non-root caller receives 403. The
/// endpoint is still "Exempt" in the policy table (no workspace context) but
/// the handler enforces the root requirement directly.
async fn kill_all_sessions<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<serde_json::Value>> {
    if !user.is_root {
        return Err(ApiErr(Error::Forbidden(
            "only root may kill all sessions".into(),
        )));
    }
    let n = ctx.manager().shutdown_all().await;
    Ok(Json(serde_json::json!({ "killed": n })))
}

/// #17 GET /workspaces/{id}/sessions — viewer, owner-scoped.
///
/// Membership (Viewer+) is still required to list a workspace at all. Within it,
/// a non-admin caller sees only **their own** sessions (#L1); root and workspace
/// **Admins** keep the full cross-user list (the sanctioned team/admin view).
async fn list_sessions<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Query(q): Query<ListSessionsQuery>,
) -> ApiResult<Json<Vec<SessionOut>>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Viewer)
        .await?;
    // Root or workspace-Admin → full list; otherwise scope to the caller's own.
    let admin = user.is_root
        || ctx
            .roles()
            .check(&user, &ws_id, WorkspaceRole::Admin)
            .await
            .is_ok();
    let scope = otto_state::SessionScope {
        workspace_id: ws_id,
        owner: (!admin).then(|| user.id.clone()),
    };
    let filter = q.into_filter()?;
    let sessions = ctx.manager().list_filtered(&[scope], &filter).await?;
    Ok(Json(visible_out(&ctx, &user, sessions).await))
}

/// GET /sessions — the caller's sessions across EVERY workspace they belong to
/// (plus the scratch workspace) in ONE query, with the same filters and the
/// same per-workspace owner scoping as #17 (root / workspace Admin → every
/// row; otherwise only the caller's own). `archived` defaults to `false`: this
/// is the tray / all-workspaces-sidebar feed, which used to fan #17 out once
/// per workspace (16 concurrent full-history reads every 20 s, starving the
/// 8-connection SQLite pool and the webview's 6 sockets).
async fn list_all_sessions<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(mut q): Query<ListSessionsQuery>,
) -> ApiResult<Json<Vec<SessionOut>>> {
    // A fetch-by-id (`ids=`) wants the rows whatever their archived state
    // (an open tab may show an archived transcript).
    if q.ids.is_none() {
        q.archived = Some(q.archived.unwrap_or(false));
    }
    let scopes: Vec<otto_state::SessionScope> = if user.is_root {
        ctx.workspaces()
            .list_all()
            .await?
            .into_iter()
            .map(|w| otto_state::SessionScope {
                workspace_id: w.id,
                owner: None,
            })
            .collect()
    } else {
        let mut scopes: Vec<otto_state::SessionScope> = ctx
            .workspaces()
            .list_for_user(&user.id)
            .await?
            .into_iter()
            .filter(|(w, _)| w.id != SCRATCH_WORKSPACE_ID)
            .map(|(w, role)| otto_state::SessionScope {
                workspace_id: w.id,
                owner: (role != WorkspaceRole::Admin).then(|| user.id.clone()),
            })
            .collect();
        // Everyone is an implicit Editor (never Admin) on scratch, so it is
        // owner-scoped — the same answer #17 gives for `/workspaces/scratch/sessions`.
        scopes.push(otto_state::SessionScope {
            workspace_id: SCRATCH_WORKSPACE_ID.to_string(),
            owner: Some(user.id.clone()),
        });
        scopes
    };
    let filter = q.into_filter()?;
    let sessions = ctx.manager().list_filtered(&scopes, &filter).await?;
    Ok(Json(visible_out(&ctx, &user, sessions).await))
}

/// Drop rows whose bound resource the caller may no longer reach, and attach
/// the live PTY state.
async fn visible_out<S: SessionsCtx>(
    ctx: &S,
    user: &User,
    sessions: Vec<Session>,
) -> Vec<SessionOut> {
    // One batched check: the caller is loaded once per request and each
    // (resource, op) is authorized once, not once per row (F9).
    ctx.check_resources(user, sessions)
        .await
        .into_iter()
        .map(|session| with_live(ctx, session))
        .collect()
}

/// #18 POST /workspaces/{id}/sessions — editor
async fn create_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(mut req): Json<CreateSessionReq>,
) -> ApiResult<Json<Session>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;

    // Stamp a minimal WorkRef into the session meta so every manually-created
    // session carries at least `origin = "manual"` for usage attribution (B1).
    // Runners that create sessions programmatically (review, product, swarm …)
    // overwrite `meta["work"]` with a richer ref; this is the plain-create path
    // fallback. We merge into the existing meta object so any caller-supplied
    // fields are preserved.
    {
        let meta = req
            .meta
            .get_or_insert_with(|| serde_json::Value::Object(Default::default()));
        if let serde_json::Value::Object(m) = meta {
            // Only stamp if the caller hasn't already supplied a work ref.
            if !m.contains_key("work") {
                let work_ref = WorkRef {
                    origin: Some("manual".to_string()),
                    ..Default::default()
                };
                if let Ok(v) = serde_json::to_value(&work_ref) {
                    m.insert("work".to_string(), v);
                }
            }
        }
    }

    let ws = ctx.workspaces().get(&ws_id).await?;
    let session = ctx.manager().create(&ws, &user.id, req, None).await?;
    Ok(Json(session))
}

/// #19 GET /sessions/{id} — owner-or-admin.
async fn get_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<SessionOut>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    Ok(Json(with_live(&ctx, session)))
}

/// `session.meta` keys a PATCH may never change (an unchanged round-trip is
/// accepted and dropped): the agent-UI-control grant and the device stamp.
pub const SERVER_OWNED_META: &[&str] = &["ui_control", "client_id", DELEGATED_BY_META];

/// Meta key naming the agent session that opened this one as a worker
/// (`POST /workspaces/{id}/sessions/open`). The server stamps it from the
/// caller's credential; it decides whether that agent's own token may
/// message this session, so a PATCH must never set or change it.
pub const DELEGATED_BY_META: &str = "delegated_by";

/// #20 PATCH /sessions/{id} — owner-or-admin
async fn patch_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(mut req): Json<UpdateSessionReq>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    if let Some(meta) = &req.meta {
        // These fields bind a terminal to its originating protected resource.
        // Ordinary metadata edits must not detach or replace that binding.
        if ["k8s", "aws", "connection_id", "source", "resource_node"]
            .iter()
            .any(|key| meta.get(*key).is_some() && meta.get(*key) != session.meta.get(*key))
        {
            return Err(ApiErr(Error::Forbidden(
                "resource session bindings cannot be edited".into(),
            )));
        }
        // Agent UI control: the grant is server-owned (only `POST
        // /sessions/{id}/ui-control`, human credentials, writes it) and the
        // device stamp decides WHICH window an agent may drive — the agent
        // holds this session's token and could otherwise grant itself
        // control or point itself at another device.
        if SERVER_OWNED_META
            .iter()
            .any(|key| meta.get(*key).is_some() && meta.get(*key) != session.meta.get(*key))
        {
            return Err(ApiErr(Error::Forbidden(
                "ui_control / client_id / delegated_by are server-owned (UI control: POST /sessions/{id}/ui-control)"
                    .into(),
            )));
        }
    }
    // The manager replaces nested objects via two writes. Do not pass even
    // unchanged binding objects through that path, which could detach them
    // transiently while a concurrent input request authorizes the session.
    if let Some(object) = req.meta.as_mut().and_then(serde_json::Value::as_object_mut) {
        for key in ["k8s", "aws", "connection_id", "source", "resource_node"]
            .iter()
            .chain(SERVER_OWNED_META)
        {
            object.remove(*key);
        }
    }
    let session = match req.title {
        Some(title) => ctx.manager().update_title(&id, &title).await?,
        None => session,
    };
    let session = match req.meta {
        Some(m) => ctx.manager().update_meta(&id, m).await?,
        None => session,
    };
    Ok(Json(session))
}

/// #21 DELETE /sessions/{id} — owner-or-admin; kills PTY + removes row
async fn delete_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    ctx.manager().remove(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /sessions/{id}/resume — open-if-live, with no destructive fallback.
async fn resume_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    if session.archived {
        return Err(Error::Conflict("session is archived — unarchive it first".into()).into());
    }
    ctx.manager().ensure_live(&id).await?;
    if !ctx.manager().is_live(&id) {
        return Err(Error::Conflict(
            "session cannot be resumed — no supported resumable process is available".into(),
        )
        .into());
    }
    Ok(Json(ctx.manager().get(&id).await?))
}

/// #22 POST /sessions/{id}/restart — owner-or-admin
async fn restart_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    Ok(Json(ctx.manager().restart(&id, None).await?))
}

/// POST /sessions/{id}/archive — owner-or-admin; kills PTY, keeps row + history
async fn archive_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    Ok(Json(ctx.manager().archive(&id).await?))
}

/// POST /sessions/{id}/unarchive — owner-or-admin
async fn unarchive_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    Ok(Json(ctx.manager().unarchive(&id).await?))
}

/// POST /sessions/{id}/kill — owner-or-admin; kills the PTY but KEEPS the row
/// un-archived ("stop the process, leave it in my list"). The session stays
/// resumable/reopenable exactly like an idle-suspended one. Previously the
/// only kill-but-keep path was the admin terminate route.
async fn kill_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    ctx.manager().kill_session(&id).await?;
    Ok(Json(ctx.manager().get(&id).await?))
}

/// Body for `POST /sessions/bulk`.
#[derive(serde::Deserialize)]
struct BulkSessionsReq {
    /// "archive" | "delete" | "kill".
    action: String,
    ids: Vec<Id>,
}

/// Per-id outcome of a bulk action.
#[derive(serde::Serialize)]
struct BulkSessionResult {
    id: Id,
    ok: bool,
    /// Set when `ok` is false: "forbidden", "not found", or the error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// POST /sessions/bulk — owner-scoped bulk archive/delete/kill. Non-owned ids
/// are SKIPPED (reported `ok:false, error:"forbidden"`) rather than failing
/// the whole batch, so "clean up my exited sessions" degrades gracefully for
/// non-admins. Capped to keep one request from tying up the manager.
async fn bulk_sessions<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<BulkSessionsReq>,
) -> ApiResult<Json<Vec<BulkSessionResult>>> {
    const BULK_CAP: usize = 200;
    if !matches!(req.action.as_str(), "archive" | "delete" | "kill") {
        return Err(ApiErr(Error::Invalid(
            "action must be one of: archive, delete, kill".into(),
        )));
    }
    if req.ids.len() > BULK_CAP {
        return Err(ApiErr(Error::Invalid(format!(
            "at most {BULK_CAP} ids per bulk request"
        ))));
    }
    // `len()` is already rejected above BULK_CAP; the `min` keeps the bound
    // visible at the allocation itself.
    let mut out = Vec::with_capacity(req.ids.len().min(BULK_CAP));
    for id in req.ids {
        let outcome = async {
            let session = ctx.manager().get(&id).await?;
            ctx.check_resource(&user, &session).await?;
            if !session_owner_or_admin(ctx.roles().as_ref(), &user, &session).await {
                return Err(Error::Forbidden("forbidden".into()));
            }
            match req.action.as_str() {
                "archive" => ctx.manager().archive(&id).await.map(|_| ()),
                "delete" => ctx.manager().remove(&id).await,
                _ => ctx.manager().kill_session(&id).await,
            }
        }
        .await;
        out.push(match outcome {
            Ok(()) => BulkSessionResult {
                id,
                ok: true,
                error: None,
            },
            Err(e) => BulkSessionResult {
                id,
                ok: false,
                error: Some(e.to_string()),
            },
        });
    }
    Ok(Json(out))
}

#[cfg(test)]
mod list_query_tests {
    use super::*;

    #[test]
    fn foreground_ids_and_sources_parse_into_the_filter() {
        let q = ListSessionsQuery {
            archived: Some(false),
            foreground: Some(true),
            with_sources: Some("channel, swarm,,".into()),
            ids: Some("a,b , c".into()),
            ..Default::default()
        };
        let f = q.into_filter().unwrap();
        assert_eq!(f.foreground, Some(true));
        assert_eq!(f.with_sources, vec!["channel", "swarm"]);
        assert_eq!(f.ids, Some(vec!["a".into(), "b".into(), "c".into()]));
        let empty = ListSessionsQuery {
            ids: Some(String::new()),
            ..Default::default()
        }
        .into_filter()
        .unwrap();
        assert_eq!(empty.ids, Some(vec![]), "ids= present but empty → no rows");
    }

    #[test]
    fn more_than_64_ids_is_rejected() {
        let ids = (0..65)
            .map(|i| format!("s{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let q = ListSessionsQuery {
            ids: Some(ids),
            ..Default::default()
        };
        assert!(matches!(q.into_filter(), Err(Error::Invalid(_))));
    }
}
