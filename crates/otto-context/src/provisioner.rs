//! The `PreSpawnHook` implementation: materializes a workspace's active
//! context for a provider just before a session spawns.

use otto_core::domain::Workspace;
use otto_core::hooks::PreSpawnHook;

use crate::config;
use crate::library::Library;
use crate::materialize;

/// Materializes Otto's library into a workspace's CLI-native form at spawn time.
pub struct Provisioner {
    library: Library,
}

impl Provisioner {
    pub fn new(library: Library) -> Self {
        Self { library }
    }
}

impl PreSpawnHook for Provisioner {
    /// Best-effort: builds the workspace context config and materializes it for
    /// `provider`. `materialize::provision` logs its own errors and never
    /// panics, so a provisioning failure never blocks the spawn.
    fn before_spawn(&self, ws: &Workspace, cwd: &str, provider: &str) {
        let cfg = config::from_settings(&ws.settings);
        let _ = materialize::provision(&self.library, &cfg, cwd, provider);
    }
}
