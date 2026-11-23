//! Axum REST router for `/containers/*` (nested under `/api/v1` by `otto-server`).

use std::path::Path;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::domain::{Session, WorkspaceRole};
use otto_core::{Error, Result};
use otto_pty::CommandSpec;
use serde::Deserialize;
use serde_json::json;

use crate::client::{validate_id, DockerClient};
use crate::types::{
    ContainerActionResp, ContainerDetail, ContainerExecReq, ContainerLogsResp, ContainerStatsResp,
    ContainersContextsResp, ContainersListResp, ContainersStatusResp, ImageInspectResp,
    ImagesListResp, InstallRuntimeReq, NetworksListResp, PruneResp, PullImageReq, PullImageResp,
    RunContainerReq, RunContainerResp, RuntimeStatusResp, SelectContextReq, StackActionResp,
    SystemPruneReq, SystemPruneResp, VolumesListResp,
};
use crate::ContainersCtx;

/// Default interactive shell probe for `docker exec -it`.
pub const DEFAULT_SHELL: &str = "command -v bash >/dev/null && exec bash || exec sh";

const MAX_EXEC_CMD_ARGS: usize = 128;
const MAX_EXEC_CMD_ARG_LEN: usize = 8192;
const MAX_EXEC_WORKDIR_LEN: usize = 4096;

/// Local problem-details mapper (mirrors `otto-k8s` / `otto-connections`).
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
            message: self.0.public_message(),
        };
        (status, Json(problem)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

#[derive(Debug, Default, Deserialize)]
pub struct ListContainersQuery {
    #[serde(default)]
    pub all: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InspectQuery {
    #[serde(default)]
    pub reveal_secrets: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct StopQuery {
    #[serde(default)]
    pub t: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
pub struct KillQuery {
    #[serde(default)]
    pub signal: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RemoveContainerQuery {
    #[serde(default)]
    pub force: Option<bool>,
    #[serde(default)]
    pub v: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ForceQuery {
    #[serde(default)]
    pub force: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PruneImagesQuery {
    #[serde(default)]
    pub dangling_only: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LogsQuery {
    #[serde(default)]
    pub tail: Option<u32>,
    #[serde(default)]
    pub since: Option<i64>,
    #[serde(default)]
    pub timestamps: Option<bool>,
}

/// Build the `/containers/*` router; `otto-server` nests this under `/api/v1`.
pub fn api_router<S: ContainersCtx>() -> Router<S> {
    Router::new()
        .route("/containers/status", get(status::<S>))
        .route("/containers/runtime", get(get_runtime::<S>))
        .route("/containers/runtime/install", post(install_runtime::<S>))
        .route("/containers/runtime/start", post(start_runtime::<S>))
        .route("/containers/runtime/stop", post(stop_runtime::<S>))
        .route("/containers/runtime/restart", post(restart_runtime::<S>))
        .route("/containers/contexts", get(list_contexts::<S>))
        .route("/containers/contexts/select", post(select_context::<S>))
        .route("/containers", get(list_containers::<S>))
        .route("/containers/run", post(run_container::<S>))
        .route(
            "/containers/{id}",
            get(inspect_container::<S>).delete(remove_container::<S>),
        )
        .route(
            "/containers/{id}/secrets/reveal",
            post(reveal_container_secrets::<S>),
        )
        .route("/containers/{id}/start", post(start_container::<S>))
        .route("/containers/{id}/stop", post(stop_container::<S>))
        .route("/containers/{id}/restart", post(restart_container::<S>))
        .route("/containers/{id}/pause", post(pause_container::<S>))
        .route("/containers/{id}/unpause", post(unpause_container::<S>))
        .route("/containers/{id}/kill", post(kill_container::<S>))
        .route("/containers/{id}/logs", get(container_logs::<S>))
        .route("/containers/{id}/stats", get(container_stats::<S>))
        .route("/containers/{id}/exec", post(exec_container::<S>))
        .route("/containers/stacks/{project}/start", post(start_stack::<S>))
        .route("/containers/stacks/{project}/stop", post(stop_stack::<S>))
        .route(
            "/containers/stacks/{project}/restart",
            post(restart_stack::<S>),
        )
        .route("/containers/images", get(list_images::<S>))
        .route("/containers/images/pull", post(pull_image::<S>))
        .route("/containers/images/prune", post(prune_images::<S>))
        .route(
            "/containers/images/{id}",
            get(inspect_image::<S>).delete(remove_image::<S>),
        )
        .route("/containers/volumes", get(list_volumes::<S>))
        .route("/containers/volumes/prune", post(prune_volumes::<S>))
        .route("/containers/volumes/{name}", delete(remove_volume::<S>))
        .route("/containers/networks", get(list_networks::<S>))
        .route("/containers/networks/prune", post(prune_networks::<S>))
        .route("/containers/networks/{id}", delete(remove_network::<S>))
        .route("/containers/system/prune", post(system_prune::<S>))
}

/// Run a Docker client operation against the cached active endpoint, evicting
/// the discovery cache if an upstream connectivity error occurs.
async fn with_client<S, T, F, Fut>(ctx: &S, op: F) -> ApiResult<Json<T>>
where
    S: ContainersCtx,
    F: FnOnce(DockerClient) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let client = ctx.containers().active_client().await;
    match op(client).await {
        Ok(val) => Ok(Json(val)),
        Err(err) => {
            if matches!(err, Error::Upstream(_)) {
                ctx.containers().invalidate_cache().await;
            }
            Err(err.into())
        }
    }
}

async fn status<S: ContainersCtx>(State(ctx): State<S>) -> Json<ContainersStatusResp> {
    let ep = ctx.containers().active_endpoint().await;
    let cli = ctx.containers().locate_docker_cli().await;
    let client = DockerClient::new(ep.socket_path);
    let resp = client.status(&ep.context_name, ep.engine_kind, cli).await;
    if !resp.available {
        ctx.containers().invalidate_cache().await;
    }
    Json(resp)
}

async fn get_runtime<S: ContainersCtx>(State(ctx): State<S>) -> Json<RuntimeStatusResp> {
    Json(ctx.containers().runtime_status(None).await)
}

async fn install_runtime<S: ContainersCtx>(
    State(ctx): State<S>,
    body: Option<Json<InstallRuntimeReq>>,
) -> ApiResult<Json<RuntimeStatusResp>> {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    Ok(Json(ctx.containers().install_runtime(&req).await?))
}

async fn start_runtime<S: ContainersCtx>(
    State(ctx): State<S>,
) -> ApiResult<Json<RuntimeStatusResp>> {
    Ok(Json(ctx.containers().start_runtime().await?))
}

async fn stop_runtime<S: ContainersCtx>(
    State(ctx): State<S>,
) -> ApiResult<Json<RuntimeStatusResp>> {
    Ok(Json(ctx.containers().stop_runtime().await?))
}

async fn restart_runtime<S: ContainersCtx>(
    State(ctx): State<S>,
) -> ApiResult<Json<RuntimeStatusResp>> {
    Ok(Json(ctx.containers().restart_runtime().await?))
}

async fn list_contexts<S: ContainersCtx>(State(ctx): State<S>) -> Json<ContainersContextsResp> {
    Json(ctx.containers().discover_contexts(false).await)
}

async fn select_context<S: ContainersCtx>(
    State(ctx): State<S>,
    Json(req): Json<SelectContextReq>,
) -> ApiResult<Json<ContainersContextsResp>> {
    Ok(Json(ctx.containers().select_context(&req).await?))
}

async fn list_containers<S: ContainersCtx>(
    State(ctx): State<S>,
    Query(q): Query<ListContainersQuery>,
) -> ApiResult<Json<ContainersListResp>> {
    let all = q.all.unwrap_or(true);
    with_client(&ctx, |c| async move { c.list_containers(all).await }).await
}

async fn run_container<S: ContainersCtx>(
    State(ctx): State<S>,
    Json(req): Json<RunContainerReq>,
) -> ApiResult<Json<RunContainerResp>> {
    // Reject non-human agent credentials: bind-mounting host paths into a new
    // container would bypass Seatbelt confinement.
    if otto_core::auth::request_is_agent() {
        return Err(Error::Forbidden(
            "an agent session's credential cannot create or run containers — a person signed in to Otto must do this"
                .into(),
        )
        .into());
    }
    with_client(&ctx, |c| async move { c.run_container(&req).await }).await
}

async fn inspect_container<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<InspectQuery>,
) -> ApiResult<Json<ContainerDetail>> {
    if q.reveal_secrets.unwrap_or(false) {
        return Err(Error::Forbidden(
            "secret environment variables cannot be revealed via GET /containers/{id}; use POST /containers/{id}/secrets/reveal"
                .into(),
        )
        .into());
    }
    with_client(
        &ctx,
        |c| async move { c.inspect_container(&id, false).await },
    )
    .await
}

async fn reveal_container_secrets<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<ContainerDetail>> {
    if otto_core::auth::request_is_agent() {
        return Err(Error::Forbidden(
            "an agent session's credential cannot reveal container secret environment variables — a person signed in to Otto must do this"
                .into(),
        )
        .into());
    }
    with_client(
        &ctx,
        |c| async move { c.inspect_container(&id, true).await },
    )
    .await
}

async fn remove_container<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<RemoveContainerQuery>,
) -> ApiResult<Json<ContainerActionResp>> {
    let force = q.force.unwrap_or(false);
    let remove_volumes = q.v.unwrap_or(false);
    with_client(&ctx, |c| async move {
        c.remove_container(&id, force, remove_volumes).await
    })
    .await
}

async fn start_container<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<ContainerActionResp>> {
    with_client(&ctx, |c| async move {
        c.container_action(&id, "start", None).await
    })
    .await
}

async fn stop_container<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<StopQuery>,
) -> ApiResult<Json<ContainerActionResp>> {
    let query = format!("t={}", q.t.unwrap_or(10).min(300));
    with_client(&ctx, |c| async move {
        c.container_action(&id, "stop", Some(&query)).await
    })
    .await
}

async fn restart_container<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<StopQuery>,
) -> ApiResult<Json<ContainerActionResp>> {
    let query = format!("t={}", q.t.unwrap_or(10).min(300));
    with_client(&ctx, |c| async move {
        c.container_action(&id, "restart", Some(&query)).await
    })
    .await
}

async fn pause_container<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<ContainerActionResp>> {
    with_client(&ctx, |c| async move {
        c.container_action(&id, "pause", None).await
    })
    .await
}

async fn unpause_container<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<ContainerActionResp>> {
    with_client(&ctx, |c| async move {
        c.container_action(&id, "unpause", None).await
    })
    .await
}

async fn kill_container<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<KillQuery>,
) -> ApiResult<Json<ContainerActionResp>> {
    let signal = q
        .signal
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("SIGKILL");
    if !signal
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        || signal.len() > 24
    {
        return Err(Error::Invalid(format!("invalid signal `{signal}`")).into());
    }
    let query = format!("signal={signal}");
    with_client(&ctx, |c| async move {
        c.container_action(&id, "kill", Some(&query)).await
    })
    .await
}

async fn container_logs<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<LogsQuery>,
) -> ApiResult<Json<ContainerLogsResp>> {
    let tail = q.tail.unwrap_or(200);
    let since = q.since;
    let timestamps = q.timestamps.unwrap_or(false);
    with_client(&ctx, |c| async move {
        c.container_logs(&id, tail, since, timestamps).await
    })
    .await
}

async fn container_stats<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<ContainerStatsResp>> {
    with_client(&ctx, |c| async move { c.container_stats(&id).await }).await
}

/// Build the pure `CommandSpec` for `docker -H unix://<sock> exec -it ...`.
pub fn exec_spec(
    docker_program: &Path,
    socket_path: &Path,
    container_id: &str,
    req: &ContainerExecReq,
) -> Result<CommandSpec> {
    validate_id("container id", container_id)?;
    let mut args = vec![
        "-H".to_string(),
        format!("unix://{}", socket_path.to_string_lossy()),
        "exec".to_string(),
        "-it".to_string(),
    ];
    if let Some(user) = req.user.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if user.starts_with('-') || user.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(Error::Invalid(format!("invalid exec user `{user}`")));
        }
        args.push("--user".into());
        args.push(user.to_string());
    }
    if let Some(wd) = req
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !wd.starts_with('/')
            || wd.len() > MAX_EXEC_WORKDIR_LEN
            || wd.chars().any(|c| c.is_control())
        {
            return Err(Error::Invalid(format!(
                "exec working_dir must be a control-character-free absolute path: `{wd}`"
            )));
        }
        args.push("--workdir".into());
        args.push(wd.to_string());
    }
    args.push(container_id.trim().to_string());
    match req.command.as_ref().filter(|c| !c.is_empty()) {
        Some(cmd) => {
            if cmd.len() > MAX_EXEC_CMD_ARGS {
                return Err(Error::Invalid(format!(
                    "exec command has too many arguments (max {MAX_EXEC_CMD_ARGS})"
                )));
            }
            for arg in cmd {
                if arg.len() > MAX_EXEC_CMD_ARG_LEN || arg.contains('\0') {
                    return Err(Error::Invalid(
                        "exec command argument is too long or contains a NUL byte".into(),
                    ));
                }
            }
            args.extend(cmd.iter().cloned());
        }
        None => args.extend([
            "sh".to_string(),
            "-c".to_string(),
            DEFAULT_SHELL.to_string(),
        ]),
    }
    Ok(CommandSpec {
        program: docker_program.to_string_lossy().into_owned(),
        args,
        cwd: None,
        env: Vec::new(),
    })
}

async fn exec_container<S: ContainersCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<ContainerExecReq>,
) -> ApiResult<Json<Session>> {
    // An agent session credential cannot open an interactive container shell over REST.
    if otto_core::auth::request_is_agent() {
        return Err(Error::Forbidden(
            "an agent session's credential cannot open a container shell — a person signed in to Otto must do this"
                .into(),
        )
        .into());
    }
    // Verify the caller has Editor access to the target workspace before spawning a PTY.
    ctx.roles()
        .check(&user, &req.workspace_id, WorkspaceRole::Editor)
        .await?;
    validate_id("container id", &id)?;
    let ep = ctx.containers().active_endpoint().await;
    let client = DockerClient::new(ep.socket_path.clone());
    // Inspect first so we surface a clear 404/409 before spawning a PTY.
    let detail = client.inspect_container(&id, false).await?;
    if !detail.running {
        return Err(Error::Conflict(format!(
            "container `{}` is not running (state: {})",
            detail.name, detail.state
        ))
        .into());
    }

    let docker_cli = ctx.containers().locate_docker_cli().await.ok_or_else(|| {
        Error::Invalid(
            "docker CLI not found on PATH — install it (e.g. `brew install docker`) to open container terminals"
                .into(),
        )
    })?;

    let spec = exec_spec(&docker_cli, &ep.socket_path, &detail.id, &req)?;
    let title = format!("{} · shell", detail.name);
    let meta = json!({
        "containers": {
            "container_id": detail.id,
            "container_name": detail.name,
            "image": detail.image,
            "context": ep.context_name,
            "mode": "exec",
        }
    });
    let session = ctx
        .spawner()
        .spawn_command(
            &req.workspace_id,
            &user.id,
            "containers",
            spec,
            title,
            Some(meta),
        )
        .await?;
    Ok(Json(session))
}

async fn start_stack<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(project): AxumPath<String>,
) -> ApiResult<Json<StackActionResp>> {
    with_client(
        &ctx,
        |c| async move { c.stack_action(&project, "start").await },
    )
    .await
}

async fn stop_stack<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(project): AxumPath<String>,
) -> ApiResult<Json<StackActionResp>> {
    with_client(
        &ctx,
        |c| async move { c.stack_action(&project, "stop").await },
    )
    .await
}

async fn restart_stack<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(project): AxumPath<String>,
) -> ApiResult<Json<StackActionResp>> {
    with_client(&ctx, |c| async move {
        c.stack_action(&project, "restart").await
    })
    .await
}

async fn list_images<S: ContainersCtx>(State(ctx): State<S>) -> ApiResult<Json<ImagesListResp>> {
    with_client(&ctx, |c| async move { c.list_images().await }).await
}

async fn inspect_image<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<ImageInspectResp>> {
    with_client(&ctx, |c| async move { c.inspect_image(&id).await }).await
}

async fn pull_image<S: ContainersCtx>(
    State(ctx): State<S>,
    Json(req): Json<PullImageReq>,
) -> ApiResult<Json<PullImageResp>> {
    with_client(&ctx, |c| async move { c.pull_image(&req).await }).await
}

async fn remove_image<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<ForceQuery>,
) -> ApiResult<Json<ContainerActionResp>> {
    let force = q.force.unwrap_or(false);
    with_client(&ctx, |c| async move { c.remove_image(&id, force).await }).await
}

async fn prune_images<S: ContainersCtx>(
    State(ctx): State<S>,
    Query(q): Query<PruneImagesQuery>,
) -> ApiResult<Json<PruneResp>> {
    let dangling_only = q.dangling_only.unwrap_or(true);
    with_client(&ctx, |c| async move { c.prune_images(dangling_only).await }).await
}

async fn list_volumes<S: ContainersCtx>(State(ctx): State<S>) -> ApiResult<Json<VolumesListResp>> {
    with_client(&ctx, |c| async move { c.list_volumes().await }).await
}

async fn remove_volume<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<ForceQuery>,
) -> ApiResult<Json<ContainerActionResp>> {
    let force = q.force.unwrap_or(false);
    with_client(&ctx, |c| async move { c.remove_volume(&name, force).await }).await
}

async fn prune_volumes<S: ContainersCtx>(State(ctx): State<S>) -> ApiResult<Json<PruneResp>> {
    with_client(&ctx, |c| async move { c.prune_volumes().await }).await
}

async fn list_networks<S: ContainersCtx>(
    State(ctx): State<S>,
) -> ApiResult<Json<NetworksListResp>> {
    with_client(&ctx, |c| async move { c.list_networks().await }).await
}

async fn remove_network<S: ContainersCtx>(
    State(ctx): State<S>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<ContainerActionResp>> {
    with_client(&ctx, |c| async move { c.remove_network(&id).await }).await
}

async fn prune_networks<S: ContainersCtx>(State(ctx): State<S>) -> ApiResult<Json<PruneResp>> {
    with_client(&ctx, |c| async move { c.prune_networks().await }).await
}

async fn system_prune<S: ContainersCtx>(
    State(ctx): State<S>,
    body: Option<Json<SystemPruneReq>>,
) -> ApiResult<Json<SystemPruneResp>> {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    with_client(&ctx, |c| async move { c.system_prune(&req).await }).await
}
