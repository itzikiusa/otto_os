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

/// `logs_args` for either target.
pub fn target_args(ns: &str, target: LogTarget<'_>, q: &LogsQuery) -> Vec<String> {
    let mut a: Vec<String> = vec!["logs".into()];
    let container = q
        .container
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    match target {
        LogTarget::Pod(pod) => a.push(pod.into()),
        LogTarget::Selector(sel) => {
            a.push("-l".into());
            a.push(sel.into());
            a.push("--prefix".into());
            a.push(format!("--max-log-requests={MAX_LOG_REQUESTS}"));
            if container.is_none() {
                a.push("--all-containers".into());
            }
        }
    }
    a.push("-n".into());
    a.push(ns.into());
    if let Some(c) = container {
        a.push("-c".into());
        a.push(c.into());
    }
    let tail = q.tail.unwrap_or(500);
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
    let mut q = q.clone();
    q.follow = Some(false);
    let argv = k.argv(target_args(ns, target, &q));
    let out = cli::run(&k.program, &argv, &k.env, LOGS_TIMEOUT, None).await?;
    Ok(cap_tail(out.stdout, LOGS_CAP))
}

/// Streaming logs: a `text/plain` body that stays open while `kubectl logs -f`
/// runs; dropping the body kills the child.
pub fn follow(k: &Kubectl, ns: &str, pod: &str, q: &LogsQuery) -> Result<Body> {
    follow_target(k, ns, LogTarget::Pod(pod), q)
}

/// Streaming logs for either target.
pub fn follow_target(k: &Kubectl, ns: &str, target: LogTarget<'_>, q: &LogsQuery) -> Result<Body> {
    let mut q = q.clone();
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

/// Chunked reader over the child's stdout; the `Child` travels inside the
/// state so it is dropped (⇒ killed) together with the stream. When stdout
/// closes, stderr is drained into the tail so an auth/RBAC failure that
/// produced no log lines is still visible to the client.
fn child_stream(
    child: Child,
    stdout: tokio::process::ChildStdout,
    stderr: Option<tokio::process::ChildStderr>,
) -> impl futures_util::Stream<Item = std::result::Result<Vec<u8>, std::io::Error>> {
    struct St {
        child: Child,
        stdout: tokio::process::ChildStdout,
        stderr: Option<tokio::process::ChildStderr>,
        done: bool,
    }
    stream::unfold(
        St {
            child,
            stdout,
            stderr,
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
                    if let Some(mut err) = st.stderr.take() {
                        let mut s = String::new();
                        let _ = tokio::time::timeout(
                            Duration::from_secs(2),
                            err.read_to_string(&mut s),
                        )
                        .await;
                        let s = cli::redact(s.trim());
                        if !s.is_empty() {
                            tail.extend_from_slice(format!("\n[kubectl] {s}\n").as_bytes());
                        }
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
                "-l",
                "app=web,tier=fe",
                "--prefix",
                "--max-log-requests=100",
                "--all-containers",
                "-n",
                "shop",
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
        assert!(a.windows(2).any(|w| w == ["-c", "nginx"]));
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
                "web-1",
                "-n",
                "shop",
                "-c",
                "web",
                "--tail=100",
                "--since=10m",
                "--previous",
                "--timestamps",
                "-f"
            ]
        );
        let d = LogsQuery::default();
        assert_eq!(
            logs_args("shop", "web-1", &d),
            vec!["logs", "web-1", "-n", "shop", "--tail=500"]
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
                "p",
                "-n",
                "shop",
                "--since-time=2026-09-01T10:00:00Z"
            ]
        );
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
