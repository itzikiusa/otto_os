//! The `post_review_comments` workflow node (engine.md §7.3, DR-004): the
//! step itself is `otto_workflows::post_comments`; this hands it the forge
//! and the one-draft post (inline, falling back to a general comment when the
//! forge rejects the anchor).

use super::super::*;
use super::NodeCall;
use otto_core::domain::ReviewComment;

struct Host<'a> {
    ctx: &'a ServerCtx,
    user: &'a User,
}

#[async_trait::async_trait]
impl otto_workflows::post_comments::PostHost for Host<'_> {
    async fn forge(
        &self,
        repo_id: &str,
    ) -> Result<(
        std::sync::Arc<dyn otto_git::GitProvider>,
        otto_git::RemoteRef,
    )> {
        let repo = self.ctx.git_store.get_repo(&repo_id.into()).await?;
        crate::modules::resolve_provider_remote(self.ctx, self.user, &repo).await
    }
    async fn post(
        &self,
        repo_id: &str,
        number: u64,
        c: &ReviewComment,
    ) -> std::result::Result<(), String> {
        // otto_workflows::post_comments claims each comment itself (at most
        // once per run); this is only the forge call, with the run's real PR
        // number (a workflow review is stored as PR 0, DR-004).
        let repo = self
            .ctx
            .git_store
            .get_repo(&repo_id.into())
            .await
            .map_err(|e| e.to_string())?;
        crate::modules::review::post::post_review_comment(self.ctx, self.user, &repo, number, c)
            .await
    }
}

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let host = Host {
        ctx: call.ctx,
        user: call.user,
    };
    let env = call.env;
    otto_workflows::post_comments::run(&host, &call.ctx.pool, &env.files, &env.run_id, call.node)
        .await
}

#[cfg(test)]
#[path = "post_review_comments_tests.rs"]
mod tests;
