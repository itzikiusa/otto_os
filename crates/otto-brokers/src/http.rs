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
use std::collections::HashMap;

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
            "/brokers/clusters/{id}/topics/stats",
            post(batch_topic_stats::<S>),
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
            "/brokers/clusters/{id}/groups/{group}/reset",
            post(reset_group_offsets::<S>),
        )
        .route("/brokers/clusters/{id}/replay", post(replay::<S>))
        .route(
            "/brokers/clusters/{id}/schema-registry/subjects",
            get(schema_subjects::<S>),
        )
        .route(
            "/brokers/clusters/{id}/schema-registry/subjects/{subject}/versions",
            get(schema_subject_versions::<S>),
        )
        .route(
            "/brokers/clusters/{id}/schema-registry/subjects/{subject}/versions/{version}",
            get(schema_subject_version_detail::<S>),
        )
        .route(
            "/brokers/clusters/{id}/schema-registry/subjects/{subject}/compatibility",
            post(schema_check_compatibility::<S>),
        )
        .route(
            "/brokers/clusters/{id}/lag-alerts",
            get(list_lag_alerts::<S>).post(create_lag_alert::<S>),
        )
        .route(
            "/brokers/clusters/{id}/lag-alerts/{alert_id}",
            axum::routing::delete(delete_lag_alert::<S>),
        )
        // ---- cluster sections (sidebar grouping) ----
        .route(
            "/workspaces/{wid}/brokers/cluster-sections",
            get(list_sections::<S>).post(create_section::<S>),
        )
        .route(
            "/brokers/cluster-sections/{id}",
            axum::routing::patch(rename_section::<S>).delete(delete_section::<S>),
        )
        .route(
            "/brokers/cluster-sections/{id}/move",
            post(move_section::<S>),
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

#[derive(Debug, Default, Deserialize)]
struct ResetQuery {
    /// Set to true to preview what the reset would do without committing.
    #[serde(default)]
    dry_run: bool,
}

// ---- cluster sections -----------------------------------------------------

/// Fetch a section and enforce the role in its workspace.
async fn authorize_section<S: BrokersCtx>(
    ctx: &S,
    user: &User,
    id: &Id,
    min: WorkspaceRole,
) -> Result<BrokerClusterSection, Error> {
    let sec = ctx.brokers().get_section(id).await?;
    ctx.roles().check(user, &sec.workspace_id, min).await?;
    Ok(sec)
}

async fn list_sections<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().list_sections(&wid).await?).into_response())
}

async fn create_section<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(wid): Path<Id>,
    Json(req): Json<UpsertSectionReq>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &wid, WorkspaceRole::Editor).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("section name required".into()).into());
    }
    let sec = ctx
        .brokers()
        .create_section(&wid, &user.id, req.parent_id.as_deref(), name)
        .await?;
    Ok((StatusCode::CREATED, Json(sec)).into_response())
}

async fn rename_section<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpsertSectionReq>,
) -> ApiResult<Response> {
    authorize_section(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(Error::Invalid("section name required".into()).into());
    }
    Ok(Json(ctx.brokers().rename_section(&id, name).await?).into_response())
}

async fn delete_section<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize_section(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    ctx.brokers().delete_section(&id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn move_section<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<MoveSectionReq>,
) -> ApiResult<Response> {
    authorize_section(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.brokers().move_section(&id, req.parent_id.as_deref()).await?).into_response())
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
    // Broker clusters are a global infrastructure library (like DB connections):
    // created workspace-independent so they appear in every workspace. The path
    // workspace is used only to authorize the caller.
    let cluster = ctx
        .brokers()
        .create_cluster(None, user.id.clone(), req)
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

/// Batch-load message counts for multiple topics in one call. The UI uses this
/// instead of N×1 `/topics/{name}/stats` calls to reduce round-trips.
async fn batch_topic_stats<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<BatchStatsReq>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    if req.names.len() > 500 {
        return Err(Error::Invalid("batch stats: at most 500 names per call".into()).into());
    }
    Ok(Json(ctx.brokers().batch_topic_stats(&id, req.names).await?).into_response())
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
    ctx.brokers()
        .audit_write(&id, &user.id, "delete_topic", serde_json::json!({ "topic": topic }))
        .await;
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
    ctx.brokers()
        .audit_write(
            &id,
            &user.id,
            "alter_configs",
            serde_json::json!({ "topic": topic, "count": req.configs.len() }),
        )
        .await;
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
    let resp = ctx.brokers().produce(&id, &topic, &req).await?;
    ctx.brokers()
        .audit_write(&id, &user.id, "produce", serde_json::json!({ "topic": topic }))
        .await;
    Ok(Json(resp).into_response())
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

async fn reset_group_offsets<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, group)): Path<(Id, String)>,
    Query(q): Query<ResetQuery>,
    Json(req): Json<GroupResetReq>,
) -> ApiResult<Response> {
    let row = authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;

    if q.dry_run {
        // Dry-run: compute target offsets and lag delta without committing.
        // Guard is still checked (the operator should know they're touching a
        // guarded cluster), but a `confirm=false` is accepted because nothing
        // is written.
        let preview = ctx.brokers().dry_run_reset(&id, &group, &req).await?;
        return Ok(Json(preview).into_response());
    }

    guard(&row, req.confirm)?;
    let detail = ctx.brokers().reset_group_offsets(&id, &group, &req).await?;
    ctx.brokers()
        .audit_write(&id, &user.id, "reset_group_offsets", serde_json::json!({ "group": group }))
        .await;
    Ok(Json(detail).into_response())
}

async fn schema_subjects<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().schema_subjects(&id).await?).into_response())
}

// ---- schema version history + compat -------------------------------------

async fn schema_subject_versions<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, subject)): Path<(Id, String)>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.brokers().schema_subject_versions(&id, &subject).await?).into_response())
}

async fn schema_subject_version_detail<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, subject, version)): Path<(Id, String, String)>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.brokers()
            .schema_subject_version_detail(&id, &subject, &version)
            .await?,
    )
    .into_response())
}

async fn schema_check_compatibility<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, subject)): Path<(Id, String)>,
    Json(req): Json<CompatCheckReq>,
) -> ApiResult<Response> {
    // Compatibility check is a read (calls the registry read endpoint); Viewer is enough.
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.brokers()
            .schema_check_compatibility(&id, &subject, &req)
            .await?,
    )
    .into_response())
}

// ---- DLQ / replay --------------------------------------------------------

async fn replay<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<ReplayReq>,
) -> ApiResult<Response> {
    let row = authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    guard(&row, req.confirm)?;
    let resp = ctx.brokers().replay(&id, &req).await?;
    ctx.brokers()
        .audit_write(
            &id,
            &user.id,
            "replay",
            serde_json::json!({
                "source_topic": req.source_topic,
                "target_topic": req.target_topic,
                "count": resp.count,
            }),
        )
        .await;
    Ok((StatusCode::CREATED, Json(resp)).into_response())
}

// ---- lag alerts ----------------------------------------------------------

async fn list_lag_alerts<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    // Pass an empty current-lags map (breach evaluation requires live group lag;
    // the UI triggers a describe-group call separately when needed).
    let empty: HashMap<String, i64> = HashMap::new();
    Ok(Json(ctx.brokers().list_lag_alerts(&id, &empty).await?).into_response())
}

async fn create_lag_alert<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<NewLagAlertReq>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let alert = ctx.brokers().create_lag_alert(&id, &req).await?;
    Ok((StatusCode::CREATED, Json(alert)).into_response())
}

async fn delete_lag_alert<S: BrokersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, alert_id)): Path<(Id, Id)>,
) -> ApiResult<Response> {
    // Verify the cluster is accessible (authorization) but the alert delete is
    // just a record removal — Editor required to modify cluster operations.
    authorize(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    ctx.brokers().delete_lag_alert(&alert_id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
