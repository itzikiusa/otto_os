//! Capability-isolated, ephemeral collaboration rooms: the HTTP handlers,
//! WebSocket upgrades and run loops. The domain lives in `otto-rooms`
//! (behind `RoomsCtx`, implemented for `ServerCtx` in `ctx.rs`).
mod ctx;
mod guest;
mod http;
mod ice;
mod recap;
mod socket;
mod terminal;
use crate::ServerCtx;
use axum::{
    routing::{delete, get, post, put},
    Router,
};
pub use otto_rooms::{capability, recap_engines, RoomRegistry};
pub fn protected_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/rooms/{id}/recaps", post(recap::create))
        .route("/room-recaps", get(recap::list))
        .route("/room-recaps/{id}", get(recap::detail))
        .route("/room-recaps/{id}/revision", get(recap::revision))
        .route("/room-recaps/{id}/export", get(recap::export))
        .route("/room-recaps/{id}/images/{image}", get(recap::image))
        .route("/room-recaps/{id}/audio", post(recap::audio))
        .route("/room-recaps/{id}/screen", post(recap::screen))
        .route("/room-recaps/{id}/gap", post(recap::gap))
        .route(
            "/room-recaps/{id}/summary",
            post(recap::summary).delete(recap::cancel_summary),
        )
        .route(
            "/room-recap-settings",
            get(recap::get_settings).put(recap::put_settings),
        )
        .route("/room-recap-capabilities", get(recap::capabilities))
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
    Router::new()
        .route("/room-join", post(http::join))
        .route(
            "/room-paste/{id}",
            post(guest::paste).layer(axum::extract::DefaultBodyLimit::max(
                guest::MAX_ROOM_PASTE_BYTES,
            )),
        )
        .route("/room-prompt/{id}", post(guest::prompt))
        .route("/room-transcript/{id}", get(guest::transcript))
        .route(
            "/room-transcript/{id}/images/{img}",
            get(guest::transcript_image),
        )
}
pub fn ws_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/ws/rooms/{id}", get(socket::upgrade))
        .route("/ws/rooms/{id}/terminal", get(terminal::upgrade))
}
