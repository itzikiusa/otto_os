//! `POST /runtime-sessions/{id}/tasks/{task_id}/stop` and `…/tasks/stop`: a
//! PERSON stops an agent's background work (the thread's owner or an admin,
//! like the turn's interrupt). Outcomes: `crate::runtime::task_stop`.

use axum::extract::{Path, State};
use axum::{Extension, Json};
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::Id;

use super::{authorized, require_human, ApiResult};
use crate::runtime::{TaskStopAllResp, TaskStopResp};
use crate::RuntimeCtx;

/// 404 unknown task, 409 already ended / thread closed.
pub(super) async fn stop_one<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path((id, task_id)): Path<(Id, String)>,
) -> ApiResult<Json<TaskStopResp>> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    Ok(Json(ctx.agent_runtime().stop_task(&id, &task_id).await?))
}

/// Every running background task; each row says how, or why not.
pub(super) async fn stop_all<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> ApiResult<Json<TaskStopAllResp>> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    Ok(Json(ctx.agent_runtime().stop_all_tasks(&id).await?))
}
