//! Pre-spawn extension hook.
//!
//! Lets a higher layer (otto-context) materialize a workspace's active skills,
//! soul and context into a CLI's native on-disk form right before a session is
//! spawned — without otto-sessions depending on that layer.

use crate::domain::Workspace;

/// Called by `SessionManager` just before spawning an agent CLI session.
///
/// **Best-effort:** implementations MUST handle their own errors (log, never
/// panic). A hook failure must never block a session from spawning.
pub trait PreSpawnHook: Send + Sync {
    fn before_spawn(&self, ws: &Workspace, cwd: &str, provider: &str);
}

/// One user-configured MCP server to merge into a workspace's `.mcp.json`.
/// A plain transport struct so `otto-sessions` (which writes `.mcp.json`) need
/// not depend on `otto-state` (which persists the rows).
#[derive(Debug, Clone)]
pub struct McpServerSpec {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::BTreeMap<String, String>,
}

/// Resolves the *enabled* user-configured MCP servers for a workspace, queried
/// by `SessionManager` just before an agent spawn so they can be merged into the
/// workspace `.mcp.json` (alongside Otto's own managed entries).
///
/// **Best-effort:** implementations handle their own errors (log, return empty);
/// a failure here must never block a session from spawning, and nothing is ever
/// auto-enabled — only servers the user flipped on are returned.
pub trait McpServerProvider: Send + Sync {
    fn enabled_servers(&self, workspace_id: &str) -> Vec<McpServerSpec>;
}
