//! `otto-mcp` — the MCP Control Plane engine.
//!
//! Otto here is an **outbound MCP client** (it connects out to registered MCP
//! servers — stdio or remote HTTP), and the home of the single **governance
//! pipeline** (`McpService::invoke`) every governed tool call funnels through:
//!
//!   allowlist → per-tool permission → policy-as-code → risk/approval gate →
//!   dry-run → execute → (guaranteed) audit → stats.
//!
//! See `docs/features/mcp-control-plane-design.md` (incl. §14 review resolutions)
//! for the threat model and the requirement traceability. The HTTP surface
//! (`api_router`) is wired into `otto-server` via the `McpCtx` trait.

pub mod client;
pub mod http;
pub mod policy;
pub mod risk;
pub mod service;
pub mod types;

pub use http::{api_router, outcome_to_resp, McpCtx};
pub use service::{canonical_hash, InvokeCtx, InvokeOutcome, McpService};
