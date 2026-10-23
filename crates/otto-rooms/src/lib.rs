//! Capability-isolated, ephemeral collaboration rooms: session rooms (shared
//! terminal, chat, presentation, annotations, consented recaps) and game rooms
//! (host + guests). This crate owns the domain — registries, room actions,
//! recap capture/archive/engines, the session-authority sync and the
//! maintenance workers. Everything the server provides (settings, secrets,
//! audit, the session manager's room authority, the data dir) comes through
//! [`RoomsCtx`]; otto-server keeps only the HTTP handlers, the WebSocket
//! upgrades and their run loops.
pub mod auth;
pub mod authority;
pub mod capability;
mod ctx;
pub mod games;
pub mod ice;
pub mod lifecycle;
pub mod recap;
pub mod recap_engines;
mod registry;
pub mod terminal;

mod actions;
mod annotations;
#[cfg(test)]
mod ctx_tests;
mod presentation;
#[cfg(test)]
mod tests;

pub use actions::Effect;
pub use ctx::{Authority, RoomAudit, RoomsCtx};
pub use registry::{
    Member, Notice, Room, RoomRegistry, SharedRoom, GRACE, INVITE_TTL, PENDING_TTL, RATE_LIMITED,
    ROOM_TTL,
};
