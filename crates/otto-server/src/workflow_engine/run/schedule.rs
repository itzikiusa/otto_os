//! The scheduler driver of a run (engine.md §3): `otto_workflows::schedule`
//! decides which nodes start, in run order, with which step number; every
//! started node runs as its own future, polled by the run's own task, and
//! the shared bookkeeping (outputs, errored, untaken edges) changes only
//! here, one settled node at a time. A node future is today's per-node body:
//! the attempt loop with retries, live session / log / activity harvesting
//! and cancel / skip handling, then the outcome (step files, edge pruning,
//! session stop, chat progress) on its own state, published through
//! [`RunBoard`].

use super::super::step_thread::StepNote;
use super::super::*;
use super::board::RunBoard;
use super::state::RunState;
use futures_util::future::BoxFuture;
use futures_util::stream::{FuturesUnordered, StreamExt};
use otto_workflows::schedule::{
    Init, Outcome, Scheduler, SkipWhy, Start, DEFAULT_MAX_PARALLEL_STEPS,
};

/// What every node of one run shares: who and where it runs as, the run's
/// input and context files, and the scope it was started with. Immutable for
/// the whole run (the mutable side is [`RunState`] and the [`RunBoard`]).
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

/// How a node future ended, for the scheduler.
struct NodeDone {
    idx: usize,
    /// The node's output (incl. a chat skip's marker output).
    output: Option<Value>,
    outcome: Outcome,
    /// The run was canceled, or its active-time budget ran out.
    canceled: bool,
    timed_out: bool,
}

/// `wf_max_parallel_steps`: per-run step slots (default 4, 1 = serial).
async fn max_parallel_steps(ctx: &ServerCtx) -> usize {
    otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("wf_max_parallel_steps")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .map_or(DEFAULT_MAX_PARALLEL_STEPS, |n| n as usize)
}

/// Run the graph: start what the scheduler says, settle what finishes,
/// until nothing runs and nothing can start, or the run is canceled or out
/// of time budget (dropping the running node futures stops them all).
pub(in crate::workflow_engine) async fn drive(cx: RunCx<'_>, rs: &mut RunState, order: &[String]) {
    if rs.canceled {
        return;
    }
    let graph = &cx.workflow.graph;
    let has_output: HashSet<String> = rs.outputs.keys().cloned().collect();
    let init = Init {
        states: &rs.states,
        has_output: &has_output,
        errored: &rs.errored,
        inactive_edges: &rs.inactive_edges,
        run_set: cx.run_set,
        prior: cx.prior_nodes,
        continue_unfinished: cx.continue_unfinished,
    };
    let mut sched = Scheduler::new(graph, order, init, max_parallel_steps(cx.ctx).await);
    let base = cx.repo.get_run(cx.run_id).await.ok();
    let board = RunBoard::new(std::mem::take(&mut rs.states), base);
    let mut active: FuturesUnordered<BoxFuture<'_, Option<NodeDone>>> = FuturesUnordered::new();
    loop {
        for start in sched.startable() {
            match start {
                Start::Adopt { idx, step_no } => {
                    let Some(n) = step_no else { continue };
                    // An old row's number is kept on the state from now on.
                    let ok = board.with(idx, |s| {
                        s.step_no.get_or_insert(n);
                        s.status == NodeStatus::Success
                    });
                    if ok {
                        files_adopt(cx, &graph.nodes[idx], n);
                    }
                }
                Start::Skip { idx, why } => {
                    if why == SkipWhy::Upstream {
                        rs.errored.insert(graph.nodes[idx].id.clone());
                    }
                    board.with(idx, |s| {
                        s.status = NodeStatus::Skipped;
                        s.logs = vec![why.log_line().into()];
                    });
                    let board = &board;
                    active.push(Box::pin(async move {
                        board.publish_current(cx, idx).await;
                        None
                    }));
                }
                Start::Queue { idx, step_no, .. } => {
                    let (in_use, cap) = sched.slots();
                    let file = step_file(&graph.nodes[idx], step_no);
                    board.with(idx, |s| {
                        s.status = NodeStatus::Running;
                        s.step_no = Some(step_no);
                        s.step_file = Some(file);
                        s.phase = Some(otto_core::workflows::StepPhase::Queued);
                        s.logs = vec![format!(
                            "waiting for a step slot ({in_use} of {cap} in use)"
                        )];
                    });
                    let board = &board;
                    active.push(Box::pin(async move {
                        board.publish_current(cx, idx).await;
                        None
                    }));
                }
                Start::Run {
                    idx,
                    step_no,
                    sources,
                } => {
                    let input = assemble_input(&sources, &rs.outputs, cx.input);
                    let node = &graph.nodes[idx];
                    let board = &board;
                    active.push(Box::pin(async move {
                        cx.env.budget.on_start().await;
                        let done = boxed(|| run_node(cx, board, node, idx, input, step_no)).await;
                        cx.env.budget.on_settle();
                        Some(done)
                    }));
                }
            }
        }
        // Ready nodes held back by an earlier one in run order say so (§3.2).
        if let Some((b, ready)) = sched.run_order_wait() {
            let note = format!(
                "waiting for {} to start (run order)",
                node_display_name(&graph.nodes[b])
            );
            for i in ready {
                board.with(i, |s| s.logs = vec![note.clone()]);
            }
        }
        let Some(done) = active.next().await else {
            break;
        };
        let Some(done) = done else {
            continue;
        };
        if done.canceled || done.timed_out {
            rs.canceled |= done.canceled;
            rs.timed_out |= done.timed_out;
            break;
        }
        let id = graph.nodes[done.idx].id.clone();
        if done.outcome.errored {
            rs.errored.insert(id.clone());
        }
        rs.inactive_edges
            .extend(done.outcome.inactive_edges.iter().cloned());
        if let Some(out) = done.output {
            rs.outputs.insert(id, out);
        }
        sched.settle(done.idx, done.outcome);
    }
    drop(active);
    rs.states = board.into_states();
    if rs.canceled {
        let running = rs
            .states
            .iter_mut()
            .filter(|s| s.status == NodeStatus::Running);
        running.for_each(otto_workflows::step_util::mark_canceled);
    }
}

/// The step file base name of `node` numbered `step_no`.
fn step_file(node: &WorkflowNode, step_no: u32) -> String {
    crate::workflow_context::step_base_name(step_no as usize, node_display_name(node), None, None)
}

/// Re-link an adopted successful step's files under its number.
fn files_adopt(cx: RunCx<'_>, node: &WorkflowNode, step_no: u32) {
    cx.files.adopt_step(&step_file(node, step_no), &node.kind);
}

/// Run one started node: mark it running under its step number, run its
/// attempts, then settle it (chat skip, stop or outcome) on its own state and
/// publish that.
async fn run_node(
    cx: RunCx<'_>,
    board: &RunBoard,
    node: &WorkflowNode,
    idx: usize,
    node_input: Value,
    step_no: u32,
) -> NodeDone {
    let mut st = board.get(idx);
    let mut done = NodeDone {
        idx,
        output: None,
        outcome: Outcome::default(),
        canceled: false,
        timed_out: false,
    };
    // Honor a cancel request (the API flips the run status to Canceled).
    if cx.repo.is_canceled(cx.run_id).await {
        done.canceled = true;
        return done;
    }
    // Stop once the run has exceeded its global time budget.
    if cx.env.budget.expired() {
        st.status = NodeStatus::Error;
        st.error = Some("workflow active time budget exhausted".into());
        board.set(idx, &st);
        done.timed_out = true;
        return done;
    }
    let step_file_base = step_file(node, step_no);
    // Re-entered after a restart (DR-006): the old agent may live on in its
    // holder beside the fresh step — stop it; a thread step resumes its thread
    // (a loop's inner steps resume their own), and the handoff its agent
    // wrote since the step started is its answer.
    let mut resumed_since = None;
    if let Some(started) = st.started_at {
        let mut logs = vec![];
        for sid in std::mem::take(&mut st.sessions) {
            stop_step_session(cx.ctx, &sid, cx.run_id, &mut logs).await;
        }
        if let Some(t) = st
            .thread
            .as_ref()
            .filter(|t| t.runner == "threads" && t.closed_at.is_none() && node.kind != "loop")
        {
            super::super::step_thread::mark_resume(cx.run_id, &node.id, t);
            resumed_since = Some(std::time::SystemTime::from(started));
        }
    }
    // The preamble lists the settled ancestors' step files (and this node's
    // own, for a loop's earlier iterations), never a sibling's (§5.2).
    let mut listed = board.step_nos_of(&otto_workflows::graph::ancestors(
        &cx.workflow.graph,
        &node.id,
    ));
    listed.insert(step_no);
    cx.files.set_listed_steps(step_no as usize, listed);
    st.status = NodeStatus::Running;
    st.started_at = Some(chrono::Utc::now());
    st.logs = vec![format!("▶ {} started", node.kind)];
    st.phase = None;
    // A wait or a question from before a pause or a restart is over.
    (st.attention, st.wait) = (None, None);
    st.step_no = Some(step_no);
    st.step_file = Some(step_file_base.clone());
    // Signal node start so the UI can show live progress immediately.
    board.publish(cx, idx, &st).await;
    if cx.progress.enabled() && is_reportable(&node.kind) {
        cx.progress
            .post(format!("▶ *{}* started", node_label(node)));
    }

    let started = Instant::now();
    let scope = StepScope {
        step_no: step_no as usize,
        iter: None,
        inner_idx: None,
    };
    let mut ch = NodeChannels::new();
    let mut attempts =
        run_attempts(cx, board, &mut st, node, idx, &node_input, &scope, &mut ch).await;
    if let Some(t) = resumed_since.filter(|_| attempts.attempt == 1) {
        attempts.attempt_started = t;
    }
    // Drain any session ids reported right as the node finished.
    while let Ok(sid) = ch.sess_rx.try_recv() {
        record_association(&mut st, sid);
    }
    // Drain any trailing live-log lines — kept on EVERY path now (R5.5): the
    // phase lines ARE the record of how the step completed.
    while let Ok(line) = ch.log_rx.try_recv() {
        st.logs.push(line);
    }
    // Drain trailing activity snapshots (last one wins), then clear it:
    // `activity` is only meaningful while the node RUNS.
    while ch.activity_rx.try_recv().is_ok() {}
    st.activity = None;
    // A marker nobody consumed (the step finished first) dies with the step.
    take_skip_marker(cx.ctx, cx.run_id, &node.id);
    if attempts.skip_current {
        let inactive_edges = skip_node(cx, board, &mut st, node, idx, started).await;
        done.output = st.output.clone();
        done.outcome = Outcome {
            errored: false,
            has_output: true,
            inactive_edges,
        };
        return done;
    }
    // Canceled mid-node: the run goes straight to finalize, which marks the
    // remaining nodes Skipped, flips the run to Canceled, and kills every
    // session this run spawned (incl. this node's, harvested just above).
    if attempts.canceled || attempts.timed_out {
        board.set(idx, &st);
        done.canceled = attempts.canceled;
        done.timed_out = attempts.timed_out;
        return done;
    }
    let (output, outcome) = settle(
        cx,
        &mut st,
        node,
        &node_input,
        started,
        &step_file_base,
        attempts,
    )
    .await;
    // "settled" is not a phase (§2.2).
    st.phase = None;
    // The step's answer is on its state: its thread may close now (§6.5).
    let states = board.snapshot();
    super::super::step_thread::close_settled(cx.ctx, &cx.workflow.graph, &states, &mut st).await;
    // Signal node finish so the inspector can update without waiting for the next poll.
    board.publish(cx, idx, &st).await;
    done.output = output;
    done.outcome = outcome;
    done
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
    /// The run was canceled, or its active-time budget ran out, mid-node.
    canceled: bool,
    timed_out: bool,
}

/// Run the node, honoring its retry policy (default: a single attempt). A
/// node that spawns an openable agent session reports its id over `sess_tx`
/// the moment it's created; we record it on the node state and persist+emit
/// immediately so the run view can open it *while running*.
#[allow(clippy::too_many_arguments)]
async fn run_attempts(
    cx: RunCx<'_>,
    board: &RunBoard,
    st: &mut NodeRunState,
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
    let (mut canceled, mut timed_out) = (false, false);
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
    let mut park = None;
    // A step's answer re-takes the run's slot beside the watch, not in it (CR1-022).
    let mut unpark = None;
    let result = loop {
        attempt += 1;
        st.attempts = Some(attempt);
        st.logs.push(format!(
            "{} — attempt {}/{}",
            node_display_name(node),
            attempt,
            attempt_limit
        ));
        board.publish(cx, idx, st).await;
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
                    timed_out = true;
                    break Err(otto_core::Error::Upstream("workflow active time budget exhausted".into()));
                }
                Some(sid) = sess_rx.recv() => {
                    // A step waiting on a person parks (§2.4); its answer unparks it.
                    match &sid {
                        AgentAssociation::Step(StepNote::NeedsYou(a)) => {
                            if park.is_none() {
                                park = Some(env.budget.park());
                                super::chat::needs_you(cx, node, a).await;
                            }
                        }
                        AgentAssociation::Step(StepNote::Resumed { .. }) => {
                            if let Some(p) = park.take() {
                                unpark = Some(Box::pin(p.unpark()));
                            }
                        }
                        _ => {}
                    }
                    if record_association(st, sid) {
                        board.publish(cx, idx, st).await;
                        // The write carried any buffered log lines too.
                        log_persist.carried();
                    }
                }
                Some(line) = log_rx.recv() => {
                    // Live progress line (R9) — append to the node's logs. The
                    // first line after a quiet spell is written (and pushed to
                    // the run detail) at once; a burst is coalesced into one
                    // write per LOG_PERSIST_EVERY / LOG_PERSIST_LINES.
                    st.logs.push(line);
                    if st.logs.len() > NODE_LIVE_LOG_CAP {
                        cap_node_logs(&mut st.logs, NODE_LIVE_LOG_CAP);
                    }
                    if log_persist.on_line(Instant::now()) {
                        board.publish(cx, idx, st).await;
                    }
                }
                () = async { unpark.as_mut().expect("guarded").await }, if unpark.is_some() => {
                    unpark = None;
                }
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(
                    log_persist.flush_at.unwrap_or_else(Instant::now),
                )), if log_persist.flush_at.is_some() => {
                    // Trailing flush of a coalesced log burst.
                    log_persist.wrote_log(Instant::now());
                    board.publish(cx, idx, st).await;
                }
                Some(a) = activity_rx.recv() => {
                    // R1.4: the live sub-agent/phase snapshot. Always kept on
                    // the state (the next emit carries it), persisted at most
                    // every 5s so a chatty sweep can't hammer the run row.
                    st.activity = Some(bound_activity(a));
                    if last_activity_persist.is_none_or(|t| t.elapsed() >= Duration::from_secs(5)) {
                        last_activity_persist = Some(Instant::now());
                        board.publish(cx, idx, st).await;
                        log_persist.carried();
                    }
                }
                wake = cancel_watch.next(repo, || take_skip_marker(ctx, run_id, node_id)) => {
                    // Cancel drops the scoped node driver; finalization kills
                    // this run's sessions. No node task remains detached.
                    let parked_at_approval = match wake {
                        CancelWake::Canceled => {
                            canceled = true;
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
                        st.logs.push(
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
                if canceled || skip_current || timed_out {
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
                // The retry works: a park of the failed attempt ends with it.
                if let Some(p) = park.take() {
                    p.unpark().await;
                }
                attempt_limit = max_eff + 1;
                st.logs.push(otto_workflows::retry::retry_log_line(
                    attempt + 1,
                    max_eff + 1,
                    sleep_ms,
                    &reason,
                ));
                let note = format!("attempt {} of {} — {reason}", attempt + 1, max_eff + 1);
                otto_workflows::step_util::note_mark(
                    st,
                    otto_core::workflows::StepPhase::Queued,
                    &note,
                );
                board.publish(cx, idx, st).await;
                // The failed attempt's agent must not stay alive next to the
                // retry's fresh one (5 live PTYs per step in a 529 storm; and
                // fd exhaustion is itself a retry class). Suspend/kill it the
                // way a successful step's session is stopped.
                if stop_step_sessions_wanted(&node.kind, &node.params) {
                    while let Ok(sid) = sess_rx.try_recv() {
                        record_association(st, sid);
                    }
                    for sid in st.sessions.clone() {
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
                        timed_out = true;
                        break Err(otto_core::Error::Upstream("workflow active time budget exhausted".into()));
                    }
                    stopped = backoff_canceled(repo, run_id, Duration::from_millis(sleep_ms)) => {
                        if stopped {
                            canceled = true;
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
        canceled,
        timed_out,
    }
}

/// Skipped mid-node via a chat `skip` command: stop THIS node's agents, mark
/// it Skipped, and let the run go on (unlike cancel). A marker output keeps
/// dependents satisfied — an empty output would make them BranchSkip and
/// cascade the whole tail away; it is stored on the node state too, so a
/// retry or restart adopts it with its output. Returns the out-edges not taken.
async fn skip_node(
    cx: RunCx<'_>,
    board: &RunBoard,
    st: &mut NodeRunState,
    node: &WorkflowNode,
    idx: usize,
    started: Instant,
) -> Vec<String> {
    let RunCx { ctx, workflow, .. } = cx;
    // A skipped review step's fleet is cancelled through the review
    // engine, like a run cancel does: killing a reviewer's PTY alone
    // makes its recovery loop respawn it, ownerless.
    for review_id in st.review_ids.clone() {
        if let Ok(review) = ctx.reviews_store.get_review(&review_id).await {
            crate::modules::cancel_running_review(ctx, &review, &workflow.workspace_id).await;
        }
    }
    for sid in &st.sessions {
        if let Err(e) = ctx.manager.kill_session(sid).await {
            tracing::warn!("skip: failed to kill workflow session {sid}: {e}");
        }
    }
    super::super::step_thread::close_open(ctx, st).await;
    st.status = NodeStatus::Skipped;
    st.logs.push("⏭ skipped via chat command".into());
    // A skipped step decided nothing, so none of its CONDITIONAL
    // out-edges is taken (those branches BranchSkip). Unconditional edges
    // stay live; the marker output below satisfies those dependents.
    let mut inactive = Vec::new();
    for e in outgoing_edges(&workflow.graph, &node.id) {
        if e.condition.is_some() {
            inactive.push(e.id.clone());
            st.logs
                .push(format!("edge → {} not taken (step skipped)", e.target));
        }
    }
    cap_node_logs(&mut st.logs, NODE_LOG_CAP);
    st.duration_ms = Some(started.elapsed().as_millis() as u64);
    st.output = Some(json!({ "skipped": true, "note": "skipped via chat command" }));
    board.publish(cx, idx, st).await;
    inactive
}

/// Settle a node whose attempts finished (not skipped, canceled or timed
/// out): record its output or error, write its step file, stop its agent,
/// prune its out-edges and report it to the chat thread. Returns the output
/// and what the scheduler needs of the outcome.
#[allow(clippy::too_many_arguments)]
async fn settle(
    cx: RunCx<'_>,
    st: &mut NodeRunState,
    node: &WorkflowNode,
    node_input: &Value,
    started: Instant,
    step_file_base: &str,
    done: Attempts,
) -> (Option<Value>, Outcome) {
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
    match result {
        Ok((out, mut ret_logs)) => {
            let failure = outcome_error(&out).map(str::to_owned);
            st.status = if failure.is_some() {
                NodeStatus::Error
            } else {
                NodeStatus::Success
            };
            st.error = failure.clone();
            st.output = Some(out.clone());
            // The step line's one-line summary (CR1-080), ≤ 200 chars.
            st.brief = brief_summary(&out).map(|s| truncate(&s, 200).to_string());
            let built = otto_workflows::step_util::output_deliverables(&out);
            if !built.is_empty() {
                st.deliverables = built;
            }
            // R5.5: the LIVE vector (which already starts with `start_line`
            // and carries every streamed phase line) is the base — the
            // returned lines are appended, never substituted for it.
            let mut logs = std::mem::take(&mut st.logs);
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
                for sid in st.sessions.clone() {
                    stop_step_session(ctx, &sid, run_id, &mut logs).await;
                }
            }
            // Prune outgoing edges whose condition fails on this output.
            let run = super::state::run_data(files, input);
            let (pruned, mut plogs) = eval_outgoing(&workflow.graph, node, &out, node_input, &run);
            logs.append(&mut plogs);
            cap_node_logs(&mut logs, NODE_LOG_CAP);
            st.logs = logs;
            st.attempts = Some(attempt);
            // Also harvest a session id carried in the output (dedups with the
            // live channel report).
            if st.thread.as_ref().is_none_or(|t| t.runner == "pty") {
                harvest_session_ids(&out, &mut st.sessions);
            }
            let elapsed = started.elapsed().as_millis() as u64;
            st.duration_ms = Some(elapsed);
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
                let caption =
                    otto_workflows::chat_status::step_caption(node_display_name(node), st, None);
                progress.post_step_file_as(files, step_file_base, caption);
            }
            let outcome = Outcome {
                errored: failure.is_some(),
                has_output: true,
                inactive_edges: pruned,
            };
            (Some(out), outcome)
        }
        Err(e) => {
            st.status = NodeStatus::Error;
            st.error = Some(e.to_string());
            let mut elogs = std::mem::take(&mut st.logs);
            elogs.append(&mut retry_logs);
            elogs.push(format!("✗ {e}"));
            // A failed step's agent is stopped like a successful one's —
            // it used to idle at its prompt until the idle sweep found it.
            if stop_step_sessions_wanted(&node.kind, &node.params) {
                for sid in st.sessions.clone() {
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
            st.logs = elogs;
            st.attempts = Some(attempt);
            st.duration_ms = Some(started.elapsed().as_millis() as u64);
            if progress.enabled() && is_reportable(&node.kind) {
                progress.post(format!(
                    "❌ *{}* failed — {}",
                    node_label(node),
                    truncate(&e.to_string(), 200)
                ));
            }
            // The failure trace file rides along too (what broke, logs).
            if progress.enabled() && (is_reportable(&node.kind) || node.kind == "review_run") {
                let caption =
                    otto_workflows::chat_status::step_caption(node_display_name(node), st, None);
                progress.post_step_file_as(files, step_file_base, caption);
            }
            let outcome = Outcome {
                errored: true,
                ..Outcome::default()
            };
            (None, outcome)
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/workflow_parallel.rs"]
mod tests;
