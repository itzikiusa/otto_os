//! The `git_checkout` workflow node (engine.md §7.1): the step itself is
//! `otto_workflows::checkout_run`; this hands it the daemon's repo access.

use super::super::*;
use super::NodeCall;
use otto_git::transport::TransportCredential;

struct Host<'a> {
    ctx: &'a ServerCtx,
    user: &'a User,
}

#[async_trait::async_trait]
impl otto_workflows::checkout_run::CheckoutHost for Host<'_> {
    async fn repo(
        &self,
        id: &str,
    ) -> Option<(otto_core::domain::Repo, Option<TransportCredential>)> {
        let repo = self.ctx.git_store.get_repo(&id.into()).await.ok()?;
        let token =
            crate::modules::review::service::repo_transport_credential(self.ctx, &repo).await;
        Some((repo, token))
    }
    async fn forge(
        &self,
        repo: &otto_core::domain::Repo,
    ) -> std::result::Result<
        (
            std::sync::Arc<dyn otto_git::GitProvider>,
            otto_git::RemoteRef,
        ),
        String,
    > {
        crate::modules::resolve_provider_remote(self.ctx, self.user, repo)
            .await
            .map_err(|e| e.to_string())
    }
}

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let (node, env, scope) = (call.node, call.env, call.scope);
    let base = crate::workflow_context::step_base_name(
        scope.step_no,
        node_display_name(node),
        scope.iter,
        scope.inner_idx,
    );
    let host = Host {
        ctx: call.ctx,
        user: call.user,
    };
    otto_workflows::checkout_run::run(
        &host,
        &env.files,
        &node.params,
        &env.run_input,
        &call.input,
        env.files.step_md_path(&base),
    )
    .await
    .map_err(otto_core::Error::Upstream)
}

#[cfg(test)]
#[path = "git_checkout_tests.rs"]
mod tests;
