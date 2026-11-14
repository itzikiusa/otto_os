//! Cached User Login + Interactive Shell Environment Resolver (`ShellEnvResolver`).
//!
//! When `ottod` runs as a macOS `launchd` service or bundled Tauri sidecar inside
//! `Otto.app`, `std::env::vars()` lacks user-exported variables from `~/.zshenv`,
//! `~/.zprofile`, `~/.zshrc`, `~/.bash_profile`, and `~/.bashrc` (for example
//! `GO_DEBUGGING_PRODAMS`, `GO_DEBUGGING_PARAMS`, `GOPRIVATE`, `GONOSUMDB`,
//! `AWS_PROFILE`, `GOPATH`, and custom `PATH` entries).
//!
//! `ShellEnvResolver` spawns `$SHELL -lic` **once** lazily in the background
//! using unique sentinel markers around `/usr/bin/env -0` so MOTD banners, ANSI
//! escapes, and `echo` output in `.zshrc` never corrupt parsing, enforces a hard
//! 3-second process-group timeout (`process_group(0)` + `-pgid` kill) so hanging
//! shell RC scripts never leak orphan processes, and caches the resulting
//! `BTreeMap<String, String>` in memory with explicit `POST /ide/shell-env/reload`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Notify;

use crate::target::TargetManager;
use crate::types::{IdeShellEnvResp, IdeSshSpec, IdeTargetSpec};

use super::is_valid_env_key;

const INTERACTIVE_SHELL_TIMEOUT: Duration = Duration::from_secs(3);
const LOGIN_FALLBACK_TIMEOUT: Duration = Duration::from_millis(1500);
const MAX_SHELL_STDOUT_BYTES: usize = 512 * 1024;
const MAX_SINGLE_ENV_VALUE_BYTES: usize = 32 * 1024;
const MAX_CACHED_ENV_VARS: usize = 512;

/// Ephemeral shell/launchd variables that should never overwrite explicit
/// working directories or leak probe state into spawned IDE child processes.
const IGNORED_SHELL_KEYS: &[&str] = &[
    "_",
    "PWD",
    "OLDPWD",
    "SHLVL",
    "TERM_SESSION_ID",
    "ZSH_EXECUTION_STRING",
    "OTTO_SHELL_ENV_PROBE",
    "XPC_SERVICE_NAME",
    "XPC_FLAGS",
];

static GLOBAL_SHELL_ENV: OnceLock<Arc<ShellEnvResolver>> = OnceLock::new();
static PROBE_TOKEN_SEQ: AtomicU64 = AtomicU64::new(1);

/// Return the process-wide shared `ShellEnvResolver`.
pub fn global_shell_env() -> &'static Arc<ShellEnvResolver> {
    GLOBAL_SHELL_ENV.get_or_init(|| Arc::new(ShellEnvResolver::new()))
}

#[derive(Debug, Clone)]
struct ShellEnvState {
    status: String,
    shell: String,
    vars: BTreeMap<String, String>,
    loaded_at: Option<String>,
    duration_ms: Option<u64>,
    error: Option<String>,
}

/// Cached login+interactive shell environment resolver.
pub struct ShellEnvResolver {
    state: RwLock<ShellEnvState>,
    warming: AtomicBool,
    notify: Notify,
}

impl Default for ShellEnvResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellEnvResolver {
    pub fn new() -> Self {
        let shell = detect_user_shell();
        let base_vars = collect_process_base_env();
        let status = if otto_core::e2e::isolated() {
            "ready"
        } else {
            "cold"
        };
        Self {
            state: RwLock::new(ShellEnvState {
                status: status.to_string(),
                shell,
                vars: base_vars,
                loaded_at: None,
                duration_ms: None,
                error: None,
            }),
            warming: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_seeded_vars(vars: BTreeMap<String, String>) -> Arc<Self> {
        Arc::new(Self {
            state: RwLock::new(ShellEnvState {
                status: "ready".to_string(),
                shell: "/bin/zsh".to_string(),
                vars,
                loaded_at: Some("2026-01-01T00:00:00Z".to_string()),
                duration_ms: Some(1),
                error: None,
            }),
            warming: AtomicBool::new(false),
            notify: Notify::new(),
        })
    }

    /// Kick off background warming on the active Tokio runtime if still cold.
    /// Never blocks the caller (`<1us` on the HTTP hot path).
    pub fn ensure_warming_in_background(self: &Arc<Self>) {
        if otto_core::e2e::isolated() {
            return;
        }
        let is_cold = self.state.read().unwrap_or_else(|e| e.into_inner()).status == "cold";
        if !is_cold {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self
            .warming
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            {
                let mut guard = self.state.write().unwrap_or_else(|e| e.into_inner());
                if guard.status == "cold" {
                    guard.status = "warming".to_string();
                }
            }
            let this = Arc::clone(self);
            handle.spawn(async move {
                this.refresh_inner().await;
            });
        }
    }

    /// Non-blocking snapshot of the cached user shell environment (`<1us`).
    /// Triggers background warming if still cold.
    pub fn snapshot(self: &Arc<Self>) -> IdeShellEnvResp {
        self.ensure_warming_in_background();
        let guard = self.state.read().unwrap_or_else(|e| e.into_inner());
        state_to_resp(&guard)
    }

    /// Non-blocking snapshot of just the resolved `KEY=VALUE` map (`<1us`).
    pub fn vars_snapshot(self: &Arc<Self>) -> BTreeMap<String, String> {
        self.ensure_warming_in_background();
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .vars
            .clone()
    }

    /// Return the resolved shell environment, awaiting an in-flight initial
    /// warm (bounded to ~3.5s) so the very first Run/Debug launch or
    /// `GET /ide/shell-env` request is guaranteed to include `.zshrc` exports.
    pub async fn get_or_load(self: &Arc<Self>) -> IdeShellEnvResp {
        if otto_core::e2e::isolated() {
            let guard = self.state.read().unwrap_or_else(|e| e.into_inner());
            return state_to_resp(&guard);
        }
        loop {
            let notified = self.notify.notified();
            let status = {
                let guard = self.state.read().unwrap_or_else(|e| e.into_inner());
                if guard.status != "cold" && guard.status != "warming" {
                    return state_to_resp(&guard);
                }
                guard.status.clone()
            };
            if status == "cold"
                && self
                    .warming
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                {
                    let mut guard = self.state.write().unwrap_or_else(|e| e.into_inner());
                    guard.status = "warming".to_string();
                }
                self.refresh_inner().await;
                let guard = self.state.read().unwrap_or_else(|e| e.into_inner());
                return state_to_resp(&guard);
            }
            if tokio::time::timeout(Duration::from_millis(3600), notified)
                .await
                .is_err()
            {
                let guard = self.state.read().unwrap_or_else(|e| e.into_inner());
                return state_to_resp(&guard);
            }
        }
    }

    /// Awaited `KEY=VALUE` map for Run/Debug process launches.
    pub async fn vars_or_load(self: &Arc<Self>) -> BTreeMap<String, String> {
        self.get_or_load().await.vars
    }

    /// Force a fresh `$SHELL -lic` capture (`POST /ide/shell-env/reload`).
    pub async fn reload(self: &Arc<Self>) -> IdeShellEnvResp {
        if otto_core::e2e::isolated() {
            let fresh = collect_process_base_env();
            let mut guard = self.state.write().unwrap_or_else(|e| e.into_inner());
            guard.status = "ready".to_string();
            guard.vars = fresh;
            guard.loaded_at = Some(Utc::now().to_rfc3339());
            guard.duration_ms = Some(0);
            guard.error = None;
            return state_to_resp(&guard);
        }
        // Serialize concurrent reload calls cleanly.
        while self
            .warming
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            let _ = tokio::time::timeout(Duration::from_secs(4), self.notify.notified()).await;
        }
        self.refresh_inner().await;
        let guard = self.state.read().unwrap_or_else(|e| e.into_inner());
        state_to_resp(&guard)
    }

    async fn refresh_inner(&self) {
        let shell = detect_user_shell();
        let started = Instant::now();
        let outcome = capture_shell_env_with_shell(&shell, INTERACTIVE_SHELL_TIMEOUT).await;
        let duration_ms = started.elapsed().as_millis() as u64;
        let now = Utc::now().to_rfc3339();

        {
            let mut guard = self.state.write().unwrap_or_else(|e| e.into_inner());
            guard.shell = shell;
            guard.loaded_at = Some(now);
            guard.duration_ms = Some(duration_ms);
            match outcome {
                Ok(mut parsed) => {
                    // Preserve any launchd base variables not present in shell output.
                    let mut merged = collect_process_base_env();
                    merged.append(&mut parsed);
                    guard.status = "ready".to_string();
                    guard.vars = merged;
                    guard.error = None;
                }
                Err(err) => {
                    guard.status = "fallback".to_string();
                    guard.vars = collect_process_base_env();
                    guard.error = Some(err);
                }
            }
        }
        self.warming.store(false, Ordering::Release);
        self.notify.notify_waiters();
    }
}

fn state_to_resp(st: &ShellEnvState) -> IdeShellEnvResp {
    let var_count = st.vars.len();
    IdeShellEnvResp {
        status: st.status.clone(),
        shell: st.shell.clone(),
        vars: st.vars.clone(),
        env: st.vars.clone(),
        var_count,
        loaded_at: st.loaded_at.clone(),
        duration_ms: st.duration_ms,
        error: st.error.clone(),
    }
}

pub(crate) fn collect_process_base_env() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in std::env::vars() {
        if should_keep_env_entry(&k, &v) {
            out.insert(k, v);
            if out.len() >= MAX_CACHED_ENV_VARS {
                break;
            }
        }
    }
    out
}

/// Minimal safe host environment subset for non-root callers so the daemon
/// owner's process environment secrets never enter `effective_env`.
pub(crate) fn collect_safe_non_root_base_env() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for key in ["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TMPDIR", "TERM"] {
        if let Ok(val) = std::env::var(key) {
            if should_keep_env_entry(key, &val) {
                out.insert(key.to_string(), val);
            }
        }
    }
    out
}

/// Probe the login shell environment on a remote SSH target (`GET /ide/shell-env`
/// and `POST /ide/shell-env/reload` when `target.kind == ssh`).
pub async fn probe_remote_shell_env(
    targets: &TargetManager,
    target: &IdeTargetSpec,
    ssh: &IdeSshSpec,
) -> otto_core::Result<IdeShellEnvResp> {
    let started = Instant::now();
    let resolved = targets.resolve_with_ssh(target, "ide-shell-env", ssh.clone())?;
    let token = PROBE_TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    let start_marker = format!("__OTTO_REMOTE_ENV_BEGIN_{token}__");
    let end_marker = format!("__OTTO_REMOTE_ENV_END_{token}__");
    let script = format!(
        "sh_bin=\"${{SHELL:-/bin/sh}}\"; \"$sh_bin\" -lc 'printf \"%s\" \"{start_marker}\"; (env -0 2>/dev/null || printenv -0 2>/dev/null); printf \"%s\" \"{end_marker}\"; printf \"\\n__OTTO_SHELL__=%s\\n\" \"${{SHELL:-/bin/sh}}\"'"
    );
    let out = targets
        .exec_shell(
            &resolved,
            &script,
            Some(&target.root),
            &BTreeMap::new(),
            Duration::from_secs(8),
        )
        .await?;
    let duration_ms = started.elapsed().as_millis() as u64;
    let shell_name = out
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("__OTTO_SHELL__="))
        .unwrap_or("/bin/sh")
        .trim()
        .to_string();

    match parse_sentinel_env_zero(out.stdout.as_bytes(), &start_marker, &end_marker) {
        Ok(vars) => {
            let var_count = vars.len();
            Ok(IdeShellEnvResp {
                status: "ready".to_string(),
                shell: shell_name,
                var_count,
                env: vars.clone(),
                vars,
                loaded_at: Some(Utc::now().to_rfc3339()),
                duration_ms: Some(duration_ms),
                error: None,
            })
        }
        Err(err) => Ok(IdeShellEnvResp {
            status: "degraded".to_string(),
            shell: shell_name,
            var_count: 0,
            vars: BTreeMap::new(),
            env: BTreeMap::new(),
            loaded_at: Some(Utc::now().to_rfc3339()),
            duration_ms: Some(duration_ms),
            error: Some(err),
        }),
    }
}

fn detect_user_shell() -> String {
    if let Ok(explicit) = std::env::var("SHELL") {
        let trimmed = explicit.trim();
        if trimmed.starts_with('/') && Path::new(trimmed).is_file() {
            return trimmed.to_string();
        }
    }
    for candidate in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
        if Path::new(candidate).is_file() {
            return candidate.to_string();
        }
    }
    "/bin/sh".to_string()
}

fn should_keep_env_entry(key: &str, value: &str) -> bool {
    is_valid_env_key(key)
        && !IGNORED_SHELL_KEYS.contains(&key)
        && value.len() <= MAX_SINGLE_ENV_VALUE_BYTES
}

/// Capture environment variables from `shell_bin` using `-lic` (login + interactive,
/// loading `.zshenv` + `.zprofile` + `.zshrc` / `.bash_profile` + `.bashrc`) with
/// fallback to `-lc` (login only) if interactive mode times out or errors.
pub(crate) async fn capture_shell_env_with_shell(
    shell_bin: &str,
    interactive_timeout: Duration,
) -> std::result::Result<BTreeMap<String, String>, String> {
    match run_shell_env_probe(shell_bin, "-lic", interactive_timeout).await {
        Ok(vars) if !vars.is_empty() => Ok(vars),
        Ok(_) | Err(_) => {
            // Fallback to login non-interactive `-lc` in case `.zshrc` exited early or timed out.
            run_shell_env_probe(shell_bin, "-lc", LOGIN_FALLBACK_TIMEOUT).await
        }
    }
}

pub(crate) async fn run_shell_env_probe(
    shell_bin: &str,
    mode_flag: &str,
    timeout_dur: Duration,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let token = PROBE_TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    let start_marker = format!("__OTTO_SHELL_ENV_BEGIN_{}_{}__", std::process::id(), token);
    let end_marker = format!("__OTTO_SHELL_ENV_END_{}_{}__", std::process::id(), token);
    let env_bin = if Path::new("/usr/bin/env").is_file() {
        "/usr/bin/env"
    } else {
        "env"
    };
    let script = format!("printf '%s' '{start_marker}'; {env_bin} -0; printf '%s' '{end_marker}'");

    let mut cmd = Command::new(shell_bin);
    cmd.args([mode_flag, &script])
        .env("TERM", "dumb")
        .env("OTTO_SHELL_ENV_PROBE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    if let Some(home) = dirs::home_dir() {
        if home.is_dir() {
            cmd.current_dir(&home);
        }
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn '{shell_bin} {mode_flag}': {e}"))?;
    let child_pid = child.id();
    let mut guard = ShellProbeGuard {
        pid: child_pid,
        disarmed: false,
    };

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "missing stdout pipe from shell probe".to_string())?;

    let read_fut = async {
        let mut buf = Vec::with_capacity(8192);
        let mut chunk = [0u8; 4096];
        loop {
            let n = stdout
                .read(&mut chunk)
                .await
                .map_err(|e| format!("read shell probe stdout: {e}"))?;
            if n == 0 {
                break;
            }
            let remaining = MAX_SHELL_STDOUT_BYTES.saturating_sub(buf.len());
            let take = n.min(remaining);
            buf.extend_from_slice(&chunk[..take]);
            if buf.len() >= MAX_SHELL_STDOUT_BYTES {
                break;
            }
            // As soon as we see the closing sentinel we have the entire `env -0` payload!
            if find_subslice(&buf, end_marker.as_bytes()).is_some() {
                break;
            }
        }
        Ok::<Vec<u8>, String>(buf)
    };

    let raw_bytes = match tokio::time::timeout(timeout_dur, read_fut).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(err)) => {
            terminate_shell_process_group(&mut child, child_pid).await;
            guard.disarmed = true;
            return Err(err);
        }
        Err(_) => {
            terminate_shell_process_group(&mut child, child_pid).await;
            guard.disarmed = true;
            return Err(format!(
                "shell environment probe '{shell_bin} {mode_flag}' timed out after {}ms",
                timeout_dur.as_millis()
            ));
        }
    };

    terminate_shell_process_group(&mut child, child_pid).await;
    guard.disarmed = true;

    parse_sentinel_env_zero(&raw_bytes, &start_marker, &end_marker)
}

/// Parse `env -0` NUL-delimited `KEY=VALUE` entries strictly between
/// `start_marker` and `end_marker`, ignoring any noisy `.zshrc` banners,
/// MOTD messages, ANSI color codes, or trailing shell exit hooks.
pub fn parse_sentinel_env_zero(
    raw: &[u8],
    start_marker: &str,
    end_marker: &str,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let start_bytes = start_marker.as_bytes();
    let end_bytes = end_marker.as_bytes();

    let start_pos = find_subslice(raw, start_bytes)
        .ok_or_else(|| "shell probe output missing start sentinel marker".to_string())?;
    let payload_start = start_pos + start_bytes.len();
    let tail = &raw[payload_start..];
    let end_rel = find_subslice(tail, end_bytes)
        .ok_or_else(|| "shell probe output missing end sentinel marker".to_string())?;
    let payload = &tail[..end_rel];

    let mut out = BTreeMap::new();
    for entry in payload.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        let Some(eq_idx) = entry.iter().position(|&b| b == b'=') else {
            continue;
        };
        let Ok(key) = std::str::from_utf8(&entry[..eq_idx]) else {
            continue;
        };
        let val = String::from_utf8_lossy(&entry[eq_idx + 1..]).into_owned();
        if should_keep_env_entry(key, &val) {
            out.insert(key.to_string(), val);
            if out.len() >= MAX_CACHED_ENV_VARS {
                break;
            }
        }
    }
    Ok(out)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

struct ShellProbeGuard {
    pid: Option<u32>,
    disarmed: bool,
}

impl Drop for ShellProbeGuard {
    #[allow(clippy::disallowed_methods)] // synchronous Drop safeguard when task is cancelled
    fn drop(&mut self) {
        if !self.disarmed {
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

async fn terminate_shell_process_group(child: &mut tokio::process::Child, pid: Option<u32>) {
    if tokio::time::timeout(Duration::from_millis(40), child.wait())
        .await
        .is_ok()
    {
        return;
    }
    #[cfg(unix)]
    if let Some(p) = pid {
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &format!("-{p}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_millis(100), child.wait()).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_parser_ignores_zshrc_motd_banners_ansi_and_multiline_values() {
        let start = "__OTTO_START_1__";
        let end = "__OTTO_END_1__";
        let mut raw = Vec::new();
        raw.extend_from_slice(b"\x1b[32mWelcome to Oh My Zsh!\x1b[0m\nFAKE_VAR=corrupted\n");
        raw.extend_from_slice(start.as_bytes());
        raw.extend_from_slice(b"GO_DEBUGGING_PRODAMS=host=127.0.0.1;env=qa\0");
        raw.extend_from_slice(b"GOPRIVATE=bitbucket.org/gamescale-rnd/*\0");
        raw.extend_from_slice(b"MULTILINE_CERT=-----BEGIN-----\nline2\n-----END-----\0");
        raw.extend_from_slice(b"PWD=/should/be/filtered\0");
        raw.extend_from_slice(b"SHLVL=3\0");
        raw.extend_from_slice(b"PATH=/Users/dev/go/bin:/opt/homebrew/bin:/usr/bin\0");
        raw.extend_from_slice(end.as_bytes());
        raw.extend_from_slice(b"\nGoodbye from .zlogout!\nTRAILING=ignored\0");

        let parsed = parse_sentinel_env_zero(&raw, start, end).unwrap();
        assert_eq!(
            parsed.get("GO_DEBUGGING_PRODAMS").map(String::as_str),
            Some("host=127.0.0.1;env=qa")
        );
        assert_eq!(
            parsed.get("GOPRIVATE").map(String::as_str),
            Some("bitbucket.org/gamescale-rnd/*")
        );
        assert_eq!(
            parsed.get("MULTILINE_CERT").map(String::as_str),
            Some("-----BEGIN-----\nline2\n-----END-----")
        );
        assert_eq!(
            parsed.get("PATH").map(String::as_str),
            Some("/Users/dev/go/bin:/opt/homebrew/bin:/usr/bin")
        );
        assert!(!parsed.contains_key("FAKE_VAR"));
        assert!(!parsed.contains_key("TRAILING"));
        assert!(!parsed.contains_key("PWD"));
        assert!(!parsed.contains_key("SHLVL"));
    }

    #[tokio::test]
    async fn shell_probe_captures_exported_vars_and_kills_hanging_shell_within_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let noisy_shell = tmp.path().join("mock_zsh.sh");
        std::fs::write(
            &noisy_shell,
            "#!/bin/sh\n\
             echo 'MOTD: loading custom zsh profile...'\n\
             export GO_DEBUGGING_PRODAMS='enabled-from-zshrc'\n\
             export GOPRIVATE='github.com/acme/*'\n\
             exec /bin/sh -c \"$2\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&noisy_shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let vars = run_shell_env_probe(
            noisy_shell.to_str().unwrap(),
            "-lic",
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(
            vars.get("GO_DEBUGGING_PRODAMS").map(String::as_str),
            Some("enabled-from-zshrc")
        );
        assert_eq!(
            vars.get("GOPRIVATE").map(String::as_str),
            Some("github.com/acme/*")
        );

        // Verify a hanging shell that spawns a child sleep is killed cleanly on timeout.
        let hanging_shell = tmp.path().join("hanging_zsh.sh");
        std::fs::write(&hanging_shell, "#!/bin/sh\nsleep 30 &\nwait\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hanging_shell, std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }

        let started = Instant::now();
        let res = run_shell_env_probe(
            hanging_shell.to_str().unwrap(),
            "-lic",
            Duration::from_millis(200),
        )
        .await;
        assert!(res.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
