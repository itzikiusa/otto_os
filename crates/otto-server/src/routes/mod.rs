//! Core REST routes (contract endpoints #1-16, #57-58).

pub mod activity;
pub mod api_client;
pub mod api_stream;
pub mod auth_routes;
pub mod grpc;
pub mod fs;
pub mod handover;
pub mod logs;
pub mod meta;
pub mod notifications;
pub mod onboarding;
pub mod settings;
pub mod usage;
pub mod users;
pub mod workflows;
pub mod workspaces;

use axum::routing::{delete, get, patch, post, put};
use axum::Router;

use crate::state::ServerCtx;

/// Routes reachable without authentication.
pub fn public_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/health", get(meta::health))
        .route("/meta", get(meta::meta))
        .route("/onboarding/root", post(onboarding::onboard_root))
        .route("/auth/login", post(auth_routes::login))
        // Provider activity ingest: gated by the per-session token Otto sets on
        // the agent PTY, not by a user bearer token (the agent's hooks have no
        // user session). Verified inside the handler.
        .route("/ingest/claude", post(activity::claude_ingest))
        .route("/ingest/codex", post(activity::codex_ingest))
        // Provider token-usage ingest (same per-session token gate as above).
        .route("/ingest/usage", post(usage::ingest))
}

/// Routes that require a bearer token (the auth middleware is layered on top
/// of this router, together with any api_extras, by `build_router`).
pub fn protected_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/auth/logout", post(auth_routes::logout))
        .route("/auth/me", get(auth_routes::me))
        // --- Agent activity (live trail + task tracker) ------------------
        .route(
            "/workspaces/{wid}/sessions/{sid}/trail",
            get(activity::list_trail).post(activity::append_trail),
        )
        .route(
            "/workspaces/{wid}/sessions/{sid}/tasks",
            get(activity::list_tasks).put(activity::put_tasks),
        )
        .route(
            "/workspaces/{wid}/activity/summary",
            get(activity::workspace_summary),
        )
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
        // --- Usage tracking & system metrics (embedded ClickHouse) -------
        .route("/usage/status", get(usage::status))
        .route("/usage/summary", get(usage::summary))
        .route("/usage/metrics", get(usage::metrics))
        .route("/usage/config", put(usage::put_config))
        .route("/usage/install", post(usage::install))
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
            "/workspaces/{wid}/api-client/grpc/describe",
            post(grpc::describe),
        )
        .route(
            "/workspaces/{wid}/api-client/grpc/invoke",
            post(grpc::invoke),
        )
        .route(
            "/workspaces/{wid}/api-client/grpc/reflect",
            post(grpc::reflect),
        )
        .route(
            "/workspaces/{wid}/api-client/oauth2/token",
            post(api_client::oauth2_token),
        )
        .route(
            "/workspaces/{wid}/api-client/cookies",
            get(api_client::list_cookies).delete(api_client::clear_cookies),
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
        .route(
            "/sessions/{id}/handover",
            post(handover::handover_session),
        )
        .route(
            "/sessions/{id}/handover/brief",
            post(handover::handover_brief),
        )
        // --- Workflow engine --------------------------------------------
        .route("/workflows/node-types", get(workflows::node_types))
        .route("/workflows/templates", get(workflows::list_templates))
        .route(
            "/workspaces/{wid}/workflows",
            get(workflows::list_workflows).post(workflows::create_workflow),
        )
        .route(
            "/workspaces/{wid}/workflows/from-template",
            post(workflows::create_from_template),
        )
        .route(
            "/workspaces/{wid}/workflows/generate",
            post(workflows::generate_workflow),
        )
        .route(
            "/workflows/{id}",
            get(workflows::get_workflow)
                .patch(workflows::update_workflow)
                .delete(workflows::delete_workflow),
        )
        .route("/workflows/{id}/run", post(workflows::run_workflow))
        .route("/workflows/{id}/runs", get(workflows::list_runs))
        .route("/workflow-runs/{id}", get(workflows::get_run))
        .route("/workflow-runs/{id}/cancel", post(workflows::cancel_run))
}
