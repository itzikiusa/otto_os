//! otto-connections — connection profiles: per-kind command builders,
//! CRUD service with Keychain-backed secrets, open-as-session and
//! test-connect, plus the REST router.

pub mod builders;
pub mod http;
pub mod service;

pub use builders::build_command;
pub use http::{api_router, owner_private_enabled, require_conn_owner_or_root, ConnectionsCtx, OpenConnectionReq};
pub use service::{ConnectionsService, Spawner};
