//! otto-server — axum composition root: core REST routes, auth middleware,
//! events WebSocket and SPA serving. Module routers from otto-sessions /
//! otto-connections / otto-git are mounted via `build_router`'s extras at
//! integration time.

pub mod agent_refs;
pub use otto_agent_run::agent_run;
pub mod agent_session;
pub mod agent_tasks_nudge;
pub mod api_helpers;
pub mod api_scripts;
pub mod api_secrets;
pub mod assistant;
pub mod auth;
pub mod browser_login_throttle;
pub mod cadence;
pub mod cancel_signal;
pub mod canvas_assist;
pub mod canvas_refs;
pub mod cli_update;
pub mod context_packet;
pub mod database_changes;
pub mod db_assist;
pub mod db_drafter;
pub mod design_assist;
pub mod design_blender;
pub mod design_format;
pub mod design_hall;
pub mod design_scene3d;
pub mod error;
pub mod eval_lab_routes;
pub mod eval_score;
pub mod feature_guard;
pub mod finding_agent;
pub mod finding_context;
pub mod goal_loop;
mod goal_loop_commands;
pub mod goal_loop_parse;
mod goal_loop_policy;
mod goal_loop_roles;
pub mod goal_loop_workspace;
pub mod history_index;
pub mod host_guard;
pub mod improve_channels;
pub mod insights;
pub mod k8s_monitor_scheduler;
pub mod live_events;
pub mod login_throttle;
pub mod lsp;
pub mod mcp_auto_approve;
pub mod mcp_capabilities;
pub mod mcp_http;
pub mod mcp_outward;
pub mod memory_gov;
pub mod mockup_assist;
pub mod model_catalog;
pub mod modules;
pub mod monitor;
pub use otto_agent_run::offload;
mod personal_agent_documents;
// Personal agents: tool-layer permission policy + live activity (batch 2026-10-03).
pub mod personal_agent_activity;
pub mod personal_agent_memory;
pub mod personal_agent_policy;
pub mod personal_agents_engine;
pub mod personal_agents_scheduler;
pub mod plugins;
pub mod policy;
pub mod product_chat;
pub mod product_media;
pub mod product_refine;
pub mod product_run;
pub mod product_swarm;
pub mod product_watcher;
pub mod proof;
pub mod provider_resolve;
pub mod repo_directory;
pub mod report_delivery;
pub mod resource_sessions;
pub use otto_review::fallback as review_fallback;
pub use otto_review::session as review_session;
use otto_review::summarizer as review_summarizer;
pub mod rooms;
pub mod routes;
pub mod run_callback;
pub mod run_channels;
pub mod run_context;
pub mod run_engine;
pub mod run_notices;
pub mod run_scheduler;
pub mod run_service;
pub mod run_sources;
pub mod run_workspace;
pub mod scheduled_tasks_engine;
pub mod scheduled_tasks_scheduler;
mod self_call;
pub mod shutdown;
pub mod skill_eval;
pub mod skill_review;
pub mod spa;
pub mod state;
pub mod swarm_agent_run;
pub mod swarm_channels;
pub mod swarm_merge;
pub mod swarm_run;
pub mod swarm_runtime;
pub mod swarm_scheduler;
pub mod swarm_verify;
pub mod swarm_wake;
pub mod swarm_workspace;
pub mod telemetry;
#[cfg(any(test, feature = "test-util"))]
pub mod test_support;
pub mod transcript_cache;
pub mod transcript_tail;
pub mod transport;
pub use otto_agent_run::turn_oracle;
pub mod ui_bridge;
pub mod ui_commands;
pub mod vault_docs_agent;
pub mod workflow_chat;
mod workflow_checkpoint;
pub mod workflow_context;
pub mod workflow_engine;
pub mod workflow_prepare;
pub mod workflow_trigger_scheduler;
mod workflow_validation;
pub mod workgraph_projector;
pub mod ws_events;
pub mod ws_fanout;

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
pub use state::ServerCtx;
pub use workflow_trigger_scheduler::spawn_workflow_event_trigger_listener;

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
///   unembedded development placeholder. The daemon uses `build_router_with_assets`
///   to supply its SPA without making UI files server compilation inputs.
pub fn build_router(
    ctx: ServerCtx,
    api_extras: Vec<Router<ServerCtx>>,
    root_extras: Vec<Router>,
) -> Router {
    build_router_with_assets(ctx, api_extras, root_extras, None)
}

/// Compose the same router and middleware with optional binary-owned UI assets.
pub fn build_router_with_assets(
    ctx: ServerCtx,
    api_extras: Vec<Router<ServerCtx>>,
    root_extras: Vec<Router>,
    assets: Option<spa::AssetLoader>,
) -> Router {
    let mut protected = routes::protected_routes().merge(rooms::protected_routes());
    for extra in api_extras {
        protected = protected.merge(extra);
    }
    // Three route_layers, applied bottom-up so the auth chokepoint runs FIRST and
    // the feature guard runs immediately after it: `route_layer` calls wrap
    // outermost-last, so the guard (added first → inner) sees the `AuthUser`
    // extension the auth middleware (added last → outer) inserts, and the
    // `MatchedPath` axum sets on the matched route. The guard adds the per-user
    // feature axis on top of the unchanged workspace-role gates in the handlers.
    //
    // The scratch guard sits between them, and deliberately AFTER the api_extras
    // merge above: the implicit Editor every user holds on the `scratch`
    // workspace must not reach the workspace-scoped families the module routers
    // mount either (connections, db, vault, memory, swarm, …), so it has to wrap
    // the whole protected router, not just the core routes.
    let protected = protected
        .route_layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            feature_guard::feature_guard::<ServerCtx>,
        ))
        .route_layer(axum::middleware::from_fn(routes::scratch_guard))
        .route_layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            auth::auth_middleware,
        ));

    let api = routes::public_routes()
        .merge(rooms::public_routes())
        .merge(protected);
    let events_tx = ctx.events.clone();
    let host_guard_state = host_guard::HostGuardState::new(ctx.pool.clone());
    let telemetry_ctx = ctx.telemetry.clone();

    let mut app = Router::new()
        .nest("/api/v1", api)
        .route("/ws/events", get(ws_events::events_ws))
        .merge(rooms::ws_routes())
        .with_state(ctx)
        .fallback(move |uri| spa::spa_fallback_with_assets(uri, assets));

    for extra in root_extras {
        app = app.merge(extra);
    }

    // Access-affecting writes broadcast `resource_access_changed` (the UI's
    // access cache refreshes on change instead of polling; live_events.rs).
    app.layer(axum::middleware::from_fn_with_state(
        events_tx,
        live_events::notify_access_changes,
    ))
    .layer(axum::middleware::from_fn_with_state(
        telemetry_ctx,
        telemetry::middleware,
    ))
    .layer(TraceLayer::new_for_http())
    // DNS-rebinding guard: refuse a `Host` we don't serve (host_guard.rs).
    // Inside CORS so a refused request still gets no CORS grant.
    .layer(axum::middleware::from_fn_with_state(
        host_guard_state,
        host_guard::host_guard_with_settings,
    ))
    .layer(cors_layer())
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
        .allow_origin(AllowOrigin::predicate(|origin: &HeaderValue, parts| {
            let request_host = parts
                .headers
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .map(host_guard::host_of);
            origin
                .to_str()
                .map(|o| is_allowed_origin_for(o, request_host.as_deref()))
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
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            // Agent UI control: the result/progress POSTs name the Otto window
            // (event-socket connection) the command was sent to.
            header::HeaderName::from_static("x-otto-ui-conn"),
            // Conditional list reads (k8s resources: an unchanged list is a 304).
            header::IF_NONE_MATCH,
            header::HeaderName::from_static("traceparent"),
        ])
        .expose_headers([
            header::ETAG,
            header::HeaderName::from_static("traceparent"),
            header::HeaderName::from_static(telemetry::ROUTE_HEADER),
        ])
        // Every call carries `Authorization`, so every call is preflighted.
        // Without a max-age WebKit caches a preflight ~5 s (per URL), so a
        // poller paid an extra OPTIONS round-trip on almost every tick. 600 s
        // is WebKit's (and Chromium's 7200 s) accepted ceiling.
        .max_age(CORS_PREFLIGHT_MAX_AGE)
}

/// How long a browser may reuse a CORS preflight (`Access-Control-Max-Age`).
const CORS_PREFLIGHT_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(600);

/// Whether a request `Origin` is trusted by [`cors_layer`], given the request's
/// own `Host` (port-stripped).
///
/// Loopback + Tauri origins are always trusted (the desktop app, `vite` in dev).
/// A private-LAN (RFC-1918) or Tailscale (`*.ts.net`) origin is trusted only
/// when it names the SAME host the request was sent to — i.e. a page this
/// machine serves on another port (vite `--host`, the network listener). It
/// used to be any LAN/tailnet origin, which let any other device's web page
/// (a router admin UI, a coworker's dev server) make CORS calls into the
/// daemon. `*.ts.net` additionally honours the `OTTO_ALLOWED_HOSTS` pin.
fn is_allowed_origin_for(origin: &str, request_host: Option<&str>) -> bool {
    if is_allowed_origin(origin) {
        return true;
    }
    let Some(req_host) = request_host else {
        return false;
    };
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let host = host_guard::host_of(rest);
    host == req_host && (is_private_lan_host(&host) || host_guard::tailscale_allowed(&host))
}

/// Origins trusted regardless of the request's `Host`: the Tauri webview
/// origins and loopback (any port). Everything else (LAN, tailnet, arbitrary
/// public web origins) goes through [`is_allowed_origin_for`].
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

    host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host.ends_with(".localhost")
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

pub mod state_archive;

/// Private-process entry point used by `ottod room-ocr`; never starts a server.
pub fn run_room_ocr_helper() -> bool {
    rooms::recap_engines::run_ocr_stdio()
}

#[cfg(test)]
mod cors_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn preflight_is_cacheable_for_allowed_origins_only() {
        // A fallback, not a `.route(…)`: the preflight is answered by the CORS
        // layer itself, and route_inventory/policy_coverage scan `.route(` literals.
        let app = Router::new()
            .fallback(|| async { "ok" })
            .layer(cors_layer());
        let preflight = |origin: &'static str| {
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/v1/health")
                .header(header::ORIGIN, origin)
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                .body(Body::empty())
                .unwrap()
        };

        let ok = app
            .clone()
            .oneshot(preflight("tauri://localhost"))
            .await
            .unwrap();
        let h = ok.headers();
        assert_eq!(h[header::ACCESS_CONTROL_MAX_AGE], "600");
        assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "tauri://localhost");

        // A LAN / tailnet origin is trusted only for its OWN host.
        let lan_preflight = |origin: &'static str, host: &'static str| {
            let mut r = preflight(origin);
            r.headers_mut()
                .insert(header::HOST, HeaderValue::from_static(host));
            r
        };
        let same = app
            .clone()
            .oneshot(lan_preflight(
                "https://192.168.1.20:5173",
                "192.168.1.20:7443",
            ))
            .await
            .unwrap();
        assert_eq!(
            same.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://192.168.1.20:5173"
        );
        let other_device = app
            .clone()
            .oneshot(lan_preflight("http://192.168.1.1", "192.168.1.20:7443"))
            .await
            .unwrap();
        assert!(other_device
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());

        let denied = app
            .oneshot(preflight("https://evil.example"))
            .await
            .unwrap();
        assert!(denied
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }
}
