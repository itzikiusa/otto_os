//! The one command runner of the `script` and `check` nodes (engine.md §7.2,
//! goal.md §2.1, DR-029) — and, until the goal wave removes it, of Goal
//! Loops' verify commands: `/bin/sh -c` in its own process group (the whole
//! group is killed on timeout, cancel or drop), the daemon's environment
//! scrubbed, optional Seatbelt confinement, values passed as environment and
//! never spliced into the command text, bounded in-memory output with the
//! whole of it optionally written to a log file.
use otto_core::domain::CmdRun;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[cfg(unix)]
struct ProcessGroup(rustix::process::Pid);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        let _ = rustix::process::kill_process_group(self.0, rustix::process::Signal::KILL);
    }
}

/// What to run.
pub struct Spec<'a> {
    pub cwd: &'a str,
    pub command: &'a str,
    pub timeout: Duration,
    /// Set after the scrub (`PATH` from the cached login shell, `OTTO_*`).
    pub env: &'a [(String, String)],
    pub sandbox: Option<&'a otto_sandbox::SandboxPolicy>,
    /// Names the sandbox's agent-cache directory.
    pub cache_tag: Option<&'a str>,
    /// Bytes of each stream kept in memory — the newest ones.
    pub output_cap: usize,
    /// Every byte of stdout and stderr, appended as it arrives (the file is
    /// never truncated: a caller running several commands keeps them all).
    pub log_file: Option<&'a Path>,
}

impl<'a> Spec<'a> {
    pub fn new(cwd: &'a str, command: &'a str, timeout: Duration) -> Self {
        Self {
            cwd,
            command,
            timeout,
            env: &[],
            sandbox: None,
            cache_tag: None,
            output_cap: crate::command_output::STREAM_BYTES,
            log_file: None,
        }
    }
}

/// How the command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// Its exit code (-1 when a signal ended it).
    Exited(i32),
    TimedOut,
    Canceled,
    /// It could not be started or read (the message says why).
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Finished {
    pub end: End,
    /// stdout, then `--- stderr ---` and stderr, each cut to its newest
    /// `output_cap` bytes.
    pub output: String,
    pub duration_ms: u64,
}

impl Finished {
    pub fn exited_with(&self, ok: &[i32]) -> bool {
        matches!(self.end, End::Exited(c) if ok.contains(&c))
    }

    /// The Goal Loops shape (messages as before).
    pub fn into_cmd_run(self, timeout: Duration) -> CmdRun {
        let (success, exit_code, output) = match self.end {
            End::Exited(c) => (c == 0, c, self.output),
            End::TimedOut => (
                false,
                -1,
                format!("verification timed out after {}s", timeout.as_secs()),
            ),
            End::Canceled => (false, -1, "verification cancelled".into()),
            End::Failed(m) => (false, -1, m),
        };
        CmdRun {
            success,
            exit_code,
            output,
            duration_ms: self.duration_ms,
        }
    }
}

/// Run `spec`; `cancel` resolving stops it (checked before the spawn too).
pub async fn execute(spec: &Spec<'_>, cancel: impl Future<Output = ()>) -> Finished {
    let start = Instant::now();
    let done = |end: End, output: String| Finished {
        end,
        output,
        duration_ms: start.elapsed().as_millis() as u64,
    };
    tokio::pin!(cancel);
    tokio::select! {
        biased;
        _ = &mut cancel => return done(End::Canceled, String::new()),
        _ = std::future::ready(()) => {}
    }
    let mut command_spec = match spec.sandbox {
        Some(policy) => {
            let tag = spec.cache_tag.filter(|t| !t.is_empty()).unwrap_or("verify");
            let cache = std::env::temp_dir()
                .join("otto-agent-cache")
                .join(format!("goal-loop-{tag}"));
            if let Some(home) = std::env::var_os("HOME") {
                otto_sandbox::seed_agent_cache(&cache, Path::new(&home), |k| std::env::var(k).ok());
            }
            let mut argv: Vec<String> = otto_sandbox::agent_cache_env(&cache)
                .into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            argv.extend([
                "/bin/sh".to_string(),
                "-c".to_string(),
                spec.command.to_string(),
            ]);
            let (program, args) = policy.wrap("/usr/bin/env", &argv);
            let mut c = tokio::process::Command::new(program);
            c.args(args);
            c
        }
        None => {
            let mut c = tokio::process::Command::new("/bin/sh");
            c.arg("-c").arg(spec.command);
            c
        }
    };
    crate::scheduled_tasks_engine::scrub_shell_env(&mut command_spec);
    command_spec
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .current_dir(spec.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command_spec.process_group(0);
    let append = |p: &Path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
    };
    let log = match spec.log_file.map(append).transpose() {
        Ok(f) => f.map(std::sync::Mutex::new),
        Err(e) => {
            return done(
                End::Failed(format!("failed to open the output log: {e}")),
                String::new(),
            )
        }
    };
    let mut child = match command_spec.spawn() {
        Ok(child) => child,
        Err(error) => {
            return done(
                End::Failed(format!("failed to spawn verification: {error}")),
                String::new(),
            )
        }
    };
    // Own a distinct group before waiting: dropping the output future alone
    // kills only the shell and would leave its child tests running.
    #[cfg(unix)]
    let _group = child
        .id()
        .and_then(|id| rustix::process::Pid::from_raw(id as i32))
        .map(ProcessGroup);
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let cap = spec.output_cap;
    // The command ends when its shell exits: whatever it left running in the
    // background is killed then, so its open pipes cannot hold the step open
    // (the 15-minute linger of runs.md §6.3).
    #[cfg(unix)]
    let pgid = _group.as_ref().map(|g| g.0);
    let status = async {
        let status = child.wait().await;
        #[cfg(unix)]
        if let Some(pgid) = pgid {
            let _ = rustix::process::kill_process_group(pgid, rustix::process::Signal::KILL);
        }
        status
    };
    let capture = async {
        tokio::try_join!(
            status,
            crate::command_output::drain_tail(stdout, cap, log.as_ref()),
            crate::command_output::drain_tail(stderr, cap, log.as_ref())
        )
    };
    let output = tokio::select! {
        biased;
        _ = &mut cancel => return done(End::Canceled, String::new()),
        result = tokio::time::timeout(spec.timeout, capture) => result,
    };
    match output {
        Ok(Ok((status, stdout, stderr))) => {
            let mut text = String::from_utf8_lossy(&stdout).into_owned();
            if !stderr.is_empty() {
                text.push_str("\n--- stderr ---\n");
                text.push_str(&String::from_utf8_lossy(&stderr));
            }
            done(End::Exited(status.code().unwrap_or(-1)), text)
        }
        Ok(Err(error)) => done(
            End::Failed(format!("verification failed: {error}")),
            String::new(),
        ),
        Err(_) => done(End::TimedOut, String::new()),
    }
}

pub async fn run(cwd: &str, command: &str, timeout_secs: u64, cancelled: &AtomicBool) -> CmdRun {
    run_with_sandbox(cwd, command, timeout_secs, cancelled, None, None).await
}

/// Goal Loops' verify command: [`execute`] with the controller's cancel flag.
pub async fn run_with_sandbox(
    cwd: &str,
    command: &str,
    timeout_secs: u64,
    cancelled: &AtomicBool,
    sandbox: Option<&otto_sandbox::SandboxPolicy>,
    cache_tag: Option<&str>,
) -> CmdRun {
    let timeout = Duration::from_secs(timeout_secs);
    let spec = Spec {
        sandbox,
        cache_tag,
        ..Spec::new(cwd, command, timeout)
    };
    // Parks on the loop flag bell (perf W8) instead of a 25 ms poll.
    let cancel = crate::goal_loop::until_flag(|| cancelled.load(Ordering::Relaxed));
    execute(&spec, cancel).await.into_cmd_run(timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn output_capture_is_bounded_but_drains_both_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let result = run(
            dir.path().to_str().unwrap(),
            "head -c 3145728 /dev/zero; head -c 3145728 /dev/zero >&2; printf done > completed",
            10,
            &AtomicBool::new(false),
        )
        .await;
        assert!(result.success, "{}", result.output);
        assert!(dir.path().join("completed").exists());
        assert!(
            result.output.len() <= 2 * crate::command_output::STREAM_BYTES + 256,
            "captured {} bytes",
            result.output.len()
        );
        assert!(result.output.contains("bytes omitted"));
    }

    /// A child left in the background neither holds the command open nor
    /// outlives it, and a log file keeps what earlier commands wrote.
    #[tokio::test]
    async fn the_shell_exit_ends_the_command_and_its_background_children() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let log = dir.path().join("all.log");
        std::fs::write(&log, "earlier\n").unwrap();
        let spec = Spec {
            log_file: Some(&log),
            ..Spec::new(
                cwd,
                "(sleep 2; touch late) & echo started",
                Duration::from_secs(30),
            )
        };
        let t0 = Instant::now();
        let done = execute(&spec, std::future::pending()).await;
        assert!(
            t0.elapsed() < Duration::from_secs(2),
            "lingered {:?}",
            t0.elapsed()
        );
        assert_eq!(done.end, End::Exited(0));
        assert!(done.output.contains("started"));
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(
            !dir.path().join("late").exists(),
            "the background child survived"
        );
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(
            text.starts_with("earlier\n") && text.contains("started"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn env_and_cwd_reach_the_command_as_values_never_as_shell_text() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap();
        let env = vec![
            ("OTTO_OUT".to_string(), "a; touch injected".to_string()),
            ("PATH".to_string(), "/usr/bin:/bin".to_string()),
        ];
        let spec = Spec {
            env: &env,
            ..Spec::new(
                cwd,
                "printf '%s|%s|' \"$OTTO_OUT\" \"$PATH\"; pwd -P",
                Duration::from_secs(5),
            )
        };
        let done = execute(&spec, std::future::pending()).await;
        assert_eq!(done.end, End::Exited(0), "{}", done.output);
        let real = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(
            done.output.trim_end(),
            format!("a; touch injected|/usr/bin:/bin|{}", real.display())
        );
        assert!(!dir.path().join("injected").exists());
    }

    #[tokio::test]
    async fn output_keeps_the_newest_bytes_and_the_log_file_keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("step4.log");
        let cmd =
            "i=0; while [ $i -lt 2000 ]; do echo line$i; i=$((i+1)); done; echo boom >&2; exit 3";
        let spec = Spec {
            output_cap: 64,
            log_file: Some(&log),
            ..Spec::new(dir.path().to_str().unwrap(), cmd, Duration::from_secs(10))
        };
        let done = execute(&spec, std::future::pending()).await;
        assert_eq!(done.end, End::Exited(3));
        assert!(done.exited_with(&[0, 3]) && !done.exited_with(&[0]));
        let (out, err) = done.output.split_once("\n--- stderr ---\n").unwrap();
        assert!(
            out.starts_with('[') && out.contains("bytes omitted]"),
            "{out}"
        );
        assert!(out.ends_with("line1999\n"), "the tail is kept: {out}");
        assert!(out.len() < 64 + 40, "{}", out.len());
        assert_eq!(err, "boom\n");
        let all = std::fs::read_to_string(&log).unwrap();
        assert!(all.starts_with("line0\n") && all.contains("line1999\n") && all.contains("boom\n"));
    }

    #[tokio::test]
    async fn a_timeout_kills_the_group_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let spec = Spec::new(
            dir.path().to_str().unwrap(),
            "(sleep 2; touch late) & sleep 5",
            Duration::from_millis(200),
        );
        let done = execute(&spec, std::future::pending()).await;
        assert_eq!(done.end, End::TimedOut);
        let run = done.into_cmd_run(Duration::from_secs(9));
        assert_eq!(run.output, "verification timed out after 9s");
        tokio::time::sleep(Duration::from_millis(2200)).await;
        assert!(!dir.path().join("late").exists(), "the group was killed");
    }

    #[tokio::test]
    async fn cancellation_before_spawn_does_not_execute_verification() {
        let dir = tempfile::tempdir().unwrap();
        let cancelled = AtomicBool::new(true);
        let run = run(
            dir.path().to_str().unwrap(),
            "touch should-not-exist",
            5,
            &cancelled,
        )
        .await;
        assert!(!run.success);
        assert!(!dir.path().join("should-not-exist").exists());
    }
    #[tokio::test]
    async fn cancellation_kills_verification_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let cancelled = AtomicBool::new(false);
        let work = run(
            dir.path().to_str().unwrap(),
            "(sleep 1; touch should-not-exist) & wait",
            10,
            &cancelled,
        );
        let stop = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancelled.store(true, Ordering::Relaxed);
            crate::goal_loop::ring_flags();
        };
        let start = Instant::now();
        let (result, ()) = tokio::join!(work, stop);
        assert!(!result.success);
        assert!(start.elapsed() < Duration::from_millis(750));
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!dir.path().join("should-not-exist").exists());
    }

    #[tokio::test]
    async fn verification_command_scrubs_daemon_env() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OTTO_SECRET_TEST", "top-secret-daemon-token");
        let result = run(
            dir.path().to_str().unwrap(),
            "printf 'secret=%s;has_path=%s' \"$OTTO_SECRET_TEST\" \"${PATH:+yes}\"",
            5,
            &AtomicBool::new(false),
        )
        .await;
        assert!(result.success, "{}", result.output);
        assert_eq!(
            result.output, "secret=;has_path=yes",
            "daemon secret leaked into verification command: {}",
            result.output
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn sandboxed_verification_command_is_confined_to_cwd() {
        if !otto_sandbox::is_supported() {
            return;
        }
        let root = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let root_path = std::fs::canonicalize(root.path()).unwrap();
        let cwd = root_path.join("verify");
        let outside = root_path.join("outside.txt");
        std::fs::create_dir_all(&cwd).unwrap();
        let policy = otto_sandbox::SandboxPolicy::for_agent(
            &cwd,
            &root_path.join("home"),
            &root_path.join("Otto"),
            &[],
            otto_sandbox::NetworkPolicy::None,
        );
        let cmd = format!("echo ok > inside.txt; echo nope > '{}'", outside.display());
        let res = run_with_sandbox(
            cwd.to_str().unwrap(),
            &cmd,
            20,
            &AtomicBool::new(false),
            Some(&policy),
            Some("unit-test"),
        )
        .await;
        assert!(cwd.join("inside.txt").exists(), "cwd must remain writable");
        assert!(
            !outside.exists(),
            "sandboxed verify command wrote outside cwd"
        );
        assert!(!res.success, "denied write must fail the verify command");
    }
}
