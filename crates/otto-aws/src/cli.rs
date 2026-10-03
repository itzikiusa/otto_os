//! `aws` CLI runner — the only way this crate talks to AWS.
//!
//! Every call is a `tokio::process::Command` (never a shell string) with
//! `kill_on_drop(true)` and a wall-clock timeout. stderr is classified before
//! it reaches a caller: credential-expiry shapes become `login required: …`
//! (the UI keys a "Sign in" button off that prefix), AccessDenied shapes become
//! `Error::Forbidden`, everything else is `Error::Invalid` with the stderr
//! **redacted** (`otto_core::redact` + AWS secret-key / session-token shapes).

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{Semaphore, SemaphorePermit};

/// Default per-call budget (§1). Streams (S3 download) are exempt.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Budget for each permission-probe call (§1 "8 s each").
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
/// The message every caller sees when the binary is absent. The UI keys off
/// the `not installed` substring to show the first-run install panel.
pub const NOT_INSTALLED_MSG: &str = "aws CLI not installed — open the AWS module to install it";

/// Daemon-wide cap on concurrent `aws` children spawned through [`run_raw`].
/// Each child is a ~60–100 MB Python process: without a global cap the
/// accounts overview (N accounts × 7 permission probes) plus a couple of
/// all-regions fan-outs could hold dozens at once. Streams that spawn their own
/// child (S3 download/upload, `sso login` PTY) are deliberately not counted —
/// they are long-lived and user-initiated, and must never queue behind list
/// calls (see [`Lane::Stream`]).
pub const MAX_CONCURRENT_CHILDREN: usize = 10;
/// Share of the cap that background work (permission probes, all-regions
/// fan-outs) may hold. The rest is reserved for what a person just clicked, so
/// a "Refresh" never waits behind a dozen queued probes (N1).
pub const BACKGROUND_PERMITS: usize = 6;
/// Permits only interactive calls can take. Interactive calls also borrow a
/// free background permit, so a quiet daemon still runs 10 clicks at once.
pub const INTERACTIVE_PERMITS: usize = MAX_CONCURRENT_CHILDREN - BACKGROUND_PERMITS;

/// Which share of the child cap a call draws from. Set for a whole subtree of
/// calls with [`background`] / [`uncapped`]; the default is `Interactive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// A request a person is waiting on: its own 4 permits, plus any free
    /// background permit.
    Interactive,
    /// Fan-outs and probes: at most [`BACKGROUND_PERMITS`] at once.
    Background,
    /// Long user-initiated transfers (S3 upload): not counted at all, like the
    /// download stream — an hour-long upload must not pin a list slot (N2).
    Stream,
}

tokio::task_local! {
    static LANE: Lane;
}

/// Run `f` with every `aws` call inside it on the background lane.
pub async fn background<F: std::future::Future>(f: F) -> F::Output {
    LANE.scope(Lane::Background, f).await
}

/// Run `f` with every `aws` call inside it outside the child cap.
pub async fn uncapped<F: std::future::Future>(f: F) -> F::Output {
    LANE.scope(Lane::Stream, f).await
}

/// The lane of the current task (`Interactive` outside any scope).
pub fn current_lane() -> Lane {
    LANE.try_with(|l| *l).unwrap_or(Lane::Interactive)
}

fn interactive_sem() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(INTERACTIVE_PERMITS))
}

fn background_sem() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| Semaphore::new(BACKGROUND_PERMITS))
}

/// Take a child slot for `lane` (`None` for [`Lane::Stream`]).
async fn acquire(lane: Lane) -> Result<Option<SemaphorePermit<'static>>> {
    let closed = |_| Error::Internal("aws runner closed".into());
    let (fg, bg) = (interactive_sem(), background_sem());
    match lane {
        Lane::Stream => Ok(None),
        Lane::Background => bg.acquire().await.map(Some).map_err(closed),
        Lane::Interactive => {
            if let Ok(p) = fg.try_acquire() {
                return Ok(Some(p));
            }
            if let Ok(p) = bg.try_acquire() {
                return Ok(Some(p));
            }
            // Whichever frees first: the own lane, or a borrowed background slot.
            tokio::select! {
                p = fg.acquire() => p.map(Some).map_err(closed),
                p = bg.acquire() => p.map(Some).map_err(closed),
            }
        }
    }
}

/// Total `aws` children spawned by [`run_raw`] since the daemon started.
static SPAWNED: AtomicU64 = AtomicU64::new(0);

/// Live counters for the CLI runner (debug / metrics surface, F14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CliStats {
    /// `aws` children running right now (permits in use, both lanes).
    pub running: usize,
    /// Calls waiting for a permit (approximate: Semaphore has no waiter count,
    /// so this is tracked alongside it).
    pub queued: u64,
    /// Children spawned since start.
    pub spawned_total: u64,
    pub max_concurrent: usize,
    /// Of `max_concurrent`, how many background work may hold.
    pub background_max: usize,
    /// Queue wait (permit acquire) and child wall time over the last
    /// [`SAMPLE_WINDOW`] calls, in ms (0 before the first call).
    pub wait_ms_p50: u64,
    pub wait_ms_p95: u64,
    pub call_ms_p50: u64,
    pub call_ms_p95: u64,
    /// Calls in the percentile window.
    pub samples: usize,
}

static QUEUED: AtomicU64 = AtomicU64::new(0);

/// How many recent calls the wait / latency percentiles cover.
pub const SAMPLE_WINDOW: usize = 256;

/// Ring of `(wait_ms, call_ms)` for the most recent calls.
fn samples() -> &'static Mutex<VecDeque<(u64, u64)>> {
    static S: OnceLock<Mutex<VecDeque<(u64, u64)>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(VecDeque::with_capacity(SAMPLE_WINDOW)))
}

fn record(wait_ms: u64, call_ms: u64) {
    let mut s = samples().lock().unwrap_or_else(|p| p.into_inner());
    if s.len() == SAMPLE_WINDOW {
        s.pop_front();
    }
    s.push_back((wait_ms, call_ms));
}

/// Nearest-rank percentile of an ascending slice (0 when empty).
fn percentile(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (pct * sorted.len()).div_ceil(100).max(1);
    sorted[rank.min(sorted.len()) - 1]
}

pub fn stats() -> CliStats {
    let (mut waits, mut calls): (Vec<u64>, Vec<u64>) = samples()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .copied()
        .unzip();
    waits.sort_unstable();
    calls.sort_unstable();
    CliStats {
        running: (INTERACTIVE_PERMITS - interactive_sem().available_permits())
            + (BACKGROUND_PERMITS - background_sem().available_permits()),
        queued: QUEUED.load(Ordering::Relaxed),
        spawned_total: SPAWNED.load(Ordering::Relaxed),
        max_concurrent: MAX_CONCURRENT_CHILDREN,
        background_max: BACKGROUND_PERMITS,
        wait_ms_p50: percentile(&waits, 50),
        wait_ms_p95: percentile(&waits, 95),
        call_ms_p50: percentile(&calls, 50),
        call_ms_p95: percentile(&calls, 95),
        samples: calls.len(),
    }
}

/// `svc op` of an argv for logs (`s3api list-objects-v2`); never the
/// remaining args, which can carry keys / bucket paths / query text.
fn op_of(args: &[String]) -> (&str, &str) {
    let mut it = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('-'));
    (it.next().unwrap_or(""), it.next().unwrap_or(""))
}

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
    // Wait for a daemon-wide slot BEFORE spawning; the permit lives until the
    // child is reaped (or this future is dropped, which kills the child). The
    // caller's budget covers the queue wait too (N1): a call stuck behind a
    // saturated cap fails within `timeout` instead of waiting forever.
    let lane = current_lane();
    let queued_at = Instant::now();
    let deadline = tokio::time::Instant::now() + timeout;
    QUEUED.fetch_add(1, Ordering::Relaxed);
    let permit = tokio::time::timeout_at(deadline, acquire(lane)).await;
    QUEUED.fetch_sub(1, Ordering::Relaxed);
    let (svc, op) = op_of(args);
    let _permit = match permit {
        Ok(p) => p?,
        Err(_) => {
            tracing::debug!(target: "otto_aws::cli", svc, op, ?lane, wait_ms = queued_at.elapsed().as_millis() as u64, status = "queue-timeout", "aws call");
            return Err(Error::Upstream(format!(
                "aws timed out after {}s waiting for a free CLI slot (too many AWS calls in flight)",
                timeout.as_secs()
            )));
        }
    };
    let wait_ms = queued_at.elapsed().as_millis() as u64;
    let started = Instant::now();
    let mut cmd = Command::new(program);
    // The child never inherits the daemon's own AWS_PROFILE: keys-mode accounts
    // would otherwise be resolved through it (and blanking it instead makes
    // the CLI look for a profile named ""). Profile mode re-adds it via `env`.
    cmd.args(args)
        .env_remove("AWS_PROFILE")
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
    SPAWNED.fetch_add(1, Ordering::Relaxed);
    if let Some(bytes) = stdin {
        if let Some(mut si) = child.stdin.take() {
            let bytes = bytes.to_vec();
            tokio::spawn(async move {
                let _ = si.write_all(&bytes).await;
                let _ = si.shutdown().await;
            });
        }
    }
    let out = match tokio::time::timeout_at(deadline, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(Error::Internal(format!("aws: {e}"))),
        Err(_) => {
            let ms = started.elapsed().as_millis() as u64;
            record(wait_ms, ms);
            tracing::debug!(target: "otto_aws::cli", svc, op, ?lane, wait_ms, ms, status = "timeout", "aws call");
            return Err(Error::Upstream(format!(
                "aws timed out after {}s",
                timeout.as_secs()
            )));
        }
    };
    let out = CliOutput {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        duration_ms: started.elapsed().as_millis() as u64,
    };
    // Per-call timing (F14): `RUST_LOG=otto_aws::cli=debug` shows every spawn
    // with its wall time, so a slow view can be attributed to a specific call.
    record(wait_ms, out.duration_ms);
    tracing::debug!(target: "otto_aws::cli", svc, op, ?lane, wait_ms, ms = out.duration_ms, status = out.status, "aws call");
    Ok(out)
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

    #[test]
    fn op_of_names_only_service_and_operation() {
        let a: Vec<String> = ["s3api", "get-object", "--bucket", "b", "--key", "secret/k"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(op_of(&a), ("s3api", "get-object"));
        assert_eq!(op_of(&[]), ("", ""));
    }

    /// The child cap is process-wide: tests that measure it run one at a time.
    fn cap_lock() -> &'static tokio::sync::Mutex<()> {
        static L: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        L.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    /// A fake `aws` that sleeps `secs` and records its live peak in `dir`.
    #[cfg(unix)]
    fn sleeper(dir: &Path, name: &str, secs: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join(name);
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nf=\"{d}/live.$$\"\ntouch \"$f\"\nls {d} | grep -c '^live' >> {d}/peaks\nsleep {secs}\nrm -f \"$f\"\n",
                d = dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    #[cfg(unix)]
    fn peak_of(dir: &Path) -> usize {
        std::fs::read_to_string(dir.join("peaks"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse::<usize>().ok())
            .max()
            .unwrap_or(0)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn background_lane_leaves_room_for_clicks_and_records_wait() {
        let _g = cap_lock().lock().await;
        let bg_dir = tempfile::tempdir().unwrap();
        let fg_dir = tempfile::tempdir().unwrap();
        let slow = sleeper(bg_dir.path(), "aws", "0.6");
        let quick = sleeper(fg_dir.path(), "aws", "0");
        // Twice the background share queued: 6 run, 6 wait.
        let probes = futures_util::future::join_all((0..BACKGROUND_PERMITS * 2).map(|_| {
            let slow = slow.clone();
            async move { background(run_raw(&slow, &[], &[], DEFAULT_TIMEOUT, None)).await }
        }));
        let click = async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let t = Instant::now();
            let out = run_raw(&quick, &[], &[], DEFAULT_TIMEOUT, None).await;
            (out, t.elapsed())
        };
        let (probes, (click, click_wall)) = tokio::join!(probes, click);
        assert!(probes.iter().all(|o| o.as_ref().is_ok_and(CliOutput::ok)));
        assert!(click.unwrap().ok());
        // Background never exceeded its share…
        let peak = peak_of(bg_dir.path());
        assert!(peak <= BACKGROUND_PERMITS, "background peak {peak}");
        // …and the click did not wait for a probe to finish (≥ 600 ms).
        assert!(
            click_wall < Duration::from_millis(500),
            "interactive call queued behind probes: {click_wall:?}"
        );
        let s = stats();
        assert!(s.samples > 0 && s.call_ms_p95 >= s.call_ms_p50, "{s:?}");
        // The second wave of probes waited ~one probe duration for a slot.
        assert!(s.wait_ms_p95 >= 300, "{s:?}");
        assert_eq!(s.background_max, BACKGROUND_PERMITS);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_timeout_covers_the_queue_wait() {
        let _g = cap_lock().lock().await;
        let dir = tempfile::tempdir().unwrap();
        let slow = sleeper(dir.path(), "aws", "1.2");
        let hold = futures_util::future::join_all((0..BACKGROUND_PERMITS).map(|_| {
            let slow = slow.clone();
            async move { background(run_raw(&slow, &[], &[], DEFAULT_TIMEOUT, None)).await }
        }));
        let late = async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let t = Instant::now();
            let r = background(run_raw(&slow, &[], &[], Duration::from_millis(300), None)).await;
            (r, t.elapsed())
        };
        let (_, (r, wall)) = tokio::join!(hold, late);
        match r {
            Err(Error::Upstream(m)) => assert!(m.contains("waiting for a free CLI slot"), "{m}"),
            other => panic!("expected a queue timeout, got {other:?}"),
        }
        assert!(wall < Duration::from_millis(900), "waited {wall:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stream_lane_is_not_counted() {
        let _g = cap_lock().lock().await;
        let dir = tempfile::tempdir().unwrap();
        let slow = sleeper(dir.path(), "aws", "0.3");
        let before = stats().running;
        let r = uncapped(async {
            let fut = run_raw(&slow, &[], &[], DEFAULT_TIMEOUT, None);
            tokio::pin!(fut);
            // While the child runs, no permit is held.
            tokio::select! {
                _ = &mut fut => unreachable!("the child sleeps 300 ms"),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
            assert_eq!(stats().running, before);
            fut.await
        })
        .await;
        assert!(r.unwrap().ok());
        assert_eq!(current_lane(), Lane::Interactive);
    }

    #[test]
    fn percentile_is_nearest_rank() {
        assert_eq!(percentile(&[], 95), 0);
        assert_eq!(percentile(&[7], 50), 7);
        let v: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&v, 50), 50);
        assert_eq!(percentile(&v, 95), 95);
        assert_eq!(percentile(&v, 100), 100);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_children_are_capped_daemon_wide() {
        let _g = cap_lock().lock().await;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("aws");
        // Each child records itself while alive so the peak can be measured.
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nf=\"{d}/live.$$\"\ntouch \"$f\"\nls {d} | grep -c '^live' >> {d}/peaks\nsleep 0.15\nrm -f \"$f\"\n",
                d = dir.path().display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let before = stats().spawned_total;
        let calls = (0..(MAX_CONCURRENT_CHILDREN * 2)).map(|_| {
            let bin = bin.clone();
            async move { run_raw(&bin, &[], &[], DEFAULT_TIMEOUT, None).await }
        });
        let outs = futures_util::future::join_all(calls).await;
        assert!(outs
            .iter()
            .all(|o| o.as_ref().map(|o| o.ok()).unwrap_or(false)));
        let peaks = std::fs::read_to_string(dir.path().join("peaks")).unwrap();
        let peak = peaks
            .lines()
            .filter_map(|l| l.trim().parse::<usize>().ok())
            .max()
            .unwrap();
        assert!(peak <= MAX_CONCURRENT_CHILDREN, "peak {peak} children");
        assert!(stats().spawned_total >= before + (MAX_CONCURRENT_CHILDREN as u64 * 2));
        assert_eq!(stats().max_concurrent, MAX_CONCURRENT_CHILDREN);
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
