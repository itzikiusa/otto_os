//! Kubernetes console REST router — every `/k8s/*` route of contract §3.
//!
//! Feature/workspace/token ceilings are enforced by the server. Every cluster
//! handler additionally checks the resource and operation before invoking
//! kubectl, and child lists and streams honor namespace-specific decisions.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::{Error, Id};
use otto_state::{AuditRepo, NewAuditEntry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::actions::{self, K8sActionReq};
use crate::clusters::{
    self, Clusters, ImportK8sClusterReq, PatchK8sClusterReq, UpsertK8sClusterReq,
};
use crate::install::{self, InstallJob, Tool, ToolStatus};
use crate::list_cache;
use crate::logs::{self, LogTarget, LogsQuery, SelectorLogsQuery};
use crate::resources::{self, Kind};
use crate::sessions::{self, ExecReq, K9sReq};
use crate::K8sCtx;
use std::sync::Arc;

/// Local problem-details mapper (orphan rule — mirrors otto-connections).
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

/// `GET /k8s/status` response (contract §3.1).
#[derive(Debug, Serialize)]
pub struct K8sStatus {
    pub kubectl: ToolStatus,
    pub k9s: ToolStatus,
    pub install: InstallJobs,
}

#[derive(Debug, Serialize)]
pub struct InstallJobs {
    pub kubectl: InstallJob,
    pub k9s: InstallJob,
}

#[derive(Debug, Deserialize)]
pub struct InstallReq {
    pub tool: Tool,
}

#[derive(Debug, Default, Deserialize)]
pub struct RefreshQuery {
    #[serde(default)]
    pub refresh: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ResourcesQuery {
    pub kind: String,
    pub ns: Option<String>,
    pub label: Option<String>,
    pub q: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ResourceQuery {
    pub kind: String,
    pub ns: Option<String>,
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct NsQuery {
    pub ns: Option<String>,
}

/// `GET …/metrics?ns=&pod=` — `pod` (with `ns`) narrows to one pod.
#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    pub ns: Option<String>,
    pub pod: Option<String>,
}

/// REST routes; the server nests this under `/api/v1` and supplies the state.
pub fn api_router<S: K8sCtx>() -> Router<S> {
    Router::new()
        .route("/k8s/status", get(status::<S>))
        .route("/k8s/install", post(install_tool::<S>))
        .route("/k8s/discover", get(discover::<S>))
        .route(
            "/k8s/clusters",
            get(list_clusters::<S>).post(create_cluster::<S>),
        )
        .route("/k8s/clusters/import", post(import_cluster::<S>))
        .route(
            "/k8s/clusters/{id}",
            get(get_cluster::<S>)
                .patch(update_cluster::<S>)
                .delete(delete_cluster::<S>),
        )
        .route("/k8s/clusters/{id}/test", post(test_cluster::<S>))
        .route("/k8s/clusters/{id}/capabilities", get(capabilities::<S>))
        .route("/k8s/clusters/{id}/namespaces", get(namespaces::<S>))
        .route("/k8s/clusters/{id}/nodes", get(nodes::<S>))
        .route("/k8s/clusters/{id}/resources", get(list_resources::<S>))
        .route("/k8s/clusters/{id}/resource", get(resource_detail::<S>))
        .route(
            "/k8s/clusters/{id}/pods/{ns}/{name}/containers",
            get(pod_containers::<S>),
        )
        .route(
            "/k8s/clusters/{id}/pods/{ns}/{name}/logs",
            get(pod_logs::<S>),
        )
        .route("/k8s/clusters/{id}/logs", get(selector_logs::<S>))
        .route("/k8s/clusters/{id}/metrics", get(metrics::<S>))
        .route("/k8s/clusters/{id}/exec", post(exec::<S>))
        .route("/k8s/clusters/{id}/k9s", post(k9s::<S>))
        .route("/k8s/clusters/{id}/actions", post(run_action::<S>))
        .route("/k8s/clusters/{id}/pod-http", post(pod_http::<S>))
        .route(
            "/k8s/clusters/{id}/pod-actions",
            get(list_pod_actions::<S>).put(save_pod_action::<S>),
        )
        .route(
            "/k8s/clusters/{id}/pod-actions/{action_id}",
            axum::routing::delete(delete_pod_action::<S>),
        )
        .merge(crate::monitor::http::routes::<S>())
        .merge(crate::monitor::fleet::routes::<S>())
}

/// Best-effort audit row (failure is logged, never propagated).
pub(crate) async fn audit<S: K8sCtx>(
    ctx: &S,
    user: &User,
    action: &str,
    target: &Id,
    detail: Value,
) {
    if let Err(e) = AuditRepo::new(ctx.pool())
        .insert(NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: action.to_string(),
            target: Some(target.clone()),
            detail: Some(detail),
            ip: None,
        })
        .await
    {
        tracing::warn!("k8s audit ({action}): {e}");
    }
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

async fn status<S: K8sCtx>(State(ctx): State<S>) -> Json<K8sStatus> {
    let data_dir = ctx.data_dir();
    let (kubectl, k9s) = tokio::join!(
        install::tool_status(Tool::Kubectl, data_dir),
        install::tool_status(Tool::K9s, data_dir)
    );
    let inst = install::installer();
    Json(K8sStatus {
        kubectl,
        k9s,
        install: InstallJobs {
            kubectl: inst.job(Tool::Kubectl),
            k9s: inst.job(Tool::K9s),
        },
    })
}

async fn install_tool<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<InstallReq>,
) -> (StatusCode, Json<InstallJob>) {
    let job =
        install::installer().start(req.tool, ctx.data_dir().to_path_buf(), ctx.events().clone());
    audit(
        &ctx,
        &user,
        "k8s.install",
        &req.tool.as_str().to_string(),
        json!({"state": job.state}),
    )
    .await;
    (StatusCode::ACCEPTED, Json(job))
}

async fn discover<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<Value>> {
    crate::access::require_setup_authority(&user)?;
    let contexts = clusters::discover(ctx.data_dir()).await?;
    Ok(Json(json!({ "contexts": contexts })))
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

fn redact_configuration(cluster: &mut otto_state::K8sCluster) {
    cluster.kubeconfig_path = None;
    cluster.context_name.clear();
    cluster.default_namespace = None;
    cluster.aws_account_id = None;
    cluster.params = json!({});
    cluster.capabilities = None;
    cluster.created_by = None;
}

async fn list_clusters<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<Vec<otto_state::K8sCluster>>> {
    let mut visible = Vec::new();
    for mut cluster in Clusters::new(&ctx).list().await? {
        if crate::access::allowed(&ctx.pool(), &user, &cluster.id, "discover", None).await? {
            if !crate::access::can_configure(&ctx.pool(), &user, &cluster.id).await? {
                redact_configuration(&mut cluster);
            }
            visible.push(cluster);
        }
    }
    Ok(Json(visible))
}

async fn create_cluster<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<UpsertK8sClusterReq>,
) -> ApiResult<(StatusCode, Json<otto_state::K8sCluster>)> {
    let c = Clusters::new(&ctx).create(&user, req).await?;
    Ok((StatusCode::CREATED, Json(c)))
}

async fn import_cluster<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<ImportK8sClusterReq>,
) -> ApiResult<(StatusCode, Json<otto_state::K8sCluster>)> {
    let c = Clusters::new(&ctx).import(&user, req).await?;
    Ok((StatusCode::CREATED, Json(c)))
}

async fn get_cluster<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<otto_state::K8sCluster>> {
    crate::access::check(&ctx.pool(), &user, &id, "discover", None).await?;
    let mut cluster = Clusters::new(&ctx).get(&id).await?;
    if !crate::access::can_configure(&ctx.pool(), &user, &id).await? {
        redact_configuration(&mut cluster);
    }
    Ok(Json(cluster))
}

async fn update_cluster<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PatchK8sClusterReq>,
) -> ApiResult<Json<otto_state::K8sCluster>> {
    crate::access::check(&ctx.pool(), &user, &id, "configure", None).await?;
    Ok(Json(Clusters::new(&ctx).update(&id, &user, req).await?))
}

async fn delete_cluster<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    crate::access::check(&ctx.pool(), &user, &id, "configure", None).await?;
    Clusters::new(&ctx).delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn test_cluster<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<clusters::K8sTestResp>> {
    crate::access::check(&ctx.pool(), &user, &id, "discover", None).await?;
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    let mut result = svc.test(&c).await?;
    if !crate::access::can_configure(&ctx.pool(), &user, &id).await? {
        result.message = if result.ok {
            "Connection succeeded"
        } else {
            "Connection failed"
        }
        .into();
    }
    Ok(Json(result))
}

async fn capabilities<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<RefreshQuery>,
) -> ApiResult<Json<clusters::K8sCapabilities>> {
    crate::access::check(&ctx.pool(), &user, &id, "discover", None).await?;
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    Ok(Json(
        svc.capabilities(&c, q.refresh.unwrap_or(false)).await?,
    ))
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

async fn namespaces<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Value>> {
    crate::access::check(&ctx.pool(), &user, &id, "discover", None).await?;
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    // Cluster-scope listing is often forbidden (Rancher project users); the
    // persisted `known_namespaces` fill in, and are appended even when the
    // list succeeds but is partial.
    let mut listed = match resources::namespaces(&k).await {
        Ok(l) => l,
        Err(e) if !c.known_namespaces.is_empty() => {
            tracing::debug!("k8s namespaces: list failed ({e}); using known namespaces");
            Vec::new()
        }
        Err(e) => return Err(e.into()),
    };
    for known in &c.known_namespaces {
        if !listed.iter().any(|n| &n.name == known) {
            listed.push(resources::NamespaceRow {
                name: known.clone(),
                status: String::new(),
                age_seconds: 0,
            });
        }
    }
    let names: Vec<String> = listed.iter().map(|n| n.name.clone()).collect();
    let visible = crate::access::namespaces_allowing_any(
        &ctx.pool(),
        &user,
        &id,
        &[
            "workloads_view",
            "resources_view",
            "secrets_view",
            "logs",
            "metrics",
            "exec",
            "apply",
            "scale",
            "restart",
            "delete",
        ],
        &names,
    )
    .await?;
    let ns: Vec<_> = listed
        .into_iter()
        .zip(visible)
        .filter_map(|(row, ok)| ok.then_some(row))
        .collect();
    Ok(Json(json!({ "namespaces": ns })))
}

async fn nodes<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Value>> {
    crate::access::check(&ctx.pool(), &user, &id, "resources_view", None).await?;
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    let caps = svc.cached_capabilities(&c).await;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    let with_metrics = caps.metrics_server
        && crate::access::allowed(&ctx.pool(), &user, &id, "metrics", None).await?;
    let rows = resources::nodes(&k, with_metrics).await?;
    Ok(Json(json!({ "nodes": rows })))
}

async fn list_resources<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<ResourcesQuery>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Response> {
    let kind =
        Kind::parse(&q.kind).ok_or_else(|| Error::Invalid(format!("unknown kind '{}'", q.kind)))?;
    crate::access::check(
        &ctx.pool(),
        &user,
        &id,
        crate::access::read_operation(kind),
        if kind.namespaced() {
            q.ns.as_deref()
        } else {
            None
        },
    )
    .await?;
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    let caps = svc.cached_capabilities(&c).await;
    let with_metrics = caps.metrics_server
        && crate::access::allowed(&ctx.pool(), &user, &id, "metrics", q.ns.as_deref()).await?;
    // 10 s cache + single-flight: concurrent viewers / agents share one
    // kubectl list (perf K2). Access was checked above, per caller.
    let ck = list_cache::key(
        id.as_str(),
        kind.as_str(),
        if kind.namespaced() {
            q.ns.as_deref()
        } else {
            None
        },
        q.label.as_deref(),
        q.q.as_deref(),
        with_metrics,
    );
    let listed = match list_cache::get(&ck) {
        Some(l) => l,
        None => {
            let _flight = crate::monitor::cache::flight(&ck).await;
            match list_cache::get(&ck) {
                Some(l) => l,
                None => {
                    let k = clusters::kubectl_for(&ctx, &c).await?;
                    // The cluster's GET-only list gateway while a console
                    // polls (perf R3); `None` ⇒ kubectl.
                    let gw = crate::list_gateway::get(c.id.as_str(), &k).await;
                    let (items, has_metrics) = resources::list(
                        &k,
                        gw.as_deref(),
                        kind,
                        q.ns.as_deref(),
                        q.label.as_deref(),
                        q.q.as_deref(),
                        with_metrics,
                    )
                    .await?;
                    let kind_s = kind.as_str();
                    let l = tokio::task::spawn_blocking(move || {
                        list_cache::build(kind_s, &items, has_metrics)
                    })
                    .await
                    .map(Arc::new)
                    .map_err(|e| Error::Internal(format!("k8s list task: {e}")))?;
                    list_cache::put(ck, l.clone());
                    l
                }
            }
        }
    };
    list_cache::touch_throttled(svc.repo(), &c.id).await;
    let etag = format!("\"{}\"", listed.version);
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| list_cache::matches(v, &listed.version));
    if unchanged {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::ETAG, etag),
        ],
        listed.body.clone(),
    )
        .into_response())
}

async fn resource_detail<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<ResourceQuery>,
) -> ApiResult<Json<Value>> {
    let kind =
        Kind::parse(&q.kind).ok_or_else(|| Error::Invalid(format!("unknown kind '{}'", q.kind)))?;
    crate::access::check(
        &ctx.pool(),
        &user,
        &id,
        crate::access::read_operation(kind),
        if kind.namespaced() {
            q.ns.as_deref()
        } else {
            None
        },
    )
    .await?;
    if q.name.trim().is_empty() {
        return Err(Error::Invalid("name is required".into()).into());
    }
    // A flag-shaped name (`--context=…`) would re-target kubectl past the grant.
    resources::validate_name("object", q.name.trim())?;
    if kind.namespaced() && q.ns.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(Error::Invalid("ns is required for namespaced kinds".into()).into());
    }
    let c = Clusters::new(&ctx).get(&id).await?;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    Ok(Json(
        resources::detail(&k, kind, q.ns.as_deref(), q.name.trim()).await?,
    ))
}

async fn pod_containers<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, ns, name)): Path<(Id, String, String)>,
) -> ApiResult<Json<Value>> {
    resources::validate_name("namespace", &ns)?;
    resources::validate_name("pod", &name)?;
    let mut allowed = false;
    for operation in ["logs", "exec", "workloads_view"] {
        allowed |= crate::access::allowed(&ctx.pool(), &user, &id, operation, Some(&ns)).await?;
    }
    if !allowed {
        return Err(Error::Forbidden("pod container access is not granted".into()).into());
    }
    let c = Clusters::new(&ctx).get(&id).await?;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    let containers = resources::containers(&k, &ns, &name).await?;
    Ok(Json(json!({ "containers": containers })))
}

async fn pod_logs<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, ns, name)): Path<(Id, String, String)>,
    Query(q): Query<LogsQuery>,
) -> ApiResult<Response> {
    logs::validate_target(&ns, &LogTarget::Pod(&name), &q)?;
    crate::access::check(&ctx.pool(), &user, &id, "logs", Some(&ns)).await?;
    let c = Clusters::new(&ctx).get(&id).await?;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    let headers = [
        (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
        (header::CACHE_CONTROL, "no-cache"),
        (header::HeaderName::from_static("x-accel-buffering"), "no"),
    ];
    if q.follow == Some(true) {
        let body = logs::follow(&k, &ns, &name, &q)?;
        let body = crate::access::guard_body(body, ctx.pool(), user, id, ns.to_string());
        return Ok((headers, body).into_response());
    }
    let text = logs::fetch(&k, &ns, &name, &q).await?;
    Ok((headers, Body::from(text)).into_response())
}

/// Workload-level logs: every pod matching `selector` in `ns`, each line
/// prefixed `[pod/<pod>/<container>] `. Same one-shot / follow semantics as
/// the per-pod route.
async fn selector_logs<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<SelectorLogsQuery>,
) -> ApiResult<Response> {
    crate::access::check(&ctx.pool(), &user, &id, "logs", Some(&q.ns)).await?;
    let ns = q.ns.trim();
    let sel = q.selector.trim();
    if ns.is_empty() || sel.is_empty() {
        return Err(Error::Invalid("ns and selector are required".into()).into());
    }
    let c = Clusters::new(&ctx).get(&id).await?;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    let headers = [
        (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
        (header::CACHE_CONTROL, "no-cache"),
        (header::HeaderName::from_static("x-accel-buffering"), "no"),
    ];
    let lq = q.logs();
    if lq.follow == Some(true) {
        let body = logs::follow_target(&k, ns, LogTarget::Selector(sel), &lq)?;
        let body = crate::access::guard_body(body, ctx.pool(), user, id, ns.to_string());
        return Ok((headers, body).into_response());
    }
    let text = logs::fetch_target(&k, ns, LogTarget::Selector(sel), &lq).await?;
    Ok((headers, Body::from(text)).into_response())
}

async fn metrics<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<MetricsQuery>,
) -> ApiResult<Json<Value>> {
    crate::access::check(&ctx.pool(), &user, &id, "metrics", q.ns.as_deref()).await?;
    let c = Clusters::new(&ctx).get(&id).await?;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    let (pods, available) = resources::metrics(&k, q.ns.as_deref(), q.pod.as_deref()).await?;
    Ok(Json(json!({ "pods": pods, "available": available })))
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

async fn exec<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<ExecReq>,
) -> ApiResult<(StatusCode, Json<otto_core::domain::Session>)> {
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    let session = sessions::exec(&ctx, &user, &c, &req).await?;
    list_cache::touch_throttled(svc.repo(), &c.id).await;
    audit(
        &ctx,
        &user,
        "k8s.exec",
        &c.id,
        json!({
            "cluster": c.name, "context": c.context_name, "ns": req.ns, "pod": req.pod,
            "container": req.container, "command": req.command, "session_id": session.id,
        }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(session)))
}

async fn k9s<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<K9sReq>,
) -> ApiResult<(StatusCode, Json<otto_core::domain::Session>)> {
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    let session = sessions::k9s(&ctx, &user, &c, &req).await?;
    list_cache::touch_throttled(svc.repo(), &c.id).await;
    audit(
        &ctx,
        &user,
        "k8s.k9s",
        &c.id,
        json!({ "cluster": c.name, "context": c.context_name, "ns": req.ns, "session_id": session.id }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(session)))
}

async fn run_action<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<K8sActionReq>,
) -> ApiResult<Json<actions::K8sActionResp>> {
    let action = req.action.trim().to_string();
    if action.is_empty() {
        return Err(Error::Invalid("action is required".into()).into());
    }
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    let result = actions::execute_authorized(&k, &req, &ctx.pool(), &user, &id).await;
    list_cache::touch_throttled(svc.repo(), &c.id).await;
    // Audit both outcomes: a denied/failed mutation attempt is as interesting
    // as a successful one. Params are logged verbatim (they carry no secrets:
    // replicas / revision / confirm_name / flags).
    let (ok, err) = match &result {
        Ok(r) => (r.ok, None),
        Err(e) => (false, Some(e.to_string())),
    };
    // The next list must show the action's effect, not a cached answer.
    list_cache::forget_cluster(c.id.as_str());
    audit(
        &ctx,
        &user,
        &format!("k8s.action.{action}"),
        &c.id,
        json!({
            "cluster": c.name, "context": c.context_name, "environment": c.environment,
            "kind": req.kind, "ns": req.ns, "name": req.name, "params": req.params,
            "ok": ok, "error": err,
        }),
    )
    .await;
    Ok(Json(result?))
}

// ---------------------------------------------------------------------------
// Pod HTTP actions (K-3)
// ---------------------------------------------------------------------------

/// RBAC operation for a pod HTTP call: a GET reads (`workloads_view`); any
/// mutating method can change the app's runtime state, so it needs the same
/// Edit-level grant as running a command in the pod (`exec`).
fn pod_http_operation(mutating: bool) -> &'static str {
    if mutating {
        "exec"
    } else {
        "workloads_view"
    }
}

async fn pod_http<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<crate::pod_http::PodHttpReq>,
) -> ApiResult<Response> {
    use crate::pod_http;
    let v = pod_http::validate(&req)?;
    let mutating = v.mutating();
    crate::access::check(
        &ctx.pool(),
        &user,
        &id,
        pod_http_operation(mutating),
        Some(&v.namespace),
    )
    .await?;
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    let base_detail = json!({
        "cluster": c.name, "context": c.context_name, "environment": c.environment,
        "ns": v.namespace, "pod": req.pod, "workload": req.workload,
        "method": v.method.as_str(), "port": v.port, "path": v.path,
        "headers": pod_http::redact_headers(req.headers.iter().flatten()),
        "body_sha256": pod_http::body_sha256(v.body.as_deref()),
    });
    if pod_http::needs_confirm(
        c.environment,
        mutating,
        req.confirm_name.as_deref(),
        v.target_name(),
    ) {
        let mut detail = base_detail;
        detail["ok"] = json!(false);
        detail["error"] = json!("confirm_required");
        audit(&ctx, &user, "k8s.pod_http", &c.id, detail).await;
        let problem = Problem {
            code: "confirm_required".into(),
            message: format!(
                "confirmation required: {} {} on a prod cluster — set confirm_name to \"{}\"",
                v.method,
                v.path,
                v.target_name()
            ),
        };
        return Ok((StatusCode::CONFLICT, Json(problem)).into_response());
    }
    let k = clusters::kubectl_for(&ctx, &c).await?;
    let result = pod_http::run(&k, c.id.as_str(), &v).await;
    list_cache::touch_throttled(svc.repo(), &c.id).await;
    let mut detail = base_detail;
    match &result {
        Ok(r) => {
            detail["ok"] = json!(true);
            detail["pods"] = json!(r.results.iter().map(|x| &x.pod).collect::<Vec<_>>());
            detail["statuses"] = json!(r
                .results
                .iter()
                .map(|x| json!({"pod": x.pod, "status": x.status, "via": x.via, "error": x.error}))
                .collect::<Vec<_>>());
        }
        Err(e) => {
            detail["ok"] = json!(false);
            detail["error"] = json!(e.to_string());
        }
    }
    audit(&ctx, &user, "k8s.pod_http", &c.id, detail).await;
    Ok(Json(result?).into_response())
}

#[derive(Debug, Default, Deserialize)]
pub struct PodActionsQuery {
    pub namespace: Option<String>,
    pub workload_kind: Option<String>,
    pub workload: Option<String>,
}

/// `PUT …/pod-actions` body.
#[derive(Debug, Deserialize)]
pub struct SavePodActionReq {
    #[serde(default)]
    pub id: Option<String>,
    pub namespace: String,
    pub workload_kind: String,
    pub workload: String,
    pub name: String,
    pub method: String,
    pub port: u32,
    pub path: String,
    #[serde(default)]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub body_template: Option<String>,
}

fn opt_trim(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

async fn list_pod_actions<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<PodActionsQuery>,
) -> ApiResult<Json<Value>> {
    let namespace = opt_trim(q.namespace);
    crate::access::check(&ctx.pool(), &user, &id, "discover", None).await?;
    let filter = otto_state::PodActionFilter {
        namespace: namespace.clone(),
        workload_kind: opt_trim(q.workload_kind)
            .map(|k| crate::pod_http::workload_kind(&k).map(|k| k.as_str().to_string()))
            .transpose()?,
        workload: opt_trim(q.workload),
    };
    let rows = otto_state::K8sPodActionsRepo::new(ctx.pool())
        .list(&id, &filter)
        .await?;
    // Namespace-scoped grants: only actions in namespaces the user may view.
    let mut visible = Vec::with_capacity(rows.len());
    let mut seen: std::collections::HashMap<String, bool> = Default::default();
    for a in rows {
        let ok = match seen.get(&a.namespace) {
            Some(ok) => *ok,
            None => {
                let ok = crate::access::allowed(
                    &ctx.pool(),
                    &user,
                    &id,
                    "workloads_view",
                    Some(&a.namespace),
                )
                .await?;
                seen.insert(a.namespace.clone(), ok);
                ok
            }
        };
        if ok {
            visible.push(a);
        }
    }
    Ok(Json(json!({ "actions": visible })))
}

async fn save_pod_action<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<SavePodActionReq>,
) -> ApiResult<Json<otto_state::PodAction>> {
    use crate::pod_http;
    let namespace = req.namespace.trim().to_string();
    if crate::access::namespace(Some(namespace.as_str()))?.is_none() {
        return Err(Error::Invalid("namespace is required".into()).into());
    }
    let kind = pod_http::workload_kind(&req.workload_kind)?;
    let workload = req.workload.trim().to_string();
    if workload.is_empty() || workload.len() > 253 {
        return Err(Error::Invalid("workload is required".into()).into());
    }
    let name = req.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(Error::Invalid("name is required (at most 120 characters)".into()).into());
    }
    let method = pod_http::parse_method(&req.method)?;
    let port = u16::try_from(req.port)
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| Error::Invalid("port must be 1-65535".into()))?;
    let path = req.path.trim().to_string();
    // Templates are filled at run time: validate with the variables blanked.
    pod_http::validate_path(&path.replace("{{", "").replace("}}", ""))?;
    let headers = req.headers.unwrap_or_default();
    pod_http::validate_headers(&headers)?;
    if headers.keys().any(|k| pod_http::is_secret_header(k)) {
        return Err(Error::Invalid(
            "saved actions cannot store credential headers (Authorization, Cookie…)".into(),
        )
        .into());
    }
    if req
        .body_template
        .as_ref()
        .is_some_and(|b| b.len() > pod_http::MAX_REQUEST_BODY)
    {
        return Err(Error::PayloadTooLarge("body template over 1 MiB".into()).into());
    }
    crate::access::check(&ctx.pool(), &user, &id, "exec", Some(&namespace)).await?;
    let c = Clusters::new(&ctx).get(&id).await?;
    let repo = otto_state::K8sPodActionsRepo::new(ctx.pool());
    let action_id = opt_trim(req.id);
    if let Some(aid) = &action_id {
        // Updating: the existing row's namespace must be editable too.
        if let Ok(existing) = repo.get(&c.id, aid).await {
            crate::access::check(&ctx.pool(), &user, &id, "exec", Some(&existing.namespace))
                .await?;
        }
    }
    let saved = repo
        .upsert(otto_state::UpsertPodAction {
            id: action_id,
            cluster_id: c.id.clone(),
            namespace,
            workload_kind: kind.as_str().to_string(),
            workload,
            name,
            method: method.as_str().to_string(),
            port,
            path,
            headers,
            body_template: req.body_template.filter(|b| !b.is_empty()),
            created_by: Some(user.id.clone()),
        })
        .await?;
    audit(
        &ctx,
        &user,
        "k8s.pod_action.save",
        &c.id,
        json!({"id": saved.id, "ns": saved.namespace, "workload": saved.workload,
               "name": saved.name, "method": saved.method, "path": saved.path}),
    )
    .await;
    Ok(Json(saved))
}

async fn delete_pod_action<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, action_id)): Path<(Id, String)>,
) -> ApiResult<StatusCode> {
    crate::access::check(&ctx.pool(), &user, &id, "discover", None).await?;
    let repo = otto_state::K8sPodActionsRepo::new(ctx.pool());
    let existing = repo.get(&id, &action_id).await?;
    crate::access::check(&ctx.pool(), &user, &id, "exec", Some(&existing.namespace)).await?;
    repo.delete(&id, &action_id).await?;
    audit(
        &ctx,
        &user,
        "k8s.pod_action.delete",
        &id,
        json!({"id": action_id, "ns": existing.namespace, "workload": existing.workload,
               "name": existing.name}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod pod_http_route_tests {
    use super::pod_http_operation;

    #[test]
    fn get_reads_and_mutations_need_exec() {
        assert_eq!(pod_http_operation(false), "workloads_view");
        assert_eq!(pod_http_operation(true), "exec");
    }
}
