//! otto-core — domain types, identifiers, errors, events and API DTOs.
//!
//! This crate is the contract of record for the Otto workspace. It has no I/O
//! and no async; every other crate depends on it and codes against these types.

pub mod access;
pub mod agent_runtime;
pub mod api;
pub mod auth;
pub mod aws_env;
pub mod cadence;
pub mod cancel;
pub mod cancel_signal;
pub mod connection_credentials;
pub mod domain;
pub mod e2e;
pub mod error;
pub mod eval_score;
pub mod event;
pub mod expr;
pub mod finding;
pub mod hooks;
pub mod id;
pub mod keyed_lock;
pub mod kubeconfig_policy;
pub mod mcp_budgets;
pub mod paths;
pub mod peer;
pub mod phone;
pub mod provider;
pub mod provider_accounts;
pub mod redact;
pub mod restart;
pub mod runtime_drafts;
pub mod secret_paths;
pub mod secrets;
pub mod supervise;
pub mod text;
pub mod ui_grant;
pub mod workflows;
pub mod workref;

pub use error::{Error, Result};

/// Build `f`'s future on the heap from a frame of its own. A debug build's
/// async poll frame keeps a stack slot for every future it builds inline;
/// nested as deep as a workflow run → node → step runner → runtime turn,
/// those slots overflowed tokio's 2 MiB worker stack (CR1-002). Built here,
/// the future's bytes leave the stack before it is polled.
#[inline(never)]
pub fn heap<F, Fut>(f: F) -> std::pin::Pin<Box<Fut>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future,
{
    Box::pin(f())
}
pub use id::{new_id, Id};

pub mod network_profiles;
