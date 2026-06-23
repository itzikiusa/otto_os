//! otto-canvas — Canvas Studio REST router.
//!
//! A visual scene (sketches, UML, sequence/flow diagrams, code/JSON blocks,
//! shapes) stored as ONE portable JSON document. The Rust side owns CRUD over
//! the document + metadata; the rich Scene schema and rendering live in the UI.
//! The agent-assisted "draw it for me" endpoints live in `otto-server`
//! (`canvas_assist.rs`) because they need the orchestrator.
//!
//! Two-tier routing (server nests this under `/api/v1`):
//!   - Collection: `/workspaces/{ws}/canvas/scenes`
//!   - Item:       `/canvas/scenes/{id}`
//!
//! Reads require workspace `Viewer`; mutations require workspace `Editor`. The
//! `Feature::Canvas` capability axis is enforced upstream by the server's
//! deny-by-default policy middleware.

pub mod http;
pub mod types;

pub use http::{router, CanvasCtx};
