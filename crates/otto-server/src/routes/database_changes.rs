//! HTTP contract for independently reviewed MySQL/PostgreSQL changes.
use crate::{
    auth::{BearerToken, CurrentAuthContext},
    database_changes as service,
    error::ApiResult,
    state::ServerCtx,
};
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Extension, Json, Router,
};
use otto_core::{Error, Id};
use otto_state::{
    database_changes::{
        ChangeAttempt, ChangeEvent, ChangeInput, DatabaseChange, DatabaseChangesRepo,
    },
    UsersRepo,
};
use serde::{Deserialize, Serialize};

pub fn api_router() -> Router<ServerCtx> {
    Router::new()
        .route("/database-changes", get(list).post(create))
        .route("/database-changes/{id}", get(detail).put(revise))
        .route("/database-changes/{id}/executors", get(executors))
        .route("/database-changes/{id}/validate", post(validate))
        .route("/database-changes/{id}/submit", post(submit))
        .route("/database-changes/{id}/approve", post(approve))
        .route("/database-changes/{id}/reject", post(reject))
        .route("/database-changes/{id}/execute", post(execute))
        .route("/database-changes/{id}/cancel", post(cancel))
        .route("/database-changes/{id}/reconcile", post(reconcile))
}
#[derive(Deserialize)]
struct ListQuery {
    before: Option<String>,
    connection_id: Option<String>,
}
#[derive(Serialize)]
struct Detail {
    change: DatabaseChange,
    attempts: Vec<ChangeAttempt>,
    history: Vec<ChangeEvent>,
}
#[derive(Deserialize)]
struct Revision {
    revision: i64,
    #[serde(default)]
    note: String,
}
#[derive(Deserialize)]
struct Revise {
    revision: i64,
    #[serde(flatten)]
    input: ChangeInput,
}
#[derive(Deserialize)]
struct Validate {
    revision: i64,
    executor_id: Id,
}
#[derive(Deserialize)]
struct Reconcile {
    revision: i64,
    attempt_id: Id,
    outcome: String,
    note: String,
}
fn expected(change: &DatabaseChange, revision: i64) -> otto_core::Result<()> {
    if change.revision != revision {
        Err(Error::Conflict(
            "change revision is stale; refresh before continuing".into(),
        ))
    } else {
        Ok(())
    }
}
async fn list(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<DatabaseChange>>> {
    service::token_gate(&auth)?;
    let mut out = Vec::new();
    for change in DatabaseChangesRepo::new(ctx.pool.clone())
        .list(q.before.as_deref())
        .await?
    {
        if q.connection_id
            .as_ref()
            .is_none_or(|id| change.targets.iter().any(|t| &t.connection_id == id))
            && service::visible(&ctx, &auth.effective_user, &change).await
        {
            out.push(change)
        }
    }
    Ok(Json(out))
}
async fn detail(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
) -> ApiResult<Json<Detail>> {
    let change = service::load_visible(&ctx, &auth, &id).await?;
    let repo = DatabaseChangesRepo::new(ctx.pool.clone());
    let mut history = Vec::new();
    for item in repo.history(&id).await? {
        // Revision snapshots can contain targets that are no longer in the
        // current revision. Authorize their original scope before returning SQL.
        if let Some(targets) = item.data.get("targets") {
            let old_targets: Vec<otto_state::database_changes::ChangeTarget> =
                serde_json::from_value(targets.clone())
                    .map_err(|_| Error::Internal("invalid revision history".into()))?;
            let mut historical = change.clone();
            historical.targets = old_targets;
            if !service::visible(&ctx, &auth.effective_user, &historical).await {
                continue;
            }
        }
        history.push(item);
    }
    Ok(Json(Detail {
        change,
        attempts: repo.attempts(&id).await?,
        history,
    }))
}
async fn create(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(input): Json<ChangeInput>,
) -> ApiResult<Json<DatabaseChange>> {
    service::token_gate(&auth)?;
    let input = service::normalize(input)?;
    service::authorize(&ctx, &auth.effective_user, &input.targets, "change_submit").await?;
    Ok(Json(
        DatabaseChangesRepo::new(ctx.pool.clone())
            .create(&input, &auth.effective_user.id, &auth.real_user.id)
            .await?,
    ))
}
async fn revise(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<Revise>,
) -> ApiResult<Json<DatabaseChange>> {
    let old = service::load_visible(&ctx, &auth, &id).await?;
    expected(&old, req.revision)?;
    service::author(&old, &auth)?;
    let input = service::normalize(req.input)?;
    service::authorize(&ctx, &auth.effective_user, &old.targets, "change_submit").await?;
    service::authorize(&ctx, &auth.effective_user, &input.targets, "change_submit").await?;
    Ok(Json(
        DatabaseChangesRepo::new(ctx.pool.clone())
            .revise(&old, &input, &auth.effective_user.id, &auth.real_user.id)
            .await?,
    ))
}
async fn validate(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<Validate>,
) -> ApiResult<Json<DatabaseChange>> {
    let old = service::load_visible(&ctx, &auth, &id).await?;
    expected(&old, req.revision)?;
    service::author(&old, &auth)?;
    service::authorize(&ctx, &auth.effective_user, &old.targets, "change_submit").await?;
    let executor = UsersRepo::new(ctx.pool.clone())
        .get(&req.executor_id)
        .await?;
    let snapshots = service::snapshots(&ctx, &old, &executor).await?;
    Ok(Json(
        DatabaseChangesRepo::new(ctx.pool.clone())
            .validate(
                &old,
                &executor.id,
                &snapshots,
                &auth.effective_user.id,
                &auth.real_user.id,
            )
            .await?,
    ))
}
async fn submit(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<Revision>,
) -> ApiResult<Json<DatabaseChange>> {
    let old = service::load_visible(&ctx, &auth, &id).await?;
    expected(&old, req.revision)?;
    service::author(&old, &auth)?;
    service::fresh_approval(&ctx, &old).await?;
    Ok(Json(
        DatabaseChangesRepo::new(ctx.pool.clone())
            .transition(
                &old,
                "awaiting_review",
                &auth.effective_user.id,
                &auth.real_user.id,
                &req.note,
            )
            .await?,
    ))
}
async fn approve(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<Revision>,
) -> ApiResult<Json<DatabaseChange>> {
    let old = service::load_visible(&ctx, &auth, &id).await?;
    expected(&old, req.revision)?;
    service::authorize(&ctx, &auth.effective_user, &old.targets, "change_approve").await?;
    service::fresh_approval(&ctx, &old).await?;
    Ok(Json(
        DatabaseChangesRepo::new(ctx.pool.clone())
            .transition(
                &old,
                "approved",
                &auth.effective_user.id,
                &auth.real_user.id,
                &req.note,
            )
            .await?,
    ))
}
async fn reject(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<Revision>,
) -> ApiResult<Json<DatabaseChange>> {
    let old = service::load_visible(&ctx, &auth, &id).await?;
    expected(&old, req.revision)?;
    service::authorize(&ctx, &auth.effective_user, &old.targets, "change_approve").await?;
    if req.note.trim().is_empty() {
        return Err(Error::Invalid("explain why revision is needed".into()).into());
    }
    Ok(Json(
        DatabaseChangesRepo::new(ctx.pool.clone())
            .transition(
                &old,
                "rejected",
                &auth.effective_user.id,
                &auth.real_user.id,
                &req.note,
            )
            .await?,
    ))
}
async fn execute(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Extension(BearerToken(token)): Extension<BearerToken>,
    Path(id): Path<Id>,
    Json(req): Json<Revision>,
) -> ApiResult<Json<DatabaseChange>> {
    let old = service::load_visible(&ctx, &auth, &id).await?;
    expected(&old, req.revision)?;
    service::authorize(&ctx, &auth.effective_user, &old.targets, "change_execute").await?;
    let snapshots = service::fresh_approval(&ctx, &old).await?;
    let approver = UsersRepo::new(ctx.pool.clone())
        .get(
            old.approved_by
                .as_ref()
                .ok_or_else(|| Error::Conflict("independent approval required".into()))?,
        )
        .await?;
    service::authorize(&ctx, &approver, &old.targets, "change_approve").await?;
    let repo = DatabaseChangesRepo::new(ctx.pool.clone());
    repo.claim(
        &old,
        &snapshots,
        &auth.effective_user.id,
        &auth.real_user.id,
    )
    .await?;
    let current = repo.get(&id).await?;
    tokio::spawn(service::run_claimed(ctx, old, auth, token));
    Ok(Json(current))
}
async fn cancel(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<Revision>,
) -> ApiResult<Json<DatabaseChange>> {
    let old = service::load_visible(&ctx, &auth, &id).await?;
    expected(&old, req.revision)?;
    let op = if old.status == "running" {
        "change_execute"
    } else {
        service::author(&old, &auth)?;
        "change_submit"
    };
    service::authorize(&ctx, &auth.effective_user, &old.targets, op).await?;
    Ok(Json(
        DatabaseChangesRepo::new(ctx.pool.clone())
            .request_cancel(&old, &auth.effective_user.id, &auth.real_user.id)
            .await?,
    ))
}
async fn reconcile(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<Reconcile>,
) -> ApiResult<Json<DatabaseChange>> {
    let old = service::load_visible(&ctx, &auth, &id).await?;
    expected(&old, req.revision)?;
    service::authorize(&ctx, &auth.effective_user, &old.targets, "change_execute").await?;
    Ok(Json(
        DatabaseChangesRepo::new(ctx.pool.clone())
            .reconcile(
                &old,
                &req.attempt_id,
                &req.outcome,
                &req.note,
                &auth.effective_user.id,
                &auth.real_user.id,
            )
            .await?,
    ))
}

#[derive(Serialize)]
struct Executor {
    id: Id,
    display_name: String,
    username: String,
}
async fn executors(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<Executor>>> {
    let change = service::load_visible(&ctx, &auth, &id).await?;
    service::authorize(&ctx, &auth.effective_user, &change.targets, "change_submit").await?;
    let mut out = Vec::new();
    for user in UsersRepo::new(ctx.pool.clone()).list().await? {
        if service::authorize(&ctx, &user, &change.targets, "change_execute")
            .await
            .is_ok()
        {
            out.push(Executor {
                id: user.id,
                display_name: user.display_name,
                username: user.username,
            });
        }
    }
    Ok(Json(out))
}
