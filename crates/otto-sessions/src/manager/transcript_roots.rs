//! Where a session's transcript lives: the one
//! [`otto_transcript::roots::TranscriptRoots::for_session`] rule for a
//! session (architecture-6w1-01) and the best-effort persist of the resolved
//! `sessions.transcript_path` once its provider id is captured.

use otto_core::domain::Session;
use otto_core::Id;
use otto_state::SessionsRepo;

use super::SessionManager;

/// Best-effort: resolve + persist `sessions.transcript_path` right after a
/// provider id is captured (design §4.2), so the conversation view never has to
/// scan `~/.codex/sessions` for this session again. Failures only log.
///
/// `roots` is [`SessionManager::transcript_roots_for`] — the one
/// [`otto_transcript::roots::TranscriptRoots::for_session`] rule
/// (architecture-6w1-01): an explicit `OTTO_TRANSCRIPT_ROOTS` pair or the
/// isolated fixture dirs win, then the session's account home, then the
/// local defaults. This never reads `$HOME` or builds a Claude layout itself.
pub(super) async fn persist_transcript_path(
    repo: &SessionsRepo,
    id: &Id,
    provider: &str,
    cwd: &str,
    psid: &str,
    roots: otto_transcript::roots::TranscriptRoots,
) {
    // Transcript resolution walks `~/.codex/sessions` / `~/.claude/projects`
    // with sync fs calls — run it on the blocking pool, not a runtime worker.
    let (provider_s, cwd_s, psid_s) = (provider.to_string(), cwd.to_string(), psid.to_string());
    let resolved = match tokio::task::spawn_blocking(move || {
        crate::lifecycle::transcript_path_in(&roots, &provider_s, &cwd_s, Some(&psid_s))
    })
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(session = %id, "transcript path resolve task failed: {e}");
            return;
        }
    };
    match resolved {
        Ok(path) => {
            if let Err(e) = repo.set_transcript_path(id, &path.to_string_lossy()).await {
                tracing::debug!(session = %id, "transcript path persist failed: {e}");
            }
        }
        Err(why) => tracing::debug!(session = %id, ?why, "transcript path not resolvable yet"),
    }
}

impl SessionManager {
    /// Canonical built-in CLI family (`"claude"`, `"codex"`, `"agy"`, `"pi"`,
    /// `"opencode"`, `"grok"`, …) for a session or custom provider slug.
    pub fn cli_family(&self, provider: &str) -> String {
        self.providers
            .model_family_for(provider)
            .unwrap_or_else(|| provider.to_string())
    }

    /// Effective custom `CLAUDE_CONFIG_DIR` for `provider` under `$HOME`, when
    /// `provider` is a Claude-family provider pointing outside `~/.claude`.
    pub fn claude_config_dir_for(&self, provider: &str) -> Option<std::path::PathBuf> {
        self.providers
            .claude_config_dir_for(provider, &otto_transcript::roots::home_dir())
    }

    /// Provider CLI home directories (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`,
    /// `GEMINI_CLI_HOME`, plus an inferred custom `CLAUDE_CONFIG_DIR`) that
    /// Seatbelt should re-open for `session` (`apply_sandbox`).
    pub(super) fn custom_provider_homes(
        &self,
        session: &Session,
        spec: &otto_pty::CommandSpec,
        home: &std::path::Path,
    ) -> Vec<std::path::PathBuf> {
        let mut out: Vec<std::path::PathBuf> = Vec::new();
        for key in ["CODEX_HOME", "CLAUDE_CONFIG_DIR", "GEMINI_CLI_HOME"] {
            if let Some(p) = spec
                .env
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .and_then(|(_, v)| otto_transcript::claude_config::expand_home_path(v, home))
                .filter(|p| p.is_absolute())
            {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        if session
            .meta
            .get("account_id")
            .and_then(|v| v.as_str())
            .is_none()
        {
            if let Some(dir) = self
                .providers
                .claude_config_dir_for(&session.provider, home)
            {
                if dir.is_absolute() && !out.contains(&dir) {
                    out.push(dir);
                }
            }
        }
        out
    }

    /// Every transcript root this session's CLI writes to, from the process
    /// environment ([`Self::transcript_roots_in`]).
    pub(crate) fn transcript_roots_for(
        &self,
        session: &Session,
    ) -> otto_transcript::roots::TranscriptRoots {
        self.transcript_roots_in(&otto_transcript::roots::RootsEnv::from_process(), session)
    }

    /// [`otto_transcript::roots::TranscriptRoots::for_session`] for `session`
    /// against injected inputs: its account home (`meta.account_id`) under
    /// this manager's provider-accounts root, the explicit / isolated roots,
    /// else the local defaults under `$HOME` (plus any custom
    /// `CLAUDE_CONFIG_DIR` configured or inferred for `session.provider`).
    pub(crate) fn transcript_roots_in(
        &self,
        env: &otto_transcript::roots::RootsEnv,
        session: &Session,
    ) -> otto_transcript::roots::TranscriptRoots {
        let home = otto_transcript::roots::home_dir();
        let accounts = self.provider_accounts.as_ref().map(|(_, r)| r.as_path());
        let roots = env.for_session(&session.meta, &home, accounts, self.data_dir.as_deref());
        if !env.is_overridden(self.data_dir.as_deref())
            && session
                .meta
                .get("account_id")
                .and_then(|v| v.as_str())
                .is_none()
        {
            if let Some(dir) = self
                .providers
                .claude_config_dir_for(&session.provider, &home)
            {
                return roots.with_custom_claude_config_dir(&session.provider, &dir);
            }
        }
        roots
    }
}
