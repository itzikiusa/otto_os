//! Workflow steps on structured runtime threads (engine.md §4): the run's
//! latched step runner (DR-019), the thread runner's call, its events turned
//! into the node's phases, wait, needs-you and thread record, and the
//! thread's retention at the step's settle and the run's end (§6.5).

use super::*;
use otto_agent_protocol::step_runner::{DaemonStepHost, ThreadStepRunner};
use otto_agent_run::step::{StepErrorClass, StepEvent, StepRunner, StepTurn};
use otto_core::workflows::{StepPhase, StepThread};
use otto_workflows::step_util::Settled;
pub(super) use otto_workflows::step_util::{record, StepNote};

/// The runner every step of a run uses, read once per run (DR-019.5):
/// `OTTO_WF_STEP_RUNNER`, else the `wf_step_runner` setting, else threads
/// (the offline e2e daemon keeps its canned PTY agent). A provider the
/// runtime has no driver for always runs on PTY.
pub(super) async fn runner_for(ctx: &ServerCtx, run_id: &Id, provider: &str) -> &'static str {
    if otto_core::agent_runtime::RuntimeProvider::parse(provider).is_none() {
        return "pty";
    }
    if let Some(r) = latched().lock().ok().and_then(|m| m.get(run_id).copied()) {
        return r;
    }
    let setting = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("wf_step_runner")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_string));
    let want = std::env::var("OTTO_WF_STEP_RUNNER").ok().or(setting);
    let pick = match want.as_deref().map(str::trim) {
        Some("pty") => "pty",
        None if otto_e2e() => "pty",
        _ => "threads",
    };
    latched()
        .lock()
        .map(|mut m| *m.entry(run_id.clone()).or_insert(pick))
        .unwrap_or(pick)
}

fn latched() -> &'static std::sync::Mutex<HashMap<Id, &'static str>> {
    static L: OnceLock<std::sync::Mutex<HashMap<Id, &'static str>>> = OnceLock::new();
    L.get_or_init(Default::default)
}

/// Steps a restart caught on an open thread: `(run, node)` → `(thread, start ordinal)`.
type Resumes = std::sync::Mutex<HashMap<(Id, String), (Id, u32)>>;

fn resumes() -> &'static Resumes {
    static R: OnceLock<Resumes> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// The re-entered step awaits its recorded thread instead of a fresh one.
pub(super) fn mark_resume(run_id: &Id, node_id: &str, t: &StepThread) {
    let ordinal = t.start_ordinal.unwrap_or(0) as u32;
    if let Ok(mut m) = resumes().lock() {
        m.insert(
            (run_id.clone(), node_id.to_string()),
            (t.thread_id.clone(), ordinal),
        );
    }
}

/// The run ended: its next execution latches the runner again.
pub(super) fn forget(run_id: &Id) {
    if let Ok(mut m) = latched().lock() {
        m.remove(run_id);
    }
}

#[cfg(test)]
thread_local! {
    /// A test's stand-in for the thread runner.
    pub(super) static FAKE_RUNNER: std::cell::RefCell<Option<std::sync::Arc<dyn StepRunner>>> =
        const { std::cell::RefCell::new(None) };
}

fn runner(ctx: &ServerCtx) -> std::sync::Arc<dyn StepRunner> {
    #[cfg(test)]
    if let Some(r) = FAKE_RUNNER.with(|f| f.borrow().clone()) {
        return r;
    }
    let host = DaemonStepHost {
        manager: ctx.manager.clone(),
        approvals: Some((
            otto_state::McpApprovalRepo::new(ctx.pool.clone()),
            ctx.events.clone(),
        )),
        resets: None,
    };
    std::sync::Arc::new(ThreadStepRunner::new(
        ctx.agent_runtime.clone(),
        std::sync::Arc::new(host),
    ))
}

/// One agent turn of a step on a runtime thread. Returns `(reply, thread id)`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    node: &WorkflowNode,
    wf_name: &str,
    run_id: &Id,
    provider: &str,
    model: Option<&str>,
    prompt: &str,
    cwd: &str,
    done_file: Option<std::path::PathBuf>,
    session_tx: &tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
) -> Result<(String, Id)> {
    let p = &node.params;
    let text = |k: &str| {
        p.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let continued_from = text("continue_from");
    let mut continue_thread = None;
    if let Some(src) = &continued_from {
        continue_thread = source_thread(ctx, run_id, src).await;
        if continue_thread.is_none() {
            let _ = log_tx.send(format!("thread of {src} was removed — started a fresh one"));
        }
    }
    if let Some(df) = &done_file {
        let _ = std::fs::remove_file(df);
    }
    let label = node_display_name(node);
    let t = StepTurn {
        user_id: user.id.clone(),
        workspace_id: ws.id.clone(),
        provider: provider.to_string(),
        model: model
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string),
        effort: text("effort"),
        cwd: cwd.into(),
        title: format!("WF {wf_name} · {label} · run {}", short_id(run_id)),
        meta: json!({
            "source": "workflow", "run_id": run_id, "wf_name": wf_name,
            "node_id": node.id, "node_kind": node.kind, "cwd": cwd,
            "work": {"kind": "workflow_run", "id": run_id},
        }),
        prompt: format!("{prompt}{WF_STEP_RULES}"),
        continue_thread,
        handoff: done_file,
        read_only: false,
        timeout: p
            .get("timeout_s")
            .and_then(Value::as_u64)
            .map(|s| Duration::from_secs(s.clamp(60, 36_000))),
        no_progress: None,
    };
    let _ = log_tx.send(format!("⏳ starting {provider} thread"));
    let (ev_tx, mut ev_rx) = tokio::sync::mpsc::unbounded_channel::<StepEvent>();
    let runner = runner(ctx);
    let resume = resumes()
        .lock()
        .ok()
        .and_then(|mut m| m.remove(&(run_id.clone(), node.id.clone())));
    let cancel = otto_core::cancel_signal::CancelSignal::new();
    let fut = async {
        match &resume {
            Some((id, ordinal)) => runner.resume(id, *ordinal, &t, ev_tx, cancel).await,
            None => runner.run(t.clone(), ev_tx, cancel).await,
        }
    };
    tokio::pin!(fut);
    let note = |n: StepNote| {
        let _ = session_tx.send(AgentAssociation::Step(n));
    };
    let forward = |e: StepEvent| match e {
        StepEvent::Waiting(w) => note(StepNote::Phase(StepPhase::Waiting, Some(w))),
        StepEvent::Started {
            thread_id,
            start_ordinal,
        } => note(StepNote::Thread {
            thread_id,
            start_ordinal,
            provider: provider.to_string(),
            continued_from: continued_from.clone(),
        }),
        StepEvent::Working => note(StepNote::Phase(StepPhase::Working, None)),
        StepEvent::NeedsYou(a) => note(StepNote::NeedsYou(a)),
        StepEvent::Resumed { answer, by, .. } => note(StepNote::Resumed { answer, by }),
        StepEvent::Finishing => note(StepNote::Phase(StepPhase::Finishing, None)),
        StepEvent::Activity(a) => {
            let _ = activity_tx.send(a);
        }
        StepEvent::Log(l) => {
            let _ = log_tx.send(l);
        }
    };
    let out = loop {
        tokio::select! {
            biased;
            Some(e) = ev_rx.recv() => forward(e),
            r = &mut fut => break r,
        }
    };
    while let Ok(e) = ev_rx.try_recv() {
        forward(e);
    }
    let out = out?;
    let (thread_id, reply, error) = (out.thread_id.clone(), out.reply.clone(), out.error.clone());
    note(StepNote::Settled(Box::new(Settled {
        turns: out.turns,
        usage: out.usage,
        model: out.model,
        effort: out.effort,
        digest: out.digest,
        error_class: out.error.as_ref().map(|e| e.class.as_str().to_string()),
        sub_threads: out.sub_threads,
    })));
    if let Some(e) = error {
        // A retry opens a fresh thread: this one is done.
        runner
            .close(&thread_id, retain_until(ctx, false).await)
            .await;
        let msg = match e.class {
            // `otto_workflows::retry` backs a per-minute limit off like an overload.
            StepErrorClass::RateLimit => format!("rate limit: {}", e.message),
            _ => e.message,
        };
        return Err(otto_core::Error::Upstream(msg));
    }
    let _ = log_tx.send("✓ step complete".into());
    Ok((reply, thread_id))
}

/// The thread of the step `src` this run recorded, unless retention removed it.
async fn source_thread(ctx: &ServerCtx, run_id: &Id, src: &str) -> Option<Id> {
    let run = WorkflowsRepo::new(ctx.pool.clone())
        .get_run(run_id)
        .await
        .ok()?;
    let t = run.nodes.into_iter().find(|n| n.node_id == src)?.thread?;
    (t.runner == "threads" && ctx.agent_runtime.get(&t.thread_id).await.is_ok())
        .then_some(t.thread_id)
}

/// When a step's thread may go: `now` for a success with 0 success hours,
/// else after the failed period (the run's outcome is not known yet).
async fn retain_until(ctx: &ServerCtx, success: bool) -> chrono::DateTime<chrono::Utc> {
    let (ok_h, failed_h) = retention_hours(ctx).await;
    let now = chrono::Utc::now();
    if success && ok_h == 0 {
        now
    } else {
        now + chrono::Duration::hours(failed_h)
    }
}

/// `workflow_thread_retention_success_hours` (24) and `…_failed_hours` (168).
async fn retention_hours(ctx: &ServerCtx) -> (i64, i64) {
    let settings = otto_state::SettingsRepo::new(ctx.pool.clone());
    let hours = async |key: &str, default: i64| {
        settings
            .get(key)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_i64())
            .map_or(default, |h| h.max(0))
    };
    (
        hours("workflow_thread_retention_success_hours", 24).await,
        hours("workflow_thread_retention_failed_hours", 168).await,
    )
}

/// A settled step closes its thread, unless a step that has not settled yet
/// continues it (`continue_from`, §4.5): the last continuer closes it.
pub(super) async fn close_settled(
    ctx: &ServerCtx,
    graph: &WorkflowGraph,
    states: &[NodeRunState],
    st: &mut NodeRunState,
) {
    let until = retain_until(ctx, st.status == NodeStatus::Success).await;
    let r = runner(ctx);
    for c in st
        .children
        .iter()
        .filter(|c| c.status == "success" && c.label.ends_with("earlier turn"))
    {
        if let Some(id) = &c.thread_id {
            r.close(id, until).await;
        }
    }
    let Some(t) = st
        .thread
        .as_ref()
        .filter(|t| t.runner == "threads" && t.closed_at.is_none())
    else {
        return;
    };
    let mine = [Some(st.node_id.as_str()), t.continued_from.as_deref()];
    let continued_later = graph.nodes.iter().any(|n| {
        let from = n.params.get("continue_from").and_then(Value::as_str);
        from.is_some()
            && mine.contains(&from)
            && n.id != st.node_id
            && states.iter().any(|s| {
                s.node_id == n.id && matches!(s.status, NodeStatus::Pending | NodeStatus::Running)
            })
    });
    if continued_later {
        return;
    }
    r.close(&t.thread_id, until).await;
    if let Some(t) = st.thread.as_mut() {
        t.closed_at = Some(chrono::Utc::now());
    }
}

/// The run ended: a stopped run's open threads are closed, and every thread
/// of the run is kept for the success or the failed period from now (§6.5).
pub(super) async fn retain_run(ctx: &ServerCtx, states: &[NodeRunState], success: bool) {
    let r = runner(ctx);
    let mut ids = vec![];
    for s in states {
        if let Some(t) = s.thread.as_ref().filter(|t| t.runner == "threads") {
            if t.closed_at.is_none() {
                r.close(&t.thread_id, chrono::Utc::now()).await;
            }
            ids.push(t.thread_id.clone());
        }
        ids.extend(s.children.iter().filter_map(|c| c.thread_id.clone()));
    }
    if ids.is_empty() {
        return;
    }
    let (ok_h, failed_h) = retention_hours(ctx).await;
    let hours = if success { ok_h } else { failed_h };
    r.set_retain_until(&ids, chrono::Utc::now() + chrono::Duration::hours(hours))
        .await;
}
