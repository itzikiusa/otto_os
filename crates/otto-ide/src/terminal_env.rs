//! The project environment an IDE terminal starts with (the session manager's
//! [`TerminalEnvSource`], wired at daemon boot).

use otto_core::auth::BoxFuture;
use otto_core::hooks::TerminalEnvSource;

use crate::types::IdeTargetSpec;
use crate::IdeCtx;

/// The same merged env a Run gets, for a root the session owner may open.
pub struct IdeTerminalEnv<S>(pub S);

/// The source the daemon hands its session manager.
pub fn source<S: IdeCtx>(ctx: S) -> std::sync::Arc<dyn TerminalEnvSource> {
    std::sync::Arc::new(IdeTerminalEnv(ctx))
}

impl<S: IdeCtx> TerminalEnvSource for IdeTerminalEnv<S> {
    fn ide_terminal_env<'a>(
        &'a self,
        user_id: &'a str,
        root: &'a str,
    ) -> BoxFuture<'a, Vec<(String, String)>> {
        Box::pin(async move {
            let ctx = &self.0;
            let Ok(user) = otto_state::UsersRepo::new(ctx.pool())
                .get(&user_id.to_string())
                .await
            else {
                return Vec::new();
            };
            let target = IdeTargetSpec::local(root.to_string());
            if ctx
                .authorize_local_target(&user, &target, false)
                .await
                .is_err()
            {
                return Vec::new();
            }
            ctx.ide()
                .environments
                .build_effective_env_for_run_as(
                    &ctx.pool(),
                    ctx.secrets(),
                    &target,
                    None,
                    user.is_root,
                )
                .await
                .map(|env| env.into_iter().collect())
                .unwrap_or_default()
        })
    }
}
