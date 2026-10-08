//! Capability-isolated, ephemeral collaboration rooms.
mod registry;
#[cfg(test)]
mod tests;
pub use registry::RoomRegistry;
mod actions;
mod annotations;
mod auth;
mod http;
mod ice;
pub(crate) use auth::owner_auth;
pub(crate) use ice::valid_origin;
mod presentation;
mod socket;
mod terminal;
use crate::ServerCtx;
use axum::{
    routing::{delete, get, post, put},
    Router,
};
pub fn protected_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/rooms/{id}/recaps", post(recap::http::create))
        .route("/room-recaps", get(recap::http::list))
        .route("/room-recaps/{id}", get(recap::http::detail))
        .route("/room-recaps/{id}/revision", get(recap::http::revision))
        .route("/room-recaps/{id}/export", get(recap::http::export))
        .route("/room-recaps/{id}/images/{image}", get(recap::http::image))
        .route("/room-recaps/{id}/audio", post(recap::http::audio))
        .route("/room-recaps/{id}/screen", post(recap::http::screen))
        .route("/room-recaps/{id}/gap", post(recap::http::gap))
        .route(
            "/room-recaps/{id}/summary",
            post(recap::http::summary).delete(recap::http::cancel_summary),
        )
        .route(
            "/room-recap-settings",
            get(recap::http::get_settings).put(recap::http::put_settings),
        )
        .route("/room-recap-capabilities", get(recap::http::capabilities))
        .route("/sessions/{id}/room", post(http::create))
        .route("/rooms", get(http::list))
        .route("/rooms/{id}", delete(http::end))
        .route("/rooms/{id}/invites", post(http::invite))
        .route(
            "/room-settings",
            get(ice::get_settings).merge(put(ice::put_settings)),
        )
}
pub fn public_routes() -> Router<ServerCtx> {
    Router::new().route("/room-join", post(http::join))
}
pub fn ws_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/ws/rooms/{id}", get(socket::upgrade))
        .route("/ws/rooms/{id}/terminal", get(terminal::upgrade))
}

/// Audit membership/permission decisions, never chat, SDP, media or credentials.
async fn audit(
    ctx: &ServerCtx,
    room: &str,
    user: Option<String>,
    member: Option<String>,
    action: &str,
) {
    ctx.audit(otto_state::NewAuditEntry {
        user_id: user,
        action: format!("room.{action}"),
        target: Some(room.into()),
        detail: member.map(|id| serde_json::json!({"member_id":id})),
        ip: None,
    })
    .await;
}
fn audit_action(action: &otto_core::api::RoomAction) -> Option<(&'static str, Option<String>)> {
    use otto_core::api::RoomAction::*;
    Some(match action {
        Admit { member_id, .. } => ("admit", Some(member_id.clone())),
        Reject { member_id } => ("reject", Some(member_id.clone())),
        Remove { member_id } => ("remove", Some(member_id.clone())),
        Role { member_id, .. } => ("role", Some(member_id.clone())),
        GrantControl { member_id } => ("grant_control", Some(member_id.clone())),
        ReleaseControl => ("release_control", None),
        Mute { member_id, .. } => ("mute", Some(member_id.clone())),
        GrantPresent { member_id, .. } => ("grant_present", Some(member_id.clone())),
        GrantAnnotation { member_id, .. } => ("grant_annotation", Some(member_id.clone())),
        RevokeAnnotation { member_id, .. } => ("revoke_annotation", Some(member_id.clone())),
        StartPresent { .. } => ("start_present", None),
        StopPresent { .. } => ("stop_present", None),
        Leave => ("leave", None),
        End => ("end", None),
        _ => return None,
    })
}

mod recap;
pub(super) mod recap_engines;
