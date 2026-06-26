//! Pure cadence logic for Scheduled Tasks — when is a task due, when does it next
//! run, and is a schedule spec valid?
//!
//! v2 adds two capabilities over the original `interval | daily | weekly` model,
//! both of which the rest of the engine threads a task's IANA `timezone` through:
//!   * **timezone-aware** daily/weekly — `at:"HH:MM"` is interpreted in the task's
//!     timezone (DST-correct), not UTC.
//!   * an optional **cron** cadence — `{cadence:"cron", expr:"<5-field cron>"}`,
//!     evaluated in the task's timezone, via the self-contained [`cron`] parser
//!     (standard Vixie semantics; no external crate).
//!
//! The cursor is a parameter (`last_run`), not a key inside the spec — the engine
//! keeps the cursor in its own column so a config edit can never clobber it. The
//! UTC default timezone makes every pre-v2 task behave exactly as before.

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use otto_core::{Error, Result};
use serde_json::Value;

/// Minimum interval (minutes) we allow for an `agent_prompt` task — bounds the
/// unattended-agent resource cost (security review). Enforced by [`validate`].
pub const MIN_INTERVAL_MIN: i64 = 5;

/// Parse a task's IANA timezone string, defaulting to UTC on empty/unknown.
pub fn task_tz(timezone: &str) -> Tz {
    let t = timezone.trim();
    if t.is_empty() {
        return Tz::UTC;
    }
    t.parse::<Tz>().unwrap_or(Tz::UTC)
}

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

/// The `at` time on `now`'s LOCAL day (in `tz`), as a UTC instant. DST-safe: a
/// non-existent local time (spring-forward gap) resolves to the next valid
/// instant; an ambiguous one (fall-back) takes the earlier offset.
fn scheduled_today(now: DateTime<Utc>, tz: Tz, h: u32, m: u32) -> Option<DateTime<Utc>> {
    let local_day = now.with_timezone(&tz).date_naive();
    let naive = local_day.and_hms_opt(h, m, 0)?;
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
        chrono::LocalResult::Ambiguous(a, _) => Some(a.with_timezone(&Utc)),
        chrono::LocalResult::None => {
            // Gap: step forward an hour until the wall time exists.
            let bumped = local_day.and_hms_opt((h + 1).min(23), m, 0)?;
            tz.from_local_datetime(&bumped)
                .earliest()
                .map(|dt| dt.with_timezone(&Utc))
        }
    }
}

/// Is the task due at `now` (given its last completed-run cursor + timezone)?
pub fn is_due(spec: &Value, last_run: Option<DateTime<Utc>>, now: DateTime<Utc>, tz: Tz) -> bool {
    match cadence(spec) {
        "interval" => match last_run {
            None => true,
            Some(l) => (now - l).num_minutes() >= every_min(spec),
        },
        "daily" => {
            let (h, m) = parse_at(spec);
            match scheduled_today(now, tz, h, m) {
                Some(t) => now >= t && last_run.is_none_or(|l| l < t),
                None => false,
            }
        }
        "weekly" => {
            let wd = spec.get("weekday").and_then(Value::as_i64).unwrap_or(0) as u32;
            if now.with_timezone(&tz).weekday().num_days_from_monday() != wd.min(6) {
                return false;
            }
            let (h, m) = parse_at(spec);
            match scheduled_today(now, tz, h, m) {
                Some(t) => now >= t && last_run.is_none_or(|l| l < t),
                None => false,
            }
        }
        "cron" => match cron::Schedule::parse(cron_expr(spec)) {
            Ok(sched) => {
                // Due when the next fire AFTER the cursor (or, if never run, after
                // a minute ago so a just-created matching minute fires) is <= now.
                let anchor = last_run.unwrap_or_else(|| now - Duration::minutes(1));
                sched.next_after(anchor, tz).is_some_and(|next| next <= now)
            }
            Err(_) => false,
        },
        _ => false,
    }
}

/// The next time the task should fire after `from` (for display in `next_run_at`).
pub fn next_run(spec: &Value, from: DateTime<Utc>, tz: Tz) -> Option<DateTime<Utc>> {
    match cadence(spec) {
        "interval" => Some(from + Duration::minutes(every_min(spec))),
        "daily" => {
            let (h, m) = parse_at(spec);
            let today = scheduled_today(from, tz, h, m)?;
            Some(if from < today { today } else { today + Duration::days(1) })
        }
        "weekly" => {
            let (h, m) = parse_at(spec);
            let wd = spec.get("weekday").and_then(Value::as_i64).unwrap_or(0).clamp(0, 6) as u32;
            let today = scheduled_today(from, tz, h, m)?;
            let cur = from.with_timezone(&tz).weekday().num_days_from_monday() as i64;
            let mut delta = (wd as i64 - cur).rem_euclid(7);
            if delta == 0 && from >= today {
                delta = 7;
            }
            Some(today + Duration::days(delta))
        }
        "cron" => cron::Schedule::parse(cron_expr(spec)).ok()?.next_after(from, tz),
        _ => None,
    }
}

fn cron_expr(spec: &Value) -> &str {
    spec.get("expr").and_then(Value::as_str).unwrap_or("")
}

/// Validate a schedule spec at create/update time.
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
        "cron" => {
            let expr = cron_expr(spec);
            cron::Schedule::parse(expr).map_err(|e| {
                Error::Invalid(format!("schedule.expr is not a valid cron expression: {e}"))
            })?;
        }
        other => {
            return Err(Error::Invalid(format!(
                "schedule.cadence must be interval|daily|weekly|cron (got '{other}')"
            )))
        }
    }
    Ok(())
}

/// A short human description of a schedule (for logs / the run summary footer).
pub fn describe(spec: &Value, tz: Tz) -> String {
    match cadence(spec) {
        "interval" => format!("every {} min", every_min(spec)),
        "daily" => {
            let (h, m) = parse_at(spec);
            format!("daily at {h:02}:{m:02} {tz}")
        }
        "weekly" => {
            let (h, m) = parse_at(spec);
            let wd = spec.get("weekday").and_then(Value::as_i64).unwrap_or(0).clamp(0, 6) as usize;
            let names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
            format!("weekly {} at {h:02}:{m:02} {tz}", names[wd])
        }
        "cron" => format!("cron `{}` ({tz})", cron_expr(spec)),
        other => other.to_string(),
    }
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

/// A minimal, self-contained standard 5-field cron parser + scheduler. Fields:
/// `minute hour day-of-month month day-of-week`. Supports `*`, lists (`1,2`),
/// ranges (`1-5`), and steps (`*/5`, `1-30/2`). Day-of-week is 0–6 with 0=Sunday
/// (7 also accepted for Sunday). Vixie semantics: when BOTH day-of-month and
/// day-of-week are restricted, a day matches if EITHER matches.
pub mod cron {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct Schedule {
        minutes: Vec<u32>,
        hours: Vec<u32>,
        doms: Vec<u32>,
        months: Vec<u32>,
        dows: Vec<u32>, // 0=Sun..6=Sat
        dom_restricted: bool,
        dow_restricted: bool,
    }

    impl Schedule {
        /// Parse a 5-field cron expression (whitespace-separated).
        pub fn parse(expr: &str) -> std::result::Result<Schedule, String> {
            let fields: Vec<&str> = expr.split_whitespace().collect();
            if fields.len() != 5 {
                return Err(format!("expected 5 fields, got {}", fields.len()));
            }
            let minutes = parse_field(fields[0], 0, 59)?;
            let hours = parse_field(fields[1], 0, 23)?;
            let doms = parse_field(fields[2], 1, 31)?;
            let months = parse_field(fields[3], 1, 12)?;
            // Normalise dow 7 → 0 (both are Sunday).
            let dows_raw = parse_field(fields[4], 0, 7)?;
            let mut dows: Vec<u32> = dows_raw.into_iter().map(|d| if d == 7 { 0 } else { d }).collect();
            dows.sort_unstable();
            dows.dedup();
            Ok(Schedule {
                minutes,
                hours,
                doms,
                months,
                dows,
                dom_restricted: fields[2].trim() != "*",
                dow_restricted: fields[4].trim() != "*",
            })
        }

        fn day_matches(&self, date: chrono::NaiveDate) -> bool {
            let dom = date.day();
            let dow = date.weekday().num_days_from_sunday(); // 0=Sun
            let dom_ok = self.doms.contains(&dom);
            let dow_ok = self.dows.contains(&dow);
            match (self.dom_restricted, self.dow_restricted) {
                (true, true) => dom_ok || dow_ok, // Vixie OR
                (true, false) => dom_ok,
                (false, true) => dow_ok,
                (false, false) => true,
            }
        }

        /// The first cron instant strictly after `after`, evaluated in `tz`.
        /// Returns `None` if nothing matches within ~4 years (e.g. impossible
        /// date). Day-level fast-forward keeps even yearly crons cheap.
        pub fn next_after(&self, after: DateTime<Utc>, tz: Tz) -> Option<DateTime<Utc>> {
            let local = after.with_timezone(&tz);
            // First candidate: the next whole minute after `after`.
            let mut cand = local
                .with_second(0)?
                .with_nanosecond(0)?
                .checked_add_signed(Duration::minutes(1))?;
            let limit = local + Duration::days(366 * 4);
            let mut guard = 0u32;
            while cand <= limit {
                guard += 1;
                if guard > 5_000_000 {
                    return None;
                }
                let date = cand.date_naive();
                if !self.months.contains(&date.month()) || !self.day_matches(date) {
                    // Skip to 00:00 of the next day (in local tz).
                    let next_day = date.checked_add_signed(Duration::days(1))?;
                    cand = match tz.from_local_datetime(&next_day.and_hms_opt(0, 0, 0)?).earliest() {
                        Some(dt) => dt,
                        None => cand.checked_add_signed(Duration::hours(1))?,
                    };
                    continue;
                }
                if !self.hours.contains(&cand.hour()) {
                    cand = cand.checked_add_signed(Duration::minutes(60 - cand.minute() as i64))?;
                    // realign to top of the next hour
                    cand = cand.with_minute(0)?;
                    continue;
                }
                if !self.minutes.contains(&cand.minute()) {
                    cand = cand.checked_add_signed(Duration::minutes(1))?;
                    continue;
                }
                return Some(cand.with_timezone(&Utc));
            }
            None
        }
    }

    /// Parse one cron field into the explicit list of matching values in [lo,hi].
    fn parse_field(field: &str, lo: u32, hi: u32) -> std::result::Result<Vec<u32>, String> {
        let mut out: Vec<u32> = Vec::new();
        for part in field.split(',') {
            let part = part.trim();
            if part.is_empty() {
                return Err("empty field part".into());
            }
            // step: BASE/STEP
            let (base, step) = match part.split_once('/') {
                Some((b, s)) => {
                    let step = s.parse::<u32>().map_err(|_| format!("bad step '{s}'"))?;
                    if step == 0 {
                        return Err("step must be > 0".into());
                    }
                    (b, step)
                }
                None => (part, 1),
            };
            // range or star
            let (start, end) = if base == "*" {
                (lo, hi)
            } else if let Some((a, b)) = base.split_once('-') {
                let a = a.parse::<u32>().map_err(|_| format!("bad range start '{a}'"))?;
                let b = b.parse::<u32>().map_err(|_| format!("bad range end '{b}'"))?;
                (a, b)
            } else {
                let v = base.parse::<u32>().map_err(|_| format!("bad value '{base}'"))?;
                (v, v)
            };
            if start < lo || end > hi || start > end {
                return Err(format!("value out of range [{lo},{hi}] in '{part}'"));
            }
            let mut v = start;
            while v <= end {
                out.push(v);
                v += step;
            }
        }
        out.sort_unstable();
        out.dedup();
        if out.is_empty() {
            return Err("no values".into());
        }
        Ok(out)
    }
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
        assert!(is_due(&s, None, utc(2026, 6, 26, 0, 0), Tz::UTC));
    }

    #[test]
    fn interval_not_due_within_window() {
        let s = json!({"cadence":"interval","every_min":60});
        let last = utc(2026, 6, 26, 10, 0);
        assert!(!is_due(&s, Some(last), utc(2026, 6, 26, 10, 30), Tz::UTC));
    }

    #[test]
    fn interval_floor_is_enforced_in_is_due() {
        let s = json!({"cadence":"interval","every_min":1});
        let last = utc(2026, 6, 26, 10, 0);
        assert!(!is_due(&s, Some(last), utc(2026, 6, 26, 10, 2), Tz::UTC));
        assert!(is_due(&s, Some(last), utc(2026, 6, 26, 10, 6), Tz::UTC));
    }

    #[test]
    fn daily_utc_unchanged() {
        let s = json!({"cadence":"daily","at":"03:00"});
        assert!(!is_due(&s, None, utc(2026, 6, 26, 2, 30), Tz::UTC));
        assert!(is_due(&s, None, utc(2026, 6, 26, 9, 0), Tz::UTC));
        assert!(!is_due(&s, Some(utc(2026, 6, 26, 3, 1)), utc(2026, 6, 26, 9, 0), Tz::UTC));
    }

    #[test]
    fn daily_in_timezone_offsets_the_fire_instant() {
        // 09:00 in New York (UTC-4 in June) == 13:00 UTC.
        let tz: Tz = "America/New_York".parse().unwrap();
        let s = json!({"cadence":"daily","at":"09:00"});
        // 12:30 UTC is before 13:00 UTC → not due yet.
        assert!(!is_due(&s, None, utc(2026, 6, 26, 12, 30), tz));
        // 13:30 UTC is after → due.
        assert!(is_due(&s, None, utc(2026, 6, 26, 13, 30), tz));
    }

    #[test]
    fn weekly_only_on_weekday_in_tz() {
        // 2026-06-26 is a Friday (weekday 4 from Monday) in UTC.
        let fri = json!({"cadence":"weekly","at":"03:00","weekday":4});
        assert!(is_due(&fri, None, utc(2026, 6, 26, 9, 0), Tz::UTC));
        let mon = json!({"cadence":"weekly","at":"03:00","weekday":0});
        assert!(!is_due(&mon, None, utc(2026, 6, 26, 9, 0), Tz::UTC));
    }

    #[test]
    fn next_run_interval_is_last_plus_every() {
        let s = json!({"cadence":"interval","every_min":60});
        let from = utc(2026, 6, 26, 10, 0);
        assert_eq!(next_run(&s, from, Tz::UTC), Some(utc(2026, 6, 26, 11, 0)));
    }

    #[test]
    fn validate_rejects_short_interval_and_bad_cadence() {
        assert!(validate(&json!({"cadence":"interval","every_min":1})).is_err());
        assert!(validate(&json!({"cadence":"interval","every_min":60})).is_ok());
        assert!(validate(&json!({"cadence":"monthly"})).is_err());
        assert!(validate(&json!({"cadence":"daily","at":"25:00"})).is_err());
        assert!(validate(&json!({"cadence":"weekly","at":"03:00","weekday":9})).is_err());
    }

    // --- cron ---

    #[test]
    fn cron_validate() {
        assert!(validate(&json!({"cadence":"cron","expr":"0 9 * * 1"})).is_ok());
        assert!(validate(&json!({"cadence":"cron","expr":"*/15 * * * *"})).is_ok());
        assert!(validate(&json!({"cadence":"cron","expr":"0 9 * *"})).is_err()); // 4 fields
        assert!(validate(&json!({"cadence":"cron","expr":"99 9 * * 1"})).is_err()); // minute > 59
        assert!(validate(&json!({"cadence":"cron","expr":"0 9 * * 9"})).is_err()); // dow > 7
    }

    #[test]
    fn cron_next_after_daily_9am() {
        let sched = cron::Schedule::parse("0 9 * * *").unwrap();
        // From 2026-06-26 08:00 UTC → next is 09:00 same day.
        let next = sched.next_after(utc(2026, 6, 26, 8, 0), Tz::UTC).unwrap();
        assert_eq!(next, utc(2026, 6, 26, 9, 0));
        // From 09:30 → next is tomorrow 09:00.
        let next2 = sched.next_after(utc(2026, 6, 26, 9, 30), Tz::UTC).unwrap();
        assert_eq!(next2, utc(2026, 6, 27, 9, 0));
    }

    #[test]
    fn cron_every_15_min() {
        let sched = cron::Schedule::parse("*/15 * * * *").unwrap();
        let next = sched.next_after(utc(2026, 6, 26, 9, 7), Tz::UTC).unwrap();
        assert_eq!(next, utc(2026, 6, 26, 9, 15));
    }

    #[test]
    fn cron_weekday_monday_9am() {
        // 2026-06-26 is Friday; next Monday is 2026-06-29.
        let sched = cron::Schedule::parse("0 9 * * 1").unwrap();
        let next = sched.next_after(utc(2026, 6, 26, 10, 0), Tz::UTC).unwrap();
        assert_eq!(next, utc(2026, 6, 29, 9, 0));
    }

    #[test]
    fn cron_due_when_passed() {
        let s = json!({"cadence":"cron","expr":"0 9 * * *"});
        // Last ran yesterday 09:00; now today 09:01 → due.
        assert!(is_due(&s, Some(utc(2026, 6, 25, 9, 0)), utc(2026, 6, 26, 9, 1), Tz::UTC));
        // Now today 08:59 → not yet.
        assert!(!is_due(&s, Some(utc(2026, 6, 25, 9, 0)), utc(2026, 6, 26, 8, 59), Tz::UTC));
    }

    #[test]
    fn cron_in_timezone() {
        // "0 9 * * *" in New York (UTC-4 June) → 13:00 UTC.
        let tz: Tz = "America/New_York".parse().unwrap();
        let sched = cron::Schedule::parse("0 9 * * *").unwrap();
        let next = sched.next_after(utc(2026, 6, 26, 0, 0), tz).unwrap();
        assert_eq!(next, utc(2026, 6, 26, 13, 0));
    }

    #[test]
    fn cron_list_and_range() {
        let sched = cron::Schedule::parse("0 9,17 * * 1-5").unwrap();
        // Friday 2026-06-26: next after 10:00 is 17:00.
        assert_eq!(sched.next_after(utc(2026, 6, 26, 10, 0), Tz::UTC).unwrap(), utc(2026, 6, 26, 17, 0));
    }
}
