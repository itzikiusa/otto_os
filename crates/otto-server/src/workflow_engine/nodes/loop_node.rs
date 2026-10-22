//! The `loop` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

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
    let max_iter = p
        .get("max_iterations")
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .clamp(1, 10);
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
            let mut merged = loop_base.clone();
            if let Value::Object(m) = &thread {
                for (mk, mv) in m {
                    merged.insert(mk.clone(), mv.clone());
                }
            }
            merged.insert("_iteration".into(), json!(i));
            let step_input = Value::Object(merged);
            if progress.enabled() && is_reportable(&skind) {
                progress.post(format!("› ▶ {sname} started"));
            }
            let _ = log_tx.send(format!("▶ {sname} — iteration {i}…"));
            let sub = WorkflowNode {
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
            match crate::workflow_checkpoint::execute_observed(
                &checkpoint_repo,
                run_id,
                checkpoint,
                &resolve_retry(&sub),
                sub.kind == "loop" || is_restart_resumable_kind(&sub.kind),
                || {
                    Box::pin(execute_inner_attempt(
                        ctx,
                        ws,
                        user,
                        &sub,
                        step_input.clone(),
                        env,
                        &sub_scope,
                        session_tx,
                        log_tx,
                        activity_tx,
                        progress,
                    ))
                },
                |line| {
                    let _ = log_tx.send(line);
                },
            )
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
                        last_iteration_error = Some(format!(
                            "loop step '{sname}' failed at iteration {i}: {error}"
                        ));
                        last = out;
                        break;
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
                }
                Err(e) => {
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
                    last_iteration_error =
                        Some(format!("loop step '{sname}' failed at iteration {i}: {e}"));
                    break;
                }
            }
        }
        let ictx = json!({ "iteration": i, "last": last, "steps": Value::Object(step_outputs), "input": input, "error": last_iteration_error });
        history.push(ictx.clone());
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
    if !until.is_empty() && !satisfied {
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
    out.insert("satisfied".into(), json!(satisfied));
    out.insert("last".into(), last);
    out.insert("history".into(), json!(history));
    if let Some(error) = last_iteration_error {
        out.insert("_workflow_error".into(), json!(error));
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
