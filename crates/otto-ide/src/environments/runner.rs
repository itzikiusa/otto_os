//! Background Test & Run execution state, SSE broadcast, and task runner (`execute_run_task`).

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::{broadcast, watch};

use crate::target::TargetManager;
use crate::types::{
    IdeRunConfig, IdeRunState, IdeRunStatus, IdeRunnerKind, IdeSshSpec, IdeTargetKind,
    IdeTargetSpec,
};

pub(super) use super::runner_cmd::{
    apply_runtime_profile_and_module_venv, build_runner_command, compile_go_run_step,
    compile_java_main_step, has_manual_classpath, is_default_go_command, resolve_run_cwd,
};
use super::runner_parsers::IncrementalTestTracker;
use super::runner_proc::{
    cleanup_remaining_process_group, kill_child_process_group, run_remote_before_launch,
    spawn_run_child, ChildProcessGroupGuard,
};
use super::runner_steps::{
    run_local_before_launch, unless_stopped, StepOutcome, BEFORE_LAUNCH_TIMEOUT,
};
use crate::line_reader::{spawn_line_drain, MAX_OUTPUT_LINE_BYTES};

pub(super) const MAX_FINISHED_RUNS: usize = 8;
pub(super) const MAX_RUN_OUTPUT_LINES: usize = 2_000;
pub(super) const MAX_RUN_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_FINISHED_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_SINGLE_LINE_BYTES: usize = 16 * 1024;

pub(super) fn evict_finished_runs(runs: &DashMap<String, Arc<ActiveRunHandle>>) {
    let mut finished: Vec<(String, String, u64)> = runs
        .iter()
        .filter_map(|e| {
            let (started_at, seq) = e.value().finished_sort_key()?;
            Some((e.key().clone(), started_at, seq))
        })
        .collect();
    if finished.len() <= MAX_FINISHED_RUNS {
        return;
    }
    finished.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));
    let evict_count = finished.len() - MAX_FINISHED_RUNS;
    for (id, _, _) in finished.into_iter().take(evict_count) {
        runs.remove(&id);
    }
}

pub(super) struct RunStateInner {
    id: String,
    title: String,
    runner: IdeRunnerKind,
    pub(super) status: IdeRunStatus,
    exit_code: Option<i32>,
    started_at: String,
    duration_ms: Option<u64>,
    output_seq: u64,
    output_bytes: usize,
    output: VecDeque<String>,
    parser: IncrementalTestTracker,
    pub(super) active_local_pid: Option<u32>,
    /// A coalesced output snapshot is scheduled for the run's watchers.
    flush_pending: bool,
}

impl RunStateInner {
    pub(super) fn new(
        id: String,
        title: String,
        runner: IdeRunnerKind,
        started_at: String,
    ) -> Self {
        Self {
            id,
            title,
            runner,
            status: IdeRunStatus::Running,
            exit_code: None,
            started_at,
            duration_ms: None,
            output_seq: 0,
            output_bytes: 0,
            output: VecDeque::new(),
            parser: IncrementalTestTracker::default(),
            active_local_pid: None,
            flush_pending: false,
        }
    }

    fn push_line(&mut self, mut line: String) {
        if line.len() > MAX_SINGLE_LINE_BYTES {
            let mut cut = MAX_SINGLE_LINE_BYTES;
            while cut > 0 && !line.is_char_boundary(cut) {
                cut -= 1;
            }
            line.truncate(cut);
            line.push_str("... [truncated]");
        }
        self.output_seq = self.output_seq.saturating_add(1);
        self.parser.ingest_line(self.runner, &line);

        let line_bytes = line.len();
        while self.output.len() >= MAX_RUN_OUTPUT_LINES
            || (self.output_bytes + line_bytes > MAX_RUN_OUTPUT_BYTES && !self.output.is_empty())
        {
            if let Some(removed) = self.output.pop_front() {
                self.output_bytes = self.output_bytes.saturating_sub(removed.len());
            }
        }
        self.output_bytes += line_bytes;
        self.output.push_back(line);
    }

    fn compact_on_finish(&mut self) {
        while self.output_bytes > MAX_FINISHED_OUTPUT_BYTES && self.output.len() > 1 {
            if let Some(removed) = self.output.pop_front() {
                self.output_bytes = self.output_bytes.saturating_sub(removed.len());
            }
        }
        self.output.shrink_to_fit();
        self.parser.compact_on_finish();
    }

    #[cfg(test)]
    pub(super) fn output_bytes(&self) -> usize {
        self.output_bytes
    }

    fn to_snapshot(&self, tail_lines: Option<usize>) -> IdeRunState {
        let output: Vec<String> = match tail_lines {
            Some(0) => Vec::new(),
            Some(n) if n < self.output.len() => {
                let skip = self.output.len() - n;
                self.output.iter().skip(skip).cloned().collect()
            }
            _ => self.output.iter().cloned().collect(),
        };
        IdeRunState {
            id: self.id.clone(),
            title: self.title.clone(),
            runner: self.runner,
            status: self.status,
            exit_code: self.exit_code,
            started_at: self.started_at.clone(),
            duration_ms: self.duration_ms,
            output_seq: self.output_seq,
            output,
            tests: self.parser.results.clone(),
            coverage_pct: self.parser.coverage_pct,
            coverage_summary: self.parser.coverage_summary.clone(),
        }
    }
}

pub(super) struct ActiveRunHandle {
    pub(super) seq: u64,
    pub(super) owner_user_id: String,
    pub(super) state: Mutex<RunStateInner>,
    pub(super) stop_tx: watch::Sender<bool>,
    pub(super) events_tx: broadcast::Sender<IdeRunState>,
}

impl ActiveRunHandle {
    pub(super) fn snapshot(&self) -> IdeRunState {
        self.snapshot_with_tail(None)
    }

    pub(super) fn snapshot_with_tail(&self, tail_lines: Option<usize>) -> IdeRunState {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .to_snapshot(tail_lines)
    }

    pub(super) fn finished_sort_key(&self) -> Option<(String, u64)> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if guard.status == IdeRunStatus::Running {
            None
        } else {
            Some((guard.started_at.clone(), self.seq))
        }
    }

    pub(super) fn stop(&self) -> IdeRunState {
        let _ = self.stop_tx.send(true);
        let snap = {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if guard.status == IdeRunStatus::Running {
                guard.status = IdeRunStatus::Stopped;
                guard.push_line("[Stopped by user]".into());
            }
            guard.to_snapshot(None)
        };
        if self.events_tx.receiver_count() > 0 {
            let _ = self.events_tx.send(snap.clone());
        }
        snap
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_run_task(
    runs: Arc<DashMap<String, Arc<ActiveRunHandle>>>,
    handle: Arc<ActiveRunHandle>,
    targets: TargetManager,
    target: IdeTargetSpec,
    ssh: Option<IdeSshSpec>,
    steps: Vec<(IdeRunConfig, BTreeMap<String, String>)>,
    netbridge: Option<Arc<crate::netbridge::NetBridgeLease>>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let started = Instant::now();
    let multi_step = steps.len() > 1;
    let mut last_exit = Some(0);
    if let Some(ref nb) = netbridge {
        append_run_line(&handle, nb.summary_banner().await);
    }

    for (idx, (cfg, mut env_map)) in steps.into_iter().enumerate() {
        if *stop_rx.borrow() {
            finish_run(
                &handle,
                IdeRunStatus::Stopped,
                None,
                started.elapsed(),
                None,
            );
            evict_finished_runs(&runs);
            return;
        }
        if multi_step {
            append_run_line(
                &handle,
                format!(
                    "── [{}/{}] {} ──",
                    idx + 1,
                    multi_step_label(&cfg),
                    cfg.name
                ),
            );
        }

        let cwd = resolve_run_cwd(&target, cfg.cwd.as_deref());
        apply_runtime_profile_and_module_venv(&cfg, &target, &cwd, &mut env_map);
        if let Some(ref nb) = netbridge {
            nb.apply_to_env(&mut env_map);
        }

        if let Some(pre) = cfg
            .before_launch
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            append_run_line(&handle, format!("$ {pre}"));
            let outcome = if target.kind == IdeTargetKind::Local {
                run_local_before_launch(
                    &handle,
                    pre,
                    &cwd,
                    &env_map,
                    &mut stop_rx,
                    BEFORE_LAUNCH_TIMEOUT,
                )
                .await
            } else {
                let remote = run_remote_before_launch(
                    &targets,
                    &target,
                    ssh.as_ref(),
                    &handle.owner_user_id,
                    pre,
                    &cwd,
                    &env_map,
                );
                match unless_stopped(&mut stop_rx, remote).await {
                    None => StepOutcome::Stopped,
                    Some(Ok(())) => StepOutcome::Done(()),
                    Some(Err(err)) => StepOutcome::Failed(err),
                }
            };
            match outcome {
                StepOutcome::Done(()) => {}
                StepOutcome::Failed(err) => {
                    end_run(
                        &runs,
                        &handle,
                        IdeRunStatus::Failed,
                        Some(1),
                        started,
                        Some(err),
                    );
                    return;
                }
                StepOutcome::Stopped => {
                    end_run(&runs, &handle, IdeRunStatus::Stopped, None, started, None);
                    return;
                }
            }
        }

        // Compile steps race Stop: dropping one kills its build's process group.
        let compiled = if target.kind == IdeTargetKind::Local
            && cfg.runner == IdeRunnerKind::JavaMain
            && !has_manual_classpath(&cfg)
        {
            let step = compile_java_main_step(&cfg, &target, &cwd, &env_map, &handle);
            Some(unless_stopped(&mut stop_rx, step).await)
        } else if target.kind == IdeTargetKind::Local
            && cfg.runner == IdeRunnerKind::GoRun
            && is_default_go_command(&cfg)
        {
            let step = compile_go_run_step(&cfg, &target, &cwd, &env_map);
            Some(unless_stopped(&mut stop_rx, step).await)
        } else {
            None
        };
        let (program, args) = match compiled {
            None => build_runner_command(&cfg, target.kind, &cwd, &env_map),
            Some(Some(Ok(built))) => built,
            Some(Some(Err(err))) => {
                end_run(
                    &runs,
                    &handle,
                    IdeRunStatus::Failed,
                    Some(1),
                    started,
                    Some(err),
                );
                return;
            }
            Some(None) => {
                end_run(&runs, &handle, IdeRunStatus::Stopped, None, started, None);
                return;
            }
        };

        append_run_line(
            &handle,
            format!("$ {} {}", program, args.join(" "))
                .trim()
                .to_string(),
        );

        let is_local = target.kind == IdeTargetKind::Local;
        let mut child = match spawn_run_child(
            &targets,
            &target,
            ssh.as_ref(),
            &handle.owner_user_id,
            &program,
            &args,
            &cwd,
            &env_map,
        )
        .await
        {
            Ok(c) => c,
            Err(err) => {
                finish_run(
                    &handle,
                    IdeRunStatus::Failed,
                    Some(127),
                    started.elapsed(),
                    Some(err),
                );
                evict_finished_runs(&runs);
                return;
            }
        };
        let pid = child.id();
        if is_local {
            if let Ok(mut guard) = handle.state.lock() {
                guard.active_local_pid = pid;
            }
        }
        let mut pg_guard = ChildProcessGroupGuard {
            pid,
            is_local,
            disarmed: false,
        };
        // Keep remote SSH stdin alive across `child.wait()` (`Child::wait` drops
        // `child.stdin` on first poll, which would trigger our remote EOF-kill watcher).
        let mut ssh_stdin = if is_local { None } else { child.stdin.take() };

        // Lossy, bounded readers drain both pipes to EOF: one invalid UTF-8 byte
        // or an endless line must never close the pipe under the program.
        let stdout_task = child.stdout.take().map(|out| {
            let h = Arc::clone(&handle);
            spawn_line_drain(out, MAX_OUTPUT_LINE_BYTES, move |l| append_run_line(&h, l))
        });
        let stderr_task = child.stderr.take().map(|err| {
            let h = Arc::clone(&handle);
            spawn_line_drain(err, MAX_OUTPUT_LINE_BYTES, move |l| append_run_line(&h, l))
        });

        let wait_res = tokio::select! {
            status = child.wait() => {
                drop(ssh_stdin.take());
                cleanup_remaining_process_group(pid, is_local).await;
                pg_guard.disarmed = true;
                Some(status)
            }
            _ = stop_rx.changed() => {
                drop(ssh_stdin.take());
                kill_child_process_group(&mut child, pid, is_local).await;
                pg_guard.disarmed = true;
                None
            }
        };
        if let Some(t) = stdout_task {
            let _ = tokio::time::timeout(Duration::from_millis(250), t).await;
        }
        if let Some(t) = stderr_task {
            let _ = tokio::time::timeout(Duration::from_millis(250), t).await;
        }

        match wait_res {
            None => {
                finish_run(
                    &handle,
                    IdeRunStatus::Stopped,
                    None,
                    started.elapsed(),
                    None,
                );
                evict_finished_runs(&runs);
                return;
            }
            Some(Ok(st)) => {
                let code = st.code().unwrap_or(if st.success() { 0 } else { 1 });
                last_exit = Some(code);
                if !st.success() {
                    finish_run(
                        &handle,
                        IdeRunStatus::Failed,
                        Some(code),
                        started.elapsed(),
                        None,
                    );
                    evict_finished_runs(&runs);
                    return;
                }
            }
            Some(Err(e)) => {
                finish_run(
                    &handle,
                    IdeRunStatus::Failed,
                    Some(1),
                    started.elapsed(),
                    Some(e.to_string()),
                );
                evict_finished_runs(&runs);
                return;
            }
        }
    }

    finish_run(
        &handle,
        IdeRunStatus::Passed,
        last_exit,
        started.elapsed(),
        None,
    );
    evict_finished_runs(&runs);
}

/// Finish the run and trim the finished-run list.
fn end_run(
    runs: &DashMap<String, Arc<ActiveRunHandle>>,
    handle: &ActiveRunHandle,
    status: IdeRunStatus,
    exit_code: Option<i32>,
    started: Instant,
    err_msg: Option<String>,
) {
    finish_run(handle, status, exit_code, started.elapsed(), err_msg);
    evict_finished_runs(runs);
}

fn multi_step_label(cfg: &IdeRunConfig) -> &'static str {
    match cfg.runner {
        IdeRunnerKind::GoBuild => "build",
        _ => "run",
    }
}

/// How long output lines gather before watchers get one snapshot of them: a
/// snapshot carries the 250-line tail and every test result, so one per line
/// would cost O(tail + tests) per line.
const RUN_OUTPUT_COALESCE: Duration = Duration::from_millis(100);

pub(super) fn append_run_line(handle: &Arc<ActiveRunHandle>, line: String) {
    let schedule = {
        let mut guard = handle.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.push_line(line);
        let due = handle.events_tx.receiver_count() > 0 && !guard.flush_pending;
        guard.flush_pending |= due;
        due
    };
    if schedule {
        let h = Arc::clone(handle);
        tokio::spawn(async move {
            tokio::time::sleep(RUN_OUTPUT_COALESCE).await;
            let snap = {
                let mut guard = h.state.lock().unwrap_or_else(|e| e.into_inner());
                guard.flush_pending = false;
                // A finished or stopped run already sent its final snapshot.
                (guard.status == IdeRunStatus::Running).then(|| guard.to_snapshot(Some(250)))
            };
            if let Some(snap) = snap {
                let _ = h.events_tx.send(snap);
            }
        });
    }
}

pub(super) fn finish_run(
    handle: &ActiveRunHandle,
    status: IdeRunStatus,
    exit_code: Option<i32>,
    elapsed: Duration,
    err_msg: Option<String>,
) {
    let snap_opt = {
        let mut guard = handle.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(msg) = err_msg {
            tracing::warn!(run_id = %guard.id, error = %msg, "IDE run failed");
            guard.push_line(format!("ERROR: {msg}"));
        }
        if guard.status != IdeRunStatus::Stopped {
            guard.status = status;
        }
        guard.exit_code = exit_code;
        guard.duration_ms = Some(elapsed.as_millis() as u64);
        guard.active_local_pid = None;
        guard.compact_on_finish();
        (handle.events_tx.receiver_count() > 0).then(|| guard.to_snapshot(Some(500)))
    };
    if let Some(snap) = snap_opt {
        let _ = handle.events_tx.send(snap);
    }
}
