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
    let row = ReviewAgentState {
        name: "Assessing change scope".into(),
        provider: cfg_agent.provider.clone(),
        model: cfg_agent.model.clone(),
        status: "pending".into(),
        note: "Inspecting changed behavior, affected consumers and risks before selecting lenses"
            .into(),
        comment_count: 0,
        session_id: None,
        findings: vec![],
        fallback: false,
        lens: String::new(),
        lens_slugs: vec![],
        score: None,
        score_reason: String::new(),
    };
    ctx.reviews_store
        .set_agents(review_id, std::slice::from_ref(&row))
        .await?;
    announce(ctx, &workspace.id, review_id, ReviewStatus::Running);
    let states = Arc::new(tokio::sync::Mutex::new(vec![row]));
    let guard = register_review_agent_cancel(&ctx.review_agent_cancels, review_id, 0);
    if review_is_cancelled(ctx, review_id).await {
        return Err(Error::Invalid("review cancelled".into()));
    }
    let candidates = cfg
        .agents
        .iter()
        .map(|a| format!("{}: {}", a.name, a.prompt))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!("{}\nConfigured lenses (reuse exact names when relevant; omit irrelevant lenses and add missing ones):\n{}\nIf configured, the Required checks lens is mandatory and its commands must be preserved. For every prior finding in context, add prior_findings:[{{finding_id,disposition:fixed|still_open|not_relevant,evidence}}] to the scope object. Read current code before claiming fixed; evidence must cite concrete code or an actually executed check. Preserve every ID, including previously waived items. Transport requirement: return a JSON ARRAY with exactly one object: {{\"severity\":\"info\",\"body\":\"the JSON scope object above, encoded as a string\"}}. This is a scope plan, not a code finding.",
        otto_review::scope::assessment_prompt(diff_path, branch_context, guidance), candidates);
    let slots = reviewer_slots();
    let _slot = otto_review::scope::acquire_slot(slots, guard.flag())
        .await
        .map_err(Error::Invalid)?;
    let started = std::time::Instant::now();
    // Reviewer rule: the grace fails only a QUIET assessor; the hour is a backstop.
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
            Some(1),
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
    if result.errored || result.findings.len() != 1 {
        return Err(Error::Invalid("scope assessment returned no plan".into()));
    }
    let body = result.findings[0]
        .body
        .trim()
        .trim_start_matches("```json")
        .trim_end_matches("```")
        .trim();
    let mut plan: ReviewScopePlan = serde_json::from_str(body)
        .map_err(|e| Error::Invalid(format!("Invalid scope assessment: {e}")))?;
    otto_review::scope::validate_prior_findings(&plan, prior_ids).map_err(Error::Invalid)?;
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
