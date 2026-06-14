//! otto-improve — scheduled per-workspace self-reflection: review recent
//! sessions and improve the workspace's memory and handling skills.
//!
//! SCAFFOLD: module bodies are filled in by the implementation plan
//! (docs/superpowers/plans/2026-06-13-agent-self-improvement-scheduled.md).

pub mod classify;
pub mod config;
pub mod digest;
pub mod engine;
pub mod http;
pub mod live;
pub mod pathsafe;
pub mod producer;
pub mod prompt;
pub mod proposal;
pub mod scheduler;

pub use engine::ImprovementEngine;
pub use http::{router, ImproveCtx};
pub use live::{LiveEvolver, LiveEvolverHandle};
pub use producer::{ProposalProducer, RealProposalProducer};
pub use scheduler::{Scheduler, SchedulerHandle};
