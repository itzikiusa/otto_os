//! Network profile configuration and authorized live session forwarding status.
use crate::{
    auth::{check_session_owner_or_admin, CurrentUser},
    error::{ApiError, ApiResult},
    state::ServerCtx,
};
use axum::{
    extract::{FromRef, Path, State},
    routing::get,
    Json, Router,
};
use otto_core::{
    domain::{Capability, Feature, User, WorkspaceRole},
    network_profiles::{NetworkProfile, NetworkProfileInput, SessionNetworkStatus},
    Error, Id,
};
use otto_state::network_profiles::NetworkProfilesRepo;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
struct NetworkCtx {
    pool: otto_state::SqlitePool,
    manager: std::sync::Arc<otto_sessions::SessionManager>,
    roles: std::sync::Arc<dyn otto_core::auth::RoleChecker>,
}
impl FromRef<ServerCtx> for NetworkCtx {
    fn from_ref(ctx: &ServerCtx) -> Self {
        Self {
            pool: ctx.pool.clone(),
            manager: ctx.manager.clone(),
            roles: ctx.roles.clone(),
        }
    }
}

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{id}/network-profiles", get(list).post(create))
        .route("/network-profiles/{id}", get(one).put(update))
        .route("/sessions/{id}/network", get(status))
}
fn repo(ctx: &NetworkCtx) -> NetworkProfilesRepo {
    NetworkProfilesRepo::new(ctx.pool.clone())
}
async fn check(ctx: &NetworkCtx, user: &User, workspace: &Id, write: bool) -> ApiResult<()> {
    ctx.roles
        .check(
            user,
            workspace,
            if write {
                WorkspaceRole::Editor
            } else {
                WorkspaceRole::Viewer
            },
        )
        .await?;
    otto_state::GrantsRepo::new(ctx.pool.clone())
        .check_global(
            user,
            Feature::Connections,
            if write {
                Capability::Edit
            } else {
                Capability::View
            },
            "Connections access required",
        )
        .await?;
    Ok(())
}
async fn list(
    State(ctx): State<NetworkCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<NetworkProfile>>> {
    check(&ctx, &user, &id, false).await?;
    let profiles = repo(&ctx).list(&id).await?;
    let mut visible = Vec::new();
    for profile in profiles {
        if otto_sessions::network::authorize(&ctx.pool, &user, &profile)
            .await
            .is_ok()
        {
            visible.push(profile);
        }
    }
    Ok(Json(visible))
}
async fn one(
    State(ctx): State<NetworkCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
) -> ApiResult<Json<NetworkProfile>> {
    let profile = repo(&ctx).get(&id).await?;
    check(&ctx, &user, &profile.workspace_id, false).await?;
    otto_sessions::network::authorize(&ctx.pool, &user, &profile).await?;
    Ok(Json(profile))
}
async fn authorize_input(
    ctx: &NetworkCtx,
    user: &User,
    workspace: &Id,
    input: &NetworkProfileInput,
) -> ApiResult<()> {
    input.validate()?;
    // Resolve the same execution permission before persisting a selectable profile.
    let candidate = NetworkProfile {
        id: String::new(),
        workspace_id: workspace.clone(),
        input: input.clone(),
        version: 1,
        created_by: user.id.clone(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    otto_sessions::network::authorize(&ctx.pool, user, &candidate).await?;
    Ok(())
}
async fn create(
    State(ctx): State<NetworkCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
    Json(input): Json<NetworkProfileInput>,
) -> ApiResult<Json<NetworkProfile>> {
    check(&ctx, &user, &id, true).await?;
    authorize_input(&ctx, &user, &id, &input).await?;
    Ok(Json(repo(&ctx).create(&id, &user.id, input).await?))
}
#[derive(Deserialize)]
struct Update {
    #[serde(flatten)]
    input: NetworkProfileInput,
    version: i64,
}
async fn update(
    State(ctx): State<NetworkCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
    Json(update): Json<Update>,
) -> ApiResult<Json<NetworkProfile>> {
    let profile = repo(&ctx).get(&id).await?;
    check(&ctx, &user, &profile.workspace_id, true).await?;
    otto_sessions::network::authorize(&ctx.pool, &user, &profile).await?;
    authorize_input(&ctx, &user, &profile.workspace_id, &update.input).await?;
    Ok(Json(
        repo(&ctx).update(&id, update.input, update.version).await?,
    ))
}
#[derive(Serialize)]
struct Status {
    #[serde(flatten)]
    current: SessionNetworkStatus,
    selected_profile_id: Option<Id>,
    restart_required: bool,
}
async fn status(
    State(ctx): State<NetworkCtx>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<Id>,
) -> ApiResult<Json<Status>> {
    let session = ctx.manager.get(&id).await?;
    crate::resource_sessions::check_with_pool(&ctx.pool, &user, &session).await?;
    check_session_owner_or_admin(ctx.roles.as_ref(), &user, &session).await?;
    check(&ctx, &user, &session.workspace_id, false).await?;
    let selected_id = match session.meta.get("network_profile_id") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(id)) => Some(id.clone()),
        _ => {
            return Err(ApiError(Error::Invalid(
                "invalid session network profile".into(),
            )))
        }
    };
    let selected = if let Some(id) = &selected_id {
        let profile = repo(&ctx).get(id).await?;
        if profile.workspace_id != session.workspace_id {
            return Err(ApiError(Error::Forbidden(
                "network profile workspace mismatch".into(),
            )));
        }
        otto_sessions::network::authorize(&ctx.pool, &user, &profile).await?;
        Some(profile)
    } else {
        None
    };
    let mut current = ctx.manager.network_status(&id);
    if let Some(active_id) = &current.profile_id {
        let profile = repo(&ctx).get(active_id).await?;
        otto_sessions::network::authorize(&ctx.pool, &user, &profile).await?;
    }
    let restart_required = current.profile_id != selected_id
        || selected
            .as_ref()
            .is_some_and(|profile| current.profile_version != Some(profile.version));
    if current.status == "disabled" && selected.is_some() {
        current.status = "stopped".into();
        current.profile_name = selected.as_ref().map(|p| p.input.name.clone());
    }
    Ok(Json(Status {
        current,
        selected_profile_id: selected_id,
        restart_required,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn direct_network_status_checks_owner_feature_and_bastion_access() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
            Extension,
        };
        use otto_core::{auth::AuthUser, domain::SessionKind};
        use tower::ServiceExt;
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let users = otto_state::UsersRepo::new(pool.clone());
        let owner = users
            .create("network-owner", "hash", "Owner", false)
            .await
            .unwrap();
        let outsider = users
            .create("network-other", "hash", "Other", false)
            .await
            .unwrap();
        let workspaces = otto_state::WorkspacesRepo::new(pool.clone());
        let workspace = workspaces
            .create("Network", "/tmp", &owner.id)
            .await
            .unwrap();
        workspaces
            .set_member(&workspace.id, &outsider.id, WorkspaceRole::Editor)
            .await
            .unwrap();
        for user in [&owner, &outsider] {
            sqlx::query("INSERT INTO user_feature_grants(user_id,feature,capability) VALUES(?,'connections','edit')").bind(&user.id).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO connections(id,workspace_id,name,kind,params_json,created_by,created_at) VALUES('ssh',?,'Office','ssh','{}',?,'2026-01-01T00:00:00Z')").bind(&workspace.id).bind(&owner.id).execute(&pool).await.unwrap();
        let profiles = NetworkProfilesRepo::new(pool.clone());
        let profile=profiles.create(&workspace.id,&owner.id,serde_json::from_value(serde_json::json!({"name":"Office","ssh_connection_id":"ssh","endpoints":[{"name":"db","remote_host":"db.internal","remote_port":5432}]})).unwrap()).await.unwrap();
        let sessions = otto_state::SessionsRepo::new(pool.clone());
        let session = sessions
            .create(otto_state::sessions::NewSession {
                workspace_id: workspace.id.clone(),
                kind: SessionKind::Agent,
                provider: "shell".into(),
                title: "Fixture".into(),
                cwd: "/tmp".into(),
                provider_session_id: None,
                connection_id: None,
                created_by: owner.id.clone(),
                meta: serde_json::json!({"network_profile_id":profile.id}),
            })
            .await
            .unwrap();
        let (events, _) = tokio::sync::broadcast::channel(16);
        let ctx = NetworkCtx {
            pool: pool.clone(),
            manager: std::sync::Arc::new(otto_sessions::SessionManager::new(
                sessions,
                events,
                otto_sessions::ProviderRegistry::new(None),
            )),
            roles: std::sync::Arc::new(otto_rbac::RbacRoleChecker::new(pool.clone())),
        };
        let request = |user: User| {
            let app = Router::new()
                .route("/sessions/{id}/network", get(status))
                .with_state(ctx.clone())
                .layer(Extension(AuthUser(user)));
            let req = Request::builder()
                .uri(format!("/sessions/{}/network", session.id))
                .body(Body::empty())
                .unwrap();
            async move { app.oneshot(req).await.unwrap().status() }
        };
        assert_eq!(request(outsider).await, StatusCode::FORBIDDEN);
        // Missing explicit resource allow must still deny the owner.
        assert_eq!(request(owner.clone()).await, StatusCode::FORBIDDEN);
        let policies = otto_state::ResourceAccessRepo::new(pool.clone());
        let mut policy = policies
            .get_policy(otto_core::access::ResourceKind::Connection, &"ssh".into())
            .await
            .unwrap();
        policy.mode = otto_core::access::AccessMode::Legacy;
        policies
            .put_policy(
                &policy,
                policy.revision,
                &otto_core::access::AccessActor {
                    real_user_id: owner.id.clone(),
                    effective_user_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(request(owner.clone()).await, StatusCode::OK);
        sqlx::query("DELETE FROM user_feature_grants WHERE user_id=?")
            .bind(&owner.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(request(owner).await, StatusCode::FORBIDDEN);
    }

    #[test]
    fn network_profile_and_direct_status_routes_keep_connections_permissions() {
        use crate::policy::{policy_for, PolicyDecision};
        use axum::http::Method;
        for path in [
            "/api/v1/workspaces/{id}/network-profiles",
            "/api/v1/network-profiles/{id}",
            "/api/v1/sessions/{id}/network",
        ] {
            assert!(matches!(
                policy_for(&Method::GET, path),
                PolicyDecision::Require(Feature::Connections, Capability::View)
            ));
        }
        assert!(matches!(
            policy_for(&Method::PUT, "/api/v1/network-profiles/{id}"),
            PolicyDecision::Require(Feature::Connections, Capability::Edit)
        ));
    }
}
