//! Review agents on runtime threads (runtime.md §7.2): the scope assessment,
//! every reviewer and the summarizer run one turn each through the shared
//! [`StepRunner`], with the same prompt, files, confinement and findings
//! contract as the terminal path in [`crate::session`]. Only the inner
//! runner changes; the recovery wrapper keeps its retries and sibling skip.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use otto_agent_run::agent_run::{FailReason, RunOutcome};
use otto_agent_run::step::{StepErrorClass, StepEvent, StepOutcome, StepRunner, StepTurn};
use otto_core::cancel_signal::CancelSignal;
use otto_core::domain::{ReviewAgentState, User, Workspace};
use otto_state::ReviewsRepo;
use serde_json::Value;

use crate::session::SharedStates;

/// Which runner a review's agents use: the workflow run's latched choice,
/// else `OTTO_WF_STEP_RUNNER`, else the `wf_step_runner` setting —
/// `threads` unless one of them says `pty`.
pub async fn uses_threads(latched: Option<&str>, settings: &otto_state::SettingsRepo) -> bool {
    let chosen = match latched {
        Some(r) => r.to_string(),
        None => match std::env::var("OTTO_WF_STEP_RUNNER") {
            Ok(r) if !r.trim().is_empty() => r,
            _ => settings
                .get("wf_step_runner")
                .await
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
        },
    };
    chosen.trim() != "pty"
}

/// The retention pair (`workflow_thread_retention_success_hours` /
/// `_failed_hours`, 24 / 168): how long a review's closed threads are kept.
pub async fn keep_periods(
    settings: &otto_state::SettingsRepo,
) -> (chrono::Duration, chrono::Duration) {
    let hours = |key: &'static str, default: i64| async move {
        let h = settings
            .get(key)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_i64());
        chrono::Duration::hours(h.filter(|h| *h >= 0).unwrap_or(default))
    };
    (
        hours("workflow_thread_retention_success_hours", 24).await,
        hours("workflow_thread_retention_failed_hours", 168).await,
    )
}

/// The meta a workflow run's review threads carry (agreed `meta.work`
/// shape): `{run_id, node_id, work: {kind: "workflow_run", id, workflow_id}}`
/// — `node_id` without its `#review.<n>` target suffix.
pub fn run_meta(run: &otto_core::workflows::WorkflowRun, node_id: &str) -> Value {
    serde_json::json!({
        "run_id": run.id,
        "node_id": node_id.split('#').next().unwrap_or(node_id),
        "work": {"kind": "workflow_run", "id": run.id, "workflow_id": run.workflow_id},
    })
}

/// The review's [`Threads`] over `runner`, or `None` when the review runs
/// on the terminal path ([`uses_threads`]); `cfg` carries the run's latch
/// and thread meta.
pub async fn threads(
    runner: Arc<dyn StepRunner>,
    has_driver: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    cfg: Option<&otto_core::api::ReviewConfig>,
    settings: &otto_state::SettingsRepo,
) -> Option<Threads> {
    if !uses_threads(cfg.and_then(|c| c.runner.as_deref()), settings).await {
        return None;
    }
    let (keep_success, keep_failed) = keep_periods(settings).await;
    Some(Threads {
        runner,
        has_driver,
        meta: cfg.map(|c| c.thread_meta.clone()).unwrap_or_default(),
        keep_failed,
        keep_success,
    })
}

/// What the summarizer's thread turn needs beside its attempt: the brief,
/// its config, workspace, user and checkout.
pub type SummarizerBrief<'a> = (
    &'a str,
    &'a otto_core::domain::ReviewAgentCfg,
    &'a Workspace,
    &'a User,
    &'a str,
);

/// The thread runner a review's agents use, and what every thread carries.
#[derive(Clone)]
pub struct Threads {
    pub runner: Arc<dyn StepRunner>,
    /// Whether a provider slug has a runtime driver; one without runs on the
    /// terminal path.
    pub has_driver: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    /// `{run_id, node_id, work}` inside a workflow run (merged into every
    /// thread's meta); `{}` for a Git-page review, whose `review_id` names it.
    pub meta: Value,
    /// Kept this long after it closes; the review's end shortens a finished
    /// review's threads to [`Self::keep_success`].
    pub keep_failed: chrono::Duration,
    pub keep_success: chrono::Duration,
}

impl Threads {
    pub fn retain_failed(&self) -> chrono::DateTime<Utc> {
        Utc::now() + self.keep_failed
    }
    pub fn retain_success(&self) -> chrono::DateTime<Utc> {
        Utc::now() + self.keep_success
    }
    /// A finished review's threads are kept for the success period from now.
    pub async fn finished(&self, rows: &[ReviewAgentState]) {
        let ids: Vec<otto_core::Id> = rows
            .iter()
            .filter(|r| r.thread)
            .filter_map(|r| r.session_id.clone())
            .collect();
        if !ids.is_empty() {
            self.runner
                .set_retain_until(&ids, self.retain_success())
                .await;
        }
    }

    /// Close a thread now, kept for the failed period.
    pub async fn close(&self, id: &otto_core::Id) {
        self.runner.close(id, self.retain_failed()).await;
    }

    /// The summarizer's (and verifier's) turn on a thread: `attempt`'s brief
    /// and result file as the handoff — the same files as the terminal turn.
    /// `ready` hears the thread id first; the text is the result file once
    /// it validates ([`crate::summarizer::valid_result`]).
    pub async fn summarizer_turn(
        &self,
        attempt: &crate::summarizer::Attempt,
        (brief, cfg, ws, user, cwd): SummarizerBrief<'_>,
        ready: tokio::sync::oneshot::Sender<String>,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<String, String> {
        let mut meta = attempt.meta.clone();
        self.meta_into(&mut meta);
        let turn = StepTurn {
            user_id: user.id.clone(),
            workspace_id: ws.id.clone(),
            provider: attempt.provider.clone(),
            model: Some(cfg.model.trim().to_string()).filter(|m| !m.is_empty()),
            effort: Some(cfg.effort.trim().to_string()).filter(|e| !e.is_empty()),
            cwd: PathBuf::from(cwd),
            title: "Review summarizer".into(),
            meta,
            prompt: attempt.prompt(brief),
            continue_thread: None,
            handoff: Some(attempt.result_path()),
            read_only: true,
            timeout: None,
            no_progress: None,
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let runner = Arc::clone(&self.runner);
        let run = with_cancel(cancel, |signal| async move {
            runner.run(turn, tx, signal).await
        });
        tokio::pin!(run);
        let mut ready = Some(ready);
        let result = loop {
            tokio::select! {
                r = &mut run => break r,
                Some(ev) = rx.recv() => {
                    if let (StepEvent::Started { thread_id, .. }, true) = (ev, ready.is_some()) {
                        let _ = ready.take().expect("checked").send(thread_id.to_string());
                    }
                }
            }
        };
        // A turn that settles in the same poll as its start still publishes.
        while let (Ok(ev), true) = (rx.try_recv(), ready.is_some()) {
            if let StepEvent::Started { thread_id, .. } = ev {
                let _ = ready.take().expect("checked").send(thread_id.to_string());
            }
        }
        let o = result.map_err(|e| e.to_string())?;
        let text = std::fs::read_to_string(attempt.result_path()).ok();
        match text.filter(|t| crate::summarizer::valid_result(t)) {
            Some(text) => Ok(text),
            None => Err(o
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "the summarizer wrote no valid result file".into())),
        }
    }

    /// Await a summarizer thread that survived a daemon restart in its holder,
    /// using its persisted `result_path` as the handoff target.
    pub async fn resume_summarizer_turn(
        &self,
        thread_id: &otto_core::Id,
        result_path: &Path,
        (brief, cfg, ws, user, cwd): SummarizerBrief<'_>,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<String, String> {
        let mut meta = serde_json::json!({
            "source": "review_summarizer",
            "read_only": true,
            "project_settings": false,
        });
        self.meta_into(&mut meta);
        let provider = cfg.provider.trim();
        if provider.is_empty() {
            return Err("summarizer provider is not configured".into());
        }
        let turn = StepTurn {
            user_id: user.id.clone(),
            workspace_id: ws.id.clone(),
            provider: provider.to_string(),
            model: Some(cfg.model.trim().to_string()).filter(|m| !m.is_empty()),
            effort: Some(cfg.effort.trim().to_string()).filter(|e| !e.is_empty()),
            cwd: PathBuf::from(cwd),
            title: "Review summarizer".into(),
            meta,
            prompt: brief.to_string(),
            continue_thread: None,
            handoff: Some(result_path.to_path_buf()),
            read_only: true,
            timeout: None,
            no_progress: None,
        };
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let runner = Arc::clone(&self.runner);
        let tid = thread_id.clone();
        let o = with_cancel(cancel, |signal| async move {
            runner.resume(&tid, 0, &turn, tx, signal).await
        })
        .await
        .map_err(|e| e.to_string())?;
        let text = tokio::fs::read_to_string(result_path).await.ok();
        match text.filter(|t| crate::summarizer::valid_result(t)) {
            Some(text) => Ok(text),
            None => Err(o
                .error
                .map(|e| e.message)
                .unwrap_or_else(|| "the resumed summarizer wrote no valid result file".into())),
        }
    }

    /// Drive a held summarizer thread through `summarizer::drive` and clean up
    /// its persisted `otto-summary-*` temp directory afterward.
    pub async fn drive_held_summarizer<C>(
        &self,
        thread_id: &otto_core::Id,
        result_path: &Path,
        brief: SummarizerBrief<'_>,
        deadline: tokio::time::Instant,
        cancelled: C,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<String, String>
    where
        C: std::future::Future<Output = ()>,
    {
        if tokio::time::Instant::now() >= deadline {
            return Err(crate::summarizer::DEADLINE_EXCEEDED.into());
        }
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let _ = ready_tx.send(thread_id.to_string());
        let turn = self.resume_summarizer_turn(thread_id, result_path, brief, cancel);
        let stop = |sid: String| async move { self.close(&sid).await };
        let res = crate::summarizer::drive(
            turn,
            ready_rx,
            deadline,
            cancelled,
            |_| async { Ok(()) },
            stop,
        )
        .await;
        crate::summarizer::cleanup_summary_dir(result_path).await;
        res
    }

    /// Drive a fresh thread-backed summarizer attempt, persisting its handoff
    /// dir across restarts and cleaning it up once the attempt finishes here.
    pub async fn drive_fresh_summarizer<C, P, PF>(
        &self,
        attempt: &mut crate::summarizer::Attempt,
        brief: SummarizerBrief<'_>,
        deadline: tokio::time::Instant,
        cancelled: C,
        publish: P,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<String, String>
    where
        C: std::future::Future<Output = ()>,
        P: FnOnce(String) -> PF,
        PF: std::future::Future<Output = Result<(), String>>,
    {
        attempt.persist_for_restart();
        attempt.meta["result_path"] = serde_json::json!(attempt.result_path().to_string_lossy());
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let turn = self.summarizer_turn(attempt, brief, ready_tx, cancel);
        let stop = |sid: String| async move { self.close(&sid).await };
        let res =
            crate::summarizer::drive(turn, ready_rx, deadline, cancelled, publish, stop).await;
        attempt.cleanup();
        res
    }

    pub(crate) fn meta_into(&self, meta: &mut Value) {
        if let (Some(m), Some(extra)) = (meta.as_object_mut(), self.meta.as_object()) {
            for (k, v) in extra {
                m.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Extract workflow linkage fields (`run_id`, `node_id`, `work`) from an
/// existing review thread's `meta` for `ReviewConfig::thread_meta`.
pub fn extract_workflow_meta(meta: &Value) -> Option<Value> {
    let kept: serde_json::Map<_, _> = ["run_id", "node_id", "work"]
        .into_iter()
        .filter_map(|k| meta.get(k).map(|v| (k.into(), v.clone())))
        .collect();
    (!kept.is_empty()).then_some(Value::Object(kept))
}

/// A review cancel flag as the runner's [`CancelSignal`], for as long as
/// `work` runs (the flag is a plain `AtomicBool` in the review registries).
pub async fn with_cancel<F, T>(
    flag: Option<&Arc<AtomicBool>>,
    work: impl FnOnce(CancelSignal) -> F,
) -> T
where
    F: std::future::Future<Output = T>,
{
    let signal = CancelSignal::new();
    let fut = work(signal.clone());
    tokio::pin!(fut);
    loop {
        tokio::select! {
            out = &mut fut => return out,
            _ = tokio::time::sleep(Duration::from_millis(200)) => {
                if flag.is_some_and(|f| f.load(Ordering::SeqCst)) {
                    signal.cancel();
                }
            }
        }
    }
}

/// The row's live state from one runner event (runtime.md §7.2 step 3):
/// admission waits stay `pending` with the wait's words; a person's wait is
/// `waiting` "needs you: …" (today's wording).
pub fn apply_event(row: &mut ReviewAgentState, ev: &StepEvent) -> bool {
    match ev {
        StepEvent::Waiting(w) => {
            row.status = "pending".into();
            row.note = w.text.clone();
        }
        StepEvent::Started { thread_id, .. } => {
            row.status = "running".into();
            row.session_id = Some(thread_id.to_string());
            row.thread = true;
            row.started_at = Some(Utc::now());
            row.finished_at = None;
            row.note = String::new();
        }
        StepEvent::NeedsYou(a) => {
            row.status = "waiting".into();
            row.note = format!("needs you: {}", a.text);
        }
        StepEvent::Resumed { .. } => {
            row.status = "running".into();
            row.note = String::new();
        }
        _ => return false,
    }
    true
}

/// A review's rows as the review step's sub-threads (`NodeRunState.children`):
/// the scope assessment, every reviewer and the summarizer, ids
/// `<review_id>.<row>`; `thread_id` is set when the row ran on a thread.
pub fn child_rows(
    review_id: &str,
    rows: &[ReviewAgentState],
) -> Vec<otto_core::workflows::ChildThread> {
    let some = |s: &str| Some(s.to_string()).filter(|s| !s.trim().is_empty());
    rows.iter()
        .enumerate()
        .map(|(i, r)| otto_core::workflows::ChildThread {
            id: format!("{review_id}.{i}"),
            thread_id: r.session_id.clone().filter(|_| r.thread),
            label: r.name.clone(),
            provider: r.provider.clone(),
            model: some(&r.model),
            effort: some(&r.effort),
            status: match r.status.as_str() {
                "done" => "success",
                "waiting" => "needs_you",
                "running" | "error" | "skipped" => r.status.as_str(),
                _ => "pending",
            }
            .into(),
            started_at: r.started_at,
            ended_at: r.finished_at,
            note: some(&r.note),
            usage: Default::default(),
        })
        .collect()
}

/// Replace review `review_id`'s rows in a step's children (a step can review
/// several targets, one review each); `true` when anything changed.
pub fn merge_children(
    children: &mut Vec<otto_core::workflows::ChildThread>,
    review_id: &str,
    rows: Vec<otto_core::workflows::ChildThread>,
) -> bool {
    let prefix = format!("{review_id}.");
    let before = children.clone();
    children.retain(|c| !c.id.starts_with(&prefix));
    children.extend(rows);
    *children != before
}

/// Why a settled turn left no findings, in the wrapper's terms (its notes
/// and retry rules key on [`FailReason`]).
pub fn fail_reason(o: &StepOutcome) -> FailReason {
    match o.error.as_ref().map(|e| e.class) {
        Some(StepErrorClass::Stalled) => FailReason::Stuck,
        Some(StepErrorClass::Timeout) => FailReason::Timeout,
        Some(StepErrorClass::Spawn) => FailReason::CreateFailed,
        Some(StepErrorClass::Canceled) => FailReason::Stopped,
        _ => FailReason::Exited,
    }
}

/// The findings an attempt left: the findings file, else the answer text
/// (the fallback the terminal watch has). `None` = no usable result.
pub fn result_text(findings: &Path, o: &StepOutcome) -> Option<String> {
    let ok = |t: &String| crate::session::parse_findings_array(t).is_some();
    std::fs::read_to_string(findings)
        .ok()
        .filter(ok)
        .or_else(|| Some(o.reply.clone()).filter(ok))
}

/// Persist one row of `states` (never the whole array).
async fn persist(
    states: &SharedStates,
    reviews: &ReviewsRepo,
    review_id: &str,
    i: usize,
    f: impl FnOnce(&mut ReviewAgentState) -> bool,
) {
    let row = {
        let mut g = states.lock().await;
        g.get_mut(i).and_then(|s| f(s).then(|| s.clone()))
    };
    if let Some(row) = row {
        let _ = reviews.set_agent_at(&review_id.to_string(), i, &row).await;
    }
}

/// One review agent attempt on a fresh thread — the twin of
/// [`crate::session::run_agent_session`], same files and prompt.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_thread(
    threads: &Threads,
    reviews: &ReviewsRepo,
    states: &SharedStates,
    ws: &Workspace,
    user: &User,
    provider: &str,
    model: &str,
    cwd: &str,
    review_id: &str,
    agent_index: usize,
    base_prompt: &str,
    grace: Duration,
    extra_dirs: Option<Value>,
    lens_slugs: &[String],
    prompt_file: Option<&Path>,
    cancel: Option<&Arc<AtomicBool>>,
    failure: &std::sync::Mutex<Option<String>>,
) -> RunOutcome {
    let path = crate::session::findings_path(review_id, agent_index);
    let _ = std::fs::remove_file(&path);
    crate::session::remove_lens_findings_files(review_id, agent_index);
    let prompt =
        crate::session::typed_reviewer_prompt(base_prompt, &path.to_string_lossy(), prompt_file);
    let mut meta =
        crate::session::review_session_meta(review_id, agent_index, !lens_slugs.is_empty());
    if let Some(dirs) = extra_dirs {
        meta["extra_dirs"] = dirs;
    }
    threads.meta_into(&mut meta);
    let (title, effort) = {
        let g = states.lock().await;
        let row = g.get(agent_index);
        (
            row.map(|s| s.name.clone()).unwrap_or_default(),
            row.map(|s| s.effort.clone()).unwrap_or_default(),
        )
    };
    let turn = StepTurn {
        user_id: user.id.clone(),
        workspace_id: ws.id.clone(),
        provider: provider.to_string(),
        model: Some(model.trim().to_string()).filter(|m| !m.is_empty()),
        effort: Some(effort).filter(|e| !e.trim().is_empty()),
        cwd: PathBuf::from(cwd),
        title,
        meta,
        prompt,
        continue_thread: None,
        handoff: Some(path.clone()),
        read_only: true,
        timeout: None,
        no_progress: Some(grace),
    };
    let outcome = drive(
        threads,
        turn,
        reviews,
        states,
        review_id,
        agent_index,
        cancel,
        None,
    )
    .await;
    let Some(o) = outcome else {
        return RunOutcome::failed(None, FailReason::CreateFailed);
    };
    *failure.lock().unwrap_or_else(|p| p.into_inner()) =
        o.error.as_ref().map(|e| e.message.clone());
    match result_text(&path, &o) {
        Some(raw) => RunOutcome::ok(raw, o.thread_id),
        None => RunOutcome::failed(Some(o.thread_id.clone()), fail_reason(&o)),
    }
}

/// Run `turn` to settle with the row following its events, then close the
/// thread (kept for the failed period; [`Threads::keep_success`] later).
/// `None` = Otto's own fault (the store), logged.
#[allow(clippy::too_many_arguments)]
pub async fn drive(
    threads: &Threads,
    turn: StepTurn,
    reviews: &ReviewsRepo,
    states: &SharedStates,
    review_id: &str,
    agent_index: usize,
    cancel: Option<&Arc<AtomicBool>>,
    // After a restart: the thread to await instead of a fresh one.
    resume: Option<otto_core::Id>,
) -> Option<StepOutcome> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runner = Arc::clone(&threads.runner);
    let run = with_cancel(cancel, |signal| async move {
        match resume {
            Some(id) => runner.resume(&id, 0, &turn, tx, signal).await,
            None => runner.run(turn, tx, signal).await,
        }
    });
    tokio::pin!(run);
    let result = loop {
        tokio::select! {
            r = &mut run => break r,
            Some(ev) = rx.recv() => {
                persist(states, reviews, review_id, agent_index, |s| apply_event(s, &ev)).await;
            }
        }
    };
    while let Ok(ev) = rx.try_recv() {
        persist(states, reviews, review_id, agent_index, |s| {
            apply_event(s, &ev)
        })
        .await;
    }
    let o = match result {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(
                review = review_id,
                agent = agent_index,
                "review thread: {e}"
            );
            return None;
        }
    };
    threads
        .runner
        .close(&o.thread_id, threads.retain_failed())
        .await;
    persist(states, reviews, review_id, agent_index, |s| {
        s.finished_at = Some(Utc::now());
        true
    })
    .await;
    Some(o)
}

#[cfg(test)]
#[path = "thread_run_tests.rs"]
mod tests;
