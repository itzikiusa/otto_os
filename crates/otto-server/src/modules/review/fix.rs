//! Human-launched local remediation (goal.md §4.1): a goal that fixes the
//! review's findings in a run worktree cut at the reviewed commit.
use super::*;

#[derive(serde::Deserialize)]
pub(super) struct FixReviewReq {
    #[serde(default = "default_target")]
    target_score: f64,
}
fn default_target() -> f64 {
    9.5
}

pub(super) async fn fix_review(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    auth: crate::auth::CurrentAuthContext,
    Json(req): Json<FixReviewReq>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::auth::require_human(&auth.0)?;
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
    if review.status != ReviewStatus::Done || review.pr_number == LOCAL_REVIEW_PR_NUMBER {
        return Err(ApiError(Error::Invalid("A completed PR review is required; local uncommitted reviews cannot seed a commit-based fix worktree".into())));
    }
    let run_context = ctx
        .reviews_store
        .get_run_context(&review_id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| {
            ApiError(Error::Invalid(
                "Review has no recorded checkout; re-run it first".into(),
            ))
        })?;
    let head = run_context
        .head_sha
        .as_deref()
        .ok_or_else(|| ApiError(Error::Invalid("Review has no recorded commit".into())))?;
    if head.len() != 40 || !head.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError(Error::Invalid(
            "Review commit must be an immutable full SHA".into(),
        )));
    }
    if !review
        .scope_plan
        .as_ref()
        .is_some_and(|p| p.head_sha == head)
    {
        return Err(ApiError(Error::Invalid(
            "Review scope and checkout differ; re-run the review".into(),
        )));
    }
    let provider = default_review_provider(&ctx).await;
    let invalid = |e: String| ApiError(Error::Invalid(e));
    let built = otto_workflows::goal::fix_review_workflow(
        &review,
        req.target_score,
        &provider,
        &repo.path,
        head,
    )
    .map_err(invalid)?;
    // Serialize launch admission, not agent execution. Repeated clicks return
    // the same goal's latest run.
    static LAUNCH: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _guard = LAUNCH
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let link = format!("review:{review_id}");
    let runs = otto_state::WorkflowsRepo::new(ctx.pool.clone());
    let ws = &repo.workspace_id;
    if let Some(wf) = runs
        .find_goal_by_origin_ref(ws, &link)
        .await
        .map_err(ApiError)?
    {
        if let Some(run) = runs.latest_run_of(&wf.id).await.map_err(ApiError)? {
            return Ok(Json(
                serde_json::json!({"workflow_id": wf.id, "run_id": run.id}),
            ));
        }
    }
    // The commit must exist before the goal is stored; provisioning cuts the
    // run worktree at this exact SHA and fails closed instead of using HEAD.
    otto_git::LocalGit::new(&repo.path)
        .rev_parse(head)
        .await
        .map_err(ApiError)?;
    let (wf, run) = crate::goals::start_built(&ctx, &user, ws, built, Some(&link))
        .await
        .map_err(ApiError)?;
    Ok(Json(
        serde_json::json!({"workflow_id": wf.id, "run_id": run.id}),
    ))
}
