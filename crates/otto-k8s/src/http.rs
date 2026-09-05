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
use crate::logs::{self, LogTarget, LogsQuery, SelectorLogsQuery};
use crate::resources::{self, Kind};
use crate::sessions::{self, ExecReq, K9sReq};
use crate::K8sCtx;

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
        .merge(crate::monitor::http::routes::<S>())
}

/// Best-effort audit row (failure is logged, never propagated).
pub(crate) async fn audit<S: K8sCtx>(ctx: &S, user: &User, action: &str, target: &Id, detail: Value) {
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
        result.message = if result.ok { "Connection succeeded" } else { "Connection failed" }.into();
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
    let mut ns = Vec::new();
    for candidate in listed {
        for op in [
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
        ] {
            if crate::access::allowed(&ctx.pool(), &user, &id, op, Some(&candidate.name)).await? {
                ns.push(candidate);
                break;
            }
        }
    }
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
    let svc = Clusters::new(&ctx);
    let c = svc.get(&id).await?;
    let caps = svc.cached_capabilities(&c).await;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    let (items, has_metrics) = resources::list(
        &k,
        kind,
        q.ns.as_deref(),
        q.label.as_deref(),
        q.q.as_deref(),
        caps.metrics_server
            && crate::access::allowed(&ctx.pool(), &user, &id, "metrics", q.ns.as_deref()).await?,
    )
    .await?;
    let _ = svc.repo().touch(&c.id).await;
    Ok(Json(
        json!({ "kind": kind.as_str(), "items": items, "has_metrics": has_metrics }),
    ))
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
    Query(q): Query<NsQuery>,
) -> ApiResult<Json<Value>> {
    crate::access::check(&ctx.pool(), &user, &id, "metrics", q.ns.as_deref()).await?;
    let c = Clusters::new(&ctx).get(&id).await?;
    let k = clusters::kubectl_for(&ctx, &c).await?;
    let (pods, available) = resources::metrics(&k, q.ns.as_deref()).await?;
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
    let _ = svc.repo().touch(&c.id).await;
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
    let _ = svc.repo().touch(&c.id).await;
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
    let _ = svc.repo().touch(&c.id).await;
    // Audit both outcomes: a denied/failed mutation attempt is as interesting
    // as a successful one. Params are logged verbatim (they carry no secrets:
    // replicas / revision / confirm_name / flags).
    let (ok, err) = match &result {
        Ok(r) => (r.ok, None),
        Err(e) => (false, Some(e.to_string())),
    };
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
