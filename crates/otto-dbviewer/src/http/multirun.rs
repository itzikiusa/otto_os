//! Multi-target / parameterised run routes ("Run on…"). Every target
//! connection is gated exactly like `POST …/db/query` (`Editor`; global
//! connections: the Database grant), checked for ALL targets before anything
//! is probed or run. Jobs are owner-scoped: another user's multi-run is a 404
//! (root sees all).

use super::*;
use crate::multirun::{MultiRunSpec, StartMultiRunReq};

/// Load every distinct target connection and require `Editor` on each.
async fn check_targets<S: DbViewerCtx>(
    ctx: &S,
    user: &User,
    spec: &MultiRunSpec,
) -> Result<Vec<Connection>, Error> {
    let mut conns: Vec<Connection> = Vec::new();
    for t in &spec.targets {
        if conns.iter().any(|c| c.id == t.connection_id) {
            continue;
        }
        let conn = ctx.db().get_connection(&t.connection_id).await?;
        check_conn_role(ctx, user, &conn, WorkspaceRole::Editor).await?;
        conns.push(conn);
    }
    Ok(conns)
}

/// `POST /db/multi-run/plan` — the preview: final statement per run, write /
/// guard flags, ClickHouse cluster detection per target. Runs only the
/// read-only topology probes.
pub(super) async fn plan<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(spec): Json<MultiRunSpec>,
) -> ApiResult<Response> {
    check_targets(&ctx, &user, &spec).await?;
    Ok(Json(ctx.db().multi_run_plan(&user.id, &spec).await?).into_response())
}

/// `POST /db/multi-runs` — start a multi-run in the background (202 + the job).
/// A guarded write without `confirm_write` is refused before anything runs.
pub(super) async fn start<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<StartMultiRunReq>,
) -> ApiResult<Response> {
    let conns = check_targets(&ctx, &user, &req.spec).await?;
    let (job, plan) = ctx.db().multi_run_start(&user.id, &req).await?;
    // Audit every confirmed guarded write — the same hook a confirmed single
    // run fires — once per run, with the exact statement it sends.
    if req.confirm_write {
        for run in plan.runs.iter().filter(|r| r.needs_confirm) {
            let target = &plan.targets[run.target];
            if let Some(conn) = conns.iter().find(|c| c.id == target.connection_id) {
                ctx.on_confirmed_write(&user, conn, &run.statement);
            }
        }
    }
    Ok((StatusCode::ACCEPTED, Json(job)).into_response())
}

/// `GET /db/multi-runs` — the caller's retained multi-runs, newest first.
pub(super) async fn list<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Response> {
    Ok(Json(ctx.db().multi_run_list(&user.id, user.is_root)).into_response())
}

/// `GET /db/multi-runs/{rid}` — status, summary and per-run detail (no rows).
pub(super) async fn get_one<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(rid): Path<String>,
) -> ApiResult<Response> {
    Ok(Json(ctx.db().multi_run_get(&user.id, user.is_root, &rid)?).into_response())
}

/// `GET /db/multi-runs/{rid}/items/{index}` — one run's full statement and
/// retained result. Result data: the caller must still hold `Editor` on the
/// run's connection (as for `…/db/query-status`).
pub(super) async fn item<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((rid, index)): Path<(String, usize)>,
) -> ApiResult<Response> {
    let detail = ctx
        .db()
        .multi_run_item(&user.id, user.is_root, &rid, index)?;
    let conn = ctx.db().get_connection(&detail.connection_id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    Ok(Json(detail).into_response())
}

/// `POST /db/multi-runs/{rid}/cancel` — stop the multi-run (idempotent).
pub(super) async fn cancel<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(rid): Path<String>,
) -> ApiResult<Response> {
    Ok(Json(ctx.db().multi_run_cancel(&user.id, user.is_root, &rid)?).into_response())
}
