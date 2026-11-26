//! The `loop` workflow node. Goal pieces (goal.md §2.2): the iteration and
//! active-time budget with the run's raised limits, the repeated-failure
//! stop, a passing `check` ending the loop at once, `hold_when_unmet`
//! (`output._workflow_hold`, settled by `run::hold`), `steps[].same_thread`
//! and inner attempts banking their time and thread (DR-111). Every stop
//! decision is `otto_workflows::criteria::loop_stop`.

use super::super::*;
use super::NodeCall;
use otto_workflows::checkpoint::{caught_thread, continue_previous, observed, AttemptClock};
use otto_workflows::criteria::{loop_stop, CheckOutput, LoopBudget, Stop, StopPoint};

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall {
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
        ..
    } = call;
    let run_id: &Id = &env.run_id;
    let p = &node.params;
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    // Only Resume raises a loop's limits, and only a goal-style loop is held.
    let limits = if p.get("hold_when_unmet").is_some() || p.get("max_active_s").is_some() {
        let run = repo.get_run(run_id).await.ok();
        run.and_then(|r| r.limits)
            .and_then(|l| l.loops.get(&node.id).cloned())
    } else {
        None
    };
    let budget = LoopBudget::new(p, limits.as_ref());
    let max_iter = u64::from(budget.max_iterations);
    let until = p
        .get("until")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let steps = p
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if steps.is_empty() {
        return Err(otto_core::Error::Invalid(
            "loop: requires at least one step".into(),
        ));
    }
    let continue_on_error = p
        .get("continue_on_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Run-level keys (repo_id, base, goals, …) flow to every step as a
    // base; the threaded prev-step output overlays them.
    let loop_base = input.as_object().cloned().unwrap_or_default();
    // Two inner steps with the same (slugged) name would collide on
    // step-file names within an iteration — disambiguate those (and
    // only those) with the inner index.
    let inner_slugs: Vec<String> = steps
        .iter()
        .enumerate()
        .map(|(k, s)| {
            crate::workflow_context::slug(
                s.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&format!("step{k}")),
            )
        })
        .collect();
    let slug_dup = |k: usize| inner_slugs.iter().filter(|s| **s == inner_slugs[k]).count() > 1;
    let mut logs = vec![];
    let mut history = vec![];
    let mut satisfied = false;
    let mut last = input.clone();
    let mut iterations = 0u64;
    let mut last_iteration_error: Option<String> = None;
    // References (repo_id/base/worktree) any inner step published — so a
    // git_pr after the loop knows the exact repo(s)/branch(es) to open,
    // deduped by repo_id (latest wins). Carries multi-repo loops too.
    let mut refs_by_repo: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut stop: Option<Stop> = None;
    let mut failed: Option<(String, String)> = None;
    for i in 1..=max_iter {
        iterations = i;
        if WorkflowsRepo::new(ctx.pool.clone())
            .is_canceled(run_id)
            .await
        {
            logs.push("loop: canceled".into());
            break;
        }
        if progress.enabled() {
            progress.post(format!("🔁 *Iteration {i}/{max_iter}*"));
        }
        // R9: live progress into the run detail (independent of Slack).
        let _ = log_tx.send(format!("🔁 iteration {i}/{max_iter}"));
        // `thread` carries across iterations (so a fix step sees the prior
        // review's findings) and updates after each step within an iteration.
        let mut thread = last.clone();
        let mut step_outputs = serde_json::Map::new();
        last_iteration_error = None;
        for (k, step) in steps.iter().enumerate() {
            let skind = step
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if skind == "loop" {
                return Err(otto_core::Error::Invalid(
                    "loop: nested loops are not allowed".into(),
                ));
            }
            let sname = step
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("step{k}"));
            stop = before_step(&repo, run_id, &node.id, &budget).await;
            if stop.is_some() {
                break;
            }
            let mut merged = loop_base.clone();
            if let Value::Object(m) = &thread {
                for (mk, mv) in m {
                    merged.insert(mk.clone(), mv.clone());
                }
            }
            merged.insert("_iteration".into(), json!(i));
            if skind == "check" {
                merged.insert("_steps".into(), Value::Object(step_outputs.clone()));
            }
            let step_input = Value::Object(merged);
            if progress.enabled() && is_reportable(&skind) {
                progress.post(format!("› ▶ {sname} started"));
            }
            let _ = log_tx.send(format!("▶ {sname} — iteration {i}…"));
            let mut sub = WorkflowNode {
                id: format!("{}#{i}.{k}", node.id),
                kind: skind.clone(),
                name: sname.clone(),
                x: 0.0,
                y: 0.0,
                params: step.get("params").cloned().unwrap_or(Value::Null),
                retry: step
                    .get("retry")
                    .and_then(|r| serde_json::from_value(r.clone()).ok()),
            };
            let same_thread = step.get("same_thread").and_then(Value::as_bool) == Some(true);
            inner_thread(&repo, run_id, &node.id, &mut sub, (i, k), same_thread).await;
            // Inner steps share the loop's step number and add
            // `-iter{X}` (the user's convention): step3-review-iter2.md.
            let sub_scope = StepScope {
                step_no: scope.step_no,
                iter: Some(i),
                inner_idx: if slug_dup(k) { Some(k) } else { None },
            };
            let sub_file_base = crate::workflow_context::step_base_name(
                sub_scope.step_no,
                &sname,
                sub_scope.iter,
                sub_scope.inner_idx,
            );
            let sub_attempt_started = std::time::SystemTime::now();
            let checkpoint = loop_checkpoint(&sub, &node.id, i, k, step_input.clone());
            let checkpoint_repo = WorkflowsRepo::new(ctx.pool.clone());
            let clock = AttemptClock::default();
            match Box::pin(crate::workflow_checkpoint::execute_timed(
                &checkpoint_repo,
                run_id,
                checkpoint,
                &resolve_retry(&sub),
                sub.kind == "loop" || otto_workflows::resume::is_restart_resumable(&sub),
                || {
                    let (sub, step_input, sub_scope) = (&sub, step_input.clone(), &sub_scope);
                    Box::pin(observed(
                        &clock,
                        session_tx,
                        step_note,
                        move |tx| async move {
                            execute_inner_attempt(
                                ctx,
                                ws,
                                user,
                                sub,
                                step_input,
                                env,
                                sub_scope,
                                &tx,
                                log_tx,
                                activity_tx,
                                progress,
                            )
                            .await
                        },
                    ))
                },
                |line| {
                    let _ = log_tx.send(line);
                },
                &clock,
            ))
            .await
            {
                Ok(outcome) => {
                    let out = outcome.output;
                    let mut slogs = outcome.logs;
                    // A completed checkpoint owns the earlier handoff.
                    // Restart time is not a new winning attempt timestamp.
                    let flogs =
                        if outcome.adopted && env.files.has_file(&format!("{sub_file_base}.md")) {
                            if outcome_error(&out).is_none() {
                                env.files.adopt_step(&sub_file_base, &skind);
                            }
                            vec![]
                        } else {
                            env.files.persist_step(
                                &sub_file_base,
                                &skind,
                                &sname,
                                &out,
                                &slogs,
                                outcome_error(&out),
                                Some(sub_attempt_started),
                            )
                        };
                    for l in slogs.drain(..).chain(flogs) {
                        let line = format!("  [{i}/{sname}] {l}");
                        let _ = log_tx.send(line.clone());
                        logs.push(line);
                    }
                    step_outputs.insert(sname.clone(), out.clone());
                    // Capture any repo reference this step published —
                    // for the loop's own output AND the run registry.
                    merge_published_refs(&env.files, &out);
                    if let Some(rid) = out.get("repo_id").and_then(Value::as_str) {
                        if !rid.trim().is_empty() {
                            refs_by_repo.insert(
                                rid.to_string(),
                                json!({
                                    "repo_id": rid,
                                    "base": out.get("base").cloned().unwrap_or(Value::Null),
                                    "worktree": out.get("worktree").cloned().unwrap_or(Value::Null),
                                }),
                            );
                        }
                    }
                    if let Some(error) = outcome_error(&out) {
                        failed = Some((sname.clone(), error.to_string()));
                        last_iteration_error = Some(format!(
                            "loop step '{sname}' failed at iteration {i}: {error}"
                        ));
                        last = out;
                        break;
                    }
                    if skind == "check" {
                        if let Ok(check) = serde_json::from_value::<CheckOutput>(out.clone()) {
                            let at = StopPoint::AfterCheck {
                                iteration: i as u32,
                                check: &check,
                            };
                            stop = loop_stop(&budget, at);
                        }
                    }
                    last = out.clone();
                    thread = out;
                    // R9: live "done" milestone into the run detail.
                    let _ = log_tx.send(match brief_summary(&last) {
                        Some(s) => format!("✅ {sname} done — {}", truncate(&s, 160)),
                        None => format!("✅ {sname} done"),
                    });
                    if progress.enabled() && is_reportable(&skind) {
                        match brief_summary(&last) {
                            Some(s) => progress.post(format!("› ✅ {sname} done\n{s}")),
                            None => progress.post(format!("› ✅ {sname} done")),
                        }
                    }
                    // Attach the iteration's handoff file too.
                    if progress.enabled() && (is_reportable(&skind) || skind == "review_run") {
                        progress.post_step_file(&env.files, &sub_file_base);
                    }
                    if stop.is_some() {
                        break;
                    }
                }
                Err(e) => {
                    if checkpoint_repo.is_canceled(run_id).await {
                        return Err(e);
                    }
                    let fline = format!("  [{i}/{sname}] ✗ {e}");
                    let _ = log_tx.send(fline.clone());
                    logs.push(fline);
                    // A failed iteration leaves its trace file BEFORE the
                    // loop bails — the fix step / a human reads what broke
                    // (the production incident left no trail at all).
                    let _ = env.files.persist_step(
                        &sub_file_base,
                        &skind,
                        &sname,
                        &Value::Null,
                        &[format!("✗ {e}")],
                        Some(&e.to_string()),
                        Some(sub_attempt_started),
                    );
                    if progress.enabled() && is_reportable(&skind) {
                        progress.post(format!(
                            "› ❌ {sname} failed — {}",
                            truncate(&e.to_string(), 200)
                        ));
                    }
                    // The failed iteration's trace file rides along.
                    if progress.enabled() && (is_reportable(&skind) || skind == "review_run") {
                        progress.post_step_file(&env.files, &sub_file_base);
                    }
                    failed = Some((sname.clone(), e.to_string()));
                    last_iteration_error =
                        Some(format!("loop step '{sname}' failed at iteration {i}: {e}"));
                    break;
                }
            }
        }
        let ictx = json!({ "iteration": i, "last": last, "steps": Value::Object(step_outputs), "input": input, "error": last_iteration_error });
        history.push(ictx.clone());
        if let (Some((step, error)), false) = (&failed, continue_on_error) {
            let at = StopPoint::StepFailed {
                iteration: i as u32,
                step,
                error,
            };
            stop = stop.or_else(|| loop_stop(&budget, at));
        }
        if stop.is_some() {
            break;
        }
        if last_iteration_error.is_some() {
            if continue_on_error {
                continue;
            }
            break;
        }
        if !until.is_empty() && otto_core::expr::eval_bool(&until, &ictx) {
            satisfied = true;
            let line = format!("loop: `{until}` satisfied at iteration {i}");
            let _ = log_tx.send(line.clone());
            logs.push(line);
            break;
        }
    }
    if stop.is_none() && !satisfied && last_iteration_error.is_none() && iterations == max_iter {
        stop = loop_stop(
            &budget,
            StopPoint::BeforeIteration {
                used: max_iter as u32,
            },
        )
        .filter(|s| matches!(s, Stop::Hold { .. }));
    }
    let (met, line, hold) = otto_workflows::criteria::stop_outcome(stop, iterations);
    satisfied |= met;
    logs.extend(line);
    if !until.is_empty() && !satisfied && hold.is_none() {
        logs.push(format!(
            "loop: reached max_iterations ({max_iter}) without satisfying `{until}`"
        ));
    }
    // Surface the loop's repo reference(s) at the top level so a
    // downstream git_pr inherits them: a single `repo_id`/`base`/`worktree`
    // (the common one-repo loop) plus a `repos` array for the multi-repo
    // case (one PR per changed repo).
    let repos: Vec<Value> = refs_by_repo.values().cloned().collect();
    let mut out = serde_json::Map::new();
    out.insert("iterations".into(), json!(iterations));
    if budget.hold_when_unmet || budget.max_active_s.is_some() {
        let active_ms = otto_workflows::hold::loop_active_ms(&repo, run_id, &node.id).await;
        out.insert("active_ms".into(), json!(active_ms));
        out.insert("max_iterations".into(), json!(max_iter));
        out.insert("max_active_s".into(), json!(budget.max_active_s));
    }
    out.insert("satisfied".into(), json!(satisfied));
    out.insert("last".into(), last);
    out.insert("history".into(), json!(history));
    if let Some(error) = last_iteration_error {
        out.insert("_workflow_error".into(), json!(error));
    }
    // A hold also reads as an error, so the scheduler runs nothing after it.
    if let Some(h) = hold {
        out.insert("_workflow_error".into(), h["reason"].clone());
        out.insert("_workflow_hold".into(), h);
    }
    if let Some((_, Value::Object(m))) = refs_by_repo.iter().next() {
        for k in ["repo_id", "base", "worktree"] {
            if let Some(v) = m.get(k) {
                out.insert(k.to_string(), v.clone());
            }
        }
    }
    if !repos.is_empty() {
        out.insert("repos".into(), Value::Array(repos));
    }
    Ok((Value::Object(out), logs))
}

/// The time budget before an inner step starts (only for a budgeted loop).
async fn before_step(
    repo: &WorkflowsRepo,
    run_id: &Id,
    loop_id: &str,
    b: &LoopBudget,
) -> Option<Stop> {
    b.max_active_s?;
    let active_ms = otto_workflows::hold::loop_active_ms(repo, run_id, loop_id).await;
    loop_stop(b, StopPoint::BeforeStep { active_ms })
}

/// A restart caught this attempt on a thread → resume it (DR-111);
/// `same_thread` → continue the previous iteration's thread of the step.
async fn inner_thread(
    repo: &WorkflowsRepo,
    run_id: &Id,
    loop_id: &str,
    sub: &mut WorkflowNode,
    (i, k): (u64, usize),
    same_thread: bool,
) {
    let cp = repo.checkpoint(run_id, &sub.id).await.ok().flatten();
    if let Some(t) = cp.as_ref().and_then(caught_thread) {
        super::super::step_thread::mark_resume(run_id, &sub.id, &t);
    }
    if same_thread && i > 1 {
        let prev = repo
            .checkpoint(run_id, &format!("{loop_id}#{}.{k}", i - 1))
            .await;
        continue_previous(&mut sub.params, prev.ok().flatten().as_ref());
    }
}

fn step_note(a: &AgentAssociation) -> Option<&otto_workflows::step_util::StepNote> {
    match a {
        AgentAssociation::Step(n) => Some(n),
        _ => None,
    }
}
