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
use crate::target::ResolvedTarget;
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
        .enrich_target_roots_authorized(&ctx, &user, &mut req.target, true)
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
    let plugins_reg = ctx.ide().plugins.clone();
    let (plugins, catalog) = tokio::task::spawn_blocking(move || plugins_reg.list())
        .await
        .map_err(|e| Error::Internal(format!("list plugins join failed: {e}")))?;
    Ok(Json(IdePluginsResp { plugins, catalog }))
}

pub(super) async fn install_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeInstallPluginReq>,
) -> ApiResult<Json<IdePluginResp>> {
    super::require_root_user(&user)?;
    let plugin = ctx.ide().plugins.install(&req.source).await?;
    crate::lsp_host::servers::invalidate_detect_cache();
    Ok(Json(IdePluginResp { plugin }))
}

pub(super) async fn toggle_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(slug): Path<String>,
    Json(req): Json<IdeTogglePluginReq>,
) -> ApiResult<Json<IdePluginResp>> {
    super::require_root_user(&user)?;
    let plugins_reg = ctx.ide().plugins.clone();
    let enabled = req.enabled;
    let plugin = tokio::task::spawn_blocking(move || plugins_reg.toggle(&slug, enabled))
        .await
        .map_err(|e| Error::Internal(format!("toggle plugin join failed: {e}")))??;
    crate::lsp_host::servers::invalidate_detect_cache();
    Ok(Json(IdePluginResp { plugin }))
}

pub(super) async fn uninstall_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(slug): Path<String>,
) -> ApiResult<Json<IdeOkResp>> {
    super::require_root_user(&user)?;
    let plugins_reg = ctx.ide().plugins.clone();
    tokio::task::spawn_blocking(move || plugins_reg.uninstall(&slug))
        .await
        .map_err(|e| Error::Internal(format!("uninstall plugin join failed: {e}")))??;
    crate::lsp_host::servers::invalidate_detect_cache();
    Ok(Json(IdeOkResp { ok: true }))
}

// ---------------------------------------------------------------------------
// Public Capability-Token Guest Collaboration Routes
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub(super) struct CollabGuestQuery {
    #[serde(default)]
    peer_id: Option<String>,
}

/// The invite token, from `X-Ide-Collab-Token` (what the UI sends) or a
/// `Bearer` header — never the query string: a write-capable capability in a
/// URI ends up in access logs and proxy histories.
pub(super) fn extract_collab_token(headers: &HeaderMap) -> Result<String> {
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

fn fill_peer_secret_from_headers(headers: &HeaderMap, peer_secret: &mut String) {
    if !peer_secret.trim().is_empty() {
        return;
    }
    if let Some(hv) = headers
        .get("x-ide-collab-peer-secret")
        .or_else(|| headers.get("x-ide-collab-token"))
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = hv.trim();
        if !trimmed.is_empty() {
            *peer_secret = trimmed.to_string();
        }
    }
}

pub(super) async fn collab_guest_join<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Json(mut req): Json<IdeCollabJoinReq>,
) -> ApiResult<Json<IdeCollabJoinResp>> {
    if req.token.trim().is_empty() {
        req.token = extract_collab_token(&headers)?;
    }
    ensure_host_active(&ctx, &room_id).await?;
    let resp = ctx.ide().collab.join_room(&room_id, req)?;
    Ok(Json(resp))
}

pub(super) async fn collab_guest_state<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<IdeCollabStateResp>> {
    let token = extract_collab_token(&headers)?;
    let mut state = ctx.ide().collab.get_state(&room_id, &token)?;
    let host_id = ctx.ide().collab.host_user_id(&room_id).unwrap_or_default();
    if let Some(debug_id) = state.room.active_debug_id.as_deref() {
        let belongs_to_host = ctx
            .ide()
            .dap
            .session_owner_and_target(debug_id)
            .is_some_and(|(owner, _)| owner == host_id);
        if belongs_to_host {
            if let Ok(fresh) = ctx.ide().dap.get_session(debug_id) {
                state.active_debug = Some(crate::collab::redact_guest_debug_snapshot(&fresh));
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
    let token = extract_collab_token(&headers)?;
    ensure_host_active(&ctx, &room_id).await?;
    let peer_id = q.peer_id.filter(|p| !p.trim().is_empty());
    let (room, rx) = ctx
        .ide()
        .collab
        .subscribe_peer(&room_id, &token, peer_id.as_deref())?;
    // When the stream ends (tab closed, network gone) the peer starts its
    // grace period; a peer with no stream left is freed after it.
    let on_close = peer_id.map(|peer_id| {
        let engine = ctx.ide().clone();
        Box::new(move || engine.collab.peer_stream_closed(&room_id, &peer_id))
            as Box<dyn FnOnce() + Send>
    });
    Ok(build_collab_sse_response(room, rx, on_close))
}

pub(super) async fn collab_guest_cursor<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Json(mut req): Json<IdeCollabCursorReq>,
) -> ApiResult<Json<IdeOkResp>> {
    fill_peer_secret_from_headers(&headers, &mut req.peer_secret);
    ctx.ide().collab.update_cursor(&room_id, req)?;
    Ok(Json(IdeOkResp { ok: true }))
}

pub(super) async fn collab_guest_edit<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Json(mut req): Json<IdeCollabEditReq>,
) -> ApiResult<Json<IdeCollabEditResp>> {
    fill_peer_secret_from_headers(&headers, &mut req.peer_secret);
    let (target, frozen_host) = ctx.ide().collab.validate_edit_precheck(&room_id, &req)?;
    if req.save && is_agent_or_git_control_path(&req.path) {
        return Err(
            Error::Forbidden("guests cannot save git or agent configuration files".into()).into(),
        );
    }
    ctx.ide()
        .targets
        .validate_subpath(&target, &req.path, false)
        .await?;
    let resolved_for_save = if req.save {
        let host_user = current_host(&ctx, &room_id, frozen_host).await?;
        let host_resolved = ctx
            .ide()
            .targets
            .resolve_write(&ctx, &host_user, &target)
            .await?;
        Some(match target.kind {
            // A guest writes with non-root path rules even in a root host's
            // room, and the guest list is checked on the RESOLVED path (a
            // committed `tools -> .git` symlink must not carry a save into
            // `.git/hooks`).
            IdeTargetKind::Local => {
                let guest_target = host_resolved.restricted_for_guest();
                let resolved_path = ctx
                    .ide()
                    .targets
                    .resolve_path(&guest_target, &req.path, false)
                    .await?;
                refuse_guest_control_path(std::path::Path::new(&resolved_path))?;
                guest_target
            }
            IdeTargetKind::Ssh => {
                let room_target =
                    ctx.ide()
                        .collab
                        .room_resolved_target(&room_id)
                        .ok_or_else(|| {
                            Error::Upstream(
                                "remote SSH target is unavailable for collaborative save".into(),
                            )
                        })?;
                if let ResolvedTarget::Ssh {
                    root,
                    additional_roots,
                    ..
                } = &room_target
                {
                    let (remote, matched) = crate::target::ssh_exec::resolve_remote_in_roots(
                        root,
                        additional_roots,
                        &req.path,
                    )?;
                    let real = ctx
                        .ide()
                        .targets
                        .confine_remote_write(&room_target, &matched, &remote)
                        .await?;
                    if let Some(why) = crate::target::guest_control_path_denial_at(
                        std::path::Path::new(&real),
                        false,
                    ) {
                        return Err(Error::Forbidden(why).into());
                    }
                }
                room_target
            }
        })
    } else {
        None
    };

    // Commit buffer version under the room lock BEFORE writing to disk so a racing
    // `base_version` conflict never overwrites target files. Peers hear "saved"
    // only after the write succeeds.
    let version = ctx.ide().collab.apply_edit(&room_id, &req, false)?;

    if let Some(resolved) = resolved_for_save {
        ctx.ide()
            .targets
            .write_file(&resolved, &req.path, &req.content, None, false)
            .await?;
        ctx.ide()
            .collab
            .mark_buffer_saved(&room_id, &req.peer_id, &req.path, version);
    }
    Ok(Json(IdeCollabEditResp { version }))
}

pub(super) async fn collab_guest_debug_action<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Json(mut req): Json<IdeCollabDebugActionReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    fill_peer_secret_from_headers(&headers, &mut req.peer_secret);
    let (target, frozen_host) = ctx.ide().collab.authorize_debug_action(&room_id, &req)?;
    let host_user = current_host(&ctx, &room_id, frozen_host).await?;
    let owns_session = ctx
        .ide()
        .dap
        .session_owner_and_target(&req.debug_id)
        .is_some_and(|(owner, _)| owner == host_user.id);
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
        let snap = latest_snap.unwrap();
        ctx.ide()
            .collab
            .publish_debug_snapshot(Some(&target), Some(host_user.id.as_str()), &snap);
        snap
    } else {
        super::runtime_debug::execute_debug_command_with_netbridge(
            &ctx,
            &host_user,
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

    Ok(Json(IdeDebugSessionResp {
        session: crate::collab::redact_guest_debug_snapshot(&session),
    }))
}

/// Git config/hooks and agent CLI config run code on the HOST's next git or
/// agent command (invisible in a diff), so an invited guest never saves them.
/// This first pass runs on the decoded REQUEST path (a `file://` URI is
/// percent-decoded before the write; also the only pass an SSH room gets
/// before its remote realpath check); local saves are checked again on the
/// resolved path by [`refuse_guest_control_path`].
fn is_agent_or_git_control_path(path: &str) -> bool {
    let path = crate::target::sdk_paths::strip_file_uri(path);
    // Lexical only: the request path may name a file on an SSH host.
    crate::target::guest_control_path_denial_at(std::path::Path::new(&path), false).is_some()
}

fn refuse_guest_control_path(resolved: &std::path::Path) -> Result<()> {
    match crate::target::guest_control_path_denial(resolved) {
        Some(why) => Err(Error::Forbidden(why)),
        None => Ok(()),
    }
}

/// The room's host as of NOW, not as of room creation: a disabled host's
/// rooms close, and a demoted host's guests write with the demoted rights.
/// (Otto disables users rather than deleting them; a missing row keeps the
/// room's copy.)
async fn current_host<S: IdeCtx>(
    ctx: &S,
    room_id: &str,
    frozen: otto_core::domain::User,
) -> Result<otto_core::domain::User> {
    match otto_state::UsersRepo::new(ctx.pool()).get(&frozen.id).await {
        Ok(user) if user.disabled => {
            ctx.ide().collab.revoke_room(room_id);
            Err(Error::Forbidden(
                "the host of this shared session no longer has access".into(),
            ))
        }
        Ok(user) => Ok(user),
        Err(Error::NotFound(_)) => Ok(frozen),
        Err(e) => Err(e),
    }
}

/// [`current_host`] for routes that only need the room to still be valid.
async fn ensure_host_active<S: IdeCtx>(ctx: &S, room_id: &str) -> Result<()> {
    let Some(host_id) = ctx.ide().collab.host_user_id(room_id) else {
        return Ok(()); // unknown room: the hub answers 404 itself
    };
    match otto_state::UsersRepo::new(ctx.pool()).get(&host_id).await {
        Ok(user) if user.disabled => {
            ctx.ide().collab.revoke_room(room_id);
            Err(Error::Forbidden(
                "the host of this shared session no longer has access".into(),
            ))
        }
        Ok(_) | Err(Error::NotFound(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn guest_save_refuses_git_and_agent_config() {
        for p in [
            "/w/.git/hooks/pre-commit",
            "/w/.git/config",
            "/w/.claude/settings.json",
            "/w/sub/.mcp.json",
            ".codex/config.toml",
            "file:///w/%2Eclaude/settings.json",
            "file:///w/%2Egit/config",
            "/w/.CLAUDE/settings.json",
            "/w/.Git/hooks/pre-commit",
        ] {
            assert!(super::is_agent_or_git_control_path(p), "{p}");
        }
        for p in ["/w/src/main.go", "/w/.github/workflows/ci.yml", "/w/git.md"] {
            assert!(!super::is_agent_or_git_control_path(p), "{p}");
        }
    }
}
