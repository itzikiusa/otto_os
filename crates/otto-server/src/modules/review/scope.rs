//! Scope assessment reuses the managed review-session lifecycle and cancellation.
use super::*;
use otto_core::domain::ReviewScopePlan;

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
    let _slot = slots
        .acquire_owned()
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(120),
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
            Duration::from_secs(120),
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
    let result = result.map_err(|_| Error::Invalid("Scope assessment exceeded its 120-second budget; no reviewers launched. Re-run review.".into()))?;
    if result.errored || result.findings.len() != 1 {
        return Err(Error::Invalid(
            "Scope assessment failed; no reviewers launched. Re-run review to retry.".into(),
        ));
    }
    let body = result.findings[0]
        .body
        .trim()
        .trim_start_matches("```json")
        .trim_end_matches("```")
        .trim();
    let mut plan: ReviewScopePlan = serde_json::from_str(body)
        .map_err(|e| Error::Invalid(format!("Invalid scope assessment: {e}")))?;
    otto_review::scope::validate(&plan).map_err(Error::Invalid)?;
    if prior_ids
        .iter()
        .any(|id| !plan.prior_findings.iter().any(|p| p.finding_id == *id))
    {
        return Err(Error::Invalid(
            "Scope assessment omitted a prior finding disposition; re-run review".into(),
        ));
    }
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

pub(super) async fn prior_review_context(
    ctx: &ServerCtx,
    repo_id: &Id,
    pr: u64,
    current: &Id,
    branches: Option<&ReviewBranches>,
    explicit: Option<&Id>,
) -> Result<(String, Vec<String>)> {
    let previous = if let Some(id) = explicit {
        let prior = ctx.reviews_store.get_review(id).await?;
        if prior.repo_id != *repo_id || prior.pr_number != pr || prior.id == *current {
            return Err(Error::Invalid(
                "Previous review must belong to the same repository and PR".into(),
            ));
        }
        vec![prior]
    } else {
        ctx.reviews_store.list_for_pr(repo_id, pr as i64).await?
    };
    for review in previous {
        if review.id == *current || review.status != ReviewStatus::Done {
            continue;
        }
        if pr == LOCAL_REVIEW_PR_NUMBER && explicit.is_none() {
            let recorded = ctx.reviews_store.get_run_context(&review.id).await?;
            let branch = branches
                .map(|b| b.source.as_str())
                .filter(|s| !s.is_empty());
            if branch.is_none()
                || recorded.as_ref().and_then(|c| c.source_branch.as_deref()) != branch
            {
                continue;
            }
        }
        let findings = ctx.findings_store.list_full_for_review(&review.id).await?;
        let ids = findings.iter().map(|f| f.id.clone()).collect();
        return Ok((format!("\n## Prior review {} — verify before fresh review\nFor EACH prior finding, cite its ID and record fixed + concrete current-code/check evidence, still_open + trigger/evidence, or not_relevant + causal-scope reason. Missing from fresh output is NOT proof of a fix. Do not inherit earlier claims without rechecking. Then perform an additional fresh review of the current causal diff.\n{}\n",
            review.id, serde_json::to_string(&findings).map_err(|e| Error::Internal(e.to_string()))?), ids));
    }
    Ok((String::new(), vec![]))
}
