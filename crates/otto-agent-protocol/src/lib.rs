//! otto-agent-protocol — the structured agent runtime (design
//! `docs/features/threads-structured-runtime.md`).
//!
//! Agent CLIs run as child processes speaking a structured stdio protocol
//! (Claude Code stream-json, Codex app-server, ACP) instead of inside a PTY.
//! One sans-IO [`StructuredDriver`] per provider translates that protocol into
//! Otto's normalized event stream (`otto_core::agent_runtime`); the
//! [`AgentRuntime`] owns the processes, sequences events into a per-session
//! replay ring, persists finished turns, and enforces one turn at a time.
//! Spawns go through the PTY sessions' own safety pipeline
//! (`SessionManager::prepare_runtime_spawn`: cwd refusal, folder trust,
//! context, Seatbelt) via [`RuntimeHost`].
//!
//! Design ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.); see
//! the design doc for the file-by-file mapping. Drivers: [`claude`] (the
//! Agent SDK's stream-json control protocol against `claude`), [`codex`]
//! (`codex app-server`) and [`acp`] (Gemini `gemini --acp`, Grok `grok …
//! agent stdio`, Antigravity `agy acp`, opencode `opencode acp`).

pub mod acp;
pub mod claude;
pub mod codex;
pub mod driver;
pub mod host;
mod http;
pub mod pi;
pub mod process;
mod questions;
pub mod ring;
pub mod runtime;
mod tool_images;
#[cfg(test)]
mod tool_images_tests;

use std::sync::Arc;

use axum::Router;
use otto_core::auth::{AuthContext, RoleChecker};

pub use driver::{
    ChildSignal, DriverError, DriverEvent, DriverFactory, DriverStep, LaunchContext, LaunchSpec,
    McpDelivery, McpServerDef, StructuredDriver,
};
pub use host::{RuntimeHost, SessionManagerHost};
pub use runtime::{AgentRuntime, RuntimeConfig};

/// The daemon surface the `/runtime-sessions` router runs against
/// (otto-server's impl: `modules/ctx_impls.rs`).
pub trait RuntimeCtx: Clone + Send + Sync + 'static {
    fn agent_runtime(&self) -> &AgentRuntime;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    /// A person's own credential (not an agent session's, MCP or share token).
    fn is_human(&self, auth: &AuthContext) -> bool;
}

/// All `/runtime-sessions` routes (handler-relative; nested under `/api/v1`).
pub fn api_router<S: RuntimeCtx>() -> Router<S> {
    http::api_router::<S>()
}

/// The production driver set — the providers T3 Code drives over a
/// structured protocol: Claude (Agent SDK transport), Codex (app-server),
/// Gemini, Grok, Antigravity (`agy acp`) and opencode (ACP), pi (its RPC
/// mode). A missing CLI fails the spawn, not the create.
pub fn default_drivers() -> Arc<dyn DriverFactory> {
    use otto_core::agent_runtime::RuntimeProvider;
    Arc::new(
        |provider: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
            Some(match provider {
                RuntimeProvider::Claude => Box::new(claude::ClaudeDriver::new()),
                RuntimeProvider::Codex => Box::new(codex::CodexDriver::with_program("codex")),
                RuntimeProvider::Gemini => Box::new(acp::AcpDriver::gemini("gemini")),
                RuntimeProvider::Grok => Box::new(acp::AcpDriver::grok()),
                RuntimeProvider::Agy => Box::new(acp::AcpDriver::agy("agy")),
                RuntimeProvider::Opencode => Box::new(acp::AcpDriver::opencode()),
                RuntimeProvider::Pi => Box::new(pi::PiDriver::with_program("pi")),
            })
        },
    )
}

/// The daemon's runtime: SessionManager safety, `publish` for WS delivery
/// (otto-server passes its streaming-path `publish_stream`).
/// Attachments live under `<data_dir>/runtime-attachments`.
pub fn daemon_runtime(
    pool: otto_state::DbPool,
    manager: Arc<otto_sessions::SessionManager>,
    data_dir: &std::path::Path,
    publish: impl Fn(otto_core::event::Event) + Send + Sync + 'static,
) -> AgentRuntime {
    // Structured children survive a restart in stdio holders: the PTY
    // holders' launcher, their own directory.
    let holders = manager.pty_holders().and_then(|pty| {
        let mut cfg = otto_pty::HolderConfig::for_data_dir_named(
            data_dir,
            "rt-holders",
            "otto-rth",
            pty.launcher.clone(),
        )?;
        cfg.orphan_ttl = pty.orphan_ttl;
        cfg.exit_linger = pty.exit_linger;
        Some(cfg)
    });
    let host = SessionManagerHost {
        manager,
        workspaces: otto_state::WorkspacesRepo::new(pool.clone()),
        publish: Box::new(publish),
        holders,
    };
    AgentRuntime::new(
        otto_state::AgentRuntimeRepo::new(pool),
        Arc::new(host),
        default_drivers(),
        RuntimeConfig {
            attachments_dir: data_dir.join("runtime-attachments"),
            ..RuntimeConfig::default()
        },
    )
}
