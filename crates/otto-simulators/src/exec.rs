//! Process execution and host environment probing abstractions.
//!
//! All simulator commands run as explicit argv arrays (never via `/bin/sh -c`)
//! with strict timeouts. Behind [`CommandRunner`] and [`HostProbe`], production
//! uses [`SystemCommandRunner`] / [`SystemHostProbe`] while tests inject
//! deterministic fakes so every platform state and endpoint can be verified
//! even on hosts without Xcode runtimes or the Android SDK.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

/// Specification for running a host CLI command.
#[derive(Debug, Clone)]
pub struct CmdSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Duration,
}

impl CmdSpec {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: Vec::new(),
            stdin: None,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_stdin(mut self, stdin: Vec<u8>) -> Self {
        self.stdin = Some(stdin);
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

/// Captured command output.
#[derive(Debug, Clone, Default)]
pub struct CmdOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CmdOutput {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }

    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    pub fn combined_err(&self) -> String {
        let err = self.stderr_str();
        let trimmed = err.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
        let out = self.stdout_str();
        let trimmed = out.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
        format!("exit code {}", self.exit_code)
    }
}

/// A handle to a live line-streaming child process (logcat / simctl log stream /
/// xcodebuild). Both stdout and stderr are drained concurrently into `rx`,
/// and the process exit code is delivered via `exit_rx`.
/// Dropping this handle cancels the reader tasks and kills the underlying child.
pub struct LogStreamHandle {
    pub rx: mpsc::Receiver<String>,
    pub exit_rx: oneshot::Receiver<i32>,
    abort: Option<tokio::task::AbortHandle>,
}

impl LogStreamHandle {
    pub fn new(
        rx: mpsc::Receiver<String>,
        exit_rx: oneshot::Receiver<i32>,
        abort: Option<tokio::task::AbortHandle>,
    ) -> Self {
        Self { rx, exit_rx, abort }
    }

    /// Wait for the streamed child process to exit and return its exit code
    /// (`-1` if the channel closed without a code).
    pub async fn wait_exit(&mut self) -> i32 {
        (&mut self.exit_rx).await.unwrap_or(-1)
    }
}

impl Drop for LogStreamHandle {
    fn drop(&mut self) {
        if let Some(a) = self.abort.take() {
            a.abort();
        }
    }
}

#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Run a command to completion, returning captured stdout/stderr.
    async fn run(&self, spec: CmdSpec) -> otto_core::Result<CmdOutput>;

    /// Run a command to completion with null stdin/stdout/stderr (used for
    /// `adb start-server` so the forked adb daemon never inherits piped FDs
    /// and hangs `wait_with_output()`).
    async fn run_silent(&self, spec: CmdSpec) -> otto_core::Result<i32>;

    /// Spawn a long-lived background process (e.g., `emulator -avd ...`) detached
    /// from the HTTP request lifecycle.
    async fn spawn_detached(&self, spec: CmdSpec) -> otto_core::Result<()>;

    /// Spawn a streaming command whose stdout and stderr lines are pushed over a channel.
    async fn spawn_log_stream(&self, spec: CmdSpec) -> otto_core::Result<LogStreamHandle>;
}

/// Host filesystem and environment probe for dynamic SDK discovery.
pub trait HostProbe: Send + Sync {
    fn path_exists(&self, path: &Path) -> bool;
    fn is_executable(&self, path: &Path) -> bool;
    fn read_to_string(&self, path: &Path) -> Option<String>;
    fn list_dir(&self, path: &Path) -> Vec<String>;
    fn env_var(&self, key: &str) -> Option<String>;
    fn home_dir(&self) -> Option<PathBuf>;
    fn which(&self, bin: &str) -> Option<PathBuf>;
}

// ---------------------------------------------------------------------------
// Production implementations
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SystemCommandRunner;

#[async_trait]
impl CommandRunner for SystemCommandRunner {
    async fn run(&self, spec: CmdSpec) -> otto_core::Result<CmdOutput> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if spec.stdin.is_some() {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| {
            otto_core::Error::Upstream(format!("failed to spawn {}: {e}", spec.program))
        })?;
        if let Some(input) = spec.stdin {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(&input).await;
                let _ = stdin.shutdown().await;
            }
        }
        let out = tokio::time::timeout(spec.timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                otto_core::Error::Upstream(format!(
                    "{} timed out after {}s",
                    spec.program,
                    spec.timeout.as_secs()
                ))
            })?
            .map_err(|e| {
                otto_core::Error::Upstream(format!("{} execution failed: {e}", spec.program))
            })?;
        Ok(CmdOutput {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }

    async fn run_silent(&self, spec: CmdSpec) -> otto_core::Result<i32> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| {
            otto_core::Error::Upstream(format!("failed to spawn {}: {e}", spec.program))
        })?;
        let status = tokio::time::timeout(spec.timeout, child.wait())
            .await
            .map_err(|_| {
                otto_core::Error::Upstream(format!(
                    "{} timed out after {}s",
                    spec.program,
                    spec.timeout.as_secs()
                ))
            })?
            .map_err(|e| {
                otto_core::Error::Upstream(format!("{} execution failed: {e}", spec.program))
            })?;
        Ok(status.code().unwrap_or(-1))
    }

    async fn spawn_detached(&self, spec: CmdSpec) -> otto_core::Result<()> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let _child = cmd.spawn().map_err(|e| {
            otto_core::Error::Upstream(format!("failed to spawn {}: {e}", spec.program))
        })?;
        Ok(())
    }

    async fn spawn_log_stream(&self, spec: CmdSpec) -> otto_core::Result<LogStreamHandle> {
        let mut cmd = Command::new(&spec.program);
        cmd.args(&spec.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| {
            otto_core::Error::Upstream(format!("failed to spawn {}: {e}", spec.program))
        })?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, rx) = mpsc::channel::<String>(256);
        let (exit_tx, exit_rx) = oneshot::channel::<i32>();

        let handle = tokio::spawn(async move {
            // Drain stdout and stderr concurrently so neither pipe buffer can fill
            // and deadlock the child, then report the exit code.
            let out_tx = tx.clone();
            let out_task = tokio::spawn(async move {
                if let Some(out) = stdout {
                    let mut lines = BufReader::new(out).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if out_tx.send(line).await.is_err() {
                            break;
                        }
                    }
                }
            });
            let err_tx = tx.clone();
            let err_task = tokio::spawn(async move {
                if let Some(err) = stderr {
                    let mut lines = BufReader::new(err).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        if err_tx.send(line).await.is_err() {
                            break;
                        }
                    }
                }
            });
            drop(tx);
            let _ = tokio::join!(out_task, err_task);
            let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(-1);
            let _ = exit_tx.send(code);
        });
        Ok(LogStreamHandle::new(
            rx,
            exit_rx,
            Some(handle.abort_handle()),
        ))
    }
}

#[derive(Debug, Default)]
pub struct SystemHostProbe;

impl HostProbe for SystemHostProbe {
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_executable(&self, path: &Path) -> bool {
        if !path.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(path)
                .map(|m| (m.permissions().mode() & 0o111) != 0)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            true
        }
    }

    fn read_to_string(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    #[allow(clippy::disallowed_methods)] // sync HostProbe impl: shallow single-level SDK dir check (10s cached)
    fn list_dir(&self, path: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(path) else {
            return Vec::new();
        };
        let mut out: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        out.sort();
        out
    }

    fn env_var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.trim().is_empty())
    }

    fn home_dir(&self) -> Option<PathBuf> {
        dirs::home_dir()
    }

    fn which(&self, bin: &str) -> Option<PathBuf> {
        let path_var = std::env::var_os("PATH")?;
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin);
            if self.is_executable(&candidate) {
                return Some(candidate);
            }
        }
        // Fallback standard macOS Homebrew / system locations when launched under launchd.
        for fallback in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            let candidate = Path::new(fallback).join(bin);
            if self.is_executable(&candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Test / fake implementations
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "test-util"))]
pub mod fake {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    type MatcherFn = Box<dyn Fn(&CmdSpec) -> Option<CmdOutput> + Send + Sync>;

    /// Deterministic command runner for unit and integration tests.
    #[derive(Clone, Default)]
    pub struct FakeCommandRunner {
        inner: Arc<Mutex<FakeRunnerInner>>,
    }

    #[derive(Default)]
    struct FakeRunnerInner {
        handlers: Vec<MatcherFn>,
        log_lines: Vec<String>,
        log_exit_code: i32,
        log_delay: Option<Duration>,
        recorded: Vec<(String, Vec<String>)>,
        detached: Vec<(String, Vec<String>)>,
    }

    impl FakeCommandRunner {
        pub fn new() -> Self {
            Self::default()
        }

        /// Register a matcher; later registrations take precedence so tests can
        /// override state transitions cleanly.
        pub fn on<F>(&self, f: F)
        where
            F: Fn(&CmdSpec) -> Option<CmdOutput> + Send + Sync + 'static,
        {
            self.inner.lock().unwrap().handlers.push(Box::new(f));
        }

        pub fn on_exact(&self, program_suffix: &str, args: &[&str], out: CmdOutput) {
            let suffix = program_suffix.to_string();
            let expected: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
            self.on(move |spec| {
                let prog_matches =
                    spec.program == suffix || spec.program.ends_with(&format!("/{suffix}"));
                if prog_matches && spec.args == expected {
                    Some(out.clone())
                } else {
                    None
                }
            });
        }

        pub fn set_log_lines(&self, lines: Vec<String>) {
            self.inner.lock().unwrap().log_lines = lines;
        }

        pub fn set_log_exit_code(&self, code: i32) {
            self.inner.lock().unwrap().log_exit_code = code;
        }

        pub fn set_log_delay(&self, delay: Duration) {
            self.inner.lock().unwrap().log_delay = Some(delay);
        }

        pub fn recorded(&self) -> Vec<(String, Vec<String>)> {
            self.inner.lock().unwrap().recorded.clone()
        }

        pub fn clear_recorded(&self) {
            self.inner.lock().unwrap().recorded.clear();
        }

        pub fn detached(&self) -> Vec<(String, Vec<String>)> {
            self.inner.lock().unwrap().detached.clone()
        }
    }

    #[async_trait]
    impl CommandRunner for FakeCommandRunner {
        async fn run(&self, spec: CmdSpec) -> otto_core::Result<CmdOutput> {
            let mut guard = self.inner.lock().unwrap();
            guard
                .recorded
                .push((spec.program.clone(), spec.args.clone()));
            for handler in guard.handlers.iter().rev() {
                if let Some(out) = handler(&spec) {
                    return Ok(out);
                }
            }
            Ok(CmdOutput {
                exit_code: 1,
                stdout: Vec::new(),
                stderr: format!("unmocked command: {} {:?}", spec.program, spec.args).into_bytes(),
            })
        }

        async fn run_silent(&self, spec: CmdSpec) -> otto_core::Result<i32> {
            let mut guard = self.inner.lock().unwrap();
            guard
                .recorded
                .push((spec.program.clone(), spec.args.clone()));
            for handler in guard.handlers.iter().rev() {
                if let Some(out) = handler(&spec) {
                    return Ok(out.exit_code);
                }
            }
            Ok(0)
        }

        async fn spawn_detached(&self, spec: CmdSpec) -> otto_core::Result<()> {
            self.inner
                .lock()
                .unwrap()
                .detached
                .push((spec.program, spec.args));
            Ok(())
        }

        async fn spawn_log_stream(&self, spec: CmdSpec) -> otto_core::Result<LogStreamHandle> {
            let (lines, exit_code, delay) = {
                let mut guard = self.inner.lock().unwrap();
                guard
                    .recorded
                    .push((spec.program.clone(), spec.args.clone()));
                (
                    guard.log_lines.clone(),
                    guard.log_exit_code,
                    guard.log_delay,
                )
            };
            let (tx, rx) = mpsc::channel(64);
            let (exit_tx, exit_rx) = oneshot::channel();
            let h = tokio::spawn(async move {
                if let Some(d) = delay {
                    tokio::time::sleep(d).await;
                }
                for line in lines {
                    if tx.send(line).await.is_err() {
                        break;
                    }
                }
                let _ = exit_tx.send(exit_code);
            });
            Ok(LogStreamHandle::new(rx, exit_rx, Some(h.abort_handle())))
        }
    }

    /// Deterministic host filesystem & environment probe for tests.
    #[derive(Clone, Default)]
    pub struct FakeHostProbe {
        inner: Arc<Mutex<FakeProbeInner>>,
    }

    #[derive(Default)]
    struct FakeProbeInner {
        existing: HashSet<PathBuf>,
        executables: HashSet<PathBuf>,
        files: HashMap<PathBuf, String>,
        dirs: HashMap<PathBuf, Vec<String>>,
        envs: HashMap<String, String>,
        bins: HashMap<String, PathBuf>,
        home: Option<PathBuf>,
    }

    impl FakeHostProbe {
        pub fn new() -> Self {
            let p = Self::default();
            p.set_home("/Users/tester");
            p
        }

        pub fn set_home(&self, home: impl Into<PathBuf>) {
            self.inner.lock().unwrap().home = Some(home.into());
        }

        pub fn add_path(&self, path: impl Into<PathBuf>) {
            self.inner.lock().unwrap().existing.insert(path.into());
        }

        pub fn add_executable(&self, path: impl Into<PathBuf>) {
            let p = path.into();
            let mut g = self.inner.lock().unwrap();
            g.existing.insert(p.clone());
            g.executables.insert(p);
        }

        pub fn add_file(&self, path: impl Into<PathBuf>, content: impl Into<String>) {
            let p = path.into();
            let mut g = self.inner.lock().unwrap();
            g.existing.insert(p.clone());
            g.files.insert(p, content.into());
        }

        pub fn add_dir(&self, path: impl Into<PathBuf>, entries: Vec<String>) {
            let p = path.into();
            let mut g = self.inner.lock().unwrap();
            g.existing.insert(p.clone());
            g.dirs.insert(p, entries);
        }

        pub fn set_env(&self, key: impl Into<String>, value: impl Into<String>) {
            self.inner
                .lock()
                .unwrap()
                .envs
                .insert(key.into(), value.into());
        }

        pub fn add_which(&self, bin: impl Into<String>, path: impl Into<PathBuf>) {
            let p = path.into();
            let mut g = self.inner.lock().unwrap();
            g.existing.insert(p.clone());
            g.executables.insert(p.clone());
            g.bins.insert(bin.into(), p);
        }
    }

    impl HostProbe for FakeHostProbe {
        fn path_exists(&self, path: &Path) -> bool {
            self.inner.lock().unwrap().existing.contains(path)
        }

        fn is_executable(&self, path: &Path) -> bool {
            self.inner.lock().unwrap().executables.contains(path)
        }

        fn read_to_string(&self, path: &Path) -> Option<String> {
            self.inner.lock().unwrap().files.get(path).cloned()
        }

        fn list_dir(&self, path: &Path) -> Vec<String> {
            self.inner
                .lock()
                .unwrap()
                .dirs
                .get(path)
                .cloned()
                .unwrap_or_default()
        }

        fn env_var(&self, key: &str) -> Option<String> {
            self.inner.lock().unwrap().envs.get(key).cloned()
        }

        fn home_dir(&self) -> Option<PathBuf> {
            self.inner.lock().unwrap().home.clone()
        }

        fn which(&self, bin: &str) -> Option<PathBuf> {
            self.inner.lock().unwrap().bins.get(bin).cloned()
        }
    }

    pub fn ok_out(stdout: impl Into<Vec<u8>>) -> CmdOutput {
        CmdOutput {
            exit_code: 0,
            stdout: stdout.into(),
            stderr: Vec::new(),
        }
    }

    pub fn err_out(stderr: impl Into<Vec<u8>>) -> CmdOutput {
        CmdOutput {
            exit_code: 1,
            stdout: Vec::new(),
            stderr: stderr.into(),
        }
    }
}
