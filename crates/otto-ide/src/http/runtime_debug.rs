//! Environments, Run Configurations, Process Runner & DAP Debugger HTTP Handlers.

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use super::collab_sse::build_debug_sse_response;
use super::{resolve_ssh_opt, ApiResult};
use crate::dap::DapStartContext;
use crate::types::{
    IdeCreatePythonVenvReq, IdeCreatePythonVenvResp, IdeDebugCommandReq, IdeDebugEvaluateReq,
    IdeDebugEvaluateResp, IdeDebugSessionResp, IdeDebugSessionsResp, IdeDebugVariablesReq,
    IdeDebugVariablesResp, IdeEnvironmentResp, IdeEnvironmentsResp, IdeOkResp, IdeRunConfigResp,
    IdeRunResp, IdeSetBreakpointsReq, IdeStartDebugReq, IdeStartRunReq, IdeTargetQuery,
    IdeUpdateEnvironmentReq, IdeUpsertRunConfigReq,
};
use crate::IdeCtx;

pub(super) async fn get_environments<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeTargetQuery>,
) -> ApiResult<Json<IdeEnvironmentsResp>> {
    let mut target = q.into_target_spec()?;
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut target)
        .await;
    let _ = ctx.ide().targets.resolve(&ctx, &user, &target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &target).await?;
    let resp = ctx
        .ide()
        .environments
        .get_environments(
            &ctx.pool(),
            ctx.secrets(),
            &ctx.ide().targets,
            &target,
            ssh.as_ref(),
        )
        .await?;
    Ok(Json(resp))
}

pub(super) async fn put_environments<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeUpdateEnvironmentReq>,
) -> ApiResult<Json<IdeEnvironmentResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let env = ctx
        .ide()
        .environments
        .update_environment(&ctx.pool(), ctx.secrets(), req)
        .await?;
    Ok(Json(IdeEnvironmentResp { env }))
}

pub(super) async fn create_python_venv<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeCreatePythonVenvReq>,
) -> ApiResult<Json<IdeCreatePythonVenvResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &req.target).await?;
    let resp = ctx
        .ide()
        .environments
        .create_python_venv(&ctx.pool(), &ctx.ide().targets, req, ssh.as_ref())
        .await?;
    Ok(Json(resp))
}

pub(super) async fn upsert_run_config<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeUpsertRunConfigReq>,
) -> ApiResult<Json<IdeRunConfigResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let config = ctx
        .ide()
        .environments
        .upsert_run_config(&ctx.pool(), req)
        .await?;
    Ok(Json(IdeRunConfigResp { config }))
}

pub(super) async fn delete_run_config<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Query(q): Query<IdeTargetQuery>,
) -> ApiResult<Json<IdeOkResp>> {
    if q.root.as_deref().is_some_and(|r| !r.trim().is_empty()) {
        let target = q.into_target_spec()?;
        let _ = ctx.ide().targets.resolve(&ctx, &user, &target).await?;
    }
    ctx.ide()
        .environments
        .delete_run_config(&ctx.pool(), &id)
        .await?;
    Ok(Json(IdeOkResp { ok: true }))
}

pub(super) async fn start_run<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeStartRunReq>,
) -> ApiResult<Json<IdeRunResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &req.target).await?;
    let run = ctx
        .ide()
        .environments
        .start_run(&ctx.pool(), ctx.secrets(), &ctx.ide().targets, req, ssh)
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
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(run_id): Path<String>,
    Query(q): Query<GetRunQuery>,
) -> ApiResult<Json<IdeRunResp>> {
    let run = ctx
        .ide()
        .environments
        .get_run_with_tail(&run_id, q.tail_lines)?;
    Ok(Json(IdeRunResp { run }))
}

pub(super) async fn stop_run<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<IdeRunResp>> {
    let run = ctx.ide().environments.stop_run(&run_id).await?;
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
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &req.target).await?;
    let run_config = match req.config_id.as_deref().filter(|s| !s.is_empty()) {
        Some(cid) => ctx
            .ide()
            .environments
            .resolve_run_config_or_auto(&ctx.pool(), &req.target, cid)
            .await
            .ok(),
        None => None,
    };
    let effective_env = ctx
        .ide()
        .environments
        .build_effective_env_for_run(&ctx.pool(), ctx.secrets(), &req.target, run_config.as_ref())
        .await
        .unwrap_or_default();
    let project_env = ctx
        .ide()
        .environments
        .get_project_env(&ctx.pool(), &req.target)
        .await
        .ok();
    let plugin_adapters = ctx.ide().plugins.active_dap_adapters();
    let start_ctx = DapStartContext::new()
        .with_ssh(ssh)
        .with_effective_env(effective_env)
        .with_project_env(project_env)
        .with_run_config(run_config)
        .with_plugin_adapters(plugin_adapters)
        .with_caller_is_root(user.is_root);

    let target_for_collab = req.target.clone();
    let session = ctx.ide().dap.start_session(req, start_ctx).await?;
    ctx.ide()
        .collab
        .publish_debug_snapshot(Some(&target_for_collab), &session);

    if let Ok(mut rx) = ctx.ide().dap.subscribe_events(&session.id) {
        let engine = ctx.ide().clone();
        let target_clone = target_for_collab;
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(snap) => {
                        engine
                            .collab
                            .publish_debug_snapshot(Some(&target_clone), &snap);
                    }
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                }
            }
        });
    }

    Ok(Json(IdeDebugSessionResp { session }))
}

pub(super) async fn list_debug_sessions<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Json<IdeDebugSessionsResp>> {
    Ok(Json(IdeDebugSessionsResp {
        sessions: ctx.ide().dap.list_sessions(),
    }))
}

pub(super) async fn get_debug_session<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    let session = ctx.ide().dap.get_session(&id)?;
    Ok(Json(IdeDebugSessionResp { session }))
}

pub(super) async fn stream_debug_events<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let initial = ctx.ide().dap.get_session(&id)?;
    let rx = ctx.ide().dap.subscribe_events(&id)?;
    Ok(build_debug_sse_response(initial, rx))
}

pub(super) async fn set_debug_breakpoints<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeSetBreakpointsReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    let session = ctx.ide().dap.set_breakpoints(&id, req).await?;
    ctx.ide().collab.publish_debug_snapshot(None, &session);
    Ok(Json(IdeDebugSessionResp { session }))
}

pub(super) async fn debug_command<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDebugCommandReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    let session = ctx.ide().dap.command(&id, req).await?;
    ctx.ide().collab.publish_debug_snapshot(None, &session);
    Ok(Json(IdeDebugSessionResp { session }))
}

pub(super) async fn debug_variables<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDebugVariablesReq>,
) -> ApiResult<Json<IdeDebugVariablesResp>> {
    let variables = ctx
        .ide()
        .dap
        .expand_variables(&id, req.variables_reference)
        .await?;
    Ok(Json(IdeDebugVariablesResp { variables }))
}

pub(super) async fn debug_evaluate<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDebugEvaluateReq>,
) -> ApiResult<Json<IdeDebugEvaluateResp>> {
    let resp = ctx.ide().dap.evaluate(&id, req).await?;
    Ok(Json(resp))
}
