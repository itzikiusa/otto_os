//! otto-review — the multi-agent code-review engine.
//!
//! - [`session`]: runs one reviewer as a real, openable PTY session (prompt
//!   injection, findings-file capture, recovery, lens files).
//! - [`summarizer`]: the managed summarizer lifecycle.
//! - [`fallback`]: the deterministic summarizer floor.
//! - [`worktree`]: isolated PR-head checkouts for review runs.
//! - [`engine`]: the pure core — diff rendering, config defaults + budgets,
//!   fan-out / orchestrator expansion, reviewer prompts, draft-comment parsing,
//!   severity and run-completeness rules.
//!
//! otto-server mounts the routes and drives the ctx-bound orchestration.

pub mod engine;
pub mod fallback;
pub mod session;
pub mod summarizer;
pub mod worktree;
