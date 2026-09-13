//! SFTP file browsing/transfer by driving the system `sftp` binary.
//!
//! Otto has no embedded SSH stack — to reuse a connection's *exact* auth
//! (keys, ssh-agent, `~/.ssh/config`, `ProxyJump`/bastion) we shell out to the
//! system `sftp` client, exactly as [`crate::SshTunnel`] does for `ssh`. The
//! daemon runs on the user's Mac, so sftp `get`/`put` read/write the user's
//! real local disk.
//!
//! Each [`SftpSession`] owns a private `ControlMaster`/`ControlPersist` socket
//! (under a unique temp dir, cleaned up on `Drop`) so the multiple batch
//! invocations a browse session makes reuse one multiplexed connection — fast
//! even through a bastion. Every method runs `sftp -b -`, feeding batch
//! commands on stdin and capturing stdout/stderr; on a non-zero exit the
//! stderr is surfaced as the error.

use std::path::PathBuf;
use std::process::Stdio;

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Connect-timeout (seconds) handed to `sftp -o ConnectTimeout`.
const CONNECT_TIMEOUT_SECS: u32 = 12;

/// One directory entry from a remote `ls -la` longname listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpEntry {
    pub name: String,
    /// "dir" | "file" | "symlink" | "other".
    pub kind: String,
    pub size: u64,
    /// The raw date/time field from the listing (e.g. "Jun 20 12:00" or
    /// "Jun 20  2025"); `None` if it couldn't be located.
    pub mtime: Option<String>,
    /// The 10-char permission string (e.g. "drwxr-xr-x").
    pub perms: String,
    /// For symlinks, the link target (the part after " -> "); `None` otherwise.
    pub symlink_target: Option<String>,
}

/// Connection params for an SFTP session — the SSH subset of a connection
/// profile. Auth is the system ssh client's (agent / `identity_file` / config).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SftpParams {
    pub host: String,
    pub port: Option<u16>,
    pub user: Option<String>,
    /// Path to a private key on disk (optional; agent/config used otherwise).
    pub identity_file: Option<String>,
    /// `ProxyJump` spec (`-J`), e.g. `bastion.example.com` (optional).
    pub jump: Option<String>,
}

/// A live SFTP "session": a set of base `sftp` args plus a private control
/// socket. Methods run `sftp -b -` per op; the control master keeps the
/// underlying connection warm between ops.
pub struct SftpSession {
    params: SftpParams,
    program: PathBuf,
    #[cfg(test)]
    probe_timeout: std::time::Duration,
    /// Unique temp dir holding the ControlMaster socket; removed on drop.
    ctl_dir: PathBuf,
    ctl_path: String,
}

impl SftpSession {
    /// Build a session from connection params. Creates a private temp dir for
    /// the control socket. No network I/O happens until the first method call.
    pub fn new(params: SftpParams) -> Result<Self> {
        Self::with_program(params, PathBuf::from("sftp"))
    }

    /// Inject the executable for isolated transport fixtures. HTTP callers never
    /// control this value; production uses the system SFTP client.
    #[doc(hidden)]
    pub fn with_program(params: SftpParams, program: PathBuf) -> Result<Self> {
        // A unique dir per session keeps the control socket private (0700 by
        // mkdir default under TMPDIR) and lets Drop clean it up wholesale.
        let ctl_dir = std::env::temp_dir().join(format!("otto-sftp-{}", uniq_token()));
        std::fs::create_dir_all(&ctl_dir)
            .map_err(|e| Error::Internal(format!("create sftp control dir: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ctl_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| Error::Internal(format!("secure sftp control dir: {e}")))?;
        }
        let ctl_path = ctl_dir.join("ctl.sock").to_string_lossy().into_owned();
        Ok(Self {
            params,
            program,
            #[cfg(test)]
            probe_timeout: std::time::Duration::from_millis(500),
            ctl_dir,
            ctl_path,
        })
    }

    /// The common `sftp` args every op uses: batch/non-interactive auth, the
    /// same trust-on-first-use host-key policy [`crate::SshTunnel`] uses
    /// (`StrictHostKeyChecking=accept-new` — under `BatchMode` a first-time host
    /// would otherwise fail with "Host key verification failed"), a bounded
    /// connect, and a per-session ControlMaster socket. The target (`user@host`)
    /// is appended by the caller via [`Self::target`].
    fn base_args(&self) -> Vec<String> {
        let p = &self.params;
        let mut args = vec![
            "-b".into(),
            "-".into(),
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "StrictHostKeyChecking=accept-new".into(),
            "-o".into(),
            format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"),
            "-o".into(),
            "ControlMaster=auto".into(),
            "-o".into(),
            format!("ControlPath={}", self.ctl_path),
            "-o".into(),
            "ControlPersist=60s".into(),
        ];
        if let Some(port) = p.port {
            args.extend(["-P".into(), port.to_string()]);
        }
        if let Some(identity) = p.identity_file.as_deref().filter(|s| !s.is_empty()) {
            args.push("-i".into());
            args.push(identity.to_string());
        }
        if let Some(jump) = p.jump.as_deref().filter(|s| !s.is_empty()) {
            args.push("-J".into());
            args.push(jump.to_string());
        }
        args.push(self.target());
        args
    }

    /// `user@host` (or `host` when no user is set), matching `build_command`.
    fn target(&self) -> String {
        match self.params.user.as_deref().filter(|s| !s.is_empty()) {
            Some(user) => format!("{user}@{}", self.params.host),
            None => self.params.host.clone(),
        }
    }

    /// Run `sftp -b -` feeding `batch` on stdin; return stdout on success or the
    /// stderr (first non-empty line, else whole) as the error on a non-zero
    /// exit. Secrets are never in argv (key/agent auth) so args are safe.
    async fn run(&self, batch: &str) -> Result<String> {
        tokio::time::timeout(std::time::Duration::from_secs(600), self.run_inner(batch))
            .await
            .map_err(|_| Error::Upstream("SFTP operation timed out after 600 seconds".into()))?
    }

    async fn run_inner(&self, batch: &str) -> Result<String> {
        let mut cmd = Command::new(&self.program);
        cmd.args(self.base_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Upstream(format!("failed to start sftp: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(batch.as_bytes())
                .await
                .map_err(|e| Error::Upstream(format!("sftp stdin write: {e}")))?;
            // Trailing newline + EOF so the last command runs and sftp exits.
            let _ = stdin.write_all(b"\n").await;
            drop(stdin);
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| Error::Upstream(format!("sftp wait: {e}")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let msg = stderr
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string();
            let msg = if msg.is_empty() {
                format!("sftp exited with {}", out.status)
            } else {
                msg
            };
            Err(Error::Upstream(msg))
        }
    }

    /// Resolve the remote working/home dir to an absolute path (anchors the UI).
    pub async fn pwd(&self) -> Result<String> {
        let out = self.run("pwd").await?;
        parse_pwd(&out)
            .ok_or_else(|| Error::Upstream("could not resolve remote working directory".into()))
    }

    /// List a remote directory via `ls -la`, parsing the longname listing.
    pub async fn list(&self, path: &str) -> Result<Vec<SftpEntry>> {
        let batch = format!("ls -la {}", quote_checked(path)?);
        let out = self.run(&batch).await?;
        Ok(parse_longname_listing(&out))
    }

    /// Best-effort exact-file metadata, bounded independently of transfers and
    /// directory browsing. A stalled probe cannot hold progress for >500ms.
    pub async fn file_size(&self, path: &str) -> Result<u64> {
        let batch = file_probe_command(path)?;
        let output = self.run_probe(&batch).await?;
        parse_file_size(&output, path)
            .ok_or_else(|| Error::Upstream("SFTP file size unavailable or ambiguous".into()))
    }

    async fn run_probe(&self, batch: &str) -> Result<String> {
        let mut command = Command::new(&self.program);
        command
            .args(self.base_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = command
            .spawn()
            .map_err(|e| Error::Upstream(format!("failed to start sftp probe: {e}")))?;
        let mut owned = ProbeChild(Some(child));
        let child = owned.0.as_mut().expect("probe child");
        let mut stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let work = async {
            tokio::try_join!(
                async move {
                    stdin.write_all(batch.as_bytes()).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.shutdown().await?;
                    drop(stdin);
                    Ok::<(), std::io::Error>(())
                },
                bounded_probe_output(stdout),
                bounded_probe_output(stderr),
                child.wait(),
            )
        };
        #[cfg(not(test))]
        let deadline = std::time::Duration::from_millis(500);
        #[cfg(test)]
        let deadline = self.probe_timeout;
        let (_, stdout, stderr, status) = tokio::time::timeout(deadline, work)
            .await
            .map_err(|_| Error::Upstream("SFTP progress probe timed out".into()))?
            .map_err(|e| Error::Upstream(format!("SFTP progress probe: {e}")))?;
        // wait() completed, so this child has already been reaped.
        owned.0.take();
        if !status.success() {
            let message = String::from_utf8_lossy(&stderr);
            return Err(Error::Upstream(
                message
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("SFTP progress probe failed")
                    .to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    }

    /// Download a remote file to a local path (sftp `get`).
    pub async fn download(&self, remote: &str, local: &str) -> Result<()> {
        let batch = format!("get {} {}", quote_checked(remote)?, quote_checked(local)?);
        self.run(&batch).await.map(|_| ())
    }

    /// Upload a local file to a remote path (sftp `put`).
    pub async fn upload(&self, local: &str, remote: &str) -> Result<()> {
        let batch = format!("put {} {}", quote_checked(local)?, quote_checked(remote)?);
        self.run(&batch).await.map(|_| ())
    }

    /// Create a remote directory.
    pub async fn mkdir(&self, path: &str) -> Result<()> {
        let batch = format!("mkdir {}", quote_checked(path)?);
        self.run(&batch).await.map(|_| ())
    }

    /// Remove a remote file.
    pub async fn remove(&self, path: &str) -> Result<()> {
        let batch = format!("rm {}", quote_checked(path)?);
        self.run(&batch).await.map(|_| ())
    }

    /// Remove a remote directory.
    pub async fn rmdir(&self, path: &str) -> Result<()> {
        let batch = format!("rmdir {}", quote_checked(path)?);
        self.run(&batch).await.map(|_| ())
    }

    /// Rename/move a remote path.
    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let batch = format!("rename {} {}", quote_checked(from)?, quote_checked(to)?);
        self.run(&batch).await.map(|_| ())
    }
}

impl Drop for SftpSession {
    fn drop(&mut self) {
        // Never block an async request on process cleanup. No connection is
        // opened for an unused session, and a stuck control client is bounded.
        let dir = self.ctl_dir.clone();
        let path = self.ctl_path.clone();
        let target = self.target();
        std::thread::spawn(move || {
            if std::path::Path::new(&path).exists() {
                if let Ok(mut child) = std::process::Command::new("ssh")
                    .args(["-o", &format!("ControlPath={path}"), "-O", "exit", &target])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                    loop {
                        if child.try_wait().ok().flatten().is_some() {
                            break;
                        }
                        if std::time::Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            }
            let _ = std::fs::remove_dir_all(dir);
        });
    }
}

/// A short, collision-resistant token for the control-socket dir name. Avoids a
/// uuid dep: pid + a monotonic counter + the coarse wall clock are unique enough
/// for a per-process temp dir.
fn uniq_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{pid}-{n}-{nanos}")
}

/// The caller can drop a probe while either pipe or wait is pending. Keep the
/// child owned until kill + wait completes; never leave a zombie to the caller.
struct ProbeChild(Option<tokio::process::Child>);
impl Drop for ProbeChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                });
            }
        }
    }
}
async fn bounded_probe_output(
    mut stream: impl tokio::io::AsyncRead + Unpin,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    const LIMIT: usize = 32 * 1024;
    let mut output = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 {
            return Ok(output);
        }
        if output.len() + count > LIMIT {
            return Err(std::io::Error::other("SFTP probe output limit exceeded"));
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

fn parse_file_size(output: &str, path: &str) -> Option<u64> {
    let mut records = output
        .lines()
        .map(str::trim_start)
        .filter(|line| looks_like_mode(line));
    let mut rest = records.next()?;
    if records.next().is_some() || !rest.starts_with('-') {
        return None;
    }
    let mut size = None;
    for column in 0..8 {
        let end = rest.find(char::is_whitespace)?;
        if column == 4 {
            size = rest[..end].parse::<u64>().ok();
        }
        rest = rest[end..].trim_start();
    }
    // Unlike general directory listing, retain repeated spaces in the path.
    // Numeric OpenSSH listings supply the full path (relative if requested so).
    use std::path::Component;
    let visible = std::path::Path::new(rest)
        .components()
        .filter(|c| *c != Component::CurDir);
    let requested = std::path::Path::new(path)
        .components()
        .filter(|c| *c != Component::CurDir);
    if visible.eq(requested) {
        size
    } else {
        None
    }
}

/// OpenSSH makeargv escapes quoted ? [ * itself. Braces need one explicit
/// escape preserved by makeargv for its subsequent GLOB_BRACE expansion.
/// See openssh-portable/sftp.c makeargv and do_globbed_ls.
fn file_probe_command(path: &str) -> Result<String> {
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid SFTP progress path".into()));
    }
    let mut out = String::from("@ls -ln \"");
    if !path.starts_with('/') {
        out.push_str("./");
    }
    for ch in path.chars() {
        if matches!(ch, '"' | '\\' | '{' | '}') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    if out.len() >= 8192 {
        return Err(Error::Invalid("SFTP progress path is too long".into()));
    }
    Ok(out)
}

/// Double-quote a remote/local path for an sftp batch command, escaping the
/// characters that are special inside double quotes (`"` and `\`). sftp's batch
/// parser honours double quotes, so this keeps names with spaces intact.
fn quote(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 2);
    out.push('"');
    for ch in path.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Quote a path for an sftp batch command, REJECTING any control character.
///
/// `sftp -b` is line-oriented and supports a `!cmd` local-shell escape, so a
/// path containing a newline (or CR) could split the batch and smuggle a
/// `!command` onto its own line — local command execution from a hostile remote
/// filename the user merely browses/clicks. No legitimate path component holds a
/// control char, so reject them all up front (defence in depth: also covers the
/// caller-supplied local path that gets interpolated into `get`/`put`).
fn quote_checked(path: &str) -> Result<String> {
    if path.chars().any(char::is_control) {
        return Err(Error::Upstream(
            "path contains a control character (rejected for safety)".into(),
        ));
    }
    Ok(quote(path))
}

/// Parse the absolute path out of sftp `pwd` output. The client prints a line
/// like `Remote working directory: /home/me`; we also tolerate a bare path.
fn parse_pwd(out: &str) -> Option<String> {
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.split_once(": ") {
            // "Remote working directory: /home/me"
            let p = rest.1.trim();
            if p.starts_with('/') {
                return Some(p.to_string());
            }
        }
        if line.starts_with('/') {
            return Some(line.to_string());
        }
    }
    None
}

/// Parse a remote `ls -la` longname listing into entries.
///
/// Robust by design: only lines whose first token looks like a mode string
/// (`^[-dlbcps][rwxsStT-]{9}`) are considered — this filters command echoes,
/// the `sftp>` prompt, blank lines, and "total N" headers. From a kept line:
/// perms = token 0, size = token 4 (numeric), name = join(tokens ≥ 8). A
/// trailing `/`, `*`, or `@` indicator on the name is stripped; a " -> " splits
/// a symlink's name from its target. The entry type comes from `perms[0]`.
/// The "." entry is excluded.
fn parse_longname_listing(out: &str) -> Vec<SftpEntry> {
    let mut entries = Vec::new();
    for raw in out.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if !looks_like_mode(trimmed) {
            continue;
        }
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        // Need at least: perms, links, owner, group, size, mon, day, time, name
        if tokens.len() < 9 {
            continue;
        }
        let perms = tokens[0].to_string();
        let size: u64 = tokens[4].parse().unwrap_or(0);
        // The date/time is tokens 5..8 ("Mon DD HH:MM" or "Mon DD  YYYY").
        let mtime = Some(tokens[5..8].join(" "));

        // Name = everything from token 8 onward (handles names with spaces).
        let name_field = tokens[8..].join(" ");

        let kind = match perms.chars().next() {
            Some('d') => "dir",
            Some('l') => "symlink",
            Some('-') => "file",
            _ => "other",
        }
        .to_string();

        // Symlinks: "name -> target".
        let (mut name, symlink_target) = match name_field.split_once(" -> ") {
            Some((n, t)) => (n.to_string(), Some(strip_indicator(t).to_string())),
            None => (name_field.clone(), None),
        };
        name = strip_indicator(&name).to_string();

        if name == "." {
            continue;
        }
        entries.push(SftpEntry {
            name,
            kind,
            size,
            mtime,
            perms,
            symlink_target,
        });
    }
    entries
}

/// Strip a trailing `ls -F`-style type indicator (`/`, `*`, `@`) from a name.
fn strip_indicator(name: &str) -> &str {
    name.strip_suffix('/')
        .or_else(|| name.strip_suffix('*'))
        .or_else(|| name.strip_suffix('@'))
        .unwrap_or(name)
}

/// True when `s` begins with a 10-char unix mode string:
/// type char in `[-dlbcps]` followed by 9 perm chars in `[rwxsStT-]`.
fn looks_like_mode(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return false;
    }
    let type_ok = matches!(bytes[0], b'-' | b'd' | b'l' | b'b' | b'c' | b'p' | b's');
    if !type_ok {
        return false;
    }
    bytes[1..10]
        .iter()
        .all(|&b| matches!(b, b'r' | b'w' | b'x' | b's' | b'S' | b't' | b'T' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_checked_rejects_control_chars() {
        // A newline would split the sftp batch line and let a planted filename
        // smuggle a `!local-command` — must be rejected (local RCE guard).
        assert!(quote_checked("foo\n!touch /tmp/PWNED").is_err());
        assert!(quote_checked("a\rb").is_err());
        assert!(quote_checked("a\tb").is_err());
        assert!(quote_checked("a\0b").is_err());
        // Ordinary names (incl. spaces/quotes/backslashes) still quote fine.
        assert_eq!(quote_checked("normal name").unwrap(), "\"normal name\"");
        assert_eq!(quote_checked(r#"a"b\c"#).unwrap(), r#""a\"b\\c""#);
    }

    #[test]
    fn parses_dirs_and_files() {
        let out = "\
total 24
drwxr-xr-x    5 me   staff   160 Jun 20 12:00 .
drwxr-xr-x   12 me   staff   384 Jun 19 09:30 ..
drwxr-xr-x    3 me   staff    96 Jun 20 12:00 src
-rw-r--r--    1 me   staff  1024 Jun 20 12:00 README.md
";
        let e = parse_longname_listing(out);
        // "." excluded; ".." kept (a real navigable entry); src + README.
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].name, "..");
        assert_eq!(e[0].kind, "dir");
        let src = e.iter().find(|x| x.name == "src").unwrap();
        assert_eq!(src.kind, "dir");
        assert_eq!(src.size, 96);
        let readme = e.iter().find(|x| x.name == "README.md").unwrap();
        assert_eq!(readme.kind, "file");
        assert_eq!(readme.size, 1024);
        assert_eq!(readme.perms, "-rw-r--r--");
        assert_eq!(readme.mtime.as_deref(), Some("Jun 20 12:00"));
    }

    #[test]
    fn parses_name_with_spaces() {
        let out = "-rw-r--r--  1 me staff 2048 Jun 20 12:00 My Report Final (v2).pdf\n";
        let e = parse_longname_listing(out);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "My Report Final (v2).pdf");
        assert_eq!(e[0].size, 2048);
        assert_eq!(e[0].kind, "file");
    }

    #[test]
    fn parses_symlink_with_target() {
        let out = "lrwxr-xr-x  1 me staff 11 Jun 20 12:00 current -> releases/42\n";
        let e = parse_longname_listing(out);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "current");
        assert_eq!(e[0].kind, "symlink");
        assert_eq!(e[0].symlink_target.as_deref(), Some("releases/42"));
    }

    #[test]
    fn parses_both_date_forms() {
        let out = "\
-rw-r--r--  1 me staff  10 Jun 20 12:00 recent.txt
-rw-r--r--  1 me staff  20 Jun 20  2025 old.txt
";
        let e = parse_longname_listing(out);
        assert_eq!(e.len(), 2);
        let recent = e.iter().find(|x| x.name == "recent.txt").unwrap();
        assert_eq!(recent.mtime.as_deref(), Some("Jun 20 12:00"));
        let old = e.iter().find(|x| x.name == "old.txt").unwrap();
        assert_eq!(old.mtime.as_deref(), Some("Jun 20 2025"));
    }

    #[test]
    fn filters_echo_and_prompt_lines() {
        // A stray command echo, the sftp prompt, and a "total" header must all
        // be filtered — only real listing rows survive.
        let out = "\
sftp> ls -la \"/home/me\"
ls -la /home/me
total 8
drwxr-xr-x  2 me staff 64 Jun 20 12:00 keep
not-a-mode-string here we go
";
        let e = parse_longname_listing(out);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "keep");
        assert_eq!(e[0].kind, "dir");
    }

    #[test]
    fn strips_ls_f_indicators() {
        let out = "\
drwxr-xr-x  2 me staff 64 Jun 20 12:00 mydir/
-rwxr-xr-x  1 me staff 64 Jun 20 12:00 run.sh*
";
        let e = parse_longname_listing(out);
        assert_eq!(e.iter().find(|x| x.kind == "dir").unwrap().name, "mydir");
        let exe = e.iter().find(|x| x.name == "run.sh").unwrap();
        assert_eq!(exe.kind, "file");
    }

    #[test]
    fn parse_pwd_from_labeled_line() {
        assert_eq!(
            parse_pwd("Remote working directory: /home/me\n").as_deref(),
            Some("/home/me")
        );
        assert_eq!(parse_pwd("/var/www\n").as_deref(), Some("/var/www"));
        assert_eq!(parse_pwd("sftp> pwd\nno path here"), None);
    }

    #[test]
    fn quote_escapes_specials() {
        assert_eq!(quote("/a/b c"), "\"/a/b c\"");
        assert_eq!(quote(r#"/a/"x"#), "\"/a/\\\"x\"");
        assert_eq!(quote(r"/a\b"), "\"/a\\\\b\"");
    }

    #[test]
    fn base_args_shape() {
        let s = SftpSession::new(SftpParams {
            host: "h.example.com".into(),
            port: Some(2222),
            user: Some("deploy".into()),
            identity_file: Some("/home/me/.ssh/id_ed25519".into()),
            jump: Some("bastion.example.com".into()),
        })
        .unwrap();
        let args = s.base_args();
        assert_eq!(args[0], "-b");
        assert_eq!(args[1], "-");
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
        assert!(args.iter().any(|a| a == "StrictHostKeyChecking=accept-new"));
        assert!(args.iter().any(|a| a.starts_with("ControlPath=")));
        assert!(args.iter().any(|a| a == "ControlPersist=60s"));
        assert_eq!(
            args[args.iter().position(|a| a == "-P").unwrap() + 1],
            "2222"
        );
        assert_eq!(
            args[args.iter().position(|a| a == "-i").unwrap() + 1],
            "/home/me/.ssh/id_ed25519"
        );
        assert_eq!(
            args[args.iter().position(|a| a == "-J").unwrap() + 1],
            "bastion.example.com"
        );
        assert_eq!(args.last().unwrap(), "deploy@h.example.com");
    }

    #[test]
    fn base_args_omit_identity_and_jump_when_absent() {
        let s = SftpSession::new(SftpParams {
            host: "h".into(),
            port: None,
            user: None,
            identity_file: None,
            jump: None,
        })
        .unwrap();
        let args = s.base_args();
        assert!(!args.iter().any(|a| a == "-P"));
        assert!(!args.iter().any(|a| a == "-i"));
        assert!(!args.iter().any(|a| a == "-J"));
        assert_eq!(args.last().unwrap(), "h");
    }
}

#[cfg(all(test, unix))]
mod file_size_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn fixture(script: &str) -> SftpSession {
        let mut session = SftpSession::new(SftpParams {
            host: "fixture.invalid".into(),
            port: None,
            user: None,
            identity_file: None,
            jump: None,
        })
        .unwrap();
        let program = session.ctl_dir.join("fixture-sftp");
        std::fs::write(
            &program,
            format!("#!/usr/bin/env python3\nimport sys,pathlib,os\n{script}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        session.program = program;
        session.probe_timeout = std::time::Duration::from_secs(3);
        session
    }
    #[tokio::test]
    async fn file_size_probes_exact_staging_path_and_bounds_ambiguity() {
        let session=fixture("pathlib.Path(__file__).with_name('batch').write_text(sys.stdin.read())\nprint('-rw-r--r-- 1 1000 1000 321 Sep 13 10:00 /remote/stage')");
        assert_eq!(session.file_size("/remote/stage").await.unwrap(), 321);
        assert_eq!(
            std::fs::read_to_string(session.ctl_dir.join("batch")).unwrap(),
            "@ls -ln \"/remote/stage\"\n"
        );
        let session=fixture("sys.stdin.read()\nprint('-rw-r--r-- 1 1 1 3 Sep 13 10:00 one\\n-rw-r--r-- 1 1 1 4 Sep 13 10:00 two')");
        assert!(session.file_size("/remote/stage").await.is_err());
        let session =
            fixture("sys.stdin.read()\nprint('drwxr-xr-x 1 1 1 3 Sep 13 10:00 /remote/stage')");
        assert!(session.file_size("/remote/stage").await.is_err());
    }
    #[test]
    fn file_size_literal_path_does_not_add_glob_patterns() {
        assert_eq!(
            file_probe_command("-stage").unwrap(),
            "@ls -ln \"./-stage\""
        );
        assert_eq!(
            file_probe_command("/a/{b,c} [x]*?").unwrap(),
            r#"@ls -ln "/a/\{b,c\} [x]*?""#
        );
        assert_eq!(
            file_probe_command(r#"/a/"x\y"#).unwrap(),
            r#"@ls -ln "/a/\"x\\y""#
        );
        assert!(file_probe_command("a\n!command").is_err());
        assert!(file_probe_command("").is_err());
    }
    #[test]
    fn file_size_parser_preserves_spaces_and_rejects_directory_child() {
        let line = "-rw-r--r-- 1 1 1 42 Sep 13 10:00 /a  b/.stage";
        assert_eq!(parse_file_size(line, "/a  b/.stage"), Some(42));
        assert_eq!(parse_file_size(line, "/a b/.stage"), None);
        assert_eq!(
            parse_file_size("-rw-r--r-- 1 1 1 42 Sep 13 10:00 /stage/child", "/stage"),
            None
        );
        assert_eq!(
            parse_file_size("-rw-r--r-- 1 1 1 42 Sep 13 10:00 /.stage", "//.stage"),
            Some(42)
        );
    }
    /// Exercise the actual OpenSSH batch/glob parser over local stdio only:
    /// no ssh process, listener, host credentials or network connection.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn file_size_real_openssh_literal_paths_over_local_stdio() {
        let session =
            fixture("os.execv('/usr/bin/sftp',['sftp','-b','-','-D','/usr/libexec/sftp-server'])");
        for name in [
            "a  b",
            "{x,y}",
            "[x]*?",
            r"back\slash",
            "quote\"",
            "-option",
        ] {
            let path = session.ctl_dir.join(name);
            std::fs::write(&path, b"fixture-data").unwrap();
            assert_eq!(
                session.file_size(path.to_str().unwrap()).await.unwrap(),
                12,
                "{name}"
            );
        }
    }
    #[tokio::test]
    async fn file_size_rejects_stdout_and_stderr_over_budget() {
        for fd in [1, 2] {
            let session = fixture(&format!("sys.stdin.read()\nos.write({fd},b'x'*65536)"));
            let error = session.file_size("/stage").await.unwrap_err().to_string();
            assert!(error.contains("output limit"), "{error}");
        }
    }
    #[tokio::test]
    async fn file_size_timeout_and_cancellation_reap_owned_child() {
        for abort in [false, true] {
            let mut fixture = fixture("");
            let pid_path = fixture.ctl_dir.join("pid");
            let quoted = format!("'{}'", pid_path.to_string_lossy().replace('\'', "'\\''"));
            std::fs::write(
                &fixture.program,
                format!(
                    "#!/bin/sh\ncat >/dev/null\nprintf '%s' \"$$\" > {quoted}\nexec sleep 60\n"
                ),
            )
            .unwrap();
            // Budget includes cold executable startup under parallel crate tests.
            // Production remains 500 ms; this fixture verifies cleanup, not startup latency.
            fixture.probe_timeout = std::time::Duration::from_secs(3);
            let session = std::sync::Arc::new(fixture);
            let task_session = session.clone();
            let mut task = tokio::spawn(async move { task_session.file_size("/stage").await });
            let pid_path = session.ctl_dir.join("pid");
            tokio::time::timeout(std::time::Duration::from_secs(4),async {while !pid_path.exists(){tokio::select!{result=&mut task=>panic!("probe exited before fixture readiness: {result:?}"),_=tokio::time::sleep(std::time::Duration::from_millis(5))=>{}}}}).await.unwrap();
            if abort {
                task.abort();
                let _ = task.await;
            } else {
                assert!(task.await.unwrap().is_err());
            }
            let pid = std::fs::read_to_string(pid_path).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let status = tokio::process::Command::new("kill")
                        .args(["-0", pid.trim()])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status()
                        .await
                        .unwrap();
                    if !status.success() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        }
    }
}
