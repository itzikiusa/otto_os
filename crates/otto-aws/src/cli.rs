//! `aws` CLI runner — the only way this crate talks to AWS.
//!
//! Every call is a `tokio::process::Command` (never a shell string) with
//! `kill_on_drop(true)` and a wall-clock timeout. stderr is classified before
//! it reaches a caller: credential-expiry shapes become `login required: …`
//! (the UI keys a "Sign in" button off that prefix), AccessDenied shapes become
//! `Error::Forbidden`, everything else is `Error::Invalid` with the stderr
//! **redacted** (`otto_core::redact` + AWS secret-key / session-token shapes).

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// Default per-call budget (§1). Streams (S3 download) are exempt.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Budget for each permission-probe call (§1 "8 s each").
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// The message every caller sees when the binary is absent. The UI keys off
/// the `not installed` substring to show the first-run install panel.
pub const NOT_INSTALLED_MSG: &str = "aws CLI not installed — open the AWS module to install it";

/// Raw result of one CLI invocation (exit status is NOT interpreted here).
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

impl CliOutput {
    pub fn ok(&self) -> bool {
        self.status == 0
    }
}

/// How a failed call's stderr should be surfaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StderrClass {
    /// Credentials are missing/expired — the user must `aws sso login` (or
    /// re-enter keys).
    LoginRequired,
    /// IAM said no.
    AccessDenied,
    /// Anything else (bad args, throttling, network…).
    Other,
}

/// The expiry / missing-credential shapes from the contract (§1) plus the
/// SSO-cache variants the CLI actually prints.
const EXPIRED_PATTERNS: &[&str] = &[
    "ExpiredToken",
    "ExpiredTokenException",
    "UnauthorizedSSOTokenError",
    "Error loading SSO Token",
    "The SSO session associated with this profile has expired",
    "Unable to locate credentials",
    "Token has expired and refresh failed",
    "The security token included in the request is expired",
    "InvalidClientTokenId",
];

const DENIED_PATTERNS: &[&str] = &[
    "AccessDenied",
    "AccessDeniedException",
    "UnauthorizedOperation",
    "not authorized to perform",
];

/// Classify a failed call's stderr. Expiry wins over denial (an expired SSO
/// token can also surface as a 403-ish message from some services).
pub fn classify_stderr(stderr: &str) -> StderrClass {
    if EXPIRED_PATTERNS.iter().any(|p| stderr.contains(p)) {
        return StderrClass::LoginRequired;
    }
    if DENIED_PATTERNS.iter().any(|p| stderr.contains(p)) {
        return StderrClass::AccessDenied;
    }
    StderrClass::Other
}

/// Redact secrets the CLI might echo: the generic scrubber (AKIA ids, PEM
/// blocks, bearer tokens, emails) plus the two AWS shapes it does not know —
/// 40-char secret keys and long session tokens — and any `--secret-access-key`
/// / `--session-token` argv echoes.
pub fn redact_stderr(stderr: &str) -> String {
    let base = otto_core::redact::redact_text(stderr).value;
    let mut out: Vec<String> = Vec::new();
    let mut redact_next = false;
    for tok in base.split(' ') {
        if redact_next && !tok.is_empty() {
            out.push("[redacted]".into());
            redact_next = false;
            continue;
        }
        let bare = tok
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '/' && c != '=');
        if matches!(
            tok,
            "--secret-access-key" | "--session-token" | "--secret_access_key"
        ) {
            out.push(tok.into());
            redact_next = true;
        } else if looks_like_aws_secret(bare) {
            out.push(tok.replace(bare, "[redacted]"));
        } else {
            out.push(tok.into());
        }
    }
    out.join(" ")
}

/// 40-char base64-ish secret access key, or a ≥100-char session token.
fn looks_like_aws_secret(w: &str) -> bool {
    let b64 = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    };
    (w.len() == 40
        && b64(w)
        && w.chars().any(|c| c.is_ascii_digit())
        && w.chars().any(|c| c.is_ascii_uppercase())
        && w.chars().any(|c| c.is_ascii_lowercase()))
        || (w.len() >= 100 && b64(w))
}

/// Map a failed call to the crate's error contract.
pub fn error_for(out: &CliOutput) -> Error {
    let first_line = out
        .stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("aws exited with an error")
        .to_string();
    match classify_stderr(&out.stderr) {
        StderrClass::LoginRequired => {
            Error::Invalid(format!("login required: {}", redact_stderr(&first_line)))
        }
        StderrClass::AccessDenied => Error::Forbidden(redact_stderr(&first_line)),
        StderrClass::Other => {
            let msg = redact_stderr(out.stderr.trim());
            let msg = if msg.is_empty() {
                format!("aws exited with status {}", out.status)
            } else {
                msg
            };
            // Keep the payload bounded — a stack trace is not a UI message.
            let clipped: String = msg.chars().take(2000).collect();
            Error::Invalid(clipped)
        }
    }
}

/// Run `program args…` with `env` added to the daemon environment. Returns the
/// raw output (non-zero exit is NOT an error here — see [`run`]).
pub async fn run_raw(
    program: &Path,
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
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::Invalid(NOT_INSTALLED_MSG.into())
        } else {
            Error::Internal(format!("spawn {}: {e}", program.display()))
        }
    })?;
    if let Some(bytes) = stdin {
        if let Some(mut si) = child.stdin.take() {
            let bytes = bytes.to_vec();
            tokio::spawn(async move {
                let _ = si.write_all(&bytes).await;
                let _ = si.shutdown().await;
            });
        }
    }
    let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(Error::Internal(format!("aws: {e}"))),
        Err(_) => {
            return Err(Error::Upstream(format!(
                "aws timed out after {}s",
                timeout.as_secs()
            )))
        }
    };
    Ok(CliOutput {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// [`run_raw`] + error mapping: non-zero exit ⇒ [`error_for`].
pub async fn run(
    program: &Path,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
    stdin: Option<&[u8]>,
) -> Result<CliOutput> {
    let out = run_raw(program, args, env, timeout, stdin).await?;
    if out.ok() {
        Ok(out)
    } else {
        Err(error_for(&out))
    }
}

/// [`run`] and parse stdout as JSON (empty stdout ⇒ `null`, which is what the
/// CLI produces for void operations like `purge-queue`).
pub async fn run_json(
    program: &Path,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
) -> Result<serde_json::Value> {
    let out = run(program, args, env, timeout, None).await?;
    parse_stdout(&out.stdout)
}

pub fn parse_stdout(stdout: &str) -> Result<serde_json::Value> {
    let t = stdout.trim();
    if t.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_str(t)
        .map_err(|e| Error::Upstream(format!("aws returned non-JSON output: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_expired_vs_denied_vs_other() {
        assert_eq!(
            classify_stderr(
                "Error when retrieving token from sso: Token has expired and refresh failed"
            ),
            StderrClass::LoginRequired
        );
        assert_eq!(
            classify_stderr("An error occurred (ExpiredTokenException) when calling the GetCallerIdentity operation: The security token included in the request is expired"),
            StderrClass::LoginRequired
        );
        assert_eq!(
            classify_stderr("Unable to locate credentials. You can configure credentials by running \"aws configure\"."),
            StderrClass::LoginRequired
        );
        assert_eq!(
            classify_stderr(
                "Error loading SSO Token: Token for https://x.awsapps.com/start does not exist"
            ),
            StderrClass::LoginRequired
        );
        assert_eq!(
            classify_stderr("An error occurred (AccessDenied) when calling the ListBuckets operation: Access Denied"),
            StderrClass::AccessDenied
        );
        assert_eq!(
            classify_stderr("An error occurred (UnauthorizedOperation) when calling the DescribeInstances operation: You are not authorized to perform this operation."),
            StderrClass::AccessDenied
        );
        assert_eq!(
            classify_stderr("An error occurred (AccessDeniedException) when calling the ListWorkGroups operation"),
            StderrClass::AccessDenied
        );
        assert_eq!(
            classify_stderr("An error occurred (NoSuchBucket) when calling the ListObjectsV2 operation: The specified bucket does not exist"),
            StderrClass::Other
        );
        assert_eq!(classify_stderr(""), StderrClass::Other);
    }

    #[test]
    fn error_for_maps_to_contract_variants() {
        let mk = |stderr: &str| CliOutput {
            status: 254,
            stdout: String::new(),
            stderr: stderr.into(),
            duration_ms: 1,
        };
        match error_for(&mk("Error loading SSO Token: x\nsecond line")) {
            Error::Invalid(m) => assert!(
                m.starts_with("login required: Error loading SSO Token"),
                "{m}"
            ),
            e => panic!("{e:?}"),
        }
        assert!(matches!(
            error_for(&mk("An error occurred (AccessDenied) when calling X")),
            Error::Forbidden(_)
        ));
        match error_for(&mk("An error occurred (NoSuchBucket) when calling X")) {
            Error::Invalid(m) => assert!(m.contains("NoSuchBucket")),
            e => panic!("{e:?}"),
        }
        match error_for(&mk("")) {
            Error::Invalid(m) => assert_eq!(m, "aws exited with status 254"),
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn redacts_aws_key_shapes() {
        let s = "creds AKIAIOSFODNN7EXAMPLE / wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY --session-token FQoGZXIvYXdzEBYaDDDDDDDD failed";
        let r = redact_stderr(s);
        assert!(!r.contains("AKIAIOSFODNN7EXAMPLE"), "{r}");
        assert!(
            !r.contains("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            "{r}"
        );
        assert!(!r.contains("FQoGZXIvYXdzEBYaDDDDDDDD"), "{r}");
        assert!(r.contains("failed"));
        // Ordinary words and ARNs survive.
        let plain = "An error occurred (NoSuchBucket) when calling the ListObjectsV2 operation: arn:aws:s3:::my-bucket";
        assert_eq!(redact_stderr(plain), plain);
    }

    #[test]
    fn parse_stdout_handles_empty_and_json() {
        assert_eq!(parse_stdout("  \n").unwrap(), serde_json::Value::Null);
        assert_eq!(parse_stdout("{\"a\":1}").unwrap()["a"], 1);
        assert!(matches!(parse_stdout("nope"), Err(Error::Upstream(_))));
    }

    #[tokio::test]
    async fn missing_binary_is_not_installed() {
        let e = run(
            Path::new("/definitely/not/aws"),
            &[],
            &[],
            DEFAULT_TIMEOUT,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(e, Error::Invalid(m) if m.contains("not installed")));
    }
}
