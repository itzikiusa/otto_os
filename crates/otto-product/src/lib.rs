//! otto-product — Product Story Analysis feature crate.
//!
//! Exposes:
//! - [`types`] — request DTOs and response types (not in otto-core).
//! - [`http`] — the [`ProductCtx`] trait and [`router`] function.
//! - [`service`] — [`ProductService`] (Phase 1 stub; Phase 2 fills methods).

pub mod http;
pub mod service;
pub mod skills;
pub mod types;

pub use http::{router, ProductCtx};
pub use service::{CommentInfo, ProductService};
pub use skills::{seed_skills, skill_body, SKILL_NAMES};
