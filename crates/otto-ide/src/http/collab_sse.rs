//! HTTP handlers for Share-with-Friends Collaboration (`/ide/collab/*`),
//! Runtime IDE Plugins (`/ide/plugins/*`), and SSE event framing helpers.

use std::convert::Infallible;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue};
use axum::response::Response;
use axum::{Extension, Json};
use bytes::Bytes;
use futures_util::stream;
use otto_core::auth::AuthUser;
use otto_core::{Error, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::{self, error::RecvError};

use super::ApiResult;
use crate::types::{
    IdeCollabCursorReq, IdeCollabDebugActionReq, IdeCollabEditReq, IdeCollabEditResp,
    IdeCollabEvent, IdeCollabJoinReq, IdeCollabJoinResp, IdeCollabRoleReq, IdeCollabRoom,
    IdeCollabRoomResp, IdeCollabRoomsResp, IdeCollabStateResp, IdeCreateCollabRoomReq,
    IdeCreateCollabRoomResp, IdeDebugAction, IdeDebugCommandReq, IdeDebugSessionResp,
    IdeDebugSessionSnapshot, IdeInstallPluginReq, IdeOkResp, IdePluginResp, IdePluginsResp,
    IdeSetBreakpointsReq, IdeTargetKind, IdeTogglePluginReq,
};
use crate::IdeCtx;

const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// E. "Share with Friends" Host Routes
// ---------------------------------------------------------------------------

pub(super) async fn create_collab_room<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeCreateCollabRoomReq>,
) -> ApiResult<Json<IdeCreateCollabRoomResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let resp = ctx.ide().collab.create_room(&user, req)?;
    Ok(Json(resp))
}

pub(super) async fn list_collab_rooms<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<IdeCollabRoomsResp>> {
    let rooms = ctx.ide().collab.list_rooms(&user)?;
    Ok(Json(IdeCollabRoomsResp { rooms }))
}

pub(super) async fn set_collab_role<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(room_id): Path<String>,
    Json(req): Json<IdeCollabRoleReq>,
) -> ApiResult<Json<IdeCollabRoomResp>> {
    let room = ctx
        .ide()
        .collab
        .set_peer_role(&user, &room_id, &req.peer_id, req.role)?;
    Ok(Json(IdeCollabRoomResp { room }))
}

pub(super) async fn close_collab_room<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(room_id): Path<String>,
) -> ApiResult<Json<IdeOkResp>> {
    ctx.ide().collab.close_room(&user, &room_id)?;
    Ok(Json(IdeOkResp { ok: true }))
}

// ---------------------------------------------------------------------------
// F. Runtime IDE Plugins Routes
// ---------------------------------------------------------------------------

pub(super) async fn list_plugins<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Json<IdePluginsResp>> {
    let (plugins, catalog) = ctx.ide().plugins.list();
    Ok(Json(IdePluginsResp { plugins, catalog }))
}

pub(super) async fn install_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Json(req): Json<IdeInstallPluginReq>,
) -> ApiResult<Json<IdePluginResp>> {
    let plugin = ctx.ide().plugins.install(&req.source).await?;
    Ok(Json(IdePluginResp { plugin }))
}

pub(super) async fn toggle_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(slug): Path<String>,
    Json(req): Json<IdeTogglePluginReq>,
) -> ApiResult<Json<IdePluginResp>> {
    let plugin = ctx.ide().plugins.toggle(&slug, req.enabled)?;
    Ok(Json(IdePluginResp { plugin }))
}

pub(super) async fn uninstall_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(slug): Path<String>,
) -> ApiResult<Json<IdeOkResp>> {
    ctx.ide().plugins.uninstall(&slug)?;
    Ok(Json(IdeOkResp { ok: true }))
}

// ---------------------------------------------------------------------------
// Public Capability-Token Guest Collaboration Routes
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub(super) struct CollabGuestQuery {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    peer_id: Option<String>,
}

pub(super) fn extract_collab_token(
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> Result<String> {
    if let Some(tok) = query_token.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(tok.to_string());
    }
    if let Some(hv) = headers
        .get("x-ide-collab-token")
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = hv.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(tok) = auth
            .strip_prefix("Bearer ")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(tok.to_string());
        }
    }
    Err(Error::Unauthorized)
}

pub(super) async fn collab_guest_join<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Json(mut req): Json<IdeCollabJoinReq>,
) -> ApiResult<Json<IdeCollabJoinResp>> {
    if req.token.trim().is_empty() {
        req.token = extract_collab_token(&headers, None)?;
    }
    let resp = ctx.ide().collab.join_room(&room_id, req)?;
    Ok(Json(resp))
}

pub(super) async fn collab_guest_state<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<CollabGuestQuery>,
) -> ApiResult<Json<IdeCollabStateResp>> {
    let token = extract_collab_token(&headers, q.token.as_deref())?;
    let mut state = ctx.ide().collab.get_state(&room_id, &token)?;
    if let Some(debug_id) = state.room.active_debug_id.as_deref() {
        if let Ok(fresh) = ctx.ide().dap.get_session(debug_id) {
            state.active_debug = Some(fresh);
        }
    }
    Ok(Json(state))
}

pub(super) async fn collab_guest_events<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<CollabGuestQuery>,
) -> ApiResult<Response> {
    let _ = &q.peer_id;
    let token = extract_collab_token(&headers, q.token.as_deref())?;
    let (room, rx) = ctx.ide().collab.subscribe(&room_id, &token)?;
    Ok(build_collab_sse_response(room, rx))
}

pub(super) async fn collab_guest_cursor<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    Json(req): Json<IdeCollabCursorReq>,
) -> ApiResult<Json<IdeOkResp>> {
    ctx.ide().collab.update_cursor(&room_id, req)?;
    Ok(Json(IdeOkResp { ok: true }))
}

pub(super) async fn collab_guest_edit<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    Json(req): Json<IdeCollabEditReq>,
) -> ApiResult<Json<IdeCollabEditResp>> {
    let (target, _host_user_id) =
        ctx.ide()
            .collab
            .authorize_editor(&room_id, &req.peer_id, &req.peer_secret)?;
    let mut saved_to_target = false;
    if req.save && target.kind == IdeTargetKind::Local {
        let resolved = ctx.ide().targets.resolve_local(&target, false)?;
        ctx.ide()
            .targets
            .write_file(&resolved, &req.path, &req.content, None, false)
            .await?;
        saved_to_target = true;
    }
    let version = ctx
        .ide()
        .collab
        .apply_edit(&room_id, &req, saved_to_target)?;
    Ok(Json(IdeCollabEditResp { version }))
}

pub(super) async fn collab_guest_debug_action<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    Json(req): Json<IdeCollabDebugActionReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    let (target, _host_user_id) = ctx.ide().collab.authorize_debug_action(&room_id, &req)?;

    let mut latest_snap = None;
    if let Some(breakpoints) = req.breakpoints.clone() {
        let updated = ctx
            .ide()
            .dap
            .set_breakpoints(
                &req.debug_id,
                IdeSetBreakpointsReq {
                    breakpoints,
                    break_on_exceptions: false,
                },
            )
            .await?;
        latest_snap = Some(updated);
    }

    let skip_cmd = latest_snap.is_some()
        && req.action == IdeDebugAction::SelectFrame
        && req.frame_id.is_none();
    let session = if skip_cmd {
        latest_snap.unwrap()
    } else {
        ctx.ide()
            .dap
            .command(
                &req.debug_id,
                IdeDebugCommandReq {
                    action: req.action,
                    thread_id: req.thread_id,
                    frame_id: req.frame_id,
                    path: req.path,
                    line: req.line,
                },
            )
            .await?
    };

    ctx.ide()
        .collab
        .publish_debug_snapshot(Some(&target), &session);
    Ok(Json(IdeDebugSessionResp { session }))
}

// ---------------------------------------------------------------------------
// Server-Sent Events (`text/event-stream`) Framing Helpers
// ---------------------------------------------------------------------------

fn sse_headers_response(body: Body) -> Response {
    let mut resp = Response::new(body);
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    resp
}

fn format_sse_data(value: &Value) -> Bytes {
    let payload = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    Bytes::from(format!("data: {payload}\n\n"))
}

pub(super) fn build_debug_sse_response(
    initial: IdeDebugSessionSnapshot,
    rx: broadcast::Receiver<IdeDebugSessionSnapshot>,
) -> Response {
    struct State {
        initial: Option<IdeDebugSessionSnapshot>,
        rx: broadcast::Receiver<IdeDebugSessionSnapshot>,
    }

    let stream = stream::unfold(
        State {
            initial: Some(initial),
            rx,
        },
        |mut st| async move {
            if let Some(first) = st.initial.take() {
                let val = serde_json::to_value(&first).unwrap_or_else(|_| json!({}));
                return Some((Ok::<Bytes, Infallible>(format_sse_data(&val)), st));
            }
            match tokio::time::timeout(SSE_KEEPALIVE_INTERVAL, st.rx.recv()).await {
                Ok(Ok(snap)) => {
                    let val = serde_json::to_value(&snap).unwrap_or_else(|_| json!({}));
                    Some((Ok(format_sse_data(&val)), st))
                }
                Ok(Err(RecvError::Lagged(_))) => {
                    Some((Ok(Bytes::from_static(b": lagged\n\n")), st))
                }
                Ok(Err(RecvError::Closed)) => None,
                Err(_) => Some((Ok(Bytes::from_static(b": keepalive\n\n")), st)),
            }
        },
    );

    sse_headers_response(Body::from_stream(stream))
}

pub(super) fn format_collab_event_frame(ev: &IdeCollabEvent) -> Bytes {
    let mut val = serde_json::to_value(ev).unwrap_or_else(|_| json!({}));
    if let Value::Object(ref mut map) = val {
        match ev {
            IdeCollabEvent::CursorMoved { .. } => {
                map.insert("kind".into(), Value::String("cursor".into()));
            }
            IdeCollabEvent::BufferUpdated { .. } => {
                map.insert("kind".into(), Value::String("edit".into()));
            }
            _ => {}
        }
    }
    format_sse_data(&val)
}

fn build_collab_sse_response(
    room: IdeCollabRoom,
    rx: broadcast::Receiver<IdeCollabEvent>,
) -> Response {
    struct State {
        initial_room: Option<IdeCollabRoom>,
        rx: broadcast::Receiver<IdeCollabEvent>,
        done: bool,
    }

    let stream = stream::unfold(
        State {
            initial_room: Some(room),
            rx,
            done: false,
        },
        |mut st| async move {
            if st.done {
                return None;
            }
            if let Some(r) = st.initial_room.take() {
                let val = json!({ "type": "room_state", "room": r });
                return Some((Ok::<Bytes, Infallible>(format_sse_data(&val)), st));
            }
            match tokio::time::timeout(SSE_KEEPALIVE_INTERVAL, st.rx.recv()).await {
                Ok(Ok(ev)) => {
                    if matches!(ev, IdeCollabEvent::RoomClosed { .. }) {
                        st.done = true;
                    }
                    let frame = format_collab_event_frame(&ev);
                    Some((Ok(frame), st))
                }
                Ok(Err(RecvError::Lagged(_))) => {
                    Some((Ok(Bytes::from_static(b": lagged\n\n")), st))
                }
                Ok(Err(RecvError::Closed)) => None,
                Err(_) => Some((Ok(Bytes::from_static(b": keepalive\n\n")), st)),
            }
        },
    );

    sse_headers_response(Body::from_stream(stream))
}
