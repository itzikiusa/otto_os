//! The `check` node — "Check criteria" (goal.md §2.1): command and rule
//! criteria, one independent evaluator turn on a fresh thread for agent
//! criteria, a person's record for human ones (the step parks on the
//! criterion until `run::hold::verify`) and the optional completion review.
//! Every decision is `otto_workflows::{check_run, criteria}`.

use super::super::*;
use super::NodeCall;
use otto_workflows::check_run::{self, CheckParams};
use otto_workflows::criteria::{self, CheckOutput, Verdict};
use otto_workflows::step_util::StepNote;

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
        ..
    } = call;
    let cp = CheckParams::parse(&node.params).map_err(otto_core::Error::Invalid)?;
    let name = node_display_name(node);
    let base =
        crate::workflow_context::step_base_name(scope.step_no, name, scope.iter, scope.inner_idx);
    let (run_id, repo) = (&env.run_id, WorkflowsRepo::new(ctx.pool.clone()));
    let iteration = scope.iter.map(|i| i as u32);
    let top = node.id.split('#').next().unwrap_or(&node.id).to_string();
    let steps = input.get("_steps").cloned().unwrap_or_else(|| json!({}));
    let ictx =
        json!({"iteration": iteration, "steps": steps, "last": input, "input": env.run_input});
    let at = otto_workflows::script_run::ScriptRun {
        files: &env.files,
        run_id: run_id.as_str(),
        run_cwd: &env.run_cwd,
        step_base: &base,
    };
    let host = super::script::Host(ctx);
    // Boxed: a check inside a loop must fit a 2 MiB worker stack.
    let decided = Box::pin(check_run::decide_commands_and_rules(
        &host,
        &at,
        &cp.criteria,
        &ictx,
    ))
    .await
    .map_err(otto_core::Error::Upstream)?;
    let handoff = |ext: &str| env.files.step_md_path(&base).map(|p| p.with_extension(ext));
    // One agent turn on a fresh thread — never the worker's: independence.
    let turn = async |label: &str, prompt: &str, h: Option<std::path::PathBuf>| {
        let mut n = node.clone();
        n.name = label.into();
        n.params = json!({"effort": cp.effort, "timeout_s": 1800});
        let provider = cp.provider.as_deref().unwrap_or(&env.default_provider);
        let (wf, model, cwd) = (&env.wf_name, cp.model.as_deref(), &env.run_cwd);
        let (reply, _) = Box::pin(run_node_agent(
            ctx,
            ws,
            user,
            &n,
            wf,
            run_id,
            provider,
            model,
            prompt,
            cwd,
            h.clone(),
            session_tx,
            log_tx,
            activity_tx,
        ))
        .await?;
        let text = h.and_then(|p| std::fs::read_to_string(p).ok());
        Ok::<_, otto_core::Error>((text, reply))
    };
    let mut logs = vec![];
    let raw = if cp.needs_agent() {
        let h = handoff("output.json");
        let path = h
            .as_ref()
            .map_or(String::new(), |p| p.display().to_string());
        let ins = cp.instructions.as_deref();
        let prompt = criteria::evaluator_prompt(&cp.criteria, &decided, ins, &path);
        let (text, reply) = turn("Check criteria", &prompt, h).await?;
        let ev = check_run::evaluation(text.as_deref(), &reply);
        if ev.is_none() {
            logs.push(format!("⚠ {}", criteria::NO_VERDICT));
        }
        ev
    } else {
        None
    };
    let run = repo.get_run(run_id).await.ok();
    let mut records = run
        .and_then(|r| r.nodes.into_iter().find(|n| n.node_id == top))
        .map(|n| n.human_log)
        .unwrap_or_default();
    let mut d = criteria::decide(raw.as_ref(), &cp.criteria, &decided, &records, iteration);
    let waits_on = |d: &criteria::Decision| {
        let id = d.park_on.as_ref()?;
        cp.criteria.iter().find(|c| &c.id == id).cloned()
    };
    while let Some(c) = waits_on(&d) {
        let (attention, mut rx, _parked) = check_run::park(run_id, &top, &c, iteration);
        let note = |n| session_tx.send(AgentAssociation::Step(n));
        let _ = note(StepNote::NeedsYou(attention));
        let Some(entry) = rx.recv().await else {
            return Err(otto_core::Error::Internal(
                "check: the wait for a person ended".into(),
            ));
        };
        let _ = note(StepNote::Human(entry.clone()));
        let _ = note(StepNote::Resumed {
            answer: None,
            by: None,
        });
        records.push(entry);
        d = criteria::decide(raw.as_ref(), &cp.criteria, &decided, &records, iteration);
    }
    let (mut eval, mut review) = (d.eval, None);
    if cp.final_review && eval.verdict == Verdict::Achieved {
        let h = handoff("review.json");
        let path = h
            .as_ref()
            .map_or(String::new(), |p| p.display().to_string());
        let (text, reply) = turn(
            "Completion review",
            &check_run::review_prompt(&cp, &path),
            h,
        )
        .await?;
        let (passed, r) = criteria::review_passes(&text.unwrap_or(reply));
        criteria::apply_review(&mut eval, passed, &r);
        review = Some(r);
    }
    let prev = match check_run::previous_iteration_id(&node.id) {
        Some(id) => repo
            .checkpoint(run_id, &id)
            .await
            .ok()
            .flatten()
            .and_then(|c| c.output),
        None => None,
    };
    let prev = check_run::previous_progress(prev.as_ref());
    let progress = criteria::record_progress(prev.as_ref().map(|(s, n)| (s.as_str(), *n)), &eval);
    let out = CheckOutput::new(&cp.criteria, &eval, progress, review);
    env.files
        .write_named(&format!("{base}.md"), &out.step_file());
    logs.push(format!("{} of {} criteria met", out.met, out.total));
    Ok((check_run::output(&out), logs))
}
