//! Subscription limits the agent CLIs already write locally.
//!
//! Codex: every `event_msg` / `token_count` line of a rollout carries the
//! plan's rate-limit snapshot as of that turn, taken from the response headers:
//!
//! ```json
//! "rate_limits":{"limit_id":"codex","limit_name":null,
//!   "primary":{"used_percent":13.0,"window_minutes":10080,"resets_at":1794983830},
//!   "secondary":null,
//!   "credits":{"has_credits":true,"unlimited":false,"balance":"62500.0"},
//!   "plan_type":"pro","rate_limit_reached_type":null}
//! ```
//!
//! Both rollout eras (with and without `ordinal`) share that payload. Older
//! CLIs wrote `resets_in_seconds` (relative to the line) or an RFC 3339
//! `resets_at`; both are read. An API-key login writes `"rate_limits":null`,
//! which [`parse_codex_rate_limits`] reports as [`CodexLimitsLine::NoLimits`]
//! so the caller can say why there is nothing to show.
//!
//! Claude Code writes no plan limits to its transcripts (they reach only the
//! status-line command and `/usage`), so there is no Claude parser here.

use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use serde_json::Value;

/// One rate-limit window as the CLI reported it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LimitWindow {
    /// Position in the payload: `primary` / `secondary`.
    pub slot: String,
    /// Window length; `None` when the CLI did not say.
    pub window_minutes: Option<u64>,
    /// 0–100 (clamped).
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

/// Prepaid credits next to the plan windows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LimitCredits {
    pub has_credits: bool,
    pub unlimited: bool,
    /// The balance as written (a decimal string, e.g. `"62500.0"`).
    pub balance: Option<String>,
}

/// The plan limits one `token_count` line carried.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LimitSnapshot {
    /// `codex` for the main allowance; another id is a model-specific bucket.
    /// `None` on CLIs that predate the field (the main allowance).
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub plan: Option<String>,
    /// The line's own timestamp — when the CLI saw these numbers.
    pub taken_at: DateTime<Utc>,
    pub windows: Vec<LimitWindow>,
    pub credits: Option<LimitCredits>,
    /// Set while a window is exhausted (`rate_limit_reached_type`).
    pub reached: Option<String>,
}

impl LimitSnapshot {
    /// The main allowance (not a model-specific bucket).
    pub fn is_main(&self) -> bool {
        matches!(self.limit_id.as_deref(), None | Some("codex"))
    }
}

/// What one rollout line says about limits.
#[derive(Debug, Clone, PartialEq)]
pub enum CodexLimitsLine {
    Snapshot(LimitSnapshot),
    /// A `token_count` line whose `rate_limits` is null or absent: this login
    /// reports no plan limits (API key, or a CLI without the field).
    NoLimits,
}

/// Parse one Codex rollout line. `None` for any line that is not a
/// `token_count` event (most lines; a cheap substring test runs first).
pub fn parse_codex_rate_limits(line: &str) -> Option<CodexLimitsLine> {
    if !line.contains("\"token_count\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = v.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let taken_at = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc));
    let Some(rl) = payload.get("rate_limits").filter(|r| r.is_object()) else {
        return Some(CodexLimitsLine::NoLimits);
    };
    // A snapshot without a time cannot be aged, and an unaged percent must
    // never be shown as current.
    let taken_at = taken_at?;
    let windows: Vec<LimitWindow> = ["primary", "secondary"]
        .into_iter()
        .filter_map(|slot| window(slot, rl.get(slot)?, taken_at))
        .collect();
    let credits = rl
        .get("credits")
        .filter(|c| c.is_object())
        .map(|c| LimitCredits {
            has_credits: c.get("has_credits").and_then(Value::as_bool) == Some(true),
            unlimited: c.get("unlimited").and_then(Value::as_bool) == Some(true),
            balance: match c.get("balance") {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Number(n)) => Some(n.to_string()),
                _ => None,
            },
        });
    if windows.is_empty() && credits.is_none() {
        return Some(CodexLimitsLine::NoLimits);
    }
    Some(CodexLimitsLine::Snapshot(LimitSnapshot {
        limit_id: str_field(rl, "limit_id"),
        limit_name: str_field(rl, "limit_name"),
        plan: str_field(rl, "plan_type"),
        taken_at,
        windows,
        credits,
        reached: str_field(rl, "rate_limit_reached_type"),
    }))
}

fn str_field(obj: &Value, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn window(slot: &str, w: &Value, taken_at: DateTime<Utc>) -> Option<LimitWindow> {
    let used = w.get("used_percent").and_then(Value::as_f64)?;
    if !used.is_finite() {
        return None;
    }
    let resets_at = match w.get("resets_at") {
        Some(Value::Number(n)) => n.as_i64().and_then(epoch),
        Some(Value::String(s)) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.with_timezone(&Utc)),
        _ => None,
    }
    .or_else(|| {
        let secs = w.get("resets_in_seconds").and_then(Value::as_i64)?;
        taken_at.checked_add_signed(chrono::Duration::seconds(secs))
    });
    Some(LimitWindow {
        slot: slot.to_string(),
        window_minutes: w
            .get("window_minutes")
            .and_then(Value::as_u64)
            .filter(|m| *m > 0),
        used_percent: used.clamp(0.0, 100.0),
        resets_at,
    })
}

fn epoch(secs: i64) -> Option<DateTime<Utc>> {
    if secs <= 0 {
        return None;
    }
    Utc.timestamp_opt(secs, 0).single()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");

    fn snapshot(line: &str) -> LimitSnapshot {
        match parse_codex_rate_limits(line) {
            Some(CodexLimitsLine::Snapshot(s)) => s,
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }

    fn first_limits_line(rel: &str) -> String {
        let text = std::fs::read_to_string(format!("{FIXTURES}/{rel}")).unwrap();
        text.lines()
            .find(|l| l.contains("\"rate_limits\":{"))
            .unwrap()
            .to_string()
    }

    #[test]
    fn reads_the_old_rollout_layout() {
        // codex-old: no `ordinal` on the record.
        let line = first_limits_line("codex-old/07-exec-command.jsonl");
        assert!(!line.contains("\"ordinal\""));
        let s = snapshot(&line);
        assert_eq!(s.taken_at.to_rfc3339(), "2026-07-27T02:09:48.870+00:00");
        assert_eq!(s.windows.len(), 1, "secondary is null");
        let w = &s.windows[0];
        assert_eq!(w.slot, "primary");
        assert_eq!(w.window_minutes, Some(10080));
        assert_eq!(w.used_percent, 18.0);
        assert_eq!(w.resets_at.unwrap().timestamp(), 1785613225);
        assert_eq!(s.plan.as_deref(), Some("xxx"));
        assert_eq!(s.limit_id.as_deref(), Some("xxxxx"));
        assert_eq!(s.limit_name, None);
        assert_eq!(s.reached, None);
        let c = s.credits.unwrap();
        assert!(!c.has_credits && !c.unlimited);
        assert_eq!(c.balance.as_deref(), Some("0"));
    }

    #[test]
    fn reads_the_new_rollout_layout() {
        // codex-new: records carry `ordinal`.
        let line = first_limits_line("codex-new/02-file-change-token-usage.jsonl");
        assert!(line.contains("\"ordinal\""));
        let s = snapshot(&line);
        assert_eq!(s.taken_at.to_rfc3339(), "2026-09-04T11:48:05.145+00:00");
        assert_eq!(s.windows[0].used_percent, 69.0);
        assert_eq!(s.windows[0].resets_at.unwrap().timestamp(), 1788751010);
    }

    #[test]
    fn reads_the_live_shape_with_credits_and_both_windows() {
        let line = r#"{"timestamp":"2026-11-12T09:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"limit_id":"codex","limit_name":null,"primary":{"used_percent":13.0,"window_minutes":300,"resets_at":1794983830},"secondary":{"used_percent":41.5,"window_minutes":10080,"resets_at":1795400000},"credits":{"has_credits":true,"unlimited":false,"balance":"62500.0"},"plan_type":"pro","rate_limit_reached_type":null}}}"#;
        let s = snapshot(line);
        assert!(s.is_main());
        assert_eq!(s.plan.as_deref(), Some("pro"));
        assert_eq!(s.windows.len(), 2);
        assert_eq!(s.windows[1].slot, "secondary");
        assert_eq!(s.windows[1].used_percent, 41.5);
        assert_eq!(s.credits.unwrap().balance.as_deref(), Some("62500.0"));
    }

    #[test]
    fn older_reset_encodings_are_read() {
        let rel = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":120.0,"window_minutes":300,"resets_in_seconds":600}}}}"#;
        let s = snapshot(rel);
        assert!(s.is_main(), "no limit_id: the main allowance");
        assert_eq!(s.windows[0].used_percent, 100.0, "clamped");
        assert_eq!(
            s.windows[0].resets_at.unwrap().to_rfc3339(),
            "2026-01-01T00:10:00+00:00"
        );
        let iso = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":5,"resets_at":"2026-01-02T00:00:00Z"}}}}"#;
        let s = snapshot(iso);
        assert_eq!(s.windows[0].window_minutes, None);
        assert_eq!(
            s.windows[0].resets_at.unwrap().to_rfc3339(),
            "2026-01-02T00:00:00+00:00"
        );
    }

    #[test]
    fn api_key_lines_report_no_limits_and_other_lines_are_ignored() {
        let null = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{},"rate_limits":null}}"#;
        assert_eq!(
            parse_codex_rate_limits(null),
            Some(CodexLimitsLine::NoLimits)
        );
        let other = r#"{"timestamp":"2026-01-01T00:00:00Z","type":"event_msg","payload":{"type":"agent_message","message":"token_count"}}"#;
        assert_eq!(parse_codex_rate_limits(other), None);
        assert_eq!(parse_codex_rate_limits("not json \"token_count\""), None);
        // No timestamp: the percent could not be aged, so it is not reported.
        let untimed = r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":5}}}}"#;
        assert_eq!(parse_codex_rate_limits(untimed), None);
    }

    #[test]
    fn every_fixture_limits_line_parses() {
        for rel in [
            "codex-old/07-exec-command.jsonl",
            "codex-old/08-custom-exec-patch.jsonl",
            "codex-new/01-command-agent-message.jsonl",
            "codex-new/03-mcp-tool-call.jsonl",
        ] {
            let text = std::fs::read_to_string(format!("{FIXTURES}/{rel}")).unwrap();
            let n = text
                .lines()
                .filter(|l| l.contains("\"rate_limits\":{"))
                .map(snapshot)
                .count();
            assert!(n > 0, "{rel}");
        }
    }
}
