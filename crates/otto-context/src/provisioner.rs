//! The `PreSpawnHook` implementation: materializes a workspace's active
//! context for a provider into its out-of-tree bundle just before a session
//! spawns, and returns the launch injection the CLI needs to load it.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use otto_core::domain::Workspace;
use otto_core::hooks::{PreSpawnHook, SpawnInjection};

use crate::config;
use crate::library::Library;
use crate::materialize;

/// A source of the Vault "Repo Brain" — a compact, ranked context block (indexed
/// repos + key dependencies + relevant symbols/knowledge/git) for a session.
/// Implemented by the memory service (`otto-memory`). Synchronous so the
/// `PreSpawnHook` stays sync; the impl bridges to its async recall internally.
pub trait RepoBrainSource: Send + Sync {
    /// Best-effort repo-brain markdown for `(workspace, cwd, focus)`. `None`
    /// (or empty) → nothing injected.
    fn brain_markdown(&self, workspace_id: &str, cwd: &str, focus: &str) -> Option<String>;
}

/// Materializes Otto's library into a provider's out-of-tree context bundle at
/// spawn time. Bundles live under `ctx_root` (`~/.otto/context` by default) —
/// the user's working tree is never touched. When a [`RepoBrainSource`] is wired
/// in, a Vault "Repo Brain" block is appended so EVERY agent session gets the
/// right repo brain, not just Product.
pub struct Provisioner {
    library: Library,
    ctx_root: PathBuf,
    brain: OnceLock<Arc<dyn RepoBrainSource>>,
}

impl Provisioner {
    /// Construct with the default bundle root (`~/.otto/context`).
    pub fn new(library: Library) -> Self {
        Self { library, ctx_root: materialize::default_context_root(), brain: OnceLock::new() }
    }

    /// Construct with an explicit bundle root (used by tests).
    pub fn with_root(library: Library, ctx_root: PathBuf) -> Self {
        Self { library, ctx_root, brain: OnceLock::new() }
    }

    /// Wire the Vault repo-brain source (once). Set after boot when the memory
    /// service exists; subsequent calls are ignored.
    pub fn set_brain_source(&self, src: Arc<dyn RepoBrainSource>) {
        let _ = self.brain.set(src);
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
        // Append the Vault Repo Brain (best-effort) so every agent gets it.
        if let Some(src) = self.brain.get() {
            if let Some(block) = src.brain_markdown(&ws.id, cwd, "") {
                if !block.trim().is_empty() {
                    return materialize::append_context_block(&self.ctx_root, cwd, provider, &block);
                }
            }
        }
        injection
    }

    /// Recompute the injection from the persisted bundle (no re-materialize).
    fn resume_injection(&self, cwd: &str, provider: &str) -> SpawnInjection {
        materialize::resume_injection(&self.ctx_root, cwd, provider)
    }
}
