//! Cached User Login + Interactive Shell Environment Resolver (`ShellEnvResolver`).
//!
//! When `ottod` runs as a macOS `launchd` service or bundled Tauri sidecar inside
//! `Otto.app`, `std::env::vars()` lacks user-exported variables from `~/.zshenv`,
//! `~/.zprofile`, `~/.zshrc`, `~/.bash_profile`, and `~/.bashrc` (for example
//! `GO_DEBUGGING_PRODAMS`, `GO_DEBUGGING_PARAMS`, `GOPRIVATE`, `GONOSUMDB`,
//! `AWS_PROFILE`, `GOPATH`, and custom `PATH` entries).
//!
//! Each `IdeEngine` / `EnvironmentManager` owns its own `Arc<ShellEnvResolver>`
//! so isolated engines and tests never share process-global state.

mod shell_env_probe;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::sync::Notify;

use crate::target::TargetManager;
use crate::types::{IdeShellEnvResp, IdeSshSpec, IdeTargetSpec};

pub(crate) use self::shell_env_probe::capture_shell_env_with_shell;
use super::is_valid_env_key;

pub(super) const INTERACTIVE_SHELL_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const LOGIN_FALLBACK_TIMEOUT: Duration = Duration::from_millis(1500);
pub(super) const MAX_SHELL_STDOUT_BYTES: usize = 512 * 1024;
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

pub(super) static PROBE_TOKEN_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
struct ShellEnvState {
    status: String,
    shell: String,
    vars: BTreeMap<String, String>,
    loaded_at: Option<String>,
    duration_ms: Option<u64>,
    error: Option<String>,
}

/// Cached login+interactive shell environment resolver owned per `IdeEngine`.
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

    /// Construct a pre-seeded resolver for deterministic tests without spawning a login shell.
    pub fn with_seeded_vars(vars: BTreeMap<String, String>) -> Arc<Self> {
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
    /// warm (bounded to ~3.5s) for root-initiated Run/Debug launches.
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

    /// Awaited `KEY=VALUE` map for root Run/Debug process launches.
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

/// Scrubbed minimal host environment subset for non-root callers so daemon
/// tokens, cloud credentials, SSH agents, and login-shell secrets never leak
/// into non-root local Run, Debug, Formatter, or LSP child processes.
pub(crate) fn collect_safe_non_root_base_env() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for key in [
        "PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "TERM",
    ] {
        if let Ok(val) = std::env::var(key) {
            if should_keep_env_entry(key, &val) {
                out.insert(key.to_string(), val);
            }
        }
    }
    out
}

/// Returns `true` when an environment variable key may hold daemon-internal,
/// cloud, VCS, or API credentials that non-root callers must not inherit
/// implicitly from host/cached environments.
pub(crate) fn is_sensitive_daemon_env_key(k: &str) -> bool {
    let upper = k.to_ascii_uppercase();
    upper.starts_with("OTTO_")
        || upper.starts_with("AWS_")
        || upper.starts_with("AZURE_")
        || upper.starts_with("GCP_")
        || upper.starts_with("GOOGLE_")
        || upper == "SSH_AUTH_SOCK"
        || upper == "SSH_AGENT_PID"
        || upper.contains("SECRET")
        || upper.contains("TOKEN")
        || upper.contains("PASSWORD")
        || upper.contains("PASSWD")
        || upper.contains("API_KEY")
        || upper.contains("PRIVATE_KEY")
        || upper.contains("CREDENTIAL")
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

pub(super) fn should_keep_env_entry(key: &str, value: &str) -> bool {
    is_valid_env_key(key)
        && !IGNORED_SHELL_KEYS.contains(&key)
        && value.len() <= MAX_SINGLE_ENV_VALUE_BYTES
}

/// Parse `env -0` NUL-delimited `KEY=VALUE` entries strictly between
/// `start_marker` and `end_marker`.
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

pub(super) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
