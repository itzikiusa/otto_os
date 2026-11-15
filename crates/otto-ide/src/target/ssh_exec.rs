//! SSH process execution and multiplexed ControlMaster sockets for [`super::TargetManager`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::{Error, Result};
use otto_ssh::{SftpParams, SftpSession};
use tokio::process::{Child, Command};

use crate::types::IdeSshSpec;

use super::fs_ops::resolve_local_existing_in_roots;
pub(crate) use super::path_resolve::*;
use super::{ExecOutput, ResolvedTarget, SshPoolKey, TargetManager, SFTP_IDLE_TTL};

const SSH_POOL_CAPACITY: usize = 32;

const REMOTE_TOOLCHAIN_PATH_EXPRS: &[&str] = &[
    "\"$HOME\"/.otto/toolchains/java/current/bin",
    "\"$HOME\"/.otto/toolchains/go/current/bin",
    "\"$HOME\"/.otto/toolchains/go/bin",
    "\"$HOME\"/.otto/toolchains/node/current/bin",
    "\"$HOME\"/.otto/toolchains/python/current/bin",
    "\"$HOME\"/.otto/toolchains/python/bin",
    "\"$HOME\"/.otto/toolchains/rust/cargo/bin",
    "\"$HOME\"/.otto/toolchains/rust/bin",
    "\"$HOME\"/.otto/toolchains/shell/bin",
    "\"$HOME\"/.otto/toolchains/json/bin",
    "\"$HOME\"/go/bin",
    "\"$HOME\"/.cargo/bin",
    "\"$HOME\"/.local/bin",
];

/// Private ControlMaster socket directory for multiplexed `ssh` exec/stdio spawns.
pub(crate) struct SshControlSocket {
    dir: PathBuf,
    pub(super) ctl_path: String,
    pub(super) ssh_target: String,
}

impl SshControlSocket {
    pub(crate) fn new(spec: &IdeSshSpec) -> Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let base = if Path::new("/tmp").is_dir() {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let dir = base.join(format!("oi-{pid}-{n}"));
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Internal(format!("create ssh control dir: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| Error::Internal(format!("secure ssh control dir: {e}")))?;
        }
        let ctl_path = dir.join("c.s").to_string_lossy().into_owned();
        let ssh_target = if spec.user.is_empty() {
            spec.host.clone()
        } else {
            format!("{}@{}", spec.user, spec.host)
        };
        Ok(Self {
            dir,
            ctl_path,
            ssh_target,
        })
    }
}

impl Drop for SshControlSocket {
    fn drop(&mut self) {
        let dir = self.dir.clone();
        let ctl = self.ctl_path.clone();
        let target = self.ssh_target.clone();
        std::thread::spawn(move || {
            if Path::new(&ctl).exists() {
                let ctl_opt = format!("ControlPath={ctl}");
                if let Ok(mut child) = std::process::Command::new("ssh")
                    .args(["-o", &ctl_opt, "-O", "exit", "--", &target])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    let _ = child.wait();
                }
            }
            #[allow(clippy::disallowed_methods)]
            let _ = std::fs::remove_dir_all(dir);
        });
    }
}

impl TargetManager {
    /// Execute a bounded command (`program` + `args`) on `target`.
    pub async fn exec_command(
        &self,
        target: &ResolvedTarget,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecOutput> {
        let mut cmd = self.build_piped_command(target, program, args, cwd, env)?;
        cmd.stdin(Stdio::null());
        run_command_bounded(cmd, timeout).await
    }

    /// Execute a bounded POSIX `sh -c <script>` command on `target`.
    pub async fn exec_shell(
        &self,
        target: &ResolvedTarget,
        script: &str,
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecOutput> {
        let args = vec!["-c".to_string(), script.to_string()];
        self.exec_command(target, "sh", &args, cwd, env, timeout)
            .await
    }

    /// Build a `tokio::process::Command` with piped `stdin`/`stdout`/`stderr`
    /// and `kill_on_drop(true)` targeting either local OS or multiplexed SSH.
    pub fn build_piped_command(
        &self,
        target: &ResolvedTarget,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
    ) -> Result<Command> {
        if program.trim().is_empty() || program.chars().any(char::is_control) {
            return Err(Error::Invalid("invalid command program name".into()));
        }
        for k in env.keys() {
            if !valid_env_key(k) {
                return Err(Error::Invalid(format!(
                    "invalid environment variable name '{k}'"
                )));
            }
        }
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let work_dir = match cwd.filter(|c| !c.trim().is_empty()) {
                    Some(c) => {
                        resolve_local_existing_in_roots(root, additional_roots, *is_root_user, c)?.0
                    }
                    None => root.clone(),
                };
                let mut cmd = Command::new(program);
                // Prevent daemon/host secrets from leaking into non-root child processes.
                if !*is_root_user {
                    cmd.env_clear();
                    cmd.envs(crate::environments::shell_env::collect_safe_non_root_base_env());
                }
                cmd.args(args)
                    .current_dir(work_dir)
                    .envs(env)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                Ok(cmd)
            }
            ResolvedTarget::Ssh {
                spec,
                root,
                actor_id,
                ..
            } => {
                let ctl = self.acquire_ssh_socket(actor_id, spec)?;
                let remote_cwd = cwd
                    .filter(|c| !c.trim().is_empty())
                    .unwrap_or(root.as_str());
                let remote_cmd = build_remote_exec_line(remote_cwd, program, args, env);
                let (mut ssh_args, dest) = build_ssh_cli_args(spec, Some(&ctl.ctl_path))?;
                ssh_args.push("--".into());
                ssh_args.push(dest);
                ssh_args.push(remote_cmd);

                let mut cmd = Command::new("ssh");
                cmd.args(ssh_args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                Ok(cmd)
            }
        }
    }

    /// Build a multiplexed SSH `Command` for long-lived run tasks whose stdin is
    /// unused: keeps local stdin piped so killing the local `ssh` client delivers
    /// immediate EOF to a remote watcher that sends `SIGTERM` to the remote process.
    pub fn build_ssh_run_command(
        &self,
        spec: &IdeSshSpec,
        actor_id: &str,
        cwd: &str,
        program: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
    ) -> Result<Command> {
        if program.trim().is_empty() || program.chars().any(char::is_control) {
            return Err(Error::Invalid("invalid command program name".into()));
        }
        for k in env.keys() {
            if !valid_env_key(k) {
                return Err(Error::Invalid(format!(
                    "invalid environment variable name '{k}'"
                )));
            }
        }
        let ctl = self.acquire_ssh_socket(actor_id, spec)?;
        let exec_line = build_remote_exec_line(cwd, program, args, env);
        let wrapped = wrap_remote_null_stdin_with_eof_kill(&exec_line);
        let (mut ssh_args, dest) = build_ssh_cli_args(spec, Some(&ctl.ctl_path))?;
        ssh_args.push("--".into());
        ssh_args.push(dest);
        ssh_args.push(wrapped);

        let mut cmd = Command::new("ssh");
        cmd.args(ssh_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        Ok(cmd)
    }

    /// Spawn a long-lived piped child process on either local OS or multiplexed SSH.
    pub async fn spawn_piped(
        &self,
        target: &ResolvedTarget,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
    ) -> Result<Child> {
        let mut cmd = self.build_piped_command(target, program, args, cwd, env)?;
        cmd.spawn()
            .map_err(|e| Error::Upstream(format!("failed to spawn '{program}': {e}")))
    }

    pub(crate) fn acquire_sftp(
        &self,
        actor_id: &str,
        spec: &IdeSshSpec,
    ) -> Result<Arc<SftpSession>> {
        let key = build_ssh_pool_key(actor_id, spec)?;
        let params = key.params.clone();
        acquire_pooled_ssh(&self.sftp_pool, key, move || SftpSession::new(params))
    }

    pub(crate) fn acquire_ssh_socket(
        &self,
        actor_id: &str,
        spec: &IdeSshSpec,
    ) -> Result<Arc<SshControlSocket>> {
        let key = build_ssh_pool_key(actor_id, spec)?;
        acquire_pooled_ssh(&self.ssh_sockets, key, || SshControlSocket::new(spec))
    }
}

fn build_ssh_pool_key(actor_id: &str, spec: &IdeSshSpec) -> Result<SshPoolKey> {
    validate_ssh_spec(spec)?;
    let identity_file = match spec.identity_file.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => Some(
            otto_core::paths::expand_home_strict(s)
                .map_err(Error::Invalid)?
                .to_string_lossy()
                .into_owned(),
        ),
        _ => None,
    };
    Ok(SshPoolKey {
        actor_id: actor_id.to_string(),
        connection_id: spec.connection_id.to_string(),
        params: SftpParams {
            host: spec.host.clone(),
            port: spec.effective_port(),
            user: (!spec.user.is_empty()).then(|| spec.user.clone()),
            identity_file,
            jump: spec.proxy_jump.clone(),
        },
    })
}

fn acquire_pooled_ssh<T, F>(
    pool: &DashMap<SshPoolKey, (Arc<T>, Instant)>,
    key: SshPoolKey,
    create: F,
) -> Result<Arc<T>>
where
    F: FnOnce() -> Result<T>,
{
    if let Some(mut entry) = pool.get_mut(&key) {
        entry.1 = Instant::now();
        return Ok(entry.0.clone());
    }
    pool.retain(|_, (item, touched)| {
        touched.elapsed() < SFTP_IDLE_TTL || Arc::strong_count(item) > 1
    });
    while pool.len() >= SSH_POOL_CAPACITY {
        let victim = pool
            .iter()
            .filter(|e| Arc::strong_count(&e.value().0) <= 1)
            .min_by_key(|e| e.value().1)
            .or_else(|| pool.iter().min_by_key(|e| e.value().1))
            .map(|e| e.key().clone());
        let Some(k) = victim else { break };
        pool.remove(&k);
    }
    let item = Arc::new(create()?);
    pool.insert(key, (item.clone(), Instant::now()));
    Ok(item)
}

/// Shared OpenSSH CLI argument builder across `target`, `dap`, `toolchains`,
/// `environments`, and `lsp_host`. Returns `(ssh_args_without_dashdash, destination)`.
pub(crate) fn build_ssh_cli_args(
    spec: &IdeSshSpec,
    ctl_path: Option<&str>,
) -> Result<(Vec<String>, String)> {
    validate_ssh_spec(spec)?;
    let mut args: Vec<String> = [
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "LogLevel=ERROR",
        "-o",
        "ConnectTimeout=12",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=2",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    if let Some(ctl) = ctl_path {
        let ctl_opt = format!("ControlPath={ctl}");
        for s in [
            "-o",
            "ControlMaster=auto",
            "-o",
            &ctl_opt,
            "-o",
            "ControlPersist=60s",
        ] {
            args.push(s.to_string());
        }
    }
    if let Some(port) = spec.effective_port() {
        args.extend(["-p".to_string(), port.to_string()]);
    }
    if let Some(id_file) = spec
        .identity_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let expanded = otto_core::paths::expand_home_strict(id_file).map_err(Error::Invalid)?;
        args.extend(["-i".to_string(), expanded.to_string_lossy().into_owned()]);
    }
    if let Some(jump) = spec
        .proxy_jump
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.extend(["-J".to_string(), jump.to_string()]);
    }
    let dest = if spec.user.trim().is_empty() {
        spec.host.clone()
    } else {
        format!("{}@{}", spec.user, spec.host)
    };
    Ok((args, dest))
}

/// Quote a remote path or SDK variable so leading `~/` or `$HOME/` expands against
/// the remote login user's `$HOME` while the remainder stays safely single-quoted.
fn quote_remote_home_expr(val: &str) -> String {
    if val == "~" || val == "$HOME" {
        return "\"$HOME\"".to_string();
    }
    if let Some(rest) = val
        .strip_prefix("~/")
        .or_else(|| val.strip_prefix("$HOME/"))
    {
        return format!("\"$HOME\"/{}", shell_words::quote(rest));
    }
    shell_words::quote(val).into_owned()
}

pub(crate) fn build_remote_exec_line(
    cwd: &str,
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> String {
    let mut path_exprs: Vec<String> = Vec::new();
    if let Some(raw_path) = env.get("PATH") {
        for seg in raw_path.split(':').filter(|s| !s.trim().is_empty()) {
            let expr = if seg == "$PATH" {
                "\"$PATH\"".to_string()
            } else {
                quote_remote_home_expr(seg)
            };
            if !path_exprs.contains(&expr) {
                path_exprs.push(expr);
            }
        }
    }
    for builtin in REMOTE_TOOLCHAIN_PATH_EXPRS {
        let s = (*builtin).to_string();
        if !path_exprs.contains(&s) {
            path_exprs.push(s);
        }
    }
    let path_tail = "\"$PATH\"".to_string();
    if !path_exprs.contains(&path_tail) {
        path_exprs.push(path_tail);
    }

    let mut parts = Vec::new();
    parts.push(format!("export PATH={};", path_exprs.join(":")));
    parts.push(format!("cd {} &&", quote_remote_home_expr(cwd)));

    let non_path_env: Vec<(&String, &String)> =
        env.iter().filter(|(k, _)| k.as_str() != "PATH").collect();
    if !non_path_env.is_empty() {
        parts.push("env".into());
        for (k, v) in non_path_env {
            let quoted_val = if matches!(
                k.as_str(),
                "GOROOT"
                    | "GOPATH"
                    | "GOMODCACHE"
                    | "JAVA_HOME"
                    | "VIRTUAL_ENV"
                    | "CARGO_HOME"
                    | "RUSTUP_HOME"
            ) {
                quote_remote_home_expr(v)
            } else {
                shell_words::quote(v).into_owned()
            };
            parts.push(format!("{k}={quoted_val}"));
        }
    }
    parts.push("exec".into());
    let prog_expr = if program.starts_with("~/") || program.starts_with("$HOME/") {
        quote_remote_home_expr(program)
    } else {
        shell_words::quote(program).into_owned()
    };
    parts.push(prog_expr);
    for arg in args {
        parts.push(shell_words::quote(arg).into_owned());
    }
    parts.join(" ")
}

/// Wrap a remote run command whose stdin is unused (`</dev/null`): duplicates
/// outer SSH `stdin` onto `fd 4` BEFORE spawning `&` background jobs because
/// non-interactive POSIX `sh` implicitly redirects `fd 0` of `&` jobs to `/dev/null`.
pub(crate) fn wrap_remote_null_stdin_with_eof_kill(exec_line: &str) -> String {
    format!(
        "exec 4<&0; \
         ( exec 4<&-; {exec_line} ) </dev/null & pid=$!; \
         ( cat <&4 >/dev/null; kill -TERM \"$pid\" ) >/dev/null 2>&1 & watcher=$!; \
         exec 4<&-; wait \"$pid\"; rc=$?; kill \"$watcher\" 2>/dev/null; exit \"$rc\""
    )
}

/// Wrap a remote command that reads binary frames on `stdin` (DAP stdio adapters)
/// or listens on TCP (`dlv`, JDWP, Node inspector) so closing the local `ssh` pipe
/// cleanly terminates the remote process without `-tt` PTY newline mangling.
pub(crate) fn wrap_remote_stdio_with_eof_kill(exec_line: &str) -> String {
    format!(
        "exec 4<&0; \
         fifo=\"/tmp/.otto-ide-$$-$(date +%s 2>/dev/null || echo 0)\"; \
         mkfifo \"$fifo\" || exit 1; \
         ( exec 4<&-; {exec_line} ) <\"$fifo\" & pid=$!; \
         exec 3>\"$fifo\"; rm -f \"$fifo\"; \
         ( cat <&4 >&3; exec 3>&-; sleep 1; kill -TERM \"$pid\" ) >/dev/null 2>&1 & watcher=$!; \
         exec 3>&- 4<&-; wait \"$pid\"; rc=$?; kill \"$watcher\" 2>/dev/null; exit \"$rc\""
    )
}

async fn run_command_bounded(mut cmd: Command, timeout: Duration) -> Result<ExecOutput> {
    let child = cmd
        .spawn()
        .map_err(|e| Error::Upstream(format!("failed to start process: {e}")))?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => Ok(ExecOutput {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }),
        Ok(Err(e)) => Err(Error::Upstream(format!("process wait failed: {e}"))),
        Err(_) => Err(Error::Upstream(format!(
            "command timed out after {}s",
            timeout.as_secs()
        ))),
    }
}
