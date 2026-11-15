//! HTTP handlers for Share-with-Friends Collaboration (`/ide/collab/*`) and
//! Runtime IDE Plugins (`/ide/plugins/*`).

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap};
use axum::response::Response;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::{Error, Result};
use serde::Deserialize;

use super::sse_framing::build_collab_sse_response;
use super::ApiResult;
use crate::types::{
    IdeCollabCursorReq, IdeCollabDebugActionReq, IdeCollabEditReq, IdeCollabEditResp,
    IdeCollabJoinReq, IdeCollabJoinResp, IdeCollabRoleReq, IdeCollabRoomResp, IdeCollabRoomsResp,
    IdeCollabStateResp, IdeCreateCollabRoomReq, IdeCreateCollabRoomResp, IdeDebugAction,
    IdeDebugCommandReq, IdeDebugSessionResp, IdeInstallPluginReq, IdeOkResp, IdePluginResp,
    IdePluginsResp, IdeSetBreakpointsReq, IdeTargetKind, IdeTogglePluginReq,
};
use crate::IdeCtx;

// ---------------------------------------------------------------------------
// E. "Share with Friends" Host Routes
// ---------------------------------------------------------------------------

pub(super) async fn create_collab_room<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeCreateCollabRoomReq>,
) -> ApiResult<Json<IdeCreateCollabRoomResp>> {
    ctx.ide()
        .environments
        .enrich_target_roots(
            &ctx.pool(),
            Some(&ctx.ide().projects),
            Some(&user),
            &mut req.target,
        )
        .await;
    let resolved = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
    let initial_debug = ctx
        .ide()
        .dap
        .active_session_for_target_and_owner(&req.target, &user.id);
    let resp =
        ctx.ide()
            .collab
            .create_room_with_resolved(&user, req, Some(resolved), initial_debug)?;
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
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeInstallPluginReq>,
) -> ApiResult<Json<IdePluginResp>> {
    super::require_root_user(&user)?;
    let plugin = ctx.ide().plugins.install(&req.source).await?;
    Ok(Json(IdePluginResp { plugin }))
}

pub(super) async fn toggle_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(slug): Path<String>,
    Json(req): Json<IdeTogglePluginReq>,
) -> ApiResult<Json<IdePluginResp>> {
    super::require_root_user(&user)?;
    let plugin = ctx.ide().plugins.toggle(&slug, req.enabled)?;
    Ok(Json(IdePluginResp { plugin }))
}

pub(super) async fn uninstall_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(slug): Path<String>,
) -> ApiResult<Json<IdeOkResp>> {
    super::require_root_user(&user)?;
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
        let belongs_to_host = ctx
            .ide()
            .dap
            .session_owner_and_target(debug_id)
            .is_some_and(|(owner, _)| owner == state.room.host_user_id);
        if belongs_to_host {
            if let Ok(fresh) = ctx.ide().dap.get_session(debug_id) {
                state.active_debug = Some(fresh);
            }
        } else {
            state.active_debug = None;
            state.room.active_debug_id = None;
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
    let (target, _host_user_id) = ctx.ide().collab.validate_edit_precheck(&room_id, &req)?;
    ctx.ide()
        .targets
        .validate_subpath(&target, &req.path, false)
        .await?;
    let mut saved_to_target = false;
    if req.save {
        let resolved = match target.kind {
            IdeTargetKind::Local => ctx.ide().targets.resolve_local(&target, false)?,
            IdeTargetKind::Ssh => {
                ctx.ide()
                    .collab
                    .room_resolved_target(&room_id)
                    .ok_or_else(|| {
                        Error::Upstream(
                            "remote SSH target is unavailable for collaborative save".into(),
                        )
                    })?
            }
        };
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
    let (target, host_user_id) = ctx.ide().collab.authorize_debug_action(&room_id, &req)?;
    let owns_session = ctx
        .ide()
        .dap
        .session_owner_and_target(&req.debug_id)
        .is_some_and(|(owner, _)| owner == host_user_id);
    if !owns_session {
        return Err(Error::Forbidden("debug session is not owned by the room host".into()).into());
    }

    if let Some(ref bps) = req.breakpoints {
        for bp in bps {
            if bp
                .condition
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
                || bp
                    .log_message
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty())
            {
                return Err(Error::Forbidden(
                    "guest collaborators cannot set conditional or logpoint expressions".into(),
                )
                .into());
            }
            ctx.ide()
                .targets
                .validate_subpath(&target, &bp.path, false)
                .await?;
        }
    }
    if let Some(path) = req.path.as_deref().filter(|s| !s.trim().is_empty()) {
        ctx.ide()
            .targets
            .validate_subpath(&target, path, false)
            .await?;
    }

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
        .publish_debug_snapshot(Some(&target), Some(host_user_id.as_str()), &session);
    Ok(Json(IdeDebugSessionResp { session }))
}
