//! The review run engine: config loading, `run_review_core`, worktree
//! materialisation, proof assembly and the findings read-models.

use super::*;

/// Load ReviewConfig from settings or fall back to the default. The default
/// config's reviewer agents follow the global default agent
/// (`default_provider` setting, else "claude"); a stored config is used as-is.
pub(crate) async fn load_review_config(ctx: &ServerCtx) -> ReviewConfig {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let global_default = repo.get("default_provider").await.ok().flatten();
    let default_provider =
        otto_core::provider::resolve_provider(&[otto_core::provider::global_default(
            global_default.as_ref(),
        )]);
    match repo.get("pr_review").await {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_else(|e| {
            tracing::warn!("failed to deserialize pr_review config: {e}; using default");
            default_review_config(&default_provider)
        }),
        _ => default_review_config(&default_provider),
    }
}

/// Load the named review-config presets (settings key `pr_review_presets`).
/// A missing/corrupt value is an empty list, never an error.
pub(crate) async fn load_review_presets(ctx: &ServerCtx) -> Vec<ReviewConfigPreset> {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    match repo.get("pr_review_presets").await {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_else(|e| {
            tracing::warn!("failed to deserialize pr_review_presets: {e}; using empty");
            Vec::new()
        }),
        _ => Vec::new(),
    }
}

/// Load the stored per-repo binding, if any. Corrupt values read as `None` so a
/// bad row can never brick reviews for the repo.
pub(crate) async fn load_repo_review_binding(
    ctx: &ServerCtx,
    repo_id: &Id,
) -> Option<RepoReviewBinding> {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    match repo.get(&repo_review_binding_key(repo_id)).await {
        Ok(Some(v)) => serde_json::from_value(v)
            .map_err(|e| {
                tracing::warn!(repo = %repo_id, "failed to deserialize repo review binding: {e}");
            })
            .ok(),
        _ => None,
    }
}

/// Resolve the EFFECTIVE review config for a repo: inline per-repo config >
/// per-repo preset reference > global `pr_review` config. A dangling preset
/// reference (preset deleted) falls back to global with a warn — reviews must
/// never fail because a preset went away.
pub(crate) async fn load_review_config_for_repo(ctx: &ServerCtx, repo_id: &Id) -> ReviewConfig {
    if let Some(binding) = load_repo_review_binding(ctx, repo_id).await {
        if let Some(cfg) = binding.config {
            return cfg;
        }
        if let Some(pid) = binding.preset_id {
            if let Some(p) = load_review_presets(ctx)
                .await
                .into_iter()
                .find(|p| p.id == pid)
            {
                return p.config;
            }
            tracing::warn!(repo = %repo_id, preset = %pid, "repo review preset missing; using global config");
        }
    }
    load_review_config(ctx).await
}

/// The review mode a `review_run` step runs with when neither the run
/// input nor the node set one: the repo's effective stored config (or the
/// global one when `repo_id` is `None`), tagged with its source for the
/// step log.
pub(crate) async fn effective_review_mode(
    ctx: &ServerCtx,
    repo_id: Option<&Id>,
) -> (otto_core::domain::ReviewMode, &'static str) {
    let cfg = match repo_id {
        Some(id) => load_review_config_for_repo(ctx, id).await,
        None => load_review_config(ctx).await,
    };
    mode_source(&cfg)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_review(
    ctx: ServerCtx,
    review_id: Id,
    repo_path: String,
    diff_text: String,
    jira_context: Option<String>,
    user_context: Option<String>,
    workspace: Workspace,
    repo_id: Id,
    pr_number: u64,
    branches: Option<ReviewBranches>,
    cfg_override: Option<ReviewConfig>,
    // Per-run/per-node execution mode (`review_run`). `None` ⇒ the stored
    // config decides, then `FanOut`.
    mode_override: Option<otto_core::domain::ReviewMode>,
) {
    // Join the launch-registered flag (or register one). The guard unregisters
    // it on every exit path (incl. the early return) — but only while the
    // registry still holds THAT flag, so an unwinding (cancelled) attempt never
    // removes a newer retry's entry. Without it every branch/local review
    // leaked an entry, and a tripped flag left behind could cancel a later
    // rerun of the same id.
    let attempt = ctx.review_cancels.join(&review_id);
    // Cancellation can commit between launch admission and this task being
    // polled. Register first so later cancellations also reach this attempt.
    if review_is_cancelled(&ctx, &review_id).await {
        return;
    }
    let result = run_review_core(
        &ctx,
        &review_id,
        &repo_path,
        diff_text,
        jira_context,
        user_context,
        &workspace,
        &repo_id,
        pr_number,
        branches.as_ref(),
        cfg_override,
        mode_override,
    )
    .await;
    if attempt.is_cancelled() || review_is_cancelled(&ctx, &review_id).await {
        return;
    }
    // The run is over (done or error): its temp artifacts are spent.
    remove_review_temp_files(&review_id).await;
    match result {
        Ok(()) => {
            tracing::info!(review = %review_id, "review complete");
            let _ = transition(&ctx, &workspace.id, &review_id, ReviewStatus::Done, None).await;
        }
        Err(e) => {
            tracing::warn!(review = %review_id, "review error: {e}");
            // The core only reaches its own teardown on the summarize path; an
            // error anywhere else would otherwise leave the reviewers' PTYs live.
            if let Ok(review) = ctx.reviews_store.get_review(&review_id).await {
                let ids: Vec<String> = review
                    .agents
                    .iter()
                    .filter_map(|a| a.session_id.clone())
                    .collect();
                otto_review::session::stop_review_sessions(&ctx.manager, &ids).await;
            }
            let msg = e.to_string();
            let _ = transition(
                &ctx,
                &workspace.id,
                &review_id,
                ReviewStatus::Error,
                Some(&msg),
            )
            .await;
        }
    }
}

/// The review's error note when its diff file can't be written for the
/// reviewers (a new run settles with it; a retry refuses with it).
pub(crate) fn diff_write_failure(cause: &dyn std::fmt::Display) -> String {
    format!("Couldn't write the diff for the reviewers: {cause}")
}

/// Write the diff to a file the agents read themselves. Pasting a large PR
/// diff into the prompt doesn't scale (and can blow past input limits); the
/// agents are real sessions with file access, so they read it on demand.
/// Fatal when it fails (ENOSPC, unwritable TMPDIR): every reviewer would
/// otherwise burn a turn on a path that does not exist and the run would
/// settle as a normal — possibly "clean" — review. The caller returns the
/// error before any agent is spawned, so the review settles as `error` with
/// this cause. Also stores the durable copy (0100): the prompt references the
/// temp path, so a retry after a reboot/temp-sweep re-materializes the file.
async fn materialize_review_diff(
    ctx: &ServerCtx,
    review_id: &Id,
    diff_text: &str,
) -> Result<std::path::PathBuf> {
    let diff_path = std::env::temp_dir().join(format!("otto-review-{review_id}.diff"));
    if let Err(e) = tokio::fs::write(&diff_path, diff_text).await {
        tracing::warn!(review = %review_id, "could not write review diff file: {e}");
        return Err(Error::Internal(diff_write_failure(&e)));
    }
    if let Err(e) = ctx.reviews_store.set_diff(review_id, diff_text).await {
        tracing::warn!(review = %review_id, "could not persist review diff: {e}");
    }
    Ok(diff_path)
}

/// Remove a review's `$TMPDIR/otto-review-<id>*` files (diff, per-agent
/// prompts, findings JSON). A finished run's durable copies live in the DB
/// (0100) and a retry re-materializes what it needs, so leaving them behind
/// only grew the temp dir (100+ files per few days of reviews).
pub(crate) async fn remove_review_temp_files(review_id: &str) {
    let prefix = format!("otto-review-{review_id}");
    let () = crate::offload::blocking(move || {
        remove_review_temp_files_in(&std::env::temp_dir(), &prefix)
    })
    .await;
}

/// [`remove_review_temp_files`] over an explicit dir — FILES only, so the
/// `otto-review-wt-<id>` worktree directory is never touched.
#[allow(clippy::disallowed_methods)] // sync helper: runs on the blocking pool via offload::blocking (or a test)
pub(crate) fn remove_review_temp_files_in(dir: &std::path::Path, prefix: &str) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if entry.file_name().to_string_lossy().starts_with(prefix)
                && entry.file_type().is_ok_and(|t| t.is_file())
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Core fallible review logic: given a unified diff text, optional Jira
/// context, and optional free-text user guidance, runs the configured agents,
/// stores comments, and updates live agent-state rows. Used by both the PR and
/// local-review flows.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_review_core(
    ctx: &ServerCtx,
    review_id: &Id,
    repo_path: &str,
    mut diff_text: String,
    jira_context: Option<String>,
    user_context: Option<String>,
    workspace: &Workspace,
    repo_id: &Id,
    pr_number: u64,
    branches: Option<&ReviewBranches>,
    cfg_override: Option<ReviewConfig>,
    mode_override: Option<otto_core::domain::ReviewMode>,
) -> Result<()> {
    let jira_ctx = jira_context.unwrap_or_default();
    // Build the optional free-text guidance block (prepended to every agent
    // prompt before the Jira block). Empty/absent => no block, identical to old
    // behaviour.
    let mut user_ctx = match user_context {
        Some(c) if !c.trim().is_empty() => {
            format!("## Reviewer guidance\n{}\n\n---\n\n", c.trim())
        }
        _ => String::new(),
    };

    let branch_ctx = otto_review::prompts::branch_context(branches);

    // 1. Load config: a per-call override (e.g. a workflow `review_run` step that
    //    sets its own providers + lenses) wins; otherwise the repo's binding
    //    (inline > preset) and finally the stored/default global config.
    let mut cfg = match cfg_override {
        Some(c) => c,
        None => load_review_config_for_repo(ctx, repo_id).await,
    };
    if branches.is_some_and(|b| !b.checked_out) {
        return Err(Error::Invalid("The PR revision could not be checked out; review cannot verify claims against a different revision. Retry after fetching the branch.".into()));
    }
    let causal_base = if pr_number != LOCAL_REVIEW_PR_NUMBER {
        let destination = branches
            .map(|b| b.dest.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Error::Invalid(
                    "PR target branch is unavailable; causal scope cannot be established".into(),
                )
            })?;
        let git = otto_git::LocalGit::new(repo_path);
        let remote_target = format!("origin/{destination}");
        let target = if git.rev_parse(&remote_target).await.is_ok() {
            remote_target
        } else {
            destination.to_string()
        };
        let causal = git.causal_review_diff(&target).await?;
        user_ctx.push_str(&format!("\nCausal PR scope is immutable merge-base {} to reviewed HEAD {}. The complete diff is in the supplied file; read in chunks as needed. Target-tip commits and worktree dirt are excluded.\n", causal.base_sha, causal.head_sha));
        diff_text = causal.text;
        Some(causal.base_sha)
    } else {
        None
    };
    let (prior_context, prior_ids) = otto_review::history::prior_review_context(
        &ctx.reviews_store,
        &ctx.findings_store,
        repo_id,
        pr_number,
        review_id,
        branches,
        cfg.previous_review_id.as_ref(),
    )
    .await?;
    user_ctx.push_str(&prior_context);
    let diff_path = materialize_review_diff(ctx, review_id, &diff_text).await?;
    let lines = otto_review::scope::changed_lines(&diff_text);
    let size = otto_review::scope::pr_size(lines, &cfg);
    let assessed = if size == "large" {
        Ok(otto_review::scope::large_pr_plan(&cfg, &prior_ids, lines))
    } else {
        super::scope::assess_scope(
            ctx,
            review_id,
            repo_path,
            &diff_text,
            &diff_path.to_string_lossy(),
            &branch_ctx,
            &format!("{user_ctx}{jira_ctx}"),
            workspace,
            &cfg,
            &prior_ids,
        )
        .await
    };
    // Scope selection narrows the lenses; it must never stop the review.
    let mut scope_plan = match assessed {
        Ok(plan) => plan,
        Err(e) if review_is_cancelled(ctx, review_id).await => return Err(e),
        Err(e) => {
            tracing::warn!(review = %review_id, "scope assessment failed ({e}); running configured lenses");
            otto_review::scope::configured_plan(&cfg, &prior_ids, &e.to_string())
        }
    };
    scope_plan.base_sha = causal_base.unwrap_or_default();
    (scope_plan.size, scope_plan.changed_lines) = (size.into(), lines);
    ctx.reviews_store
        .set_scope_plan(review_id, &scope_plan)
        .await?;
    cfg.agents = otto_review::scope::selected_agents(&scope_plan, &cfg);
    user_ctx.push_str(&format!(
        "\n## Assessed scope and exact selected lenses\n{}\n",
        serde_json::to_string(&scope_plan).unwrap_or_default()
    ));
    // Execution mode: the caller's override (the run input, then the
    // `review_run` node) wins over the stored config, which wins over fan-out.
    let _requested_mode = mode_override.or(cfg.mode).unwrap_or_default();
    // Adaptive sessions always use managed slots, including cross-provider checks.
    // Provider-internal delegation cannot enforce the machine-wide concurrency bound.
    let mode = otto_core::domain::ReviewMode::FanOut;
    tracing::info!(review = %review_id, mode = mode.as_str(), "review mode");

    // 2. Expand agent×provider pairs (fan-out) or one orchestrator per provider.
    let agent_runs = expand_agent_runs(&cfg, mode, &ctx.context_library);
    let run_count = agent_runs.len();

    let agent_states = otto_review::prompts::seed_states(&agent_runs, &cfg.summarizer);
    ctx.reviews_store
        .set_agents(review_id, &agent_states)
        .await?;

    // 3. Resolve the root user (review agents run as autonomous sessions on its
    //    behalf, like channel sessions) and the per-agent grace period.
    let review_user = otto_state::UsersRepo::new(ctx.pool.clone())
        .list()
        .await
        .ok()
        .and_then(|us| us.into_iter().find(|u| u.is_root))
        .ok_or_else(|| Error::Internal("no root user to run review agents".into()))?;
    let timeout = review_agent_timeout(diff_text.len(), cfg.timeout_secs);
    // An orchestrator run carries every lens, so it needs more than one lens'
    // budget — bounded, and still only a budget (`watch_for_result`'s deadline
    // fires on an IDLE agent, never on a working one).
    let timeout = if mode == otto_core::domain::ReviewMode::Orchestrator {
        orchestrator_budget(timeout, cfg.agents.len())
    } else {
        timeout
    };

    // Pre-trust the repo folder for every provider we'll run (reviewers + the
    // claude summarizer) so no agent stalls on the interactive "trust this
    // folder?" prompt and silently times out with zero findings. Trust does NOT
    // hand the checkout's own config to the reviewer: every review session is
    // `read_only` (forced Seatbelt, no shell) with `project_settings: false`
    // (`--setting-sources user`, so a PR's `.claude/settings.json` hooks never
    // load) — see `otto_review::session::review_session_meta`.
    {
        let mut trusted = std::collections::HashSet::<String>::new();
        for provider in agent_runs
            .iter()
            .map(|r| r.provider.clone())
            .chain(std::iter::once("claude".to_string()))
        {
            if trusted.insert(provider.clone()) {
                otto_sessions::trust::ensure_trusted(&provider, repo_path);
            }
        }
    }

    // Where this run reads the code: the source branch scopes local finding
    // resolution (S2-05) and the checkout + head are what a Retry re-enters
    // instead of the user's main checkout (S2-06).
    let run_context = otto_state::ReviewRunContext {
        source_branch: branches.map(|b| b.source.clone()).filter(|b| !b.is_empty()),
        cwd: Some(repo_path.to_string()),
        head_sha: otto_git::LocalGit::new(repo_path)
            .rev_parse("HEAD")
            .await
            .ok()
            .filter(|s| !s.is_empty()),
    };
    if let Err(e) = ctx
        .reviews_store
        .set_run_context(review_id, &run_context)
        .await
    {
        tracing::warn!(review = %review_id, "could not persist review run context: {e}");
    }
    let diff_path_str = diff_path.to_string_lossy().to_string();

    // Diff SCALE, stated to every reviewer. Without it an agent has no idea
    // whether it was handed a 3-file tweak or a 170-file module, so it reviews
    // until it has "enough" findings and stops — satisficing that reads as a
    // shallow review on a large PR. Naming the size, and requiring the agent to
    // account for every file, converts "find some bugs" into a bounded sweep it
    // can be held to.
    let changed_files = diff_text
        .lines()
        .filter(|l| l.starts_with("+++ b/"))
        .count()
        .max(1);
    let changed_lines = diff_text
        .lines()
        .filter(|l| {
            (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"))
        })
        .count();

    // 4. Run each reviewer as a real, openable session. Each task persists its
    //    own live state (running → waiting → done/error) so the UI poll shows
    //    progress; one stuck/failed agent never aborts the others.
    let states: otto_review::session::SharedStates =
        Arc::new(tokio::sync::Mutex::new(agent_states));
    // Per-review cancel flag (set by POST /reviews/{id}/cancel). Threaded into
    // each agent's recovery loop so a Cancel short-circuits in-flight retries.
    let cancel_flag: Option<Arc<std::sync::atomic::AtomicBool>> =
        ctx.review_cancels.flag(review_id.as_str());
    // Make the FULL skill library available to CLAUDE review agents as first-class
    // skills (`Skill(<name>)`), staged into a shared out-of-tree bundle wired in
    // below via `meta.extra_dirs` → `--add-dir` (claude-only — see
    // `otto_review::session::review_skills_extra_dirs`). Skills are generic and
    // cross-context (e.g. `grill` is useful for review, not only product), and a
    // review config usually carries the lens in the agent NAME rather than the
    // `skill` field — so rather than guess which one each agent needs, we register
    // them all and let claude invoke whichever it wants. Works in ANY repo, not
    // only ones carrying an in-tree `.claude/skills`. codex/agy don't use this
    // bundle; their lens method travels inline in the prompt. Best-effort (None →
    // even claude falls back to the inlined prompt text).
    let review_skills_dir = stage_all_review_skills(&ctx.context_library).await;

    // Each reviewer watches its OWN cancel flag (the per-agent Stop button).
    // Whole-review cancel trips every per-agent flag too, so review-level
    // semantics are unchanged; pre-tripping here closes the race where cancel
    // lands between set_agents and this registration. EVERY flag is registered
    // before the first spawn (R3): the spawns are staggered below, and a Stop
    // during the stagger must still reach the reviewers not started yet.
    let agent_cancels: Vec<CancelGuard> = (0..run_count)
        .map(|i| {
            let guard = register_review_agent_cancel(&ctx.review_agent_cancels, review_id, i);
            if cancel_flag
                .as_ref()
                .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
            {
                guard
                    .flag()
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            guard
        })
        .collect();

    let mut set = tokio::task::JoinSet::new();
    for (i, run) in agent_runs.into_iter().enumerate() {
        // Space the cold starts out (R3) — no cap, no semaphore: the JoinSet
        // still runs every reviewer at once, they just don't all boot in the
        // same second.
        if i > 0 {
            tokio::time::sleep(REVIEW_SPAWN_STAGGER).await;
        }
        let manager = Arc::clone(&ctx.manager);
        let agent_cancel = Arc::clone(agent_cancels[i].flag());
        let reviews = ctx.reviews_store.clone();
        let states = Arc::clone(&states);
        let ws = workspace.clone();
        let user = review_user.clone();
        let cwd = repo_path.to_string();
        let review_id_s = review_id.to_string();
        let review_skills_dir = review_skills_dir.clone();
        // An orchestrator run composes its lens block HERE: it names one output
        // path per lens, and those are keyed on this run's index.
        let prompt_lens = if run.lenses.is_empty() {
            run.prompt_lens.clone()
        } else {
            compose_orchestrator_prompt(
                &run.lenses,
                &run.provider,
                |slug| otto_review::session::lens_findings_path(&review_id_s, i, slug),
                &otto_review::session::findings_path(&review_id_s, i),
            )
        };
        let lens_slugs = run.lens_slugs();
        let prompt = otto_review::prompts::reviewer_prompt(
            &branch_ctx,
            changed_files,
            changed_lines,
            &prompt_lens,
            &user_ctx,
            &jira_ctx,
            &diff_path_str,
        );
        // Persist the prompt so a per-agent Retry can re-run exactly this agent:
        // temp file for the in-flight injection path, DB row (0100) so retry
        // survives reboots / temp sweeps / daemon redeploys.
        let _ = std::fs::write(otto_review::session::prompt_path(review_id, i), &prompt);
        if let Err(e) = ctx
            .reviews_store
            .set_agent_prompt(review_id, i, &prompt)
            .await
        {
            tracing::warn!(review = %review_id, agent = i, "could not persist agent prompt: {e}");
        }
        let max_attempts = cfg.max_attempts;
        let slots = reviewer_slots();
        set.spawn(async move {
            // SI-11: wait for a reviewer slot (a closed semaphore never
            // happens — it lives for the process).
            let Ok(_slot) = otto_review::scope::acquire_slot(slots, &agent_cancel).await else {
                return (
                    i,
                    otto_review::session::AgentRunResult {
                        findings: vec![],
                        errored: true,
                    },
                );
            };
            let res = otto_review::session::run_agent_session_with_recovery(
                &manager,
                &reviews,
                &states,
                &ws,
                &user,
                &run.provider,
                &run.model,
                &cwd,
                &review_id_s,
                i,
                &prompt,
                timeout,
                max_attempts,
                Some(&agent_cancel),
                review_skills_dir.as_deref(),
                &lens_slugs,
            )
            .await;
            (i, res)
        });
    }

    // Collect each agent's findings for the summarizer (each task already
    // persisted its own live state, so a panicked task just yields no findings).
    let mut agent_findings: Vec<Vec<otto_core::domain::ReviewFinding>> =
        vec![Vec::new(); run_count];
    while let Some(joined) = set.join_next().await {
        if let Ok((i, res)) = joined {
            if !res.errored {
                agent_findings[i] = res.findings;
            }
        }
    }
    // Every reviewer has finished (stopped ones included) — drop their flags.
    drop(agent_cancels);

    // If the review was cancelled while the agents ran, stop here: skip the
    // (expensive) summarizer and do NOT persist findings. The cancel handler
    // owns the terminal `cancelled` status + teardown.
    if cancel_flag
        .as_ref()
        .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
    {
        tracing::info!(review = %review_id, "review cancelled — skipping summarizer/persist");
        return Ok(());
    }

    // 4–7. Summarize the per-agent findings and persist comments + workflow
    // findings. Shared with `retry_summarizer` (which re-runs ONLY this stage
    // from the stored per-agent findings, without re-running the reviewers).
    let result = summarize_and_persist(
        ctx,
        review_id,
        repo_path,
        workspace,
        &review_user,
        repo_id,
        pr_number,
        &cfg.summarizer,
        &agent_findings,
        &format!("{user_ctx}{jira_ctx}"),
        resolve_blocker(pr_number, branches, &diff_text),
    )
    .await;

    // 8. The review is over: the reviewers' sessions are autonomous, nobody
    //    types into them again, and each holds a PTY plus the CLI's whole fd
    //    footprint. Suspend/kill them (and drop the per-lens scratch files)
    //    whether the summarizer succeeded or not.
    let session_ids: Vec<String> = {
        let g = states.lock().await;
        g.iter().filter_map(|s| s.session_id.clone()).collect()
    };
    otto_review::session::stop_review_sessions(&ctx.manager, &session_ids).await;
    let rid = review_id.to_string();
    for i in 0..run_count {
        otto_review::session::remove_lens_findings_files(&rid, i);
    }
    result
}

/// Flags stop work immediately; the durable status covers daemon/retry paths
/// that do not have a registered flag.
pub(crate) async fn review_is_cancelled(ctx: &ServerCtx, review_id: &Id) -> bool {
    ctx.review_cancels.is_signalled(review_id.as_str())
        || ctx
            .reviews_store
            .review_status(review_id)
            .await
            .is_ok_and(|s| s == ReviewStatus::Cancelled)
}

/// Open/blocker/total finding counts for a review, from the persistent store.
pub(crate) async fn review_findings_counts(ctx: &ServerCtx, review_id: &Id) -> (u64, u64, u64) {
    let all = ctx
        .findings_store
        .list_for_review(review_id)
        .await
        .unwrap_or_default();
    let total = all.len() as u64;
    let is_open = |f: &otto_state::ReviewFindingRow| {
        matches!(
            f.state,
            otto_state::FindingState::Open | otto_state::FindingState::Regressed
        )
    };
    let open = all.iter().filter(|f| is_open(f)).count() as u64;
    let blocker = all
        .iter()
        .filter(|f| is_blocking_severity(&f.severity) && is_open(f))
        .count() as u64;
    (total, open, blocker)
}

/// Short, human-facing one-liners for a review's OPEN findings (severity dot + a
/// truncated first line + path:line), highest-severity first, capped at `max`.
/// Used to stream a review step's findings into a chat thread.
pub(crate) async fn review_finding_briefs(
    ctx: &ServerCtx,
    review_id: &Id,
    max: usize,
) -> Vec<String> {
    let all = ctx
        .findings_store
        .list_for_review(review_id)
        .await
        .unwrap_or_default();
    let is_open = |f: &otto_state::ReviewFindingRow| {
        matches!(
            f.state,
            otto_state::FindingState::Open | otto_state::FindingState::Regressed
        )
    };
    let mut open: Vec<&otto_state::ReviewFindingRow> = all.iter().filter(|f| is_open(f)).collect();
    open.sort_by_key(|f| severity_rank(&f.severity));
    open.into_iter()
        .take(max)
        .map(|f| {
            let sev = match severity_rank(&f.severity) {
                0 | 1 => "🔴",
                2 => "🟡",
                _ => "🔵",
            };
            let first = f
                .body
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let body = if first.chars().count() > 140 {
                format!("{}…", first.chars().take(140).collect::<String>())
            } else {
                first.to_string()
            };
            match &f.path {
                Some(p) if !p.is_empty() => {
                    let loc = f.line.map(|l| format!(":{l}")).unwrap_or_default();
                    format!("{sev} {body} (`{p}{loc}`)")
                }
                _ => format!("{sev} {body}"),
            }
        })
        .collect()
}

/// Resolve the global default reviewer provider (the `default_provider` setting,
/// else "claude"). Mirrors [`load_review_config`]'s provider resolution.
pub(crate) async fn default_review_provider(ctx: &ServerCtx) -> String {
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    otto_core::provider::resolve_provider(&[otto_core::provider::global_default(
        global_default.as_ref(),
    )])
}

/// Open-finding counts for a review bucketed by the reviewer severity vocabulary
/// (`bug`/`warn`/`info`). Backs the workflow `review_run` configurable scoring.
pub(crate) async fn review_open_counts_by_severity(
    ctx: &ServerCtx,
    review_id: &Id,
) -> (u64, u64, u64) {
    let all = ctx
        .findings_store
        .list_for_review(review_id)
        .await
        .unwrap_or_default();
    let is_open = |f: &otto_state::ReviewFindingRow| {
        matches!(
            f.state,
            otto_state::FindingState::Open | otto_state::FindingState::Regressed
        )
    };
    let mut bug = 0u64;
    let mut warn = 0u64;
    let mut info = 0u64;
    for f in all.iter().filter(|f| is_open(f)) {
        // Findings arrive in two severity vocabularies depending on which agent
        // wrote them: the reviewer contract (bug/warn/info) and the summarizer's
        // comment tiers (high/medium/info, plus occasional blocker/critical/
        // major/minor). Bucket by synonym — an unrecognized severity counts as
        // the middle tier, NOT info: silently down-weighting unknown severities
        // let 5-high/10-medium reviews score 80 "passed".
        match f.severity.to_ascii_lowercase().as_str() {
            "bug" | "blocker" | "critical" | "high" => bug += 1,
            "warn" | "warning" | "major" | "medium" | "minor" => warn += 1,
            "info" | "nit" | "note" | "low" => info += 1,
            _ => warn += 1,
        }
    }
    (bug, warn, info)
}

/// The repo's bound git-account credential for fetch/push transport, or
/// `None` (no account, unreadable secret) — git then uses the user's own
/// credential helper / SSH agent.
pub(crate) async fn repo_transport_credential(
    ctx: &ServerCtx,
    repo: &otto_core::domain::Repo,
) -> Option<otto_git::transport::TransportCredential> {
    let acc = ctx
        .git_store
        .get_account(repo.git_account_id.as_ref()?)
        .await
        .ok()?;
    let token = otto_core::secrets::get_async(&ctx.secrets, &acc.token_ref)
        .await
        .ok()
        .flatten()?;
    otto_git::transport::TransportCredential::for_repo(token, &acc, repo).ok()
}

/// Resolve the repo's git token (the fetch must reach a private origin) and
/// materialize the PR head via [`otto_review::worktree::materialize_pr_worktree`].
pub(crate) async fn materialize_pr_worktree(
    ctx: &ServerCtx,
    repo: &otto_core::domain::Repo,
    review_id: &Id,
    pr_number: u64,
    source: Option<&str>,
    head_sha: Option<&str>,
    suffix: &str,
) -> Option<PrWorktree> {
    // Re-read the git token (resolve_provider_remote consumed it) so the fetch
    // can reach a private origin. Fetch is metadata-only (updates refs) and
    // never touches a working tree.
    let git_token = repo_transport_credential(ctx, repo).await;
    otto_review::worktree::materialize_pr_worktree(
        repo, review_id, pr_number, source, head_sha, suffix, git_token,
    )
    .await
}

/// True when `a` and `b` name the same directory (symlinks / trailing `/`
/// resolved; falls back to a plain string compare when either is missing).
pub(crate) fn same_dir(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a.trim_end_matches('/') == b.trim_end_matches('/'),
    }
}

/// Fetch PR diff + Jira context and delegate to `run_review_core`.
///
/// `user` is the caller that started the review; their ownership of the repo's
/// bound git account (and any supplied issue account) is enforced here so the
/// review never acts through another user's credentials (S4).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_pr_review_inner(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    review_id: &Id,
    repo_id: &Id,
    pr_number: u64,
    issue_account_id: Option<String>,
    issue_key: Option<String>,
    user_context: Option<String>,
) -> Result<()> {
    // 1. Load repo + its workspace, and resolve provider.
    let repo = ctx.git_store.get_repo(repo_id).await?;
    let workspace = ctx.workspaces.get(&repo.workspace_id).await?;
    let (provider, remote) = resolve_provider_remote(ctx, user, &repo).await?;

    // The authoritative diff is built from local immutable objects after the
    // reviewed checkout exists. Do not fetch/render a truncated forge preview.
    let diff_for_agents = String::new();

    // 3. Optionally fetch the linked Jira story.
    let jira_context = match (issue_account_id, issue_key) {
        (Some(account_id), Some(ref key)) => {
            let ctx_str = async {
                let account = ctx.issues_store.get_account(&account_id).await?;
                // S4: only the issue account's owner (or root) may use its token.
                otto_core::auth::authorize_owner(&account, user)?;
                let token = otto_core::secrets::get_async(&ctx.secrets, &account.token_ref)
                    .await?
                    .ok_or_else(|| {
                        otto_core::Error::Invalid(format!(
                            "token missing for issue account {}",
                            account.id
                        ))
                    })?;
                let client = otto_issues::Tracker::for_account(&account, &token);
                let detail = client.get_issue(key).await?;
                let ctx_str = format!(
                    "## Linked Jira story\n{} — {} [{}]\n\n{}\n\n---\n\n",
                    detail.key, detail.summary, detail.status, detail.description
                );
                otto_core::Result::Ok(ctx_str)
            }
            .await;
            match ctx_str {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!(review = %review_id, "failed to fetch Jira story: {e}; proceeding without context");
                    None
                }
            }
        }
        _ => None,
    };

    // 4. Resolve the PR's source/destination branches and materialize the LATEST
    //    source into an ISOLATED worktree, so reviewers verify against the real
    //    code being merged — never the user's working tree (which may be on a
    //    different branch and must NOT be mutated). A fork PR's branch is not on
    //    `origin`, so the PR head ref / head sha is the fallback. If no checkout
    //    can be built the review still runs in repo.path, but the prompt stops
    //    claiming a source checkout and the run is partial (resolves nothing).
    let pr_detail = match provider.get_pr(&remote, pr_number).await {
        Ok(pr) => Some(pr),
        Err(e) => {
            tracing::warn!(review = %review_id, "get_pr (for branch names) failed: {e}");
            None
        }
    };
    let pr_wt = materialize_pr_worktree(
        ctx,
        &repo,
        review_id,
        pr_number,
        pr_detail.as_ref().map(|p| p.summary.source_branch.as_str()),
        pr_detail
            .as_ref()
            .and_then(|p| p.summary.head_sha.as_deref()),
        "",
    )
    .await;
    let branches = pr_detail.map(|pr| ReviewBranches {
        source: pr.summary.source_branch,
        dest: pr.summary.target_branch,
        checked_out: pr_wt.is_some(),
    });
    let review_cwd = pr_wt
        .as_ref()
        .map_or_else(|| repo.path.clone(), |w| w.path.clone());

    let result = run_review_core(
        ctx,
        review_id,
        &review_cwd,
        diff_for_agents,
        jira_context,
        user_context,
        &workspace,
        repo_id,
        pr_number,
        branches.as_ref(),
        None,
        None,
    )
    .await;

    // Tear down the throwaway worktree + branch (best-effort; leaves no litter in
    // the user's branch list). The branch is review-only, so force-delete is safe.
    if let Some(wt) = pr_wt {
        teardown_pr_worktree(&repo.path, wt).await;
    }
    result
}

#[cfg(test)]
mod review_cleanup_tests {
    use super::*;

    #[test]
    fn finished_review_temp_files_are_removed_but_not_the_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        for f in [
            "otto-review-R1.diff",
            "otto-review-R1-0.json",
            "otto-review-R1-0.prompt",
        ] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        std::fs::write(d.join("otto-review-R2.diff"), "other").unwrap();
        std::fs::create_dir(d.join("otto-review-R1-wt")).unwrap();
        remove_review_temp_files_in(d, "otto-review-R1");
        assert!(!d.join("otto-review-R1.diff").exists());
        assert!(!d.join("otto-review-R1-0.json").exists());
        assert!(!d.join("otto-review-R1-0.prompt").exists());
        assert!(
            d.join("otto-review-R2.diff").exists(),
            "another review's files stay"
        );
        assert!(
            d.join("otto-review-R1-wt").is_dir(),
            "directories are never removed"
        );
    }

    #[test]
    fn only_settled_review_agents_are_retryable() {
        for s in ["done", "error", "skipped"] {
            assert!(agent_retryable(s), "{s}");
        }
        for s in ["pending", "running", "waiting", ""] {
            assert!(!agent_retryable(s), "{s}");
        }
    }
}
