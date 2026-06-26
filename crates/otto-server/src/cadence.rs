//! Pure cadence logic for Scheduled Tasks — when is a task due, when does it next
//! run, and is a schedule spec valid?
//!
//! Mirrors the `interval | daily | weekly` spec format used by
//! `swarm_scheduler` / `workflow_trigger_scheduler`, with two deliberate changes:
//!   * the **cursor is a parameter** (`last_run`), not a `last_run` key inside the
//!     spec — the scheduled-tasks engine keeps the cursor in its own column so a
//!     config edit can never clobber it (the `cli_update` separation), and
//!   * daily/weekly use the robust `cli_update` catch-up comparison
//!     (`now >= scheduled_today && last < scheduled_today`) so a missed window
//!     still fires at the next opportunity.
//!
//! (`is_due` is conceptually the third copy of this logic in the tree, after
//! swarm/workflow — deliberately self-contained here to avoid touching those files
//! while 5 worktrees are in flight; dedup into one shared helper is a follow-up.)

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use otto_core::{Error, Result};
use serde_json::Value;

/// Minimum interval (minutes) we allow for an `agent_prompt` task — bounds the
/// unattended-agent resource cost (security review). Enforced by [`validate`].
pub const MIN_INTERVAL_MIN: i64 = 5;

/// Parse `at: "HH:MM"` from the spec, defaulting to 09:00 and clamping to range.
fn parse_at(spec: &Value) -> (u32, u32) {
    spec.get("at")
        .and_then(Value::as_str)
        .and_then(|s| {
            let mut it = s.split(':');
            let h = it.next()?.trim().parse::<u32>().ok()?;
            let m = it.next().unwrap_or("0").trim().parse::<u32>().ok()?;
            Some((h.min(23), m.min(59)))
        })
        .unwrap_or((9, 0))
}

fn every_min(spec: &Value) -> i64 {
    spec.get("every_min")
        .and_then(Value::as_i64)
        .unwrap_or(60)
        .max(MIN_INTERVAL_MIN)
}

fn cadence(spec: &Value) -> &str {
    spec.get("cadence").and_then(Value::as_str).unwrap_or("interval")
}

/// Today's `at` time as a UTC instant.
fn scheduled_today(now: DateTime<Utc>, h: u32, m: u32) -> Option<DateTime<Utc>> {
    Utc.with_ymd_and_hms(now.year(), now.month(), now.day(), h, m, 0).single()
}

/// Is the task due at `now` given its last completed-run cursor?
///
/// * `interval` — drift-based: due when never run, or `now - last >= every_min`
///   (floored to [`MIN_INTERVAL_MIN`]). Naturally catch-up-safe.
/// * `daily`    — due when `now >= today@at` and we haven't run since `today@at`.
/// * `weekly`   — same, gated on `weekday` (0=Mon … 6=Sun; default Monday).
pub fn is_due(spec: &Value, last_run: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match cadence(spec) {
        "interval" => match last_run {
            None => true,
            Some(l) => (now - l).num_minutes() >= every_min(spec),
        },
        "daily" => {
            let (h, m) = parse_at(spec);
            match scheduled_today(now, h, m) {
                Some(t) => now >= t && last_run.is_none_or(|l| l < t),
                None => false,
            }
        }
        "weekly" => {
            let wd = spec.get("weekday").and_then(Value::as_i64).unwrap_or(0) as u32;
            if now.weekday().num_days_from_monday() != wd.min(6) {
                return false;
            }
            let (h, m) = parse_at(spec);
            match scheduled_today(now, h, m) {
                Some(t) => now >= t && last_run.is_none_or(|l| l < t),
                None => false,
            }
        }
        _ => false,
    }
}

/// The next time the task should fire after `from` (for display in `next_run_at`).
pub fn next_run(spec: &Value, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match cadence(spec) {
        "interval" => Some(from + Duration::minutes(every_min(spec))),
        "daily" => {
            let (h, m) = parse_at(spec);
            let today = scheduled_today(from, h, m)?;
            Some(if from < today { today } else { today + Duration::days(1) })
        }
        "weekly" => {
            let (h, m) = parse_at(spec);
            let wd = spec.get("weekday").and_then(Value::as_i64).unwrap_or(0).clamp(0, 6) as u32;
            let today = scheduled_today(from, h, m)?;
            let cur = from.weekday().num_days_from_monday() as i64;
            let mut delta = (wd as i64 - cur).rem_euclid(7);
            if delta == 0 && from >= today {
                delta = 7;
            }
            Some(today + Duration::days(delta))
        }
        _ => None,
    }
}

/// Validate a schedule spec at create/update time. Rejects an unknown cadence, an
/// interval below [`MIN_INTERVAL_MIN`], and an out-of-range `at`/`weekday`.
pub fn validate(spec: &Value) -> Result<()> {
    match cadence(spec) {
        "interval" => {
            let raw = spec.get("every_min").and_then(Value::as_i64).unwrap_or(60);
            if raw < MIN_INTERVAL_MIN {
                return Err(Error::Invalid(format!(
                    "schedule.every_min must be at least {MIN_INTERVAL_MIN} minutes"
                )));
            }
        }
        "daily" => check_at(spec)?,
        "weekly" => {
            check_at(spec)?;
            let wd = spec.get("weekday").and_then(Value::as_i64).unwrap_or(0);
            if !(0..=6).contains(&wd) {
                return Err(Error::Invalid("schedule.weekday must be 0..=6 (Mon..Sun)".into()));
            }
        }
        other => {
            return Err(Error::Invalid(format!(
                "schedule.cadence must be interval|daily|weekly (got '{other}')"
            )))
        }
    }
    Ok(())
}

fn check_at(spec: &Value) -> Result<()> {
    let at = spec.get("at").and_then(Value::as_str).unwrap_or("09:00");
    let mut it = at.split(':');
    let ok = (|| {
        let h = it.next()?.trim().parse::<u32>().ok()?;
        let m = it.next()?.trim().parse::<u32>().ok()?;
        (h < 24 && m < 60).then_some(())
    })();
    ok.ok_or_else(|| Error::Invalid(format!("schedule.at must be 'HH:MM' (got '{at}')")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn interval_due_when_never_run() {
        let s = json!({"cadence":"interval","every_min":60});
        assert!(is_due(&s, None, utc(2026, 6, 26, 0, 0)));
    }

    #[test]
    fn interval_not_due_within_window() {
        let s = json!({"cadence":"interval","every_min":60});
        let last = utc(2026, 6, 26, 10, 0);
        assert!(!is_due(&s, Some(last), utc(2026, 6, 26, 10, 30)));
    }

    #[test]
    fn interval_due_after_window() {
        let s = json!({"cadence":"interval","every_min":60});
        let last = utc(2026, 6, 26, 9, 0);
        assert!(is_due(&s, Some(last), utc(2026, 6, 26, 10, 30)));
    }

    #[test]
    fn interval_floor_is_enforced_in_is_due() {
        // every_min below the floor is clamped up to MIN_INTERVAL_MIN.
        let s = json!({"cadence":"interval","every_min":1});
        let last = utc(2026, 6, 26, 10, 0);
        assert!(!is_due(&s, Some(last), utc(2026, 6, 26, 10, 2))); // 2 < 5
        assert!(is_due(&s, Some(last), utc(2026, 6, 26, 10, 6))); // 6 >= 5
    }

    #[test]
    fn daily_not_due_before_time() {
        let s = json!({"cadence":"daily","at":"03:00"});
        assert!(!is_due(&s, None, utc(2026, 6, 26, 2, 30)));
    }

    #[test]
    fn daily_due_after_time_and_catch_up() {
        let s = json!({"cadence":"daily","at":"03:00"});
        assert!(is_due(&s, None, utc(2026, 6, 26, 9, 0)));
        // ran yesterday → catch up today
        assert!(is_due(&s, Some(utc(2026, 6, 25, 3, 1)), utc(2026, 6, 26, 9, 0)));
        // already ran today → not due
        assert!(!is_due(&s, Some(utc(2026, 6, 26, 3, 1)), utc(2026, 6, 26, 9, 0)));
    }

    #[test]
    fn weekly_only_on_weekday() {
        // 2026-06-26 is a Friday (weekday 4 from Monday).
        let fri = json!({"cadence":"weekly","at":"03:00","weekday":4});
        assert!(is_due(&fri, None, utc(2026, 6, 26, 9, 0)));
        let mon = json!({"cadence":"weekly","at":"03:00","weekday":0});
        assert!(!is_due(&mon, None, utc(2026, 6, 26, 9, 0)));
    }

    #[test]
    fn next_run_interval_is_last_plus_every() {
        let s = json!({"cadence":"interval","every_min":60});
        let from = utc(2026, 6, 26, 10, 0);
        assert_eq!(next_run(&s, from), Some(utc(2026, 6, 26, 11, 0)));
    }

    #[test]
    fn validate_rejects_short_interval_and_bad_cadence() {
        assert!(validate(&json!({"cadence":"interval","every_min":1})).is_err());
        assert!(validate(&json!({"cadence":"interval","every_min":60})).is_ok());
        assert!(validate(&json!({"cadence":"monthly"})).is_err());
        assert!(validate(&json!({"cadence":"daily","at":"25:00"})).is_err());
        assert!(validate(&json!({"cadence":"daily","at":"03:30"})).is_ok());
        assert!(validate(&json!({"cadence":"weekly","at":"03:00","weekday":9})).is_err());
    }
}
