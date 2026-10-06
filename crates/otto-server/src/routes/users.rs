//! Endpoints #7-10: users CRUD (root only).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{CreateUserReq, UpdateUserReq};
use otto_core::domain::User;
use otto_core::{Error, Id};
use otto_rbac::AuthRepo;
use otto_state::UsersRepo;

use crate::auth::{require_root, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// `GET /api/v1/users`
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<User>>> {
    require_root(&user)?;
    Ok(Json(UsersRepo::new(ctx.pool.clone()).list().await?))
}

/// `POST /api/v1/users` — 409 on duplicate username.
pub async fn create(
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateUserReq>,
) -> ApiResult<Json<User>> {
    require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    if req.username.trim().is_empty() {
        return Err(Error::Invalid("username must not be empty".into()).into());
    }
    // Enforce the shared minimum-password policy (same rule as onboarding).
    otto_rbac::validate_password(&req.password)?;
    let hash = otto_rbac::hash_password(&req.password)?;
    let display_name = req.display_name.unwrap_or_else(|| req.username.clone());
    let created = UsersRepo::new(ctx.pool.clone())
        .create(req.username.trim(), &hash, &display_name, false)
        .await?;
    Ok(Json(created))
}

/// `PATCH /api/v1/users/{id}`
pub async fn update(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateUserReq>,
) -> ApiResult<Json<User>> {
    require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    let repo = UsersRepo::new(ctx.pool.clone());
    let target = repo.get(&id).await?;
    if target.is_root && req.disabled == Some(true) {
        return Err(Error::Invalid("the root user cannot be disabled".into()).into());
    }
    let password_hash = match &req.password {
        // Enforce the shared minimum-password policy on any password change.
        Some(p) => {
            otto_rbac::validate_password(p)?;
            if id == user.id {
                verify_current_password(&repo, &target.username, req.current_password.as_deref())
                    .await?;
            }
            Some(otto_rbac::hash_password(p)?)
        }
        None => None,
    };
    let updated = repo
        .update(
            &id,
            req.display_name.as_deref(),
            password_hash.as_deref(),
            req.disabled,
        )
        .await?;

    // A changed credential or a disabled account must invalidate every
    // outstanding token for this user (login sessions + API tokens), so a
    // leaked/old token can't keep working after the change.
    if password_hash.is_some() || req.disabled == Some(true) {
        AuthRepo::new(ctx.pool.clone())
            .revoke_all_for_user(&id)
            .await?;
    }

    Ok(Json(updated))
}

/// A caller changing its OWN password must prove the current one (S8-310):
/// otherwise a stolen UI token (XSS, a shared machine) turns into a
/// persistent account takeover. A wrong guess counts on the login throttle's
/// per-username key (so the token can't brute-force it either) and answers
/// 403 — never 401, which would sign the real user out.
async fn verify_current_password(
    repo: &UsersRepo,
    username: &str,
    current: Option<&str>,
) -> ApiResult<()> {
    let Some(current) = current.filter(|c| !c.is_empty()) else {
        return Err(Error::Invalid(
            "current_password is required to change your own password".into(),
        )
        .into());
    };
    let throttle = crate::login_throttle::global();
    let key = crate::login_throttle::username_key(username);
    if throttle.check_locked(&key).is_some() {
        return Err(Error::Forbidden(
            "too many wrong passwords for this account; try again later".into(),
        )
        .into());
    }
    let record = repo.get_by_username(username).await?;
    let ok = match otto_rbac::verify_password_bounded(current, &record.password_hash).await {
        Ok(r) => r?,
        Err(otto_rbac::VerifySaturated) => {
            return Err(Error::Conflict("too many password checks in flight; retry".into()).into())
        }
    };
    if !ok {
        throttle.record_failure_pinned(&key);
        return Err(Error::Forbidden("current password is incorrect".into()).into());
    }
    Ok(())
}

/// `DELETE /api/v1/users/{id}` — soft delete (disables); 400 for root user.
pub async fn remove(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    let repo = UsersRepo::new(ctx.pool.clone());
    let target = repo.get(&id).await?;
    if target.is_root {
        return Err(Error::Invalid("the root user cannot be disabled".into()).into());
    }
    repo.update(&id, None, None, Some(true)).await?;
    // Disabling the account invalidates all of its outstanding tokens.
    AuthRepo::new(ctx.pool.clone())
        .revoke_all_for_user(&id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::CurrentAuthContext;
    use otto_core::auth::AuthContext;

    fn human(user: &User) -> CurrentAuthContext {
        CurrentAuthContext(AuthContext {
            real_user: user.clone(),
            effective_user: user.clone(),
            scope: None,
            mcp_only: false,
            mcp_scope: None,
            mcp_internal: false,
            mcp_session_id: None,
            managed_session_id: None,
        })
    }

    fn pw() -> String {
        otto_core::new_id()
    }

    /// S8-310: changing your OWN password needs the current one; root
    /// resetting another user's password does not.
    #[tokio::test]
    async fn own_password_change_requires_the_current_password() {
        let pool = crate::test_support::mem_pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
        let current = pw();
        let repo = UsersRepo::new(pool.clone());
        let root = repo
            .create(
                "s8310root",
                &otto_rbac::hash_password(&current).unwrap(),
                "Root",
                true,
            )
            .await
            .unwrap();
        let other = repo.create("s8310bob", "x", "Bob", false).await.unwrap();
        let call = |id: &Id, current_password: Option<String>| {
            let ctx = ctx.clone();
            let root = root.clone();
            let id = id.clone();
            async move {
                update(
                    Path(id),
                    State(ctx),
                    human(&root),
                    CurrentUser(root.clone()),
                    Json(UpdateUserReq {
                        display_name: None,
                        password: Some(pw()),
                        disabled: None,
                        current_password,
                    }),
                )
                .await
                .map(|_| ())
                .map_err(|e| e.0)
            }
        };
        assert!(matches!(call(&root.id, None).await, Err(Error::Invalid(_))));
        assert!(matches!(
            call(&root.id, Some(pw())).await,
            Err(Error::Forbidden(_))
        ));
        assert!(call(&root.id, Some(current)).await.is_ok());
        // Another user's reset (root acting as admin) needs no current password.
        assert!(call(&other.id, None).await.is_ok());
    }
}
