//! Provider-independent projects; workspace role + Agents feature gated.
use crate::{
    auth::{require_ws_role, CurrentUser},
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
        // Membership never grants terminal ownership or protected-resource
        // access. Owner-or-admin is already decided by the query: it is scoped
        // to the project's workspace and, for a non-admin, to `created_by =
        // user` (the per-row re-check cost one role query per row). The
        // resource-bound rows (k8s / aws / db_assist) are decided in one batch.
        // Continue paging until enough accessible rows are found.
        let admitted = crate::resource_sessions::check_many(&ctx, &user, page.items).await;
        visible.extend(admitted.into_iter().take(limit - visible.len()));
    }
    Ok(Json(visible))
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_state::sessions::NewSession;

    /// The page shows a member only their own project sessions and an admin
    /// every one — decided by the owner-scoped query plus ONE batched
    /// resource check, not a per-row owner/admin re-check — and a row with a
    /// malformed resource binding stays hidden even from its owner.
    #[tokio::test]
    async fn sessions_page_shows_owners_their_rows_and_admins_all() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = crate::test_support::mem_pool().await;
        let ctx = ServerCtx::for_tests(&pool, tmp.path().to_path_buf()).await;
        let users = otto_state::UsersRepo::new(pool.clone());
        let admin = users.create("admin", "x", "Admin", false).await.unwrap();
        let alice = users.create("alice", "x", "Alice", false).await.unwrap();
        let bob = users.create("bob", "x", "Bob", false).await.unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        let ws = ctx.workspaces.create("A", &root, &admin.id).await.unwrap();
        for (u, role) in [
            (&admin, WorkspaceRole::Admin),
            (&alice, WorkspaceRole::Editor),
            (&bob, WorkspaceRole::Editor),
        ] {
            ctx.workspaces
                .set_member(&ws.id, &u.id, role)
                .await
                .unwrap();
        }
        let project = repo(&ctx)
            .create(
                &ws.id,
                &alice.id,
                ProjectInput {
                    name: "p".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let sessions_repo = otto_state::SessionsRepo::new(pool.clone());
        let mk = |by: &Id, meta: serde_json::Value| NewSession {
            workspace_id: ws.id.clone(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "codex".into(),
            title: "t".into(),
            cwd: root.clone(),
            provider_session_id: None,
            connection_id: None,
            created_by: by.clone(),
            meta,
        };
        let pid = project.id.to_string();
        let mine = serde_json::json!({ "project_id": pid });
        let a1 = sessions_repo
            .create(mk(&alice.id, mine.clone()))
            .await
            .unwrap();
        let b1 = sessions_repo
            .create(mk(&bob.id, mine.clone()))
            .await
            .unwrap();
        // Another project's row, and a row whose aws binding is malformed.
        let other = repo(&ctx)
            .create(
                &ws.id,
                &alice.id,
                ProjectInput {
                    name: "other".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        sessions_repo
            .create(mk(
                &alice.id,
                serde_json::json!({ "project_id": other.id.to_string() }),
            ))
            .await
            .unwrap();
        sessions_repo
            .create(mk(
                &alice.id,
                serde_json::json!({ "project_id": pid, "aws": { "nope": 1 } }),
            ))
            .await
            .unwrap();

        let page = |u: &otto_core::domain::User| {
            let (ctx, u, id) = (ctx.clone(), u.clone(), project.id.clone());
            async move {
                let mut ids: Vec<String> = sessions(
                    State(ctx),
                    CurrentUser(u),
                    Path(id),
                    Query(SessionQuery { limit: None }),
                )
                .await
                .unwrap()
                .0
                .into_iter()
                .map(|s| s.id.to_string())
                .collect();
                ids.sort();
                ids
            }
        };
        let mut both = vec![a1.id.to_string(), b1.id.to_string()];
        both.sort();
        assert_eq!(page(&alice).await, vec![a1.id.to_string()]);
        assert_eq!(page(&bob).await, vec![b1.id.to_string()]);
        assert_eq!(page(&admin).await, both);
    }
}
