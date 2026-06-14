//! Core REST routes (contract endpoints #1-16, #57-58).

pub mod api_client;
pub mod auth_routes;
pub mod fs;
pub mod logs;
pub mod meta;
pub mod notifications;
pub mod onboarding;
pub mod settings;
pub mod users;
pub mod workspaces;

use axum::routing::{delete, get, patch, post};
use axum::Router;

use crate::state::ServerCtx;

/// Routes reachable without authentication.
pub fn public_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/health", get(meta::health))
        .route("/meta", get(meta::meta))
        .route("/onboarding/root", post(onboarding::onboard_root))
        .route("/auth/login", post(auth_routes::login))
}

/// Routes that require a bearer token (the auth middleware is layered on top
/// of this router, together with any api_extras, by `build_router`).
pub fn protected_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/auth/logout", post(auth_routes::logout))
        .route("/auth/me", get(auth_routes::me))
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", patch(users::update).delete(users::remove))
        .route(
            "/workspaces",
            get(workspaces::list).post(workspaces::create),
        )
        .route(
            "/workspaces/{id}",
            patch(workspaces::update).delete(workspaces::archive),
        )
        .route(
            "/workspaces/{id}/members",
            get(workspaces::members).put(workspaces::set_members),
        )
        .route("/settings", get(settings::get_all).put(settings::put_all))
        .route(
            "/notifications",
            get(notifications::list).delete(notifications::clear),
        )
        .route(
            "/notifications/settings",
            get(notifications::get_settings).put(notifications::put_settings),
        )
        .route(
            "/notifications/read-all",
            post(notifications::mark_all_read),
        )
        .route(
            "/notifications/{id}/read",
            post(notifications::mark_read),
        )
        .route("/notifications/{id}", delete(notifications::dismiss))
        .route("/fs/browse", get(fs::browse))
        .route("/fs/read", get(fs::read_file))
        .route("/logs/daemon", get(logs::daemon_logs))
        // --- API client ("Postman") -------------------------------------
        .route(
            "/workspaces/{wid}/api-client/collections",
            get(api_client::list_collections).post(api_client::create_collection),
        )
        .route(
            "/workspaces/{wid}/api-client/collections/{id}",
            patch(api_client::update_collection).delete(api_client::delete_collection),
        )
        .route(
            "/workspaces/{wid}/api-client/collections/{id}/openapi",
            get(api_client::export_openapi),
        )
        .route(
            "/workspaces/{wid}/api-client/requests",
            get(api_client::list_requests).post(api_client::create_request),
        )
        .route(
            "/workspaces/{wid}/api-client/requests/{id}",
            get(api_client::get_request)
                .patch(api_client::update_request)
                .delete(api_client::delete_request),
        )
        .route(
            "/workspaces/{wid}/api-client/environments",
            get(api_client::list_environments).post(api_client::create_environment),
        )
        .route(
            "/workspaces/{wid}/api-client/environments/{id}",
            patch(api_client::update_environment).delete(api_client::delete_environment),
        )
        .route(
            "/workspaces/{wid}/api-client/environments/{id}/activate",
            post(api_client::activate_environment),
        )
        .route(
            "/workspaces/{wid}/api-client/history",
            get(api_client::list_history).delete(api_client::clear_history),
        )
        .route(
            "/workspaces/{wid}/api-client/execute",
            post(api_client::execute),
        )
        .route(
            "/workspaces/{wid}/api-client/automations",
            get(api_client::list_automations).post(api_client::create_automation),
        )
        .route(
            "/workspaces/{wid}/api-client/automations/{id}",
            patch(api_client::update_automation).delete(api_client::delete_automation),
        )
        .route(
            "/workspaces/{wid}/api-client/automations/{id}/run",
            post(api_client::run_automation),
        )
        .route("/api-client/import-curl", post(api_client::import_curl))
}
