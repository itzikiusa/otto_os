//! The central feature-policy guard (RBAC Task 1.4).
//!
//! One Axum middleware, layered **immediately after** [`crate::auth::auth_middleware`]
//! on the protected routes, that enforces the *feature* authorization axis for
//! every request the auth chokepoint admits. It reads the route **template** from
//! the [`axum::extract::MatchedPath`] (e.g. `/api/v1/connections/{id}/db/query`)
//! plus the method, looks the pair up in [`crate::policy::policy_for`], and:
//!
//! - [`PolicyDecision::Exempt`] → passes the request through (public / token /
//!   self-owned / workspace-axis / catalog routes the feature axis doesn't gate);
//! - [`PolicyDecision::Require`]`(feature, cap)` → **allows iff** the caller is
//!   root **or** [`GrantsRepo::capability_of`] for that feature is `>= cap`,
//!   otherwise `403`;
//! - [`PolicyDecision::Deny`] → `403` (fail closed: any protected route with no
//!   policy entry is denied).
//!
//! This is an *additional* axis on top of the unchanged workspace-role /
//! ownership gates that stay in the handlers (`require_ws_role`, owner checks).
//! The guard never weakens those; `effective = min(feature_grant, ws_role)`. Root
//! bypasses the feature axis here (and `capability_of` independently returns
//! `Admin` for root), never depending on grant rows.
//!
//! Denials are rendered through the standard [`ApiError`] → JSON `Problem` path
//! (`{"code":"forbidden","message":...}`), matching every other handler.

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use otto_core::auth::AuthUser;
use otto_core::Error;
use otto_state::GrantsRepo;

use crate::error::ApiError;
use crate::policy::{policy_for, PolicyDecision};
use crate::state::ServerCtx;

/// App state that can hand the guard a [`GrantsRepo`].
///
/// Implemented for the production [`ServerCtx`] (constructs a repo from the
/// shared pool — a cheap handle clone) and, in tests, for a minimal state so the
/// guard can be exercised without assembling the full server context.
pub trait HasGrants {
    fn grants(&self) -> GrantsRepo;
}

impl HasGrants for ServerCtx {
    fn grants(&self) -> GrantsRepo {
        GrantsRepo::new(self.pool.clone())
    }
}

/// The feature-policy guard middleware.
///
/// Generic over the app state so the same code runs in production (`ServerCtx`)
/// and in the RBAC matrix tests (a minimal `HasGrants` state). Layer it with
/// `from_fn_with_state(state, feature_guard::<S>)` as a `route_layer` directly
/// after the auth middleware, so the [`AuthUser`] extension is present and the
/// [`MatchedPath`] is set for the matched route.
pub async fn feature_guard<S>(State(state): State<S>, req: Request, next: Next) -> Response
where
    S: HasGrants + Clone + Send + Sync + 'static,
{
    // The matched route template, including the `/api/v1` nest prefix the policy
    // table keys on. A request with no `MatchedPath` never matched a route in
    // this router (it would 404 below) — but if one somehow reaches the guard, we
    // fail closed.
    let Some(matched) = req.extensions().get::<MatchedPath>() else {
        return forbidden("no matched route").into_response();
    };
    let template = matched.as_str().to_string();
    let method = req.method().clone();

    match policy_for(&method, &template) {
        PolicyDecision::Exempt => next.run(req).await,
        PolicyDecision::Deny => forbidden("route not permitted").into_response(),
        PolicyDecision::Require(feature, needed) => {
            // The authenticated user, inserted by `auth_middleware`. Absent ⇒ the
            // guard was mounted outside the auth chokepoint; fail closed (401).
            let Some(AuthUser(user)) = req.extensions().get::<AuthUser>().cloned() else {
                return ApiError(Error::Unauthorized).into_response();
            };
            // Root bypasses the feature axis unconditionally (never depends on a
            // grant row). For everyone else, the granted capability must meet or
            // exceed the requirement.
            if user.is_root {
                return next.run(req).await;
            }
            match state.grants().capability_of(&user, feature).await {
                Ok(have) if have >= needed => next.run(req).await,
                Ok(_) => forbidden(&format!(
                    "requires {}:{}",
                    feature.as_str(),
                    needed.as_str()
                ))
                .into_response(),
                // A repo error here is an authorization failure → fail closed.
                Err(e) => ApiError(e).into_response(),
            }
        }
    }
}

/// Build a `403 Forbidden` `ApiError` (rendered as the JSON `Problem` body).
fn forbidden(reason: &str) -> ApiError {
    ApiError(Error::Forbidden(reason.to_string()))
}
