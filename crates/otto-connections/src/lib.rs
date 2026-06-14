//! otto-connections — connection profiles: per-kind command builders,
//! CRUD service with Keychain-backed secrets, open-as-session and
//! test-connect, plus the REST router.

pub mod builders;
pub mod http;
pub mod service;

pub use builders::build_command;
pub use http::{api_router, ConnectionsCtx, OpenConnectionReq};
pub use service::{ConnectionsService, Spawner};
