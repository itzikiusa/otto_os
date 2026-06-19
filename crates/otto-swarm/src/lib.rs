//! otto-swarm — Agent Swarm persistence façade, API router, recruiter/planner
//! prompt design, and preset templates. The orchestration runtime (Coordinator,
//! scheduler, one-turn runner, board ingest) lives in otto-server, where
//! SessionManager + Orchestrator are available.
//!
//! See docs/superpowers/specs/2026-06-18-agent-swarm-design.md.

pub mod http;
pub mod presets;
pub mod recruiter;
pub mod service;
pub mod types;

pub use http::{router, SwarmCtx};
pub use service::SwarmService;
pub use types::*;
