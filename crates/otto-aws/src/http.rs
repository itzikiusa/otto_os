//! `/aws/*` REST router (contract: `docs/design/aws-k8s-consoles.md` §2,
//! mirrored in `docs/contracts/api.md` "AWS console").
//!
//! Authorization is the server's policy table (`otto-server/src/policy.rs`,
//! "AWS console" block) — handlers do not re-check feature grants, with one
//! exception: `import-kubeconfig` also creates a Kubernetes cluster row, so it
//! additionally requires `kubernetes:Admin` (checked here via
//! `GrantsRepo::check_global`). Mutations write audit rows.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::domain::{Capability, Feature, Session};
use otto_core::{Error, Id};
use otto_state::{AuditRepo, GrantsRepo, NewAuditEntry};
use serde::{Deserialize, Serialize};

use crate::accounts::{
    AwsAccount, AwsPermissions, AwsService, AwsTestResp, LoginReq, UpsertAwsAccountReq,
};
use crate::discover::{self, DiscoverResp};
use crate::install::{self, AwsStatus, InstallJob};
use crate::{athena, ec2, eks, s3, sqs, AwsCtx};

/// Local problem-details mapper (orphan rule: cannot impl IntoResponse for
/// `otto_core::Error` here). Same table as otto-connections.
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
pub fn api_router<S: AwsCtx>() -> Router<S> {
    Router::new()
        // --- plumbing ---
        .route("/aws/status", get(status::<S>))
        .route("/aws/install", post(install_cli::<S>))
        .route("/aws/discover", get(discover_profiles::<S>))
        .route("/aws/regions", get(regions::<S>))
        // --- accounts ---
        .route(
            "/aws/accounts",
            get(list_accounts::<S>).post(create_account::<S>),
        )
        .route(
            "/aws/accounts/{id}",
            get(get_account::<S>)
                .patch(update_account::<S>)
                .delete(delete_account::<S>),
        )
        .route("/aws/accounts/{id}/test", post(test_account::<S>))
        .route("/aws/accounts/{id}/permissions", get(permissions::<S>))
        .route("/aws/accounts/{id}/login", post(login::<S>))
        // --- S3 (View) ---
        .route("/aws/accounts/{id}/s3/buckets", get(s3_buckets::<S>))
        .route(
            "/aws/accounts/{id}/s3/buckets/{bucket}/objects",
            get(s3_objects::<S>),
        )
        .route(
            "/aws/accounts/{id}/s3/buckets/{bucket}/object",
            get(s3_head::<S>),
        )
        .route(
            "/aws/accounts/{id}/s3/buckets/{bucket}/preview",
            get(s3_preview::<S>),
        )
        .route(
            "/aws/accounts/{id}/s3/buckets/{bucket}/download",
            get(s3_download::<S>),
        )
        // --- SQS ---
        .route("/aws/accounts/{id}/sqs/queues", get(sqs_queues::<S>))
        .route(
            "/aws/accounts/{id}/sqs/queues/attributes",
            get(sqs_attributes::<S>),
        )
        .route("/aws/accounts/{id}/sqs/queues/peek", post(sqs_peek::<S>))
        .route("/aws/accounts/{id}/sqs/queues/send", post(sqs_send::<S>))
        .route(
            "/aws/accounts/{id}/sqs/queues/delete-message",
            post(sqs_delete_message::<S>),
        )
        .route("/aws/accounts/{id}/sqs/queues/purge", post(sqs_purge::<S>))
        .route(
            "/aws/accounts/{id}/sqs/queues/redrive",
            post(sqs_redrive::<S>),
        )
        // --- EC2 ---
        .route("/aws/accounts/{id}/ec2/instances", get(ec2_instances::<S>))
        .route(
            "/aws/accounts/{id}/ec2/instances/{instance_id}",
            get(ec2_instance::<S>),
        )
        .route(
            "/aws/accounts/{id}/ec2/instances/{instance_id}/start",
            post(ec2_start::<S>),
        )
        .route(
            "/aws/accounts/{id}/ec2/instances/{instance_id}/stop",
            post(ec2_stop::<S>),
        )
        .route(
            "/aws/accounts/{id}/ec2/instances/{instance_id}/reboot",
            post(ec2_reboot::<S>),
        )
        // --- Athena ---
        .route(
            "/aws/accounts/{id}/athena/workgroups",
            get(athena_workgroups::<S>),
        )
        .route(
            "/aws/accounts/{id}/athena/databases",
            get(athena_databases::<S>),
        )
        .route("/aws/accounts/{id}/athena/tables", get(athena_tables::<S>))
        .route(
            "/aws/accounts/{id}/athena/history",
            get(athena_history::<S>),
        )
        .route("/aws/accounts/{id}/athena/query", post(athena_query::<S>))
        .route(
            "/aws/accounts/{id}/athena/query/{qid}",
            get(athena_status::<S>),
        )
        .route(
            "/aws/accounts/{id}/athena/query/{qid}/cancel",
            post(athena_cancel::<S>),
        )
        // --- EKS ---
        .route("/aws/accounts/{id}/eks/clusters", get(eks_clusters::<S>))
        .route(
            "/aws/accounts/{id}/eks/clusters/{name}",
            get(eks_cluster::<S>),
        )
        .route(
            "/aws/accounts/{id}/eks/clusters/{name}/import-kubeconfig",
            post(eks_import_kubeconfig::<S>),
        )
}

/// Best-effort audit row (failure is logged, never propagated).
async fn audit<S: AwsCtx>(
    ctx: &S,
    user: &Id,
    action: &str,
    target: String,
    detail: serde_json::Value,
) {
    if let Err(e) = AuditRepo::new(ctx.pool())
        .insert(NewAuditEntry {
            user_id: Some(user.clone()),
            action: action.to_string(),
            target: Some(target),
            detail: Some(detail),
            ip: None,
        })
        .await
    {
        tracing::warn!(action, "aws audit insert failed: {e}");
    }
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

/// GET /aws/status — Aws:View
async fn status<S: AwsCtx>(State(ctx): State<S>) -> Json<AwsStatus> {
    Json(install::status(ctx.data_dir()).await)
}

/// POST /aws/install — Aws:Admin. Idempotent while a job is running.
async fn install_cli<S: AwsCtx>(State(ctx): State<S>) -> (StatusCode, Json<InstallJob>) {
    let job = install::installer().start(ctx.data_dir().to_path_buf(), ctx.events().clone());
    (StatusCode::ACCEPTED, Json(job))
}

/// GET /aws/discover — Aws:View. Names + metadata only, never key values.
async fn discover_profiles<S: AwsCtx>(State(_ctx): State<S>) -> Json<DiscoverResp> {
    Json(DiscoverResp {
        profiles: discover::discover(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Region {
    pub code: &'static str,
    pub name: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegionsResp {
    pub regions: Vec<Region>,
}

/// Static commercial-partition region list (plus GovCloud).
pub const REGIONS: &[(&str, &str)] = &[
    ("us-east-1", "US East (N. Virginia)"),
    ("us-east-2", "US East (Ohio)"),
    ("us-west-1", "US West (N. California)"),
    ("us-west-2", "US West (Oregon)"),
    ("af-south-1", "Africa (Cape Town)"),
    ("ap-east-1", "Asia Pacific (Hong Kong)"),
    ("ap-south-1", "Asia Pacific (Mumbai)"),
    ("ap-south-2", "Asia Pacific (Hyderabad)"),
    ("ap-southeast-1", "Asia Pacific (Singapore)"),
    ("ap-southeast-2", "Asia Pacific (Sydney)"),
    ("ap-southeast-3", "Asia Pacific (Jakarta)"),
    ("ap-southeast-4", "Asia Pacific (Melbourne)"),
    ("ap-southeast-5", "Asia Pacific (Malaysia)"),
    ("ap-northeast-1", "Asia Pacific (Tokyo)"),
    ("ap-northeast-2", "Asia Pacific (Seoul)"),
    ("ap-northeast-3", "Asia Pacific (Osaka)"),
    ("ca-central-1", "Canada (Central)"),
    ("ca-west-1", "Canada West (Calgary)"),
    ("eu-central-1", "Europe (Frankfurt)"),
    ("eu-central-2", "Europe (Zurich)"),
    ("eu-west-1", "Europe (Ireland)"),
    ("eu-west-2", "Europe (London)"),
    ("eu-west-3", "Europe (Paris)"),
    ("eu-north-1", "Europe (Stockholm)"),
    ("eu-south-1", "Europe (Milan)"),
    ("eu-south-2", "Europe (Spain)"),
    ("il-central-1", "Israel (Tel Aviv)"),
    ("me-south-1", "Middle East (Bahrain)"),
    ("me-central-1", "Middle East (UAE)"),
    ("sa-east-1", "South America (São Paulo)"),
    ("us-gov-east-1", "AWS GovCloud (US-East)"),
    ("us-gov-west-1", "AWS GovCloud (US-West)"),
];

/// GET /aws/regions — Aws:View
async fn regions<S: AwsCtx>(State(_ctx): State<S>) -> Json<RegionsResp> {
    Json(RegionsResp {
        regions: REGIONS
            .iter()
            .map(|(code, name)| Region { code, name })
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

/// GET /aws/accounts — Aws:View
async fn list_accounts<S: AwsCtx>(State(ctx): State<S>) -> ApiResult<Json<Vec<AwsAccount>>> {
    Ok(Json(AwsService::from_ctx(&ctx).list().await?))
}

/// POST /aws/accounts — Aws:Admin
async fn create_account<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<UpsertAwsAccountReq>,
) -> ApiResult<(StatusCode, Json<AwsAccount>)> {
    let a = AwsService::from_ctx(&ctx).create(&user.id, req).await?;
    Ok((StatusCode::CREATED, Json(a)))
}

/// GET /aws/accounts/{id} — Aws:View
async fn get_account<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
) -> ApiResult<Json<AwsAccount>> {
    Ok(Json(AwsService::from_ctx(&ctx).get(&id).await?))
}

/// PATCH /aws/accounts/{id} — Aws:Admin
async fn update_account<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Json(req): Json<UpsertAwsAccountReq>,
) -> ApiResult<Json<AwsAccount>> {
    Ok(Json(AwsService::from_ctx(&ctx).update(&id, req).await?))
}

/// DELETE /aws/accounts/{id} — Aws:Admin → 204
async fn delete_account<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    AwsService::from_ctx(&ctx).delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /aws/accounts/{id}/test — Aws:View
async fn test_account<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
) -> ApiResult<Json<AwsTestResp>> {
    Ok(Json(AwsService::from_ctx(&ctx).test(&id).await?))
}

#[derive(Debug, Deserialize, Default)]
struct RefreshQuery {
    refresh: Option<bool>,
}

/// GET /aws/accounts/{id}/permissions?refresh= — Aws:View
async fn permissions<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<RefreshQuery>,
) -> ApiResult<Json<AwsPermissions>> {
    Ok(Json(
        AwsService::from_ctx(&ctx)
            .permissions(&id, q.refresh.unwrap_or(false))
            .await?,
    ))
}

/// POST /aws/accounts/{id}/login — Aws:Edit → Session (`aws sso login` PTY)
async fn login<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<LoginReq>,
) -> ApiResult<Json<Session>> {
    Ok(Json(
        AwsService::from_ctx(&ctx)
            .login(&id, &req.workspace_id, &user.id)
            .await?,
    ))
}

// ---------------------------------------------------------------------------
// S3
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct RegionQ {
    region: Option<String>,
}

/// GET /aws/accounts/{id}/s3/buckets — AwsS3:View
async fn s3_buckets<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<RegionQ>,
) -> ApiResult<Json<s3::BucketsResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(s3::list_buckets(&svc, &a, q.region.as_deref()).await?))
}

/// GET /aws/accounts/{id}/s3/buckets/{bucket}/objects?prefix=&token=&max= — AwsS3:View
async fn s3_objects<S: AwsCtx>(
    State(ctx): State<S>,
    Path((id, bucket)): Path<(Id, String)>,
    Query(q): Query<s3::ObjectsQuery>,
) -> ApiResult<Json<s3::ObjectsResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(s3::list_objects(&svc, &a, &bucket, &q).await?))
}

/// GET /aws/accounts/{id}/s3/buckets/{bucket}/object?key= — AwsS3:View
async fn s3_head<S: AwsCtx>(
    State(ctx): State<S>,
    Path((id, bucket)): Path<(Id, String)>,
    Query(q): Query<s3::KeyQuery>,
) -> ApiResult<Json<s3::HeadResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(
        s3::head_object(&svc, &a, &bucket, &q.key, q.region.as_deref()).await?,
    ))
}

/// GET /aws/accounts/{id}/s3/buckets/{bucket}/preview?key=&max_bytes= — AwsS3:View
async fn s3_preview<S: AwsCtx>(
    State(ctx): State<S>,
    Path((id, bucket)): Path<(Id, String)>,
    Query(q): Query<s3::KeyQuery>,
) -> ApiResult<Json<s3::PreviewResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(
        s3::preview(&svc, &a, &bucket, &q.key, q.max_bytes, q.region.as_deref()).await?,
    ))
}

/// Filename for `Content-Disposition` — the key's last segment, quotes and
/// control chars stripped.
fn attachment_name(key: &str) -> String {
    let base = key
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("download");
    let clean: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .collect();
    if clean.is_empty() {
        "download".into()
    } else {
        clean
    }
}

/// GET /aws/accounts/{id}/s3/buckets/{bucket}/download?key= — AwsS3:View.
/// Streams `aws s3 cp s3://… -`; the child dies with the response body.
async fn s3_download<S: AwsCtx>(
    State(ctx): State<S>,
    Path((id, bucket)): Path<(Id, String)>,
    Query(q): Query<s3::KeyQuery>,
) -> ApiResult<Response> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    let dl = s3::download(&svc, &a, &bucket, &q.key, q.region.as_deref()).await?;
    let ct = dl
        .head
        .content_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    let disposition = format!("attachment; filename=\"{}\"", attachment_name(&q.key));
    let mut resp = Response::new(dl.body);
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&ct).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition).unwrap_or(HeaderValue::from_static("attachment")),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(dl.head.size));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(resp)
}

// ---------------------------------------------------------------------------
// SQS
// ---------------------------------------------------------------------------

/// GET /aws/accounts/{id}/sqs/queues?prefix= — AwsSqs:View
async fn sqs_queues<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<sqs::QueuesQuery>,
) -> ApiResult<Json<sqs::QueuesResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(sqs::list_queues(&svc, &a, &q).await?))
}

/// GET /aws/accounts/{id}/sqs/queues/attributes?url= — AwsSqs:View
async fn sqs_attributes<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<sqs::UrlQuery>,
) -> ApiResult<Json<sqs::AttributesResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(
        sqs::attributes(&svc, &a, &q.url, q.region.as_deref()).await?,
    ))
}

/// POST /aws/accounts/{id}/sqs/queues/peek — AwsSqs:View (non-mutating POST)
async fn sqs_peek<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(rq): Query<sqs::RegionQuery>,
    Json(req): Json<sqs::PeekReq>,
) -> ApiResult<Json<sqs::PeekResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(sqs::peek(&svc, &a, &req, rq.region.as_deref()).await?))
}

/// POST /aws/accounts/{id}/sqs/queues/send — AwsSqs:Edit (audited)
async fn sqs_send<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(rq): Query<sqs::RegionQuery>,
    Json(req): Json<sqs::SendReq>,
) -> ApiResult<Json<sqs::SendResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    let resp = sqs::send(&svc, &a, &req, rq.region.as_deref()).await?;
    audit(
        &ctx,
        &user.id,
        "aws.sqs.send",
        req.url.clone(),
        serde_json::json!({ "account_id": id, "message_id": resp.message_id, "bytes": req.body.len() }),
    )
    .await;
    Ok(Json(resp))
}

/// POST /aws/accounts/{id}/sqs/queues/delete-message — AwsSqs:Edit → 204
async fn sqs_delete_message<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(rq): Query<sqs::RegionQuery>,
    Json(req): Json<sqs::DeleteMessageReq>,
) -> ApiResult<StatusCode> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    sqs::delete_message(&svc, &a, &req, rq.region.as_deref()).await?;
    audit(
        &ctx,
        &user.id,
        "aws.sqs.delete_message",
        req.url.clone(),
        serde_json::json!({ "account_id": id }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /aws/accounts/{id}/sqs/queues/purge — AwsSqs:Edit (typed confirm) → 204
async fn sqs_purge<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(rq): Query<sqs::RegionQuery>,
    Json(req): Json<sqs::PurgeReq>,
) -> ApiResult<StatusCode> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    sqs::purge(&svc, &a, &req, rq.region.as_deref()).await?;
    audit(
        &ctx,
        &user.id,
        "aws.sqs.purge",
        req.url.clone(),
        serde_json::json!({ "account_id": id }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /aws/accounts/{id}/sqs/queues/redrive — AwsSqs:Edit
async fn sqs_redrive<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(rq): Query<sqs::RegionQuery>,
    Json(req): Json<sqs::RedriveReq>,
) -> ApiResult<Json<sqs::RedriveResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    let resp = sqs::redrive(&svc, &a, &req, rq.region.as_deref()).await?;
    audit(
        &ctx,
        &user.id,
        "aws.sqs.redrive",
        req.source_arn.clone(),
        serde_json::json!({ "account_id": id, "destination_arn": req.destination_arn, "task_handle": resp.task_handle }),
    )
    .await;
    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// EC2
// ---------------------------------------------------------------------------

/// GET /aws/accounts/{id}/ec2/instances?region=&state=&q= — AwsEc2:View
async fn ec2_instances<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<ec2::InstancesQuery>,
) -> ApiResult<Json<ec2::InstancesResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(ec2::list_instances(&svc, &a, &q).await?))
}

/// GET /aws/accounts/{id}/ec2/instances/{instance_id}?region= — AwsEc2:View
async fn ec2_instance<S: AwsCtx>(
    State(ctx): State<S>,
    Path((id, instance_id)): Path<(Id, String)>,
    Query(q): Query<ec2::RegionQuery>,
) -> ApiResult<Json<ec2::InstanceDetail>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(
        ec2::describe_instance(&svc, &a, &instance_id, q.region.as_deref()).await?,
    ))
}

async fn ec2_power<S: AwsCtx>(
    ctx: &S,
    user: &Id,
    id: &Id,
    instance_id: &str,
    action: ec2::PowerAction,
    region: Option<&str>,
    body: Option<Json<ec2::ConfirmReq>>,
) -> ApiResult<Json<ec2::StateChangeResp>> {
    let svc = AwsService::from_ctx(ctx);
    let a = svc.get_row(id).await?;
    let confirm = body.and_then(|Json(b)| b.confirm_id);
    let resp = ec2::power(&svc, &a, instance_id, action, confirm.as_deref(), region).await?;
    audit(
        ctx,
        user,
        &format!("aws.ec2.{}", action.as_str()),
        instance_id.to_string(),
        serde_json::json!({ "account_id": id, "region": region, "previous_state": resp.previous_state, "current_state": resp.current_state }),
    )
    .await;
    Ok(Json(resp))
}

/// POST /aws/accounts/{id}/ec2/instances/{instance_id}/start?region= — AwsEc2:Edit
async fn ec2_start<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, instance_id)): Path<(Id, String)>,
    Query(q): Query<ec2::RegionQuery>,
    body: Option<Json<ec2::ConfirmReq>>,
) -> ApiResult<Json<ec2::StateChangeResp>> {
    ec2_power(
        &ctx,
        &user.id,
        &id,
        &instance_id,
        ec2::PowerAction::Start,
        q.region.as_deref(),
        body,
    )
    .await
}

/// POST /aws/accounts/{id}/ec2/instances/{instance_id}/stop?region= — AwsEc2:Edit (typed confirm)
async fn ec2_stop<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, instance_id)): Path<(Id, String)>,
    Query(q): Query<ec2::RegionQuery>,
    body: Option<Json<ec2::ConfirmReq>>,
) -> ApiResult<Json<ec2::StateChangeResp>> {
    ec2_power(
        &ctx,
        &user.id,
        &id,
        &instance_id,
        ec2::PowerAction::Stop,
        q.region.as_deref(),
        body,
    )
    .await
}

/// POST /aws/accounts/{id}/ec2/instances/{instance_id}/reboot?region= — AwsEc2:Edit (typed confirm)
async fn ec2_reboot<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, instance_id)): Path<(Id, String)>,
    Query(q): Query<ec2::RegionQuery>,
    body: Option<Json<ec2::ConfirmReq>>,
) -> ApiResult<Json<ec2::StateChangeResp>> {
    ec2_power(
        &ctx,
        &user.id,
        &id,
        &instance_id,
        ec2::PowerAction::Reboot,
        q.region.as_deref(),
        body,
    )
    .await
}

// ---------------------------------------------------------------------------
// Athena
// ---------------------------------------------------------------------------

/// GET /aws/accounts/{id}/athena/workgroups — AwsAthena:View
async fn athena_workgroups<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<athena::RegionQuery>,
) -> ApiResult<Json<athena::WorkgroupsResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(
        athena::workgroups(&svc, &a, q.region.as_deref()).await?,
    ))
}

/// GET /aws/accounts/{id}/athena/databases?catalog= — AwsAthena:View
async fn athena_databases<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<athena::CatalogQuery>,
) -> ApiResult<Json<athena::DatabasesResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(athena::databases(&svc, &a, &q).await?))
}

/// GET /aws/accounts/{id}/athena/tables?database=&catalog= — AwsAthena:View
async fn athena_tables<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<athena::CatalogQuery>,
) -> ApiResult<Json<athena::TablesResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(athena::tables(&svc, &a, &q).await?))
}

/// GET /aws/accounts/{id}/athena/history?workgroup=&max= — AwsAthena:View
async fn athena_history<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<athena::HistoryQuery>,
) -> ApiResult<Json<athena::HistoryResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(athena::history(&svc, &a, &q).await?))
}

/// POST /aws/accounts/{id}/athena/query — AwsAthena:Edit (audited)
async fn athena_query<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(rq): Query<athena::RegionQuery>,
    Json(req): Json<athena::QueryReq>,
) -> ApiResult<Json<athena::QueryStartedResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    let resp = athena::start_query(&svc, &a, &req, rq.region.as_deref()).await?;
    audit(
        &ctx,
        &user.id,
        "aws.athena.execute",
        resp.query_execution_id.clone(),
        serde_json::json!({
            "account_id": id,
            "database": req.database,
            "workgroup": req.workgroup,
            "sql": req.sql.chars().take(2000).collect::<String>(),
        }),
    )
    .await;
    Ok(Json(resp))
}

/// GET /aws/accounts/{id}/athena/query/{qid}?token=&max= — AwsAthena:View
async fn athena_status<S: AwsCtx>(
    State(ctx): State<S>,
    Path((id, qid)): Path<(Id, String)>,
    Query(q): Query<athena::StatusQuery>,
) -> ApiResult<Json<athena::AthenaQueryStatus>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(athena::status(&svc, &a, &qid, &q).await?))
}

/// POST /aws/accounts/{id}/athena/query/{qid}/cancel — AwsAthena:View → 204
async fn athena_cancel<S: AwsCtx>(
    State(ctx): State<S>,
    Path((id, qid)): Path<(Id, String)>,
    Query(q): Query<athena::RegionQuery>,
) -> ApiResult<StatusCode> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    athena::cancel(&svc, &a, &qid, q.region.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// EKS
// ---------------------------------------------------------------------------

/// GET /aws/accounts/{id}/eks/clusters?region= — AwsEks:View
async fn eks_clusters<S: AwsCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<eks::RegionQuery>,
) -> ApiResult<Json<eks::ClustersResp>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(
        eks::list_clusters(&svc, &a, q.region.as_deref()).await?,
    ))
}

/// GET /aws/accounts/{id}/eks/clusters/{name}?region= — AwsEks:View
async fn eks_cluster<S: AwsCtx>(
    State(ctx): State<S>,
    Path((id, name)): Path<(Id, String)>,
    Query(q): Query<eks::RegionQuery>,
) -> ApiResult<Json<eks::ClusterDetail>> {
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    Ok(Json(
        eks::describe_cluster(&svc, &a, &name, q.region.as_deref()).await?,
    ))
}

/// POST /aws/accounts/{id}/eks/clusters/{name}/import-kubeconfig?region= —
/// AwsEks:Edit **and** Kubernetes:Admin (creates a cluster row) → K8sCluster (201)
async fn eks_import_kubeconfig<S: AwsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, name)): Path<(Id, String)>,
    Query(q): Query<eks::RegionQuery>,
    body: Option<Json<eks::ImportReq>>,
) -> ApiResult<(StatusCode, Json<eks::K8sCluster>)> {
    GrantsRepo::new(ctx.pool())
        .check_global(
            &user,
            Feature::Kubernetes,
            Capability::Admin,
            "importing an EKS kubeconfig creates a Kubernetes cluster entry — requires kubernetes:Admin",
        )
        .await?;
    let svc = AwsService::from_ctx(&ctx);
    let a = svc.get_row(&id).await?;
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let pool = ctx.pool();
    let cluster =
        eks::import_kubeconfig(&svc, &pool, &a, &name, &req, q.region.as_deref(), &user.id).await?;
    audit(
        &ctx,
        &user.id,
        "aws.eks.import_kubeconfig",
        name.clone(),
        serde_json::json!({ "account_id": id, "cluster_id": cluster.id, "kubeconfig_path": cluster.kubeconfig_path }),
    )
    .await;
    let _ = ctx
        .events()
        .send(otto_core::event::Event::K8sClusterUpdated {
            cluster_id: cluster.id.clone(),
            deleted: false,
        });
    Ok((StatusCode::CREATED, Json(cluster)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_name_is_last_segment_and_sanitized() {
        assert_eq!(attachment_name("logs/2024/app.log"), "app.log");
        assert_eq!(attachment_name("weird\"name\n.txt"), "weirdname.txt");
        assert_eq!(attachment_name("dir/"), "dir");
        assert_eq!(attachment_name("\"\""), "download");
    }

    #[test]
    fn region_list_is_static_and_plentiful() {
        assert!(REGIONS.len() >= 30);
        assert!(REGIONS.iter().any(|(c, _)| *c == "eu-west-1"));
        // Duplicate codes would confuse a select box.
        let mut codes: Vec<&str> = REGIONS.iter().map(|(c, _)| *c).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), REGIONS.len());
    }
}
