//! Posting one stored review comment to its PR — the approve route's core,
//! shared with the workflow `post_review_comments` node (DR-004), which
//! passes the run's real PR number (a workflow review is stored as PR 0).

use super::*;

/// What [`post_one`] did with a comment.
pub(crate) enum PostOutcome {
    /// This call put it on the PR (inline, or as a general comment when the
    /// forge refused the anchor).
    Posted(ReviewComment),
    /// Already on the PR (an earlier run, a concurrent request holding the
    /// claim): never re-posted.
    AlreadyPosted(ReviewComment),
    /// A person declined it: not posted.
    Declined,
    /// The forge refused it (auth, network, 5xx): its error text. The comment
    /// is Approved and unposted (claim released), so a later call can retry.
    Failed(ReviewComment, String),
}

/// Approve comment `comment_id` and post it to PR `pr_number` **at most
/// once**: the post is claimed atomically (posted 0→1) before the forge
/// call, re-checked for a racing decline under the claim, and released on
/// failure. `record_learning` (the approve route: a person's decision)
/// appends the repo's review file when this call posted and queues the
/// approval for learning; a workflow's automated post does neither.
/// `Err` is the store only.
pub(crate) async fn post_one(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    comment_id: &Id,
    pr_number: u64,
    record_learning: bool,
) -> Result<PostOutcome> {
    let comment = ctx.reviews_store.get_comment(comment_id).await?;
    if comment.state == CommentState::Declined {
        return Ok(PostOutcome::Declined);
    }
    let review = ctx.reviews_store.get_review(&comment.review_id).await?;
    let repo = ctx.git_store.get_repo(&review.repo_id).await?;
    let posted = if comment.posted || !ctx.reviews_store.claim_comment_post(comment_id).await? {
        None
    } else if ctx.reviews_store.get_comment(comment_id).await?.state == CommentState::Declined {
        ctx.reviews_store.release_comment_post(comment_id).await?;
        return Ok(PostOutcome::Declined);
    } else {
        let sent = post_review_comment(ctx, user, &repo, pr_number, &comment).await;
        if sent.is_err() {
            let _ = ctx.reviews_store.release_comment_post(comment_id).await;
        } else if record_learning {
            if let Err(e) = append_to_review_file(&repo.path, review.pr_number, &comment).await {
                tracing::warn!(comment = %comment_id, "failed to append to review file: {e}");
            }
        }
        Some(sent)
    };
    let ok = !matches!(posted, Some(Err(_)));
    let updated = ctx
        .reviews_store
        .set_comment_state(comment_id, CommentState::Approved, ok)
        .await?;
    if record_learning {
        queue_review_learning(ctx, &repo.workspace_id, &updated);
    }
    Ok(match posted {
        None => PostOutcome::AlreadyPosted(updated),
        Some(Ok(())) => PostOutcome::Posted(updated),
        Some(Err(e)) => PostOutcome::Failed(updated, e),
    })
}

/// Post one approved review comment to its PR. Inline first; when the forge
/// rejects the ANCHOR (GitHub 422 "line is not part of the diff" → `Conflict`,
/// GitLab/Bitbucket 400 on an invalid position) it falls back to a general
/// comment that cites `path:line`, instead of silently leaving the approved
/// finding unposted. Any other failure (auth, network, 5xx) is NOT retried —
/// a 5xx may have created the comment, and a second call would duplicate it.
pub(crate) async fn post_review_comment(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    repo: &otto_core::domain::Repo,
    pr_number: u64,
    comment: &ReviewComment,
) -> std::result::Result<(), String> {
    let (provider, remote) = match resolve_provider_remote(ctx, user, repo).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(comment = %comment.id, "failed to resolve provider for approve: {e}");
            return Err(e.to_string());
        }
    };
    let req = NewPrCommentReq {
        body: comment.body.clone(),
        path: comment.path.clone(),
        line: comment.line,
        in_reply_to: None,
        side: None,
        old_line: None,
        commit_id: None,
    };
    let err = match provider.comment(&remote, pr_number, &req).await {
        Ok(_) => return Ok(()),
        Err(e) => e,
    };
    let inline = comment.path.is_some() && comment.line.is_some();
    if !(inline && inline_anchor_rejected(&err)) {
        tracing::warn!(comment = %comment.id, "failed to post comment to PR: {err}");
        return Err(err.to_string());
    }
    tracing::info!(comment = %comment.id, "inline anchor rejected ({err}); posting as a general comment");
    let general = NewPrCommentReq {
        body: general_comment_body(comment),
        path: None,
        line: None,
        in_reply_to: None,
        side: None,
        old_line: None,
        commit_id: None,
    };
    match provider.comment(&remote, pr_number, &general).await {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::warn!(comment = %comment.id, "failed to post fallback comment to PR: {e}");
            Err(e.to_string())
        }
    }
}

#[cfg(test)]
#[path = "post_tests.rs"]
mod tests;
