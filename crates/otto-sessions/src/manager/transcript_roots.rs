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
    /// else the local defaults under `$HOME`.
    pub(crate) fn transcript_roots_in(
        &self,
        env: &otto_transcript::roots::RootsEnv,
        session: &Session,
    ) -> otto_transcript::roots::TranscriptRoots {
        let accounts = self.provider_accounts.as_ref().map(|(_, r)| r.as_path());
        env.for_session(
            &session.meta,
            &otto_transcript::roots::home_dir(),
            accounts,
            self.data_dir.as_deref(),
        )
    }
}
