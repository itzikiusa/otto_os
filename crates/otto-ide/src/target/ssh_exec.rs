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
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};

use crate::lsp_host::pool_spawn::kill_process_group_sync;
use crate::types::IdeSshSpec;

use super::fs_ops::resolve_local_existing_in_roots;
pub(crate) use super::path_resolve::*;
use super::{ExecOutput, ResolvedTarget, SshPoolKey, TargetManager, SFTP_IDLE_TTL};

const SSH_POOL_CAPACITY: usize = 32;
pub(crate) const MAX_EXEC_STDOUT_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_EXEC_STDERR_BYTES: usize = 2 * 1024 * 1024;

/// Reaps a spawned child's process group (`-pgid`) on drop unless disarmed
/// after normal exit — guarantees timed-out or cancelled subtrees never linger.
pub(crate) struct ProcessGroupKillGuard(Option<u32>);

impl ProcessGroupKillGuard {
    pub(crate) fn new(pid: Option<u32>) -> Self {
        Self(pid)
    }

    pub(crate) fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for ProcessGroupKillGuard {
    fn drop(&mut self) {
        if let Some(pgid) = self.0.take() {
            #[cfg(unix)]
            kill_process_group_sync(pgid);
            #[cfg(not(unix))]
            let _ = pgid;
        }
    }
}

/// Read up to `cap` bytes from `reader`, draining any excess to EOF so the
/// child process never blocks on a full pipe while keeping memory bounded.
pub(crate) async fn read_pipe_capped<R: AsyncRead + Unpin>(
    mut reader: R,
    cap: usize,
) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        let room = cap.saturating_sub(out.len());
        if room > 0 {
            out.extend_from_slice(&chunk[..n.min(room)]);
        }
    }
    Ok(out)
}

/// Atomically create a private `0o700` directory under `base` (`O_EXCL`-like:
/// fails and retries with a new sequence number if a path already exists, so a
/// pre-created symlink/directory in `/tmp` is never reused).
pub(crate) fn create_private_temp_dir(base: &Path, prefix: &str) -> Result<PathBuf> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    for _ in 0..64 {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("{prefix}-{pid}-{n}"));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Internal(format!("create ssh control dir: {e}"))),
        }
    }
    Err(Error::Internal(
        "create ssh control dir: exhausted unique directory attempts".into(),
    ))
}

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
        let base = if Path::new("/tmp").is_dir() {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let dir = create_private_temp_dir(&base, "oi")?;
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
        let (mut cmd, preamble) =
            self.build_piped_command_with_preamble(target, program, args, cwd, env)?;
        if preamble.is_none() {
            cmd.stdin(Stdio::null());
        }
        run_command_bounded(cmd, preamble, timeout).await
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
    /// The returned preamble MUST be written with `write_ssh_env_preamble`.
    pub(crate) fn build_piped_command_with_preamble(
        &self,
        target: &ResolvedTarget,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
    ) -> Result<(Command, Option<Vec<u8>>)> {
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
                #[cfg(unix)]
                cmd.process_group(0);
                Ok((cmd, None))
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
                let (remote_cmd, preamble) =
                    build_remote_exec_with_stdin_env(remote_cwd, program, args, env);
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
                #[cfg(unix)]
                cmd.process_group(0);
                Ok((cmd, preamble))
            }
        }
    }

    /// Build a multiplexed SSH `Command` for long-lived run tasks whose stdin is
    /// unused: keeps local stdin piped so killing the local `ssh` client delivers
    /// immediate EOF to a remote watcher that sends `SIGTERM` to the remote process.
    pub(crate) fn build_ssh_run_command_with_preamble(
        &self,
        spec: &IdeSshSpec,
        actor_id: &str,
        cwd: &str,
        program: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
    ) -> Result<(Command, Option<Vec<u8>>)> {
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
        let env_preamble = build_remote_stdin_env_preamble(env);
        let wrapped = wrap_remote_null_stdin_with_eof_kill_and_env(
            &exec_line,
            env_preamble.as_ref().map(|(r, _)| r.as_str()),
        );
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
        #[cfg(unix)]
        cmd.process_group(0);
        Ok((cmd, env_preamble.map(|(_, b)| b)))
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
        let (mut cmd, preamble) =
            self.build_piped_command_with_preamble(target, program, args, cwd, env)?;
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Upstream(format!("failed to spawn '{program}': {e}")))?;
        write_ssh_env_preamble(&mut child, preamble, false).await?;
        Ok(child)
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

/// Format non-`PATH` environment variables as POSIX `export KEY=VALUE` lines to be
/// streamed over SSH `stdin` (never placed in `ssh` or remote `sh -c` `argv`).
pub(crate) fn format_remote_env_exports(env: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    for (k, v) in env.iter().filter(|(k, _)| k.as_str() != "PATH") {
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
            posix_quote(v)
        };
        out.push_str(&format!("export {k}={quoted_val}\n"));
    }
    out
}

/// Build a sentinel-delimited POSIX `sh` `stdin` reader preamble and matching byte
/// payload for non-`PATH` environment variables so secret values never appear in
/// local `ps aux` or remote `/proc/<pid>/cmdline`.
pub(crate) fn build_remote_stdin_env_preamble(
    env: &BTreeMap<String, String>,
) -> Option<(String, Vec<u8>)> {
    let exports = format_remote_env_exports(env);
    if exports.is_empty() {
        return None;
    }
    let sentinel = format!("__OTTO_ENV_{:032x}__", rand::random::<u128>());
    let reader = format!(
        "_oes=\"\"; while IFS= read -r _ol && [ \"$_ol\" != '{sentinel}' ]; do _oes=\"${{_oes}}${{_ol}}\n\"; done; eval \"$_oes\"; unset _oes _ol;"
    );
    let payload = format!("{exports}{sentinel}\n").into_bytes();
    Some((reader, payload))
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

pub(crate) fn build_remote_exec_with_stdin_env(
    cwd: &str,
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> (String, Option<Vec<u8>>) {
    let exec_line = build_remote_exec_line(cwd, program, args, env);
    match build_remote_stdin_env_preamble(env) {
        Some((reader, payload)) => (format!("{reader} {exec_line}"), Some(payload)),
        None => (exec_line, None),
    }
}

/// Wrap a remote run command whose stdin is unused (`</dev/null`): duplicates
/// outer SSH `stdin` onto `fd 4` BEFORE spawning `&` background jobs because
/// non-interactive POSIX `sh` implicitly redirects `fd 0` of `&` jobs to `/dev/null`.
#[cfg(test)]
pub(crate) fn wrap_remote_null_stdin_with_eof_kill(exec_line: &str) -> String {
    wrap_remote_null_stdin_with_eof_kill_and_env(exec_line, None)
}

pub(crate) fn wrap_remote_null_stdin_with_eof_kill_and_env(
    exec_line: &str,
    env_reader: Option<&str>,
) -> String {
    let prefix = match env_reader {
        Some(r) => format!("{r} "),
        None => String::new(),
    };
    format!(
        "{prefix}exec 4<&0; \
         ( exec 4<&-; {exec_line} ) </dev/null & pid=$!; \
         ( cat <&4 >/dev/null; kill -TERM \"$pid\" ) >/dev/null 2>&1 & watcher=$!; \
         exec 4<&-; wait \"$pid\"; rc=$?; kill \"$watcher\" 2>/dev/null; exit \"$rc\""
    )
}

/// Wrap a remote command that reads binary frames on `stdin` (DAP/LSP stdio adapters)
/// or listens on TCP (`dlv`, JDWP, Node inspector) so closing the local `ssh` pipe
/// cleanly terminates the remote process without `-tt` PTY newline mangling.
pub(crate) fn wrap_remote_stdio_with_eof_kill(exec_line: &str) -> String {
    wrap_remote_stdio_with_eof_kill_and_env(exec_line, None)
}

pub(crate) fn wrap_remote_stdio_with_eof_kill_and_env(
    exec_line: &str,
    env_reader: Option<&str>,
) -> String {
    let prefix = match env_reader {
        Some(r) => format!("{r} "),
        None => String::new(),
    };
    format!(
        "{prefix}exec 4<&0; \
         fifo=\"/tmp/.otto-ide-$$-$(date +%s 2>/dev/null || echo 0)\"; \
         mkfifo -m 600 \"$fifo\" || exit 1; \
         ( exec 4<&-; {exec_line} ) <\"$fifo\" & pid=$!; \
         exec 3>\"$fifo\"; rm -f \"$fifo\"; \
         ( cat <&4 >&3; exec 3>&-; sleep 1; kill -TERM \"$pid\" ) >/dev/null 2>&1 & watcher=$!; \
         exec 3>&- 4<&-; wait \"$pid\"; rc=$?; kill \"$watcher\" 2>/dev/null; exit \"$rc\""
    )
}

pub(crate) async fn write_ssh_env_preamble(
    child: &mut Child,
    preamble: Option<Vec<u8>>,
    close_stdin_after: bool,
) -> Result<()> {
    if let Some(bytes) = preamble {
        use tokio::io::AsyncWriteExt;
        if close_stdin_after {
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(&bytes)
                    .await
                    .map_err(|e| Error::Upstream(format!("write ssh env preamble: {e}")))?;
                let _ = stdin.shutdown().await;
            }
        } else if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(&bytes)
                .await
                .map_err(|e| Error::Upstream(format!("write ssh env preamble: {e}")))?;
            stdin
                .flush()
                .await
                .map_err(|e| Error::Upstream(format!("flush ssh env preamble: {e}")))?;
        }
    } else if close_stdin_after {
        let _ = child.stdin.take();
    }
    Ok(())
}

pub(crate) async fn run_command_bounded_with_caps(
    mut cmd: Command,
    preamble: Option<Vec<u8>>,
    timeout: Duration,
    stdout_cap: usize,
    stderr_cap: usize,
) -> Result<ExecOutput> {
    #[cfg(unix)]
    cmd.process_group(0);
    cmd.kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Upstream(format!("failed to start process: {e}")))?;
    let mut guard = ProcessGroupKillGuard::new(child.id());
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let work = async {
        let feed = async move {
            if let Some(bytes) = preamble {
                if let Some(mut si) = stdin {
                    use tokio::io::AsyncWriteExt;
                    if let Err(e) = si.write_all(&bytes).await {
                        if e.kind() != std::io::ErrorKind::BrokenPipe {
                            return Err(Error::Upstream(format!("write ssh env preamble: {e}")));
                        }
                    }
                    let _ = si.shutdown().await;
                }
            } else {
                drop(stdin);
            }
            Ok(())
        };
        let read_out = async move {
            match stdout {
                Some(r) => read_pipe_capped(r, stdout_cap).await,
                None => Ok(Vec::new()),
            }
        };
        let read_err = async move {
            match stderr {
                Some(r) => read_pipe_capped(r, stderr_cap).await,
                None => Ok(Vec::new()),
            }
        };
        let (fed, out_res, err_res) = tokio::join!(feed, read_out, read_err);
        fed?;
        let stdout_bytes =
            out_res.map_err(|e| Error::Upstream(format!("read process stdout: {e}")))?;
        let stderr_bytes =
            err_res.map_err(|e| Error::Upstream(format!("read process stderr: {e}")))?;
        let status = child
            .wait()
            .await
            .map_err(|e| Error::Upstream(format!("process wait failed: {e}")))?;
        Ok(ExecOutput {
            exit_code: status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
            stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        })
    };

    match tokio::time::timeout(timeout, work).await {
        Ok(res) => {
            guard.disarm();
            res
        }
        Err(_) => Err(Error::Upstream(format!(
            "command timed out after {}s",
            timeout.as_secs()
        ))),
    }
}

async fn run_command_bounded(
    cmd: Command,
    preamble: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<ExecOutput> {
    run_command_bounded_with_caps(
        cmd,
        preamble,
        timeout,
        MAX_EXEC_STDOUT_BYTES,
        MAX_EXEC_STDERR_BYTES,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_private_temp_dir_sets_0700_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = create_private_temp_dir(tmp.path(), "oi").unwrap();
        assert!(dir.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "ssh control dir must be 0o700, got {mode:#o}");
        }
    }

    #[tokio::test]
    async fn run_command_bounded_caps_output_and_reaps_timeout_subtree() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "yes abcdefgh | head -c 100000"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let out = run_command_bounded_with_caps(cmd, None, Duration::from_secs(5), 256, 256)
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.stdout.len(), 256);

        let mut slow = Command::new("/bin/sh");
        slow.args(["-c", "sleep 30 & wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let start = tokio::time::Instant::now();
        let err = run_command_bounded_with_caps(slow, None, Duration::from_millis(150), 256, 256)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"), "unexpected: {err}");
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
