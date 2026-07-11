//! otto-vault — the Vault v3 docs home: file-backed, Obsidian-parity, OKF-
//! native markdown knowledge vaults. A vault is a registered local directory
//! of markdown files (which may be a live Obsidian vault); SQLite holds only a
//! derived index (notes, links, tags, FTS) rebuildable from disk at any time.
//! No embeddings anywhere — recall is FTS5 + the link graph + agent-authored
//! OKF docs.

pub mod engine;
pub mod http;
pub mod okf;
pub mod parse;
pub mod resolve;
pub mod scan;
pub mod store;
pub mod types;

pub use engine::VaultEngine;
pub use http::{router, VaultCtx};
pub use types::*;
