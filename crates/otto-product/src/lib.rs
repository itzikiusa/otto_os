//! otto-product — Product Story Analysis feature crate.
//!
//! Exposes:
//! - [`types`] — request DTOs and response types (not in otto-core).
//! - [`http`] — the [`ProductCtx`] trait and [`router`] function.
//! - [`service`] — [`ProductService`] (Phase 1 stub; Phase 2 fills methods).
//! - [`analysis`] — the analyze / rewrite / generate-tests / generate-plan /
//!   retry / stop / approve handlers that launch the runners.
//! - [`run`] — the analysis / rewrite / test-gen / plan-gen runners, driven
//!   through the host's [`ProductRunHost`] (see [`host`]).
//! - [`chat`] / [`refine`] / [`media`] / [`swarm`] — the story-studio handlers
//!   (discovery chat, refinement threads, attachments + annotations, Product↔Swarm), driven through [`ProductStudioHost`].
//! - [`design_format`] / [`design_scene3d`] — the design artifact formats and
//!   the `scene3d` validator + Blender-script generator.
//! - [`watcher`] — the story watcher (Jira/Confluence comment reconcile).

pub mod analysis;
pub mod chat;
pub mod design_format;
pub mod design_scene3d;
pub mod extract;
pub mod host;
pub mod http;
pub mod media;
pub mod memory_facade;
pub mod refine;
pub mod run;
pub mod service;
pub mod skills;
pub mod swarm;
pub mod types;
pub mod watcher;

pub use host::{BudgetGate, ProductRunHost, ProductStudioHost};
pub use http::{router, ProductCtx};
pub use memory_facade::ProductMemory;
pub use service::{build_watch_cursor, validate_tree_kind, CommentInfo, ProductService};
pub use skills::{analysis_lenses, seed_skills, skill_body, SKILL_NAMES};
