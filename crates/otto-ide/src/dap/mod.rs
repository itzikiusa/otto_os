//! Full Debug Adapter Protocol (DAP) Engine (`crates/otto-ide/src/dap`).
//!
//! Architecture:
//! - [`wire`]: Hardened `Content-Length` DAP framing (`MAX_MESSAGE_BYTES = 16 MiB`).
//! - [`adapters`]: Language-aware adapter discovery & launch/attach builders for
//!   Go (`dlv dap`), Python (`debugpy.adapter` + zero-install stdlib `bdb` bridge),
//!   Rust (`codelldb` / `lldb-dap` + `cargo --message-format=json` artifact locator),
//!   JavaScript/TypeScript (`js-debug` + zero-install Node V8 Inspector bridge),
//!   Java (`java-debug` + zero-install `jdb`/JDWP bridge), and runtime IDE plugins.
//! - [`session`]: Stateful [`DapManager`] managing live debug sessions, breakpoint
//!   sync, stepping, variable expansion, REPL evaluation, automatic `stopped`
//!   inspection, IntelliJ-style inline ghost variable values, and SSE broadcasts.

pub mod adapters;
pub(crate) mod adapters_env;
pub(crate) mod adapters_go_py;
pub(crate) mod adapters_jvm_js;
pub(crate) mod adapters_py;
pub(crate) mod adapters_rust_sh;
pub(crate) mod adapters_shell;
pub(crate) mod java_bridge;
pub(crate) mod lldb_bridge;
pub(crate) mod node_bridge;
pub(crate) mod py_bridge;
pub(crate) mod rust_build_cache;
mod rust_build_depinfo;
pub mod session;
pub(crate) mod session_entry;
pub(crate) mod session_inline;
pub(crate) mod session_inspect;
pub(crate) mod session_transport;
pub(crate) mod shell_bridge;
#[cfg(unix)]
mod shell_bridge_eval;
mod shell_prelude;
pub mod wire;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_core::{Error, Result};
use otto_ssh::{SshTunnel, SshTunnelConfig};
use tokio::process::{Child, Command};

use crate::types::{IdeSshSpec, IdeTargetKind, IdeTargetSpec};

pub use adapters::{resolve_dap_adapter, DapAdapterTransport, ResolvedDapAdapter};
pub use java_bridge::JavaJdwpBridgeSpec;
pub type JavaJdbBridgeSpec = JavaJdwpBridgeSpec;
pub use node_bridge::NodeInspectorBridgeSpec;
pub use session::{DapManager, DapStartContext};
pub use shell_bridge::ShellBridgeSpec;
pub use wire::{DapEvent, DapMessage, DapReader, DapRequest, DapResponse, DapWriter};

/// Owns background child processes, Unix process-group IDs, temp IPC dirs, and SSH tunnels.
#[derive(Default)]
pub(crate) struct BridgeGuard {
    children: Mutex<Vec<Child>>,
    pgids: Mutex<Vec<u32>>,
    tunnels: Mutex<Vec<SshTunnel>>,
    temp_dirs: Mutex<Vec<PathBuf>>,
}

impl BridgeGuard {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn push_child(&self, child: Child) {
        if let Some(pid) = child.id() {
            if let Ok(mut pg) = self.pgids.lock() {
                pg.push(pid);
            }
        }
        if let Ok(mut guard) = self.children.lock() {
            guard.push(child);
        }
    }

    pub(crate) fn push_tunnel(&self, tunnel: SshTunnel) {
        if let Ok(mut guard) = self.tunnels.lock() {
            guard.push(tunnel);
        }
    }

    pub(crate) fn push_temp_dir(&self, dir: PathBuf) {
        if let Ok(mut guard) = self.temp_dirs.lock() {
            guard.push(dir);
        }
    }

    pub(crate) fn pids(&self) -> Vec<u32> {
        self.pgids.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub(crate) fn kill_all(&self) {
        #[cfg(unix)]
        let pgids_to_kill: Vec<u32> = self
            .pgids
            .lock()
            .map(|mut g| g.drain(..).collect())
            .unwrap_or_default();
        if let Ok(mut guard) = self.children.lock() {
            for child in guard.iter_mut() {
                let _ = child.start_kill();
            }
            guard.clear();
        }
        if let Ok(mut guard) = self.tunnels.lock() {
            guard.clear();
        }
        let dirs_to_remove: Vec<PathBuf> = self
            .temp_dirs
            .lock()
            .map(|mut d| d.drain(..).collect())
            .unwrap_or_default();

        #[cfg(unix)]
        for pgid in pgids_to_kill {
            crate::lsp_host::pool_spawn::terminate_process_group(pgid);
        }

        if !dirs_to_remove.is_empty() {
            std::thread::spawn(move || {
                for d in dirs_to_remove {
                    #[allow(clippy::disallowed_methods)] // dedicated cleanup OS thread
                    let _ = std::fs::remove_dir_all(d);
                }
            });
        }
    }
}

impl Drop for BridgeGuard {
    fn drop(&mut self) {
        self.kill_all();
    }
}

/// Validate and convert [`IdeSshSpec`] into [`SshTunnelConfig`].
pub(crate) fn ssh_tunnel_config(ssh: &IdeSshSpec) -> Result<SshTunnelConfig> {
    ssh.to_ssh_tunnel_config()
}

/// Safe POSIX single-quote shell escaping for remote SSH command strings.
pub(crate) fn shell_quote(raw: &str) -> String {
    crate::target::ssh_exec::posix_quote(raw)
}

fn build_ssh_remote_command(
    ssh: &IdeSshSpec,
    command: &str,
    args: &[String],
    cwd: &str,
    env: &BTreeMap<String, String>,
    kill_on_stdin_eof: bool,
) -> Result<(Command, Option<Vec<u8>>)> {
    for k in env.keys() {
        if !crate::target::ssh_exec::valid_env_key(k) {
            return Err(Error::Invalid(format!(
                "invalid environment variable name '{k}'"
            )));
        }
    }
    let (mut ssh_args, dest) = crate::target::ssh_exec::build_ssh_cli_args(ssh, None)?;
    let exec_line = crate::target::ssh_exec::build_remote_exec_line(cwd, command, args, env);
    let env_preamble = crate::target::ssh_exec::build_remote_stdin_env_preamble(env);
    let remote_body = if kill_on_stdin_eof {
        crate::target::ssh_exec::wrap_remote_stdio_with_eof_kill_and_env(
            &exec_line,
            env_preamble.as_ref().map(|(r, _)| r.as_str()),
        )
    } else {
        match env_preamble.as_ref() {
            Some((reader, _)) => format!("{reader} {exec_line}"),
            None => exec_line,
        }
    };
    ssh_args.push("--".into());
    ssh_args.push(dest);
    ssh_args.push(format!("sh -lc {}", shell_quote(&remote_body)));
    let mut cmd = Command::new("ssh");
    cmd.args(&ssh_args).kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    Ok((cmd, env_preamble.map(|(_, b)| b)))
}

/// Spawn a child process with piped stdio on either a Local or Remote SSH target.
///
/// Clears ambient daemon environment variables before applying `env` so non-root
/// debug subprocesses never inherit daemon credentials or cloud tokens.
pub(crate) async fn spawn_target_process(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    command: &str,
    args: &[String],
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> Result<Child> {
    target.validate()?;
    match target.kind {
        IdeTargetKind::Local => {
            let mut cmd = Command::new(command);
            cmd.args(args)
                .current_dir(cwd)
                .env_clear()
                .envs(crate::environments::shell_env::collect_safe_non_root_base_env())
                .envs(env)
                .kill_on_drop(true)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(unix)]
            cmd.process_group(0);
            cmd.spawn()
                .map_err(|e| Error::Upstream(format!("failed to spawn '{command}': {e}")))
        }
        IdeTargetKind::Ssh => {
            let spec = ssh.ok_or_else(|| {
                Error::Invalid("SSH connection metadata required for remote debug target".into())
            })?;
            let (mut cmd, preamble) =
                build_ssh_remote_command(spec, command, args, cwd, env, true)?;
            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| {
                Error::Upstream(format!("failed to spawn remote '{command}' via ssh: {e}"))
            })?;
            crate::target::ssh_exec::write_ssh_env_preamble(&mut child, preamble, false).await?;
            Ok(child)
        }
    }
}

/// Execute a bounded probe/build command on the target and capture stdout/stderr.
pub(crate) async fn run_target_command(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    command: &str,
    args: &[String],
    cwd: &str,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<std::process::Output> {
    target.validate()?;
    let child = match target.kind {
        IdeTargetKind::Local => {
            let mut cmd = Command::new(command);
            cmd.args(args)
                .current_dir(cwd)
                .env_clear()
                .envs(crate::environments::shell_env::collect_safe_non_root_base_env())
                .envs(env)
                .kill_on_drop(true)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(unix)]
            cmd.process_group(0);
            cmd.spawn()
                .map_err(|e| Error::Upstream(format!("failed to spawn '{command}': {e}")))?
        }
        IdeTargetKind::Ssh => {
            let spec = ssh.ok_or_else(|| {
                Error::Invalid("SSH connection metadata required for remote debug target".into())
            })?;
            let (mut cmd, preamble) =
                build_ssh_remote_command(spec, command, args, cwd, env, false)?;
            if preamble.is_some() {
                cmd.stdin(Stdio::piped());
            } else {
                cmd.stdin(Stdio::null());
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            let mut c = cmd.spawn().map_err(|e| {
                Error::Upstream(format!("failed to spawn remote '{command}' via ssh: {e}"))
            })?;
            if preamble.is_some() {
                crate::target::ssh_exec::write_ssh_env_preamble(&mut c, preamble, true).await?;
            }
            c
        }
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(Error::Upstream(format!("'{command}' failed: {e}"))),
        Err(_) => Err(Error::Upstream(format!(
            "'{command}' timed out after {}s",
            timeout.as_secs()
        ))),
    }
}

/// Read source file text safely (local disk confined to workspace roots + `secret_paths` guard,
/// or remote SSH) for computing IntelliJ-style inline ghost variable values around `paused_line`.
pub(crate) async fn read_target_file_for_inline(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    path: &str,
    caller_is_root: bool,
) -> Option<String> {
    const MAX_INLINE_FILE_BYTES: u64 = 512 * 1024;
    match target.kind {
        IdeTargetKind::Local => {
            let canon = tokio::fs::canonicalize(path).await.ok()?;
            if !caller_is_root {
                if crate::target::enforce_non_root_path(&canon).is_err() {
                    return None;
                }
                let allowed = std::iter::once(target.root.clone())
                    .chain(target.merged_extra_roots())
                    .any(|r| {
                        std::fs::canonicalize(&r)
                            .map(|cr| canon.starts_with(&cr))
                            .unwrap_or(false)
                    });
                if !allowed {
                    return None;
                }
            }
            let meta = tokio::fs::metadata(&canon).await.ok()?;
            if !meta.is_file() || meta.len() > MAX_INLINE_FILE_BYTES {
                return None;
            }
            tokio::fs::read_to_string(&canon).await.ok()
        }
        IdeTargetKind::Ssh => {
            let args = vec![
                "-c".to_string(),
                MAX_INLINE_FILE_BYTES.to_string(),
                "--".to_string(),
                path.to_string(),
            ];
            let out = run_target_command(
                target,
                ssh,
                "head",
                &args,
                &target.root,
                &BTreeMap::new(),
                Duration::from_secs(4),
            )
            .await
            .ok()?;
            if !out.status.success() {
                return None;
            }
            String::from_utf8(out.stdout).ok()
        }
    }
}
