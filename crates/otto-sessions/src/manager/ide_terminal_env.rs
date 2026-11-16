//! The project environment an IDE terminal starts with (`meta.ide_terminal` +
//! `meta.ide_root`), from the server's [`otto_core::hooks::TerminalEnvSource`].

use super::*;

impl SessionManager {
    /// Wire the IDE terminal env source (see [`Self::ide_terminal_env`]).
    pub fn set_terminal_env(&self, source: Arc<dyn otto_core::hooks::TerminalEnvSource>) {
        let _ = self.terminal_env.set(source);
    }

    /// An IDE terminal's project environment: the IDE project's merged Run env,
    /// so `.env` vars, project vars and the SDK reach the shell as they reach a
    /// Run. Empty for every other session.
    pub(super) async fn ide_terminal_env(&self, session: &Session) -> Vec<(String, String)> {
        let root = session.meta.get("ide_root").and_then(|v| v.as_str());
        let is_ide_terminal = session.provider == "shell"
            && session.meta.get("ide_terminal").and_then(|v| v.as_bool()) == Some(true);
        match (self.terminal_env.get(), root) {
            (Some(source), Some(root)) if is_ide_terminal && !root.is_empty() => {
                source.ide_terminal_env(&session.created_by, root).await
            }
            _ => Vec::new(),
        }
    }
}
