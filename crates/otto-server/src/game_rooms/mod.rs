//! Two-player game capabilities are isolated from sessions and owner credentials.
mod registry;
#[cfg(test)]
mod tests;
mod types;
pub use registry::Registry;
mod http;
mod socket;
use crate::ServerCtx;
use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
pub fn protected_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/game-rooms", post(http::create))
        .layer(DefaultBodyLimit::max(2048))
}
pub fn public_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/game-room-join", post(http::join))
        .layer(DefaultBodyLimit::max(2048))
}
pub fn ws_routes() -> Router<ServerCtx> {
    Router::new().route("/ws/game-rooms/{id}", get(socket::upgrade))
}
#[cfg(test)]
mod api_tests;
