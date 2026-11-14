//! SSH process execution, multiplexed ControlMaster sockets, port-forwarding,
//! and path validation helpers for [`super::TargetManager`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use otto_ssh::{
    valid_proxy_jump, valid_ssh_host, valid_ssh_user, SftpParams, SftpSession, SshTunnel,
    SshTunnelConfig,
};
use tokio::process::{Child, Command};

use crate::types::IdeSshSpec;

use super::fs_ops::{enforce_non_root_path, resolve_local_existing_in_roots};
use super::sdk_paths::strip_file_uri;
use super::{ExecOutput, ResolvedTarget, SshPoolKey, TargetManager, SFTP_IDLE_TTL};

const SSH_POOL_CAPACITY: usize = 32;

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
                let _ = std::process::Command::new("ssh")
                    .args([
                        "-o",
                        &format!("ControlPath={ctl}"),
                        "-O",
                        "exit",
                        "--",
                        &target,
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
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

    /// Open a local port forward (`ssh -L`) to `remote_host:remote_port` on an
    /// SSH target (used by TCP-mode DAP adapters like `dlv` or JDWP).
    pub async fn open_ssh_tunnel(
        &self,
        spec: &IdeSshSpec,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<SshTunnel> {
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
        let cfg = SshTunnelConfig {
            host: spec.host.clone(),
            port: spec.effective_port().unwrap_or(22),
            user: spec.user.clone(),
            identity_file,
            proxy_jump: spec.proxy_jump.clone(),
        };
        SshTunnel::open(&cfg, remote_host, remote_port).await
    }

    pub(crate) fn acquire_sftp(
        &self,
        actor_id: &str,
        spec: &IdeSshSpec,
    ) -> Result<Arc<SftpSession>> {
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
        let key = SshPoolKey {
            actor_id: actor_id.to_string(),
            connection_id: spec.connection_id.to_string(),
            params: SftpParams {
                host: spec.host.clone(),
                port: spec.effective_port(),
                user: (!spec.user.is_empty()).then(|| spec.user.clone()),
                identity_file,
                jump: spec.proxy_jump.clone(),
            },
        };
        if let Some(mut entry) = self.sftp_pool.get_mut(&key) {
            entry.1 = Instant::now();
            return Ok(entry.0.clone());
        }
        self.sftp_pool.retain(|_, (sess, touched)| {
            touched.elapsed() < SFTP_IDLE_TTL || Arc::strong_count(sess) > 1
        });
        while self.sftp_pool.len() >= SSH_POOL_CAPACITY {
            let victim = self
                .sftp_pool
                .iter()
                .filter(|e| Arc::strong_count(&e.value().0) <= 1)
                .min_by_key(|e| e.value().1)
                .or_else(|| self.sftp_pool.iter().min_by_key(|e| e.value().1))
                .map(|e| e.key().clone());
            let Some(k) = victim else { break };
            self.sftp_pool.remove(&k);
        }
        let session = Arc::new(SftpSession::new(key.params.clone())?);
        self.sftp_pool
            .insert(key, (session.clone(), Instant::now()));
        Ok(session)
    }

    pub(crate) fn acquire_ssh_socket(
        &self,
        actor_id: &str,
        spec: &IdeSshSpec,
    ) -> Result<Arc<SshControlSocket>> {
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
        let key = SshPoolKey {
            actor_id: actor_id.to_string(),
            connection_id: spec.connection_id.to_string(),
            params: SftpParams {
                host: spec.host.clone(),
                port: spec.effective_port(),
                user: (!spec.user.is_empty()).then(|| spec.user.clone()),
                identity_file,
                jump: spec.proxy_jump.clone(),
            },
        };
        if let Some(mut existing) = self.ssh_sockets.get_mut(&key) {
            existing.1 = Instant::now();
            return Ok(existing.0.clone());
        }
        self.ssh_sockets.retain(|_, (sock, touched)| {
            touched.elapsed() < SFTP_IDLE_TTL || Arc::strong_count(sock) > 1
        });
        while self.ssh_sockets.len() >= SSH_POOL_CAPACITY {
            let victim = self
                .ssh_sockets
                .iter()
                .filter(|e| Arc::strong_count(&e.value().0) <= 1)
                .min_by_key(|e| e.value().1)
                .or_else(|| self.ssh_sockets.iter().min_by_key(|e| e.value().1))
                .map(|e| e.key().clone());
            let Some(k) = victim else { break };
            self.ssh_sockets.remove(&k);
        }
        let sock = Arc::new(SshControlSocket::new(spec)?);
        self.ssh_sockets.insert(key, (sock.clone(), Instant::now()));
        Ok(sock)
    }
}

// ---------------------------------------------------------------------------
// Validation & Path Resolution Helpers
// ---------------------------------------------------------------------------

pub(crate) fn canonicalize_local_root_dir(raw_root: &str, is_root_user: bool) -> Result<PathBuf> {
    let decoded = strip_file_uri(raw_root);
    let expanded = otto_core::paths::expand_home_strict(&decoded).map_err(Error::Invalid)?;
    let canonical = expanded
        .canonicalize()
        .map_err(|e| map_io_err("resolve workspace root", &expanded, e))?;
    let meta = std::fs::metadata(&canonical)
        .map_err(|e| map_io_err("stat workspace root", &canonical, e))?;
    if !meta.is_dir() {
        return Err(Error::Invalid(format!(
            "target root '{}' is not a directory",
            canonical.display()
        )));
    }
    if otto_core::secret_paths::is_denied_dir(&canonical) {
        return Err(Error::Forbidden(format!(
            "{} holds protected credentials; access denied",
            canonical.display()
        )));
    }
    if !is_root_user {
        enforce_non_root_path(&canonical)?;
    }
    Ok(canonical)
}

pub(crate) fn validate_ssh_spec(spec: &IdeSshSpec) -> Result<()> {
    if !valid_ssh_host(&spec.host) {
        return Err(Error::Invalid(format!(
            "SSH host '{}' is not a valid hostname or IP",
            spec.host
        )));
    }
    if !valid_ssh_user(&spec.user) {
        return Err(Error::Invalid(format!(
            "SSH user '{}' is not a valid username",
            spec.user
        )));
    }
    if let Some(id_file) = spec
        .identity_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if id_file.starts_with('-') || id_file.chars().any(char::is_control) {
            return Err(Error::Invalid(
                "SSH identity_file contains unsafe characters".into(),
            ));
        }
        let _ = otto_core::paths::expand_home_strict(id_file).map_err(Error::Invalid)?;
    }
    if let Some(jump) = spec.proxy_jump.as_deref().filter(|s| !s.trim().is_empty()) {
        if !valid_proxy_jump(jump) {
            return Err(Error::Invalid(format!(
                "SSH proxy_jump '{jump}' is invalid"
            )));
        }
    }
    Ok(())
}

pub(crate) fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn posix_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Shared OpenSSH CLI argument builder across `target`, `dap`, `toolchains`,
/// `environments`, and `lsp_host`. Returns `(ssh_args_without_dashdash, destination)`.
pub(crate) fn build_ssh_cli_args(
    spec: &IdeSshSpec,
    ctl_path: Option<&str>,
) -> Result<(Vec<String>, String)> {
    validate_ssh_spec(spec)?;
    let mut args = vec![
        "-T".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
        "-o".into(),
        "ConnectTimeout=12".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=2".into(),
    ];
    if let Some(ctl) = ctl_path {
        args.extend([
            "-o".into(),
            "ControlMaster=auto".into(),
            "-o".into(),
            format!("ControlPath={ctl}"),
            "-o".into(),
            "ControlPersist=60s".into(),
        ]);
    }
    if let Some(port) = spec.effective_port() {
        args.push("-p".into());
        args.push(port.to_string());
    }
    if let Some(id_file) = spec
        .identity_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let expanded = otto_core::paths::expand_home_strict(id_file).map_err(Error::Invalid)?;
        args.push("-i".into());
        args.push(expanded.to_string_lossy().into_owned());
    }
    if let Some(jump) = spec
        .proxy_jump
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("-J".into());
        args.push(jump.to_string());
    }
    let dest = if spec.user.trim().is_empty() {
        spec.host.clone()
    } else {
        format!("{}@{}", spec.user, spec.host)
    };
    Ok((args, dest))
}

pub(crate) fn build_remote_exec_line(
    cwd: &str,
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> String {
    let mut parts = Vec::new();
    parts.push(format!("cd {} &&", shell_words::quote(cwd)));
    if !env.is_empty() {
        parts.push("env".into());
        for (k, v) in env {
            parts.push(format!("{k}={}", shell_words::quote(v)));
        }
    }
    parts.push("exec".into());
    parts.push(shell_words::quote(program).into_owned());
    for arg in args {
        parts.push(shell_words::quote(arg).into_owned());
    }
    parts.join(" ")
}

fn wrap_remote_null_stdin_with_eof_kill(exec_line: &str) -> String {
    format!(
        "( {exec_line} ) </dev/null & pid=$!; \
         ( cat >/dev/null; kill -TERM \"$pid\" ) >/dev/null 2>&1 & watcher=$!; \
         wait \"$pid\"; rc=$?; kill \"$watcher\" 2>/dev/null; exit \"$rc\""
    )
}

/// Wrap a remote command that may read binary frames on `stdin` (such as DAP
/// stdio adapters) OR ignore `stdin` while listening on TCP (`dlv`, JDWP, Node
/// inspector) so closing the local `ssh` pipe cleanly terminates the remote
/// process without `-tt` PTY newline mangling.
pub(crate) fn wrap_remote_stdio_with_eof_kill(exec_line: &str) -> String {
    format!(
        "fifo=\"/tmp/.otto-ide-$$-$(date +%s 2>/dev/null || echo 0)\"; \
         mkfifo \"$fifo\" || exit 1; \
         ( {exec_line} ) <\"$fifo\" & pid=$!; \
         exec 3>\"$fifo\"; rm -f \"$fifo\"; \
         ( cat >&3; exec 3>&-; sleep 1; kill -TERM \"$pid\" ) >/dev/null 2>&1 & watcher=$!; \
         exec 3>&-; wait \"$pid\"; rc=$?; kill \"$watcher\" 2>/dev/null; exit \"$rc\""
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

pub(crate) async fn resolve_local_prospective_in_roots(
    root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
    raw: &str,
    create_dirs: bool,
) -> Result<(PathBuf, PathBuf)> {
    let decoded = strip_file_uri(raw);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("file path must not be empty".into()));
    }
    let abs = if trimmed.starts_with('~') {
        otto_core::paths::expand_home_strict(trimmed).map_err(Error::Invalid)?
    } else {
        let p = Path::new(trimmed);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        }
    };
    let mut prospective = otto_core::secret_paths::canonicalize_prospective(&abs)
        .ok_or_else(|| Error::Invalid(format!("invalid target path '{}'", abs.display())))?;

    let matched_root = std::iter::once(root)
        .chain(additional_roots.iter().map(PathBuf::as_path))
        .find(|r| prospective.starts_with(r))
        .ok_or_else(|| {
            Error::Forbidden(format!(
                "path '{}' escapes workspace root '{}'",
                prospective.display(),
                root.display()
            ))
        })?
        .to_path_buf();

    if !is_root_user {
        enforce_non_root_path(&prospective)?;
    }
    if create_dirs {
        if let Some(parent) = prospective.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| map_io_err("create parent directories", parent, e))?;
            if let Ok(canon_parent) = parent.canonicalize() {
                if !canon_parent.starts_with(&matched_root) {
                    return Err(Error::Forbidden(
                        "parent directory escapes workspace root".into(),
                    ));
                }
                if !is_root_user {
                    enforce_non_root_path(&canon_parent)?;
                }
                if let Some(name) = prospective.file_name() {
                    prospective = canon_parent.join(name);
                }
            }
        }
    }
    Ok((prospective, matched_root))
}

pub(crate) fn normalize_remote_root(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid remote root path".into()));
    }
    if !trimmed.starts_with('/') && !trimmed.starts_with('~') {
        return Err(Error::Invalid(
            "remote SSH root path must start with '/' or '~'".into(),
        ));
    }
    let clean = trimmed.trim_end_matches('/');
    Ok(if clean.is_empty() {
        "/".into()
    } else {
        clean.into()
    })
}

pub(crate) fn resolve_remote_in_roots(
    root: &str,
    additional_roots: &[String],
    raw: &str,
) -> Result<(String, String)> {
    let decoded = strip_file_uri(raw);
    if decoded.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "remote path contains control characters".into(),
        ));
    }
    let trimmed = decoded.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Ok((root.to_string(), root.to_string()));
    }
    let combined = if trimmed.starts_with('/') || trimmed.starts_with('~') {
        trimmed.to_string()
    } else {
        join_remote(root, trimmed)
    };
    let normalized = lexical_normalize_remote(&combined)?;
    for candidate_root in std::iter::once(root).chain(additional_roots.iter().map(String::as_str)) {
        if candidate_root == "/"
            || normalized == candidate_root
            || normalized.starts_with(&format!("{candidate_root}/"))
        {
            return Ok((normalized, candidate_root.to_string()));
        }
    }
    Err(Error::Forbidden(format!(
        "remote path '{normalized}' is outside workspace root '{root}'"
    )))
}

pub(crate) fn resolve_remote_in_roots_or_sdk(
    root: &str,
    additional_roots: &[String],
    raw: &str,
) -> Result<(String, Option<String>)> {
    if let Ok((p, r)) = resolve_remote_in_roots(root, additional_roots, raw) {
        return Ok((p, Some(r)));
    }
    let decoded = strip_file_uri(raw);
    let normalized = lexical_normalize_remote(decoded.trim())?;
    let is_remote_sdk = [
        "/usr/local/go/",
        "/usr/lib/go",
        "/usr/share/go",
        "/usr/lib/python",
        "/usr/lib/jvm/",
        "/.otto/toolchains/",
        "/go/pkg/mod/",
        "/.cargo/registry/",
        "/.rustup/toolchains/",
    ]
    .iter()
    .any(|prefix| normalized.contains(prefix));
    if is_remote_sdk {
        return Ok((normalized, None));
    }
    Err(Error::Forbidden(format!(
        "remote path '{normalized}' is outside workspace root '{root}'"
    )))
}

fn lexical_normalize_remote(path: &str) -> Result<String> {
    let is_tilde = path.starts_with('~');
    let mut stack: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if stack.is_empty() || (is_tilde && stack.len() == 1) {
                    return Err(Error::Forbidden("path traversal above root refused".into()));
                }
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    if is_tilde {
        Ok(stack.join("/"))
    } else {
        Ok(format!("/{}", stack.join("/")))
    }
}

pub(crate) fn join_remote(base: &str, rel: &str) -> String {
    let b = base.trim_end_matches('/');
    let r = rel.trim_start_matches('/');
    if b.is_empty() {
        format!("/{r}")
    } else {
        format!("{b}/{r}")
    }
}

pub(crate) fn remote_parent(path: &str) -> Option<String> {
    let clean = path.trim_end_matches('/');
    if clean.is_empty() || clean == "/" || clean == "~" {
        return None;
    }
    match clean.rsplit_once('/') {
        Some(("", _)) => Some("/".into()),
        Some((parent, _)) => Some(parent.into()),
        None => None,
    }
}

pub(crate) fn map_io_err(action: &str, path: &Path, err: std::io::Error) -> Error {
    let msg = format!("cannot {action} '{}': {err}", path.display());
    match err.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound(msg),
        std::io::ErrorKind::PermissionDenied => Error::Forbidden(msg),
        std::io::ErrorKind::AlreadyExists => Error::Conflict(msg),
        _ => Error::Invalid(msg),
    }
}
