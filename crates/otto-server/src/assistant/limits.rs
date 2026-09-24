//! Usage-limit detection for the assistant router. Pure: scans text the
//! provider CLI drew (the PTY screen tail) or wrote (a short reply turn) for
//! its "you hit your plan limit" line. There is no limit tracking in
//! `otto-usage` yet, so this is the only signal: a hit becomes an
//! `assistant_limit` event plus a "continue on X?" needs-you item — never a
//! silent switch (plan §2.4). Periodic `/usage` / `/status` probing is not
//! implemented (`source: "probe"` is reserved).

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// `AssistantLimitState` on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitState {
    pub provider: String,
    pub account_id: Option<String>,
    pub limited: bool,
    pub until: Option<String>,
    pub message: String,
    pub source: String,
    pub detected_at: String,
}

/// What a scan found.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitHit {
    /// The provider's own line (trimmed, ≤ 240 chars).
    pub message: String,
    /// When it resets, when the text said so machine-readably.
    pub until: Option<DateTime<Utc>>,
}

/// Lower-case needles that mean "plan / usage limit reached". Kept specific:
/// a generic "rate limit" inside a tool's output must not trip it.
const NEEDLES: &[&str] = &[
    "usage limit reached",
    "claude ai usage limit reached",
    "you've hit your limit",
    "you've hit your usage limit",
    "you have hit your usage limit",
    "you've reached your usage limit",
    "5-hour limit reached",
    "weekly limit reached",
    "opus limit reached",
    "usage limit has been reached",
    "limit will reset at",
];

/// Scan `text` (screen rows joined by `\n`, or a reply) for a limit line.
/// Only the LAST 20 non-empty lines are considered — the limit notice is what
/// the CLI printed last, and an older mention higher up (the user asking about
/// limits, say) must not re-trigger it.
pub fn detect_limit(text: &str) -> Option<LimitHit> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = &lines[lines.len().saturating_sub(20)..];
    for line in tail.iter().rev() {
        let lower = line.to_lowercase();
        if NEEDLES.iter().any(|n| lower.contains(n)) {
            let clean = line
                .trim()
                .trim_matches(|c: char| "│|>⎿ ".contains(c))
                .trim();
            let message: String = clean.chars().take(240).collect();
            return Some(LimitHit {
                until: epoch_after_pipe(clean),
                message,
            });
        }
    }
    None
}

/// Claude Code writes the limit as `Claude AI usage limit reached|<epoch>` in
/// its transcript — the part after the pipe is the reset instant.
fn epoch_after_pipe(line: &str) -> Option<DateTime<Utc>> {
    let (_, tail) = line.rsplit_once('|')?;
    let secs: i64 = tail.trim().parse().ok()?;
    // Sanity: a plausible instant (2020..2100), seconds not millis.
    if !(1_577_836_800..4_102_444_800).contains(&secs) {
        return None;
    }
    Utc.timestamp_opt(secs, 0).single()
}

/// Build the wire state for a hit.
pub fn state_for(
    provider: &str,
    account_id: Option<&str>,
    hit: &LimitHit,
    source: &str,
    now: DateTime<Utc>,
) -> LimitState {
    LimitState {
        provider: provider.to_string(),
        account_id: account_id.map(str::to_string),
        limited: true,
        until: hit.until.map(|t| t.to_rfc3339()),
        message: hit.message.clone(),
        source: source.to_string(),
        detected_at: now.to_rfc3339(),
    }
}

/// Replace the snapshot entry for (provider, account) and expire entries whose
/// `until` has passed (they flip to `limited: false`). Returns the new list.
pub fn upsert_snapshot(
    mut list: Vec<LimitState>,
    state: LimitState,
    now: DateTime<Utc>,
) -> Vec<LimitState> {
    list.retain(|s| !(s.provider == state.provider && s.account_id == state.account_id));
    list.push(state);
    expire(list, now)
}

/// Flip entries whose reset instant has passed to `limited: false`.
pub fn expire(mut list: Vec<LimitState>, now: DateTime<Utc>) -> Vec<LimitState> {
    for s in &mut list {
        let passed = s
            .until
            .as_deref()
            .and_then(|u| DateTime::parse_from_rfc3339(u).ok())
            .is_some_and(|u| u.with_timezone(&Utc) <= now);
        if passed {
            s.limited = false;
        }
    }
    list
}

/// Providers currently limited (for the router's failover input). An entry
/// with no known reset time counts as limited for 5 hours after detection.
pub fn limited_providers(list: &[LimitState], now: DateTime<Utc>) -> Vec<String> {
    list.iter()
        .filter(|s| s.limited)
        .filter(|s| match s.until.as_deref() {
            Some(u) => DateTime::parse_from_rfc3339(u)
                .map(|u| u.with_timezone(&Utc) > now)
                .unwrap_or(true),
            None => DateTime::parse_from_rfc3339(&s.detected_at)
                .map(|d| now - d.with_timezone(&Utc) < chrono::Duration::hours(5))
                .unwrap_or(false),
        })
        .map(|s| s.provider.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 24, h, 0, 0).unwrap()
    }

    #[test]
    fn detects_the_claude_and_codex_limit_lines() {
        let claude = "⏺ Working on it\n\n  ⎿  5-hour limit reached ∙ resets 2pm\n     /upgrade to increase your usage limit.";
        let hit = detect_limit(claude).unwrap();
        assert!(hit.message.contains("5-hour limit reached"));
        let codex = "■ You've hit your usage limit. Upgrade to Pro or try again in 2 hours 5 minutes.";
        assert!(detect_limit(codex).is_some());
        let transcript = "Claude AI usage limit reached|1790265600";
        let hit = detect_limit(transcript).unwrap();
        assert_eq!(hit.until.unwrap().timestamp(), 1_790_265_600);
    }

    #[test]
    fn ordinary_text_and_old_mentions_do_not_trip_it() {
        assert!(detect_limit("The API returned a rate limit error, retrying.").is_none());
        assert!(detect_limit("").is_none());
        // A mention far above the last 20 lines is ignored.
        let mut text = String::from("you've hit your usage limit (quoted earlier)\n");
        for i in 0..25 {
            text.push_str(&format!("line {i}\n"));
        }
        assert!(detect_limit(&text).is_none());
        // A bogus epoch is not a reset time.
        assert!(detect_limit("usage limit reached|12").unwrap().until.is_none());
    }

    #[test]
    fn snapshot_replaces_per_provider_and_expires() {
        let hit = LimitHit {
            message: "limit".into(),
            until: Some(at(14)),
        };
        let s1 = state_for("claude", None, &hit, "pty", at(10));
        let list = upsert_snapshot(Vec::new(), s1.clone(), at(10));
        assert_eq!(limited_providers(&list, at(11)), vec!["claude".to_string()]);
        // Same provider again replaces, never duplicates.
        let list = upsert_snapshot(list, s1, at(10));
        assert_eq!(list.len(), 1);
        // After the reset instant it is no longer limited.
        assert!(limited_providers(&list, at(15)).is_empty());
        assert!(!expire(list, at(15))[0].limited);
        // No reset time: limited for 5 hours after detection.
        let open = LimitHit {
            message: "limit".into(),
            until: None,
        };
        let list = vec![state_for("codex", None, &open, "pty", at(8))];
        assert_eq!(limited_providers(&list, at(12)), vec!["codex".to_string()]);
        assert!(limited_providers(&list, at(14)).is_empty());
    }
}
