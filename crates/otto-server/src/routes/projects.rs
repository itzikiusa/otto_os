//! Provider-independent projects; workspace role + Agents feature gated.
use crate::{
    auth::{require_session_owner_or_admin, require_ws_role, CurrentUser},
    error::ApiResult,
    state::ServerCtx,
};
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use otto_core::{
    domain::{Session, WorkspaceRole},
    Id,
};
use otto_state::projects::{Project, ProjectInput, ProjectsRepo};
use serde::Deserialize;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{id}/projects", get(list).post(create))
        .route("/projects/{id}", get(get_one).put(update))
        .route("/projects/{id}/sessions", get(sessions))
}
fn repo(ctx: &ServerCtx) -> ProjectsRepo {
    ProjectsRepo::new(ctx.pool.clone())
}
async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<Project>>> {
    require_ws_role(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list(&id).await?))
}
async fn create(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
    Json(input): Json<ProjectInput>,
) -> ApiResult<Json<Project>> {
    require_ws_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(repo(&ctx).create(&id, &user.id, input).await?))
}
async fn get_one(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
) -> ApiResult<Json<Project>> {
    let project = repo(&ctx).get(&id).await?;
    require_ws_role(&ctx, &user, &project.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(project))
}
#[derive(Deserialize)]
struct UpdateProjectReq {
    #[serde(flatten)]
    input: ProjectInput,
    context_version: i64,
}
async fn update(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
    Json(input): Json<UpdateProjectReq>,
) -> ApiResult<Json<Project>> {
    let project = repo(&ctx).get(&id).await?;
    require_ws_role(&ctx, &user, &project.workspace_id, WorkspaceRole::Editor).await?;
    Ok(Json(
        repo(&ctx)
            .update(&id, input.input, input.context_version)
            .await?,
    ))
}
#[derive(Deserialize)]
struct SessionQuery {
    limit: Option<i64>,
}
async fn sessions(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
    Query(query): Query<SessionQuery>,
) -> ApiResult<Json<Vec<Session>>> {
    let project = repo(&ctx).get(&id).await?;
    require_ws_role(&ctx, &user, &project.workspace_id, WorkspaceRole::Viewer).await?;
    let admin = user.is_root
        || ctx
            .roles
            .check(&user, &project.workspace_id, WorkspaceRole::Admin)
            .await
            .is_ok();
    let owner = if admin { None } else { Some(user.id.as_str()) };
    let limit = query.limit.unwrap_or(200).clamp(1, 500) as usize;
    let mut visible = Vec::new();
    let mut before: Option<(String, Id)> = None;
    while visible.len() < limit {
        let cursor = before.as_ref().map(|(at, id)| (at.as_str(), id.as_str()));
        let page = repo(&ctx)
            .session_page(&id, &project.workspace_id, owner, cursor, 200)
            .await?;
        if page.items.is_empty() {
            break;
        }
        before = page.next;
        for session in page.items {
            // Membership never grants terminal ownership or protected-resource
            // access. Continue paging until enough accessible rows are found.
            if require_session_owner_or_admin(&ctx, &user, &session)
                .await
                .is_ok()
            {
                visible.push(session);
            }
            if visible.len() == limit {
                break;
            }
        }
    }
    Ok(Json(visible))
}
