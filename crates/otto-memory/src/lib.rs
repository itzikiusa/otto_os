//! otto-memory — a workspace-scoped knowledge store with keyword + semantic
//! (vector) hybrid recall. First consumer: the Product section; the core is
//! domain-agnostic (collections), so other areas can adopt it later.

pub mod embed;
pub mod http;
pub mod index;
pub mod ingest;
pub mod remote;
pub mod retrieve;
pub mod service;
pub mod test_support;
pub mod types;
pub mod vault;

pub use http::{router, GraphData, GraphNode, MemoryCtx};
pub use service::MemoryService;
pub use types::*;
