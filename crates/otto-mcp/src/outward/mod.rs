//! "Otto as an MCP server" — the OUTWARD surface's engine. External agents
//! (Claude Code, Copilot, …) call the `otto.*` tools over stdio (`ottod
//! mcp-server`) or Streamable HTTP; every call funnels through `otto-server`'s
//! governed choke point (`mcp_outward::governed_invoke`: scope → enable →
//! approval → execute → audit), which needs server state and stays there.
//!
//! What lives here is the part that needs none:
//! - [`catalog`] — the static tool catalog ([`otto_tool_specs`]) and the
//!   policy lists ([`DEFAULT_ENABLED`], [`DANGEROUS`], [`IRREVERSIBLE`]) with
//!   their pure enable / approval decisions;
//! - [`exec`] — [`route_for`] (tool → self-call) and the self-call executor;
//! - [`refs`] — friendly-reference + workspace-pin tables and verdicts;
//! - [`ui_commands`] — the agent UI-control command catalog;
//! - [`jsonrpc`] — the Streamable HTTP JSON-RPC framing behind the narrow
//!   [`OutwardTools`] trait the server implements.

pub mod catalog;
pub mod exec;
pub mod jsonrpc;
pub mod refs;
pub mod ui_commands;

pub use catalog::*;
pub use exec::*;
pub use jsonrpc::OutwardTools;
pub use refs::*;

#[cfg(test)]
mod tests;
