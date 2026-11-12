//! Scope assessment reuses the managed review-session lifecycle and cancellation.
use super::*;
use otto_core::domain::ReviewScopePlan;

const SCOPE_ABSOLUTE_CAP: Duration = Duration::from_secs(60 * 60);

#[allow(clippy::too_many_arguments)]
pub(super) async fn assess_scope(
    ctx: &ServerCtx,
    review_id: &Id,
    repo_path: &str,
    diff: &str,
    diff_path: &str,
    branch_context: &str,
    guidance: &str,
    workspace: &Workspace,
    cfg: &ReviewConfig,
    prior_ids: &[String],
) -> Result<ReviewScopePlan> {
    let head = otto_git::LocalGit::new(repo_path).rev_parse("HEAD").await?;
    if let Some(plan) = otto_review::scope::documentation_plan(diff, &head).filter(|_| {
        prior_ids.is_empty() && !cfg.agents.iter().any(|a| a.name == CHECKS_REVIEWER_NAME)
    }) {
        ctx.reviews_store.set_scope_plan(review_id, &plan).await?;
        return Ok(plan);
    }
    let user = otto_state::UsersRepo::new(ctx.pool.clone())
        .list()
        .await?
        .into_iter()
        .find(|u| u.is_root)
        .ok_or_else(|| Error::Internal("no root user to assess review scope".into()))?;
    let cfg_agent = &cfg.summarizer;
    let row = otto_review::scope::scope_row(&cfg_agent.provider, &cfg_agent.model);
    ctx.reviews_store
        .set_agents(review_id, std::slice::from_ref(&row))
        .await?;
    announce(ctx, &workspace.id, review_id, ReviewStatus::Running);
    let states = Arc::new(tokio::sync::Mutex::new(vec![row]));
    let guard = register_review_agent_cancel(&ctx.review_agent_cancels, review_id, 0);
    if review_is_cancelled(ctx, review_id).await {
        return Err(Error::Invalid("review cancelled".into()));
    }
    let prompt = otto_review::scope::scope_prompt(diff_path, branch_context, guidance, cfg);
    let slots = reviewer_slots();
    let _slot = otto_review::scope::acquire_slot(slots, guard.flag())
        .await
        .map_err(Error::Invalid)?;
    let started = std::time::Instant::now();
    // Reviewer rule: the grace fails only a QUIET assessor; the hour is a
    // backstop for the whole step, the second attempt included.
    let grace = otto_review::engine::review_agent_timeout(diff.len(), cfg.timeout_secs);
    let result = tokio::time::timeout(
        SCOPE_ABSOLUTE_CAP,
        otto_review::session::run_agent_session_with_recovery(
            &ctx.manager,
            &ctx.reviews_store,
            &states,
            workspace,
            &user,
            &cfg_agent.provider,
            &cfg_agent.model,
            repo_path,
            review_id,
            0,
            &prompt,
            grace,
            Some(otto_review::scope::SCOPE_ATTEMPTS),
            Some(guard.flag()),
            None,
            &[],
        ),
    )
    .await;
    let ids = states
        .lock()
        .await
        .iter()
        .filter_map(|a| a.session_id.clone())
        .collect::<Vec<_>>();
    otto_review::session::stop_review_sessions(&ctx.manager, &ids).await;
    if review_is_cancelled(ctx, review_id).await {
        return Err(Error::Invalid("review cancelled".into()));
    }
    let result =
        result.map_err(|_| Error::Invalid("scope assessment ran past its 1-hour cap".into()))?;
    // A failed session's row note says why (paste refused, stuck, timed out).
    let note = states.lock().await.first().map(|s| s.note.clone());
    let mut plan = otto_review::scope::plan_from_run(&result, &note.unwrap_or_default(), prior_ids)
        .map_err(Error::Invalid)?;
    if cfg.agents.iter().any(|a| a.name == CHECKS_REVIEWER_NAME)
        && !plan.lenses.iter().any(|l| l.name == CHECKS_REVIEWER_NAME)
    {
        return Err(Error::Invalid(
            "Scope assessment omitted configured required checks".into(),
        ));
    }
    plan.head_sha = head;
    plan.source = format!(
        "agent assessment of diff and repository · {}s",
        started.elapsed().as_secs()
    );
    ctx.reviews_store.set_scope_plan(review_id, &plan).await?;
    announce(ctx, &workspace.id, review_id, ReviewStatus::Running);
    Ok(plan)
}
