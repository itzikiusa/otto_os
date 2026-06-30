//! otto-server — axum composition root: core REST routes, auth middleware,
//! events WebSocket and SPA serving. Module routers from otto-sessions /
//! otto-connections / otto-git are mounted via `build_router`'s extras at
//! integration time.

pub mod agent_run;
pub mod agent_session;
pub mod api_helpers;
pub mod auth;
pub mod cadence;
pub mod canvas_assist;
pub mod db_assist;
pub mod mockup_assist;
pub mod context_packet;
pub mod db_drafter;
pub mod embedder;
pub mod vault_routes;
pub mod memory_gov;
pub mod cli_update;
pub mod error;
pub mod eval_lab_routes;
pub mod eval_score;
pub mod feature_guard;
pub mod finding_agent;
pub mod finding_context;
pub mod goal_loop;
pub mod goal_loop_parse;
pub mod goal_loop_workspace;
pub mod improve_channels;
pub mod insights;
pub mod login_throttle;
pub mod lsp;
pub mod mcp_capabilities;
pub mod mcp_http;
pub mod mcp_outward;
pub mod modules;
pub mod monitor;
pub mod plugins;
pub mod policy;
pub mod proof;
pub mod product_chat;
pub mod product_media;
pub mod product_refine;
pub mod product_run;
pub mod product_swarm;
pub mod product_watcher;
pub mod review_session;
pub mod routes;
pub mod run_callback;
pub mod run_channels;
pub mod run_context;
pub mod run_engine;
pub mod run_scheduler;
pub mod run_service;
pub mod run_sources;
pub mod run_workspace;
pub mod scheduled_tasks_engine;
pub mod scheduled_tasks_scheduler;
pub mod skill_eval;
pub mod spa;
pub mod state;
pub mod swarm_agent_run;
pub mod swarm_channels;
pub mod swarm_merge;
pub mod swarm_run;
pub mod swarm_runtime;
pub mod swarm_scheduler;
pub mod swarm_verify;
pub mod swarm_workspace;
pub mod workflow_chat;
pub mod workflow_engine;
pub mod workflow_trigger_scheduler;
pub mod workgraph_projector;
pub mod ws_events;

use axum::http::{header, HeaderValue, Method};
use axum::routing::get;
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;

pub use auth::{require_ws_role, CurrentUser};
pub use error::{ApiError, ApiResult};
pub use monitor::{
    spawn_budget_sampler, spawn_metrics_sampler, spawn_session_event_listener,
    spawn_usage_recorder, AuthScanner, CredentialMonitor,
};
pub use workflow_trigger_scheduler::spawn_workflow_event_trigger_listener;
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
    // Two route_layers, applied bottom-up so the auth chokepoint runs FIRST and
    // the feature guard runs immediately after it: `route_layer` calls wrap
    // outermost-last, so the guard (added first → inner) sees the `AuthUser`
    // extension the auth middleware (added second → outer) inserts, and the
    // `MatchedPath` axum sets on the matched route. The guard adds the per-user
    // feature axis on top of the unchanged workspace-role gates in the handlers.
    let protected = protected
        .route_layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            feature_guard::feature_guard::<ServerCtx>,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
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

    app.layer(TraceLayer::new_for_http()).layer(cors_layer())
}

/// CORS policy for the daemon.
///
/// Auth is a bearer token in the `Authorization` header (never a cookie), so
/// CORS is not the primary security boundary — but we still drop the previous
/// `CorsLayer::permissive()` (which echoed *any* origin) for a restricted
/// allowlist that rejects arbitrary public web origins while keeping every way
/// the UI actually reaches the daemon working:
///   - the Tauri native shell (`tauri://localhost`, `http://tauri.localhost`);
///   - same-origin / loopback (the SPA served by the daemon, and `vite` in dev);
///   - private LAN + Tailscale hosts (the remote/mobile access feature).
///
/// `allow_credentials` stays off (we don't use cookies), which keeps a
/// non-wildcard origin list valid. Methods/headers are pinned to what the API
/// and the SPA use.
fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, _parts| {
            origin
                .to_str()
                .map(is_allowed_origin)
                .unwrap_or(false)
        }))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
}

/// Whether a request `Origin` is trusted by [`cors_layer`].
///
/// Accepts the Tauri webview origins, loopback (any port), RFC-1918 private LAN
/// ranges, and Tailscale (`*.ts.net`) hosts — the surfaces the desktop app and
/// the remote/mobile access feature legitimately use. Everything else (arbitrary
/// public web origins) is rejected.
fn is_allowed_origin(origin: &str) -> bool {
    // Tauri native shell.
    if origin == "tauri://localhost" || origin == "http://tauri.localhost" {
        return true;
    }
    // Strip the scheme; only http(s) origins beyond this point.
    let rest = match origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    {
        Some(r) => r,
        None => return false,
    };
    // Host is everything before an optional `:port`. IPv6 literals are bracketed
    // (`[::1]:7700`); for those, key off the closing bracket.
    let host = if let Some(end) = rest.find(']') {
        &rest[..=end]
    } else {
        rest.split(':').next().unwrap_or(rest)
    };

    host == "localhost"
        || host == "127.0.0.1"
        || host == "[::1]"
        || host.ends_with(".localhost")
        || host.ends_with(".ts.net") // Tailscale
        || is_private_lan_host(host)
}

/// True for RFC-1918 private IPv4 hosts (`10.0.0.0/8`, `172.16.0.0/12`,
/// `192.168.0.0/16`) so the daemon is reachable from other devices on a home or
/// office LAN (the remote/mobile access feature).
fn is_private_lan_host(host: &str) -> bool {
    let Ok(ip) = host.parse::<std::net::Ipv4Addr>() else {
        return false;
    };
    let [a, b, ..] = ip.octets();
    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
}
