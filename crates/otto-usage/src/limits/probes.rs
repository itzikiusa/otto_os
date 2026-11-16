//! Antigravity and Grok plan limits, asked of each CLI itself — neither writes
//! them to a file, and Otto never reads a login token (T3 Code reads the
//! token files and calls the providers' endpoints; Otto does not):
//!   * `agy -p /usage --output-format json` — agy answers its `/usage` command
//!     in print mode without starting a turn or spending quota
//!     (`command.data.groups[].buckets[]`: `window` `5h`/`weekly`,
//!     `remaining_fraction`, `reset_time`);
//!   * `grok agent stdio` — its ACP server answers the `_x.ai/billing`
//!     extension request after `initialize`, no session or prompt
//!     (`config.currentPeriod {type, start, end}`, `config.creditUsagePercent`
//!     — absent while nothing is used — and `subscription_tier`);
//!   * `muse usage --json` — Muse Code's subscription allowance, no model
//!     turn: the plan `tier`, the rolling `window` and the `weekly` limit,
//!     each with `usedPercent` and `resetsAtMs` (T3 Code reads the same
//!     report over Muse's session protocol).
//!
//! Machine logins only (root's rows). Answers are cached for [`PROBE_TTL`];
//! an expired one is served while a background probe refreshes it, and one
//! probe per CLI runs at a time.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::{unknown_row, AccountLimits, LimitStatus, WindowLimit, STALE_AFTER_SECS};

/// How long an answer (or failure) is reused.
pub const PROBE_TTL: Duration = Duration::from_secs(5 * 60);
/// The hard limit on one CLI exchange.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// Maximum inline wait on a cold probe before returning immediately while the
/// spawned task finishes in the background.
const COLD_INLINE_BUDGET: Duration = Duration::from_millis(200);

/// One CLI's newest answer: its raw JSON, or why there is none.
#[derive(Debug, Clone)]
pub(crate) struct Reading {
    pub taken_at: DateTime<Utc>,
    pub answer: Result<Value, String>,
}

pub(crate) type Readings = HashMap<&'static str, Reading>;

fn cache() -> &'static Mutex<Readings> {
    static CACHE: OnceLock<Mutex<Readings>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The CLIs whose limits come from asking the CLI itself.
#[derive(Debug, Clone, Copy)]
enum ProbedCli {
    Agy,
    Grok,
    Muse,
}

impl ProbedCli {
    const ALL: [Self; 3] = [Self::Agy, Self::Grok, Self::Muse];

    fn name(self) -> &'static str {
        match self {
            Self::Agy => "agy",
            Self::Grok => "grok",
            Self::Muse => "muse",
        }
    }

    /// One probe at a time per CLI.
    fn flight(self) -> &'static tokio::sync::Mutex<()> {
        static AGY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        static GROK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        static MUSE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        match self {
            Self::Agy => &AGY,
            Self::Grok => &GROK,
            Self::Muse => &MUSE,
        }
    }
}

/// The cached answers (what the rows are built from).
pub(crate) fn readings() -> Readings {
    cache().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Make sure each installed CLI (`program(name)` is `Some`) has an answer: a
/// cold probe waits briefly (`COLD_INLINE_BUDGET`) and otherwise finishes in
/// the background; an expired one refreshes in the background immediately.
pub async fn refresh(program: impl Fn(&str) -> Option<String>) {
    let now = Utc::now();
    let mut handles = Vec::new();
    for cli in ProbedCli::ALL {
        let Some(program) = program(cli.name()) else {
            continue;
        };
        let held = readings().get(cli.name()).map(|r| r.taken_at);
        match held {
            Some(at) if (now - at).to_std().is_ok_and(|age| age < PROBE_TTL) => {}
            Some(_) => {
                if cli.flight().try_lock().is_ok() {
                    tokio::spawn(probe(cli, program));
                }
            }
            None => {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    probe(cli, program).await;
                    let _ = tx.send(());
                });
                handles.push(rx);
            }
        }
    }
    if !handles.is_empty() {
        let _ =
            tokio::time::timeout(COLD_INLINE_BUDGET, futures_util::future::join_all(handles)).await;
    }
}

async fn probe(cli: ProbedCli, program: String) {
    let _held = cli.flight().lock().await;
    let fresh = readings().get(cli.name()).is_some_and(|r| {
        (Utc::now() - r.taken_at)
            .to_std()
            .is_ok_and(|a| a < PROBE_TTL)
    });
    if fresh {
        return;
    }
    let answer = match cli {
        ProbedCli::Agy => query_agy(&program, PROBE_TIMEOUT).await,
        ProbedCli::Grok => query_grok(&program, PROBE_TIMEOUT).await,
        ProbedCli::Muse => query_muse(&program, PROBE_TIMEOUT).await,
    };
    cache().lock().unwrap_or_else(|e| e.into_inner()).insert(
        cli.name(),
        Reading {
            taken_at: Utc::now(),
            answer,
        },
    );
}

fn short(s: &str) -> String {
    let s = s.trim();
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or(s);
    line.chars().take(200).collect()
}

/// `agy -p /usage --output-format json` → `command.data`.
pub(crate) async fn query_agy(program: &str, timeout: Duration) -> Result<Value, String> {
    let mut cmd =
        super::cli::build_cli_command(program, &["-p", "/usage", "--output-format", "json"], true);
    cmd.current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = match tokio::time::timeout(timeout, cmd.output()).await {
        Err(_) => return Err(format!("agy didn’t answer within {} s.", timeout.as_secs())),
        Ok(Err(e)) => return Err(format!("Couldn’t start agy ({program}): {e}")),
        Ok(Ok(out)) => out,
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).map_err(|_| {
        let why = short(&String::from_utf8_lossy(&out.stderr));
        let why = if why.is_empty() { short(&stdout) } else { why };
        format!("agy gave no usage answer: {why}")
    })?;
    match v.pointer("/command/data") {
        Some(data) if v["status"] == "SUCCESS" => Ok(data.clone()),
        _ => Err(format!(
            "agy gave no usage answer: {}",
            short(v["response"].as_str().unwrap_or("unknown error"))
        )),
    }
}

/// `muse usage --json` → its report (`allowance`, or the report itself).
/// Signed out, Muse prints why ("not logged in: run `muse login` first")
/// instead of JSON.
pub(crate) async fn query_muse(program: &str, timeout: Duration) -> Result<Value, String> {
    let mut cmd = super::cli::build_cli_command(program, &["usage", "--json"], true);
    cmd.current_dir(std::env::temp_dir())
        .envs([otto_core::agent_runtime::MUSE_NO_AUTO_UPDATE])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = match tokio::time::timeout(timeout, cmd.output()).await {
        Err(_) => {
            return Err(format!(
                "muse didn’t answer within {} s.",
                timeout.as_secs()
            ))
        }
        Ok(Err(e)) => return Err(format!("Couldn’t start muse ({program}): {e}")),
        Ok(Ok(out)) => out,
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    match serde_json::from_str::<Value>(stdout.trim()) {
        Ok(v) if v.is_object() => Ok(v
            .get("allowance")
            .filter(|a| a.is_object())
            .unwrap_or(&v)
            .clone()),
        _ => {
            let why = short(&String::from_utf8_lossy(&out.stderr));
            let why = if why.is_empty() { short(&stdout) } else { why };
            let why = why.trim_start_matches("muse usage:").trim();
            Err(format!("muse gave no usage answer: {why}"))
        }
    }
}

/// `grok agent stdio`: `initialize`, then `_x.ai/billing` → its result.
pub(crate) async fn query_grok(program: &str, timeout: Duration) -> Result<Value, String> {
    let mut cmd = super::cli::build_cli_command(program, &["agent", "stdio"], true);
    let mut child = cmd
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Couldn’t start grok ({program}): {e}"))?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.start_kill();
        return Err("Couldn’t talk to grok.".into());
    };
    let exchange = async {
        let mut lines = BufReader::new(stdout).lines();
        let init = json!({"protocolVersion": 1, "clientCapabilities": {}});
        rpc(&mut stdin, &mut lines, 1, "initialize", init).await?;
        rpc(&mut stdin, &mut lines, 2, "_x.ai/billing", json!({})).await
    };
    let out = tokio::time::timeout(timeout, exchange).await;
    let _ = child.start_kill();
    let _ = child.wait().await;
    out.unwrap_or_else(|_| {
        Err(format!(
            "grok didn’t answer within {} s.",
            timeout.as_secs()
        ))
    })
}

async fn rpc(
    stdin: &mut tokio::process::ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let gone = |_| "grok exited before answering.".to_string();
    stdin
        .write_all(format!("{req}\n").as_bytes())
        .await
        .map_err(gone)?;
    stdin.flush().await.map_err(gone)?;
    // Notifications (MCP server lists, …) arrive in between: skip them.
    for _ in 0..10_000 {
        let Ok(Some(line)) = lines.next_line().await else {
            return Err("grok exited before answering (is it signed in?).".into());
        };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v["id"] != id {
            continue;
        }
        if let Some(err) = v.get("error") {
            return Err(format!(
                "grok refused {method}: {}",
                short(err["message"].as_str().unwrap_or("unknown error"))
            ));
        }
        return Ok(v["result"].clone());
    }
    Err(format!("grok never answered {method}."))
}

fn at(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// A window from what is LEFT (`left` 0–100) as of `now`.
fn window(
    id: String,
    name: String,
    minutes: Option<u64>,
    left: f64,
    resets_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> WindowLimit {
    let reset = resets_at.is_some_and(|r| r <= now);
    let left = (!reset).then_some(left.clamp(0.0, 100.0));
    WindowLimit {
        id,
        name,
        window_minutes: minutes,
        used_percent: left.map(|l| 100.0 - l),
        left_percent: left,
        resets_at,
        reset_since_snapshot: reset,
        active: true,
        severity: None,
    }
}

fn row(
    provider: &str,
    label: &str,
    r: &Reading,
    plan: Option<String>,
    windows: Vec<WindowLimit>,
    now: DateTime<Utc>,
) -> AccountLimits {
    let limit_reached = windows
        .iter()
        .find(|w| w.left_percent == Some(0.0))
        .map(|w| w.name.clone());
    AccountLimits {
        key: format!("{provider}:default"),
        provider: provider.into(),
        account_id: None,
        label: label.into(),
        source: "default".into(),
        status: if (now - r.taken_at).num_seconds() > STALE_AFTER_SECS {
            LimitStatus::Stale
        } else {
            LimitStatus::Ok
        },
        reason: None,
        plan,
        taken_at: Some(r.taken_at),
        windows,
        credits: None,
        limit_reached,
        check: None,
    }
}

fn unknown(provider: &str, label: &str, reason: &str) -> AccountLimits {
    unknown_row(
        format!("{provider}:default"),
        provider,
        None,
        label.into(),
        "default",
        reason,
    )
}

pub(crate) const AGY_LABEL: &str = "Default Antigravity login";
const GROK_LABEL: &str = "Default Grok login";
const MUSE_LABEL: &str = "Default Muse login";

/// The Antigravity row: one window per quota bucket (`Gemini Models · 5-hour`).
pub(crate) fn agy_row(r: &Reading, now: DateTime<Utc>) -> AccountLimits {
    let data = match &r.answer {
        Ok(d) => d,
        Err(why) => return unknown("agy", AGY_LABEL, why),
    };
    let mut windows = Vec::new();
    for group in data["groups"].as_array().into_iter().flatten() {
        let gname = group["name"].as_str().unwrap_or("Quota");
        for b in group["buckets"].as_array().into_iter().flatten() {
            let Some(frac) = b["remaining_fraction"].as_f64() else {
                continue;
            };
            let (wname, minutes) = match b["window"].as_str() {
                Some("5h") => ("5-hour".to_string(), Some(300)),
                Some("weekly") => ("Weekly".to_string(), Some(7 * 24 * 60)),
                _ => (b["name"].as_str().unwrap_or("Quota").to_string(), None),
            };
            windows.push(window(
                b["id"].as_str().unwrap_or(gname).to_string(),
                format!("{gname} · {wname}"),
                minutes,
                frac * 100.0,
                at(&b["reset_time"]),
                now,
            ));
        }
    }
    if windows.is_empty() {
        return unknown("agy", AGY_LABEL, "agy reports no quota for this login.");
    }
    row("agy", AGY_LABEL, r, None, windows, now)
}

/// The Grok row: the subscription period's credit use.
pub(crate) fn grok_row(r: &Reading, now: DateTime<Utc>) -> AccountLimits {
    let data = match &r.answer {
        Ok(d) => d,
        Err(why) => return unknown("grok", GROK_LABEL, why),
    };
    let plan = data["subscription_tier"].as_str().map(str::to_string);
    let period = &data["config"]["currentPeriod"];
    let kind = period["type"].as_str().unwrap_or("");
    let (start, end) = (at(&period["start"]), at(&period["end"]));
    let mut used = data["config"]["creditUsagePercent"].as_f64();
    // Grok leaves the percent out while nothing of the period is used.
    if used.is_none() && start.is_some() && end.is_some() {
        used = Some(0.0);
    }
    let Some(used) = used else {
        let mut row = unknown(
            "grok",
            GROK_LABEL,
            "grok reports no plan usage for this login (an API key, or no subscription).",
        );
        row.plan = plan;
        return row;
    };
    let name = match kind {
        "USAGE_PERIOD_TYPE_WEEKLY" => "Weekly",
        "USAGE_PERIOD_TYPE_MONTHLY" => "Monthly",
        _ => "Subscription",
    };
    let minutes = start
        .zip(end)
        .and_then(|(s, e)| u64::try_from((e - s).num_minutes()).ok())
        .filter(|m| *m > 0);
    let w = window(
        "subscription".into(),
        name.into(),
        minutes,
        100.0 - used,
        end,
        now,
    );
    row("grok", GROK_LABEL, r, plan, vec![w], now)
}

/// The Muse row: the rolling session window and the weekly limit, the tier
/// as the plan. Field names are read camelCase or snake_case, resets as
/// epoch milliseconds or RFC 3339.
pub(crate) fn muse_row(r: &Reading, now: DateTime<Utc>) -> AccountLimits {
    let data = match &r.answer {
        Ok(d) => d,
        Err(why) => return unknown("muse", MUSE_LABEL, why),
    };
    let num = |v: &Value, a: &str, b: &str| v.get(a).or_else(|| v.get(b)).and_then(Value::as_f64);
    let plan = data["tier"]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    let mut windows = Vec::new();
    for (key, name, default_minutes) in [
        ("window", "Session", None),
        ("weekly", "Weekly", Some(7 * 24 * 60)),
    ] {
        let w = &data[key];
        let Some(used) = num(w, "usedPercent", "used_percent") else {
            continue;
        };
        let minutes = num(w, "windowDurationMins", "window_duration_mins")
            .filter(|m| *m > 0.0)
            .map(|m| m as u64)
            .or(default_minutes);
        let resets = num(w, "resetsAtMs", "resets_at_ms")
            .and_then(|ms| DateTime::from_timestamp_millis(ms as i64))
            .or_else(|| at(&w["resetsAt"]))
            .or_else(|| at(&w["resets_at"]));
        windows.push(window(
            key.into(),
            name.into(),
            minutes,
            100.0 - used,
            resets,
            now,
        ));
    }
    if windows.is_empty() {
        let mut row = unknown(
            "muse",
            MUSE_LABEL,
            "muse reports no plan usage for this login (an API key, or nothing used this window yet).",
        );
        row.plan = plan;
        return row;
    }
    row("muse", MUSE_LABEL, r, plan, windows, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn reading(answer: Result<Value, String>) -> Reading {
        Reading {
            taken_at: t("2026-11-13T13:00:00Z"),
            answer,
        }
    }

    /// Trimmed from a real `agy -p /usage --output-format json` (agy 1.5.8).
    const AGY: &str = r#"{"conversation_id":"","status":"SUCCESS","response":"…","num_turns":0,"command":{"name":"usage","data":{"groups":[{"name":"Gemini Models","buckets":[{"id":"gemini-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":0.88,"reset_time":"2026-11-14T00:27:32Z"},{"id":"gemini-5h","name":"Five Hour Limit Remaining","window":"5h","remaining_fraction":0,"reset_time":"2026-11-13T15:24:17Z"}]},{"name":"Claude and GPT models","buckets":[{"id":"3p-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":1,"reset_time":"2026-11-13T12:00:00Z"}]}]}}}"#;

    #[test]
    fn agy_buckets_become_windows_per_group() {
        let data = serde_json::from_str::<Value>(AGY).unwrap()["command"]["data"].clone();
        let row = agy_row(&reading(Ok(data)), t("2026-11-13T13:05:00Z"));
        assert_eq!(row.key, "agy:default");
        assert_eq!(row.status, LimitStatus::Ok);
        let names: Vec<_> = row.windows.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Gemini Models · Weekly",
                "Gemini Models · 5-hour",
                "Claude and GPT models · Weekly"
            ]
        );
        let weekly = &row.windows[0];
        assert_eq!(weekly.left_percent, Some(88.0));
        assert_eq!(weekly.window_minutes, Some(10080));
        assert_eq!(weekly.resets_at, Some(t("2026-11-14T00:27:32Z")));
        // An empty bucket is the reached limit; a passed reset drops the percent.
        assert_eq!(row.limit_reached.as_deref(), Some("Gemini Models · 5-hour"));
        assert!(row.windows[2].reset_since_snapshot);
        assert_eq!(row.windows[2].left_percent, None);
    }

    #[test]
    fn grok_period_is_one_window_and_a_missing_percent_is_zero_used() {
        // A real `_x.ai/billing` result (grok 1.0.77) with nothing used yet.
        let data = json!({"config":{"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","start":"2026-11-08T05:59:43.931+00:00","end":"2026-11-15T05:59:43.931+00:00"},"prepaidBalance":{"val":0},"isUnifiedBillingUser":true},"subscription_tier":"SuperGrok Heavy"});
        let row = grok_row(&reading(Ok(data)), t("2026-11-13T13:05:00Z"));
        assert_eq!(row.key, "grok:default");
        assert_eq!(row.plan.as_deref(), Some("SuperGrok Heavy"));
        let w = &row.windows[0];
        assert_eq!((w.name.as_str(), w.window_minutes), ("Weekly", Some(10080)));
        assert_eq!((w.used_percent, w.left_percent), (Some(0.0), Some(100.0)));
        let used = json!({"config":{"creditUsagePercent":37.5,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_MONTHLY","end":"2026-12-01T00:00:00Z"}}});
        let w = &grok_row(&reading(Ok(used)), t("2026-11-13T13:05:00Z")).windows[0];
        assert_eq!((w.name.as_str(), w.left_percent), ("Monthly", Some(62.5)));
    }

    #[test]
    fn muse_allowance_is_a_session_and_a_weekly_window() {
        // The shape T3 Code decodes (`MuseSubscriptionUsage`).
        let data = json!({"observedAtMs":1_794_571_200_000_i64,"tier":"Muse Pro","window":{"usedPercent":40.0,"windowDurationMins":300,"resetsAtMs":1_794_582_000_000_i64},"weekly":{"usedPercent":100.0,"resetsAtMs":1_795_000_000_000_i64}});
        let row = muse_row(&reading(Ok(data)), t("2026-11-13T13:05:00Z"));
        assert_eq!(row.key, "muse:default");
        assert_eq!(row.plan.as_deref(), Some("Muse Pro"));
        let names: Vec<_> = row.windows.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Session", "Weekly"]);
        assert_eq!(row.windows[0].left_percent, Some(60.0));
        assert_eq!(row.windows[0].window_minutes, Some(300));
        assert_eq!(row.windows[1].window_minutes, Some(10080));
        assert_eq!(row.limit_reached.as_deref(), Some("Weekly"));
        // snake_case with an RFC 3339 reset reads the same.
        let snake = json!({"window":{"used_percent":10,"resets_at":"2026-11-13T18:00:00Z"}});
        let w = &muse_row(&reading(Ok(snake)), t("2026-11-13T13:05:00Z")).windows[0];
        assert_eq!(
            (w.left_percent, w.resets_at),
            (Some(90.0), Some(t("2026-11-13T18:00:00Z")))
        );
        let none = muse_row(
            &reading(Ok(json!({"tier":"Free"}))),
            t("2026-11-13T13:05:00Z"),
        );
        assert_eq!(none.status, LimitStatus::Unknown);
        assert_eq!(none.plan.as_deref(), Some("Free"));
    }

    #[test]
    fn a_failed_or_empty_answer_is_unknown_with_why() {
        let now = t("2026-11-13T13:05:00Z");
        let row = grok_row(&reading(Err("grok exited before answering.".into())), now);
        assert_eq!(row.status, LimitStatus::Unknown);
        assert_eq!(row.reason.as_deref(), Some("grok exited before answering."));
        let row = grok_row(&reading(Ok(json!({"config":{}}))), now);
        assert!(row.reason.unwrap().contains("no plan usage"));
        let row = agy_row(&reading(Ok(json!({"groups":[]}))), now);
        assert_eq!(row.status, LimitStatus::Unknown);
    }

    fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.display().to_string()
    }

    #[tokio::test]
    async fn the_clis_are_asked_and_their_answers_read() {
        let dir = tempfile::tempdir().unwrap();
        let agy = script(
            dir.path(),
            "agy",
            &format!(
                "[ \"$1 $2 $3 $4\" = '-p /usage --output-format json' ] || exit 2\ncat <<'EOF'\n{AGY}\nEOF"
            ),
        );
        let data = query_agy(&agy, PROBE_TIMEOUT).await.unwrap();
        assert_eq!(data["groups"][0]["name"], "Gemini Models");

        // ACP: answer `initialize`, a notification in between, then the billing request.
        let grok = script(
            dir.path(),
            "grok",
            r#"[ "$1 $2" = 'agent stdio' ] || exit 2
while read -r l; do
  case "$l" in
    *'"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{}}'; echo '{"jsonrpc":"2.0","method":"_x.ai/mcp/servers_updated","params":{}}' ;;
    *'"_x.ai/billing"'*) echo '{"jsonrpc":"2.0","id":2,"result":{"subscription_tier":"SuperGrok"}}' ;;
  esac
done"#,
        );
        let data = query_grok(&grok, PROBE_TIMEOUT).await.unwrap();
        assert_eq!(data["subscription_tier"], "SuperGrok");

        let silent = script(dir.path(), "silent", "sleep 30");
        let err = query_grok(&silent, Duration::from_millis(300))
            .await
            .unwrap_err();
        assert!(err.contains("didn’t answer"), "{err}");
        let refused = script(
            dir.path(),
            "refused",
            "echo 'Error: not signed in' >&2; exit 1",
        );
        let err = query_agy(&refused, PROBE_TIMEOUT).await.unwrap_err();
        assert!(err.contains("not signed in"), "{err}");

        let muse = script(
            dir.path(),
            "muse",
            r#"[ "$1 $2 $MUSE_NO_AUTO_UPDATE" = 'usage --json 1' ] || exit 2
echo '{"schemaVersion":1,"allowance":{"tier":"Pro","window":{"usedPercent":5}}}'"#,
        );
        let data = query_muse(&muse, PROBE_TIMEOUT).await.unwrap();
        assert_eq!(data["tier"], "Pro");
        let signed_out = script(
            dir.path(),
            "muse-out",
            "echo 'muse usage: not logged in: run `muse login` first' >&2",
        );
        let err = query_muse(&signed_out, PROBE_TIMEOUT).await.unwrap_err();
        assert_eq!(
            err,
            "muse gave no usage answer: not logged in: run `muse login` first"
        );
    }
}
