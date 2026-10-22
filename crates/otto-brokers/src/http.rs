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
use otto_core::domain::{Feature, User, WorkspaceRole};
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

    /// SQLite pool used to resolve per-user feature grants for **global**
    /// (workspace-less) clusters. `None` — the default, for check-only test
    /// contexts with no state DB — keeps that branch root-only.
    fn pool(&self) -> Option<otto_state::DbPool> {
        None
    }
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
}

// ---- authorization helpers ------------------------------------------------

/// Fetch the cluster row and enforce the workspace role; for **global**
/// (workspace-less) clusters the caller's `Database` grant answers instead.
///
/// Same shape as the connections / DB Explorer gates: a row with no workspace
/// has no workspace role to check, and falling back to root-only made every
/// global cluster unusable for every other account. Brokers have no dedicated
/// Feature key — `policy.rs` gates them on `Database` (read=View, mutate=Edit),
/// so that's the axis here too. A context without a pool fails closed.
async fn authorize<S: BrokersCtx>(
    ctx: &S,
    user: &User,
    id: &Id,
    min: WorkspaceRole,
) -> Result<BrokerClusterRow, Error> {
    let row = ctx.brokers().get_row(id).await?;
    match &row.workspace_id {
        Some(ws) => ctx.roles().check(user, ws, min).await?,
        None => match ctx.pool() {
            Some(pool) => {
                let need = otto_state::capability_for_role(min);
                otto_state::GrantsRepo::new(pool)
                    .check_global(
                        user,
                        Feature::Database,
                        need,
                        &format!(
                            "global broker clusters need the Database '{}' grant",
                            need.as_str()
                        ),
                    )
                    .await?;
            }
            None if user.is_root => {}
            None => {
                return Err(Error::Forbidden(
                    "global broker clusters are managed by root".into(),
                ));
            }
        },
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
        .audit_write(
            &id,
            &user.id,
            "delete_topic",
            serde_json::json!({ "topic": topic }),
        )
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
    Json(body): Json<ConsumeHttpReq>,
) -> ApiResult<Response> {
    authorize(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.brokers()
            .consume_with(&id, &topic, &body.req, &body.opts)
            .await?,
    )
    .into_response())
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
        .audit_write(
            &id,
            &user.id,
            "produce",
            serde_json::json!({ "topic": topic }),
        )
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
    // The service audits every committed attempt (success or failure) and
    // refuses (409) a group that still has live members.
    let detail = ctx
        .brokers()
        .reset_group_offsets(&id, &group, &req, &user.id)
        .await?;
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
    // Own the publication even if the client disconnects (concurrency-async-5-01,
    // the `state_archive` convention): hyper drops this handler's future on
    // client EOF, which used to stop the produce loop mid-way, leave the row
    // `running` and skip the closing audit entry. The task finishes the loop,
    // records the final status and writes the audit entry either way.
    let svc = Arc::clone(ctx.brokers());
    let user_id = user.id.clone();
    let task = tokio::spawn(async move {
        let resp = svc.replay(&id, &req, &user_id).await?;
        svc.audit_write(
            &id,
            &user_id,
            "replay",
            serde_json::json!({
                "source_topic": req.source_topic,
                "target_topic": req.target_topic,
                "count": resp.count,
                "replay_id": resp.replay_id,
                "error": resp.error,
                "evidence_saved": resp.evidence_saved,
            }),
        )
        .await;
        Ok::<_, Error>(resp)
    });
    let resp = task
        .await
        .map_err(|e| Error::Internal(format!("replay task failed: {e}")))??;
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

#[cfg(test)]
mod lag_alert_route_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use futures_util::future::BoxFuture;
    use otto_state::{BrokerClustersRepo, BrokerOpsRepo};
    use tower::ServiceExt;

    struct NoSecrets;
    impl otto_core::secrets::SecretStore for NoSecrets {
        fn get(&self, _: &str) -> otto_core::Result<Option<String>> {
            Ok(None)
        }
        fn put(&self, _: &str, _: &str) -> otto_core::Result<()> {
            Ok(())
        }
        fn delete(&self, _: &str) -> otto_core::Result<()> {
            Ok(())
        }
    }
    struct AllowAll;
    impl RoleChecker for AllowAll {
        fn check<'a>(
            &'a self,
            _u: &'a User,
            _ws: &'a Id,
            _m: WorkspaceRole,
        ) -> BoxFuture<'a, otto_core::Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }
    #[derive(Clone)]
    struct Ctx {
        brokers: Arc<BrokersService>,
        roles: Arc<dyn RoleChecker>,
        pool: Option<otto_state::DbPool>,
    }
    impl BrokersCtx for Ctx {
        fn brokers(&self) -> &Arc<BrokersService> {
            &self.brokers
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn pool(&self) -> Option<otto_state::DbPool> {
            self.pool.clone()
        }
    }

    /// The wiring the daemon uses (`new(..).with_ops(..)`) serves the lag
    /// alerts list: 200 with an empty list, not the "ops repo not
    /// initialised" 500 the unwired service returned.
    #[tokio::test]
    async fn lag_alerts_list_is_200_when_wired_like_the_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("b.db")).await.unwrap();
        let user = otto_core::new_id();
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,disabled,created_at) VALUES(?,?, 'hash','Fixture',1,0,strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
            .bind(&user).bind(&user).execute(&pool).await.unwrap();
        let svc = BrokersService::new(
            BrokerClustersRepo::new(pool.clone()),
            Arc::new(NoSecrets),
            None,
        )
        .with_ops(BrokerOpsRepo::new(pool.clone()));
        let req: UpsertClusterReq = serde_json::from_value(serde_json::json!({
            "name":"c", "bootstrap_servers":"127.0.0.1:1", "security_protocol":"plaintext"
        }))
        .unwrap();
        let cluster = svc.create_cluster(None, user.clone(), req).await.unwrap();
        let ctx = Ctx {
            brokers: Arc::new(svc),
            roles: Arc::new(AllowAll),
            pool: None,
        };
        let root = User {
            id: user,
            username: "root".into(),
            display_name: "Root".into(),
            is_root: true,
            disabled: false,
            created_at: chrono::Utc::now(),
        };
        let app = api_router::<Ctx>()
            .layer(Extension(AuthUser(root)))
            .with_state(ctx);
        let res = app
            .oneshot(
                Request::get(format!("/brokers/clusters/{}/lag-alerts", cluster.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    // ---- test-quality-3-03: the prod / read-only guard and the global grant ----

    /// A state DB with one user; `is_root` picks the account kind.
    async fn fixture(is_root: bool) -> (tempfile::TempDir, otto_state::DbPool, User) {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("b.db")).await.unwrap();
        let id = otto_core::new_id();
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,disabled,created_at) VALUES(?,?, 'hash','Fixture',?,0,strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
            .bind(&id).bind(&id).bind(is_root).execute(&pool).await.unwrap();
        let user = User {
            id,
            username: "u".into(),
            display_name: "U".into(),
            is_root,
            disabled: false,
            created_at: chrono::Utc::now(),
        };
        (dir, pool, user)
    }

    /// A global cluster at an unreachable broker: the guard runs before any
    /// broker call, so no Kafka is needed to see it refuse.
    async fn cluster(svc: &BrokersService, owner: &User, extra: serde_json::Value) -> Id {
        let mut body = serde_json::json!({
            "name": "c", "bootstrap_servers": "127.0.0.1:1", "security_protocol": "plaintext"
        });
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        let req: UpsertClusterReq = serde_json::from_value(body).unwrap();
        svc.create_cluster(None, owner.id.clone(), req)
            .await
            .unwrap()
            .id
    }

    fn app_for(svc: BrokersService, user: User, pool: Option<otto_state::DbPool>) -> Router {
        let ctx = Ctx {
            brokers: Arc::new(svc),
            roles: Arc::new(AllowAll),
            pool,
        };
        api_router::<Ctx>()
            .layer(Extension(AuthUser(user)))
            .with_state(ctx)
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, String) {
        let mut req = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                req = req.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = http_body_util::BodyExt::collect(res.into_body())
            .await
            .unwrap()
            .to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The six mutating routes, each with a body that deserializes. The replay
    /// selector is out of range, so once past the guard it fails validation
    /// (400) before any broker connection.
    fn mutations(
        id: &str,
        confirm: bool,
    ) -> Vec<(&'static str, String, Option<serde_json::Value>)> {
        let q = if confirm { "?confirm=true" } else { "" };
        vec![
            (
                "POST",
                format!("/brokers/clusters/{id}/topics"),
                Some(serde_json::json!({"name": "t", "confirm": confirm})),
            ),
            (
                "DELETE",
                format!("/brokers/clusters/{id}/topics/t{q}"),
                None,
            ),
            (
                "PUT",
                format!("/brokers/clusters/{id}/topics/t/configs"),
                Some(serde_json::json!({"configs": [], "confirm": confirm})),
            ),
            (
                "POST",
                format!("/brokers/clusters/{id}/topics/t/produce"),
                Some(serde_json::json!({"value": "v", "confirm": confirm})),
            ),
            (
                "POST",
                format!("/brokers/clusters/{id}/groups/g/reset"),
                Some(serde_json::json!({"mode": "earliest", "confirm": confirm})),
            ),
            (
                "POST",
                format!("/brokers/clusters/{id}/replay"),
                Some(serde_json::json!({
                    "source_topic": "a", "target_topic": "b",
                    "selector": {"type": "latest", "count": 0}, "confirm": confirm
                })),
            ),
        ]
    }

    #[tokio::test]
    async fn guarded_clusters_refuse_every_mutation_without_confirm() {
        for (extra, why) in [
            (serde_json::json!({"environment": "prod"}), "production"),
            (serde_json::json!({"read_only": true}), "read-only"),
        ] {
            let (_dir, pool, root) = fixture(true).await;
            let svc = BrokersService::new(
                BrokerClustersRepo::new(pool.clone()),
                Arc::new(NoSecrets),
                None,
            );
            let id = cluster(&svc, &root, extra).await;
            let app = app_for(svc, root, None);
            for (method, uri, body) in mutations(&id, false) {
                let (status, text) = call(&app, method, &uri, body).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {text}");
                assert!(text.contains(why), "{method} {uri} names the guard: {text}");
            }
            // The guard honours confirm: replay then fails its own validation.
            let (method, uri, body) = mutations(&id, true).pop().unwrap();
            let (status, text) = call(&app, method, &uri, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "confirmed replay: {text}");
        }
    }

    #[tokio::test]
    async fn a_dev_cluster_and_a_guarded_dry_run_pass_the_guard() {
        let (_dir, pool, root) = fixture(true).await;
        let svc = BrokersService::new(
            BrokerClustersRepo::new(pool.clone()),
            Arc::new(NoSecrets),
            None,
        );
        let dev = cluster(&svc, &root, serde_json::json!({"environment": "dev"})).await;
        let prod = cluster(&svc, &root, serde_json::json!({"environment": "prod"})).await;
        let app = app_for(svc, root, None);
        // Dev: no confirm needed — replay reaches its selector validation.
        let (method, uri, body) = mutations(&dev, false).pop().unwrap();
        let (status, text) = call(&app, method, &uri, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "dev replay: {text}");
        // A dry-run reset on prod writes nothing, so the guard lets it through
        // without confirm. It then talks to the unreachable broker: any
        // non-403 answer, or still waiting on the broker, is past the guard.
        let dry_run = format!("/brokers/clusters/{prod}/groups/g/reset?dry_run=true");
        let fut = call(
            &app,
            "POST",
            &dry_run,
            Some(serde_json::json!({"mode": "earliest"})),
        );
        if let Ok((status, text)) =
            tokio::time::timeout(std::time::Duration::from_secs(3), fut).await
        {
            assert_ne!(status, StatusCode::FORBIDDEN, "dry run refused: {text}");
        }
    }

    /// A global (workspace-less) cluster answers to the caller's Database
    /// grant: View can't delete it, Edit can.
    #[tokio::test]
    async fn global_cluster_mutations_need_the_database_edit_grant() {
        let (_dir, pool, user) = fixture(false).await;
        let svc = BrokersService::new(
            BrokerClustersRepo::new(pool.clone()),
            Arc::new(NoSecrets),
            None,
        );
        let id = cluster(&svc, &user, serde_json::json!({})).await;
        let grants = otto_state::GrantsRepo::new(pool.clone());
        grants
            .set_grants(
                &user.id,
                &[(Feature::Database, otto_core::domain::Capability::View)],
            )
            .await
            .unwrap();
        let app = app_for(svc, user.clone(), Some(pool.clone()));
        let uri = format!("/brokers/clusters/{id}");
        let (status, text) = call(&app, "DELETE", &uri, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
        assert!(text.contains("Database"), "{text}");
        grants
            .set_grants(
                &user.id,
                &[(Feature::Database, otto_core::domain::Capability::Edit)],
            )
            .await
            .unwrap();
        let (status, text) = call(&app, "DELETE", &uri, None).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "{text}");
        assert!(status.is_success(), "{status}: {text}");
    }
}
