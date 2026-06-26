//! otto-workgraph — the "new internal concept" behind Mission Control.
//!
//! A thin service over [`otto_state::WorkGraphRepo`] that persists work-graph
//! mutations, appends the matching audit event, and broadcasts
//! [`otto_core::event::Event::WorkGraphUpdated`] so the UI updates live. The
//! per-source PROJECTION logic (which module event maps to which work item) and
//! all I/O enrichment (reading source repos, usage cost) live in the daemon's
//! projector (`otto-server`), which calls into this service. Pure normalization
//! (status + risk) lives in [`normalize`] and on the otto-state enums, so it is
//! unit-testable without a database.

pub mod normalize;
pub mod service;

pub use normalize::risk;
pub use service::WorkGraphService;
