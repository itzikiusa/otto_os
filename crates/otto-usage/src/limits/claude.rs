//! Claude plan limits: an in-memory latest-snapshot store per Claude config
//! dir, fed by two sources that never read a login token:
//!   * PASSIVE — `rate_limit_event` frames a structured (stream-json) Claude
//!     session already prints ([`record_claude_rate_limit`]);
//!   * ACTIVE, opt-in per login — the login's own `claude` CLI answering the
//!     `get_usage` control request (`cli.rs`), which replaces every window.
//!
//! Shapes, from `@anthropic-ai/claude-agent-sdk` 0.3.322 `sdk.d.ts`:
//!   * `SDKRateLimitInfo` (the event's `rate_limit_info`): `{status, resetsAt?:
//!     epoch s, rateLimitType?: five_hour|seven_day|seven_day_opus|
//!     seven_day_sonnet|seven_day_overage_included|overage, utilization?}` —
//!     one window per event; `utilization` is a 0–1 fraction on the stream
//!     (T3 Code multiplies it by 100), so a value above 1 is read as a percent;
//!   * `SDKControlGetUsageResponse`: `{subscription_type, rate_limits_available,
//!     rate_limits: {five_hour, seven_day, seven_day_opus, seven_day_sonnet:
//!     {utilization 0–100, resets_at ISO}, model_scoped?: [{display_name,
//!     utilization, resets_at}]}}`.
//!
//! Only the newest reading per window is kept; nothing is persisted.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use super::{LimitStatus, WindowLimit, STALE_AFTER_SECS};

/// One window as last seen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaudeWindow {
    pub name: String,
    pub minutes: Option<u64>,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
    /// Display order.
    pub rank: u8,
}

/// Everything known about one Claude login.
#[derive(Debug, Clone, Default)]
pub(crate) struct ClaudeEntry {
    pub plan: Option<String>,
    pub windows: BTreeMap<String, ClaudeWindow>,
    /// The per-model bucket name `get_usage` listed (names the
    /// `seven_day_overage_included` stream events).
    pub scoped_name: Option<String>,
    /// The last active check: when it finished and, if it failed, why.
    pub checked_at: Option<DateTime<Utc>>,
    pub check_error: Option<String>,
}

fn store() -> &'static Mutex<HashMap<PathBuf, ClaudeEntry>> {
    static STORE: OnceLock<Mutex<HashMap<PathBuf, ClaudeEntry>>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

pub(crate) fn entry(dir: &Path) -> Option<ClaudeEntry> {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(dir)
        .cloned()
}

pub(crate) fn update(dir: &Path, f: impl FnOnce(&mut ClaudeEntry)) {
    let mut map = store().lock().unwrap_or_else(|e| e.into_inner());
    // Bounded by the logins on this Mac; a runaway caller cannot grow it.
    if map.len() >= 256 && !map.contains_key(dir) {
        return;
    }
    f(map.entry(dir.to_path_buf()).or_default());
}

/// The config dir a session ran with: its `CLAUDE_CONFIG_DIR`, else the
/// default `~/.claude` (the key the Usage page's default Claude row reads).
pub fn claude_config_dir(config_dir: Option<&Path>) -> PathBuf {
    match config_dir {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => otto_transcript::roots::home_dir().join(".claude"),
    }
}

/// PASSIVE capture: record one `rate_limit_event` (the whole stream-json
/// message, or just its `rate_limit_info`) seen at `observed_at` for the login
/// at `config_dir` (`None` = the default `~/.claude`). Returns whether a
/// window was recorded. Never blocks on IO; safe to call from a stdout pump.
pub fn record_claude_rate_limit(
    config_dir: Option<&Path>,
    event: &Value,
    observed_at: DateTime<Utc>,
) -> bool {
    let dir = claude_config_dir(config_dir);
    let scoped = entry(&dir).and_then(|e| e.scoped_name);
    let Some((id, window)) = parse_rate_limit_info(event, scoped.as_deref(), observed_at) else {
        return false;
    };
    update(&dir, |e| {
        e.windows.insert(id, window);
    });
    true
}

/// [`record_claude_rate_limit`] from a raw stdout line; a cheap substring
/// test skips every other frame without parsing it.
pub fn record_claude_line(
    config_dir: Option<&Path>,
    line: &str,
    observed_at: DateTime<Utc>,
) -> bool {
    if !line.contains("\"rate_limit_event\"") {
        return false;
    }
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|v| record_claude_rate_limit(config_dir, &v, observed_at))
}

/// `(window id, window)` from an event; `None` for an event without a usable
/// window (no type, no utilization, an `overage` / unnamed bucket).
pub(crate) fn parse_rate_limit_info(
    event: &Value,
    scoped_name: Option<&str>,
    observed_at: DateTime<Utc>,
) -> Option<(String, ClaudeWindow)> {
    let info = event.get("rate_limit_info").unwrap_or(event);
    let kind = info.get("rateLimitType").and_then(Value::as_str)?;
    let raw = info.get("utilization").and_then(Value::as_f64)?;
    if !raw.is_finite() || raw < 0.0 {
        return None;
    }
    let used = if raw <= 1.0 { raw * 100.0 } else { raw };
    let resets_at = info
        .get("resetsAt")
        .and_then(Value::as_i64)
        .filter(|s| *s > 0)
        .and_then(|s| Utc.timestamp_opt(s, 0).single());
    let (name, minutes, rank) = match kind {
        "seven_day_overage_included" => (format!("Weekly · {}", scoped_name?), Some(WEEK), 4),
        k => fixed_window(k)?,
    };
    let id = if kind == "seven_day_overage_included" {
        scoped_id(scoped_name?)
    } else {
        kind.to_string()
    };
    Some((
        id,
        ClaudeWindow {
            name,
            minutes,
            used_percent: used.clamp(0.0, 100.0),
            resets_at,
            observed_at,
            rank,
        },
    ))
}

const WEEK: u64 = 7 * 24 * 60;

fn fixed_window(kind: &str) -> Option<(String, Option<u64>, u8)> {
    Some(match kind {
        "five_hour" => ("5-hour".into(), Some(300), 0),
        "seven_day" => ("Weekly".into(), Some(WEEK), 1),
        "seven_day_opus" => ("Weekly · Opus".into(), Some(WEEK), 2),
        "seven_day_sonnet" => ("Weekly · Sonnet".into(), Some(WEEK), 3),
        _ => return None,
    })
}

fn scoped_id(display_name: &str) -> String {
    let slug: String = display_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("seven_day_{slug}")
}

/// What a `get_usage` answer says.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum UsageAnswer {
    Windows {
        plan: Option<String>,
        windows: BTreeMap<String, ClaudeWindow>,
        scoped_name: Option<String>,
    },
    /// `rate_limits_available: false` — an API key / Bedrock / Vertex login.
    Unavailable,
}

/// Parse the `response` of a successful `get_usage` control response.
pub(crate) fn parse_get_usage(resp: &Value, observed_at: DateTime<Utc>) -> Option<UsageAnswer> {
    if resp.get("rate_limits_available").and_then(Value::as_bool) != Some(true) {
        return resp
            .get("rate_limits_available")
            .is_some()
            .then_some(UsageAnswer::Unavailable);
    }
    let rl = resp.get("rate_limits").filter(|r| r.is_object())?;
    let mut windows = BTreeMap::new();
    for kind in [
        "five_hour",
        "seven_day",
        "seven_day_opus",
        "seven_day_sonnet",
    ] {
        let Some(w) = rl.get(kind).filter(|w| w.is_object()) else {
            continue;
        };
        let Some(used) = w.get("utilization").and_then(Value::as_f64) else {
            continue;
        };
        let (name, minutes, rank) = fixed_window(kind)?;
        windows.insert(
            kind.to_string(),
            ClaudeWindow {
                name,
                minutes,
                used_percent: used.clamp(0.0, 100.0),
                resets_at: iso(w.get("resets_at")),
                observed_at,
                rank,
            },
        );
    }
    let mut scoped_name = None;
    for m in rl
        .get("model_scoped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(used)) = (
            m.get("display_name").and_then(Value::as_str),
            m.get("utilization").and_then(Value::as_f64),
        ) else {
            continue;
        };
        scoped_name.get_or_insert_with(|| name.to_string());
        windows.insert(
            scoped_id(name),
            ClaudeWindow {
                name: format!("Weekly · {name}"),
                minutes: Some(WEEK),
                used_percent: used.clamp(0.0, 100.0),
                resets_at: iso(m.get("resets_at")),
                observed_at,
                rank: 4,
            },
        );
    }
    Some(UsageAnswer::Windows {
        plan: resp
            .get("subscription_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        windows,
        scoped_name,
    })
}

fn iso(v: Option<&Value>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v?.as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// The served form of an entry: `(status, taken_at, windows)`. `taken_at` is
/// the OLDEST window's reading, so one old window makes the row stale rather
/// than borrowing a newer window's freshness. `None` with no windows.
pub(crate) fn served(
    e: &ClaudeEntry,
    now: DateTime<Utc>,
) -> Option<(LimitStatus, DateTime<Utc>, Vec<WindowLimit>)> {
    let taken_at = e.windows.values().map(|w| w.observed_at).min()?;
    let mut list: Vec<(&String, &ClaudeWindow)> = e.windows.iter().collect();
    list.sort_by_key(|(id, w)| (w.rank, (*id).clone()));
    let windows = list
        .into_iter()
        .map(|(id, w)| {
            let reset = w.resets_at.is_some_and(|r| r <= now);
            let used = (!reset).then_some(w.used_percent);
            WindowLimit {
                id: id.clone(),
                name: w.name.clone(),
                window_minutes: w.minutes,
                used_percent: used,
                left_percent: used.map(|u| (100.0 - u).clamp(0.0, 100.0)),
                resets_at: w.resets_at,
                reset_since_snapshot: reset,
            }
        })
        .collect();
    let status = if (now - taken_at).num_seconds() > STALE_AFTER_SECS {
        LimitStatus::Stale
    } else {
        LimitStatus::Ok
    };
    Some((status, taken_at, windows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn a_stream_event_records_its_one_window_as_a_percent() {
        let now = at("2026-11-12T10:00:00Z");
        let ev = json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","resetsAt":1794990000,"rateLimitType":"five_hour","utilization":0.82},"uuid":"u","session_id":"s"});
        let (id, w) = parse_rate_limit_info(&ev, None, now).unwrap();
        assert_eq!(id, "five_hour");
        assert_eq!(w.name, "5-hour");
        assert!((w.used_percent - 82.0).abs() < 1e-9);
        assert_eq!(w.resets_at.unwrap().timestamp(), 1794990000);
        // A bare `rate_limit_info` works too; a percent above 1 is kept.
        let info =
            json!({"status":"allowed","rateLimitType":"seven_day_sonnet","utilization":41.0});
        let (id, w) = parse_rate_limit_info(&info, None, now).unwrap();
        assert_eq!((id.as_str(), w.used_percent), ("seven_day_sonnet", 41.0));
        assert_eq!(w.resets_at, None);
    }

    #[test]
    fn events_without_a_usable_window_are_ignored() {
        let now = at("2026-11-12T10:00:00Z");
        for ev in [
            json!({"rate_limit_info":{"status":"allowed","rateLimitType":"five_hour"}}),
            json!({"rate_limit_info":{"status":"allowed","utilization":0.5}}),
            json!({"rate_limit_info":{"status":"rejected","rateLimitType":"overage","utilization":1.0}}),
            json!({"rate_limit_info":{"status":"allowed","rateLimitType":"seven_day_overage_included","utilization":0.2}}),
        ] {
            assert!(parse_rate_limit_info(&ev, None, now).is_none(), "{ev}");
        }
        // Named once get_usage listed the bucket.
        let ev = json!({"rate_limit_info":{"status":"allowed","rateLimitType":"seven_day_overage_included","utilization":0.2}});
        let (id, w) = parse_rate_limit_info(&ev, Some("Fable"), now).unwrap();
        assert_eq!(
            (id.as_str(), w.name.as_str()),
            ("seven_day_fable", "Weekly · Fable")
        );
    }

    #[test]
    fn record_merges_per_window_and_keys_by_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("acc");
        let t1 = at("2026-11-12T10:00:00Z");
        let five = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1794990000,"rateLimitType":"five_hour","utilization":0.1}}"#;
        assert!(record_claude_line(Some(&cfg), five, t1));
        let week = json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"seven_day","utilization":0.3}});
        assert!(record_claude_rate_limit(
            Some(&cfg),
            &week,
            t1 + chrono::Duration::minutes(5)
        ));
        assert!(!record_claude_line(
            Some(&cfg),
            r#"{"type":"assistant"}"#,
            t1
        ));
        let e = entry(&cfg).unwrap();
        assert_eq!(e.windows.len(), 2);
        let (status, taken, windows) = served(&e, t1 + chrono::Duration::minutes(10)).unwrap();
        assert_eq!(status, LimitStatus::Ok);
        assert_eq!(taken, t1, "the oldest window's time");
        assert_eq!(windows[0].name, "5-hour");
        assert_eq!(windows[0].left_percent, Some(90.0));
        // An hour later the oldest reading is stale.
        let (status, ..) = served(&e, t1 + chrono::Duration::hours(1)).unwrap();
        assert_eq!(status, LimitStatus::Stale);
        assert!(entry(&dir.path().join("other")).is_none());
    }

    #[test]
    fn get_usage_answers_parse_every_documented_window() {
        let now = at("2026-11-12T10:00:00Z");
        let resp = json!({
            "session": {"total_cost_usd": 0},
            "subscription_type": "max",
            "rate_limits_available": true,
            "rate_limits": {
                "five_hour": {"utilization": 23.5, "resets_at": "2026-11-12T13:00:00Z"},
                "seven_day": {"utilization": 41.2, "resets_at": "2026-11-15T00:00:00Z"},
                "seven_day_opus": null,
                "seven_day_sonnet": {"utilization": null, "resets_at": null},
                "model_scoped": [{"display_name": "Fable", "utilization": 62.8, "resets_at": "2026-11-16T00:00:00Z"}],
                "extra_usage": {"is_enabled": false, "monthly_limit": null, "used_credits": null}
            }
        });
        let Some(UsageAnswer::Windows {
            plan,
            windows,
            scoped_name,
        }) = parse_get_usage(&resp, now)
        else {
            panic!("expected windows");
        };
        assert_eq!(plan.as_deref(), Some("max"));
        assert_eq!(scoped_name.as_deref(), Some("Fable"));
        let ids: Vec<_> = windows.keys().cloned().collect();
        assert_eq!(ids, ["five_hour", "seven_day", "seven_day_fable"]);
        assert_eq!(windows["five_hour"].used_percent, 23.5);
        assert_eq!(
            windows["seven_day"].resets_at,
            Some(at("2026-11-15T00:00:00Z"))
        );
        let unavailable =
            json!({"subscription_type": null, "rate_limits_available": false, "rate_limits": null});
        assert_eq!(
            parse_get_usage(&unavailable, now),
            Some(UsageAnswer::Unavailable)
        );
        assert_eq!(parse_get_usage(&json!({}), now), None);
    }

    #[test]
    fn a_passed_reset_withdraws_the_percent() {
        let t = at("2026-11-12T10:00:00Z");
        let mut e = ClaudeEntry::default();
        e.windows.insert(
            "five_hour".into(),
            ClaudeWindow {
                name: "5-hour".into(),
                minutes: Some(300),
                used_percent: 50.0,
                resets_at: Some(t + chrono::Duration::minutes(5)),
                observed_at: t,
                rank: 0,
            },
        );
        let (_, _, w) = served(&e, t + chrono::Duration::minutes(10)).unwrap();
        assert!(w[0].reset_since_snapshot);
        assert_eq!(w[0].used_percent, None);
        assert!(served(&ClaudeEntry::default(), t).is_none());
    }
}
