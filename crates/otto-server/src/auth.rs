//! Bearer-auth middleware + `CurrentUser` extractor + role helpers.

use axum::extract::{FromRequestParts, Request, State};
use axum::http::header;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use otto_core::auth::AuthUser;
use otto_core::domain::{Session, User, WorkspaceRole};
use otto_core::{Error, Id};

use crate::error::ApiError;
use crate::state::ServerCtx;

/// Raw bearer token of the current request, inserted by the middleware so
/// logout can revoke it.
#[derive(Debug, Clone)]
pub struct BearerToken(pub String);

/// Middleware: validate `Authorization: Bearer <token>` and insert
/// [`AuthUser`] (and [`BearerToken`]) into request extensions. Applied to
/// every `/api/v1` route except the public exemptions (health, meta,
/// onboarding/root, auth/login).
pub async fn auth_middleware(
    State(ctx): State<ServerCtx>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(str::to_owned);

    let Some(token) = token else {
        return ApiError(Error::Unauthorized).into_response();
    };

    match ctx.authenticator.authenticate(&token).await {
        Ok(user) => {
            req.extensions_mut().insert(BearerToken(token));
            req.extensions_mut().insert(AuthUser(user));
            next.run(req).await
        }
        Err(e) => ApiError(e).into_response(),
    }
}

/// Extractor for the authenticated user (reads the [`AuthUser`] extension
/// inserted by [`auth_middleware`]); rejects with 401 when absent.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .map(|a| CurrentUser(a.0.clone()))
            .ok_or(ApiError(Error::Unauthorized))
    }
}

/// Require at least `min` role for `user` in workspace `ws_id` (root passes).
pub async fn require_ws_role(
    ctx: &ServerCtx,
    user: &User,
    ws_id: &Id,
    min: WorkspaceRole,
) -> Result<(), ApiError> {
    ctx.roles.check(user, ws_id, min).await.map_err(ApiError)
}

/// Require the global root role.
pub fn require_root(user: &User) -> Result<(), ApiError> {
    if user.is_root {
        Ok(())
    } else {
        Err(ApiError(Error::Forbidden("requires root".into())))
    }
}

/// Gate access to a session by ownership-or-admin.
///
/// Returns `Ok(())` when **any** of:
/// - `user.is_root`
/// - `session.created_by == user.id`  (the owner)
/// - the user holds [`WorkspaceRole::Admin`] in `session.workspace_id`
///
/// Returns `Forbidden` otherwise. Root is handled without a DB round-trip;
/// the admin check delegates to the existing `WorkspacesRepo::role_of` resolver
/// (which also short-circuits for root), keeping the bypass logic in one place.
pub async fn require_session_owner_or_admin(
    ctx: &ServerCtx,
    user: &User,
    session: &Session,
) -> Result<(), ApiError> {
    check_session_owner_or_admin(&ctx.workspaces, user, session).await
}

/// Inner implementation operating on a bare `WorkspacesRepo` so unit tests can
/// call it without constructing a full `ServerCtx`.
pub(crate) async fn check_session_owner_or_admin(
    workspaces: &otto_state::WorkspacesRepo,
    user: &User,
    session: &Session,
) -> Result<(), ApiError> {
    if user.is_root || session.created_by == user.id {
        return Ok(());
    }
    match workspaces
        .role_of(user, &session.workspace_id)
        .await
        .map_err(ApiError)?
    {
        Some(WorkspaceRole::Admin) => Ok(()),
        _ => Err(ApiError(Error::Forbidden(
            "not the session owner or a workspace admin".into(),
        ))),
    }
}

#[cfg(test)]
mod session_owner_tests {
    use chrono::Utc;
    use otto_core::domain::{Session, SessionKind, SessionStatus, User};
    use otto_state::WorkspacesRepo;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use sqlx::SqlitePool;

    // ---- helpers -----------------------------------------------------------

    async fn mem_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("in-memory sqlite");
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    fn make_user(id: &str, is_root: bool) -> User {
        User {
            id: id.into(),
            username: id.into(),
            display_name: id.into(),
            is_root,
            disabled: false,
            created_at: Utc::now(),
        }
    }

    fn make_session(session_id: &str, workspace_id: &str, created_by: &str) -> Session {
        Session {
            id: session_id.into(),
            workspace_id: workspace_id.into(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "test".into(),
            status: SessionStatus::Running,
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: created_by.into(),
            created_at: Utc::now(),
            last_active_at: Utc::now(),
            archived: false,
            meta: serde_json::Value::Null,
        }
    }

    async fn seed_user(pool: &SqlitePool, id: &str, is_root: bool) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, 'x', ?, ?, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(id)
        .bind(is_root as i64)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed user");
    }

    async fn seed_workspace(pool: &SqlitePool, ws_id: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
             VALUES (?, 'ws', '/tmp', '{}', 0, ?)",
        )
        .bind(ws_id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed workspace");
    }

    async fn set_member(pool: &SqlitePool, ws_id: &str, user_id: &str, role: &str) {
        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)",
        )
        .bind(ws_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await
        .expect("set member");
    }

    // ---- tests -------------------------------------------------------------

    /// The session owner is allowed regardless of workspace role.
    #[tokio::test]
    async fn owner_is_allowed() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice", false).await;
        seed_workspace(&pool, "ws1").await;
        set_member(&pool, "ws1", "alice", "viewer").await; // only viewer

        let alice = make_user("alice", false);
        let session = make_session("s1", "ws1", "alice"); // alice owns it
        let repo = WorkspacesRepo::new(pool.clone());

        let result = super::check_session_owner_or_admin(&repo, &alice, &session).await;
        assert!(result.is_ok(), "owner must be allowed: {result:?}");
    }

    /// A workspace Editor who is NOT the owner is denied.
    #[tokio::test]
    async fn non_owner_editor_is_forbidden() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice", false).await;
        seed_user(&pool, "bob", false).await;
        seed_workspace(&pool, "ws1").await;
        set_member(&pool, "ws1", "alice", "editor").await;
        set_member(&pool, "ws1", "bob", "editor").await;

        let alice = make_user("alice", false);
        let session = make_session("s1", "ws1", "bob"); // bob owns it
        let repo = WorkspacesRepo::new(pool.clone());

        let result = super::check_session_owner_or_admin(&repo, &alice, &session).await;
        assert!(result.is_err(), "editor non-owner must be denied");
        assert!(
            matches!(result.unwrap_err().0, otto_core::Error::Forbidden(_)),
            "must be Forbidden"
        );
    }

    /// A workspace Admin who is NOT the owner is allowed.
    #[tokio::test]
    async fn workspace_admin_non_owner_is_allowed() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice", false).await;
        seed_user(&pool, "bob", false).await;
        seed_workspace(&pool, "ws1").await;
        set_member(&pool, "ws1", "alice", "admin").await; // alice is ws-admin
        set_member(&pool, "ws1", "bob", "editor").await;

        let alice = make_user("alice", false);
        let session = make_session("s1", "ws1", "bob"); // bob owns it
        let repo = WorkspacesRepo::new(pool.clone());

        let result = super::check_session_owner_or_admin(&repo, &alice, &session).await;
        assert!(result.is_ok(), "workspace admin must be allowed: {result:?}");
    }

    /// Root is always allowed, with no DB round-trip needed.
    #[tokio::test]
    async fn root_is_always_allowed() {
        let pool = mem_pool().await;
        // No rows seeded at all — root must not need them.
        let root = make_user("root", true);
        let session = make_session("s1", "ws1", "someone-else");
        let repo = WorkspacesRepo::new(pool.clone());

        let result = super::check_session_owner_or_admin(&repo, &root, &session).await;
        assert!(result.is_ok(), "root must always be allowed: {result:?}");
    }
}
