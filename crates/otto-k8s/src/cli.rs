//! `kubectl` runner — the only way this crate talks to a cluster.
//!
//! Contract (`docs/design/aws-k8s-consoles.md` §1, §4.1): `tokio::process::Command`
//! (never a shell string), `kill_on_drop(true)`, a per-call timeout (30 s
//! default; logs/exec streams are handled elsewhere without one), `-o json`
//! everywhere and normalisation in Rust. Failures are classified from stderr:
//!
//! - binary missing ⇒ `Error::Invalid("kubectl not installed — …")` — the UI keys
//!   off the `not installed` prefix to show the installer panel;
//! - `forbidden` ⇒ `Error::Forbidden("cluster RBAC: <first line>")` (the
//!   capability probe deliberately does NOT run `auth can-i --list`);
//! - `NotFound` from the API server ⇒ `Error::NotFound`;
//! - anything else ⇒ `Error::Invalid` with a **redacted** stderr tail.

use std::process::Stdio;
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use otto_state::K8sCluster;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Default per-call wall clock for one kubectl invocation.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// `--request-timeout` handed to every non-streaming kubectl call (§4.1).
pub const REQUEST_TIMEOUT: &str = "20s";
/// Message prefix the UI matches to offer the installer.
pub const NOT_INSTALLED_MSG: &str =
    "kubectl not installed — open the Kubernetes module to install it";

/// Raw result of one CLI run (contract §1 `CliOutput`).
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

/// Run `program args…` with `env` added to the daemon environment, feeding
/// `stdin` when given, bounded by `timeout`. Non-zero exit is an error (see the
/// module doc for the classification); callers that want the raw exit use
/// [`run_raw`].
pub async fn run(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
    stdin: Option<&[u8]>,
) -> Result<CliOutput> {
    let out = run_raw(program, args, env, timeout, stdin).await?;
    if out.status != 0 {
        return Err(classify_failure(program, &out.stderr));
    }
    Ok(out)
}

/// Like [`run`] but a non-zero exit is returned as data, not an error. A
/// missing binary / spawn failure / timeout is still an error.
pub async fn run_raw(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
    stdin: Option<&[u8]>,
) -> Result<CliOutput> {
    let started = Instant::now();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| spawn_error(program, &e))?;
    if let Some(bytes) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            let owned = bytes.to_vec();
            // Write in a task so a child that never reads stdin can't deadlock
            // us against a full pipe; errors here surface as a non-zero exit.
            tokio::spawn(async move {
                let _ = pipe.write_all(&owned).await;
                let _ = pipe.shutdown().await;
            });
        }
    }
    let waited = tokio::time::timeout(timeout, child.wait_with_output()).await;
    let output = match waited {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(Error::Internal(format!("{program}: {e}"))),
        Err(_) => {
            return Err(Error::Upstream(format!(
                "{program} timed out after {}s",
                timeout.as_secs()
            )))
        }
    };
    Ok(CliOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// Bytes of stderr kept by [`run_raw_capped`] (the classification only
/// reads the first lines).
const CAPPED_STDERR: usize = 64 * 1024;

/// [`run_raw`] for an output whose size the caller does not control (a pod's
/// `/proxy` body): stdout is read incrementally and the child is killed once
/// `cap` bytes arrived, instead of `wait_with_output` buffering ALL of it
/// before any cap applies (S6-09). A capped run reports exit 0 — the cut was
/// ours — with `stdout` holding the first `cap` bytes.
pub async fn run_raw_capped(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
    cap: usize,
) -> Result<CliOutput> {
    use tokio::io::AsyncReadExt;
    let started = Instant::now();
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| spawn_error(program, &e))?;
    let (Some(mut out), Some(mut err)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(Error::Internal(format!("{program}: no stdio pipes")));
    };
    let read = async {
        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut ebuf = vec![0u8; 8 * 1024];
        let (mut out_done, mut err_done, mut capped) = (false, false, false);
        while !(out_done && err_done) {
            tokio::select! {
                r = out.read(&mut buf), if !out_done => {
                    let n = r?;
                    if n == 0 {
                        out_done = true;
                    } else {
                        let take = n.min(cap.saturating_sub(stdout.len()));
                        stdout.extend_from_slice(&buf[..take]);
                        if stdout.len() >= cap {
                            capped = true;
                            break;
                        }
                    }
                }
                r = err.read(&mut ebuf), if !err_done => {
                    let n = r?;
                    if n == 0 {
                        err_done = true;
                    } else {
                        let take = n.min(CAPPED_STDERR.saturating_sub(stderr.len()));
                        stderr.extend_from_slice(&ebuf[..take]);
                    }
                }
            }
        }
        let status = if capped {
            let _ = child.start_kill();
            let _ = child.wait().await;
            0
        } else {
            child.wait().await?.code().unwrap_or(-1)
        };
        Ok::<_, std::io::Error>((status, stdout, stderr))
    };
    // On timeout `read` (and its borrow of `child`) is dropped; `child` drops
    // at return and `kill_on_drop` ends the process.
    let (status, stdout, stderr) = match tokio::time::timeout(timeout, read).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(Error::Internal(format!("{program}: {e}"))),
        Err(_) => {
            return Err(Error::Upstream(format!(
                "{program} timed out after {}s",
                timeout.as_secs()
            )))
        }
    };
    Ok(CliOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// Map a spawn error: ENOENT ⇒ the `not installed` invariant the UI relies on.
pub fn spawn_error(program: &str, e: &std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        return Error::Invalid(not_installed_message(program));
    }
    Error::Internal(format!("spawn {program}: {e}"))
}

/// The `not installed` message for a tool (kubectl / k9s).
pub fn not_installed_message(program: &str) -> String {
    let tool = std::path::Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);
    if tool == "kubectl" {
        NOT_INSTALLED_MSG.to_string()
    } else {
        format!("{tool} not installed — open the Kubernetes module to install it")
    }
}

/// Message prefix for an API server that rejected the credentials (KS-2) —
/// typically a stale cached exec token after an SSO re-login / rotated keys.
pub const CREDENTIALS_REJECTED_MSG: &str =
    "credentials rejected — re-authenticate (e.g. aws sso login), then Refresh";

/// Does this stderr say the API server rejected the credentials?
pub fn is_unauthorized_stderr(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("(unauthorized)")
        || lower.contains("you must be logged in")
        || lower.contains("error: unauthorized")
        || lower.contains("unauthorized:")
}

/// Is `e` the classified [`CREDENTIALS_REJECTED_MSG`] failure?
pub fn is_credentials_rejected(e: &Error) -> bool {
    matches!(e, Error::Invalid(m) if m.starts_with(CREDENTIALS_REJECTED_MSG))
}

/// Classify a non-zero kubectl exit from its stderr (contract §4.6).
pub fn classify_failure(program: &str, stderr: &str) -> Error {
    let first = first_meaningful_line(stderr);
    let lower = first.to_ascii_lowercase();
    if is_unauthorized_stderr(stderr) {
        return Error::Invalid(format!("{CREDENTIALS_REJECTED_MSG}: {}", redact(&first)));
    }
    if lower.contains("forbidden") {
        return Error::Forbidden(format!("cluster RBAC: {}", redact(&first)));
    }
    if lower.contains("(notfound)")
        || lower.contains("not found") && lower.contains("error from server")
    {
        return Error::NotFound(redact(&first));
    }
    if lower.contains("executable file not found") || lower.contains("command not found") {
        return Error::Invalid(not_installed_message(program));
    }
    let tail = redact(&tail_lines(stderr, 6));
    let tool = std::path::Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);
    if tail.is_empty() {
        Error::Invalid(format!("{tool} failed"))
    } else {
        Error::Invalid(format!("{tool}: {tail}"))
    }
}

/// First non-blank stderr line (kubectl prefixes the real reason on line one).
pub fn first_meaningful_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn tail_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Redact secrets (tokens, keys, PEM blocks) before an error message leaves the
/// daemon — kubectl happily echoes bearer tokens on TLS/auth failures.
pub fn redact(s: &str) -> String {
    strip_aws_keys(&otto_core::redact::redact_text(s).value)
}

/// Blank AWS access-key ids (`AKIA…`/`ASIA…` + 16 upper-alnum) wherever they
/// appear — `otto_core::redact` only classifies whole whitespace-delimited
/// words, so `token=AKIA…` or `(AKIA…)` would slip through. EKS token errors
/// from the `aws eks get-token` exec plugin do embed them.
pub fn strip_aws_keys(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        let is_key = i + 20 <= b.len()
            && (b[i..].starts_with(b"AKIA") || b[i..].starts_with(b"ASIA"))
            && b[i + 4..i + 20]
                .iter()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && (i + 20 == b.len() || !b[i + 20].is_ascii_alphanumeric());
        if is_key {
            out.push_str("[redacted]");
            i += 20;
        } else {
            // Advance one UTF-8 scalar.
            let ch = s[i..].chars().next().expect("char");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// A kubectl handle bound to one cluster: the base flags + injected env
/// (for `eks` clusters the linked AWS account's credentials, so the kubeconfig's
/// `aws eks get-token` exec plugin can mint a token).
#[derive(Debug, Clone)]
pub struct Kubectl {
    /// Program path (located binary, or bare `kubectl` for PATH lookup).
    pub program: String,
    /// Flags every call starts with — see [`base_args`].
    pub base: Vec<String>,
    /// Same flags without `--request-timeout`, for exec/logs -f/k9s streams.
    pub base_stream: Vec<String>,
    pub env: Vec<(String, String)>,
    /// KS-2: set when this handle uses a CACHED exec-token overlay. On a
    /// credentials-rejected failure the runner calls it once — it drops the
    /// cached token and builds a fresh handle — and retries the call.
    pub reauth: Option<Reauth>,
}

/// Rebuild a handle after dropping a stale cached token (see [`Kubectl::reauth`]).
#[derive(Clone)]
pub struct Reauth(
    pub std::sync::Arc<dyn Fn() -> crate::BoxFut<'static, Result<Kubectl>> + Send + Sync + 'static>,
);

impl std::fmt::Debug for Reauth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Reauth(..)")
    }
}

/// Retry decision for KS-2: only a credentials-rejected failure on a handle
/// that used a cached overlay is worth one retry with a fresh token.
pub fn should_reauth(e: &Error, has_reauth: bool) -> bool {
    has_reauth && is_credentials_rejected(e)
}

/// §4.1: `["--kubeconfig", path?, "--context", ctx, "--request-timeout", "20s"]`
/// — `--kubeconfig` omitted when the row has no path (kubectl's own default
/// resolution). We never pass `-o wide`; `-o json` is appended per call.
pub fn base_args(cluster: &K8sCluster) -> Vec<String> {
    let mut v = base_args_stream(cluster);
    v.push("--request-timeout".into());
    v.push(REQUEST_TIMEOUT.into());
    v
}

/// Base flags WITHOUT `--request-timeout` — kubectl applies that value to the
/// underlying HTTP client, which would tear down a long-lived `exec` / `logs -f`
/// stream mid-flight. Only used for streaming invocations and PTY sessions.
pub fn base_args_stream(cluster: &K8sCluster) -> Vec<String> {
    let mut v = Vec::with_capacity(6);
    if let Some(p) = cluster
        .kubeconfig_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        v.push("--kubeconfig".into());
        v.push(p.to_string());
    }
    v.push("--context".into());
    v.push(cluster.context_name.clone());
    v
}

impl Kubectl {
    pub fn new(
        program: impl Into<String>,
        cluster: &K8sCluster,
        env: Vec<(String, String)>,
    ) -> Self {
        Self {
            program: program.into(),
            base: base_args(cluster),
            base_stream: base_args_stream(cluster),
            env,
            reauth: None,
        }
    }

    /// Attach the KS-2 re-auth hook (cached-overlay handles only).
    pub fn with_reauth(mut self, reauth: Reauth) -> Self {
        self.reauth = Some(reauth);
        self
    }

    /// Full argv for a non-streaming call: base flags + `args`.
    pub fn argv<I, S>(&self, args: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut v = self.base.clone();
        v.extend(args.into_iter().map(Into::into));
        v
    }

    /// Full argv for a streaming call (no `--request-timeout`).
    pub fn argv_stream<I, S>(&self, args: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut v = self.base_stream.clone();
        v.extend(args.into_iter().map(Into::into));
        v
    }

    /// Run `kubectl <base> <args>` with the default timeout.
    pub async fn run<I, S>(&self, args: I) -> Result<CliOutput>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.run_timeout(args, DEFAULT_TIMEOUT).await
    }

    pub async fn run_timeout<I, S>(&self, args: I, timeout: Duration) -> Result<CliOutput>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let args: Vec<String> = args.into_iter().map(Into::into).collect();
        let argv = self.argv(args.iter().cloned());
        tracing::debug!(program = %self.program, args = ?argv, "kubectl");
        match run(&self.program, &argv, &self.env, timeout, None).await {
            Err(e) if should_reauth(&e, self.reauth.is_some()) => {
                tracing::info!("k8s: cached exec token rejected — dropping it and retrying once");
                let fresh = (self.reauth.as_ref().expect("checked").0)().await?;
                let argv = fresh.argv(args);
                run(&fresh.program, &argv, &fresh.env, timeout, None).await
            }
            other => other,
        }
    }

    /// [`Self::run_timeout`] with stdout capped at `cap` bytes (see
    /// [`run_raw_capped`]); same re-auth retry and failure classification.
    pub async fn run_timeout_capped<I, S>(
        &self,
        args: I,
        timeout: Duration,
        cap: usize,
    ) -> Result<CliOutput>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        async fn once(
            program: &str,
            argv: &[String],
            env: &[(String, String)],
            timeout: Duration,
            cap: usize,
        ) -> Result<CliOutput> {
            let out = run_raw_capped(program, argv, env, timeout, cap).await?;
            if out.status != 0 {
                return Err(classify_failure(program, &out.stderr));
            }
            Ok(out)
        }
        let args: Vec<String> = args.into_iter().map(Into::into).collect();
        let argv = self.argv(args.iter().cloned());
        tracing::debug!(program = %self.program, args = ?argv, "kubectl (capped)");
        match once(&self.program, &argv, &self.env, timeout, cap).await {
            Err(e) if should_reauth(&e, self.reauth.is_some()) => {
                tracing::info!("k8s: cached exec token rejected — dropping it and retrying once");
                let fresh = (self.reauth.as_ref().expect("checked").0)().await?;
                let argv = fresh.argv(args);
                once(&fresh.program, &argv, &fresh.env, timeout, cap).await
            }
            other => other,
        }
    }

    /// Run and parse stdout as JSON (`-o json` must be part of `args`).
    pub async fn json<I, S>(&self, args: I) -> Result<Value>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let out = self.run(args).await?;
        parse_json(&out.stdout)
    }
}

/// Parse kubectl's JSON output; an empty stdout (e.g. `get` with nothing to
/// show in some versions) is an empty object rather than an error.
pub fn parse_json(stdout: &str) -> Result<Value> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_str(trimmed)
        .map_err(|e| Error::Upstream(format!("kubectl returned invalid JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_raw_capped_kills_an_endless_producer_at_the_cap() {
        // S6-09: `yes` never ends — `wait_with_output` would grow without
        // bound; the capped runner stops at `cap` bytes, well inside the
        // timeout, and reports the cut as success.
        let started = Instant::now();
        let out = run_raw_capped(
            "sh",
            &["-c".into(), "yes 0123456789".into()],
            &[],
            Duration::from_secs(20),
            100_000,
        )
        .await
        .unwrap();
        assert_eq!(out.status, 0);
        assert_eq!(out.stdout.len(), 100_000);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn run_raw_capped_passes_small_output_and_exit_codes_through() {
        let out = run_raw_capped(
            "sh",
            &["-c".into(), "printf hello; echo oops >&2; exit 3".into()],
            &[],
            Duration::from_secs(20),
            1000,
        )
        .await
        .unwrap();
        assert_eq!(out.status, 3);
        assert_eq!(out.stdout, "hello");
        assert!(out.stderr.contains("oops"));
    }
    use chrono::Utc;
    use otto_core::domain::Environment;
    use otto_state::K8sClusterSource;

    fn cluster(path: Option<&str>) -> K8sCluster {
        K8sCluster {
            id: "01".into(),
            name: "c".into(),
            source: K8sClusterSource::Kubeconfig,
            kubeconfig_path: path.map(str::to_string),
            context_name: "prod-eu".into(),
            default_namespace: None,
            aws_account_id: None,
            environment: Environment::Dev,
            color: None,
            known_namespaces: Vec::new(),
            params: serde_json::json!({}),
            capabilities: None,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_used_at: None,
        }
    }

    #[test]
    fn base_args_with_and_without_kubeconfig() {
        assert_eq!(
            base_args(&cluster(Some("/x/kube.yaml"))),
            vec![
                "--kubeconfig",
                "/x/kube.yaml",
                "--context",
                "prod-eu",
                "--request-timeout",
                "20s"
            ]
        );
        assert_eq!(
            base_args(&cluster(None)),
            vec!["--context", "prod-eu", "--request-timeout", "20s"]
        );
        // Blank path is treated as NULL (UI sends "" when the field is cleared).
        assert_eq!(base_args(&cluster(Some("  "))), base_args(&cluster(None)));
        assert_eq!(
            base_args_stream(&cluster(Some("/x/kube.yaml"))),
            vec!["--kubeconfig", "/x/kube.yaml", "--context", "prod-eu"]
        );
    }

    #[test]
    fn argv_appends_after_base() {
        let k = Kubectl::new("kubectl", &cluster(None), vec![]);
        assert_eq!(
            k.argv(["get", "pods", "-o", "json"]),
            vec![
                "--context",
                "prod-eu",
                "--request-timeout",
                "20s",
                "get",
                "pods",
                "-o",
                "json"
            ]
        );
        assert_eq!(
            k.argv_stream(["logs", "-f", "p"]),
            vec!["--context", "prod-eu", "logs", "-f", "p"]
        );
    }

    #[test]
    fn forbidden_is_classified_as_rbac() {
        let e = classify_failure(
            "kubectl",
            "Error from server (Forbidden): pods is forbidden: User \"dev\" cannot list resource \"pods\" in API group \"\" in the namespace \"kube-system\"\n",
        );
        match e {
            Error::Forbidden(m) => {
                assert!(
                    m.starts_with("cluster RBAC: Error from server (Forbidden)"),
                    "{m}"
                );
                assert!(!m.contains('\n'));
            }
            other => panic!("expected Forbidden, got {other:?}"),
        }
    }

    #[test]
    fn not_found_and_generic_failures() {
        assert!(matches!(
            classify_failure(
                "kubectl",
                "Error from server (NotFound): pods \"x\" not found"
            ),
            Error::NotFound(_)
        ));
        match classify_failure(
            "/opt/bin/kubectl",
            "error: the server doesn't have a resource type \"rollouts\"",
        ) {
            Error::Invalid(m) => assert_eq!(
                m,
                "kubectl: error: the server doesn't have a resource type \"rollouts\""
            ),
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(classify_failure("kubectl", ""), Error::Invalid(m) if m == "kubectl failed")
        );
    }

    #[test]
    fn error_messages_are_redacted() {
        let stderr =
            "error: You must be logged in to the server (Unauthorized) token=AKIAIOSFODNN7EXAMPLE";
        match classify_failure("kubectl", stderr) {
            Error::Invalid(m) => {
                assert!(!m.contains("AKIAIOSFODNN7EXAMPLE"), "{m}");
                assert!(m.contains("[redacted]"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unauthorized_is_classified_as_credentials_rejected() {
        for stderr in [
            "error: You must be logged in to the server (Unauthorized)",
            "E1003 memcache.go:265] couldn't get current server API group list: the server has asked for the client to provide credentials\nerror: You must be logged in to the server (the server has asked for the client to provide credentials)",
            "Error from server (Unauthorized): pods is forbidden",
        ] {
            let e = classify_failure("kubectl", stderr);
            assert!(is_credentials_rejected(&e), "{stderr} => {e:?}");
        }
        assert!(!is_credentials_rejected(&classify_failure(
            "kubectl",
            "Error from server (NotFound): pods \"x\" not found"
        )));
    }

    #[test]
    fn reauth_only_for_rejected_credentials_with_a_hook() {
        let rejected = classify_failure("kubectl", "error: Unauthorized");
        assert!(should_reauth(&rejected, true));
        assert!(!should_reauth(&rejected, false));
        let other = classify_failure("kubectl", "error: boom");
        assert!(!should_reauth(&other, true));
    }

    #[test]
    fn missing_binary_is_not_installed() {
        let e = spawn_error(
            "kubectl",
            &std::io::Error::from(std::io::ErrorKind::NotFound),
        );
        assert!(matches!(e, Error::Invalid(m) if m == NOT_INSTALLED_MSG));
        let e = spawn_error(
            "/data/bin/k9s",
            &std::io::Error::from(std::io::ErrorKind::NotFound),
        );
        assert!(matches!(e, Error::Invalid(m) if m.starts_with("k9s not installed")));
    }

    #[tokio::test]
    async fn run_reports_enoent_as_not_installed() {
        let r = run(
            "/definitely/not/a/kubectl",
            &["version".to_string()],
            &[],
            Duration::from_secs(5),
            None,
        )
        .await;
        assert!(matches!(r, Err(Error::Invalid(m)) if m.contains("not installed")));
    }

    #[tokio::test]
    async fn run_raw_returns_exit_status_and_feeds_stdin() {
        let out = run_raw(
            "sh",
            &["-c".to_string(), "cat; exit 3".to_string()],
            &[],
            Duration::from_secs(5),
            Some(b"hello"),
        )
        .await
        .unwrap();
        assert_eq!(out.status, 3);
        assert_eq!(out.stdout, "hello");
        assert!(matches!(
            run("sh", &["-c".into(), "echo boom >&2; exit 1".into()], &[], Duration::from_secs(5), None).await,
            Err(Error::Invalid(m)) if m == "sh: boom"
        ));
    }

    #[tokio::test]
    async fn run_times_out() {
        let r = run(
            "sh",
            &["-c".to_string(), "sleep 5".to_string()],
            &[],
            Duration::from_millis(200),
            None,
        )
        .await;
        assert!(matches!(r, Err(Error::Upstream(m)) if m.contains("timed out")));
    }

    #[test]
    fn parse_json_tolerates_empty() {
        assert!(parse_json("  \n").unwrap().is_object());
        assert_eq!(parse_json("{\"a\":1}").unwrap()["a"], 1);
        assert!(parse_json("nope").is_err());
    }
}
