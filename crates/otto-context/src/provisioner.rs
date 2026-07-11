//! The `PreSpawnHook` implementation: materializes a workspace's active
//! context for a provider into its out-of-tree bundle just before a session
//! spawns, and returns the launch injection the CLI needs to load it.

use std::path::PathBuf;

use otto_core::domain::Workspace;
use otto_core::hooks::{PreSpawnHook, SpawnInjection};

use crate::config;
use crate::library::Library;
use crate::materialize;

/// Materializes Otto's library into a provider's out-of-tree context bundle at
/// spawn time. Bundles live under `ctx_root` (`~/.otto/context` by default) —
/// the user's working tree is never touched.
pub struct Provisioner {
    library: Library,
    ctx_root: PathBuf,
}

impl Provisioner {
    /// Construct with the default bundle root (`~/.otto/context`).
    pub fn new(library: Library) -> Self {
        Self { library, ctx_root: materialize::default_context_root() }
    }

    /// Construct with an explicit bundle root (used by tests).
    pub fn with_root(library: Library, ctx_root: PathBuf) -> Self {
        Self { library, ctx_root }
    }
}

impl PreSpawnHook for Provisioner {
    /// Best-effort: builds the workspace context config, materializes it for
    /// `provider` into its bundle, and returns the launch injection.
    /// `materialize::provision` logs its own errors and never panics, so a
    /// provisioning failure never blocks the spawn (it just yields empty
    /// injection).
    fn before_spawn(&self, ws: &Workspace, cwd: &str, provider: &str) -> SpawnInjection {
        let cfg = config::from_settings(&ws.settings);
        let (_result, injection) =
            materialize::provision(&self.library, &cfg, cwd, provider, &self.ctx_root);
        injection
    }

    /// Recompute the injection from the persisted bundle (no re-materialize).
    fn resume_injection(&self, cwd: &str, provider: &str) -> SpawnInjection {
        materialize::resume_injection(&self.ctx_root, cwd, provider)
    }
}
