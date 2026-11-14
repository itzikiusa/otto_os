//! Background Test & Run execution engine (`execute_run_task`, `build_runner_command`),
//! multi-repo Java source-first classpath compilation, Maven/Gradle/Spring Boot
//! profile injection, and Unix process-group lifecycle cleanup.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

use crate::target::TargetManager;
use crate::types::{
    IdeRunConfig, IdeRunState, IdeRunStatus, IdeRunnerKind, IdeSshSpec, IdeTargetKind,
    IdeTargetSpec,
};

pub(super) use super::runner_cmd::{
    apply_runtime_profile_and_module_venv, build_runner_command, compile_java_main_step,
    has_manual_classpath, resolve_program_in_env, resolve_run_cwd,
};
use super::runner_parsers::IncrementalTestTracker;

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
    status: IdeRunStatus,
    exit_code: Option<i32>,
    started_at: String,
    duration_ms: Option<u64>,
    output_seq: u64,
    output_bytes: usize,
    output: VecDeque<String>,
    parser: IncrementalTestTracker,
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
        }
    }
}

pub(super) struct ActiveRunHandle {
    pub(super) seq: u64,
    pub(super) owner_user_id: String,
    pub(super) state: Mutex<RunStateInner>,
    pub(super) stop_tx: watch::Sender<bool>,
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
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if guard.status == IdeRunStatus::Running {
            guard.status = IdeRunStatus::Stopped;
            guard.push_line("[Stopped by user]".into());
        }
        guard.to_snapshot(None)
    }
}

struct ChildProcessGroupGuard {
    pid: Option<u32>,
    is_local: bool,
    disarmed: bool,
}

impl Drop for ChildProcessGroupGuard {
    #[allow(clippy::disallowed_methods)] // synchronous Drop safeguard against orphan child processes
    fn drop(&mut self) {
        if !self.disarmed && self.is_local {
            #[cfg(unix)]
            if let Some(pid) = self.pid {
                let _ = std::process::Command::new("/bin/kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}

async fn kill_child_process_group(
    child: &mut tokio::process::Child,
    pid: Option<u32>,
    is_local: bool,
) {
    #[cfg(unix)]
    if is_local {
        if let Some(p) = pid {
            let neg = format!("-{p}");
            let _ = Command::new("/bin/kill")
                .args(["-TERM", &neg])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
            if tokio::time::timeout(Duration::from_millis(150), child.wait())
                .await
                .is_ok()
            {
                let _ = Command::new("/bin/kill")
                    .args(["-KILL", &neg])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
                return;
            }
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &neg])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, is_local);

    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_millis(120), child.wait()).await;
}

async fn cleanup_remaining_process_group(pid: Option<u32>, is_local: bool) {
    #[cfg(unix)]
    if is_local {
        if let Some(p) = pid {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &format!("-{p}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, is_local);
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_run_task(
    runs: Arc<DashMap<String, Arc<ActiveRunHandle>>>,
    handle: Arc<ActiveRunHandle>,
    targets: TargetManager,
    target: IdeTargetSpec,
    ssh: Option<IdeSshSpec>,
    steps: Vec<(IdeRunConfig, BTreeMap<String, String>)>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let started = Instant::now();
    let multi_step = steps.len() > 1;
    let mut last_exit = Some(0);

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

        let mut temp_java_build_dir: Option<PathBuf> = None;
        let (program, args) = if target.kind == IdeTargetKind::Local
            && cfg.runner == IdeRunnerKind::JavaMain
            && !has_manual_classpath(&cfg)
        {
            match compile_java_main_step(&cfg, &target, &cwd, &env_map, &handle).await {
                Ok((prog, built_args, out_dir)) => {
                    temp_java_build_dir = Some(out_dir);
                    (prog, built_args)
                }
                Err(err) => {
                    finish_run(
                        &handle,
                        IdeRunStatus::Failed,
                        Some(1),
                        started.elapsed(),
                        Some(err),
                    );
                    evict_finished_runs(&runs);
                    return;
                }
            }
        } else {
            build_runner_command(&cfg, &cwd, &env_map)
        };

        if let Some(pre) = cfg
            .before_launch
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            append_run_line(&handle, format!("$ {pre}"));
            if let Err(err) = run_before_launch(
                &targets,
                &target,
                ssh.as_ref(),
                &handle.owner_user_id,
                pre,
                &cwd,
                &env_map,
            )
            .await
            {
                if let Some(dir) = temp_java_build_dir.take() {
                    let _ = tokio::fs::remove_dir_all(dir).await;
                }
                finish_run(
                    &handle,
                    IdeRunStatus::Failed,
                    Some(1),
                    started.elapsed(),
                    Some(err),
                );
                evict_finished_runs(&runs);
                return;
            }
        }

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
        ) {
            Ok(c) => c,
            Err(err) => {
                if let Some(dir) = temp_java_build_dir.take() {
                    let _ = tokio::fs::remove_dir_all(dir).await;
                }
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
        let mut pg_guard = ChildProcessGroupGuard {
            pid,
            is_local,
            disarmed: false,
        };

        let stdout_task = child.stdout.take().map(|out| {
            let h = Arc::clone(&handle);
            tokio::spawn(async move {
                let mut reader = BufReader::new(out).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    append_run_line(&h, line);
                }
            })
        });
        let stderr_task = child.stderr.take().map(|err| {
            let h = Arc::clone(&handle);
            tokio::spawn(async move {
                let mut reader = BufReader::new(err).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    append_run_line(&h, line);
                }
            })
        });

        let wait_res = tokio::select! {
            status = child.wait() => {
                cleanup_remaining_process_group(pid, is_local).await;
                pg_guard.disarmed = true;
                Some(status)
            }
            _ = stop_rx.changed() => {
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
        if let Some(dir) = temp_java_build_dir.take() {
            let _ = tokio::fs::remove_dir_all(dir).await;
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

fn multi_step_label(cfg: &IdeRunConfig) -> &'static str {
    match cfg.runner {
        IdeRunnerKind::GoBuild => "build",
        _ => "run",
    }
}

pub(super) fn append_run_line(handle: &ActiveRunHandle, line: String) {
    let mut guard = handle.state.lock().unwrap_or_else(|e| e.into_inner());
    guard.push_line(line);
}

pub(super) fn finish_run(
    handle: &ActiveRunHandle,
    status: IdeRunStatus,
    exit_code: Option<i32>,
    elapsed: Duration,
    err_msg: Option<String>,
) {
    let mut guard = handle.state.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(msg) = err_msg {
        guard.push_line(format!("ERROR: {msg}"));
    }
    if guard.status != IdeRunStatus::Stopped {
        guard.status = status;
    }
    guard.exit_code = exit_code;
    guard.duration_ms = Some(elapsed.as_millis() as u64);
    guard.compact_on_finish();
}

async fn run_before_launch(
    targets: &TargetManager,
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    actor_id: &str,
    script: &str,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    match target.kind {
        IdeTargetKind::Local => {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", script])
                .current_dir(cwd)
                .envs(env_map)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            #[cfg(unix)]
            cmd.process_group(0);

            let out = cmd.output().await.map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!(
                    "before_launch failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Ok(())
        }
        IdeTargetKind::Ssh => {
            let spec = ssh.ok_or("missing SSH spec for before_launch")?;
            let res = targets
                .resolve_with_ssh(target, actor_id, spec.clone())
                .map_err(|e| e.to_string())?;
            let out = targets
                .exec_shell(&res, script, Some(cwd), env_map, Duration::from_secs(120))
                .await
                .map_err(|e| e.to_string())?;
            if !out.success() {
                return Err(format!("before_launch failed: {}", out.stderr.trim()));
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_run_child(
    targets: &TargetManager,
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    actor_id: &str,
    program: &str,
    args: &[String],
    cwd: &str,
    env_map: &BTreeMap<String, String>,
) -> std::result::Result<tokio::process::Child, String> {
    match target.kind {
        IdeTargetKind::Local => {
            let resolved_prog = resolve_program_in_env(program, env_map);
            let mut cmd = Command::new(resolved_prog);
            cmd.args(args)
                .current_dir(cwd)
                .envs(env_map)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            #[cfg(unix)]
            cmd.process_group(0);
            cmd.spawn().map_err(|e| format!("spawn '{program}': {e}"))
        }
        IdeTargetKind::Ssh => {
            let spec = ssh.ok_or("missing SSH connection spec for remote run")?;
            let mut cmd = targets
                .build_ssh_run_command(spec, actor_id, cwd, program, args, env_map)
                .map_err(|e| e.to_string())?;
            #[cfg(unix)]
            cmd.process_group(0);
            cmd.spawn().map_err(|e| format!("spawn ssh: {e}"))
        }
    }
}
