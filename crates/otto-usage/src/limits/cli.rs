//! ACTIVE Claude check, opt-in per login: run that login's own `claude` CLI
//! and ask it for plan usage over its stream-json control protocol. Otto never
//! reads the login; the CLI calls Anthropic's usage endpoint itself.
//!
//! The exchange is the Agent SDK's, from `@anthropic-ai/claude-agent-sdk`
//! 0.3.322 `sdk.mjs` (`ProcessTransport.initialize` builds the argv; `Query.
//! request` frames a control request as `{request_id, type:"control_request",
//! request}` + `\n` and resolves on the `control_response` whose
//! `response.request_id` matches, rejecting when `response.subtype` is not
//! `success`; `usage_EXPERIMENTAL_…` sends `{subtype:"get_usage",
//! skip_behaviors:true}` and returns `response.response`):
//!   1. spawn `claude --output-format stream-json --verbose --input-format
//!      stream-json --tools= --setting-sources= --strict-mcp-config
//!      --no-session-persistence` (no tools, no settings / hooks, no MCP
//!      servers, no transcript) with the login's `CLAUDE_CONFIG_DIR`;
//!   2. write `initialize`, wait for its success;
//!   3. write `get_usage`, wait for its answer; no prompt is ever sent;
//!   4. kill the child. The whole exchange has a hard timeout.
//!
//! Answers are cached for [`CHECK_TTL`] and one check per login runs at a time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::claude::{self, UsageAnswer};

/// How long a check's answer (or failure) is reused.
pub const CHECK_TTL: Duration = Duration::from_secs(5 * 60);
/// The hard limit on one CLI exchange.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

const INIT_ID: &str = "otto-init";
const USAGE_ID: &str = "otto-usage";
/// Lines read before giving up on an answer (a CLI that floods stdout).
const MAX_LINES: usize = 10_000;

/// Run one `get_usage` exchange. `Err` carries a sentence the page shows.
pub(crate) async fn query_usage(
    program: &str,
    config_dir: &Path,
    default_login: bool,
    timeout: Duration,
) -> Result<Value, String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args([
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--tools=",
        "--setting-sources=",
        "--strict-mcp-config",
        "--no-session-persistence",
    ])
    .current_dir(std::env::temp_dir())
    // The subscription login, never an API key that happens to be exported.
    .env_remove("ANTHROPIC_API_KEY")
    .env_remove("ANTHROPIC_AUTH_TOKEN")
    .env_remove("CLAUDE_CODE_OAUTH_TOKEN")
    .env_remove("NODE_OPTIONS")
    .env("CLAUDE_CODE_ENTRYPOINT", "sdk-ts")
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .kill_on_drop(true);
    if default_login {
        cmd.env_remove("CLAUDE_CONFIG_DIR");
    } else {
        cmd.env("CLAUDE_CONFIG_DIR", config_dir);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Couldn’t start the Claude CLI ({program}): {e}"))?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.start_kill();
        return Err("Couldn’t talk to the Claude CLI.".into());
    };
    let exchange = async {
        let mut lines = BufReader::new(stdout).lines();
        send(
            &mut stdin,
            INIT_ID,
            json!({"subtype": "initialize", "hooks": null}),
        )
        .await?;
        await_response(&mut lines, INIT_ID).await?;
        send(
            &mut stdin,
            USAGE_ID,
            json!({"subtype": "get_usage", "skip_behaviors": true}),
        )
        .await?;
        await_response(&mut lines, USAGE_ID).await
    };
    let out = tokio::time::timeout(timeout, exchange).await;
    let _ = child.start_kill();
    let _ = child.wait().await;
    match out {
        Ok(r) => r,
        Err(_) => Err(format!(
            "The Claude CLI didn’t answer within {} s.",
            timeout.as_secs().max(1)
        )),
    }
}

async fn send(
    stdin: &mut tokio::process::ChildStdin,
    id: &str,
    request: Value,
) -> Result<(), String> {
    let line = json!({"request_id": id, "type": "control_request", "request": request});
    stdin
        .write_all(format!("{line}\n").as_bytes())
        .await
        .map_err(|_| "The Claude CLI exited before answering.".to_string())?;
    stdin
        .flush()
        .await
        .map_err(|_| "The Claude CLI exited before answering.".to_string())
}

async fn await_response(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: &str,
) -> Result<Value, String> {
    for _ in 0..MAX_LINES {
        let Ok(Some(line)) = lines.next_line().await else {
            return Err(
                "The Claude CLI exited before answering (is this login signed in?).".into(),
            );
        };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("control_response") {
            continue;
        }
        let Some(resp) = v.get("response") else {
            continue;
        };
        if resp.get("request_id").and_then(Value::as_str) != Some(id) {
            continue;
        }
        if resp.get("subtype").and_then(Value::as_str) == Some("success") {
            return Ok(resp.get("response").cloned().unwrap_or(Value::Null));
        }
        let why = resp
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        let why: String = why.chars().take(200).collect();
        return Err(format!("The Claude CLI refused the usage request: {why}"));
    }
    Err("The Claude CLI never answered the usage request.".into())
}

/// One lock per login: a second caller waits for the first check, then finds
/// its answer cached instead of starting another CLI.
fn flight(dir: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static FLIGHTS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let mut map = FLIGHTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if map.len() > 256 {
        map.retain(|_, m| Arc::strong_count(m) > 1);
    }
    map.entry(dir.to_path_buf()).or_default().clone()
}

/// Whether a check for `dir` is running now.
pub(crate) fn in_flight(dir: &Path) -> bool {
    flight(dir).try_lock().is_err()
}

/// Check `dir` unless an answer younger than [`CHECK_TTL`] exists; record the
/// answer (windows replace the stored ones) or the failure. `program`: the
/// `claude` binary (`None`: not installed → a failure, not a spawn).
pub(crate) async fn check(
    program: Option<&str>,
    dir: &Path,
    default_login: bool,
    timeout: Duration,
    now: impl Fn() -> DateTime<Utc>,
) {
    let lock = flight(dir);
    let _held = lock.lock().await;
    let fresh = claude::entry(dir)
        .and_then(|e| e.checked_at)
        .is_some_and(|at| (now() - at).to_std().is_ok_and(|age| age < CHECK_TTL));
    if fresh {
        return;
    }
    let result = match program {
        None => Err("The Claude CLI isn’t installed (Settings ▸ Providers).".to_string()),
        Some(p) => query_usage(p, dir, default_login, timeout).await,
    };
    let at = now();
    let answer = result.and_then(|resp| {
        claude::parse_get_usage(&resp, at)
            .ok_or_else(|| "The Claude CLI answered in a shape Otto doesn’t know.".to_string())
    });
    claude::update(dir, |e| {
        e.checked_at = Some(at);
        match answer {
            Ok(UsageAnswer::Windows {
                plan,
                windows,
                scoped_name,
            }) => {
                e.check_error = None;
                e.plan = plan.or(e.plan.take());
                e.scoped_name = scoped_name.or(e.scoped_name.take());
                e.windows = windows;
            }
            Ok(UsageAnswer::Unavailable) => {
                e.check_error = Some(
                    "This login reports no plan limits (an API key, Bedrock or Vertex login)."
                        .into(),
                );
            }
            Err(why) => e.check_error = Some(why),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake `claude`: logs its argv, CLAUDE_CONFIG_DIR and every stdin line
    /// to `<dir>/log`, counts runs in `<dir>/runs`, and answers the two
    /// control requests with `usage_json` (an SDK-shaped get_usage reply).
    fn fake_cli(dir: &Path, usage_json: &str) -> PathBuf {
        let p = dir.join("claude");
        let log = dir.join("log");
        let runs = dir.join("runs");
        let script = format!(
            r#"#!/bin/sh
echo run >> '{runs}'
echo "argv: $*" >> '{log}'
echo "config: $CLAUDE_CONFIG_DIR" >> '{log}'
echo "apikey: ${{ANTHROPIC_API_KEY:-none}}" >> '{log}'
echo 'not json, a warning'
read line; echo "in: $line" >> '{log}'
echo '{{"type":"system","subtype":"init","session_id":"s"}}'
echo '{{"type":"control_response","response":{{"subtype":"success","request_id":"otto-init","response":{{}}}}}}'
read line; echo "in: $line" >> '{log}'
echo '{{"type":"control_response","response":{{"subtype":"success","request_id":"other","response":{{}}}}}}'
echo '{{"type":"control_response","response":{{"subtype":"success","request_id":"otto-usage","response":{usage}}}}}'
read line
"#,
            runs = runs.display(),
            log = log.display(),
            usage = usage_json,
        );
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    const USAGE: &str = r#"{"subscription_type":"max","rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":23.5,"resets_at":"2099-01-01T00:00:00Z"},"seven_day":{"utilization":41.0,"resets_at":"2099-01-05T00:00:00Z"}}}"#;

    fn runs(dir: &Path) -> usize {
        std::fs::read_to_string(dir.join("runs"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn the_exchange_is_initialize_then_get_usage_with_the_login_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), USAGE);
        let cfg = dir.path().join("account-home");
        let resp = query_usage(cli.to_str().unwrap(), &cfg, false, CHECK_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(resp["subscription_type"], "max");
        let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
        assert!(log.contains("--input-format stream-json"), "{log}");
        assert!(
            log.contains("--tools= --setting-sources= --strict-mcp-config"),
            "{log}"
        );
        assert!(log.contains(&format!("config: {}", cfg.display())), "{log}");
        let ins: Vec<Value> = log
            .lines()
            .filter_map(|l| l.strip_prefix("in: "))
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(ins.len(), 2, "no prompt is ever sent");
        assert_eq!(ins[0]["type"], "control_request");
        assert_eq!(ins[0]["request_id"], "otto-init");
        assert_eq!(ins[0]["request"]["subtype"], "initialize");
        assert_eq!(ins[1]["request_id"], "otto-usage");
        assert_eq!(ins[1]["request"]["subtype"], "get_usage");
        assert_eq!(ins[1]["request"]["skip_behaviors"], true);
    }

    #[tokio::test]
    async fn an_error_response_is_a_reason_not_a_number() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("claude");
        std::fs::write(
            &p,
            "#!/bin/sh\nread l\necho '{\"type\":\"control_response\",\"response\":{\"subtype\":\"success\",\"request_id\":\"otto-init\",\"response\":{}}}'\nread l\necho '{\"type\":\"control_response\",\"response\":{\"subtype\":\"error\",\"request_id\":\"otto-usage\",\"error\":\"not logged in\"}}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = query_usage(p.to_str().unwrap(), dir.path(), false, CHECK_TIMEOUT)
            .await
            .expect_err("an error subtype");
        assert!(err.contains("not logged in"), "{err}");
    }

    #[tokio::test]
    async fn a_silent_cli_times_out_and_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("claude");
        std::fs::write(&p, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        let err = query_usage(
            p.to_str().unwrap(),
            dir.path(),
            false,
            Duration::from_millis(300),
        )
        .await
        .expect_err("timeout");
        assert!(err.contains("didn’t answer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_check_is_cached_and_failures_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), USAGE);
        let cfg = dir.path().join("cfg");
        let now = Utc::now();
        check(
            Some(cli.to_str().unwrap()),
            &cfg,
            false,
            CHECK_TIMEOUT,
            || now,
        )
        .await;
        check(
            Some(cli.to_str().unwrap()),
            &cfg,
            false,
            CHECK_TIMEOUT,
            || now,
        )
        .await;
        assert_eq!(runs(dir.path()), 1, "the second check reused the answer");
        let e = claude::entry(&cfg).unwrap();
        assert_eq!(e.plan.as_deref(), Some("max"));
        assert_eq!(e.windows["five_hour"].used_percent, 23.5);
        assert_eq!(e.check_error, None);
        // Past the TTL it runs again.
        let later = now + chrono::Duration::minutes(6);
        check(
            Some(cli.to_str().unwrap()),
            &cfg,
            false,
            CHECK_TIMEOUT,
            || later,
        )
        .await;
        assert_eq!(runs(dir.path()), 2);
        // No CLI: recorded as a failure, the old windows stay with their age.
        let missing = dir.path().join("missing");
        check(None, &missing, false, CHECK_TIMEOUT, || now).await;
        let e = claude::entry(&missing).unwrap();
        assert!(e.check_error.unwrap().contains("isn’t installed"));
        assert!(e.windows.is_empty());
    }

    #[tokio::test]
    async fn concurrent_checks_share_one_cli_run() {
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), USAGE);
        let cfg = dir.path().join("cfg");
        let program = cli.to_str().unwrap().to_string();
        let now = Utc::now();
        let a = check(Some(program.as_str()), &cfg, false, CHECK_TIMEOUT, || now);
        let b = check(Some(program.as_str()), &cfg, false, CHECK_TIMEOUT, || now);
        tokio::join!(a, b);
        assert_eq!(runs(dir.path()), 1);
    }
}
