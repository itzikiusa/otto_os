//! Bearer-auth middleware + `CurrentUser` extractor + role helper.

use axum::extract::{FromRequestParts, Request, State};
use axum::http::header;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use otto_core::auth::AuthUser;
use otto_core::domain::{User, WorkspaceRole};
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
