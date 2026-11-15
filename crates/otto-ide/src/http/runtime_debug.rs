//! Run Configurations, Process Runner & DAP Debugger HTTP Handlers.

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::Error;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use super::sse_framing::{build_debug_sse_response, build_run_sse_response};
use super::ApiResult;
use crate::dap::DapStartContext;
use crate::types::{
    IdeCreatePythonVenvReq, IdeCreatePythonVenvResp, IdeDebugCommandReq, IdeDebugEvaluateReq,
    IdeDebugEvaluateResp, IdeDebugSessionResp, IdeDebugSessionsResp, IdeDebugVariablesReq,
    IdeDebugVariablesResp, IdeOkResp, IdeRunConfigResp, IdeRunResp, IdeSetBreakpointsReq,
    IdeStartDebugReq, IdeStartRunReq, IdeTargetKind, IdeTargetQuery, IdeTargetSpec,
    IdeUpsertRunConfigReq,
};
use crate::IdeCtx;

pub(super) async fn execute_create_python_venv<S: IdeCtx>(
    ctx: &S,
    user: &User,
    mut req: IdeCreatePythonVenvReq,
) -> ApiResult<IdeCreatePythonVenvResp> {
    if req.target.root.trim().is_empty() {
        if let Some(r) = req.root.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            req.target = IdeTargetSpec::local(r.to_string());
        }
    }
    ctx.ide()
        .environments
        .enrich_target_roots(
            &ctx.pool(),
            Some(&ctx.ide().projects),
            Some(user),
            &mut req.target,
        )
        .await;
    let ssh = ctx
        .ide()
        .targets
        .resolve_write(ctx, user, &req.target)
        .await?
        .into_ssh_spec();
    if let Some(explicit_root) = req
        .root
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != req.target.root)
    {
        let mut sub_target = req.target.clone();
        sub_target.root = explicit_root.to_string();
        let _ = ctx
            .ide()
            .targets
            .resolve_write(ctx, user, &sub_target)
            .await?;
    }
    let resp = ctx
        .ide()
        .environments
        .create_python_venv(&ctx.pool(), &ctx.ide().targets, req, ssh.as_ref())
        .await?;
    Ok(resp)
}

pub(super) async fn create_python_venv<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeCreatePythonVenvReq>,
) -> ApiResult<Json<IdeCreatePythonVenvResp>> {
    Ok(Json(execute_create_python_venv(&ctx, &user, req).await?))
}

pub(super) async fn upsert_run_config<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeUpsertRunConfigReq>,
) -> ApiResult<Json<IdeRunConfigResp>> {
    let _ = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
    let config = ctx
        .ide()
        .environments
        .upsert_run_config_for_user(&ctx.pool(), req, user.is_root)
        .await?;
    Ok(Json(IdeRunConfigResp { config }))
}

pub(super) async fn delete_run_config<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Query(q): Query<IdeTargetQuery>,
) -> ApiResult<Json<IdeOkResp>> {
    let explicit_target = if q.root.as_deref().is_some_and(|r| !r.trim().is_empty())
        || q.kind == Some(IdeTargetKind::Ssh)
    {
        let spec = q.into_target_spec()?;
        let _ = ctx.ide().targets.resolve_write(&ctx, &user, &spec).await?;
        Some(spec)
    } else {
        None
    };
    let target = ctx
        .ide()
        .environments
        .get_run_config_target(&ctx.pool(), explicit_target.as_ref(), &id)
        .await?;
    if explicit_target.is_none() {
        let _ = ctx
            .ide()
            .targets
            .resolve_write(&ctx, &user, &target)
            .await?;
    }
    ctx.ide()
        .environments
        .delete_run_config(&ctx.pool(), &target, &id)
        .await?;
    Ok(Json(IdeOkResp { ok: true }))
}

pub(super) async fn start_run<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeStartRunReq>,
) -> ApiResult<Json<IdeRunResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    if !user.is_root && req.target.kind == IdeTargetKind::Local {
        return Err(Error::Forbidden(
            "local unsandboxed execution requires root; use a remote SSH target".into(),
        )
        .into());
    }
    let ssh = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?
        .into_ssh_spec();
    let proj_nb = ctx
        .ide()
        .environments
        .get_project_env(&ctx.pool(), &req.target)
        .await
        .ok()
        .and_then(|p| p.network_bridge);
    let netbridge = match proj_nb.as_ref().filter(|nb| nb.enabled) {
        Some(nb) => {
            ctx.ide()
                .netbridge
                .acquire_for_launch_with_confirm(
                    &ctx,
                    &user,
                    &req.target,
                    nb,
                    req.confirm_prod_targets,
                )
                .await?
        }
        None => None,
    };
    let run = otto_telemetry::context::measure_result("ide.run.launch", async {
        ctx.ide()
            .environments
            .start_run_with_netbridge(
                &ctx.pool(),
                ctx.secrets(),
                &ctx.ide().targets,
                Some(&ctx.ide().projects),
                Some(&user),
                req,
                ssh,
                netbridge,
            )
            .await
    })
    .await?;
    Ok(Json(IdeRunResp { run }))
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct GetRunQuery {
    #[serde(default)]
    pub tail_lines: Option<usize>,
}

pub(super) async fn get_run<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(run_id): Path<String>,
    Query(q): Query<GetRunQuery>,
) -> ApiResult<Json<IdeRunResp>> {
    let run = ctx
        .ide()
        .environments
        .get_run_for_user(&user, &run_id, q.tail_lines)?;
    Ok(Json(IdeRunResp { run }))
}

pub(super) async fn stream_run_events<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(run_id): Path<String>,
) -> ApiResult<Response> {
    let (initial, rx) = ctx
        .ide()
        .environments
        .subscribe_run_for_user(&user, &run_id)?;
    Ok(build_run_sse_response(initial, rx))
}

pub(super) async fn stop_run<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<IdeRunResp>> {
    let run = ctx
        .ide()
        .environments
        .stop_run_for_user(&user, &run_id)
        .await?;
    Ok(Json(IdeRunResp { run }))
}

pub(super) async fn start_debug_session<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeStartDebugReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    ctx.ide()
        .environments
        .enrich_target_roots(
            &ctx.pool(),
            Some(&ctx.ide().projects),
            Some(&user),
            &mut req.target,
        )
        .await;
    let ssh = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?
        .into_ssh_spec();
    if !user.is_root && req.target.kind == IdeTargetKind::Local {
        return Err(Error::Forbidden(
            "local unsandboxed debugging requires root; use a remote SSH target".into(),
        )
        .into());
    }
    let run_config = match req.config_id.as_deref().filter(|s| !s.is_empty()) {
        Some(cid) => Some(
            ctx.ide()
                .environments
                .resolve_run_config_or_auto(&ctx.pool(), &req.target, cid)
                .await?,
        ),
        None => None,
    };
    let adhoc_cfg = if run_config.is_none() {
        Some(crate::types::IdeRunConfig {
            cwd: req.launch.cwd.clone(),
            target_path: (!req.launch.program.trim().is_empty())
                .then(|| req.launch.program.clone()),
            env_file: req.launch.env_file.clone(),
            inherit_shell_env: req.launch.inherit_shell_env.unwrap_or(true),
            sdk_path: req.launch.sdk_path.clone(),
            ..Default::default()
        })
    } else {
        None
    };
    let (loaded_proj, effective_env) = ctx
        .ide()
        .environments
        .build_effective_env_with_loaded_opts(
            &ctx.pool(),
            ctx.secrets(),
            &req.target,
            run_config.as_ref().or(adhoc_cfg.as_ref()),
            false,
            user.is_root,
        )
        .await?;
    let netbridge = match loaded_proj.network_bridge.as_ref().filter(|nb| nb.enabled) {
        Some(nb) => {
            ctx.ide()
                .netbridge
                .acquire_for_launch_with_confirm(
                    &ctx,
                    &user,
                    &req.target,
                    nb,
                    req.confirm_prod_targets,
                )
                .await?
        }
        None => None,
    };
    let go_build_tags = loaded_proj.go_build_tags.clone();
    let project_env = Some(loaded_proj);
    let plugin_adapters = ctx.ide().plugins.active_dap_adapters();
    let start_ctx = DapStartContext::new()
        .with_ssh(ssh)
        .with_effective_env(effective_env)
        .with_project_env(project_env)
        .with_go_build_tags(go_build_tags)
        .with_run_config(run_config)
        .with_plugin_adapters(plugin_adapters)
        .with_netbridge(netbridge)
        .with_caller_is_root(user.is_root)
        .with_owner_user_id(&user.id);

    let target_for_collab = req.target.clone();
    let owner_id = user.id.clone();
    let (session, mut rx) = otto_telemetry::context::measure_result("ide.dap.launch", async {
        ctx.ide().dap.start_session_async(req, start_ctx)
    })
    .await?;
    ctx.ide()
        .collab
        .publish_debug_snapshot(Some(&target_for_collab), Some(&owner_id), &session);

    let engine = ctx.ide().clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(snap) => {
                    engine.collab.publish_debug_snapshot(
                        Some(&target_for_collab),
                        Some(&owner_id),
                        &snap,
                    );
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    Ok(Json(IdeDebugSessionResp { session }))
}

pub(super) async fn list_debug_sessions<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<IdeDebugSessionsResp>> {
    let sessions = ctx.ide().dap.list_sessions_for_user(&user);
    Ok(Json(IdeDebugSessionsResp { sessions }))
}

pub(super) async fn get_debug_session<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    ctx.ide().authorize_debug_session(&user, &id)?;
    let session = ctx.ide().dap.get_session(&id)?;
    Ok(Json(IdeDebugSessionResp { session }))
}

pub(super) async fn stream_debug_events<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    ctx.ide().authorize_debug_session(&user, &id)?;
    let rx = ctx.ide().dap.subscribe_events(&id)?;
    let initial = ctx.ide().dap.get_session(&id)?;
    Ok(build_debug_sse_response(initial, rx))
}

pub(super) async fn set_debug_breakpoints<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeSetBreakpointsReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    ctx.ide().authorize_debug_session(&user, &id)?;
    let owner_target = ctx.ide().dap.session_owner_and_target(&id);
    let session = ctx.ide().dap.set_breakpoints(&id, req).await?;
    ctx.ide().collab.publish_debug_snapshot(
        owner_target.as_ref().map(|(_, t)| t),
        owner_target.as_ref().map(|(o, _)| o.as_str()),
        &session,
    );
    Ok(Json(IdeDebugSessionResp { session }))
}

pub(super) async fn debug_command<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDebugCommandReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    ctx.ide().authorize_debug_session(&user, &id)?;
    let owner_target = ctx.ide().dap.session_owner_and_target(&id);
    let session = ctx.ide().dap.command(&id, req).await?;
    ctx.ide().collab.publish_debug_snapshot(
        owner_target.as_ref().map(|(_, t)| t),
        owner_target.as_ref().map(|(o, _)| o.as_str()),
        &session,
    );
    Ok(Json(IdeDebugSessionResp { session }))
}

pub(super) async fn debug_variables<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDebugVariablesReq>,
) -> ApiResult<Json<IdeDebugVariablesResp>> {
    ctx.ide().authorize_debug_session(&user, &id)?;
    let variables = ctx
        .ide()
        .dap
        .expand_variables(&id, req.variables_reference)
        .await?;
    Ok(Json(IdeDebugVariablesResp { variables }))
}

pub(super) async fn debug_evaluate<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDebugEvaluateReq>,
) -> ApiResult<Json<IdeDebugEvaluateResp>> {
    ctx.ide().authorize_debug_session(&user, &id)?;
    let resp = ctx.ide().dap.evaluate(&id, req).await?;
    Ok(Json(resp))
}
