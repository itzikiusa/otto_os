//! Pre-spawn extension hook.
//!
//! Lets a higher layer (otto-context) materialize a workspace's active skills,
//! soul and context into an Otto-owned bundle OUTSIDE the working tree right
//! before a session is spawned — without otto-sessions depending on that layer.
//! The hook returns the launch flags (and env) the CLI needs to load that bundle
//! (e.g. `--add-dir`, `--append-system-prompt-file`, codex `-c
//! developer_instructions=…`), which the manager appends to the spawn command.

use crate::domain::Workspace;

/// Extra launch configuration a provider needs so its CLI loads the out-of-tree
/// context bundle: CLI args appended to the spawn (and resume) command, plus env
/// vars added to the child's environment. Empty when nothing was materialized.
#[derive(Debug, Clone, Default)]
pub struct SpawnInjection {
    /// Args appended to the provider's launch command (after its own flags).
    pub args: Vec<String>,
    /// Env vars added to the child process (e.g. a swapped `CODEX_HOME`).
    pub env: Vec<(String, String)>,
}

/// Called by `SessionManager` just before spawning an agent CLI session.
///
/// **Best-effort:** implementations MUST handle their own errors (log, never
/// panic) and return an empty [`SpawnInjection`] on failure. A hook failure must
/// never block a session from spawning.
pub trait PreSpawnHook: Send + Sync {
    /// Materialize the workspace's active context for `provider` into its
    /// out-of-tree bundle and return the launch injection needed to load it.
    fn before_spawn(&self, ws: &Workspace, cwd: &str, provider: &str) -> SpawnInjection;

    /// Recompute the launch injection for a RESUME, reading the already-
    /// materialized bundle (no `Workspace` is available on the restart path, and
    /// the bundle persists across daemon restarts). Returns empty if no bundle
    /// exists for `(provider, cwd)`.
    fn resume_injection(&self, cwd: &str, provider: &str) -> SpawnInjection;
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
