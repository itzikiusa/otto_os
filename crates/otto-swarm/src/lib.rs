//! otto-swarm — Agent Swarm persistence façade, API router, recruiter/planner
//! prompt design, preset templates, and the orchestration [`runtime`] (Coordinator,
//! scheduler, one-turn runner, verifier, channel triggers) behind the
//! [`runtime::SwarmHost`] trait the daemon implements.
//!
//! See docs/superpowers/specs/2026-06-18-agent-swarm-design.md.

pub mod http;
pub mod presets;
pub mod recruiter;
pub mod runtime;
pub mod service;
pub mod types;

pub use http::{router, SwarmCtx};
pub use service::SwarmService;
pub use types::*;
