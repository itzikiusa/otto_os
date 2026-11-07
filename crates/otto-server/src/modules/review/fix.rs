//! Human-launched local remediation, using the existing Goal Loop controller.
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
) -> ApiResult<Json<otto_core::domain::GoalLoop>> {
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
    let (definition, config) = otto_review::fix_goal::build(&review, req.target_score, &provider)
        .map_err(|e| ApiError(Error::Invalid(e)))?;
    let budget = crate::budget::check_budget(&ctx, &repo.workspace_id, "").await;
    if budget.blocked {
        return Err(ApiError(Error::Invalid(
            budget
                .reason
                .unwrap_or_else(|| "Agent budget exceeded".into()),
        )));
    }
    // Serialize launch admission, not agent execution. Repeated clicks return
    // the same durable loop, including an incomplete loop the user can Resume.
    static LAUNCH: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _guard = LAUNCH
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let link = format!("review:{review_id}");
    if let Some(existing) = ctx
        .goal_loops_repo
        .list_by_workspace(&repo.workspace_id)
        .await
        .map_err(ApiError)?
        .into_iter()
        .find(|g| g.config.source_links.contains(&link))
    {
        if existing.status == otto_core::domain::GoalLoopStatus::Draft {
            ctx.goal_loops_repo
                .set_review_base(&existing.id, head)
                .await
                .map_err(ApiError)?;
            crate::goal_loop::start_loop(&ctx, &existing.id)
                .await
                .map_err(ApiError)?;
            return Ok(Json(
                ctx.goal_loops_repo
                    .get(&existing.id)
                    .await
                    .map_err(ApiError)?,
            ));
        }
        return Ok(Json(existing));
    }
    // Verify the object exists before persisting the goal; provisioning resolves
    // this exact SHA again and fails closed instead of falling back to HEAD.
    otto_git::LocalGit::new(&repo.path)
        .rev_parse(head)
        .await
        .map_err(ApiError)?;
    let goal = ctx
        .goal_loops_repo
        .create(otto_state::NewGoalLoop {
            workspace_id: repo.workspace_id.clone(),
            name: definition.title.clone(),
            repo_path: repo.path,
            definition,
            config,
            limits: otto_core::domain::GoalLoopLimits {
                max_iterations: 6,
                max_runtime_secs: 7200,
                per_phase_timeout_secs: 1200,
                ..Default::default()
            },
            created_by: user.id,
        })
        .await
        .map_err(ApiError)?;
    ctx.goal_loops_repo
        .set_review_base(&goal.id, head)
        .await
        .map_err(ApiError)?;
    crate::goal_loop::start_loop(&ctx, &goal.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(
        ctx.goal_loops_repo.get(&goal.id).await.map_err(ApiError)?,
    ))
}
