//! Message Brokers REST router. Nested under `/api/v1` by the server, which
//! supplies state via [`BrokersCtx`]. Reads require workspace `Viewer`; cluster
//! management and mutations require `Editor` (global clusters: root only).
//! Mutations on a guarded cluster (production / read-only) need an explicit
//! `confirm`, mirroring the DB Explorer write-gate.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id};
use otto_state::BrokerClusterRow;
use serde::Deserialize;

use crate::service::BrokersService;
use crate::types::*;

/// Server-side context required by the Message Brokers routes.
pub trait BrokersCtx: Clone + Send + Sync + 'static {
    fn brokers(&self) -> &Arc<BrokersService>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
}

/// Local problem mapper (orphan rule: can't impl `IntoResponse` for `Error`).
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

pub fn api_router<S: BrokersCtx>() -> Router<S> {
    Router::new()
        .route(
            "/workspaces/{wid}/brokers/clusters",
            get(list_clusters::<S>).post(create_cluster::<S>),
        )
        .route(
            "/brokers/clusters/{id}",
            get(get_cluster::<S>)
                .patch(update_cluster::<S>)
                .delete(delete_cluster::<S>),
        )
        .route("/brokers/clusters/{id}/test", post(test::<S>))
        .route("/brokers/clusters/{id}/overview", get(overview::<S>))
        .route("/brokers/clusters/{id}/metrics", get(metrics::<S>))
        .route(
            "/brokers/clusters/{id}/topics",
            get(list_topics::<S>).post(create_topic::<S>),
        )
        .route(
            "/brokers/clusters/{id}/topics/{topic}",
            get(topic_detail::<S>).delete(delete_topic::<S>),
        )
        .route(
            "/brokers/clusters/{id}/topics/{topic}/stats",
            get(topic_stats::<S>),
        )
        .route(
            "/brokers/clusters/{id}/topics/{topic}/configs",
            get(get_configs::<S>).put(put_configs::<S>),
        )
        .route(
            "/brokers/clusters/{id}/topics/{topic}/consume",
            post(consume::<S>),
        )
        .route(
            "/brokers/clusters/{id}/topics/{topic}/produce",
            post(produce::<S>),
        )
        .route("/brokers/clusters/{id}/groups", get(list_groups::<S>))
        .route(
            "/brokers/clusters/{id}/groups/{group}",
            get(describe_group::<S>),
        )
        .route(
            "/brokers/clusters/{id}/schema-registry/subjects",
            get(schema_subjects::<S>),
        )
}

// ---- authorization helpers ------------------------------------------------

/// Fetch the cluster row and enforce the workspace role (root for globals).
async fn authorize<S: BrokersCtx>(
    ctx: &S,
    user: &User,
    id: &Id,
    min: WorkspaceRole,
) -> Result<BrokerClusterRow, Error> {
    let row = ctx.brokers().get_row(id).await?;
    match &row.workspace_id {
        Some(ws) => ctx.roles().check(user, ws, min).await?,
        None => {
            if !user.is_root {
                return Err(Error::Forbidden(
                    "global broker clusters are managed by root".into(),
                ));
            }
        }
    }
    Ok(row)
}

/// Reject a mutation on a guarded (prod / read-only) cluster unless confirmed.
fn guard(row: &BrokerClusterRow, confirmed: bool) -> Result<(), Error> {
    let guarded = row.read_only || row.environment == "prod";
    if guarded && !confirmed {
        let why = if row.read_only {
            "read-only"
        } else {
            "production"
        };
        return Err(Error::Forbidden(format!(
            "cluster '{}' is {why}; set confirm=true to proceed",
            row.name
        )));
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
struct ConfirmQuery {
    #[serde(default)]
    confirm: bool,
}

// ---- cluster CRUD ---------------------------------------------------------

async fn list_clusters<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &wid, WorkspaceRole::Viewer)
        .await?;
    let clusters = ctx.brokers().list_clusters(&wid).await?;
    Ok(Json(clusters).into_response())
}

async fn create_cluster<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
    Json(req): Json<UpsertClusterReq>,
) -> ApiResult<Response> {
    ctx.roles()
        .check(&user, &wid, WorkspaceRole::Editor)
        .await?;
    let cluster = ctx
        .brokers()
        .create_cluster(Some(wid), user.id.clone(), req)
        .await?;
    Ok((StatusCode::CREATED, Json(cluster)).into_response())
}

async fn get_cluster<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().get_cluster(&id).await?).into_response())
}

async fn update_cluster<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpsertClusterReq>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.brokers().update_cluster(&id, req).await?).into_response())
}

async fn delete_cluster<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    ctx.brokers().delete_cluster(&id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn test<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.brokers().test(&id).await?).into_response())
}

// ---- cluster reads --------------------------------------------------------

async fn overview<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().overview(&id).await?).into_response())
}

async fn metrics<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().metrics(&id).await?).into_response())
}

// ---- topics ---------------------------------------------------------------

async fn list_topics<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().list_topics(&id).await?).into_response())
}

async fn create_topic<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<CreateTopicReq>,
) -> ApiResult<Response> {
    let row = authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    guard(&row, req.confirm)?;
    let topic = ctx.brokers().create_topic(&id, &req).await?;
    Ok((StatusCode::CREATED, Json(topic)).into_response())
}

async fn topic_detail<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, topic)): Path<(Id, String)>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().topic_detail(&id, &topic).await?).into_response())
}

async fn topic_stats<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, topic)): Path<(Id, String)>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().topic_stats(&id, &topic).await?).into_response())
}

async fn delete_topic<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, topic)): Path<(Id, String)>,
    Query(q): Query<ConfirmQuery>,
) -> ApiResult<Response> {
    let row = authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    guard(&row, q.confirm)?;
    ctx.brokers().delete_topic(&id, &topic).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn get_configs<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, topic)): Path<(Id, String)>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().topic_configs(&id, &topic).await?).into_response())
}

async fn put_configs<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, topic)): Path<(Id, String)>,
    Json(req): Json<AlterConfigsReq>,
) -> ApiResult<Response> {
    let row = authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    guard(&row, req.confirm)?;
    let configs = ctx
        .brokers()
        .alter_configs(&id, &topic, &req.configs)
        .await?;
    Ok(Json(configs).into_response())
}

async fn consume<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, topic)): Path<(Id, String)>,
    Json(req): Json<ConsumeReq>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().consume(&id, &topic, &req).await?).into_response())
}

async fn produce<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, topic)): Path<(Id, String)>,
    Json(req): Json<ProduceReq>,
) -> ApiResult<Response> {
    let row = authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    guard(&row, req.confirm)?;
    Ok(Json(ctx.brokers().produce(&id, &topic, &req).await?).into_response())
}

// ---- consumer groups + schema registry ------------------------------------

async fn list_groups<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().list_groups(&id).await?).into_response())
}

async fn describe_group<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, group)): Path<(Id, String)>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().describe_group(&id, &group).await?).into_response())
}

async fn schema_subjects<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().schema_subjects(&id).await?).into_response())
}
