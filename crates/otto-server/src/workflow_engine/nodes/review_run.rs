//! The `review_run` workflow node, moved out of `execute_node`'s match (see
//! `nodes/mod.rs`). `execute` builds one [`ReviewPlan`] (config, reviewers,
//! goals, prior run context) and reviews each target in two phases —
//! [`launch`] (repo check, checkpointed review start) and [`score_target`]
//! (counts → score → verdict → output). The gate itself is pure and
//! unit-tested: [`review_score`], [`review_verdict`], [`goals_from_reply`]
//! and [`aggregate`].

use super::super::*;
use super::NodeCall;

/// Per-severity deductions (percent off 100) over the OPEN findings —
/// `scoring: { bug, warn, info }`. Defaults (20/5/5) preserve the historical
/// blocking/advisory formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workflow_engine) struct ScoreWeights {
    pub(in crate::workflow_engine) bug: i64,
    pub(in crate::workflow_engine) warn: i64,
    pub(in crate::workflow_engine) info: i64,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            bug: 20,
            warn: 5,
            info: 5,
        }
    }
}

impl ScoreWeights {
    /// The node's `scoring` overrides over the defaults.
    fn from_params(p: &Value) -> Self {
        let d = Self::default();
        let weight = |key: &str, default: i64| -> i64 {
            p.get("scoring")
                .and_then(|s| s.get(key))
                .and_then(Value::as_i64)
                .unwrap_or(default)
        };
        Self {
            bug: weight("bug", d.bug),
            warn: weight("warn", d.warn),
            info: weight("info", d.info),
        }
    }
}

/// The review's own score: 100 minus the weighted OPEN findings, clamped to
/// 0..=100. An EMPTY diff scores 0, not 100: the formula starts at a perfect
/// 100 and subtracts findings, so "nothing was reviewed" and "reviewed,
/// spotless" would produce the same number. Only one of them earned it.
pub(in crate::workflow_engine) fn review_score(
    bugs: u64,
    warns: u64,
    infos: u64,
    w: ScoreWeights,
    no_changes: bool,
) -> i64 {
    if no_changes {
        return 0;
    }
    (100 - bugs as i64 * w.bug - warns as i64 * w.warn - infos as i64 * w.info).clamp(0, 100)
}

/// Whether one target's review passed, and why (not).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workflow_engine) struct ReviewVerdict {
    /// Completed (status `done`) with full reviewer coverage.
    pub(in crate::workflow_engine) complete: bool,
    pub(in crate::workflow_engine) passed: bool,
    pub(in crate::workflow_engine) reason: String,
}

/// A review must COMPLETE (status "done" with full coverage), have had
/// something to review, AND clear the threshold to pass. The reasons stay
/// distinct: an incomplete review (error / timeout / still running) and an
/// empty diff BOTH produce zero findings, so reporting either as "below
/// threshold" is nonsense — the score was never the problem.
pub(in crate::workflow_engine) fn review_verdict(
    status: &str,
    coverage_complete: bool,
    no_changes: bool,
    score: i64,
    threshold: i64,
    base: &str,
) -> ReviewVerdict {
    let complete = status == "done" && coverage_complete;
    let passed = complete && !no_changes && score >= threshold;
    let reason = if passed {
        "passed".to_string()
    } else if !complete {
        if status == "done" {
            "review coverage incomplete or summary fallback not allowed".into()
        } else {
            format!("review did not complete (status: {status})")
        }
    } else if no_changes {
        format!("empty diff vs {base} — nothing was reviewed")
    } else {
        format!("below threshold ({score} < {threshold})")
    };
    ReviewVerdict {
        complete,
        passed,
        reason,
    }
}

/// Read the goals agent's reply: `(goals_score, goals_detail)`.
///
/// The reply is searched for the object the prompt asked for — one with an
/// overall `score` or a `goals` array — so a quoted `{"name":…}` from a
/// `package.json` ahead of it can't shadow it. A reply with no such object,
/// or one without an overall score, is NOT parseable: it keeps the review
/// score (the goals can't be graded) and says so in the detail, instead of
/// silently reporting `goals: []` as if no goals were configured.
pub(in crate::workflow_engine) fn goals_from_reply(
    reply: &str,
    review_score: i64,
) -> (Option<i64>, Value) {
    let found = otto_core::text::extract_json_where(reply, |v| {
        v.get("score").and_then(Value::as_i64).is_some()
            || v.get("goals").is_some_and(Value::is_array)
    });
    match found {
        Some(v) => match v.get("score").and_then(Value::as_i64) {
            Some(gs) => (
                Some(gs.clamp(0, 100)),
                v.get("goals").cloned().unwrap_or(json!([])),
            ),
            None => {
                let mut detail = v
                    .get("goals")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                detail.push(json!({ "note": "goals eval reply had no overall score" }));
                (Some(review_score), Value::Array(detail))
            }
        },
        None => (
            Some(review_score),
            json!([{ "note": "goals eval reply not parseable" }]),
        ),
    }
}

/// Fold the per-target outputs into the step output: `(output, passed)`.
///
/// Single target: its output as is. Several: the FIRST reviewed repo's fields
/// stay mirrored at the top level (back-compat for single-repo consumers);
/// the strict union gates multi-repo runs — worst score, pass only when EVERY
/// target was reviewed and passed — with per-repo detail under `reviews[]`
/// and every reviewed reference under `repos[]`. `outs` is non-empty.
pub(in crate::workflow_engine) fn aggregate(
    outs: Vec<serde_json::Map<String, Value>>,
    n_targets: usize,
) -> (serde_json::Map<String, Value>, bool) {
    let mut out = outs.first().cloned().unwrap_or_default();
    if n_targets <= 1 {
        let passed = out.get("passed").and_then(Value::as_bool).unwrap_or(false);
        return (out, passed);
    }
    let agg_score = outs
        .iter()
        .filter_map(|o| o.get("score").and_then(Value::as_i64))
        .min()
        .unwrap_or(0);
    let agg_passed = outs.len() == n_targets
        && outs
            .iter()
            .all(|o| o.get("passed").and_then(Value::as_bool).unwrap_or(false));
    out.insert("score".into(), json!(agg_score));
    out.insert("passed".into(), json!(agg_passed));
    // EVERY reviewed reference, in the `repos[]` shape collect_pr_targets
    // consumes — without this a downstream git_pr would see only the first
    // repo mirrored at top level and never open the other entries' PRs.
    let repo_targets: Vec<Value> = outs
        .iter()
        .map(|o| {
            let mut m = serde_json::Map::new();
            for k in ["repo_id", "worktree", "base"] {
                if let Some(v) = o.get(k) {
                    m.insert(k.to_string(), v.clone());
                }
            }
            Value::Object(m)
        })
        .collect();
    out.insert("repos".into(), Value::Array(repo_targets));
    out.insert(
        "reviews".into(),
        Value::Array(outs.into_iter().map(Value::Object).collect()),
    );
    (out, agg_passed)
}

/// What every target of one `review_run` step shares: the node call, the
/// reviewer config, the goals, the prior run context and the progress-post
/// label fragments.
struct ReviewPlan<'a> {
    ctx: &'a ServerCtx,
    ws: &'a Workspace,
    user: &'a User,
    node: &'a WorkflowNode,
    input: &'a Value,
    env: &'a RunEnv,
    scope: &'a StepScope,
    session_tx: &'a tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    log_tx: &'a tokio::sync::mpsc::UnboundedSender<String>,
    activity_tx: &'a tokio::sync::mpsc::UnboundedSender<NodeActivity>,
    progress: &'a ProgressSink,
    threshold: i64,
    await_done: bool,
    timeout_s: u64,
    mode: otto_core::domain::ReviewMode,
    providers: Vec<String>,
    lenses: Vec<String>,
    check_specs: Vec<(String, String)>,
    cfg_override: Option<otto_core::api::ReviewConfig>,
    weights: ScoreWeights,
    goals: Vec<String>,
    jira_context: Option<String>,
    run_context: Option<String>,
    iter_label: String,
    /// " · lenses: …", " · providers: …", " · N check(s) …" (each may be empty).
    label_tail: String,
    n_targets: usize,
}

impl ReviewPlan<'_> {
    fn multi(&self) -> bool {
        self.n_targets > 1
    }
}

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let p = &call.node.params;
    // When set, the step itself FAILS if the score is below threshold — so a
    // downstream "create PR" step is error-skipped unless the review passed.
    let require_pass = p
        .get("require_pass")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (targets, logs_pre) = resolve_targets(
        call.ctx,
        call.ws,
        p,
        &call.input,
        call.env,
        call.env.run_cwd.as_str(),
    )
    .await?;
    let (plan, mut logs) = plan_review(&call, targets.len(), logs_pre).await;
    // Review every target; a per-target failure in the multi-repo case
    // is logged and skipped (one bad repo never sinks the others), the
    // single-target case keeps its hard error.
    let mut outs: Vec<serde_json::Map<String, Value>> = Vec::new();
    for (t_idx, target) in targets.iter().enumerate() {
        let Some(launched) = launch(&plan, t_idx, target, &mut logs).await? else {
            continue;
        };
        let status = await_review(
            plan.ctx,
            plan.ws,
            &plan.env.run_id,
            &launched.review_id,
            plan.await_done.then_some(plan.timeout_s),
            plan.session_tx,
            &launched.tag,
            &mut logs,
        )
        .await?;
        outs.push(score_target(&plan, &launched, &status, &mut logs).await);
    }
    if outs.is_empty() {
        return Err(otto_core::Error::Upstream(format!(
            "review_run: no declared repo could be reviewed ({})",
            logs.join("; ")
        )));
    }
    let (mut out, passed) = aggregate(outs, plan.n_targets);
    // `require_pass`: fail the step (so downstream gates error-skip) when the
    // score didn't clear the bar. A failed check surfaces as a blocking
    // finding from the checks reviewer, which drops the score below it.
    if require_pass && !passed {
        let reason = if plan.multi() {
            "one or more required review targets did not pass".to_string()
        } else {
            out.get("reason")
                .and_then(Value::as_str)
                .unwrap_or("review did not pass")
                .to_string()
        };
        out.insert(
            "_workflow_error".into(),
            json!(format!("review_run: {reason} (require_pass)")),
        );
    }
    Ok((Value::Object(out), logs))
}

/// Build the step's shared [`ReviewPlan`] from the node params, the run
/// input and the run's context files. Returns it with the step's opening
/// log lines.
async fn plan_review<'a>(
    call: &'a NodeCall<'a>,
    n_targets: usize,
    mut logs: Vec<String>,
) -> (ReviewPlan<'a>, Vec<String>) {
    let NodeCall {
        ctx,
        ws,
        user,
        node,
        env,
        scope,
        session_tx,
        log_tx,
        activity_tx,
        progress,
        ..
    } = *call;
    let input = &call.input;
    let p = &node.params;
    let threshold = p.get("threshold").and_then(Value::as_u64).unwrap_or(80) as i64;
    let await_done = p.get("await").and_then(Value::as_bool).unwrap_or(true);
    // A multi-agent review over a large PR routinely runs past 15 minutes
    // (a 170-file diff took 13.8 min with 10 reviewers; a 20k-line one blew
    // past it). `.min(1800)` was a CEILING that silently capped even an
    // explicit node param, so the node abandoned reviews that then went on
    // to complete and persist findings minutes later — the run scored those
    // as a clean sweep. Default 5h, node-tunable, with a 60s FLOOR — the
    // same shape as the human_approval arm. RUN_WALL_CLOCK_TIMEOUT (10h)
    // remains the real backstop.
    let timeout_s = p
        .get("timeout_s")
        .and_then(Value::as_u64)
        .unwrap_or(18_000)
        .max(60);
    // R2: which execution mode the reviewers run in. The RUN's own input
    // wins over the node's param (that is what makes the run dialog an
    // override); a node param wins over the repo's stored config. Read
    // from `env.run_input`, never the node input — node inputs lose
    // run-level keys hop by hop (`assemble_input`).
    let (mode, mode_src) = match resolve_review_mode_source(&env.run_input, p) {
        Some(r) => r,
        None => {
            // `otto_core::Id` is a plain `String` alias.
            let rid: Option<Id> = p
                .get("repo_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            crate::modules::effective_review_mode(ctx, rid.as_ref()).await
        }
    };
    let _ = log_tx.send(format!("review_run: mode {} ({mode_src})", mode.as_str()));
    // Reviewer providers + lenses (skills) — drive the SAME multi-agent
    // engine as PR review (multi-provider × multi-lens, one summarizer that
    // consolidates + scores). Empty → the stored/default PR-review config.
    let providers = param_str_list(p, "providers");
    let lenses = {
        let mut l = param_str_list(p, "lenses");
        for s in param_str_list(p, "skills") {
            if !l.contains(&s) {
                l.push(s);
            }
        }
        l
    };
    // Iteration label so a fix→review loop streams "Review #1, #2, …".
    let iter_label = input
        .get("_iteration")
        .and_then(Value::as_u64)
        .map(|n| format!("Review #{n}"))
        .unwrap_or_else(|| "Review".to_string());
    // Rich reviewer config (PR-review parity, design §F): `reviewers` =
    // per-lens provider sets + optional per-reviewer instructions, with an
    // optional `summarizer`. Falls back to the flat providers/lenses form,
    // then to the stored/default PR-review config.
    let reviewers = p
        .get("reviewers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // User-defined reviewer CHECKS — commands (e.g. `go test -tags=component
    // ./...`) the reviewer agent RUNS and reports failures on, in addition
    // to goals. A safety net for what the implementer's skill may have
    // skipped. From the node's `checks` or the run input's `checks`.
    let check_specs = parse_checks(p.get("checks").or_else(|| input.get("checks")));
    let cfg_override = review_config(ctx, p, &reviewers, &providers, &lenses, &check_specs).await;
    let label_tail = label_tail(&reviewers, &lenses, &providers, &check_specs);
    // Optional goals — assessed by an agent per target and blended in.
    let goals: Vec<String> = p
        .get("goals")
        .and_then(Value::as_array)
        .or_else(|| input.get("goals").and_then(Value::as_array))
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let (jira_context, run_context) = prior_run_context(env, p, &mut logs);
    let plan = ReviewPlan {
        ctx,
        ws,
        user,
        node,
        input,
        env,
        scope,
        session_tx,
        log_tx,
        activity_tx,
        progress,
        threshold,
        await_done,
        timeout_s,
        mode,
        providers,
        lenses,
        check_specs,
        cfg_override,
        weights: ScoreWeights::from_params(p),
        goals,
        jira_context,
        run_context,
        iter_label,
        label_tail,
        n_targets,
    };
    (plan, logs)
}

/// The reviewer config override: the rich `reviewers` form, else the flat
/// providers/lenses form, else `None` (the stored/default PR-review config) —
/// plus a dedicated "Required checks" reviewer when checks were given.
async fn review_config(
    ctx: &ServerCtx,
    p: &Value,
    reviewers: &[Value],
    providers: &[String],
    lenses: &[String],
    check_specs: &[(String, String)],
) -> Option<otto_core::api::ReviewConfig> {
    let mut cfg_override = if !reviewers.is_empty() {
        let default_provider = crate::modules::default_review_provider(ctx).await;
        Some(crate::modules::workflow_review_config_from_json(
            &default_provider,
            reviewers,
            p.get("summarizer"),
        ))
    } else if providers.is_empty() && lenses.is_empty() {
        None
    } else {
        let default_provider = crate::modules::default_review_provider(ctx).await;
        Some(crate::modules::workflow_review_config(
            &default_provider,
            providers,
            lenses,
        ))
    };
    // Append a dedicated "Required checks" reviewer that RUNS the commands
    // and reports any failure as a blocking (bug) finding — so it drops the
    // score and the loop keeps fixing until the checks pass. Delegated to
    // the agent (it has the repo + the implementer's context), not run by us.
    if !check_specs.is_empty() {
        let default_provider = crate::modules::default_review_provider(ctx).await;
        let cfg = cfg_override.get_or_insert_with(|| {
            crate::modules::workflow_review_config(&default_provider, &[], &[])
        });
        cfg.agents
            .push(checks_review_agent(&default_provider, check_specs));
    }
    cfg_override
}

/// Progress-post label fragments shared by every target.
fn label_tail(
    reviewers: &[Value],
    lenses: &[String],
    providers: &[String],
    check_specs: &[(String, String)],
) -> String {
    let lens_list: Vec<String> = if reviewers.is_empty() {
        lenses.to_vec()
    } else {
        reviewers
            .iter()
            .filter_map(|r| {
                r.get("lens")
                    .or_else(|| r.get("skill"))
                    .or_else(|| r.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    };
    let lens_txt = if lens_list.is_empty() {
        String::new()
    } else {
        format!(" · lenses: {}", lens_list.join(", "))
    };
    let prov_txt = if providers.is_empty() {
        String::new()
    } else {
        format!(" · providers: {}", providers.join(", "))
    };
    let chk_txt = if check_specs.is_empty() {
        String::new()
    } else {
        format!(
            " · {} check(s) delegated to the reviewer",
            check_specs.len()
        )
    };
    format!("{lens_txt}{prov_txt}{chk_txt}")
}

/// What this run already established about the change, handed to the
/// reviewers: `(ticket, step briefs)`.
///
/// Every OTHER node kind gets the run context directory named in its
/// preamble and reads it; `review_run` is not an agent node, so its
/// reviewers used to see the diff and nothing else — no ticket, no brief
/// from the steps that ran before them. They then re-derive the system from
/// raw hunks, and behavior that is documented, ticketed and intentional
/// comes back as a defect of whichever change happened to touch its line.
/// The engine already renders both blocks into every reviewer prompt
/// (`run_review_core`); only this caller withheld them.
///
/// `include_context: false` opts out.
fn prior_run_context(
    env: &RunEnv,
    p: &Value,
    logs: &mut Vec<String>,
) -> (Option<String>, Option<String>) {
    let want_context = p
        .get("include_context")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let (jira_context, run_context) = if want_context {
        let jira = env.files.read_jira_md();
        // The step handoffs, oldest first, capped — the reviewer prompt
        // also carries the whole diff, so this must not grow unbounded.
        let mut brief = String::new();
        for name in env.files.list_step_mds() {
            let Some(body) = env.files.read_named(&name) else {
                continue;
            };
            if brief.len() + body.len() > REVIEW_CONTEXT_CAP {
                brief.push_str(
                    "\n\n[remaining step files omitted — context cap reached; \
                             they are in the run context directory]\n",
                );
                break;
            }
            brief.push_str(&format!("\n\n### {name}\n\n{body}"));
        }
        let framed = (!brief.trim().is_empty()).then(|| {
            format!(
                "These steps ran BEFORE this review, in this workflow, over this same \
                         change. Treat them as established fact about the system — not as claims \
                         to re-verify from the diff. In particular: behavior they record as \
                         existing, documented or intentional was NOT introduced by this change, \
                         and impact they already assessed is not yours to re-litigate. Use them \
                         so you do not reconstruct the system from hunks alone.{brief}"
            )
        });
        (jira, framed)
    } else {
        (None, None)
    };
    if jira_context.is_some() || run_context.is_some() {
        logs.push(format!(
            "review_run: reviewers given prior run context ({}{}{} chars)",
            if jira_context.is_some() {
                "ticket + "
            } else {
                ""
            },
            if run_context.is_some() {
                "step briefs, "
            } else {
                ""
            },
            jira_context.as_ref().map_or(0, |s| s.len())
                + run_context.as_ref().map_or(0, |s| s.len()),
        ));
    }
    (jira_context, run_context)
}

/// One target whose review was started (or adopted).
struct Launched {
    repo_id: String,
    worktree: String,
    review_id: Id,
    /// The RESOLVED base branch (what was actually reviewed).
    base: String,
    no_changes: bool,
    /// " [label i/n]" in the multi-repo case, else empty.
    tag: String,
}

/// Launch phase: validate the repo, post the start line, and start (or adopt,
/// on a restart) the checkpointed review. `Ok(None)` is a multi-repo target
/// that was logged and skipped; the single-target case keeps its hard error.
async fn launch(
    plan: &ReviewPlan<'_>,
    t_idx: usize,
    target: &ReviewTarget,
    logs: &mut Vec<String>,
) -> Result<Option<Launched>> {
    let (repo_id, worktree, want_base, label) = target;
    let ReviewPlan {
        ctx,
        node,
        input,
        scope,
        session_tx,
        log_tx,
        progress,
        threshold,
        ..
    } = *plan;
    let run_id = &plan.env.run_id;
    let multi = plan.multi();
    let iter_label = &plan.iter_label;
    let label_tail = &plan.label_tail;
    // Validate the repo exists (clear error if not); the review
    // engine looks the repo up again by id internally. Its name
    // labels the per-repo progress lines when the target didn't
    // carry one (input-published references).
    let label = match ctx.git_store.get_repo(repo_id).await {
        Ok(r) if label.is_empty() => r.name,
        Ok(_) => label.clone(),
        Err(e) => {
            let msg = format!("review_run [{label}]: repo: {e}");
            if multi {
                logs.push(msg);
                return Ok(None);
            }
            return Err(otto_core::Error::NotFound(msg));
        }
    };
    let tag = if multi {
        format!(" [{} {}/{}]", label, t_idx + 1, plan.n_targets)
    } else {
        String::new()
    };
    if progress.enabled() {
        progress.post(format!(
            "🔍 *{iter_label}{tag}* started (pass ≥ {threshold}){label_tail}"
        ));
    }
    let mut association = loop_checkpoint(
        node,
        node.id.split('#').next().unwrap_or(&node.id),
        scope.iter.unwrap_or(0),
        t_idx,
        json!({"params":node.params,"input":input,"iteration":scope.iter,"target":t_idx}),
    );
    association.node_id = format!("{}#review.{t_idx}", node.id);
    association.name = format!("{} — {}", node_display_name(node), label);
    let prior_review_id = WorkflowsRepo::new(ctx.pool.clone())
        .checkpoint(run_id, &association.node_id)
        .await?
        .and_then(|checkpoint| checkpoint.output)
        .and_then(|output| {
            output
                .get("review_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    let (review_id, resolved_base, no_changes) = match crate::modules::run_review_for_branch(
        ctx,
        repo_id,
        worktree,
        want_base.as_deref(),
        plan.cfg_override.clone(),
        plan.jira_context.clone(),
        plan.run_context.clone(),
        Some(plan.mode),
        (run_id, association),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("review_run{tag}: {e}");
            if multi {
                logs.push(msg);
                if progress.enabled() {
                    progress.post(format!(
                        "🔍 *{iter_label}{tag}* skipped — {}",
                        truncate(&e.to_string(), 200)
                    ));
                }
                return Ok(None);
            }
            return Err(e);
        }
    };
    let _ = session_tx.send(AgentAssociation::Review(review_id.clone()));
    // Publish the RESOLVED branch from here on — the loop harvest,
    // the repos registry and a downstream git_pr must target what
    // was actually reviewed, not the pre-resolution wish.
    let base = resolved_base.branch.clone();
    let action = if prior_review_id.as_deref() == Some(&review_id) {
        "adopted"
    } else {
        "started"
    };
    let line = format!(
        "review_run{tag}: {action} review {review_id} ({worktree} vs {base}; iteration {})",
        scope.iter.unwrap_or(0)
    );
    let _ = log_tx.send(line.clone());
    logs.push(line);
    if no_changes {
        // Nothing was reviewed. Say so where the run is read — the
        // step otherwise looks identical to a clean sweep (status
        // "done", 0 findings) and used to score 100/PASS.
        logs.push(format!(
            "review_run{tag}: EMPTY diff vs {base} — 0 reviewers ran; \
                     this is NOT a clean review. Check the base branch: a run that \
                     declares the PR's own source branch as the base diffs it against itself."
        ));
    }
    Ok(Some(Launched {
        repo_id: repo_id.clone(),
        worktree: worktree.clone(),
        review_id,
        base,
        no_changes,
        tag,
    }))
}

/// Goals phase: grade the goals against the target's worktree with the
/// configured summarizer provider. `(None, [])` without goals or with an
/// empty diff — goals graded against code this run never reviewed would
/// blend a passing number into the score.
async fn grade_goals(
    plan: &ReviewPlan<'_>,
    t: &Launched,
    review_score: i64,
    logs: &mut Vec<String>,
) -> (Option<i64>, Value) {
    if plan.goals.is_empty() || t.no_changes {
        return (None, json!([]));
    }
    let p = &plan.node.params;
    // Score goals with the configured summarizer provider (it's the
    // node's consolidating agent), else the first reviewer provider,
    // else the workspace/global default — never a bare hardcoded
    // "claude" that ignores the user's Providers config.
    let goals_provider = match p
        .get("summarizer")
        .and_then(|s| s.get("provider"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        Some(s) => s.to_string(),
        None => match plan.providers.first() {
            Some(s) => s.clone(),
            None => crate::modules::default_review_provider(plan.ctx).await,
        },
    };
    let gprompt = format!(
        "Assess whether each goal below is met for the repository at `{}` \
         (review the code/tests). Reply with ONLY JSON of the form \
         {{\"goals\":[{{\"goal\":\"...\",\"met\":true,\"score\":0,\"note\":\"...\"}}],\"score\":0}} \
         where each score and the overall score are 0–100.\n\nGoals:\n- {}",
        t.worktree,
        plan.goals.join("\n- ")
    );
    match run_node_agent(
        plan.ctx,
        plan.ws,
        plan.user,
        plan.node,
        &plan.env.wf_name,
        &plan.env.run_id,
        &goals_provider,
        None,
        &gprompt,
        &t.worktree,
        None,
        plan.session_tx,
        plan.log_tx,
        plan.activity_tx,
    )
    .await
    {
        Ok((reply, _sid)) => goals_from_reply(&reply, review_score),
        Err(e) => {
            logs.push(format!("review_run{}: goals eval failed: {e}", t.tag));
            (Some(review_score), json!([{ "note": "goals eval failed" }]))
        }
    }
}

/// Score/verdict phase: counts → review score → goals → verdict, the chat
/// verdict post, and the target's output object.
async fn score_target(
    plan: &ReviewPlan<'_>,
    t: &Launched,
    status: &str,
    logs: &mut Vec<String>,
) -> serde_json::Map<String, Value> {
    let ctx = plan.ctx;
    let (threshold, iter_label) = (plan.threshold, &plan.iter_label);
    let (review_id, base, tag, no_changes) = (&t.review_id, &t.base, &t.tag, t.no_changes);
    let (total, open, blocker) = crate::modules::review_findings_counts(ctx, review_id).await;
    let blocking = blocker as i64;
    let advisory = (open.saturating_sub(blocker)) as i64;
    let (bugs, warns, infos) = crate::modules::review_open_counts_by_severity(ctx, review_id).await;
    let review_score = review_score(bugs, warns, infos, plan.weights, no_changes);
    let (goals_score, goals_detail) = grade_goals(plan, t, review_score, logs).await;
    let score = match goals_score {
        Some(gs) => (review_score + gs) / 2,
        None => review_score,
    };
    let coverage_complete = ctx
        .reviews_store
        .get_review(review_id)
        .await
        .map(|review| {
            review_coverage_complete(
                &review,
                plan.node
                    .params
                    .get("allow_summary_fallback")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )
        })
        .unwrap_or(false);
    let ReviewVerdict {
        complete,
        passed,
        reason,
    } = review_verdict(
        status,
        coverage_complete,
        no_changes,
        score,
        threshold,
        base,
    );
    logs.push(format!(
        "review_run{tag}: score {score} (review {review_score}{}) — {reason}",
        goals_score
            .map(|g| format!(", goals {g}"))
            .unwrap_or_default(),
    ));
    // Stream the verdict + top findings to the chat thread.
    let finding_briefs = crate::modules::review_finding_briefs(ctx, review_id, 10).await;
    if plan.progress.enabled() {
        let verdict = if passed {
            "✅ passed".to_string()
        } else if !complete {
            format!("⚠️ review did not complete (status: {status})")
        } else if no_changes {
            format!("⚠️ empty diff vs `{base}` — nothing was reviewed")
        } else {
            "⚠️ below threshold".to_string()
        };
        // Only headline a score for a COMPLETED review that had a
        // diff — otherwise the "100/100" is an artifact of a review
        // that never finished or never had anything to look at.
        let mut msg = if complete && !no_changes {
            format!("🔍 *{iter_label}{tag}* done — score *{score}/100* (pass ≥ {threshold}) — {verdict}")
        } else {
            format!("🔍 *{iter_label}{tag}* — {verdict}")
        };
        if no_changes {
            msg.push_str("\nNo reviewers ran. Check the base branch this run declared.");
        } else if finding_briefs.is_empty() {
            msg.push_str("\nFindings: none 🎉");
        } else {
            msg.push_str(&format!("\nFindings ({open} open):"));
            for b in &finding_briefs {
                msg.push_str(&format!("\n • {b}"));
            }
        }
        plan.progress.post(msg);
    }
    let out = json!({
        "review_id": review_id, "status": status,
        // Publish the exact reference reviewed so a downstream git_pr
        // (or another review) opens the PR on the SAME
        // repo/branch/worktree — no need to re-type them on the PR
        // node (design: PR is aware).
        "repo_id": t.repo_id, "base": base, "worktree": t.worktree,
        "total": total, "open": open, "blocking": blocking, "advisory": advisory,
        "severity": { "bug": bugs, "warn": warns, "info": infos },
        "review_score": review_score, "goals_score": goals_score, "goals": goals_detail,
        // The checks delegated to the reviewer agent (it runs them and
        // reports failures as findings — see the cfg injection above).
        "checks_requested": plan.check_specs.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>(),
        "score": score, "threshold": threshold, "passed": passed,
        // TRUE ⇒ the diff vs `base` was empty: no reviewer ran and
        // `total: 0` means "nothing looked at", not "nothing found".
        // A downstream step that posts or gates on this review must
        // branch on it rather than on the finding counts.
        "no_changes": no_changes, "coverage_complete": coverage_complete, "reason": reason,
        "findings": finding_briefs, "providers": plan.providers, "lenses": plan.lenses,
    });
    out.as_object().cloned().unwrap_or_default()
}

/// One review target: `(repo_id, worktree, want_base, label)`.
type ReviewTarget = (String, String, Option<String>, String);

/// Which repo(s)/worktree(s)/base(s) to review — (repo_id, worktree,
/// want_base, label), in priority order:
///
/// 1. NODE params name a target → the classic single-repo path.
/// 2. The input carries a published `repos[]` array (a multi-repo
///    review/loop handing its full set forward) → one target each,
///    so a chained review re-reviews EVERY repo, not just the
///    first-mirrored one.
/// 3. The input carries a published single `worktree` reference →
///    single target (an upstream step handing off the exact place
///    it worked).
/// 4. The run's repos registry (the declared branches/worktrees,
///    source+destination) → one target per valid entry.
/// 5. One implicit target from the run context (design §B
///    resilience: working_directory/run_cwd → registered repo).
///
/// A bare `input.repo_id`/`working_directory` never counts as a
/// target by itself — run-start seeding puts those into the input
/// of every declared run, and counting them would permanently mask
/// the multi-entry paths.
///
/// Returns the targets plus the notes emitted while resolving them, which
/// the step folds into its logs.
async fn resolve_targets(
    ctx: &ServerCtx,
    ws: &Workspace,
    p: &Value,
    input: &Value,
    env: &RunEnv,
    run_cwd: &str,
) -> Result<(Vec<ReviewTarget>, Vec<String>)> {
    let params_explicit = p
        .get("repo_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
        || p.get("worktree_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some();
    // Notes emitted while resolving targets, before the per-target log
    // buffer below exists; folded into it verbatim.
    let mut logs_pre: Vec<String> = Vec::new();
    let input_repo_targets: Vec<(String, String, Option<String>, String)> = input
        .get("repos")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let rid = t
                        .get("repo_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())?;
                    let wt = t
                        .get("worktree")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())?;
                    let base = t
                        .get("base")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    Some((rid.to_string(), expand_tilde(wt), base, String::new()))
                })
                .collect()
        })
        .unwrap_or_default();
    let input_worktree_ref = input
        .get("worktree")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
        || input
            .get("worktree_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some();
    let mut targets: Vec<(String, String, Option<String>, String)> = Vec::new();
    if !params_explicit && !input_repo_targets.is_empty() {
        // `repos[]` is threaded VERBATIM from node to node (every
        // agent_prompt copies it into its output), so it is a snapshot of
        // what the run started with. `repos.json` is the live registry —
        // run-brief.md calls it "the authoritative list", and a checkout
        // step that discovers the PR's real destination branch corrects it
        // there. Let the registry win on `base` for an entry it also
        // declares, or that correction is silently discarded and the
        // review diffs against the stale branch anyway.
        let declared = env.files.repos();
        targets = input_repo_targets
            .into_iter()
            .map(|(rid, wt, base, label)| {
                let live = declared
                    .iter()
                    .find(|e| e.repo_id.as_deref() == Some(rid.as_str()))
                    .or_else(|| {
                        declared
                            .iter()
                            .find(|e| e.worktree.as_deref() == Some(wt.as_str()))
                    })
                    .and_then(|e| e.base.clone())
                    .filter(|b| !b.trim().is_empty());
                match live {
                    Some(b) if Some(&b) != base.as_ref() => {
                        logs_pre.push(format!(
                            "review_run: base for {rid} taken from repos.json ({b}), \
                                     not the value threaded through the run input ({})",
                            base.as_deref().unwrap_or("unset")
                        ));
                        (rid, wt, Some(b), label)
                    }
                    _ => (rid, wt, base, label),
                }
            })
            .collect();
    } else if !params_explicit && !input_worktree_ref {
        // Declarations: EVERY entry failed to resolve → fail with the
        // reasons. Falling through to the run-cwd path would review
        // whatever happens to be checked out — and an empty diff there
        // reads as a false-green "score 100" on a run whose declared
        // target never resolved.
        let declared = env.files.repos();
        if let Some(msg) = crate::workflow_context::all_declared_errored(&declared) {
            return Err(otto_core::Error::Invalid(format!(
                "review_run: no declared repos[] entry resolved — {msg}"
            )));
        }
        for e in declared
            .iter()
            .filter(|e| e.error.is_none() && e.repo_id.is_some() && e.worktree.is_some())
        {
            targets.push((
                e.repo_id.clone().unwrap_or_default(),
                e.worktree.clone().unwrap_or_default(),
                // Declared source, else per-repo default-branch
                // detection inside run_review_for_branch.
                e.base.clone(),
                e.repo.clone(),
            ));
        }
    }
    if targets.is_empty() {
        // Resilient repo resolution (design §B): explicit repo_id wins,
        // else derive it from the step's worktree_path / the run's
        // working_directory / run_cwd against the workspace's registered
        // repos. Never a bare "missing repo_id" for a workflow that was
        // given a working directory.
        let repo_id = resolve_step_repo_id(ctx, ws, p, input, run_cwd)
            .await?
            .ok_or_else(|| {
                otto_core::Error::Invalid(
                    "review_run: no repo_id; pass repo_id or a working_directory/worktree_path \
                             under a registered repo, or declare repos on the run"
                        .into(),
                )
            })?;
        // The directory the implementer worked in (the run's
        // working_directory) IS what we review — fall back to it
        // (run_cwd), not the bare repo checkout, so the reviewer sees the
        // same place the agent changed. A prior step's published
        // `worktree` (e.g. another review) wins first.
        let worktree = p
            .get("worktree_path")
            .and_then(Value::as_str)
            .or_else(|| input.get("worktree").and_then(Value::as_str))
            .or_else(|| input.get("worktree_path").and_then(Value::as_str))
            .or_else(|| input.get("working_directory").and_then(Value::as_str))
            .filter(|s| !s.trim().is_empty())
            .map(expand_tilde)
            .unwrap_or_else(|| run_cwd.to_string());
        // The DESIRED base: node params → input → the run's ambient base.
        // `None` ⇒ run_review_for_branch resolves the repo's default
        // branch; a named branch that doesn't exist falls back the same
        // way instead of exiting 128.
        let want_base = p
            .get("base")
            .and_then(Value::as_str)
            .or_else(|| input.get("base").and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| env.run_base.clone());
        targets.push((repo_id, worktree, want_base, String::new()));
    }
    Ok((targets, logs_pre))
}

/// Wait for a launched review to settle — event-driven, re-reading only its
/// status and agent rows — and surface every reviewer session under this
/// step as it appears. `timeout_s: None` is a fire-and-forget review: only
/// the sessions already spawned are surfaced. Returns the review's settled
/// status (`done` / `error` / `cancelled` / `timeout`, or `running` when not
/// awaited); a run cancel stops the wait with an error.
#[allow(clippy::too_many_arguments)]
async fn await_review(
    ctx: &ServerCtx,
    ws: &Workspace,
    run_id: &Id,
    review_id: &Id,
    timeout_s: Option<u64>,
    session_tx: &tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    tag: &str,
    logs: &mut Vec<String>,
) -> Result<String> {
    let mut status = "running".to_string();
    // Surface each reviewer's live session so the run view shows ALL of
    // them running (per-lens reviewers + the checks reviewer), not just
    // an opaque "review started". Ids flow through `session_tx` →
    // `states[idx].sessions`; a set dedups repeated poll harvests. The
    // summarizer publishes its managed session while it is running too.
    let mut seen_sessions: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(timeout_s) = timeout_s {
        let deadline = Instant::now() + Duration::from_secs(timeout_s);
        // Event-driven: subscribe BEFORE the first read, then
        // re-read only the status + agent rows (never the
        // comments) when this review's `ReviewChanged` or a run
        // cancel arrives, or every [`REVIEW_WAIT_SAFETY_RECHECK`].
        // It used to load the full review and decode every
        // comment every 2 s for the review's whole lifetime.
        let mut review_events = ctx.events.subscribe();
        let mut run_cancels = otto_state::workflows::subscribe_cancels();
        let runs = WorkflowsRepo::new(ctx.pool.clone());
        loop {
            if let Ok((review_status, agents)) = ctx.reviews_store.review_agents(review_id).await {
                use otto_core::domain::ReviewStatus as RS;
                for a in &agents {
                    if let Some(sid) = &a.session_id {
                        if seen_sessions.insert(sid.clone()) {
                            let _ = session_tx.send(AgentAssociation::Session(sid.clone()));
                        }
                    }
                }
                match review_status {
                    RS::Done => {
                        status = "done".into();
                        break;
                    }
                    RS::Error => {
                        status = "error".into();
                        break;
                    }
                    RS::Cancelled => {
                        status = "cancelled".into();
                        break;
                    }
                    RS::Running => {}
                }
            }
            if runs.is_canceled(run_id).await {
                // Stop HERE: breaking out went on to the goals
                // agent / the next target's review before the
                // engine's cancel poll dropped this future, and a
                // review started in that gap was never cancelled.
                return Err(otto_core::Error::Internal("run canceled".into()));
            }
            if Instant::now() >= deadline {
                status = "timeout".into();
                logs.push(format!("review_run{tag}: timed out waiting for review"));
                // Don't abandon it running: inside a fix/review loop
                // the next iteration would start a second fleet on the
                // same worktree next to it.
                if let Ok(review) = ctx.reviews_store.get_review(review_id).await {
                    crate::modules::cancel_running_review(ctx, &review, &ws.id).await;
                }
                break;
            }
            let wake_at = tokio::time::Instant::now() + REVIEW_WAIT_SAFETY_RECHECK;
            let wake_at = wake_at.min(tokio::time::Instant::from_std(deadline));
            tokio::select! {
                () = wake_on_event(
                    &mut review_events,
                    |ev| matches!(ev, Event::ReviewChanged { review_id: rid, .. } if rid == review_id),
                    wake_at,
                ) => {}
                // Any run's cancel (rare): the status read above
                // decides whether it was this run's.
                _ = run_cancels.recv() => {}
            }
        }
    } else {
        // Fire-and-forget review: still surface any reviewer sessions that
        // have already spawned so they're attributable under this step.
        if let Ok((_, agents)) = ctx.reviews_store.review_agents(review_id).await {
            for a in &agents {
                if let Some(sid) = &a.session_id {
                    if seen_sessions.insert(sid.clone()) {
                        let _ = session_tx.send(AgentAssociation::Session(sid.clone()));
                    }
                }
            }
        }
    }
    Ok(status)
}
