//! otto-server — axum composition root: core REST routes, auth middleware,
//! events WebSocket and SPA serving. Module routers from otto-sessions /
//! otto-connections / otto-git are mounted via `build_router`'s extras at
//! integration time.

pub mod api_helpers;
pub mod auth;
pub mod error;
pub mod lsp;
pub mod modules;
pub mod monitor;
pub mod review_session;
pub mod routes;
pub mod skill_eval;
pub mod spa;
pub mod state;
pub mod workflow_engine;
pub mod ws_events;

use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub use auth::{require_ws_role, CurrentUser};
pub use error::{ApiError, ApiResult};
pub use monitor::{
    spawn_metrics_sampler, spawn_session_event_listener, spawn_usage_recorder, AuthScanner,
    CredentialMonitor,
};
pub use state::ServerCtx;

/// Build the full daemon router.
///
/// - Core API routes plus every router in `api_extras` are nested under
///   `/api/v1` with the bearer-auth middleware applied (public exemptions:
///   `/health`, `/meta`, `/onboarding/root`, `/auth/login`). Extras' handlers
///   read the authenticated user from the `otto_core::auth::AuthUser` request
///   extension (or via the [`CurrentUser`] extractor).
/// - `root_extras` are merged at the root (terminal WS routers — they
///   self-authenticate via `?token=`).
/// - `/ws/events` is served here; unmatched non-API paths fall back to the
///   SPA (embedded behind the `embed-ui` feature, placeholder otherwise).
pub fn build_router(
    ctx: ServerCtx,
    api_extras: Vec<Router<ServerCtx>>,
    root_extras: Vec<Router>,
) -> Router {
    let mut protected = routes::protected_routes();
    for extra in api_extras {
        protected = protected.merge(extra);
    }
    let protected = protected.route_layer(axum::middleware::from_fn_with_state(
        ctx.clone(),
        auth::auth_middleware,
    ));

    let api = routes::public_routes().merge(protected);

    let mut app = Router::new()
        .nest("/api/v1", api)
        .route("/ws/events", get(ws_events::events_ws))
        .with_state(ctx)
        .fallback(spa::spa_fallback);

    for extra in root_extras {
        app = app.merge(extra);
    }

    app.layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}
