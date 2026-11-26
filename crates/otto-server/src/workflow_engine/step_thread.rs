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
    if let Ok(mut m) = resumes().lock() {
        m.retain(|(id, _), _| id != run_id);
    }
}

#[cfg(test)]
thread_local! {
    /// A test's stand-in for the thread runner.
    pub(super) static FAKE_RUNNER: std::cell::RefCell<Option<std::sync::Arc<dyn StepRunner>>> =
        const { std::cell::RefCell::new(None) };
}

/// The thread runner; given the run's user, a usage limit parks the step
/// until the reset the user's limits know of (§4.2), else it fails it.
fn runner(ctx: &ServerCtx, user: Option<&User>) -> std::sync::Arc<dyn StepRunner> {
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
        resets: user.map(|u| usage_resets(ctx, u)),
    };
    std::sync::Arc::new(ThreadStepRunner::new(
        ctx.agent_runtime.clone(),
        std::sync::Arc::new(host),
    ))
}

type ResetsAt = dyn Fn(String, Value) -> otto_core::auth::BoxFuture<'static, Option<chrono::DateTime<chrono::Utc>>>
    + Send
    + Sync;

/// When `provider`'s plan limit resets for `user`: `reset_for` over the
/// user's limits, keyed by the step's account (`meta.account_id`) and the
/// daemon's Claude config dir.
pub(super) fn usage_resets(ctx: &ServerCtx, user: &User) -> std::sync::Arc<ResetsAt> {
    let (pool, data_dir) = (ctx.pool.clone(), ctx.data_dir.clone());
    let (uid, root) = (user.id.clone(), user.is_root);
    std::sync::Arc::new(move |provider: String, meta: Value| {
        let (pool, data_dir, uid) = (pool.clone(), data_dir.clone(), uid.clone());
        Box::pin(async move {
            let limits =
                otto_usage::limits::remaining_limits_filtered(&pool, &uid, root, &data_dir, None);
            let claude_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
            let account = meta.get("account_id").and_then(Value::as_str);
            otto_usage::limits::reset_for(
                &limits.await.ok()?,
                &provider,
                account,
                claude_dir.as_deref(),
            )
        }) as otto_core::auth::BoxFuture<'static, _>
    })
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
    // A loop step with `same_thread` names the previous iteration's thread.
    let mut continue_thread = text("continue_thread");
    if let Some(src) = continued_from
        .as_ref()
        .filter(|_| continue_thread.is_none())
    {
        continue_thread = source_thread(ctx, run_id, src).await;
        if continue_thread.is_none() {
            let _ = log_tx.send(format!("thread of {src} was removed — started a fresh one"));
        }
    }
    let resume = resumes()
        .lock()
        .ok()
        .and_then(|mut m| m.remove(&(run_id.clone(), node.id.clone())));
    // A resumed thread may have written its handoff while the daemon was down.
    if let Some(df) = done_file.as_ref().filter(|_| resume.is_none()) {
        let _ = std::fs::remove_file(df);
    }
    let workflow_id = WorkflowsRepo::new(ctx.pool.clone())
        .run_head(run_id, false)
        .await
        .ok()
        .flatten()
        .map(|h| h.workflow_id);
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
            "work": {"kind": "workflow_run", "id": run_id, "workflow_id": workflow_id},
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
    let runner = runner(ctx, Some(user));
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
            label: otto_workflows::step_util::iteration_label(&node.id, label),
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
    keep_digest_items(ctx, run_id, &node.id, &thread_id).await;
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

/// `thread.available` on a read (§6.5): false once retention deleted the thread.
pub(crate) async fn fill_thread_available(ctx: &ServerCtx, nodes: &mut [NodeRunState]) {
    let threads = nodes.iter_mut().filter_map(|n| n.thread.as_mut());
    for t in threads.filter(|t| t.runner == "threads") {
        t.available = Some(ctx.agent_runtime.get(&t.thread_id).await.is_ok());
    }
}

/// The thread's newest ≤ 50 timeline items, kept beside the run's context
/// files as `.threads/<node>-<attempt>.json` (§6.4) so the run view still has
/// them after retention deleted the thread. Each settled turn of a node
/// writes one file, so the next number is the attempt.
async fn keep_digest_items(ctx: &ServerCtx, run_id: &Id, node_id: &str, thread_id: &Id) {
    let Some(prefix) = digest_prefix(node_id) else {
        return;
    };
    let Ok(items) = ctx.agent_runtime.digest_items(thread_id, 50).await else {
        return;
    };
    let dir = otto_workflows::attachments::context_dir(&ctx.data_dir, run_id).join(".threads");
    crate::offload::blocking(move || {
        let _ = std::fs::create_dir_all(&dir);
        #[allow(clippy::disallowed_methods)] // runs inside offload::blocking
        let n = std::fs::read_dir(&dir).map_or(0, |rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                .count()
        });
        let body = serde_json::to_vec(&items).unwrap_or_default();
        let _ = std::fs::write(dir.join(format!("{prefix}{}.json", n + 1)), body);
    })
    .await;
}

/// `<node>-`, the digest files' prefix — none for an id that is no plain file
/// name (an imported graph's `../x` must not write outside the folder).
pub(super) fn digest_prefix(node_id: &str) -> Option<String> {
    let id = otto_core::paths::safe_component(node_id).filter(|n| !n.starts_with('.'))?;
    Some(format!("{id}-"))
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
    let r = runner(ctx, None);
    for c in st
        .children
        .iter()
        .filter(|c| c.status == "success")
        .filter(|c| c.label.ends_with("earlier turn") || c.label.contains(" · iteration "))
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
    let continuers: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| {
            let from = n.params.get("continue_from").and_then(Value::as_str);
            from.is_some()
                && mine.contains(&from)
                && n.id != st.node_id
                && states.iter().any(|s| {
                    s.node_id == n.id
                        && matches!(s.status, NodeStatus::Pending | NodeStatus::Running)
                })
        })
        .map(|n| n.id.as_str())
        .collect();
    if !continuers.is_empty() {
        // Kept for a later continuer that is not the next step: no idle process meanwhile (§4.5).
        let next = |e: &otto_core::workflows::WorkflowEdge| {
            e.source == st.node_id && continuers.contains(&e.target.as_str())
        };
        if !graph.edges.iter().any(next) {
            r.release_process(&t.thread_id).await;
        }
        return;
    }
    r.close(&t.thread_id, until).await;
    if let Some(t) = st.thread.as_mut() {
        t.closed_at = Some(chrono::Utc::now());
    }
}

/// The run ended (or is held): a stopped run's open threads are closed —
/// and marked so, so a Resume starts the step afresh instead of awaiting a
/// stopped turn — and every thread of the run is kept for the success or
/// the failed period from now (§6.5).
pub(super) async fn retain_run(ctx: &ServerCtx, states: &mut [NodeRunState], success: bool) {
    let r = runner(ctx, None);
    let (ok_h, failed_h) = retention_hours(ctx).await;
    let now = chrono::Utc::now();
    let until = now + chrono::Duration::hours(if success { ok_h } else { failed_h });
    let mut ids = vec![];
    for s in states.iter_mut() {
        if let Some(t) = s.thread.as_mut().filter(|t| t.runner == "threads") {
            if t.closed_at.is_none() {
                r.close(&t.thread_id, until).await;
                t.closed_at = Some(now);
            }
            ids.push(t.thread_id.clone());
        }
        ids.extend(s.children.iter().filter_map(|c| c.thread_id.clone()));
    }
    if !ids.is_empty() {
        r.set_retain_until(&ids, until).await;
    }
}

/// A skipped step's thread stops with it (the run goes on without it).
pub(super) async fn close_open(ctx: &ServerCtx, st: &mut NodeRunState) {
    let Some(t) = st
        .thread
        .as_mut()
        .filter(|t| t.runner == "threads" && t.closed_at.is_none())
    else {
        return;
    };
    runner(ctx, None)
        .close(&t.thread_id, retain_until(ctx, false).await)
        .await;
    t.closed_at = Some(chrono::Utc::now());
}
