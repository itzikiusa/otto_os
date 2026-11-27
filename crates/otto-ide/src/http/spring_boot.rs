//! HTTP handlers for Spring Boot Actuator inspection (`POST /ide/spring/actuator`)
//! and DevTools live restart (`POST /ide/spring/devtools-restart`).

use axum::extract::State;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::Error;

use super::ApiResult;
use crate::target::ResolvedTarget;
use crate::types::{
    IdeSpringActuatorReq, IdeSpringActuatorResp, IdeSpringDevtoolsRestartReq,
    IdeSpringDevtoolsRestartResp,
};
use crate::IdeCtx;

pub(super) async fn spring_actuator<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeSpringActuatorReq>,
) -> ApiResult<Json<IdeSpringActuatorResp>> {
    if let Some(ref target) = req.target {
        if !target.root.trim().is_empty() {
            let _ = ctx.ide().targets.resolve(&ctx, &user, target).await?;
        }
    }
    let resp = crate::spring_boot::actuator::inspect_actuator(&req).await?;
    Ok(Json(resp))
}

pub(super) async fn spring_devtools_restart<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeSpringDevtoolsRestartReq>,
) -> ApiResult<Json<IdeSpringDevtoolsRestartResp>> {
    let resolved = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
    let canon_root = match resolved {
        ResolvedTarget::Local { root, .. } => root,
        ResolvedTarget::Ssh { .. } => {
            return Err(Error::Invalid(
                "DevTools restart trigger is supported on local workspaces".into(),
            )
            .into());
        }
    };
    let resp = crate::spring_boot::actuator::trigger_devtools_restart(
        &canon_root,
        req.trigger_file.as_deref(),
    )
    .await?;
    Ok(Json(resp))
}
