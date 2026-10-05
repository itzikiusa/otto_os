//! otto-canvas — Canvas Studio REST router.
//!
//! A visual scene (sketches, UML, sequence/flow diagrams, code/JSON blocks,
//! shapes) stored as ONE portable JSON document. The Rust side owns CRUD over
//! the document + metadata; the rich Scene schema and rendering live in the UI.
//! The agent-assisted "draw it for me" endpoints live in [`assist`]; the
//! agent turn itself is run by the host through [`CanvasAssistCtx`] (otto-server
//! owns the session manager + turn runner and mounts those two routes).
//!
//! Two-tier routing (server nests this under `/api/v1`):
//!   - Collection: `/workspaces/{ws}/canvas/scenes`
//!   - Item:       `/canvas/scenes/{id}`
//!
//! Reads require workspace `Viewer`; mutations require workspace `Editor`. The
//! `Feature::Canvas` capability axis is enforced upstream by the server's
//! deny-by-default policy middleware.

pub mod assist;
pub mod assist_ctx;
pub mod http;
pub mod types;

pub use assist_ctx::{AgentTurn, CanvasAssistCtx};
pub use http::{router, CanvasCtx};
