//! otto-sessions — workspace terminal sessions: provider registry, the
//! `SessionManager` (PTY lifecycle + status detection), the sessions REST
//! router and the terminal WebSocket.

pub mod http;
pub mod lifecycle;
pub mod manager;
pub mod mcp;
pub mod prompt_guard;
pub mod providers;
pub mod trust;
pub mod ws;

pub use http::{api_router, SessionsCtx};
pub use lifecycle::{check_resumability, Resumability};
pub use manager::{OutputScanner, SessionManager};
pub use prompt_guard::{CompositeScanner, PromptGuard};
pub use providers::{ProviderRegistry, ProviderSpec};
pub use ws::ws_router;
