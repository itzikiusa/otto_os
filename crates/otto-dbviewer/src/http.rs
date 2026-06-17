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
use crate::types::{CompletionContext, QueryRequest};

/// Server-side context required by the DB Explorer routes.
pub trait DbViewerCtx: Clone + Send + Sync + 'static {
    fn db(&self) -> &Arc<DbViewerService>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
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
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    limit: Option<i64>,
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
    Ok(Json(ctx.db().schema_children(&id, &req.path).await?).into_response())
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

async fn run_query<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<QueryRequest>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.db().run(&id, &req).await?).into_response())
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
    ctx.roles()
        .check(&user, &widget.workspace_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(ctx.db().run_widget(&id).await?).into_response())
}
