//! DB Explorer REST router. Nested under `/api/v1` by the server, which also
//! supplies the state via [`DbViewerCtx`]. Reads require workspace `Viewer`,
//! writes/execution require `Editor` (global connections: root only) — the same
//! model as `otto-connections`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{Connection, User, WorkspaceRole};
use otto_core::{Error, Id};
use otto_state::{NewSavedQuery, NewWidget};
use serde::Deserialize;
use serde_json::Value;

use crate::export::ExportFormat as PathFormat;
use crate::service::DbViewerService;
use crate::types::{statement_is_write, CompletionContext, Engine, QueryRequest};

/// Server-side context required by the DB Explorer routes.
pub trait DbViewerCtx: Clone + Send + Sync + 'static {
    fn db(&self) -> &Arc<DbViewerService>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;

    /// Audit hook fired when a guarded (production / read-only) connection runs
    /// a write the caller explicitly confirmed (`confirm_write`). Default no-op
    /// so this crate stays decoupled from the server's audit log; `ServerCtx`
    /// overrides it to append a `db.write_confirmed` entry. Best-effort — it
    /// must not block or fail the query.
    fn on_confirmed_write(&self, _user: &User, _conn: &Connection, _statement: &str) {}

    /// The natural-language→SQL drafter, when the server has configured one
    /// (it wires this to the agent/LLM). Default `None` keeps this crate free of
    /// any agent dependency; the `nl-to-sql` route returns a clear 400 when it's
    /// unset. Mirrors the `db()`/`roles()` accessor style.
    fn drafter(&self) -> Option<std::sync::Arc<dyn crate::nl::SqlDrafter>> {
        None
    }
}

/// Local problem-details mapper (orphan rule: can't impl IntoResponse for
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

#[derive(Debug, Deserialize)]
struct PathReq {
    path: String,
    /// Optional prefix filter for lazy children (Redis keyspace key filtering).
    #[serde(default)]
    filter: Option<String>,
    /// When `true`, the response includes an `approx_row_count` from
    /// `information_schema.table_rows` (InnoDB estimate — may be wildly off;
    /// opt-in because it adds a second query on every `object_detail` call).
    #[serde(default)]
    approx_row_count: bool,
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SchemaGraphReq {
    /// Schema/database to diagram.
    schema: String,
    /// Cap on tables introspected (each is one round-trip); clamped server-side.
    #[serde(default)]
    max_tables: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct CancelReq {
    /// The `query_id` the client sent on the `db/query` request to be cancelled.
    query_id: String,
}

#[derive(Debug, Deserialize)]
struct NewSavedQueryReq {
    #[serde(default)]
    connection_id: Option<Id>,
    name: String,
    statement: String,
}

#[derive(Debug, Deserialize)]
struct CreateDashboardReq {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct UpdateDashboardReq {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    layout: Option<Value>,
    #[serde(default)]
    refresh_secs: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CreateWidgetReq {
    connection_id: Id,
    title: String,
    statement: String,
    #[serde(default = "default_viz")]
    viz: String,
    #[serde(default)]
    dashboard_id: Option<Id>,
    #[serde(default)]
    mapping: Option<Value>,
    #[serde(default)]
    options: Option<Value>,
}

fn default_viz() -> String {
    "table".to_string()
}

#[derive(Debug, Default, Deserialize)]
struct UpdateWidgetReq {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    statement: Option<String>,
    #[serde(default)]
    viz: Option<String>,
    #[serde(default)]
    dashboard_id: Option<String>,
    #[serde(default)]
    mapping: Option<Value>,
    #[serde(default)]
    options: Option<Value>,
}

/// REST routes; the server nests this under `/api/v1` and supplies the state.
pub fn api_router<S: DbViewerCtx>() -> Router<S> {
    Router::new()
        // Connection-scoped operations.
        .route("/connections/{id}/db/test", post(test::<S>))
        .route("/connections/{id}/db/capabilities", get(capabilities::<S>))
        .route("/connections/{id}/db/schema", get(schema_root::<S>))
        .route("/connections/{id}/db/schema/children", post(schema_children::<S>))
        .route("/connections/{id}/db/object", post(object_detail::<S>))
        .route("/connections/{id}/db/schema-graph", post(schema_graph::<S>))
        .route("/connections/{id}/db/query", post(run_query::<S>))
        .route("/connections/{id}/db/export", post(export_query::<S>))
        .route(
            "/connections/{id}/db/export-to-path",
            post(export_to_path::<S>),
        )
        .route("/connections/{id}/db/cancel", post(cancel_query::<S>))
        .route("/connections/{id}/db/completion", post(completion::<S>))
        .route("/connections/{id}/db/import", post(import_query::<S>))
        .route("/connections/{id}/db/nl-to-sql", post(nl_to_sql::<S>))
        .route("/connections/{id}/db/history", get(history::<S>))
        // Saved queries.
        .route(
            "/workspaces/{wid}/db/saved-queries",
            get(list_saved::<S>).post(create_saved::<S>),
        )
        .route("/db/saved-queries/{qid}", delete(delete_saved::<S>))
        // Dashboards.
        .route(
            "/workspaces/{wid}/db/dashboards",
            get(list_dashboards::<S>).post(create_dashboard::<S>),
        )
        .route(
            "/db/dashboards/{id}",
            get(get_dashboard::<S>)
                .patch(update_dashboard::<S>)
                .delete(delete_dashboard::<S>),
        )
        // Widgets.
        .route(
            "/workspaces/{wid}/db/widgets",
            get(list_widgets::<S>).post(create_widget::<S>),
        )
        .route(
            "/db/widgets/{id}",
            patch(update_widget::<S>).delete(delete_widget::<S>),
        )
        .route("/db/widgets/{id}/run", post(run_widget::<S>))
}

/// Editor in the connection's workspace; for global connections: root only.
async fn check_conn_role<S: DbViewerCtx>(
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

/// Per-user ownership gate for the by-id saved-query / dashboard / widget
/// handlers. The list views are owner-private (they filter to
/// `created_by = caller` for non-root), so the by-id paths must enforce the same
/// ownership axis — otherwise a same-workspace co-member who learns a resource id
/// could read/mutate/delete another user's "private" item with just the
/// workspace Viewer/Editor role checked above. Allowed: root, the owner, or a
/// workspace **Admin** of the resource's workspace (the documented "sees all"
/// tier — mirrors `session_owner_or_admin`).
async fn require_owner_or_ws_admin<S: DbViewerCtx>(
    ctx: &S,
    user: &User,
    created_by: &Id,
    workspace_id: &Id,
) -> Result<(), Error> {
    if user.is_root || &user.id == created_by {
        return Ok(());
    }
    ctx.roles()
        .check(user, workspace_id, WorkspaceRole::Admin)
        .await
        .map_err(|_| Error::Forbidden("not the owner of this resource".into()))
}

// --- Connection-scoped handlers ---------------------------------------------

async fn test<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.db().test(&id).await?).into_response())
}

async fn capabilities<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.db().capabilities(&id).await?).into_response())
}

async fn schema_root<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.db().schema_root(&id).await?).into_response())
}

async fn schema_children<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PathReq>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.db().schema_children(&id, &req.path, req.filter.as_deref()).await?).into_response())
}

async fn object_detail<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PathReq>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.db().object_detail(&id, &req.path, req.approx_row_count).await?).into_response())
}

/// Read-only relationship graph (ERD) for a schema: tables + columns + FK edges.
/// Gated at `Viewer` like the other introspection routes.
async fn schema_graph<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SchemaGraphReq>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Viewer).await?;
    // Default to 60 tables; clamp so a request can't fan out into thousands of
    // per-table introspection round-trips.
    let max_tables = req.max_tables.unwrap_or(60).clamp(1, 200);
    Ok(Json(ctx.db().schema_graph(&id, &req.schema, max_tables).await?).into_response())
}

async fn run_query<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<QueryRequest>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    let result = ctx.db().run(&id, &user.id, &req).await?;
    // A confirmed write on a guarded (production / read-only) connection is a
    // security-relevant override worth auditing. Fire only after a successful
    // run so we don't log writes the engine rejected. `confirm_write` is a no-op
    // on an unguarded connection, so gate on `is_write_guarded()` to avoid noise
    // — and on the statement actually being a write (the same classifier
    // `guard_write` uses), so a guarded SELECT sent with `confirm_write:true`
    // isn't over-logged. Conservatively treat an unmappable engine as a write.
    let is_write = match Engine::from_kind(conn.kind) {
        Some(engine) => statement_is_write(engine, &req.statement),
        None => true,
    };
    if req.confirm_write && conn.is_write_guarded() && is_write {
        ctx.on_confirmed_write(&user, &conn, &req.statement);
    }
    Ok(Json(result).into_response())
}

/// Cancel an in-flight query (server-side, engine-native). Gated at the SAME
/// role as `run_query` (`Editor`; global connections: root only): issuing a
/// `KILL QUERY` is a privileged operation against the live database, and only
/// someone who could have *started* the query should be able to stop it. An
/// unknown / already-finished `query_id` is a no-op success (204), never an
/// error — the caller just wants the query gone.
async fn cancel_query<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CancelReq>,
) -> ApiResult<StatusCode> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    ctx.db().cancel(&id, &req.query_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Request body for the server-side export endpoint.
#[derive(Debug, Deserialize)]
struct ExportReq {
    statement: String,
    /// `csv` (default) or `json`.
    #[serde(default)]
    format: ExportFormat,
    /// Optional node context (active database) — same semantics as `QueryRequest.node`.
    #[serde(default)]
    node: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ExportFormat {
    #[default]
    Csv,
    Json,
}

/// Export a query result as CSV or NDJSON without the row cap applied to the
/// interactive query endpoint. Returns a file attachment so the browser downloads
/// it directly. Gated at `Editor` — same as `run_query` — because this executes
/// a statement against the live database.
async fn export_query<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<ExportReq>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;

    // Run without a row cap — the user opted in to a full export.
    let query_req = QueryRequest {
        statement: req.statement,
        max_rows: None,
        node: req.node,
        ..QueryRequest::default()
    };
    let result = ctx.db().run(&id, &user.id, &query_req).await?;

    match req.format {
        ExportFormat::Csv => {
            // Minimal CSV: RFC 4180 — double-quote fields that contain commas,
            // quotes, or newlines. Values are rendered as their JSON scalar form
            // (strings unquoted for readability; nulls as empty cells).
            let mut out = String::new();
            // Header row
            let header_row: Vec<String> = result.columns.iter().map(|c| csv_field(&c.name)).collect();
            out.push_str(&header_row.join(","));
            out.push('\n');
            for row in &result.rows {
                let fields: Vec<String> = row.iter().map(csv_value).collect();
                out.push_str(&fields.join(","));
                out.push('\n');
            }
            Ok((
                [
                    (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
                    (header::CONTENT_DISPOSITION, "attachment; filename=\"export.csv\""),
                ],
                out,
            )
                .into_response())
        }
        ExportFormat::Json => {
            // JSON array of objects [{col: val, …}].
            let col_names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
            let objects: Vec<serde_json::Map<String, Value>> = result
                .rows
                .into_iter()
                .map(|row| {
                    col_names
                        .iter()
                        .zip(row)
                        .map(|(&k, v)| (k.to_string(), v))
                        .collect()
                })
                .collect();
            let body = serde_json::to_string(&objects).unwrap_or_default();
            Ok((
                [
                    (header::CONTENT_TYPE, "application/json; charset=utf-8"),
                    (header::CONTENT_DISPOSITION, "attachment; filename=\"export.json\""),
                ],
                body,
            )
                .into_response())
        }
    }
}

/// Escape a string for a CSV field (RFC 4180).
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Render a JSON `Value` as a CSV cell (null → empty; strings unquoted when safe).
fn csv_value(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => csv_field(s),
        other => csv_field(&other.to_string()),
    }
}

/// Request body for the **streaming** local-file export endpoint.
#[derive(Debug, Deserialize)]
struct ExportToPathReq {
    /// The statement to run, uncapped (unless `max_rows` is set).
    statement: String,
    /// Optional node context (active database) — same semantics as `QueryRequest.node`.
    #[serde(default)]
    node: Option<String>,
    /// Selectable output format (default `csv`).
    #[serde(default)]
    format: PathFormat,
    /// Destination on the daemon host. A leading `~` is expanded to the daemon
    /// user's home. If it's an existing directory, the file is written as
    /// `<dir>/export.<ext>`; otherwise it's the full file path and its parent
    /// directory is created if missing.
    local_path: String,
    /// Optional cap; blank/absent = all rows. Stops the stream early when set.
    #[serde(default)]
    max_rows: Option<usize>,
}

/// Expand a leading `~` to the daemon user's `$HOME` (mirrors the SFTP handler).
fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix('~') {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}{rest}")
    } else {
        path.to_string()
    }
}

/// Resolve `local_path` (with `~` expanded) + the format's extension into the
/// final destination file path, ensuring the parent directory exists. When the
/// path is an existing directory, the file is `<dir>/export.<ext>`; otherwise the
/// path is taken as the full file path and its parent dir is created.
fn resolve_export_dest(local_path: &str, format: PathFormat) -> Result<std::path::PathBuf, ApiErr> {
    let expanded = expand_home(local_path);
    let p = std::path::Path::new(&expanded);
    let dest = if p.is_dir() {
        p.join(format!("export.{}", format.extension()))
    } else {
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ApiErr(Error::Invalid(format!("create local dir: {e}"))))?;
            }
        }
        p.to_path_buf()
    };
    Ok(dest)
}

/// Stream a query result to a **local file** on the daemon host, in a selectable
/// format, with bounded daemon memory (the driver streams row/chunk-by-chunk —
/// see `Driver::export_to_path`). For ClickHouse over an SSH tunnel this writes
/// the user's local path instead of a server-side `INTO OUTFILE` that would land
/// on the tunnel/remote host. Gated at `Editor` (global connections: root only),
/// same as `run_query` — it executes a statement against the live database.
async fn export_to_path<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<ExportToPathReq>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;

    if req.local_path.trim().is_empty() {
        return Err(ApiErr(Error::Invalid("local_path is required".into())));
    }
    let dest = resolve_export_dest(&req.local_path, req.format)?;

    // Stream progress as NDJSON so a large export never idles out the browser
    // fetch, and the user gets a live byte counter instead of a frozen spinner.
    // The export runs on a spawned task (the service is `Arc`-shared); a ticker
    // polls the destination file's size for progress, and the final line carries
    // the exact {rows, bytes, path}. No driver / ExportSink change — progress is
    // observed via the on-disk file size, leaving the bounded-RAM path untouched.
    let db = ctx.db().clone();
    let node = req
        .node
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let statement = req.statement.clone();
    let format = req.format;
    let max_rows = req.max_rows;
    let uid = user.id.clone();
    let conn_id = id.clone();
    let dest_task = dest.clone();

    // The producer only ever sends Ok — Infallible documents that the stream
    // itself never errors (export failures arrive as a `{error}` data line).
    let (tx, rx) =
        tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::convert::Infallible>>(16);

    tokio::spawn(async move {
        let export = db.export_to_path(
            &conn_id,
            &uid,
            &statement,
            node.as_deref(),
            format,
            max_rows,
            &dest_task,
        );
        tokio::pin!(export);
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(300));
        // If the export stalls the task >300ms, don't fire a burst of catch-up
        // ticks when it yields — one delayed tick is enough.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await; // consume the immediate first tick
        loop {
            tokio::select! {
                res = &mut export => {
                    let line = match res {
                        Ok((counts, duration_ms)) => serde_json::json!({
                            "done": true,
                            "local_path": dest_task.to_string_lossy(),
                            "rows": counts.rows,
                            "bytes": counts.bytes,
                            "duration_ms": duration_ms,
                        }),
                        Err(e) => serde_json::json!({ "error": e.to_string() }),
                    };
                    let _ = tx.send(Ok(ndjson_line(&line))).await;
                    break;
                }
                _ = ticker.tick() => {
                    let bytes = tokio::fs::metadata(&dest_task).await.map(|m| m.len()).unwrap_or(0);
                    if tx.send(Ok(ndjson_line(&serde_json::json!({ "bytes": bytes })))).await.is_err() {
                        // Client disconnected — stop reporting but still finish the
                        // file (so a closed browser tab doesn't truncate the export).
                        let _ = (&mut export).await;
                        break;
                    }
                }
            }
        }
    });

    let stream =
        futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|i| (i, rx)) });
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response())
}

/// Serialize one NDJSON progress line (`<json>\n`) as response bytes.
fn ndjson_line(v: &serde_json::Value) -> axum::body::Bytes {
    let mut s = serde_json::to_string(v).unwrap_or_default();
    s.push('\n');
    axum::body::Bytes::from(s)
}

/// Request body for the local-file → table import endpoint.
#[derive(Debug, Deserialize)]
struct ImportReq {
    /// Source file on the daemon host (leading `~` expands to the daemon home).
    local_path: String,
    /// File format.
    format: crate::import::ImportFormat,
    /// Destination table (must already exist).
    table: String,
    /// Rows per INSERT; clamped 1..=5000 server-side (default 500).
    #[serde(default)]
    batch_size: Option<usize>,
    /// Typed-confirmation acknowledgement for a guarded (Prod/read-only) connection.
    #[serde(default)]
    confirm_write: bool,
}

/// Import a local file into a SQL table. Gated at `Editor` (global: root). Streams
/// a single NDJSON result line (`{done,rows,batches}` or `{error}`) so the client
/// uses the same reader as export. A write on a guarded connection without
/// `confirm_write` returns the standard `write_blocked:` error in the `{error}`
/// line. Each INSERT batch routes through `DbViewerService::run` → `guard_write`,
/// so no parallel safety code is added here.
async fn import_query<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<ImportReq>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    if req.local_path.trim().is_empty() || req.table.trim().is_empty() {
        return Err(ApiErr(Error::Invalid(
            "local_path and table are required".into(),
        )));
    }
    let path = expand_home(&req.local_path);
    let batch_size = req.batch_size.unwrap_or(500).clamp(1, 5000);

    let db = ctx.db().clone();
    let uid = user.id.clone();
    let conn_id = id.clone();
    let (table, format, confirm) = (req.table.clone(), req.format, req.confirm_write);

    // The producer only ever sends Ok — Infallible documents that the stream
    // itself never errors (import failures arrive as an `{error}` data line). v1
    // runs the import to completion and emits one final line (no progress ticker
    // — there's no on-disk file to size as with export).
    let (tx, rx) =
        tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::convert::Infallible>>(4);
    tokio::spawn(async move {
        let line = match db
            .import_from_path(&conn_id, &uid, &path, format, &table, batch_size, confirm)
            .await
        {
            Ok(c) => serde_json::json!({ "done": true, "rows": c.rows, "batches": c.batches }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        };
        let _ = tx.send(Ok(ndjson_line(&line))).await;
    });

    let stream =
        futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|i| (i, rx)) });
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response())
}

/// Request body for the verified NL→SQL endpoint.
#[derive(Debug, Deserialize)]
struct NlToSqlReq {
    /// The user's plain-English question.
    question: String,
    /// Optional active-database node (same semantics as `QueryRequest.node`).
    #[serde(default)]
    node: Option<String>,
    /// Draft/validate retries; clamped 1..=4 server-side (default 3).
    #[serde(default)]
    max_attempts: Option<u32>,
}

/// Draft a **read** query from natural language and return it only after it has
/// been validated with `EXPLAIN` against the live schema. Gated at `Editor`
/// (global connections: root) — it runs `EXPLAIN` against the database. Returns
/// 400 when no drafter is configured.
async fn nl_to_sql<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<NlToSqlReq>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;

    let engine = Engine::from_kind(conn.kind)
        .ok_or_else(|| Error::Invalid("connection is not a browsable database".into()))?;
    let drafter = ctx
        .drafter()
        .ok_or_else(|| Error::Invalid("NL-to-SQL is not configured on this server".into()))?;

    let summary = ctx
        .db()
        .schema_summary(&id, req.node.as_deref(), 40)
        .await
        .unwrap_or_default();

    let validator = crate::service::ServiceValidator {
        db: ctx.db(),
        conn_id: id.clone(),
        user_id: user.id.clone(),
        node: req.node.clone(),
    };

    let outcome = crate::nl::drive_nl_to_sql(
        engine,
        &req.question,
        &summary,
        drafter.as_ref(),
        &validator,
        req.max_attempts.unwrap_or(3),
    )
    .await?;

    Ok(Json(outcome).into_response())
}

async fn completion<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CompletionContext>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.db().completion(&id, &req).await?).into_response())
}

async fn history<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Viewer).await?;
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    // Root sees all history on the connection; non-root callers see only
    // their own rows (#L11). Legacy rows (user_id = NULL, pre-0041) are
    // invisible to non-root callers — acceptable, as they predate multi-user.
    let entries = if user.is_root {
        ctx.db().list_history(&id, limit).await?
    } else {
        ctx.db().list_history_for_user(&id, &user.id, limit).await?
    };
    Ok(Json(entries).into_response())
}

// --- Saved queries ----------------------------------------------------------

async fn list_saved<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &wid, WorkspaceRole::Viewer).await?;
    // Root sees all saved queries in the workspace; non-root callers see only
    // their own (#L12). The `created_by` column has been present since 0021, so
    // no legacy-null concern here.
    let queries = if user.is_root {
        ctx.db().list_saved(&wid).await?
    } else {
        ctx.db().list_saved_for_user(&wid, &user.id).await?
    };
    Ok(Json(queries).into_response())
}

async fn create_saved<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
    Json(req): Json<NewSavedQueryReq>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &wid, WorkspaceRole::Editor).await?;
    let saved = ctx
        .db()
        .create_saved(NewSavedQuery {
            workspace_id: wid,
            connection_id: req.connection_id,
            name: req.name,
            statement: req.statement,
            created_by: user.id,
        })
        .await?;
    Ok(Json(saved).into_response())
}

async fn delete_saved<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(qid): Path<Id>,
) -> ApiResult<StatusCode> {
    // Deletion requires editor on the workspace the query was saved in, AND
    // ownership (owner / ws-Admin / root) — saved queries are owner-private.
    let saved = ctx.db().get_saved(&qid).await?;
    ctx.roles()
        .check(&user, &saved.workspace_id, WorkspaceRole::Editor)
        .await?;
    require_owner_or_ws_admin(&ctx, &user, &saved.created_by, &saved.workspace_id).await?;
    ctx.db().delete_saved(&qid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Dashboards -------------------------------------------------------------

async fn list_dashboards<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &wid, WorkspaceRole::Viewer).await?;
    // Root sees all dashboards; non-root sees only their own (#L13).
    let dashboards = if user.is_root {
        ctx.db().list_dashboards(&wid).await?
    } else {
        ctx.db().list_dashboards_for_user(&wid, &user.id).await?
    };
    Ok(Json(dashboards).into_response())
}

async fn create_dashboard<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
    Json(req): Json<CreateDashboardReq>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &wid, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.db().create_dashboard(&wid, &req.name, &user.id).await?).into_response())
}

async fn get_dashboard<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let dash = ctx.db().get_dashboard(&id).await?;
    ctx.roles()
        .check(&user, &dash.workspace_id, WorkspaceRole::Viewer)
        .await?;
    require_owner_or_ws_admin(&ctx, &user, &dash.created_by, &dash.workspace_id).await?;
    Ok(Json(dash).into_response())
}

async fn update_dashboard<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpdateDashboardReq>,
) -> ApiResult<Response> {
    let dash = ctx.db().get_dashboard(&id).await?;
    ctx.roles()
        .check(&user, &dash.workspace_id, WorkspaceRole::Editor)
        .await?;
    require_owner_or_ws_admin(&ctx, &user, &dash.created_by, &dash.workspace_id).await?;
    let updated = ctx
        .db()
        .update_dashboard(
            &id,
            req.name.as_deref(),
            req.layout.as_ref(),
            req.refresh_secs.map(Some),
        )
        .await?;
    Ok(Json(updated).into_response())
}

async fn delete_dashboard<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let dash = ctx.db().get_dashboard(&id).await?;
    ctx.roles()
        .check(&user, &dash.workspace_id, WorkspaceRole::Editor)
        .await?;
    require_owner_or_ws_admin(&ctx, &user, &dash.created_by, &dash.workspace_id).await?;
    ctx.db().delete_dashboard(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Widgets ----------------------------------------------------------------

async fn list_widgets<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &wid, WorkspaceRole::Viewer).await?;
    // Root sees all widgets; non-root sees only their own (#L13).
    let widgets = if user.is_root {
        ctx.db().list_widgets(&wid).await?
    } else {
        ctx.db().list_widgets_for_user(&wid, &user.id).await?
    };
    Ok(Json(widgets).into_response())
}

async fn create_widget<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
    Json(req): Json<CreateWidgetReq>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &wid, WorkspaceRole::Editor).await?;
    let widget = ctx
        .db()
        .create_widget(NewWidget {
            workspace_id: wid,
            dashboard_id: req.dashboard_id,
            connection_id: req.connection_id,
            title: req.title,
            statement: req.statement,
            viz: req.viz,
            mapping: req.mapping.unwrap_or_else(|| serde_json::json!({})),
            options: req.options.unwrap_or_else(|| serde_json::json!({})),
            created_by: user.id,
        })
        .await?;
    Ok(Json(widget).into_response())
}

async fn update_widget<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpdateWidgetReq>,
) -> ApiResult<Response> {
    let widget = ctx.db().get_widget(&id).await?;
    ctx.roles()
        .check(&user, &widget.workspace_id, WorkspaceRole::Editor)
        .await?;
    require_owner_or_ws_admin(&ctx, &user, &widget.created_by, &widget.workspace_id).await?;
    let updated = ctx
        .db()
        .update_widget(
            &id,
            req.dashboard_id.as_deref().map(Some),
            req.title.as_deref(),
            req.statement.as_deref(),
            req.viz.as_deref(),
            req.mapping.as_ref(),
            req.options.as_ref(),
        )
        .await?;
    Ok(Json(updated).into_response())
}

async fn delete_widget<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let widget = ctx.db().get_widget(&id).await?;
    ctx.roles()
        .check(&user, &widget.workspace_id, WorkspaceRole::Editor)
        .await?;
    require_owner_or_ws_admin(&ctx, &user, &widget.created_by, &widget.workspace_id).await?;
    ctx.db().delete_widget(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn run_widget<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    let widget = ctx.db().get_widget(&id).await?;
    // Security (audit S7): `run_widget` executes the widget's stored statement
    // through the SAME `DbViewerService::run` path as `run_query` — which can
    // run arbitrary SQL/commands, including writes and DDL. It must therefore
    // require the SAME role as `run_query`: `Editor` (for global connections:
    // root only via `check_conn_role`). Gating it on `Viewer` was a privilege
    // escalation — a workspace Viewer could trigger arbitrary stored writes by
    // running a dashboard tile. We deliberately do NOT try to read-only-classify
    // the statement here: that classification is per-engine (SQL keywords vs
    // Redis commands vs Mongo ops) and lives privately in each driver, so a
    // route-level reimplementation would be exactly the bypassable half-measure
    // we're trying to avoid. Gating on the connection (not just the widget's
    // workspace) also matches every other execution path and correctly handles
    // global connections.
    let conn = ctx.db().get_connection(&widget.connection_id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    // Widgets are owner-private: only the owner / ws-Admin / root may execute a
    // stored widget statement (don't let a co-member run another user's tile).
    require_owner_or_ws_admin(&ctx, &user, &widget.created_by, &widget.workspace_id).await?;
    Ok(Json(ctx.db().run_widget(&id, &user.id).await?).into_response())
}

#[cfg(test)]
mod tests {
    //! Role-gate tests for the connection-execution path that `run_widget` now
    //! shares with `run_query` (audit S7). We exercise [`check_conn_role`]
    //! directly: it is the load-bearing gate, and testing it with a recording
    //! [`RoleChecker`] avoids standing up a full `DbViewerService` (DB + secrets)
    //! while still proving the security property — a Viewer is rejected where
    //! `Editor` is required, and global connections stay root-only.

    use std::sync::Mutex;

    use otto_core::auth::{BoxFuture, RoleChecker};
    use otto_core::domain::{Connection, ConnectionKind, User, WorkspaceRole};
    use otto_core::{new_id, Error, Id, Result};

    use super::{check_conn_role, require_owner_or_ws_admin};
    use crate::service::DbViewerService;
    use std::sync::Arc;

    /// A `RoleChecker` whose verdict is fixed per `(role-held)` and which records
    /// the `min` role it was asked to verify, so a test can assert the gate
    /// demanded `Editor` (not `Viewer`).
    struct StubRoles {
        /// The role this user actually holds in the workspace.
        held: WorkspaceRole,
        /// The last `min` role `check` was asked for.
        last_min: Mutex<Option<WorkspaceRole>>,
    }

    impl StubRoles {
        fn new(held: WorkspaceRole) -> Self {
            Self {
                held,
                last_min: Mutex::new(None),
            }
        }
    }

    impl RoleChecker for StubRoles {
        fn check<'a>(
            &'a self,
            _user: &'a User,
            _workspace_id: &'a Id,
            min: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            *self.last_min.lock().unwrap() = Some(min);
            // `held >= min` (WorkspaceRole derives Ord: Viewer<Editor<Admin, the
            // same ordering otto-rbac uses) passes; else 403.
            let ok = self.held >= min;
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(Error::Forbidden("insufficient role".into()))
                }
            })
        }
    }

    /// A `DbViewerCtx` carrying only the roles stub — `db()` is never touched by
    /// `check_conn_role`, so a check-only test needs no live service.
    #[derive(Clone)]
    struct TestCtx {
        roles: Arc<dyn RoleChecker>,
    }

    impl super::DbViewerCtx for TestCtx {
        fn db(&self) -> &Arc<DbViewerService> {
            unreachable!("check_conn_role must not dereference the service")
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
    }

    fn user(is_root: bool) -> User {
        User {
            id: new_id(),
            username: "u".into(),
            display_name: "U".into(),
            is_root,
            disabled: false,
            created_at: chrono::Utc::now(),
        }
    }

    fn conn(workspace_id: Option<Id>) -> Connection {
        Connection {
            id: new_id(),
            workspace_id,
            name: "c".into(),
            kind: ConnectionKind::Mysql,
            params: serde_json::json!({}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Default::default(),
            read_only: false,
            created_by: new_id(),
            created_at: chrono::Utc::now(),
            last_opened_at: None,
            pinned: false,
        }
    }

    #[tokio::test]
    async fn viewer_is_rejected_from_widget_execution_gate() {
        // A workspace Viewer hitting the run_widget gate (Editor required) → 403.
        let stub = Arc::new(StubRoles::new(WorkspaceRole::Viewer));
        let ctx = TestCtx {
            roles: stub.clone(),
        };
        let ws = new_id();
        let c = conn(Some(ws));
        let err = check_conn_role(&ctx, &user(false), &c, WorkspaceRole::Editor)
            .await
            .expect_err("viewer must be denied Editor");
        assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
        // And the gate really demanded Editor, not Viewer.
        assert_eq!(*stub.last_min.lock().unwrap(), Some(WorkspaceRole::Editor));
    }

    #[tokio::test]
    async fn editor_passes_widget_execution_gate() {
        // The legitimate dashboard editor still runs widgets.
        let stub = Arc::new(StubRoles::new(WorkspaceRole::Editor));
        let ctx = TestCtx { roles: stub };
        let c = conn(Some(new_id()));
        check_conn_role(&ctx, &user(false), &c, WorkspaceRole::Editor)
            .await
            .expect("editor allowed");
    }

    #[tokio::test]
    async fn global_connection_requires_root() {
        // A global (workspace-less) connection: a non-root user is denied
        // regardless of any workspace role, while root passes — matching every
        // other execution path.
        let stub = Arc::new(StubRoles::new(WorkspaceRole::Admin));
        let ctx = TestCtx {
            roles: stub.clone(),
        };
        let global = conn(None);

        let err = check_conn_role(&ctx, &user(false), &global, WorkspaceRole::Editor)
            .await
            .expect_err("non-root denied on global conn");
        assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
        // The roles checker isn't even consulted for a global connection.
        assert_eq!(*stub.last_min.lock().unwrap(), None);

        check_conn_role(&ctx, &user(true), &global, WorkspaceRole::Editor)
            .await
            .expect("root passes on global conn");
    }

    #[tokio::test]
    async fn import_gate_requires_editor() {
        // File import is a write path — a Viewer is denied the Editor gate.
        let stub = Arc::new(StubRoles::new(WorkspaceRole::Viewer));
        let ctx = TestCtx { roles: stub.clone() };
        let c = conn(Some(new_id()));
        let err = check_conn_role(&ctx, &user(false), &c, WorkspaceRole::Editor)
            .await
            .expect_err("viewer denied");
        assert!(matches!(err, Error::Forbidden(_)));
        assert_eq!(*stub.last_min.lock().unwrap(), Some(WorkspaceRole::Editor));
    }

    #[tokio::test]
    async fn nl_to_sql_gate_requires_editor() {
        // The NL→SQL route shares run_query's gate: a Viewer is denied Editor.
        let stub = Arc::new(StubRoles::new(WorkspaceRole::Viewer));
        let ctx = TestCtx { roles: stub.clone() };
        let c = conn(Some(new_id()));
        let err = check_conn_role(&ctx, &user(false), &c, WorkspaceRole::Editor)
            .await
            .expect_err("viewer denied");
        assert!(matches!(err, Error::Forbidden(_)));
        assert_eq!(*stub.last_min.lock().unwrap(), Some(WorkspaceRole::Editor));
    }

    // -- Per-user ownership gate (by-id IDOR fix) ----------------------------
    //
    // The by-id saved-query / dashboard / widget handlers run
    // `require_owner_or_ws_admin` after the workspace-role check, so a
    // same-workspace co-member who learns a resource id can't read/mutate/run
    // another user's owner-private item. We test the gate directly (the
    // load-bearing check) with the recording `StubRoles`.

    #[tokio::test]
    async fn owner_passes_ownership_gate_without_role_check() {
        // The owner passes for their OWN resource even as a bare Viewer; the
        // gate short-circuits before consulting workspace roles.
        let stub = Arc::new(StubRoles::new(WorkspaceRole::Viewer));
        let ctx = TestCtx { roles: stub.clone() };
        let u = user(false);
        let ws = new_id();
        require_owner_or_ws_admin(&ctx, &u, &u.id, &ws)
            .await
            .expect("owner allowed");
        assert_eq!(*stub.last_min.lock().unwrap(), None, "owner short-circuits");
    }

    #[tokio::test]
    async fn non_owner_editor_is_denied_ownership_gate() {
        // A same-workspace Editor who is NOT the owner is denied: the gate
        // demands ws-Admin (the documented "sees all" tier), which Editor lacks.
        let stub = Arc::new(StubRoles::new(WorkspaceRole::Editor));
        let ctx = TestCtx { roles: stub.clone() };
        let bob = user(false);
        let alice_id = new_id(); // a different owner
        let ws = new_id();
        let err = require_owner_or_ws_admin(&ctx, &bob, &alice_id, &ws)
            .await
            .expect_err("non-owner editor must be denied");
        assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
        assert_eq!(*stub.last_min.lock().unwrap(), Some(WorkspaceRole::Admin));
    }

    #[tokio::test]
    async fn ws_admin_and_root_pass_ownership_gate() {
        // A workspace Admin passes for someone else's resource (sees-all tier).
        let admin_stub = Arc::new(StubRoles::new(WorkspaceRole::Admin));
        let admin_ctx = TestCtx { roles: admin_stub };
        let alice_id = new_id();
        let ws = new_id();
        require_owner_or_ws_admin(&admin_ctx, &user(false), &alice_id, &ws)
            .await
            .expect("ws-admin allowed");

        // Root passes without consulting roles at all.
        let root_stub = Arc::new(StubRoles::new(WorkspaceRole::Viewer));
        let root_ctx = TestCtx { roles: root_stub.clone() };
        require_owner_or_ws_admin(&root_ctx, &user(true), &alice_id, &ws)
            .await
            .expect("root allowed");
        assert_eq!(*root_stub.last_min.lock().unwrap(), None, "root short-circuits");
    }
}
