//! Pod logs — one-shot (capped) and `follow=true` streaming.
//!
//! Non-follow: `kubectl logs …` with a 60 s wall clock and a 5 MiB cap (the tail
//! is kept — newest lines matter). Follow: the child's stdout is wrapped in an
//! axum streaming body; the `tokio::process::Child` lives inside the stream
//! state with `kill_on_drop(true)`, so when the client disconnects and axum
//! drops the body the `kubectl logs -f` process is killed with it (contract
//! §3.2). Streams use the no-`--request-timeout` base flags — kubectl applies
//! that flag to the HTTP client and would cut a long follow.

use std::process::Stdio;
use std::time::Duration;

use axum::body::Body;
use futures_util::stream;
use otto_core::{Error, Result};
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::cli::{self, Kubectl};
use crate::resources;

/// One-shot budget.
pub const LOGS_TIMEOUT: Duration = Duration::from_secs(60);
/// One-shot byte cap.
pub const LOGS_CAP: usize = 5 * 1024 * 1024;

/// `GET …/logs` query (contract §3.2).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LogsQuery {
    pub container: Option<String>,
    pub tail: Option<i64>,
    pub since: Option<String>,
    #[serde(default)]
    pub previous: Option<bool>,
    #[serde(default)]
    pub follow: Option<bool>,
    #[serde(default)]
    pub timestamps: Option<bool>,
}

/// What `kubectl logs` reads from: one pod, or every pod matching a label
/// selector (a workload's `spec.selector`) — the latter with `--prefix` so
/// each line carries `[pod/<pod>/<container>] ` for the UI's per-pod filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogTarget<'a> {
    Pod(&'a str),
    Selector(&'a str),
}

/// `GET …/logs?ns=&selector=` query (workload-level logs). The log options
/// are repeated rather than `#[serde(flatten)]`ed: serde_urlencoded can't
/// deserialize numbers/bools through a flattened struct.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SelectorLogsQuery {
    pub ns: String,
    pub selector: String,
    pub container: Option<String>,
    pub tail: Option<i64>,
    pub since: Option<String>,
    #[serde(default)]
    pub previous: Option<bool>,
    #[serde(default)]
    pub follow: Option<bool>,
    #[serde(default)]
    pub timestamps: Option<bool>,
}

impl SelectorLogsQuery {
    pub fn logs(&self) -> LogsQuery {
        LogsQuery {
            container: self.container.clone(),
            tail: self.tail,
            since: self.since.clone(),
            previous: self.previous,
            follow: self.follow,
            timestamps: self.timestamps,
        }
    }
}

/// Upper bound on the pods one selector stream fans out to (kubectl's
/// `--max-log-requests`, default 5 — far too low for a real deployment).
pub const MAX_LOG_REQUESTS: usize = 100;

/// Build the `logs` argv (after the base flags) from the query.
pub fn logs_args(ns: &str, pod: &str, q: &LogsQuery) -> Vec<String> {
    target_args(ns, LogTarget::Pod(pod), q)
}

/// Reject a pod / container / namespace that could be parsed as a kubectl
/// flag (`--context=other`, `-A`, `--server=…`) — see
/// [`crate::resources::validate_name`]. The selector is passed as a single
/// `--selector=<sel>` token, so it can't smuggle a flag of its own.
pub fn validate_target(ns: &str, target: &LogTarget<'_>, q: &LogsQuery) -> Result<()> {
    resources::validate_name("namespace", ns)?;
    match target {
        LogTarget::Pod(pod) => resources::validate_name("pod", pod)?,
        LogTarget::Selector(sel) => {
            if sel.trim().is_empty() {
                return Err(Error::Invalid("selector is required".into()));
            }
        }
    }
    if let Some(c) = q
        .container
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        resources::validate_name("container", c)?;
    }
    Ok(())
}

/// Largest `--tail` we ask for: with the byte cap below it is only a hint, but
/// it stops a huge tail from making the API server stream a whole log.
pub const MAX_TAIL: i64 = 100_000;

/// `logs_args` for either target. Every option is one `--flag=value` token
/// and the pod name follows `--`, so no value can be read as a flag.
pub fn target_args(ns: &str, target: LogTarget<'_>, q: &LogsQuery) -> Vec<String> {
    let mut a: Vec<String> = vec!["logs".into()];
    let container = q
        .container
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    if let LogTarget::Selector(sel) = target {
        a.push(format!("--selector={sel}"));
        a.push("--prefix".into());
        a.push(format!("--max-log-requests={MAX_LOG_REQUESTS}"));
        if container.is_none() {
            a.push("--all-containers".into());
        }
    }
    a.push(format!("--namespace={ns}"));
    if let Some(c) = container {
        a.push(format!("--container={c}"));
    }
    let tail = q.tail.unwrap_or(500).min(MAX_TAIL);
    if tail >= 0 {
        a.push(format!("--tail={tail}"));
    }
    if let Some(s) = q.since.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // Accept both a duration ("10m") and an RFC3339 instant.
        if s.contains('T') && s.contains(':') {
            a.push(format!("--since-time={s}"));
        } else {
            a.push(format!("--since={s}"));
        }
    }
    if q.previous == Some(true) {
        a.push("--previous".into());
    }
    if q.timestamps == Some(true) {
        a.push("--timestamps".into());
    }
    if q.follow == Some(true) {
        a.push("-f".into());
    } else {
        // One-shot: the server stops at the cap instead of us buffering past it.
        a.push(format!("--limit-bytes={LOGS_CAP}"));
    }
    if let LogTarget::Pod(pod) = target {
        a.push("--".into());
        a.push(pod.into());
    }
    a
}

/// Keep the last `cap` bytes on a UTF-8 line boundary and mark truncation.
pub fn cap_tail(text: String, cap: usize) -> String {
    if text.len() <= cap {
        return text;
    }
    let mut start = text.len() - cap;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let cut = text[start..]
        .find('\n')
        .map(|i| start + i + 1)
        .unwrap_or(start);
    format!(
        "[otto: output truncated to the last {} bytes]\n{}",
        cap,
        &text[cut..]
    )
}

/// One-shot logs (`follow` ignored): text, tail-capped.
pub async fn fetch(k: &Kubectl, ns: &str, pod: &str, q: &LogsQuery) -> Result<String> {
    fetch_target(k, ns, LogTarget::Pod(pod), q).await
}

/// One-shot logs for either target.
pub async fn fetch_target(
    k: &Kubectl,
    ns: &str,
    target: LogTarget<'_>,
    q: &LogsQuery,
) -> Result<String> {
    validate_target(ns, &target, q)?;
    let mut q = q.clone();
    q.follow = Some(false);
    let argv = k.argv(target_args(ns, target, &q));
    let mut cmd = Command::new(&k.program);
    cmd.args(&argv)
        .envs(k.env.iter().map(|(a, b)| (a.as_str(), b.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| cli::spawn_error(&k.program, &e))?;
    let (status, text, stderr) = read_capped(child, LOGS_CAP, LOGS_TIMEOUT)
        .await
        .map_err(|e| match e {
            ReadCapped::Timeout => Error::Upstream(format!(
                "{} logs timed out after {}s",
                k.program,
                LOGS_TIMEOUT.as_secs()
            )),
            ReadCapped::Io(e) => Error::Internal(format!("{}: {e}", k.program)),
        })?;
    if status != 0 {
        return Err(cli::classify_failure(&k.program, &stderr));
    }
    Ok(text)
}

#[derive(Debug)]
enum ReadCapped {
    Timeout,
    Io(std::io::Error),
}

/// Read a child's stdout incrementally, keeping only its last `cap` bytes
/// (a ring — memory stays at `cap` however much kubectl prints), with stderr
/// drained concurrently into its own small ring. Past `deadline` the child is
/// killed. Returns `(exit, tail-capped stdout, stderr tail)`.
async fn read_capped(
    mut child: Child,
    cap: usize,
    deadline: Duration,
) -> std::result::Result<(i32, String, String), ReadCapped> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ReadCapped::Io(std::io::Error::other("no stdout")))?;
    let (err_ring, drain) = drain_stderr(child.stderr.take());
    let read = async {
        let mut ring: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
        let mut truncated = false;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = stdout.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            ring.extend(&buf[..n]);
            if ring.len() > cap {
                let excess = ring.len() - cap;
                ring.drain(..excess);
                truncated = true;
            }
        }
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((status.code().unwrap_or(-1), Vec::from(ring), truncated))
    };
    let (status, bytes, truncated) = match tokio::time::timeout(deadline, read).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(ReadCapped::Io(e)),
        // Dropping `read` drops `child` (kill_on_drop) — the process dies here.
        Err(_) => return Err(ReadCapped::Timeout),
    };
    if let Some(t) = drain {
        let _ = tokio::time::timeout(Duration::from_secs(2), t).await;
    }
    let stderr =
        String::from_utf8_lossy(&err_ring.lock().unwrap_or_else(|p| p.into_inner())).into_owned();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let text = if truncated {
        // The ring already holds the last `cap` bytes; drop the partial first
        // line and add the marker `cap_tail` would have.
        let cut = text.find('\n').map(|i| i + 1).unwrap_or(0);
        format!(
            "[otto: output truncated to the last {} bytes]\n{}",
            cap,
            &text[cut..]
        )
    } else {
        text
    };
    Ok((status, text, stderr))
}

/// Streaming logs: a `text/plain` body that stays open while `kubectl logs -f`
/// runs; dropping the body kills the child.
pub fn follow(k: &Kubectl, ns: &str, pod: &str, q: &LogsQuery) -> Result<Body> {
    follow_target(k, ns, LogTarget::Pod(pod), q)
}

/// Streaming logs for either target.
pub fn follow_target(k: &Kubectl, ns: &str, target: LogTarget<'_>, q: &LogsQuery) -> Result<Body> {
    let mut q = q.clone();
    validate_target(ns, &target, &q)?;
    q.follow = Some(true);
    let argv = k.argv_stream(target_args(ns, target, &q));
    let mut cmd = Command::new(&k.program);
    cmd.args(&argv)
        .envs(k.env.iter().map(|(a, b)| (a.as_str(), b.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| cli::spawn_error(&k.program, &e))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("kubectl logs: no stdout".into()))?;
    let stderr = child.stderr.take();
    Ok(Body::from_stream(child_stream(child, stdout, stderr)))
}

/// Last bytes of stderr kept while a follow stream runs.
const STDERR_RING: usize = 8 * 1024;

/// Drain `stderr` for the life of the child into a ring of its last
/// `STDERR_RING` bytes. Draining concurrently matters: kubectl blocks once
/// 64 KiB of unread stderr (warnings across a 100-pod selector stream) fills
/// the pipe, which silently stalled the log stream.
fn drain_stderr(
    stderr: Option<tokio::process::ChildStderr>,
) -> (
    std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    Option<tokio::task::JoinHandle<()>>,
) {
    let ring = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let Some(mut err) = stderr else {
        return (ring, None);
    };
    let sink = ring.clone();
    let task = tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            match err.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut r = sink.lock().unwrap_or_else(|p| p.into_inner());
                    r.extend_from_slice(&buf[..n]);
                    if r.len() > STDERR_RING {
                        let cut = r.len() - STDERR_RING;
                        r.drain(..cut);
                    }
                }
            }
        }
    });
    (ring, Some(task))
}

/// Chunked reader over the child's stdout; the `Child` travels inside the
/// state so it is dropped (⇒ killed) together with the stream. stderr is
/// drained concurrently (see [`drain_stderr`]); when stdout closes its tail is
/// appended so an auth/RBAC failure that produced no log lines is still
/// visible to the client.
fn child_stream(
    child: Child,
    stdout: tokio::process::ChildStdout,
    stderr: Option<tokio::process::ChildStderr>,
) -> impl futures_util::Stream<Item = std::result::Result<Vec<u8>, std::io::Error>> {
    struct St {
        child: Child,
        stdout: tokio::process::ChildStdout,
        ring: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        drain: Option<tokio::task::JoinHandle<()>>,
        done: bool,
    }
    impl Drop for St {
        fn drop(&mut self) {
            if let Some(t) = self.drain.take() {
                t.abort();
            }
        }
    }
    let (ring, drain) = drain_stderr(stderr);
    stream::unfold(
        St {
            child,
            stdout,
            ring,
            drain,
            done: false,
        },
        |mut st| async move {
            if st.done {
                return None;
            }
            let mut buf = vec![0u8; 16 * 1024];
            match st.stdout.read(&mut buf).await {
                Ok(0) => {
                    st.done = true;
                    let mut tail = Vec::new();
                    // Let the drainer see stderr's EOF (the child is exiting).
                    if let Some(t) = st.drain.take() {
                        let _ = tokio::time::timeout(Duration::from_secs(2), t).await;
                    }
                    let raw =
                        std::mem::take(&mut *st.ring.lock().unwrap_or_else(|p| p.into_inner()));
                    let s = cli::redact(String::from_utf8_lossy(&raw).trim());
                    if !s.is_empty() {
                        tail.extend_from_slice(format!("\n[kubectl] {s}\n").as_bytes());
                    }
                    let _ = st.child.start_kill();
                    if tail.is_empty() {
                        None
                    } else {
                        Some((Ok(tail), st))
                    }
                }
                Ok(n) => {
                    buf.truncate(n);
                    Some((Ok(buf), st))
                }
                Err(e) => {
                    st.done = true;
                    let _ = st.child.start_kill();
                    Some((Err(e), st))
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[test]
    fn selector_argv_fans_out_with_prefix() {
        let q = LogsQuery {
            tail: Some(200),
            follow: Some(true),
            ..Default::default()
        };
        assert_eq!(
            target_args("shop", LogTarget::Selector("app=web,tier=fe"), &q),
            vec![
                "logs",
                "--selector=app=web,tier=fe",
                "--prefix",
                "--max-log-requests=100",
                "--all-containers",
                "--namespace=shop",
                "--tail=200",
                "-f"
            ]
        );
        // A named container replaces --all-containers.
        let c = LogsQuery {
            container: Some("nginx".into()),
            ..Default::default()
        };
        let a = target_args("shop", LogTarget::Selector("app=web"), &c);
        assert!(!a.iter().any(|x| x == "--all-containers"));
        assert!(a.iter().any(|x| x == "--container=nginx"));
    }

    #[test]
    fn logs_argv_from_query() {
        let q = LogsQuery {
            container: Some("web".into()),
            tail: Some(100),
            since: Some("10m".into()),
            previous: Some(true),
            follow: Some(true),
            timestamps: Some(true),
        };
        assert_eq!(
            logs_args("shop", "web-1", &q),
            vec![
                "logs",
                "--namespace=shop",
                "--container=web",
                "--tail=100",
                "--since=10m",
                "--previous",
                "--timestamps",
                "-f",
                "--",
                "web-1"
            ]
        );
        let d = LogsQuery::default();
        assert_eq!(
            logs_args("shop", "web-1", &d),
            vec![
                "logs",
                "--namespace=shop",
                "--tail=500",
                "--limit-bytes=5242880",
                "--",
                "web-1"
            ]
        );
        let t = LogsQuery {
            since: Some("2026-09-01T10:00:00Z".into()),
            tail: Some(-1),
            ..Default::default()
        };
        assert_eq!(
            logs_args("shop", "p", &t),
            vec![
                "logs",
                "--namespace=shop",
                "--since-time=2026-09-01T10:00:00Z",
                "--limit-bytes=5242880",
                "--",
                "p"
            ]
        );
    }

    #[test]
    fn flag_shaped_names_are_rejected() {
        let q = LogsQuery::default();
        for bad in [
            "--context=x",
            "-A",
            "--server=https://evil.example",
            "a b",
            "",
        ] {
            assert!(
                validate_target("shop", &LogTarget::Pod(bad), &q).is_err(),
                "pod {bad:?} must be rejected"
            );
            assert!(
                validate_target(bad, &LogTarget::Pod("web-1"), &q).is_err(),
                "ns {bad:?} must be rejected"
            );
            let c = LogsQuery {
                container: Some(bad.into()),
                ..Default::default()
            };
            if !bad.is_empty() {
                assert!(validate_target("shop", &LogTarget::Pod("web-1"), &c).is_err());
            }
        }
        assert!(validate_target("shop", &LogTarget::Pod("web-1"), &q).is_ok());
        assert!(validate_target("shop", &LogTarget::Selector("app=web"), &q).is_ok());
        // The pod is always behind `--`; a selector is one `--selector=` token.
        let a = target_args("shop", LogTarget::Selector("--context=x"), &q);
        assert!(a.iter().all(|x| x != "--context=x"));
    }

    #[test]
    fn tail_is_clamped() {
        let q = LogsQuery {
            tail: Some(i64::MAX),
            ..Default::default()
        };
        let a = logs_args("shop", "p", &q);
        assert!(a.iter().any(|x| x == &format!("--tail={MAX_TAIL}")));
    }

    #[tokio::test]
    async fn read_capped_keeps_only_the_tail_in_a_ring() {
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "i=0; while [ $i -lt 20000 ]; do echo \"line $i\"; i=$((i+1)); done; echo bad >&2",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        let child = cmd.spawn().unwrap();
        let (status, text, stderr) = read_capped(child, 1000, Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(status, 0);
        assert!(text.starts_with("[otto: output truncated to the last 1000 bytes]\n"));
        assert!(text.ends_with("line 19999\n"));
        assert!(text.len() < 1100);
        assert!(stderr.contains("bad"));
    }

    #[tokio::test]
    async fn read_capped_kills_the_child_at_the_deadline() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo start; while true; do sleep 0.1; done"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = cmd.spawn().unwrap();
        let r = read_capped(child, 1000, Duration::from_millis(300)).await;
        assert!(matches!(r, Err(ReadCapped::Timeout)));
    }

    #[test]
    fn cap_keeps_the_tail() {
        let text: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let capped = cap_tail(text.clone(), 100);
        assert!(capped.starts_with("[otto: output truncated"));
        assert!(capped.ends_with("line 999\n"));
        assert!(capped.len() < 200);
        assert_eq!(cap_tail("short".into(), 100), "short");
    }

    #[tokio::test]
    async fn follow_stream_ends_when_child_exits_and_appends_stderr() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "printf 'a\\nb\\n'; echo oops >&2"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().unwrap();
        let out = child.stdout.take().unwrap();
        let err = child.stderr.take();
        let chunks: Vec<Vec<u8>> = child_stream(child, out, err)
            .map(|c| c.unwrap())
            .collect()
            .await;
        let all = String::from_utf8(chunks.concat()).unwrap();
        assert!(all.starts_with("a\nb\n"));
        assert!(all.contains("[kubectl] oops"));
    }

    #[tokio::test]
    async fn heavy_stderr_does_not_stall_stdout() {
        // 200 KiB of stderr before stdout: without a concurrent drain the
        // child blocks on the full stderr pipe and stdout never arrives.
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "i=0; while [ $i -lt 4000 ]; do echo 'warning: some noisy kubectl message here' >&2; i=$((i+1)); done; echo done",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        let mut child = cmd.spawn().unwrap();
        let out = child.stdout.take().unwrap();
        let err = child.stderr.take();
        let chunks = tokio::time::timeout(
            Duration::from_secs(20),
            child_stream(child, out, err)
                .map(|c| c.unwrap())
                .collect::<Vec<Vec<u8>>>(),
        )
        .await
        .expect("stream stalled on a full stderr pipe");
        let all = String::from_utf8(chunks.concat()).unwrap();
        assert!(all.starts_with("done\n"));
        let tail = all.split("[kubectl] ").nth(1).unwrap();
        assert!(tail.len() <= STDERR_RING + 2);
    }

    #[tokio::test]
    async fn dropping_the_stream_kills_the_child() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo start; while true; do sleep 0.1; done"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().unwrap();
        let pid = child.id().unwrap().to_string();
        let out = child.stdout.take().unwrap();
        let err = child.stderr.take();
        let mut s = Box::pin(child_stream(child, out, err));
        let first = s.next().await.unwrap().unwrap();
        assert_eq!(first, b"start\n");
        let alive = |pid: &str| {
            std::process::Command::new("kill")
                .args(["-0", pid])
                .status()
                .unwrap()
                .success()
        };
        assert!(alive(&pid));
        drop(s);
        // kill_on_drop ⇒ SIGKILL on drop; give the kernel a beat to reap.
        for _ in 0..20 {
            if !alive(&pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // A killed-but-not-yet-reaped zombie still answers kill -0; check its
        // state via ps instead: 'Z' (zombie) or gone both mean "not running".
        let ps = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid])
            .output()
            .unwrap();
        let stat = String::from_utf8_lossy(&ps.stdout).trim().to_string();
        assert!(
            stat.is_empty() || stat.starts_with('Z'),
            "child still running (stat={stat:?})"
        );
    }
}
