//! The PTY spawn's safety pipeline, applied to a structured-runtime child
//! (`otto-agent-protocol`; design `docs/features/threads-structured-runtime.md`
//! §Safety). Same helpers, same order as [`SessionManager::create`]: folder
//! refusal, trust pre-seed, context provisioning, then the Seatbelt wrap as the
//! very last step over the fully-built spec. What the runtime cannot honour
//! yet (named provider accounts, network profiles) is REFUSED, never skipped.

use super::*;

impl SessionManager {
    /// Prepare `spec` (the runtime driver's launch command) for spawning as
    /// agent session `session` in `ws`. `tail_args` go after the context
    /// hook's injected flags (e.g. `agent stdio` for `grok`). `session` is a synthetic, never
    /// persisted [`Session`] (`kind = Agent`) carrying the runtime session's
    /// id, provider, cwd, owner and meta — exactly the fields the PTY path's
    /// helpers read.
    ///
    /// Errors: the cwd is refused (`/`, `$HOME` while confined, the data dir —
    /// [`Self::check_session_cwd`]); the daemon is shutting down; the session
    /// asks for a named provider account or the workspace selects a network
    /// profile (not supported by the runtime yet — refused so traffic and
    /// credentials never silently take a different path than a PTY session's).
    pub async fn prepare_runtime_spawn(
        &self,
        ws: &Workspace,
        session: &Session,
        spec: &mut CommandSpec,
        tail_args: Vec<String>,
    ) -> Result<()> {
        self.refuse_while_shutting_down()?;
        if session.kind != SessionKind::Agent {
            return Err(Error::Invalid("runtime sessions are agent sessions".into()));
        }
        if session.meta.get("account_id").is_some_and(|v| !v.is_null()) {
            return Err(Error::Invalid(
                "named provider accounts are not supported for structured sessions yet".into(),
            ));
        }
        if self.networks.prepare(session).await?.is_some() {
            return Err(Error::Invalid(
                "this workspace routes agents through a network profile, which structured \
                 sessions do not support yet — use a terminal session"
                    .into(),
            ));
        }
        let read_only = session.meta.get("read_only").and_then(|v| v.as_bool()) == Some(true);
        let unconfined = !read_only && !self.process_sandbox_enabled().await;
        self.check_session_cwd(ws, &session.cwd, unconfined)?;
        let _ = std::fs::create_dir_all(&session.cwd);
        crate::trust::ensure_trusted_async(&session.provider, &session.cwd).await;
        if let Some(hook) = &self.pre_spawn_hook {
            let hook = Arc::clone(hook);
            let ws_owned = ws.clone();
            let cwd = session.cwd.clone();
            let prov = session.provider.clone();
            let injection =
                tokio::task::spawn_blocking(move || hook.before_spawn(&ws_owned, &cwd, &prov))
                    .await
                    .unwrap_or_default();
            spec.args.extend(injection.args);
            spec.env.extend(injection.env);
        }
        // A subcommand that must follow the injected top-level flags.
        spec.args.extend(tail_args);
        // Last, over the fully-built spec — as `create` does.
        self.apply_sandbox(spec, session).await;
        Ok(())
    }

    /// Whether `provider` would run Seatbelt-confined (the runtime's drivers
    /// pass this to the provider's own sandbox flags, as
    /// `interaction_mode_apply` does for PTY sessions).
    pub async fn runtime_has_process_sandbox(&self, provider: &str) -> bool {
        self.has_process_sandbox(SessionKind::Agent, provider).await
    }
}
