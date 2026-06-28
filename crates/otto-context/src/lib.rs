//! otto-context — Otto's library of skills/souls/context and the per-CLI
//! materialization that injects them into claude/codex at session spawn.
//!
//! Implemented per the spec
//! (docs/superpowers/specs/2026-06-13-otto-context-provisioning-design.md).

pub mod config;
pub mod http;
pub mod library;
pub mod materialize;
pub mod merge;
pub mod provisioner;
pub mod repomap;
pub mod user_skills;

pub use http::{router, ContextCtx};
pub use library::Library;
pub use provisioner::Provisioner;
