//! The node loop of a run, one node per [`step`]: scope and edge decisions,
//! then (for a node that runs) the attempt loop with retries, live session /
//! log / activity harvesting and cancel / skip handling, then the outcome
//! (step files, edge pruning, session stop, chat progress). Moved verbatim
//! out of `run_workflow_with_budget`; the driver loops over `step` until it
//! returns [`NodeFlow::Stop`] or the order runs out.

use super::super::*;
use super::state::{NodeFlow, RunState};

/// What every node of one run shares: who and where it runs as, the run's
/// input and context files, and the scope it was started with. Immutable for
/// the whole node loop (the mutable side is [`RunState`]).
#[derive(Clone, Copy)]
pub(in crate::workflow_engine) struct RunCx<'a> {
    pub(in crate::workflow_engine) ctx: &'a ServerCtx,
    pub(in crate::workflow_engine) ws: &'a Workspace,
    pub(in crate::workflow_engine) workflow: &'a Workflow,
    pub(in crate::workflow_engine) run_id: &'a Id,
    pub(in crate::workflow_engine) repo: &'a WorkflowsRepo,
    pub(in crate::workflow_engine) user: &'a User,
    pub(in crate::workflow_engine) env: &'a RunEnv,
    pub(in crate::workflow_engine) input: &'a Value,
    pub(in crate::workflow_engine) files:
        &'a std::sync::Arc<crate::workflow_context::RunContextFiles>,
    pub(in crate::workflow_engine) progress: &'a ProgressSink,
    pub(in crate::workflow_engine) run_set: Option<&'a HashSet<String>>,
    pub(in crate::workflow_engine) prior_nodes: Option<&'a [NodeRunState]>,
    pub(in crate::workflow_engine) continue_unfinished: bool,
}

/// Persist the run's node states and push them to the run view, naming
/// `states[idx]` as the node that changed (the run is still "running").
async fn publish_node(cx: RunCx<'_>, states: &[NodeRunState], idx: usize) {
    let rev = cx
        .repo
        .update_run_progress(cx.run_id, states)
        .await
        .unwrap_or(0);
    emit_run_updated(
        cx.ctx,
        &cx.workflow.workspace_id,
        cx.run_id,
        "running",
        Some(&states[idx].node_id),
        rev,
        Some(&states[idx]),
        states,
        false,
    );
}

/// Mark `states[idx]` Skipped with one log line and publish it.
async fn skip_with(cx: RunCx<'_>, states: &mut [NodeRunState], idx: usize, why: &str) {
    states[idx].status = NodeStatus::Skipped;
    states[idx].logs = vec![why.into()];
    publish_node(cx, states, idx).await;
}

/// One node of the loop: cancel / budget checks, adoption, run scope and the
/// incoming-edge decision; a node that runs goes on to [`execute_step`].
pub(in crate::workflow_engine) async fn step(
    cx: RunCx<'_>,
    rs: &mut RunState,
    node_id: String,
) -> NodeFlow {
    let RunCx {
        workflow,
        run_id,
        repo,
        input,
        files,
        run_set,
        prior_nodes,
        continue_unfinished,
        env,
        ..
    } = cx;
    // Canceled before it started (the Pending→Running CAS lost).
    if rs.canceled {
        return NodeFlow::Stop;
    }
    // Honor a cancel request (the API flips the run status to Canceled).
    if repo.is_canceled(run_id).await {
        rs.canceled = true;
        return NodeFlow::Stop;
    }

    // Stop once the run has exceeded its global time budget.
    if env.budget.expired() {
        rs.timed_out = true;
        if let Some(state) = rs.states.iter_mut().find(|state| state.node_id == node_id) {
            state.status = NodeStatus::Error;
            state.error = Some("workflow active time budget exhausted".into());
        }
        return NodeFlow::Stop;
    }

    let Some(node) = workflow.graph.nodes.iter().find(|n| n.id == node_id) else {
        return NodeFlow::Next;
    };
    let idx = rs.states.iter().position(|s| s.node_id == node_id).unwrap();

    let adopted = rs.states[idx].status != NodeStatus::Pending
        && (continue_unfinished || run_set.is_some_and(|set| !set.contains(&node_id)));
    if adopted {
        if matches!(
            rs.states[idx].status,
            NodeStatus::Success | NodeStatus::Error
        ) {
            rs.step_counter += 1;
            if rs.states[idx].status == NodeStatus::Success {
                let base = crate::workflow_context::step_base_name(
                    rs.step_counter,
                    node_display_name(node),
                    None,
                    None,
                );
                files.adopt_step(&base, &node.kind);
            }
        }
        return NodeFlow::Next;
    }

    // Outside the run scope (start-from-here) → skip without running. A
    // retry re-entry already adopted the node's prior state — keep it.
    if run_set.is_some_and(|set| !set.contains(&node_id)) {
        if rs.states[idx].status != NodeStatus::Pending {
            return NodeFlow::Next;
        }
        skip_with(cx, &mut rs.states, idx, "outside run scope").await;
        return NodeFlow::Next;
    }

    // Decide whether to run this node based on its incoming edges. Only
    // edges whose source is within the run scope constrain control flow (a
    // start-from-here run leaves ancestors out of scope; their edges don't
    // poison or branch-skip the entry node — it falls back to the run input).
    let views: Vec<EdgeView> = incoming_edges(&workflow.graph, &node_id)
        .iter()
        .filter(|e| dependency_is_relevant(&e.source, run_set, prior_nodes))
        .map(|e| EdgeView {
            source: e.source.clone(),
            errored: rs.errored.contains(&e.source),
            has_output: rs.outputs.contains_key(&e.source),
            edge_active: !rs.inactive_edges.contains(&e.id),
        })
        .collect();
    let node_input = match decide_node(&views) {
        NodeDecision::ErrorSkip => {
            rs.errored.insert(node_id.clone());
            skip_with(cx, &mut rs.states, idx, SKIPPED_UPSTREAM).await;
            return NodeFlow::Next;
        }
        NodeDecision::BranchSkip => {
            skip_with(cx, &mut rs.states, idx, "skipped (branch not taken)").await;
            return NodeFlow::Next;
        }
        NodeDecision::Run(satisfied) => assemble_input(&satisfied, &rs.outputs, input),
    };

    // No cross-run node-result cache: every node executes in its own run
    // (the cache and its `cache_exempt_kind` were removed in 344315ea7).
    execute_step(cx, rs, node, idx, node_input).await
}

/// The live channels of one executing node: agent sessions it spawns, its
/// streamed log lines (R9) and its sub-agent / phase snapshot (R1.4). One set
/// per node, so lines never leak into the next node.
struct NodeChannels {
    sess_tx: tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    sess_rx: tokio::sync::mpsc::UnboundedReceiver<AgentAssociation>,
    log_tx: tokio::sync::mpsc::UnboundedSender<String>,
    log_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    activity_tx: tokio::sync::mpsc::UnboundedSender<NodeActivity>,
    activity_rx: tokio::sync::mpsc::UnboundedReceiver<NodeActivity>,
}

impl NodeChannels {
    fn new() -> Self {
        let (sess_tx, sess_rx) = tokio::sync::mpsc::unbounded_channel::<AgentAssociation>();
        let (log_tx, log_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (activity_tx, activity_rx) = tokio::sync::mpsc::unbounded_channel::<NodeActivity>();
        Self {
            sess_tx,
            sess_rx,
            log_tx,
            log_rx,
            activity_tx,
            activity_rx,
        }
    }
}

/// How a node's attempt loop ended.
struct Attempts {
    result: Result<(Value, Vec<String>)>,
    /// The attempt number that produced `result`.
    attempt: u32,
    /// When that attempt began: a step file written during a FAILED earlier
    /// attempt must not be mistaken for the winning attempt's handoff.
    attempt_started: std::time::SystemTime,
    /// A chat `skip` aborted this node; the run continues.
    skip_current: bool,
    /// Retry notes (and stopped-session lines) to append to the node's logs.
    retry_logs: Vec<String>,
}

/// Execute one in-scope node: mark it running, run its attempts, then settle
/// it (skip, stop or outcome) and publish the final state.
async fn execute_step(
    cx: RunCx<'_>,
    rs: &mut RunState,
    node: &WorkflowNode,
    idx: usize,
    node_input: Value,
) -> NodeFlow {
    let start_line = format!("▶ {} started", node.kind);
    rs.states[idx].status = NodeStatus::Running;
    rs.states[idx].started_at = Some(chrono::Utc::now());
    rs.states[idx].logs = vec![start_line];
    // Signal node start so the UI can show live progress immediately.
    publish_node(cx, &rs.states, idx).await;
    if cx.progress.enabled() && is_reportable(&node.kind) {
        cx.progress
            .post(format!("▶ *{}* started", node_label(node)));
    }

    let started = Instant::now();
    // This node executes — it owns the next step-file number.
    rs.step_counter += 1;
    let scope = StepScope {
        step_no: rs.step_counter,
        iter: None,
        inner_idx: None,
    };
    let step_file_base = crate::workflow_context::step_base_name(
        rs.step_counter,
        node_display_name(node),
        None,
        None,
    );
    let mut ch = NodeChannels::new();
    let done = run_attempts(cx, rs, node, idx, &node_input, &scope, &mut ch).await;
    // Drain any session ids reported right as the node finished.
    while let Ok(sid) = ch.sess_rx.try_recv() {
        record_association(&mut rs.states[idx], sid);
    }
    // Drain any trailing live-log lines — kept on EVERY path now (R5.5): the
    // phase lines ARE the record of how the step completed.
    while let Ok(line) = ch.log_rx.try_recv() {
        rs.states[idx].logs.push(line);
    }
    // Drain trailing activity snapshots (last one wins), then clear it:
    // `activity` is only meaningful while the node RUNS.
    while ch.activity_rx.try_recv().is_ok() {}
    rs.states[idx].activity = None;
    // A marker nobody consumed (the step finished first) dies with the step.
    take_skip_marker(cx.ctx, cx.run_id, &node.id);
    if done.skip_current {
        skip_node(cx, rs, node, idx, started).await;
        return NodeFlow::Next;
    }
    // Canceled mid-node: bail straight to the finalize block, which marks the
    // remaining nodes Skipped, flips the run to Canceled, and kills every
    // session this run spawned (incl. this node's, harvested just above).
    if rs.canceled || rs.timed_out {
        return NodeFlow::Stop;
    }
    settle(
        cx,
        rs,
        node,
        idx,
        &node_input,
        started,
        &step_file_base,
        done,
    )
    .await;
    // Signal node finish so the inspector can update without waiting for the next poll.
    publish_node(cx, &rs.states, idx).await;
    NodeFlow::Next
}

/// Run the node, honoring its retry policy (default: a single attempt). A
/// node that spawns an openable agent session reports its id over `sess_tx`
/// the moment it's created; we record it on the node state and persist+emit
/// immediately so the run view can open it *while running*.
#[allow(clippy::too_many_arguments)]
async fn run_attempts(
    cx: RunCx<'_>,
    rs: &mut RunState,
    node: &WorkflowNode,
    idx: usize,
    node_input: &Value,
    scope: &StepScope,
    ch: &mut NodeChannels,
) -> Attempts {
    let RunCx {
        ctx,
        ws,
        run_id,
        repo,
        user,
        env,
        progress,
        ..
    } = cx;
    let RunState {
        states,
        canceled,
        timed_out,
        ..
    } = rs;
    // The node future reads the senders; the monitor below drains the receivers.
    let (sess_tx, log_tx, activity_tx) = (&ch.sess_tx, &ch.log_tx, &ch.activity_tx);
    let (sess_rx, log_rx, activity_rx) = (&mut ch.sess_rx, &mut ch.log_rx, &mut ch.activity_rx);
    let node_id = node.id.as_str();
    let policy = resolve_retry(node);
    let mut attempt: u32 = 0;
    let mut attempt_limit = policy.max_attempts + 1;
    let mut backoff = policy.backoff_ms;
    let mut retry_logs: Vec<String> = vec![];
    // Live sub-agent / phase snapshot of a RUNNING agent step (R1.4): rate-
    // limited to one persist per 5s — it changes on every count change and
    // `update_run_progress` rewrites `nodes_json`.
    let mut last_activity_persist: Option<Instant> = None;
    // Live-log persist coalescing (see LOG_PERSIST_EVERY).
    let mut log_persist = LogPersist::default();
    // Snapshot when the (latest) attempt began: a step file the agent wrote
    // during a FAILED earlier attempt must not be mistaken for the winning
    // attempt's handoff (persist_step compares mtimes against this).
    // Assigned at the top of every attempt; the loop runs at least once.
    let mut attempt_started;
    // Catch a cancel WHILE this node runs — the node-boundary check only
    // fires between nodes, so without this a long agent turn would ignore a
    // cancel for up to TURN_TIMEOUT. Perf W1: a cancel is ANNOUNCED
    // (`request_cancel` → `subscribe_cancels`) and wakes the select at
    // once; the 2 s tick only consumes the in-memory chat `skip` marker,
    // and reads the run's status (status-only, never the 50–200 KB body)
    // on its first tick and then every CANCEL_SAFETY_TICKS as a backstop.
    let mut cancel_watch = CancelWatch::new(run_id, CANCEL_POLL);
    // A chat `skip` command marks this run in `wf_skip_current`; unlike a
    // cancel it aborts only the CURRENT node, which is then marked Skipped and
    // the run continues to the next node.
    let mut skip_current = false;
    let result = loop {
        attempt += 1;
        states[idx].attempts = Some(attempt);
        states[idx].logs.push(format!(
            "{} — attempt {}/{}",
            node_display_name(node),
            attempt,
            attempt_limit
        ));
        publish_node(cx, states, idx).await;
        attempt_started = std::time::SystemTime::now();
        // Heap-pin the large node future before the driver's nested select frames.
        let fut = Box::pin(async {
            if node.kind == "loop" {
                let checkpoint = loop_checkpoint(node, &node.id, 0, 0, node_input.clone());
                crate::workflow_checkpoint::execute(repo, run_id, checkpoint, &policy, true, || {
                    execute_node(
                        ctx,
                        ws,
                        user,
                        node,
                        node_input.clone(),
                        env,
                        scope,
                        sess_tx,
                        log_tx,
                        activity_tx,
                        progress,
                    )
                })
                .await
            } else {
                execute_node(
                    ctx,
                    ws,
                    user,
                    node,
                    node_input.clone(),
                    env,
                    scope,
                    sess_tx,
                    log_tx,
                    activity_tx,
                    progress,
                )
                .await
            }
        });
        let attempt_res = crate::workflow_node_driver::drive(fut, async |mut done| {
            loop {
            tokio::select! {
                biased;
                () = env.budget.exhausted() => {
                    *timed_out = true;
                    break Err(otto_core::Error::Upstream("workflow active time budget exhausted".into()));
                }
                Some(sid) = sess_rx.recv() => {
                    if record_association(&mut states[idx], sid) {
                        publish_node(cx, states, idx).await;
                        // The write carried any buffered log lines too.
                        log_persist.carried();
                    }
                }
                Some(line) = log_rx.recv() => {
                    // Live progress line (R9) — append to the node's logs. The
                    // first line after a quiet spell is written (and pushed to
                    // the run detail) at once; a burst is coalesced into one
                    // write per LOG_PERSIST_EVERY / LOG_PERSIST_LINES.
                    states[idx].logs.push(line);
                    if states[idx].logs.len() > NODE_LIVE_LOG_CAP {
                        cap_node_logs(&mut states[idx].logs, NODE_LIVE_LOG_CAP);
                    }
                    if log_persist.on_line(Instant::now()) {
                        publish_node(cx, states, idx).await;
                    }
                }
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(
                    log_persist.flush_at.unwrap_or_else(Instant::now),
                )), if log_persist.flush_at.is_some() => {
                    // Trailing flush of a coalesced log burst.
                    log_persist.wrote_log(Instant::now());
                    publish_node(cx, states, idx).await;
                }
                Some(a) = activity_rx.recv() => {
                    // R1.4: the live sub-agent/phase snapshot. Always kept on
                    // the state (the next emit carries it), persisted at most
                    // every 5s so a chatty sweep can't hammer the run row.
                    states[idx].activity = Some(bound_activity(a));
                    if last_activity_persist.is_none_or(|t| t.elapsed() >= Duration::from_secs(5)) {
                        last_activity_persist = Some(Instant::now());
                        publish_node(cx, states, idx).await;
                        log_persist.carried();
                    }
                }
                wake = cancel_watch.next(repo, || take_skip_marker(ctx, run_id, node_id)) => {
                    // Cancel drops the scoped node driver; finalization kills
                    // this run's sessions. No node task remains detached.
                    let parked_at_approval = match wake {
                        CancelWake::Canceled => {
                            *canceled = true;
                            break Err(otto_core::Error::Internal("run canceled".into()));
                        }
                        CancelWake::Skip { parked_at_approval } => parked_at_approval,
                    };
                    // A chat `skip` command (consume-once) for THIS step → abort
                    // it and skip it; the run continues to the next node. The
                    // marker is keyed by (run, step), so one set for an earlier
                    // step never fires on this one before it did any work. An
                    // approval gate is never skippable — that would pass it
                    // unapproved (the chat user needn't hold approve rights).
                    // That includes a LOOP parked at an approval inside it:
                    // skipping the loop dropped the gate and let the steps
                    // after it (a PR, say) run with nobody's sign-off.
                    if node.kind == "human_approval" || parked_at_approval {
                        states[idx].logs.push(
                            "⚠ skip ignored — an approval step must be approved or rejected".into(),
                        );
                    } else {
                        skip_current = true;
                        break Err(otto_core::Error::Internal("step skipped".into()));
                    }
                }
                r = &mut done => break r.unwrap_or_else(|_| Err(otto_core::Error::Internal("workflow node driver closed without a result".into()))),
            }
            }
        }).await;
        match attempt_res {
            Ok(ok) => break Ok(ok),
            Err(e) => {
                // Canceled or skipped mid-node → stop immediately, never retry.
                if *canceled || skip_current || *timed_out {
                    break Err(e);
                }
                // R5.2: a `529 Overloaded` / fd-exhaustion failure needs a
                // real pause (and a bigger budget) — 2s→4s just burns the
                // attempts against a provider that is still overloaded.
                let jitter = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() as u64 % 5_000)
                    .unwrap_or(0);
                let plan = if is_retryable(&node.kind) {
                    retry_backoff(&e.to_string(), &policy, attempt, backoff, jitter)
                } else {
                    None
                };
                let Some((sleep_ms, max_eff, reason)) = plan else {
                    break Err(e);
                };
                attempt_limit = max_eff + 1;
                states[idx].logs.push(format!(
                    "↻ retry {}/{} in {}s ({reason})",
                    attempt + 1,
                    max_eff + 1,
                    sleep_ms / 1000
                ));
                publish_node(cx, states, idx).await;
                // The failed attempt's agent must not stay alive next to the
                // retry's fresh one (5 live PTYs per step in a 529 storm; and
                // fd exhaustion is itself a retry class). Suspend/kill it the
                // way a successful step's session is stopped.
                if stop_step_sessions_wanted(&node.kind, &node.params) {
                    while let Ok(sid) = sess_rx.try_recv() {
                        record_association(&mut states[idx], sid);
                    }
                    for sid in states[idx].sessions.clone() {
                        stop_step_session(ctx, &sid, run_id, &mut retry_logs).await;
                    }
                }
                // Bail out of the backoff promptly if the run is canceled
                // — before AND during the sleep (up to ~65s for an
                // overloaded provider). It used to check once and then
                // sleep blind, and to settle the step as a FAILED step: a
                // "❌ failed" chat post and trace for a run the user
                // stopped, and a retry clicked meanwhile raced a second
                // attempt of this driver.
                tokio::select! {
                    biased;
                    () = env.budget.exhausted() => {
                        *timed_out = true;
                        break Err(otto_core::Error::Upstream("workflow active time budget exhausted".into()));
                    }
                    stopped = backoff_canceled(repo, run_id, Duration::from_millis(sleep_ms)) => {
                        if stopped {
                            *canceled = true;
                            break Err(otto_core::Error::Internal("run canceled".into()));
                        }
                    }
                }
                backoff = ((sleep_ms as f64) * policy.factor) as u64;
                backoff = backoff.clamp(1, 60_000);
            }
        }
    };
    Attempts {
        result,
        attempt,
        attempt_started,
        skip_current,
        retry_logs,
    }
}

/// Skipped mid-node via a chat `skip` command: stop THIS node's agents, mark
/// it Skipped, and continue to the NEXT node (unlike cancel, the run
/// proceeds). A marker output keeps dependents satisfied — an empty output
/// would make them BranchSkip and cascade the whole tail away.
async fn skip_node(
    cx: RunCx<'_>,
    rs: &mut RunState,
    node: &WorkflowNode,
    idx: usize,
    started: Instant,
) {
    let RunCx { ctx, workflow, .. } = cx;
    let states = &mut rs.states;
    // A skipped review step's fleet is cancelled through the review
    // engine, like a run cancel does: killing a reviewer's PTY alone
    // makes its recovery loop respawn it, ownerless.
    for review_id in states[idx].review_ids.clone() {
        if let Ok(review) = ctx.reviews_store.get_review(&review_id).await {
            crate::modules::cancel_running_review(ctx, &review, &workflow.workspace_id).await;
        }
    }
    for sid in &states[idx].sessions {
        if let Err(e) = ctx.manager.kill_session(sid).await {
            tracing::warn!("skip: failed to kill workflow session {sid}: {e}");
        }
    }
    states[idx].status = NodeStatus::Skipped;
    states[idx].logs.push("⏭ skipped via chat command".into());
    // A skipped step decided nothing, so none of its CONDITIONAL
    // out-edges is taken (those branches BranchSkip). Nothing used to be
    // pruned here — both sides of a branch after a skipped condition /
    // review ran. Unconditional edges stay live; the marker output below
    // satisfies those dependents.
    for e in outgoing_edges(&workflow.graph, &node.id) {
        if e.condition.is_some() {
            rs.inactive_edges.insert(e.id.clone());
            states[idx]
                .logs
                .push(format!("edge → {} not taken (step skipped)", e.target));
        }
    }
    cap_node_logs(&mut states[idx].logs, NODE_LOG_CAP);
    states[idx].duration_ms = Some(started.elapsed().as_millis() as u64);
    rs.outputs.insert(
        node.id.clone(),
        json!({ "skipped": true, "note": "skipped via chat command" }),
    );
    publish_node(cx, &rs.states, idx).await;
}

/// Settle a node whose attempts finished (not skipped, canceled or timed
/// out): record its output or error, write its step file, stop its agent,
/// prune its out-edges and report it to the chat thread.
#[allow(clippy::too_many_arguments)]
async fn settle(
    cx: RunCx<'_>,
    rs: &mut RunState,
    node: &WorkflowNode,
    idx: usize,
    node_input: &Value,
    started: Instant,
    step_file_base: &str,
    done: Attempts,
) {
    let RunCx {
        ctx,
        workflow,
        run_id,
        input,
        files,
        progress,
        ..
    } = cx;
    let Attempts {
        result,
        attempt,
        attempt_started,
        mut retry_logs,
        ..
    } = done;
    let node_id = &node.id;
    let states = &mut rs.states;
    match result {
        Ok((out, mut ret_logs)) => {
            let failure = outcome_error(&out).map(str::to_owned);
            states[idx].status = if failure.is_some() {
                NodeStatus::Error
            } else {
                NodeStatus::Success
            };
            states[idx].error = failure.clone();
            if failure.is_some() {
                rs.errored.insert(node_id.clone());
            }
            states[idx].output = Some(out.clone());
            // R5.5: the LIVE vector (which already starts with `start_line`
            // and carries every streamed phase line) is the base — the
            // returned lines are appended, never substituted for it.
            let mut logs = std::mem::take(&mut states[idx].logs);
            logs.append(&mut ret_logs);
            logs.append(&mut retry_logs);
            // Warn-only output validation against the node's declared schema.
            for w in validate_node_output(&node.kind, &out) {
                logs.push(format!("⚠ {w}"));
            }
            // Step handoff files: raw output + the curated summary (kept
            // if the agent wrote its own during THIS attempt), then merge
            // any published repo/branch/worktree refs into repos.json.
            let mut flogs = files.persist_step(
                step_file_base,
                &node.kind,
                node_display_name(node),
                &out,
                &logs,
                failure.as_deref(),
                Some(attempt_started),
            );
            logs.append(&mut flogs);
            merge_published_refs(files, &out);
            // R5.1: the step succeeded — stop the agent session it spawned.
            // With sub-agents allowed this leaks whole TREES of agents
            // otherwise. `review_run`'s sessions belong to the review, and
            // `keep_session: true` opts a step out.
            if stop_step_sessions_wanted(&node.kind, &node.params) {
                for sid in states[idx].sessions.clone() {
                    stop_step_session(ctx, &sid, run_id, &mut logs).await;
                }
            }
            // Prune outgoing edges whose condition fails on this output.
            let (pruned, mut plogs) = eval_outgoing(&workflow.graph, node, &out, node_input, input);
            rs.inactive_edges.extend(pruned);
            logs.append(&mut plogs);
            cap_node_logs(&mut logs, NODE_LOG_CAP);
            states[idx].logs = logs;
            states[idx].attempts = Some(attempt);
            // Also harvest a session id carried in the output (dedups with the
            // live channel report).
            harvest_session_ids(&out, &mut states[idx].sessions);
            let elapsed = started.elapsed().as_millis() as u64;
            states[idx].duration_ms = Some(elapsed);
            if progress.enabled() && is_reportable(&node.kind) && failure.is_none() {
                let dur = format!("{:.1}s", elapsed as f64 / 1000.0);
                match brief_summary(&out) {
                    Some(s) => {
                        progress.post(format!("✅ *{}* done ({dur})\n{s}", node_label(node)))
                    }
                    None => progress.post(format!("✅ *{}* done ({dur})", node_label(node))),
                }
            }
            // Attach the step's handoff file beneath its progress line —
            // the thread carries the brief, the file the full detail.
            // review_run streams its own block but its file rides too.
            if progress.enabled() && (is_reportable(&node.kind) || node.kind == "review_run") {
                progress.post_step_file(files, step_file_base);
            }
            rs.outputs.insert(node_id.clone(), out);
        }
        Err(e) => {
            states[idx].status = NodeStatus::Error;
            states[idx].error = Some(e.to_string());
            let mut elogs = std::mem::take(&mut states[idx].logs);
            elogs.append(&mut retry_logs);
            elogs.push(format!("✗ {e}"));
            // A failed step's agent is stopped like a successful one's —
            // it used to idle at its prompt until the idle sweep found it.
            if stop_step_sessions_wanted(&node.kind, &node.params) {
                for sid in states[idx].sessions.clone() {
                    stop_step_session(ctx, &sid, run_id, &mut elogs).await;
                }
            }
            // A failed step leaves a trace file too — the error is part of
            // the handoff trail (a fix step or a human reads what broke).
            let mut flogs = files.persist_step(
                step_file_base,
                &node.kind,
                node_display_name(node),
                &Value::Null,
                &elogs,
                Some(&e.to_string()),
                Some(attempt_started),
            );
            elogs.append(&mut flogs);
            cap_node_logs(&mut elogs, NODE_LOG_CAP);
            states[idx].logs = elogs;
            states[idx].attempts = Some(attempt);
            states[idx].duration_ms = Some(started.elapsed().as_millis() as u64);
            if progress.enabled() && is_reportable(&node.kind) {
                progress.post(format!(
                    "❌ *{}* failed — {}",
                    node_label(node),
                    truncate(&e.to_string(), 200)
                ));
            }
            // The failure trace file rides along too (what broke, logs).
            if progress.enabled() && (is_reportable(&node.kind) || node.kind == "review_run") {
                progress.post_step_file(files, step_file_base);
            }
            rs.errored.insert(node_id.clone());
        }
    }
}
