//! DB Explorer REST router. Nested under `/api/v1` by the server, which also
//! supplies the state via [`DbViewerCtx`]. Reads require workspace `Viewer`,
//! writes/execution require `Editor` (global connections: root only) — the same
//! model as `otto-connections`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
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
        .route("/connections/{id}/db/completion", post(completion::<S>))
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
    Ok(Json(ctx.db().object_detail(&id, &req.path).await?).into_response())
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
    let result = ctx.db().run(&id, &req).await?;
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
    Ok(Json(ctx.db().list_history(&id, limit).await?).into_response())
}

// --- Saved queries ----------------------------------------------------------

async fn list_saved<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.db().list_saved(&wid).await?).into_response())
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
    // Deletion requires editor on the workspace the query was saved in.
    let saved = ctx.db().get_saved(&qid).await?;
    ctx.roles()
        .check(&user, &saved.workspace_id, WorkspaceRole::Editor)
        .await?;
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
    Ok(Json(ctx.db().list_dashboards(&wid).await?).into_response())
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
    Ok(Json(ctx.db().list_widgets(&wid).await?).into_response())
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
    Ok(Json(ctx.db().run_widget(&id).await?).into_response())
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

    use super::check_conn_role;
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
}
