//! The API client's REST execute engine (moved out of `otto-server`'s
//! `routes/api_client.rs`, architecture-1-07r): environment + Keychain secret
//! resolution, the secret ↔ host binding, per-(workspace, user) cookie jars
//! and pooled clients, SSH/SOCKS tunnels, the outbound send itself, history
//! recording + retention, and the automation step runner.
//!
//! Everything the engine needs from the daemon goes through [`ApiClientCtx`]
//! (repo, secret store, workspace settings, role check, tunnel connection
//! lookup, the event broadcast and the response cache); `otto-server`
//! implements it for its server context and keeps only the HTTP handlers. Failures
//! are an [`EngineError`], which the route maps 1:1 onto `otto_core::Error`
//! (so every status and body stays what the handlers always returned).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use http::HeaderMap;
use otto_core::api::{
    ApiClientStorage, ApiResolvedRequest, ApiResponse, ApiRunStepResult, ExecuteApiReq,
    RunSavedRequestReq, RunSavedRequestResp,
};
use otto_core::auth::AuthContext;
#[cfg(test)]
use otto_core::domain::ApiHistoryEntry;
use otto_core::domain::{ApiEnvironment, ApiRequest, Connection, ConnectionKind, User};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::Id;
use otto_ssh::{SshTunnel, SshTunnelConfig};
use otto_state::{ApiClientRepo, NewApiHistory};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::rest::{eval_assertion_with, header_value, json_path, percent_decode, StepOutcome};
use crate::scripts::{self as api_scripts, ScriptRequest, ScriptResponse};
use crate::secrets as api_secrets;

mod auth;
mod binding;
mod client;
mod env;
mod execute;
mod history;
mod multipart;
mod send;
mod steps;
mod tunnel;
mod vars;

pub use binding::*;
pub use client::*;
pub use env::*;
pub use execute::*;
pub use history::*;
pub use send::*;
pub use steps::*;
pub use tunnel::*;
pub use vars::*;
// Engine-internal only (nothing public to re-export).
use auth::*;
use multipart::*;

#[cfg(test)]
mod tests;

/// Agent-facing request/response body cap.
pub const AGENT_BODY_MAX: usize = 64 * 1024;
/// Execution timeout for outbound requests.
const EXECUTE_TIMEOUT: Duration = Duration::from_secs(60);

/// What the engine needs from the daemon. `otto-server` implements it for
/// server context (`routes/api_client/ctx.rs`); tests can implement it over a
/// bare pool. `Clone + 'static` because history retention runs off the
/// response path on a spawned task.
#[async_trait::async_trait]
pub trait ApiClientCtx: Clone + Send + Sync + 'static {
    /// The API-client tables.
    fn repo(&self) -> ApiClientRepo;
    /// The Keychain-backed secret store (request / environment blobs).
    fn secrets(&self) -> &Arc<dyn SecretStore>;
    /// The workspace's `settings` object, `None` when it can't be read.
    async fn workspace_settings(&self, wid: &Id) -> Option<Value>;
    /// Editor role (or better) on `wid` for `user`.
    async fn require_editor(&self, user: &User, wid: &Id) -> otto_core::Result<()>;
    /// The `ssh`-kind connection `conn_id` a request tunnels through, after
    /// checking `actor` may use it as a shell. Errors are human-readable
    /// (surfaced as a 502).
    async fn tunnel_connection(&self, conn_id: &Id, actor: &Id) -> Result<Connection, String>;
    /// Broadcast an event to connected clients (history appends).
    fn broadcast(&self, event: Event);
    /// Park a response body in the bounded download cache for `(wid, user)`;
    /// returns its `body_id`.
    fn park_body(&self, wid: &Id, user: &Id, content_type: Option<String>, raw: Vec<u8>) -> Id;
}

/// An engine failure. Each variant maps onto the same-named
/// `otto_core::Error` variant (400 / 409 / 502 / 404 / 500 in the route),
/// `Core` passes a storage or role error through unchanged.
#[derive(Debug)]
pub enum EngineError {
    /// Bad input: an unknown secret ref, malformed vars, a failed script.
    Invalid(String),
    /// Needs a confirmation (`needs_confirm=…`).
    Conflict(String),
    /// The send, tunnel or upstream failed.
    Upstream(String),
    /// An entity outside the caller's workspace.
    NotFound(String),
    /// An unexpected internal failure (serialization).
    Internal(String),
    /// A storage / role error, unchanged.
    Core(otto_core::Error),
}

/// Engine result alias.
pub type EngineResult<T> = Result<T, EngineError>;

impl From<otto_core::Error> for EngineError {
    fn from(e: otto_core::Error) -> Self {
        EngineError::Core(e)
    }
}

impl From<EngineError> for otto_core::Error {
    fn from(e: EngineError) -> Self {
        use otto_core::Error as E;
        match e {
            EngineError::Invalid(m) => E::Invalid(m),
            EngineError::Conflict(m) => E::Conflict(m),
            EngineError::Upstream(m) => E::Upstream(m),
            EngineError::NotFound(m) => E::NotFound(m),
            EngineError::Internal(m) => E::Internal(m),
            EngineError::Core(e) => e,
        }
    }
}

impl fmt::Display for EngineError {
    /// The `otto_core::Error` text the route would put in the body.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use otto_core::Error as E;
        let owned = match self {
            EngineError::Core(e) => return e.fmt(f),
            EngineError::Invalid(m) => E::Invalid(m.clone()),
            EngineError::Conflict(m) => E::Conflict(m.clone()),
            EngineError::Upstream(m) => E::Upstream(m.clone()),
            EngineError::NotFound(m) => E::NotFound(m.clone()),
            EngineError::Internal(m) => E::Internal(m.clone()),
        };
        owned.fmt(f)
    }
}

impl std::error::Error for EngineError {}

/// 404 when an entity belongs to a different workspace than the path's `wid`.
pub fn ensure_in_workspace(entity_ws: &Id, wid: &Id) -> EngineResult<()> {
    if entity_ws == wid {
        Ok(())
    } else {
        Err(EngineError::NotFound("not in this workspace".into()))
    }
}

/// Truncate UTF-8 text at a byte cap without splitting a character.
pub fn truncate_string(value: &mut String, max: usize) {
    if value.len() <= max {
        return;
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}
