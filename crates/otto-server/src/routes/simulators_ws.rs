//! Root-mounted WebSocket routes for Mobile Simulators & Emulators:
//! - `WS /ws/simulators/{id}/stream` (live screencast binary frames + bidirectional input/control)
//! - `WS /ws/simulators/{id}/logs` (live device log stream)
//!
//! Auth matches `/ws/browser/{tab_id}/live`:
//! - Bearer ONLY via `Sec-WebSocket-Protocol: otto-bearer, <token>` (never `?token=`).
//! - Share-scoped, MCP-only, and read-only agent tokens are refused via `RootRoute::Simulators`.
//! - `Feature::Browser` `View` is required to watch/tail logs; `Edit` to drive input/actions.
//! - Grant is re-checked every 5s off the socket loop (revoked → close; narrowed → drop drive).

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use otto_core::domain::{Capability, Feature};
use otto_core::Error;
use otto_simulators::types::MAX_CLIENT_FRAME_BYTES;
use otto_simulators::ws::{
    serve_logs_socket, serve_stream_socket, GrantRecheckFn, LogsQuery, StreamQuery,
};
use otto_state::GrantsRepo;

use crate::error::ApiError;
use crate::state::ServerCtx;

/// Root-mounted simulator WebSocket routes (self-authenticating).
pub fn ws_router(ctx: ServerCtx) -> Router {
    Router::new()
        .route("/ws/simulators/{id}/stream", get(stream_ws))
        .route("/ws/simulators/{id}/logs", get(logs_ws))
        .with_state(ctx)
}

/// Shared 4-step auth + `RootRoute::Simulators` gate + `Feature::Browser` grant
/// check used by both initial WS upgrade and the 5s recheck task (`DUP-7`).
async fn check_sim_ws_access(ctx: &ServerCtx, token: &str) -> Result<bool, ApiError> {
    let Ok(auth) = ctx.authenticator.authenticate(token).await else {
        return Err(ApiError(Error::Unauthorized));
    };
    if let Err(e) = crate::feature_guard::root_route_gate(
        crate::feature_guard::RootRoute::Simulators,
        &auth,
        Some(&ctx.pool),
    )
    .await
    {
        return Err(ApiError(e));
    }
    let cap = GrantsRepo::new(ctx.pool.clone())
        .capability_of(&auth.effective_user, Feature::Browser)
        .await
        .unwrap_or(Capability::None);
    if cap < Capability::View {
        return Err(ApiError(Error::Forbidden("requires browser:view".into())));
    }
    Ok(cap >= Capability::Edit)
}

async fn authenticate_ws(ctx: &ServerCtx, headers: &HeaderMap) -> Result<(String, bool), ApiError> {
    let Some(token) = crate::ws_events::token_from_subprotocol(headers) else {
        return Err(ApiError(Error::Unauthorized));
    };
    let can_drive = check_sim_ws_access(ctx, &token).await?;
    Ok((token, can_drive))
}

fn make_recheck(ctx: ServerCtx, token: String) -> GrantRecheckFn {
    Box::new(move || {
        let ctx = ctx.clone();
        let token = token.clone();
        Box::pin(async move { check_sim_ws_access(&ctx, &token).await.ok() })
    })
}

/// `GET /ws/simulators/{id}/stream`
async fn stream_ws(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    Query(q): Query<StreamQuery>,
    headers: HeaderMap,
    State(ctx): State<ServerCtx>,
) -> Response {
    let (token, can_drive) = match authenticate_ws(&ctx, &headers).await {
        Ok(ok) => ok,
        Err(err) => return err.into_response(),
    };
    let fps = q.fps.unwrap_or(2).clamp(1, 10);
    let format = q.format;
    let service = ctx.simulators.clone();
    let recheck = make_recheck(ctx, token);
    ws.max_message_size(MAX_CLIENT_FRAME_BYTES)
        .max_frame_size(MAX_CLIENT_FRAME_BYTES)
        .protocols([crate::ws_events::BEARER_SUBPROTOCOL])
        .on_upgrade(move |socket| {
            serve_stream_socket(socket, service, id, fps, format, can_drive, Some(recheck))
        })
}

/// `GET /ws/simulators/{id}/logs`
async fn logs_ws(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
    headers: HeaderMap,
    State(ctx): State<ServerCtx>,
) -> Response {
    let (token, _can_drive) = match authenticate_ws(&ctx, &headers).await {
        Ok(ok) => ok,
        Err(err) => return err.into_response(),
    };
    // INPUT-5: reject unknown non-empty level or oversized filter with 400 Bad Request.
    let (filter, min_level) = match q.validate() {
        Ok(ok) => ok,
        Err(err) => return ApiError(err).into_response(),
    };
    let service = ctx.simulators.clone();
    let recheck = make_recheck(ctx, token);
    ws.max_message_size(MAX_CLIENT_FRAME_BYTES)
        .max_frame_size(MAX_CLIENT_FRAME_BYTES)
        .protocols([crate::ws_events::BEARER_SUBPROTOCOL])
        .on_upgrade(move |socket| {
            serve_logs_socket(socket, service, id, filter, min_level, Some(recheck))
        })
}
