//! The PR / local review HTTP routes, the findings + comment actions, merge
//! readiness and the review-config routes.

use super::*;

/// Routes under /api/v1 for PR review agents (PR + local).
pub fn pr_review_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/repos/{id}/prs/{number}/review",
            post(start_review).get(get_review),
        )
        .route("/repos/{id}/prs/{number}/reviews", get(list_reviews))
        .route("/repos/{id}/local-reviews", get(list_local_reviews))
        .route("/pr-review-comments/{cid}/approve", post(approve_comment))
        .route("/pr-review-comments/{cid}/decline", post(decline_comment))
        .route("/pr-review-comments/{cid}", patch(edit_review_comment))
        .route(
            "/repos/{id}/local-review",
            post(start_local_review).get(get_local_review),
        )
        .route("/reviews/{review_id}", get(get_review_by_id))
        .route("/reviews/{review_id}/handoff", post(handoff_review))
        .route("/reviews/{review_id}/cancel", post(cancel_review))
        .route(
            "/reviews/{review_id}/agents/{index}/retry",
            post(retry_review_agent),
        )
        .route(
            "/reviews/{review_id}/summarizer/retry",
            post(retry_summarizer),
        )
        .route(
            "/reviews/{review_id}/agents/{index}/stop",
            post(stop_review_agent),
        )
        .route("/repos/{id}/pr/draft", post(draft_pr))
        .route(
            "/repos/{id}/draft-commit-message",
            post(draft_commit_message),
        )
        // A1 verified-review loop: findings list, lifecycle state update, and
        // merge-readiness assembly. Registered here (not in routes/mod.rs) so
        // they share the review-module handler context.
        .route("/reviews/{review_id}/findings", get(list_review_findings))
        .route(
            "/reviews/{review_id}/findings/{fingerprint}/state",
            post(set_finding_state),
        )
        .route(
            "/reviews/{review_id}/merge-readiness",
            get(get_merge_readiness),
        )
        // PR-keyed twin of the row above: the merge modal asks by (repo, PR)
        // and gets the same numbers whether or not a review run exists.
        .route("/repos/{id}/prs/{number}/readiness", get(get_pr_readiness))
}

// ---------------------------------------------------------------------------
// A1 Verified review loop — findings + merge-readiness handlers
// ---------------------------------------------------------------------------

/// `GET /reviews/{review_id}/findings` — list all persistent findings for a
/// review run, keyed by fingerprint with their current lifecycle state.
pub(crate) async fn list_review_findings(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<otto_core::finding::Finding>>> {
    // Resolve the review to check workspace access.
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    // Widened to the full workflow `Finding` (superset of the legacy row: all old
    // fields retained + the workflow fields added). See docs/contracts/api.md.
    let findings = ctx
        .findings_store
        .list_full_for_review(&review_id)
        .await
        .map_err(ApiError)?;
    Ok(Json(findings))
}

/// `POST /reviews/{review_id}/findings/{fingerprint}/state`
/// Body: `{ "state": "open"|"fixing"|"resolved"|"regressed"|"declined", "fix_session_id"?: string }`
/// — update the lifecycle state of a finding identified by its fingerprint.
#[derive(serde::Deserialize)]
pub(crate) struct SetFindingStateReq {
    pub(crate) state: String,
    #[serde(default)]
    pub(crate) fix_session_id: Option<String>,
}

pub(crate) async fn set_finding_state(
    Path((review_id, fingerprint)): Path<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<SetFindingStateReq>,
) -> ApiResult<Json<otto_state::ReviewFindingRow>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    let new_state = otto_state::FindingState::parse(&body.state).ok_or_else(|| {
        ApiError(Error::Invalid(format!(
            "unknown finding state: {}",
            body.state
        )))
    })?;
    let row = ctx
        .findings_store
        .set_state(
            &review_id,
            &fingerprint,
            new_state,
            body.fix_session_id.as_deref(),
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(row))
}

/// PR merge readiness: findings (only when a review run exists), the live
/// provider numbers, and the two facts only the local checkout knows.
#[derive(Debug, serde::Serialize)]
pub(crate) struct PrReadiness {
    /// Aggregate CI state as the provider reports it ("none" when unknown).
    pub ci_status: String,
    pub approvals: u64,
    pub mergeable: Option<bool>,
    pub conflicts: bool,
    /// Findings block — `None` when no review has ever run for this PR.
    pub review: Option<ReadinessReview>,
    /// Commits on the local source branch that `origin/<source>` does not have;
    /// `None` when the branch is not checked out locally.
    pub unpushed: Option<u64>,
    /// `fresh` (the target's tip is already an ancestor of the source),
    /// `behind`, or `unknown` when either ref is missing locally.
    pub branch_freshness: &'static str,
}

/// The findings half of [`PrReadiness`], keyed to the review run it came from.
#[derive(Debug, serde::Serialize)]
pub(crate) struct ReadinessReview {
    pub review_id: Id,
    pub unresolved_total: u64,
    pub unresolved_blocker_count: u64,
    pub total_findings: u64,
}

/// `unpushed` = commits on the local `source` not on `origin/<source>` (None
/// when the branch is not checked out locally); `branch_freshness` = whether
/// `origin/<target>` is an ancestor of `source` (fresh) or not (behind);
/// "unknown" when either ref is missing.
pub(crate) async fn local_branch_facts(
    repo_path: &str,
    source: &str,
    target: &str,
) -> (Option<u64>, &'static str) {
    let git = otto_git::LocalGit::new(repo_path);
    // `is_ancestor_of` guard_refs BOTH arguments; run it first so an
    // option-like provider value never reaches the `rev-list` argv below.
    let fresh = match git
        .is_ancestor_of(&format!("origin/{target}"), source)
        .await
    {
        Ok(true) => "fresh",
        Ok(false) => "behind",
        Err(otto_core::Error::Invalid(_)) => return (None, "unknown"),
        Err(_) => "unknown",
    };
    if !git.branch_exists(source).await {
        return (None, "unknown");
    }
    let unpushed = git
        .run(&[
            "rev-list",
            "--count",
            &format!("origin/{source}..{source}"),
            "--",
        ])
        .await
        .ok()
        .and_then(|s| s.trim().parse().ok());
    (unpushed, fresh)
}

/// Shared body of both readiness routes. `review` is `Some` only when a review
/// run exists for the PR; the provider and local-checkout blocks are
/// best-effort and degrade to "unknown" rather than failing the request.
pub(crate) async fn compute_readiness(
    ctx: &ServerCtx,
    user: &User,
    repo: &otto_core::domain::Repo,
    pr_number: u64,
    review: Option<&Review>,
) -> ApiResult<PrReadiness> {
    // Findings aggregate from the persistent findings store, keyed on the
    // WORKFLOW `status` axis (§15.1): unresolved = open|accepted|fixed; a
    // blocker is an unresolved critical/high finding.
    use otto_core::finding::{FindingSeverity, FindingStatus};
    let review_block = match review {
        None => None,
        Some(rev) => {
            let all_findings = ctx
                .findings_store
                .list_full_for_review(&rev.id)
                .await
                .map_err(ApiError)?;
            let is_unresolved = |s: FindingStatus| {
                matches!(
                    s,
                    FindingStatus::Open | FindingStatus::Accepted | FindingStatus::Fixed
                )
            };
            Some(ReadinessReview {
                review_id: rev.id.clone(),
                unresolved_total: all_findings
                    .iter()
                    .filter(|f| is_unresolved(f.status))
                    .count() as u64,
                unresolved_blocker_count: all_findings
                    .iter()
                    .filter(|f| {
                        is_unresolved(f.status)
                            && matches!(
                                f.severity,
                                FindingSeverity::Critical | FindingSeverity::High
                            )
                    })
                    .count() as u64,
                total_findings: all_findings.len() as u64,
            })
        }
    };

    // Best-effort: live PR detail for approvals + ci_status + mergeable. This
    // call may fail (rate-limits, no token); silently degrade.
    let detail = match resolve_provider_remote(ctx, user, repo).await {
        Ok((provider, remote_ref)) => provider.get_pr(&remote_ref, pr_number).await.ok(),
        Err(_) => None,
    };

    // The local facts are named by the PR's branches, so without a detail there
    // is no ref to probe.
    let (unpushed, branch_freshness) = match &detail {
        Some(d) => {
            local_branch_facts(
                &repo.path,
                &d.summary.source_branch,
                &d.summary.target_branch,
            )
            .await
        }
        None => (None, "unknown"),
    };

    Ok(PrReadiness {
        ci_status: detail
            .as_ref()
            .and_then(|d| d.summary.ci_status.clone())
            .unwrap_or_else(|| "none".to_string()),
        approvals: detail
            .as_ref()
            .map(|d| d.approved_by.len() as u64)
            .unwrap_or(0),
        mergeable: detail.as_ref().and_then(|d| d.mergeable),
        conflicts: false,
        review: review_block,
        unpushed,
        branch_freshness,
    })
}

/// `GET /reviews/{review_id}/merge-readiness` — assemble the full merge-readiness
/// picture: open/total findings from `review_merge_readiness` view, the PR's
/// ci_status, approvals, and mergeable flag from the provider.
pub(crate) async fn get_merge_readiness(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let r = compute_readiness(&ctx, &user, &repo, review.pr_number, Some(&review)).await?;
    // `review` is Some by construction here, so the findings numbers are real;
    // the fallbacks only keep the JSON shape total.
    let (unresolved_total, blocker_count, total_findings) = r
        .review
        .as_ref()
        .map(|rv| {
            (
                rv.unresolved_total,
                rv.unresolved_blocker_count,
                rv.total_findings,
            )
        })
        .unwrap_or((0, 0, 0));

    Ok(Json(serde_json::json!({
        "unresolved_total": unresolved_total,
        "unresolved_blocker_count": blocker_count,
        "total_findings": total_findings,
        "resolved_count": total_findings.saturating_sub(unresolved_total),
        "ci_status": r.ci_status,
        "approvals": r.approvals,
        "mergeable": r.mergeable,
        "conflicts": r.conflicts,
        "branch_freshness": r.branch_freshness,
        "unpushed": r.unpushed,
    })))
}

/// `GET /repos/{id}/prs/{number}/readiness` — the same picture keyed by PR
/// instead of by review run: the merge modal opens straight from a PR, which
/// may never have been reviewed in Otto.
pub(crate) async fn get_pr_readiness(
    Path((id, number)): Path<(Id, u64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<PrReadiness>> {
    let repo = ctx.git_store.get_repo(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;
    let review = ctx
        .reviews_store
        .latest_for_pr(&repo.id, number)
        .await
        .map_err(ApiError)?;
    Ok(Json(
        compute_readiness(&ctx, &user, &repo, number, review.as_ref()).await?,
    ))
}

/// Point-of-action budget gate for EVERY path that spawns reviewer sessions —
/// PR and local reviews, agent/summarizer retries and branch reviews (Run with
/// Otto, workflow `review_run`); it used to guard the PR path only. Checks
/// the workspace-level cap (the provider isn't known yet). A blocked budget is
/// `Invalid` (400) like every other budget gate in the daemon.
pub(crate) async fn review_budget_gate(ctx: &ServerCtx, workspace_id: &str) -> Result<()> {
    let verdict = crate::routes::usage::check_budget(ctx, workspace_id, "").await;
    if verdict.blocked {
        return Err(Error::Invalid(format!(
            "Budget exceeded — review blocked: {}",
            verdict.reason.unwrap_or_else(|| "cap reached".to_string())
        )));
    }
    Ok(())
}

#[derive(serde::Deserialize)]
pub(crate) struct RepoPrPath {
    pub(crate) id: Id,
    pub(crate) number: u64,
}

pub(crate) async fn start_review(
    Path(RepoPrPath {
        id: repo_id,
        number,
    }): Path<RepoPrPath>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<StartReviewReq>>,
) -> crate::error::ApiResult<Json<Review>> {
    // Resolve workspace role via the repo.
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    let req = body.map(|b| b.0).unwrap_or_default();

    // Point-of-action budget gate (A1/A2), before touching the DB.
    review_budget_gate(&ctx, &repo.workspace_id)
        .await
        .map_err(ApiError)?;

    // Create the review row (status=running) and return it immediately.
    let review = ctx
        .reviews_store
        .create_review(&repo_id, number)
        .await
        .map_err(crate::error::ApiError)?;

    // Register a cancel flag for this run so POST /reviews/{id}/cancel can stop
    // it. run_review_core looks this up by review_id and threads it into each
    // agent's recovery loop.
    let registration = ctx.review_cancels.register(&review.id);

    // Spawn the background runner.
    let review_id = review.id.clone();
    let ctx_bg = ctx.clone();
    let repo_path = repo.path.clone();
    let ws_id = repo.workspace_id.clone();
    let issue_account_id = req.issue_account_id;
    let issue_key = req.issue_key;
    let user_context = req.context;
    // Carry the caller into the background task so credential-ownership (S4) is
    // enforced against the user who started the review, not whoever happens to
    // own the repo's bound accounts.
    let review_user = user.clone();
    // Notify subscribers that a review is now running.
    announce(&ctx, &ws_id, &review_id, ReviewStatus::Running);
    tokio::spawn(async move {
        let result = run_pr_review_inner(
            &ctx_bg,
            &review_user,
            &review_id,
            &repo_id,
            number,
            issue_account_id,
            issue_key,
            user_context,
        )
        .await;
        // If a Cancel landed while we ran, the cancel handler already set the
        // terminal `cancelled` status + tore everything down — don't overwrite it.
        let was_cancelled = registration.is_cancelled();
        if was_cancelled {
            tracing::info!(review = %review_id, "PR review ended after cancel — leaving cancelled status");
        } else {
            match result {
                Ok(()) => {
                    tracing::info!(review = %review_id, "PR review complete");
                    let _ = transition(&ctx_bg, &ws_id, &review_id, ReviewStatus::Done, None).await;
                }
                Err(e) => {
                    tracing::warn!(review = %review_id, "PR review error: {e}");
                    let msg = e.to_string();
                    let _ =
                        transition(&ctx_bg, &ws_id, &review_id, ReviewStatus::Error, Some(&msg))
                            .await;
                }
            }
        }
        // Drop the cancel flag from the registry now the run is over.
        drop(registration);
        drop(repo_path); // keep bound
    });

    Ok(Json(review))
}

/// Cancel an in-flight review. Signals the run's cancel flag (short-circuiting
/// the agent recovery loop), kills the live agent PTY sessions, marks the run
/// `cancelled`, cleans up its temp files and broadcasts. 409 if not running.
pub(crate) async fn cancel_review(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    if review.status != ReviewStatus::Running {
        return Err(ApiError(Error::Conflict(format!(
            "review is {} — only a running review can be cancelled",
            review.status.as_str()
        ))));
    }

    cancel_running_review(&ctx, &review, &repo.workspace_id).await;
    Ok(Json(
        ctx.reviews_store
            .get_review(&review_id)
            .await
            .map_err(ApiError)?,
    ))
}

pub(crate) async fn get_review_by_id(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(review))
}

pub(crate) async fn get_review(
    Path(RepoPrPath {
        id: repo_id,
        number,
    }): Path<RepoPrPath>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let review = ctx
        .reviews_store
        .latest_for_pr(&repo_id, number)
        .await
        .map_err(crate::error::ApiError)?
        .ok_or_else(|| crate::error::ApiError(Error::NotFound("no review for this PR".into())))?;

    Ok(Json(review))
}

/// Feedback is evidence about the user's disposition, not proof of a defect.
/// The engine checks the workspace opt-in and deduplicates comment/disposition.
pub(crate) fn queue_review_learning(ctx: &ServerCtx, workspace_id: &Id, comment: &ReviewComment) {
    let engine = Arc::clone(&ctx.improve_engine);
    let wid = workspace_id.clone();
    let comment = comment.clone();
    tokio::spawn(async move {
        let disposition = comment.state.as_str();
        let narrative = format!(
            "User disposition: {disposition}. Source: /api/v1/reviews/{}; comment ID: {}.\n\
             Location: {}:{}\nReview comment (untrusted observation):\n{}\n\
             This is feedback on one finding. Approval does not independently prove its technical claim; \
             decline does not prove the inverse or supply a rejection reason. Learn only a narrow, \
             evidence-supported lesson, or propose no change.",
            comment.review_id, comment.id, comment.path.as_deref().unwrap_or("general"),
            comment.line.unwrap_or(0), comment.body);
        if let Err(e) = engine
            .learn_review_feedback(&wid, &comment.id, disposition, &narrative)
            .await
        {
            tracing::warn!(comment = %comment.id, "review feedback learning failed: {e}");
        }
    });
}

pub(crate) async fn approve_comment(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<ReviewComment>> {
    let comment = ctx
        .reviews_store
        .get_comment(&cid)
        .await
        .map_err(crate::error::ApiError)?;
    let review = ctx
        .reviews_store
        .get_review(&comment.review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    // A DECLINED comment is not approvable: a decline that lands while a
    // "Post all" loop is still walking its snapshot must not be posted anyway.
    // Restoring it to draft (`PATCH … {restore_draft:true}`) is the way back.
    if comment.state == CommentState::Declined {
        return Err(crate::error::ApiError(Error::Conflict(
            "comment was declined — restore it to draft before approving".into(),
        )));
    }

    // Post the comment to the PR provider — at most once.
    // - A LOCAL review (pr #0 sentinel) has no PR: approving it is a local
    //   decision; calling the forge only 404'd against "PR #0".
    // - An already-posted comment is never re-posted, and the post is CLAIMED
    //   atomically (posted 0→1) before the forge call, so a double click or a
    //   retried request can't post twice; a failed call releases the claim.
    let pr_posted = if review.pr_number == LOCAL_REVIEW_PR_NUMBER {
        comment.posted
    } else if comment.posted {
        true
    } else if !ctx
        .reviews_store
        .claim_comment_post(&cid)
        .await
        .map_err(crate::error::ApiError)?
    {
        // Another request holds (or completed) the post.
        true
    } else {
        // Re-read under the claim: a decline that raced the check above wins.
        let fresh = ctx.reviews_store.get_comment(&cid).await;
        if matches!(&fresh, Ok(c) if c.state == CommentState::Declined) {
            let _ = ctx.reviews_store.release_comment_post(&cid).await;
            return Err(crate::error::ApiError(Error::Conflict(
                "comment was declined — restore it to draft before approving".into(),
            )));
        }
        let posted = post_review_comment(&ctx, &user, &repo, review.pr_number, &comment).await;
        if !posted {
            let _ = ctx.reviews_store.release_comment_post(&cid).await;
        }
        posted
    };

    // Append to the review markdown file.
    if let Err(e) = append_to_review_file(&repo.path, review.pr_number, &comment).await {
        tracing::warn!(comment = %cid, "failed to append to review file: {e}");
    }

    let updated = ctx
        .reviews_store
        .set_comment_state(&cid, CommentState::Approved, pr_posted)
        .await
        .map_err(crate::error::ApiError)?;
    queue_review_learning(&ctx, &repo.workspace_id, &updated);
    Ok(Json(updated))
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
) -> bool {
    let (provider, remote) = match resolve_provider_remote(ctx, user, repo).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(comment = %comment.id, "failed to resolve provider for approve: {e}");
            return false;
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
        Ok(_) => return true,
        Err(e) => e,
    };
    let inline = comment.path.is_some() && comment.line.is_some();
    if !(inline && inline_anchor_rejected(&err)) {
        tracing::warn!(comment = %comment.id, "failed to post comment to PR: {err}");
        return false;
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
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(comment = %comment.id, "failed to post fallback comment to PR: {e}");
            false
        }
    }
}

pub(crate) async fn decline_comment(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<ReviewComment>> {
    let comment = ctx
        .reviews_store
        .get_comment(&cid)
        .await
        .map_err(crate::error::ApiError)?;
    let review = ctx
        .reviews_store
        .get_review(&comment.review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    // Keep `posted` as-is: declining a comment that is already on the PR does
    // not un-post it, and resetting the flag let a later approve post it AGAIN.
    let updated = ctx
        .reviews_store
        .set_comment_state(&cid, CommentState::Declined, comment.posted)
        .await
        .map_err(crate::error::ApiError)?;
    queue_review_learning(&ctx, &repo.workspace_id, &updated);
    Ok(Json(updated))
}

/// `PATCH /pr-review-comments/{cid}` body.
#[derive(Deserialize)]
pub(crate) struct EditReviewCommentReq {
    /// New body for a DRAFT comment (a person's edit of the agent's draft).
    #[serde(default)]
    pub(crate) body: Option<String>,
    /// Move a DECLINED comment back to draft.
    #[serde(default)]
    pub(crate) restore_draft: bool,
    /// Record an APPROVED-but-unposted comment as posted WITHOUT sending it:
    /// the client found the copy an earlier failed (5xx) attempt created on
    /// the PR, so it must stop being offered as postable (S15-302). Claimed
    /// atomically (posted 0→1), so it can't race a concurrent approve.
    #[serde(default)]
    pub(crate) mark_posted: bool,
}

/// `PATCH /pr-review-comments/{cid}` — a person edits an agent-drafted
/// comment before approving it, or restores a declined one to draft. Never
/// touches a posted comment (409); nothing is sent anywhere.
pub(crate) async fn edit_review_comment(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<EditReviewCommentReq>,
) -> crate::error::ApiResult<Json<ReviewComment>> {
    let comment = ctx
        .reviews_store
        .get_comment(&cid)
        .await
        .map_err(crate::error::ApiError)?;
    let review = ctx
        .reviews_store
        .get_review(&comment.review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    if req.mark_posted {
        if req.body.is_some() || req.restore_draft {
            return Err(crate::error::ApiError(Error::Invalid(
                "`mark_posted` stands alone — no `body` / `restore_draft`".into(),
            )));
        }
        let claimed = comment.state == CommentState::Approved
            && !comment.posted
            && ctx
                .reviews_store
                .claim_comment_post(&cid)
                .await
                .map_err(crate::error::ApiError)?;
        if !claimed {
            return Err(crate::error::ApiError(Error::Conflict(
                "only an approved, unposted comment can be marked posted".into(),
            )));
        }
        return ctx
            .reviews_store
            .get_comment(&cid)
            .await
            .map(Json)
            .map_err(crate::error::ApiError);
    }
    let body = req.body.as_deref().map(str::trim);
    if body.is_some_and(str::is_empty) {
        return Err(crate::error::ApiError(Error::Invalid(
            "comment body cannot be empty".into(),
        )));
    }
    if body.is_none() && !req.restore_draft {
        return Err(crate::error::ApiError(Error::Invalid(
            "nothing to change — send `body` and/or `restore_draft`".into(),
        )));
    }
    ctx.reviews_store
        .edit_unposted_comment(&cid, body, req.restore_draft)
        .await
        .map_err(crate::error::ApiError)?
        .map(Json)
        .ok_or_else(|| {
            crate::error::ApiError(Error::Conflict(
                "only an unposted draft can be edited, and only a declined comment restored".into(),
            ))
        })
}

// ---------------------------------------------------------------------------
// Local review routes
// ---------------------------------------------------------------------------

/// Sentinel pr_number for local reviews (real PRs are ≥ 1).
pub(crate) const LOCAL_REVIEW_PR_NUMBER: u64 = 0;

pub(crate) async fn start_local_review(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<LocalReviewReq>,
) -> crate::error::ApiResult<Json<Review>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    review_budget_gate(&ctx, &repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    let git = otto_git::LocalGit::new(&repo.path);
    // Resolve the user-picked base first: a ref that doesn't exist locally
    // (or an empty one) falls back to `origin/<base>` / the default branch,
    // and a truly unresolvable base reports the candidates tried instead of a
    // raw "git exited 128".
    let want = Some(body.base.as_str()).filter(|s| !s.trim().is_empty());
    let resolved = git
        .resolve_base(want)
        .await
        .map_err(crate::error::ApiError)?;
    let diff_text = match git.review_diff_text(&resolved.diff_ref).await {
        Ok(d) => d,
        Err(e) => {
            return Err(crate::error::ApiError(Error::Invalid(format!(
                "git diff failed: {e}"
            ))));
        }
    };

    // Create the review row.
    let review = ctx
        .reviews_store
        .create_review(&repo_id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .map_err(crate::error::ApiError)?;
    let review_id = review.id.clone();

    if diff_text.trim().is_empty() {
        // No changes vs base — complete immediately with a note.
        let note = format!("No changes vs {}", body.base);
        tracing::info!(review = %review_id, "{note}");
        let ctx_note = ctx.clone();
        let rid = review_id.clone();
        let ws_id_local = repo.workspace_id.clone();
        // Notify that the review started (running) and will complete immediately.
        announce(&ctx, &ws_id_local, &rid, ReviewStatus::Running);
        tokio::spawn(async move {
            // Seed an empty agent list and mark done immediately.
            let _ = transition(&ctx_note, &ws_id_local, &rid, ReviewStatus::Done, None).await;
        });
    } else {
        // Spawn the review core in the background.
        let workspace = ctx
            .workspaces
            .get(&repo.workspace_id)
            .await
            .map_err(crate::error::ApiError)?;
        // Notify that the review runner is starting; run_review emits done/error.
        announce(&ctx, &workspace.id, &review_id, ReviewStatus::Running);
        let ctx_bg = ctx.clone();
        let repo_path = repo.path.clone();
        let repo_id_bg = repo.id.clone();
        // Name the working-tree's current branch (source) and the diff base
        // (dest) for the reviewers. cwd stays the working tree — that IS the code
        // under review for a local review (uncommitted changes vs base).
        let local_branches = git
            .current_branch()
            .await
            .ok()
            .filter(|c| !c.is_empty())
            .map(|source| ReviewBranches {
                source,
                dest: body.base.clone(),
                checked_out: true,
            });
        tokio::spawn(async move {
            // Local/working-tree review: no PR, so pr_number = 0 (the fingerprint
            // accommodates pr 0).
            run_review(
                ctx_bg,
                review_id,
                repo_path,
                diff_text,
                None,
                None,
                workspace,
                repo_id_bg,
                0,
                local_branches,
                None,
                None,
            )
            .await;
        });
    }

    Ok(Json(review))
}

pub(crate) async fn get_local_review(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let review = ctx
        .reviews_store
        .latest_for_pr(&repo_id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .map_err(crate::error::ApiError)?
        .ok_or_else(|| {
            crate::error::ApiError(Error::NotFound("no local review for this repo".into()))
        })?;

    Ok(Json(review))
}

/// GET /repos/{id}/prs/{number}/reviews — all runs for a PR, newest first.
pub(crate) async fn list_reviews(
    Path(RepoPrPath {
        id: repo_id,
        number,
    }): Path<RepoPrPath>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Vec<Review>>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let reviews = ctx
        .reviews_store
        .list_for_pr(&repo_id, number as i64)
        .await
        .map_err(crate::error::ApiError)?;

    Ok(Json(reviews))
}

/// GET /repos/{id}/local-reviews — all local review runs, newest first.
pub(crate) async fn list_local_reviews(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Vec<Review>>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let reviews = ctx
        .reviews_store
        .list_for_pr(&repo_id, LOCAL_REVIEW_PR_NUMBER as i64)
        .await
        .map_err(crate::error::ApiError)?;

    Ok(Json(reviews))
}

// ---------------------------------------------------------------------------
// Handoff route
// ---------------------------------------------------------------------------

pub(crate) async fn handoff_review(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(body): Json<HandoffReq>,
) -> crate::error::ApiResult<Json<Session>> {
    // Load review and its repo.
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    let workspace = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    // Filter comments based on the optional comment_ids list.
    let comments_to_send: Vec<&otto_core::domain::ReviewComment> =
        if let Some(ref ids) = body.comment_ids {
            let id_set: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
            review
                .comments
                .iter()
                .filter(|c| id_set.contains(c.id.as_str()))
                .collect()
        } else {
            // All non-declined comments.
            review
                .comments
                .iter()
                .filter(|c| c.state != otto_core::domain::CommentState::Declined)
                .collect()
        };

    if comments_to_send.is_empty() {
        return Err(crate::error::ApiError(Error::Invalid(
            "no findings to hand off (all declined or list is empty)".into(),
        )));
    }

    // Build the handoff prompt.
    let mut prompt = String::from(
        "A code review of the changes in this repository found the following issues. \
         Please review each and fix the ones that are valid, then summarize what you changed:\n\n",
    );
    for c in &comments_to_send {
        let loc = match (&c.path, c.line) {
            (Some(p), Some(l)) => format!("{}:{}", p, l),
            (Some(p), None) => p.clone(),
            _ => "(general)".to_string(),
        };
        prompt.push_str(&format!("- {} [{}] {}\n", loc, c.severity.as_str(), c.body));
    }

    // Spawn the agent session.
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(body.provider.clone()),
        title: Some("Fix review findings".to_string()),
        cwd: Some(repo.path.clone()),
        connection_id: None,
        model: None,
        meta: None,
    };
    let session = ctx
        .manager
        .create(&workspace, &user.id, req, None)
        .await
        .map_err(crate::error::ApiError)?;

    // Write the prompt into the session after a short delay (mirrors PtySpawner).
    delay_session_input(
        &ctx,
        &user,
        &session.id,
        prompt,
        Duration::from_millis(1500),
        token,
    );

    Ok(Json(session))
}

// ---------------------------------------------------------------------------
// PR Review config routes
// ---------------------------------------------------------------------------

pub(crate) async fn get_review_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> crate::error::ApiResult<Json<ReviewConfig>> {
    Ok(Json(load_review_config(&ctx).await))
}

pub(crate) async fn put_review_config(
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ReviewConfig>,
) -> crate::error::ApiResult<Json<ReviewConfig>> {
    crate::auth::require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let value = serde_json::to_value(&body).map_err(|e| {
        crate::error::ApiError(otto_core::Error::Internal(format!("serialize: {e}")))
    })?;
    repo.put("pr_review", &value)
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(body))
}

pub(crate) async fn get_review_presets(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> crate::error::ApiResult<Json<Vec<ReviewConfigPreset>>> {
    Ok(Json(load_review_presets(&ctx).await))
}

pub(crate) async fn put_review_presets(
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(body): Json<Vec<ReviewConfigPreset>>,
) -> crate::error::ApiResult<Json<Vec<ReviewConfigPreset>>> {
    crate::auth::require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    // Reject blank/duplicate ids up front — a dangling or ambiguous id would
    // silently fall repos back to the global config.
    let mut seen = std::collections::HashSet::new();
    for p in &body {
        if p.id.trim().is_empty() {
            return Err(crate::error::ApiError(Error::Invalid(
                "preset id must not be empty".to_string(),
            )));
        }
        if !seen.insert(p.id.as_str()) {
            return Err(crate::error::ApiError(Error::Invalid(format!(
                "duplicate preset id: {}",
                p.id
            ))));
        }
    }
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let value = serde_json::to_value(&body).map_err(|e| {
        crate::error::ApiError(otto_core::Error::Internal(format!("serialize: {e}")))
    })?;
    repo.put("pr_review_presets", &value)
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(body))
}

/// Resolve a repo id to its workspace and check the caller's role there.
pub(crate) async fn require_repo_role(
    ctx: &ServerCtx,
    user: &User,
    repo_id: &Id,
    role: WorkspaceRole,
) -> crate::error::ApiResult<()> {
    let repo = ctx
        .git_store
        .get_repo(repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(ctx, user, &repo.workspace_id, role).await?;
    Ok(())
}

/// Build the `GET /repos/{id}/review-config` response: the stored binding
/// (if any) resolved to its scope + effective config.
pub(crate) async fn repo_review_config_resp(ctx: &ServerCtx, repo_id: &Id) -> RepoReviewConfigResp {
    if let Some(binding) = load_repo_review_binding(ctx, repo_id).await {
        if let Some(cfg) = binding.config {
            return RepoReviewConfigResp {
                scope: "custom".to_string(),
                preset_id: None,
                preset_name: None,
                config: cfg,
            };
        }
        if let Some(pid) = binding.preset_id {
            let preset = load_review_presets(ctx)
                .await
                .into_iter()
                .find(|p| p.id == pid);
            return match preset {
                Some(p) => RepoReviewConfigResp {
                    scope: "preset".to_string(),
                    preset_id: Some(p.id),
                    preset_name: Some(p.name),
                    config: p.config,
                },
                // Dangling reference: report it (preset_id set, name None) but
                // surface the config reviews will ACTUALLY run with — global.
                None => RepoReviewConfigResp {
                    scope: "preset".to_string(),
                    preset_id: Some(pid),
                    preset_name: None,
                    config: load_review_config(ctx).await,
                },
            };
        }
    }
    RepoReviewConfigResp {
        scope: "global".to_string(),
        preset_id: None,
        preset_name: None,
        config: load_review_config(ctx).await,
    }
}

pub(crate) async fn get_repo_review_config(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<RepoReviewConfigResp>> {
    require_repo_role(&ctx, &user, &repo_id, WorkspaceRole::Viewer).await?;
    Ok(Json(repo_review_config_resp(&ctx, &repo_id).await))
}

pub(crate) async fn put_repo_review_config(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<RepoReviewBinding>,
) -> crate::error::ApiResult<Json<RepoReviewConfigResp>> {
    require_repo_role(&ctx, &user, &repo_id, WorkspaceRole::Editor).await?;
    if body.config.is_none() && body.preset_id.is_none() {
        return Err(crate::error::ApiError(Error::Invalid(
            "binding needs a preset_id or an inline config (DELETE to revert to global)"
                .to_string(),
        )));
    }
    if let Some(pid) = &body.preset_id {
        if body.config.is_none() && !load_review_presets(&ctx).await.iter().any(|p| &p.id == pid) {
            return Err(crate::error::ApiError(Error::Invalid(format!(
                "unknown preset id: {pid}"
            ))));
        }
    }
    let settings = otto_state::SettingsRepo::new(ctx.pool.clone());
    let value = serde_json::to_value(&body).map_err(|e| {
        crate::error::ApiError(otto_core::Error::Internal(format!("serialize: {e}")))
    })?;
    settings
        .put(&repo_review_binding_key(&repo_id), &value)
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(repo_review_config_resp(&ctx, &repo_id).await))
}

pub(crate) async fn delete_repo_review_config(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<RepoReviewConfigResp>> {
    require_repo_role(&ctx, &user, &repo_id, WorkspaceRole::Editor).await?;
    let settings = otto_state::SettingsRepo::new(ctx.pool.clone());
    settings
        .delete(&repo_review_binding_key(&repo_id))
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(repo_review_config_resp(&ctx, &repo_id).await))
}

/// Routes for PR review agent configuration (global config, named presets,
/// and the per-repo binding).
pub fn review_config_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/settings/pr-review",
            get(get_review_config).put(put_review_config),
        )
        .route(
            "/settings/pr-review/presets",
            get(get_review_presets).put(put_review_presets),
        )
        .route(
            "/repos/{id}/review-config",
            get(get_repo_review_config)
                .put(put_repo_review_config)
                .delete(delete_repo_review_config),
        )
}

// ---------------------------------------------------------------------------
// Provider update route
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tests: plain sync fs / process / secret store is fine
mod readiness_tests {
    use super::local_branch_facts;
    use std::path::Path;
    use std::process::Command;

    /// Run git in `dir`, panicking with git's own stderr on failure.
    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn readiness_fills_unpushed_and_freshness() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&origin).unwrap();
        std::fs::create_dir_all(&work).unwrap();

        // Bare origin with `main` and `feature`, then a clone that moves ahead.
        git(
            &origin,
            &["init", "-q", "--bare", "--initial-branch=main", "."],
        );
        git(
            tmp.path(),
            &["clone", "-q", origin.to_str().unwrap(), "work"],
        );
        git(&work, &["config", "user.name", "t"]);
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["config", "commit.gpgsign", "false"]);
        std::fs::write(work.join("a.txt"), "one\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "init"]);
        git(&work, &["push", "-q", "-u", "origin", "main"]);

        git(&work, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(work.join("b.txt"), "two\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "feature 1"]);
        git(&work, &["push", "-q", "-u", "origin", "feature"]);

        // Two local commits that origin/feature has not seen …
        for (name, body) in [("c.txt", "three\n"), ("d.txt", "four\n")] {
            std::fs::write(work.join(name), body).unwrap();
            git(&work, &["add", "-A"]);
            git(&work, &["commit", "-q", "-m", name]);
        }
        // … and a main that moved on, so origin/main is NOT an ancestor.
        git(&work, &["checkout", "-q", "main"]);
        std::fs::write(work.join("e.txt"), "five\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "main moves"]);
        git(&work, &["push", "-q", "origin", "main"]);
        git(&work, &["checkout", "-q", "feature"]);

        let path = work.to_str().unwrap();
        assert_eq!(
            local_branch_facts(path, "feature", "main").await,
            (Some(2), "behind")
        );

        // Merging origin/main in makes it an ancestor → fresh. Unpushed grows to
        // four: the two feature commits, main's commit, and the merge commit —
        // none of them reachable from origin/feature.
        git(&work, &["merge", "-q", "--no-edit", "origin/main"]);
        assert_eq!(
            local_branch_facts(path, "feature", "main").await,
            (Some(4), "fresh")
        );

        // A source branch that does not exist locally is unknown on both axes.
        assert_eq!(
            local_branch_facts(path, "no-such-branch", "main").await,
            (None, "unknown")
        );
    }
}
