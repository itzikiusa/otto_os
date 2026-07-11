//! otto-memory — a workspace-scoped agent knowledge store with keyword (FTS5)
//! recall. First consumer: the Product section; the core is domain-agnostic
//! (collections), so other areas can adopt it later. The file-backed docs home
//! (Obsidian-style markdown vaults, OKF) lives in the `otto-vault` crate.

pub mod governance;
pub mod http;
pub mod ingest;
pub mod remote;
pub mod retrieve;
pub mod service;
pub mod test_support;
pub mod types;
pub mod vault;

pub use governance::{
    ForgetResp, ImportReq, ImportResp, MergeReq, MergeResp, SetStateReq, SplitPart, SplitReq,
    SplitResp, UndoForgetReq,
};
pub use http::{router, GraphData, GraphNode, MemoryCtx};
pub use service::MemoryService;
pub use types::*;
