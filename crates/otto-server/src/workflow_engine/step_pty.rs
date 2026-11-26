//! The PTY step runner (engine.md §4.3): one agent turn as a visible,
//! openable terminal session through the shared `run_session_turn` flow and
//! the turn oracle. Frozen — moved here unchanged when steps moved to runtime
//! threads; used when the run latched `wf_step_runner = "pty"` and for
//! providers the runtime has no driver for. Removed one release later (Q4).

use super::*;

/// Default no-REAL-progress trip for agent-backed workflow steps (overridable
/// via the `wf_step_stall_secs` setting; 0 disables). Progress means the
/// provider's transcript/rollout growing or child processes burning CPU — not
/// PTY repaints, which a stuck TUI spinner keeps emitting forever. A tripped
/// step is retried per `resolve_retry`. Separate from — and never longer
/// than — the 10h max session lifespan backstop. See design R5.
const WF_STEP_STALL_DEFAULT: Duration = Duration::from_secs(5 * 60);

/// Run one agent turn as a visible, openable session (reusing the shared
/// `run_session_turn` flow), reporting the session id over `session_tx` the
/// moment the session exists so the run view can open it live. Returns
/// `(reply, session_id)`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    node: &WorkflowNode,
    wf_name: &str,
    run_id: &Id,
    provider: &str,
    // Optional model alias (e.g. "opus"/"haiku"). Empty/None = provider default.
    // Forwarded via `meta.model` so `SessionManager::model_args` adds `--model`.
    model: Option<&str>,
    prompt: &str,
    cwd: &str,
    // The step's handoff `.md` (named by the preamble) as a completion MARKER:
    // for providers without a pollable transcript (codex/agy) it is the ONLY
    // positive "the step finished" signal — without it they sit at their input
    // box until the stall trip errors the step. `None` for steps whose reply
    // must come from the transcript (canvas/goals/PR-draft agents).
    done_file: Option<std::path::PathBuf>,
    session_tx: &tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    // The node's LIVE log channel: the oracle's phase lines stream here as the
    // turn runs and are kept in the step's logs afterwards (R1.4/R5.5).
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    // Live sub-agent snapshot for the run view's chip + nested rows.
    activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
) -> Result<(String, Id)> {
    // Title carries the workflow name, the step, and a short run id so two
    // concurrent runs of the same workflow are distinguishable in the Agents
    // list (they'd otherwise both read "Workflow: Write tests").
    let label = if node.name.is_empty() {
        node.kind.as_str()
    } else {
        node.name.as_str()
    };
    let title = format!("WF {wf_name} · {label} · run {}", short_id(run_id));
    // Stamp the FULL run id (+ workflow name) into meta so every workflow-spawned
    // session is provably attributable to its specific run — the UI/global Agents
    // list can filter + group sessions by exact run.
    let mut meta = json!({
        "source": "workflow",
        "run_id": run_id,
        "wf_name": wf_name,
        "node_id": node.id,
        "node_kind": node.kind,
        "cwd": cwd,
    });
    // Forward the model alias so the spawn adds `--model` (providers without the
    // flag ignore it). Empty string = provider default → omitted.
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        meta["model"] = json!(m);
    }
    // R5.5: the step's phases are visible from the first line. The engine logs
    // the boot line itself (the oracle's `Phase::Booting` is therefore never
    // re-logged by the forwarder below).
    let _ = log_tx.send(format!("⏳ starting {provider} session"));
    // E2E: the offline daemon never spawns a CLI, so a spec that needs a step
    // with sub-agents drives a synthetic sequence off a prompt sentinel.
    if otto_e2e() {
        if let Some((n, hold)) = e2e_subagents(prompt) {
            e2e_subagent_sequence(n, hold, log_tx, activity_tx).await;
        }
    }
    let tx = session_tx.clone();
    // The session id the turn creates — the phase forwarder needs it to ask the
    // oracle for this step's sub-agent rows (it only exists once `on_ready`
    // fires, a few seconds in).
    let live_sid: std::sync::Arc<std::sync::Mutex<Option<Id>>> = Default::default();
    let sid_slot = live_sid.clone();
    // R1.3: sub-agents ARE allowed; the appended block is a completion protocol
    // (finish every task you launched, handoff LAST) and the turn oracle holds
    // the step open until the parent's pending set is empty.
    let guarded = format!("{prompt}{WF_STEP_RULES}");
    // R5: EVERY agent-backed step gets the no-real-progress trip (retryable via
    // resolve_retry). Heavy kinds are safe under a short threshold because the
    // clock only counts genuine stalls — transcript/rollout growth and child
    // CPU reset it (see agent_session), so a long review or quiet test run
    // never false-trips while a codex spinner can no longer mask a hang.
    let stall = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("wf_step_stall_secs")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .map_or(WF_STEP_STALL_DEFAULT, Duration::from_secs);
    let stuck_after = if stall.is_zero() {
        crate::agent_session::STUCK_IDLE
    } else {
        stall
    };
    // A leftover handoff from a PRIOR attempt/run of this step would complete
    // the new turn instantly with stale content — clear it before submitting.
    if let Some(df) = &done_file {
        let _ = std::fs::remove_file(df);
    }
    // Steps whose reply must come from the transcript (goals eval, PR draft,
    // canvas) never name a handoff file — the oracle completes them on the turn
    // ending with nothing pending, and the "handoff missing" ⚠ line would be a
    // lie for them.
    let expects_handoff = done_file.is_some();
    // R1: the turn ORACLE decides completion for workflow steps — a claude
    // `end_turn` while sub-agents are still working is not "done". Phases stream
    // back on `phase_tx`; the accepted rule rides `outcome_tx`.
    let (phase_tx, mut phase_rx) = tokio::sync::mpsc::unbounded_channel::<Phase>();
    let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel::<CompleteVia>();
    let turn = crate::agent_session::run_session_turn_with(
        ctx,
        ws,
        user,
        None,
        &title,
        cwd,
        provider,
        meta,
        &guarded,
        stuck_after,
        crate::agent_session::TurnOpts {
            done_file,
            done_file_validator: None,
            transcript_fallback: false,
            cancel: None,
            // Silence is not evidence that delegated work has completed.
            quiet_done: None,
            // The retry spawns a fresh session; don't leave the stuck one
            // alive (its spinner defeats the idle-suspend sweep).
            kill_on_stall: true,
            oracle: true,
            phase_tx: Some(phase_tx),
            outcome_tx: Some(outcome_tx),
        },
        move |id| {
            let _ = tx.send(AgentAssociation::Session(id.to_string()));
            if let Ok(mut slot) = sid_slot.lock() {
                *slot = Some(id.clone());
            }
        },
    );
    tokio::pin!(turn);
    // Phase → log/activity forwarder. Runs BESIDE the turn (not spawned) so it
    // can read the live session without cloning the whole context.
    let mut feed = PhaseFeed::new(provider);
    let result = loop {
        tokio::select! {
            biased;
            Some(p) = phase_rx.recv() => {
                let sid = live_sid.lock().ok().and_then(|g| g.clone());
                feed.on_phase(ctx, sid.as_ref(), p, log_tx, activity_tx).await;
            }
            r = &mut turn => break r,
        }
    };
    while let Ok(p) = phase_rx.try_recv() {
        let sid = live_sid.lock().ok().and_then(|g| g.clone());
        feed.on_phase(ctx, sid.as_ref(), p, log_tx, activity_tx)
            .await;
    }
    let out = result.map_err(|e| e.0)?;
    // How the oracle accepted the turn — `Err` means the sender was dropped
    // without a send (E2E short-circuit / legacy path): nothing to report.
    if let Ok(via) = outcome_rx.await {
        if expects_handoff || via != CompleteVia::IdleTurnNoHandoff {
            let _ = log_tx.send(feed.completion_line(via));
        }
    }
    Ok(out)
}

/// Turns the oracle's phase stream into the node's kept log lines and the live
/// `activity` snapshot.
///
/// Emission rule (R5.5): a line is logged only when the phase IDENTITY changes
/// (`phase_key` ignores a countdown's remaining time), and an identity re-logs
/// at most once per 5 s — so `⚙ working` appears once, the countdown phases
/// once on entry, and `🧩 sub-agents: …` on every count change.
pub(super) struct PhaseFeed {
    provider: String,
    /// Seeded with `Booting`: `run_node_agent` already logged its `⏳` line.
    last_key: Option<(u8, usize, usize)>,
    logged_at: HashMap<(u8, usize, usize), Instant>,
    /// `📄 handoff file written` is logged once, the first time a phase implies
    /// the file exists.
    handoff_logged: bool,
    last_probe: Option<Instant>,
    subagents: Vec<SubagentActivity>,
    last_progress_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Remembered for the completion line's `{n}` / `{id}`.
    pub(super) pending: usize,
    pub(super) running_id: Option<String>,
    /// Incremental reader behind the sub-agent snapshot (see [`StepProbe`]).
    probe: Option<StepProbe>,
}

impl PhaseFeed {
    pub(super) fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_string(),
            last_key: Some(turn_oracle::phase_key(&Phase::Booting)),
            logged_at: HashMap::new(),
            handoff_logged: false,
            last_probe: None,
            subagents: vec![],
            last_progress_at: None,
            pending: 0,
            running_id: None,
            probe: None,
        }
    }

    async fn on_phase(
        &mut self,
        ctx: &ServerCtx,
        sid: Option<&Id>,
        p: Phase,
        log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
        activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
    ) {
        let key = turn_oracle::phase_key(&p);
        if self.last_key == Some(key) {
            return;
        }
        self.last_key = Some(key);
        // Refresh the sub-agent snapshot (one transcript read, ≤ once per 5s).
        // A `Subagents` phase gets the read unconditionally: its identity only
        // changes when a count changes (≤ 2 × pending ids over a step), and the
        // snapshot it is annotated with — and the rows the run view shows —
        // would otherwise describe the PREVIOUS count.
        if matches!(p, Phase::Subagents { .. })
            || self
                .last_probe
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(5))
        {
            self.last_probe = Some(Instant::now());
            if let Some((subs, stamp)) = step_activity_probe(ctx, sid, &mut self.probe).await {
                self.running_id = subs
                    .iter()
                    .find(|s| s.status == turn_oracle::SubStatus::Running)
                    .map(|s| s.id.clone());
                self.subagents = subs.iter().map(to_subagent_activity).collect();
                self.last_progress_at = stamp.map(chrono::DateTime::<chrono::Utc>::from);
            }
        }
        let mut line = turn_oracle::phase_line(&p, &self.provider);
        // Name the description that just completed, when we know it.
        if let Phase::Subagents { done, .. } = &p {
            if *done > 0 {
                if let Some(d) = self
                    .subagents
                    .iter()
                    .rfind(|s| s.status == "done")
                    .map(|s| s.description.clone())
                {
                    line.push_str(&format!(" ({d} ✓)"));
                }
            }
        }
        // `hold_reason` is for a step the engine is deliberately HOLDING while
        // it looks idle. A handoff written with nothing pending is just the turn
        // finishing, so it is not one of them.
        let holds = matches!(
            p,
            Phase::IdleConfirming { .. }
                | Phase::HandoffMissingGrace { .. }
                | Phase::HandoffWrittenWaiting { pending: 1.. }
                | Phase::BashLinger { .. }
        );
        self.pending = match &p {
            Phase::HandoffWrittenWaiting { pending } | Phase::BashLinger { pending, .. } => {
                *pending
            }
            Phase::Subagents { running, .. } => *running,
            _ => 0,
        };
        // The handoff file is on disk the moment either of these phases holds.
        if !self.handoff_logged
            && matches!(
                p,
                Phase::IdleConfirming { .. } | Phase::HandoffWrittenWaiting { .. }
            )
        {
            self.handoff_logged = true;
            let _ = log_tx.send("📄 handoff file written".to_string());
        }
        // 5s floor per identity: A→B→A inside the window logs A once.
        let recent = self
            .logged_at
            .get(&key)
            .is_some_and(|t| t.elapsed() < Duration::from_secs(5));
        if !recent {
            self.logged_at.insert(key, Instant::now());
            let _ = log_tx.send(line.clone());
        }
        let _ = activity_tx.send(bound_activity(NodeActivity {
            phase: phase_text(&line),
            updated_at: chrono::Utc::now(),
            last_progress_at: self.last_progress_at,
            pending_tasks: self.pending as u32,
            subagents: self.subagents.clone(),
            hold_reason: holds.then(|| line.clone()),
        }));
    }

    /// The `✓`/`⚠` line that closes an accepted step (design §4, verbatim).
    pub(super) fn completion_line(&self, via: CompleteVia) -> String {
        match via {
            CompleteVia::HandoffAndIdleTurn => "✓ step complete (handoff + idle turn)".into(),
            CompleteVia::CodexTaskComplete => {
                "✓ step complete (codex task_complete + handoff)".into()
            }
            CompleteVia::IdleTurnNoHandoff => {
                "⚠ handoff file missing — accepted the agent's final reply after 90s idle".into()
            }
            CompleteVia::HandoffLingerCap => format!(
                "⚠ handoff written; {} sub-agent{} never reported after 15m — moving on",
                self.pending,
                if self.pending == 1 { "" } else { "s" }
            ),
            CompleteVia::BashLingerCap => format!(
                "⚠ background task {} never finished after 15m — moving on",
                self.running_id.as_deref().unwrap_or("(unknown)")
            ),
            CompleteVia::QuietFallback => {
                "⚠ legacy completion record: silence fallback (no longer accepted)".into()
            }
        }
    }
}

/// A phase log line without its glyph — what `NodeActivity.phase` carries.
pub(super) fn phase_text(line: &str) -> String {
    line.trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim()
        .to_string()
}

fn to_subagent_activity(s: &turn_oracle::SubagentInfo) -> SubagentActivity {
    SubagentActivity {
        id: s.id.chars().take(64).collect(),
        description: s.description.chars().take(80).collect(),
        status: match s.status {
            turn_oracle::SubStatus::Running => "running",
            turn_oracle::SubStatus::Done => "done",
            turn_oracle::SubStatus::Failed => "failed",
        }
        .to_string(),
        started_at: s.started_at.map(chrono::DateTime::<chrono::Utc>::from),
        finished_at: s.finished_at.map(chrono::DateTime::<chrono::Utc>::from),
    }
}

/// The running step's sub-agents + newest child-file mtime, straight from the
/// oracle. `None` until the session exists (or for a provider with no
/// transcript) — the snapshot then simply carries no rows.
///
/// Each probe used to re-read and re-parse the WHOLE step transcript on the
/// runtime (once per sub-agent start/finish — ~40 × 40 ms on a 30 MB step). The
/// [`StepProbe`] kept in `slot` reads only what was appended since the last
/// probe and re-reads only new/changed sidecars, off the runtime.
async fn step_activity_probe(
    ctx: &ServerCtx,
    sid: Option<&Id>,
    slot: &mut Option<StepProbe>,
) -> Option<(
    Vec<turn_oracle::SubagentInfo>,
    Option<std::time::SystemTime>,
)> {
    let sid = sid?;
    let s = ctx.manager.get(sid).await.ok()?;
    if s.provider != "claude" {
        return None;
    }
    let psid = s.provider_session_id.clone()?;
    let mut probe = match slot.take() {
        Some(p) if &p.sid == sid && p.psid == psid => p,
        _ => {
            let (sid, cwd) = (sid.clone(), s.cwd.clone());
            crate::offload::blocking(move || StepProbe::new(sid, &cwd, psid)).await
        }
    };
    let (probe, out) = crate::offload::blocking(move || {
        let out = probe.run();
        (probe, out)
    })
    .await;
    *slot = Some(probe);
    out
}

/// Per-step incremental state for [`step_activity_probe`].
struct StepProbe {
    sid: Id,
    psid: String,
    proj: std::path::PathBuf,
    main: std::path::PathBuf,
    tail: turn_oracle::ClaudeTail,
    metas: turn_oracle::SubagentMetaCache,
}

impl StepProbe {
    /// Resolve the transcript paths. Blocking (canonicalize).
    fn new(sid: Id, cwd: &str, psid: String) -> Self {
        // claude symlink-resolves the spawn cwd for its transcript dir.
        let cwd = std::fs::canonicalize(cwd)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| cwd.to_string());
        let proj = otto_orchestrator::claude_pty::project_dir(&cwd);
        let main = proj.join(format!("{psid}.jsonl"));
        Self {
            sid,
            tail: turn_oracle::ClaudeTail::new(&main),
            psid,
            proj,
            main,
            metas: turn_oracle::SubagentMetaCache::default(),
        }
    }

    /// One probe. Blocking IO (only the appended bytes + changed sidecars).
    fn run(
        &mut self,
    ) -> Option<(
        Vec<turn_oracle::SubagentInfo>,
        Option<std::time::SystemTime>,
    )> {
        let scan = self.tail.poll()?;
        let subdir = self.proj.join(&self.psid).join("subagents");
        let stamp = turn_oracle::progress_stamp(&self.main, Some(&subdir), None);
        let subs = turn_oracle::subagents_cached(&self.proj, &self.psid, &scan, &mut self.metas);
        Some((subs, stamp))
    }
}

/// Parse the E2E sub-agent sentinel from a step prompt:
/// `OTTO_E2E_SUBAGENTS: <n>` (1..=40) with an optional ` hold_ms=<ms>`
/// (1000..=60000) on the same line; without it the hold is
/// `OTTO_E2E_STEP_HOLD_MS` (default 4000 ms).
pub(super) fn e2e_subagents(prompt: &str) -> Option<(usize, Duration)> {
    let line = prompt
        .lines()
        .find(|l| l.trim_start().starts_with("OTTO_E2E_SUBAGENTS:"))?;
    let rest = line.trim_start().strip_prefix("OTTO_E2E_SUBAGENTS:")?;
    let n: usize = rest.split_whitespace().next()?.parse().ok()?;
    if !(1..=40).contains(&n) {
        return None;
    }
    let hold = rest
        .split_whitespace()
        .find_map(|t| t.strip_prefix("hold_ms="))
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| (1000..=60_000).contains(ms))
        .or_else(|| {
            std::env::var("OTTO_E2E_STEP_HOLD_MS")
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
        })
        .unwrap_or(4000);
    Some((n, Duration::from_millis(hold)))
}

/// The synthetic phase/activity sequence the E2E daemon emits for a step whose
/// prompt carries the sub-agent sentinel — the real oracle never runs there (no
/// CLI is spawned). Mirrors a real run's line order; the canned reply's own
/// `agent turn complete` follows once `run_session_turn_with` returns.
async fn e2e_subagent_sequence(
    n: usize,
    hold: Duration,
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
) {
    let half = hold / 2;
    let _ = log_tx.send("✉ prompt accepted".to_string());
    let started = chrono::Utc::now();
    let mut subs: Vec<SubagentActivity> = (1..=n)
        .map(|i| SubagentActivity {
            id: format!("e2e-agent-{i}"),
            description: format!("E2E sub-agent {i}"),
            status: "running".into(),
            started_at: Some(started),
            finished_at: None,
        })
        .collect();
    let _ = log_tx.send(format!("🧩 sub-agents: {n} running · 0 done"));
    let _ = activity_tx.send(bound_activity(NodeActivity {
        phase: format!("sub-agents: {n} running · 0 done"),
        updated_at: chrono::Utc::now(),
        last_progress_at: Some(started),
        pending_tasks: n as u32,
        subagents: subs.clone(),
        hold_reason: None,
    }));
    tokio::time::sleep(half).await;
    let finished = chrono::Utc::now();
    for s in &mut subs {
        s.status = "done".into();
        s.finished_at = Some(finished);
    }
    let _ = log_tx.send(format!("🧩 sub-agents: 0 running · {n} done"));
    let _ = activity_tx.send(bound_activity(NodeActivity {
        phase: format!("sub-agents: 0 running · {n} done"),
        updated_at: finished,
        last_progress_at: Some(finished),
        pending_tasks: 0,
        subagents: subs,
        hold_reason: None,
    }));
    tokio::time::sleep(hold.saturating_sub(half)).await;
    let _ = log_tx.send("📄 handoff file written".to_string());
    let _ = log_tx.send("✓ step complete (handoff + idle turn)".to_string());
}
