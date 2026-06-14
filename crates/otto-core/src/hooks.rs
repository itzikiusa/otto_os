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
