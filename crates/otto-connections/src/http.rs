//! Connections REST router (contract endpoints #25–#30).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::api::{
    MoveSectionReq, Problem, ReorderSectionsReq, SectionScopeQuery, SftpDownloadReq,
    SftpDownloadResp, SftpListResp, SftpMkdirReq, SftpReadResp, SftpRemoveReq, SftpRenameReq,
    SftpUploadReq, TestConnectionResp, UpsertConnectionReq, UpsertSectionReq,
};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{Connection, ConnectionKind, ConnectionSection, Session, User, WorkspaceRole};
use otto_core::{Error, Id};
use otto_ssh::{SftpParams, SftpSession};
use otto_state::SqlitePool;
use serde::Deserialize;

use crate::conn_import::{
    self, ImportCreateReq, ImportCreateResult, ImportFailure, ImportScanResult, ImportSource,
    SourceStatus,
};
use crate::service::{key_perms_warning_for, ConnectionsService, DbTester, Spawner};

/// Server-side context required by the connections routes.
pub trait ConnectionsCtx: Clone + Send + Sync + 'static {
    fn connections(&self) -> &Arc<ConnectionsService>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn spawner(&self) -> &Arc<dyn Spawner>;
    /// SQLite pool used to read daemon settings (e.g. `connections.owner_private`).
    fn pool(&self) -> SqlitePool;
    /// Optional hook to route DB-kind test probes through the DB Explorer's
    /// warm-tunnel pool (reuses a cached `ssh -L` forward). Returns `None` in
    /// unit-test contexts or any setup that doesn't have a `DbViewerService`
    /// wired in; the fallback is the CLI subprocess path.
    fn db_tester(&self) -> Option<Arc<dyn DbTester>> {
        None
    }
}

/// Returns `true` when the `connections.owner_private` setting is explicitly
/// set to `true`. Absent or non-boolean ⇒ `false` (default-off).
pub async fn owner_private_enabled<S: ConnectionsCtx>(ctx: &S) -> bool {
    otto_state::SettingsRepo::new(ctx.pool())
        .get("connections.owner_private")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// When `connections.owner_private` is ON, require the caller to be root or
/// the connection's creator, else 403.
pub fn require_conn_owner_or_root(user: &User, conn: &Connection) -> Result<(), Error> {
    if user.is_root || conn.created_by == user.id {
        Ok(())
    } else {
        Err(Error::Forbidden(
            "connection belongs to another user".into(),
        ))
    }
}

/// Local problem-details mapper (orphan rule: cannot impl IntoResponse for
/// `otto_core::Error` here).
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

/// Body of `POST /connections/{id}/open`. `workspace_id` is only needed for
/// global connections (which have no workspace of their own).
#[derive(Debug, Default, Deserialize)]
pub struct OpenConnectionReq {
    pub title: Option<String>,
    pub workspace_id: Option<Id>,
}

/// Body of `PATCH /connections/{id}/pin`.
#[derive(Debug, Deserialize)]
pub struct PinConnectionReq {
    pub pinned: bool,
}

/// REST routes; the server nests this under `/api/v1` and supplies the state.
pub fn api_router<S: ConnectionsCtx>() -> Router<S> {
    Router::new()
        .route(
            "/workspaces/{id}/connections",
            get(list_connections::<S>).post(create_connection::<S>),
        )
        // --- Import connections from other DB tools ---
        .route(
            "/workspaces/{id}/connections/import/sources",
            get(import_sources::<S>),
        )
        .route(
            "/workspaces/{id}/connections/import/scan",
            post(import_scan::<S>),
        )
        .route(
            "/workspaces/{id}/connections/import/create",
            post(import_create::<S>),
        )
        .route(
            "/connections/{id}",
            axum::routing::patch(update_connection::<S>).delete(delete_connection::<S>),
        )
        .route("/connections/{id}/open", post(open_connection::<S>))
        .route("/connections/{id}/test", post(test_connection::<S>))
        .route("/connections/{id}/pin", patch(pin_connection::<S>))
        // --- SFTP file browser (SSH connections only) ---
        .route("/connections/{id}/sftp/list", get(sftp_list::<S>))
        .route("/connections/{id}/sftp/read", get(sftp_read::<S>))
        .route("/connections/{id}/sftp/download", post(sftp_download::<S>))
        .route("/connections/{id}/sftp/upload", post(sftp_upload::<S>))
        .route("/connections/{id}/sftp/mkdir", post(sftp_mkdir::<S>))
        .route("/connections/{id}/sftp/remove", post(sftp_remove::<S>))
        .route("/connections/{id}/sftp/rename", post(sftp_rename::<S>))
        .route(
            "/workspaces/{id}/connection-sections",
            get(list_sections::<S>).post(create_section::<S>),
        )
        .route(
            "/workspaces/{id}/connection-sections/reorder",
            post(reorder_sections::<S>),
        )
        .route(
            "/connection-sections/{id}",
            axum::routing::patch(rename_section::<S>).delete(delete_section::<S>),
        )
        .route("/connection-sections/{id}/move", post(move_section::<S>))
}

/// Editor in the connection's workspace; for global connections: root only.
async fn check_conn_role<S: ConnectionsCtx>(
    ctx: &S,
    user: &User,
    conn: &Connection,
    min: WorkspaceRole,
) -> Result<(), Error> {
    match &conn.workspace_id {
        Some(ws) => ctx.roles().check(user, ws, min).await,
        None => {
            if user.is_root {
                Ok(())
            } else {
                Err(Error::Forbidden(
                    "global connections are managed by root".into(),
                ))
            }
        }
    }
}

/// #25 GET /workspaces/{id}/connections — viewer
async fn list_connections<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Vec<Connection>>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Viewer)
        .await?;
    // When owner-private is ON, non-root users see only their own connections.
    if owner_private_enabled(&ctx).await && !user.is_root {
        Ok(Json(ctx.connections().list_for(&ws_id, &user.id).await?))
    } else {
        Ok(Json(ctx.connections().list(&ws_id).await?))
    }
}

/// #26 POST /workspaces/{id}/connections — editor
async fn create_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<UpsertConnectionReq>,
) -> ApiResult<Json<Connection>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    // Connections are a global library — created workspace-independent. The
    // path workspace is used only to authorize the caller.
    let conn = ctx.connections().create(None, &user.id, req).await?;
    Ok(Json(conn))
}

// --- Import connections from other DB tools ---------------------------------
//
// The daemon runs locally, so it reads each tool's config from its default
// macOS location — the user picks a *tool*, never a file. Editor-gated like the
// create endpoint (the path workspace authorizes; created connections are
// global). Imported connections always create with `secret: None` (every tool
// keeps passwords encrypted / in an OS keychain — unrecoverable here).

/// Body of `POST …/connections/import/scan`.
#[derive(Debug, Deserialize)]
struct ImportScanBody {
    source: ImportSource,
}

/// GET /workspaces/{id}/connections/import/sources — editor.
/// Detects all four tools at their default macOS paths (stat + parse-count).
async fn import_sources<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Vec<SourceStatus>>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    // Each `source_status` touches disk; offload off the async runtime worker.
    let statuses = tokio::task::spawn_blocking(|| {
        ImportSource::ALL
            .iter()
            .map(|s| conn_import::source_status(*s))
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| ApiErr(Error::Internal(format!("import scan task failed: {e}"))))?;
    Ok(Json(statuses))
}

/// POST /workspaces/{id}/connections/import/scan — editor.
/// Locates the chosen tool's default config, reads it, and parses it.
async fn import_scan<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(body): Json<ImportScanBody>,
) -> ApiResult<Json<ImportScanResult>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    let source = body.source;
    let result = tokio::task::spawn_blocking(move || conn_import::scan_source(source))
        .await
        .map_err(|e| ApiErr(Error::Internal(format!("import scan task failed: {e}"))))?;
    Ok(Json(result))
}

/// POST /workspaces/{id}/connections/import/create — editor.
/// Best-effort batch create: each item goes through the normal create path with
/// `secret: None`; one failure never aborts the batch.
async fn import_create<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<ImportCreateReq>,
) -> ApiResult<Json<ImportCreateResult>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    let mut created = Vec::new();
    let mut failed = Vec::new();
    for item in req.connections {
        let name = item.name.clone();
        let create_req = UpsertConnectionReq {
            name: item.name,
            kind: item.kind,
            params: item.params,
            // Tools keep passwords encrypted / in OS keychains — unrecoverable.
            secret: None,
            first_command: None,
            section_id: req.section_id.clone(),
            environment: item.environment,
            read_only: item.read_only,
        };
        // Connections are a global library — created workspace-independent
        // (mirrors `create_connection`; the path workspace only authorizes).
        match ctx.connections().create(None, &user.id, create_req).await {
            Ok(conn) => created.push(conn),
            Err(e) => failed.push(ImportFailure {
                name,
                error: e.to_string(),
            }),
        }
    }
    Ok(Json(ImportCreateResult { created, failed }))
}

/// #27 PATCH /connections/{id} — ws editor (global: root)
async fn update_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpsertConnectionReq>,
) -> ApiResult<Json<Connection>> {
    let conn = ctx.connections().get(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    if owner_private_enabled(&ctx).await {
        require_conn_owner_or_root(&user, &conn)?;
    }
    Ok(Json(ctx.connections().update(&id, req).await?))
}

/// #28 DELETE /connections/{id} — ws editor (global: root)
async fn delete_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let conn = ctx.connections().get(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    if owner_private_enabled(&ctx).await {
        require_conn_owner_or_root(&user, &conn)?;
    }
    ctx.connections().delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// #29 POST /connections/{id}/open — editor → Session
async fn open_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    body: Option<Json<OpenConnectionReq>>,
) -> ApiResult<Json<Session>> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let conn = ctx.connections().get(&id).await?;
    if owner_private_enabled(&ctx).await {
        require_conn_owner_or_root(&user, &conn)?;
    }
    let ws_id = conn
        .workspace_id
        .clone()
        .or(req.workspace_id)
        .ok_or_else(|| {
            Error::Invalid("opening a global connection requires 'workspace_id'".into())
        })?;
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    let session = ctx
        .connections()
        .open(&conn, &ws_id, &user.id, req.title, ctx.spawner().as_ref())
        .await?;
    Ok(Json(session))
}

/// #30 POST /connections/{id}/test — editor
///
/// For DB-kind connections (MySQL, Redis, MongoDB, ClickHouse) the probe is
/// routed through the DB Explorer's driver path when a [`DbTester`] is wired
/// in. The driver path reuses the warm SSH tunnel cache (cached `ssh -L` local
/// forward) rather than spawning a fresh `ssh -J` child per probe, so a second
/// `test` on an already-open connection skips the SSH handshake entirely.
/// SSH and Custom connections always fall back to the CLI subprocess path.
async fn test_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<TestConnectionResp>> {
    let conn = ctx.connections().get(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    if owner_private_enabled(&ctx).await {
        require_conn_owner_or_root(&user, &conn)?;
    }
    // DB-kind connections can reuse the warm SSH tunnel cache via the driver
    // path. SSH and Custom kinds have no driver backing and use the CLI path.
    let is_db_kind = matches!(
        conn.kind,
        ConnectionKind::Mysql
            | ConnectionKind::Redis
            | ConnectionKind::Mongodb
            | ConnectionKind::Clickhouse
    );
    let mut resp = if is_db_kind {
        if let Some(tester) = ctx.db_tester() {
            tester.test_db_connection(&id).await?
        } else {
            ctx.connections().test(&conn).await?
        }
    } else {
        ctx.connections().test(&conn).await?
    };
    // Overlay the SSH key-permission warning here — this single spot covers both
    // the driver path (which can't see the key file) and the CLI path uniformly,
    // including DB-over-SSH-tunnel connections (params["ssh"]["identity_file"]).
    // Independent of the probe outcome.
    resp.warn_key_perms = key_perms_warning_for(&conn.params);
    Ok(Json(resp))
}

/// PATCH /connections/{id}/pin — editor (global: root); toggle the pinned flag.
async fn pin_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PinConnectionReq>,
) -> ApiResult<Json<Connection>> {
    let conn = ctx.connections().get(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.connections().set_pinned(&id, req.pinned).await?))
}

// --- SFTP file browser ------------------------------------------------------
//
// Browse/read/transfer files over a connection's existing SSH auth by driving
// the system `sftp` binary (see `otto_ssh::SftpSession`). Mutations and
// transfers require Connections:Edit (enforced by the central policy guard) and
// the connection's workspace Editor role (enforced here, mirroring sibling
// handlers); browse/read require View / Viewer. Only `kind == ssh` is allowed.

/// Read a string param off a connection's JSON params (empty → None).
fn conn_param<'a>(conn: &'a Connection, key: &str) -> Option<&'a str> {
    conn.params
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Build `SftpParams` from an SSH connection's profile. Errors if the kind is
/// not `ssh` or the host is missing.
fn sftp_params_for(conn: &Connection) -> Result<SftpParams, Error> {
    if conn.kind != ConnectionKind::Ssh {
        return Err(Error::Invalid(
            "SFTP is only available for SSH connections".into(),
        ));
    }
    let host = conn_param(conn, "host").ok_or_else(|| {
        Error::Invalid("connection has no host — SFTP requires an SSH host".into())
    })?;
    // Port: accept a JSON number or numeric string; default 22.
    let port = match conn.params.get("port") {
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .ok_or_else(|| Error::Invalid("connection port is not a valid port number".into()))?,
        Some(serde_json::Value::String(s)) if !s.is_empty() => s
            .parse::<u16>()
            .map_err(|_| Error::Invalid("connection port is not a valid port number".into()))?,
        _ => 22,
    };
    Ok(SftpParams {
        host: host.to_string(),
        port,
        user: conn_param(conn, "user").map(str::to_string),
        identity_file: conn_param(conn, "identity_file").map(str::to_string),
        jump: conn_param(conn, "jump").map(str::to_string),
    })
}

/// Resolve the connection, enforce the workspace role, ensure it's SSH, and
/// build a live `SftpSession`. Shared by every SFTP handler.
async fn open_sftp<S: ConnectionsCtx>(
    ctx: &S,
    user: &User,
    id: &Id,
    min: WorkspaceRole,
) -> Result<SftpSession, ApiErr> {
    let conn = ctx.connections().get(id).await?;
    check_conn_role(ctx, user, &conn, min).await?;
    if owner_private_enabled(ctx).await {
        require_conn_owner_or_root(user, &conn)?;
    }
    let params = sftp_params_for(&conn)?;
    Ok(SftpSession::new(params)?)
}

/// Expand a leading `~` to the daemon user's `$HOME`.
fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}{rest}")
    } else {
        path.to_string()
    }
}

/// Query params for `GET …/sftp/list` and `…/sftp/read`.
#[derive(Debug, Default, Deserialize)]
pub struct SftpPathQuery {
    #[serde(default)]
    pub path: Option<String>,
}

/// GET /connections/{id}/sftp/list?path= — Connections:View. Empty path → pwd().
async fn sftp_list<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<SftpPathQuery>,
) -> ApiResult<Json<SftpListResp>> {
    let sftp = open_sftp(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let path = match q.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => sftp.pwd().await.map_err(ApiErr)?,
    };
    let entries = sftp.list(&path).await.map_err(ApiErr)?;
    Ok(Json(SftpListResp {
        path,
        entries: entries
            .into_iter()
            .map(|e| otto_core::api::SftpEntry {
                name: e.name,
                kind: e.kind,
                size: e.size,
                mtime: e.mtime,
                perms: e.perms,
                symlink_target: e.symlink_target,
            })
            .collect(),
    }))
}

/// POST /connections/{id}/sftp/download — Connections:Edit.
async fn sftp_download<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SftpDownloadReq>,
) -> ApiResult<Json<SftpDownloadResp>> {
    let sftp = open_sftp(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let local = expand_home(&req.local_path);
    // If the destination is an existing directory, sftp `get` lands the file
    // under it using the remote basename; otherwise treat it as a full file
    // path and ensure its parent dir exists.
    let local_path = std::path::Path::new(&local);
    let dest = if local_path.is_dir() {
        let base = std::path::Path::new(&req.remote_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".to_string());
        local_path.join(base)
    } else {
        if let Some(parent) = local_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ApiErr(Error::Invalid(format!("create local dir: {e}"))))?;
            }
        }
        local_path.to_path_buf()
    };
    let dest_str = dest.to_string_lossy().into_owned();
    sftp.download(&req.remote_path, &dest_str)
        .await
        .map_err(ApiErr)?;
    let bytes = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    Ok(Json(SftpDownloadResp {
        local_path: dest_str,
        bytes,
    }))
}

/// POST /connections/{id}/sftp/upload — Connections:Edit.
async fn sftp_upload<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SftpUploadReq>,
) -> ApiResult<StatusCode> {
    let sftp = open_sftp(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let local = expand_home(&req.local_path);
    sftp.upload(&local, &req.remote_path).await.map_err(ApiErr)?;
    Ok(StatusCode::OK)
}

/// POST /connections/{id}/sftp/mkdir — Connections:Edit.
async fn sftp_mkdir<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SftpMkdirReq>,
) -> ApiResult<StatusCode> {
    let sftp = open_sftp(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    sftp.mkdir(&req.path).await.map_err(ApiErr)?;
    Ok(StatusCode::OK)
}

/// POST /connections/{id}/sftp/remove — Connections:Edit. `dir` → rmdir else rm.
async fn sftp_remove<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SftpRemoveReq>,
) -> ApiResult<StatusCode> {
    let sftp = open_sftp(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if req.dir {
        sftp.rmdir(&req.path).await.map_err(ApiErr)?;
    } else {
        sftp.remove(&req.path).await.map_err(ApiErr)?;
    }
    Ok(StatusCode::OK)
}

/// POST /connections/{id}/sftp/rename — Connections:Edit.
async fn sftp_rename<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SftpRenameReq>,
) -> ApiResult<StatusCode> {
    let sftp = open_sftp(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    sftp.rename(&req.from, &req.to).await.map_err(ApiErr)?;
    Ok(StatusCode::OK)
}

/// Max bytes returned by the SFTP text viewer (1 MiB).
const SFTP_READ_CAP: u64 = 1024 * 1024;

/// GET /connections/{id}/sftp/read?path= — Connections:View. Downloads to a
/// temp file, reads up to a size cap, returns text + a truncated flag.
async fn sftp_read<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<SftpPathQuery>,
) -> ApiResult<Json<SftpReadResp>> {
    let sftp = open_sftp(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let remote = q
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| ApiErr(Error::Invalid("'path' is required".into())))?;

    // Download to a private temp file, then read+cap it locally.
    let tmp_dir = std::env::temp_dir().join(format!(
        "otto-sftp-read-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp_dir)
        .map_err(|e| ApiErr(Error::Internal(format!("create temp dir: {e}"))))?;
    let tmp_file = tmp_dir.join("file");
    let tmp_str = tmp_file.to_string_lossy().into_owned();

    let result = sftp.download(remote, &tmp_str).await;
    let resp = result.map_err(ApiErr).and_then(|_| {
        read_capped(&tmp_file).map_err(|e| ApiErr(Error::Invalid(format!("read file: {e}"))))
    });
    // Always clean up the temp dir, success or not.
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let (text, truncated) = resp?;
    Ok(Json(SftpReadResp { text, truncated }))
}

/// Read up to `SFTP_READ_CAP` bytes from a local file, returning the UTF-8
/// (lossy) text and whether it was truncated by the cap.
fn read_capped(path: &std::path::Path) -> std::io::Result<(String, bool)> {
    use std::io::Read;
    let size = std::fs::metadata(path)?.len();
    let truncated = size > SFTP_READ_CAP;
    let mut buf = Vec::with_capacity((size.min(SFTP_READ_CAP)) as usize);
    std::fs::File::open(path)?
        .take(SFTP_READ_CAP)
        .read_to_end(&mut buf)?;
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

// --- Connection sections ----------------------------------------------------

/// Sections live in one of two independent trees. Absent/unknown → the default
/// Connections-page tree; the Database Explorer passes "db".
fn norm_scope(scope: Option<&str>) -> ApiResult<&'static str> {
    match scope.map(str::trim) {
        None | Some("") | Some("connections") => Ok("connections"),
        Some("db") => Ok("db"),
        Some(_) => Err(ApiErr(Error::Invalid("unknown section scope".into()))),
    }
}

/// GET /workspaces/{id}/connection-sections — viewer.
/// Returns the single global section tree (the `scope` query param is accepted
/// for backward compatibility but ignored — both pages share one tree). The
/// path workspace is used only to authorize the caller.
async fn list_sections<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Query(_q): Query<SectionScopeQuery>,
) -> ApiResult<Json<Vec<ConnectionSection>>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(ctx.connections().list_sections().await?))
}

/// POST /workspaces/{id}/connection-sections — editor
async fn create_section<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<UpsertSectionReq>,
) -> ApiResult<Json<ConnectionSection>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiErr(Error::Invalid("section name required".into())));
    }
    let scope = norm_scope(req.scope.as_deref())?;
    Ok(Json(
        ctx.connections()
            .create_section(&ws_id, &user.id, req.parent_id.as_deref(), name, scope)
            .await?,
    ))
}

/// PATCH /connection-sections/{id} — editor in the section's workspace
async fn rename_section<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpsertSectionReq>,
) -> ApiResult<Json<ConnectionSection>> {
    let sec = ctx.connections().get_section(&id).await?;
    ctx.roles()
        .check(&user, &sec.workspace_id, WorkspaceRole::Editor)
        .await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiErr(Error::Invalid("section name required".into())));
    }
    Ok(Json(ctx.connections().rename_section(&id, name).await?))
}

/// DELETE /connection-sections/{id} — editor; connections fall back to ungrouped
async fn delete_section<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let sec = ctx.connections().get_section(&id).await?;
    ctx.roles()
        .check(&user, &sec.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.connections().delete_section(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /workspaces/{id}/connection-sections/reorder — editor
async fn reorder_sections<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<ReorderSectionsReq>,
) -> ApiResult<StatusCode> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    ctx.connections().reorder_sections(&ws_id, &req.ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /connection-sections/{id}/move — editor; reparent (None = top-level)
async fn move_section<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<MoveSectionReq>,
) -> ApiResult<Json<ConnectionSection>> {
    let sec = ctx.connections().get_section(&id).await?;
    ctx.roles()
        .check(&user, &sec.workspace_id, WorkspaceRole::Editor)
        .await?;
    Ok(Json(
        ctx.connections()
            .reparent_section(&id, req.parent_id.as_deref())
            .await?,
    ))
}
