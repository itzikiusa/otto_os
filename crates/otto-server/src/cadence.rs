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

use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc,
};
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
    spec.get("cadence")
        .and_then(Value::as_str)
        .unwrap_or("interval")
}

/// Resolve a LOCAL wall-clock time in `tz` to a UTC instant, DST-safe: an
/// ambiguous time (fall-back overlap) takes the EARLIER instant, and a
/// non-existent one (spring-forward gap) resolves to the first valid instant
/// after the gap (the moment the clock jumps) — Vixie cron's "a job skipped by
/// the gap runs right after it" behaviour.
fn resolve_local(tz: Tz, naive: NaiveDateTime) -> Option<DateTime<Utc>> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(a, _) => Some(a.with_timezone(&Utc)),
        LocalResult::None => {
            // Gap: walk the wall clock forward to where it exists again. Real
            // gaps are an hour (at most a few); the cap only bounds a bad tzdb.
            let mut n = naive;
            for _ in 0..(24 * 60) {
                n = n.checked_add_signed(Duration::minutes(1))?;
                if let Some(dt) = tz.from_local_datetime(&n).earliest() {
                    return Some(dt.with_timezone(&Utc));
                }
            }
            None
        }
    }
}

/// The `at` time on a given LOCAL date (in `tz`), as a UTC instant (DST-safe,
/// see [`resolve_local`]).
fn scheduled_on(day: NaiveDate, tz: Tz, h: u32, m: u32) -> Option<DateTime<Utc>> {
    resolve_local(tz, day.and_hms_opt(h, m, 0)?)
}

/// The `at` time on `now`'s LOCAL day (in `tz`), as a UTC instant. DST-safe: a
/// non-existent local time (spring-forward gap) resolves to the first valid
/// instant after the gap; an ambiguous one (fall-back) takes the earlier offset.
fn scheduled_today(now: DateTime<Utc>, tz: Tz, h: u32, m: u32) -> Option<DateTime<Utc>> {
    scheduled_on(now.with_timezone(&tz).date_naive(), tz, h, m)
}

/// How far back a never-run cron schedule looks for a missed first fire when
/// the caller knows when the schedule was created (see [`is_due_since`]).
const FIRST_FIRE_LOOKBACK_DAYS: i64 = 7;

/// Is the task due at `now` (given its last completed-run cursor + timezone)?
pub fn is_due(spec: &Value, last_run: Option<DateTime<Utc>>, now: DateTime<Utc>, tz: Tz) -> bool {
    is_due_since(spec, last_run, None, now, tz)
}

/// [`is_due`] plus the schedule's creation instant. A cron that has never run
/// otherwise only looks one minute back, so its FIRST fire is lost when the Mac
/// slept / the daemon was down at that minute (or a tick straddled it) and it
/// waits a whole period. With `created` known, the never-run anchor is the
/// creation time (bounded to [`FIRST_FIRE_LOOKBACK_DAYS`]), so a missed first
/// fire is caught up exactly once — like `daily`/`weekly` already do.
pub fn is_due_since(
    spec: &Value,
    last_run: Option<DateTime<Utc>>,
    created: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    tz: Tz,
) -> bool {
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
                // Due when the next fire AFTER the cursor is <= now. Never run: after
                // the creation time (bounded), else after a minute ago so a
                // just-created matching minute fires.
                let anchor = last_run.unwrap_or_else(|| match created {
                    Some(c) => c.max(now - Duration::days(FIRST_FIRE_LOOKBACK_DAYS)),
                    None => now - Duration::minutes(1),
                });
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
        // Daily/weekly step LOCAL dates (not `+ n days` in UTC), so the shown
        // next run stays at the wall-clock `at` across a DST change.
        "daily" => {
            let (h, m) = parse_at(spec);
            let day = from.with_timezone(&tz).date_naive();
            let today = scheduled_on(day, tz, h, m)?;
            if from < today {
                Some(today)
            } else {
                scheduled_on(day.succ_opt()?, tz, h, m)
            }
        }
        "weekly" => {
            let (h, m) = parse_at(spec);
            let wd = spec
                .get("weekday")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .clamp(0, 6) as u32;
            let day = from.with_timezone(&tz).date_naive();
            let today = scheduled_on(day, tz, h, m)?;
            let cur = day.weekday().num_days_from_monday() as i64;
            let mut delta = (wd as i64 - cur).rem_euclid(7);
            if delta == 0 && from >= today {
                delta = 7;
            }
            scheduled_on(day.checked_add_signed(Duration::days(delta))?, tz, h, m)
        }
        "cron" => cron::Schedule::parse(cron_expr(spec))
            .ok()?
            .next_after(from, tz),
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
                return Err(Error::Invalid(
                    "schedule.weekday must be 0..=6 (Mon..Sun)".into(),
                ));
            }
        }
        "cron" => {
            let expr = cron_expr(spec);
            let sched = cron::Schedule::parse(expr).map_err(|e| {
                Error::Invalid(format!("schedule.expr is not a valid cron expression: {e}"))
            })?;
            // A well-formed but impossible date (e.g. `0 0 31 2 *`) would be
            // saved and then silently never fire.
            if sched.next_after(Utc::now(), Tz::UTC).is_none() {
                return Err(Error::Invalid(format!(
                    "schedule.expr '{expr}' never fires (no matching date)"
                )));
            }
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
            let wd = spec
                .get("weekday")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .clamp(0, 6) as usize;
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
            let mut dows: Vec<u32> = dows_raw
                .into_iter()
                .map(|d| if d == 7 { 0 } else { d })
                .collect();
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

        /// Does any wall-clock minute in `[from, to)` match the schedule? Used
        /// for the minutes a spring-forward gap removed from the local clock.
        fn gap_matches(&self, from: NaiveDateTime, to: NaiveDateTime) -> bool {
            let mut n = from;
            let mut steps = 0u32;
            while n < to && steps < 24 * 60 {
                steps += 1;
                let date = n.date();
                if self.months.contains(&date.month())
                    && self.day_matches(date)
                    && self.hours.contains(&n.hour())
                    && self.minutes.contains(&n.minute())
                {
                    return true;
                }
                match n.checked_add_signed(Duration::minutes(1)) {
                    Some(next) => n = next,
                    None => return false,
                }
            }
            false
        }

        /// The first cron instant strictly after `after`, evaluated in `tz`.
        /// Returns `None` if nothing matches within ~4 years (e.g. impossible
        /// date). Day-level fast-forward keeps even yearly crons cheap.
        ///
        /// Walks REAL instants (whole UTC minutes) and matches each one's LOCAL
        /// wall-clock fields. It never rebuilds a local time through `with_*` on
        /// a `DateTime<Tz>`: those resolve via `.single()` and return `None` on an
        /// ambiguous (fall-back) wall time, which used to end the search — and,
        /// since the cursor only advances on a run, the schedule — for good.
        ///
        /// DST semantics (Vixie cron's): a job with a FIXED hour list fires once
        /// on fall-back day (in the first pass of the repeated hour) and, when its
        /// time falls in a spring-forward gap, fires once right after the gap. A
        /// job whose hour field is `*` runs in real time through both.
        pub fn next_after(&self, after: DateTime<Utc>, tz: Tz) -> Option<DateTime<Utc>> {
            // First candidate: the next whole minute after `after` (UTC arithmetic).
            let secs = after.timestamp();
            let mut cand = DateTime::<Utc>::from_timestamp(secs - secs.rem_euclid(60) + 60, 0)?;
            let limit = after.checked_add_signed(Duration::days(366 * 4))?;
            let every_hour = self.hours.len() == 24;
            let mut guard = 0u32;
            while cand <= limit {
                guard += 1;
                if guard > 5_000_000 {
                    return None;
                }
                let local = cand.with_timezone(&tz).naive_local();
                // Spring-forward: the wall clock jumped straight to `local`. A
                // fixed-hour job whose time was in the skipped span fires now.
                if !every_hour {
                    let prev = cand
                        .checked_sub_signed(Duration::minutes(1))?
                        .with_timezone(&tz)
                        .naive_local();
                    let expected = prev.checked_add_signed(Duration::minutes(1))?;
                    if local > expected && self.gap_matches(expected, local) {
                        return Some(cand);
                    }
                }
                let date = local.date();
                if !self.months.contains(&date.month()) || !self.day_matches(date) {
                    // Skip to the first instant of the next LOCAL day.
                    let next = resolve_local(tz, date.succ_opt()?.and_hms_opt(0, 0, 0)?);
                    cand = match next {
                        Some(n) if n > cand => n,
                        _ => cand.checked_add_signed(Duration::hours(1))?,
                    };
                    continue;
                }
                if !self.hours.contains(&local.hour()) {
                    // Top of the next hour (real minutes, so a skip can never land
                    // on — or loop in — a wall time that does not exist).
                    cand =
                        cand.checked_add_signed(Duration::minutes(60 - local.minute() as i64))?;
                    continue;
                }
                // Fall-back: this is the SECOND pass of the repeated hour. A
                // fixed-hour job already had its chance in the first pass.
                if !every_hour {
                    if let LocalResult::Ambiguous(first, _) = tz.from_local_datetime(&local) {
                        if first.with_timezone(&Utc) != cand {
                            cand = cand.checked_add_signed(Duration::minutes(1))?;
                            continue;
                        }
                    }
                }
                if !self.minutes.contains(&local.minute()) {
                    cand = cand.checked_add_signed(Duration::minutes(1))?;
                    continue;
                }
                return Some(cand);
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
                let a = a
                    .parse::<u32>()
                    .map_err(|_| format!("bad range start '{a}'"))?;
                let b = b
                    .parse::<u32>()
                    .map_err(|_| format!("bad range end '{b}'"))?;
                (a, b)
            } else {
                let v = base
                    .parse::<u32>()
                    .map_err(|_| format!("bad value '{base}'"))?;
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
        assert!(!is_due(
            &s,
            Some(utc(2026, 6, 26, 3, 1)),
            utc(2026, 6, 26, 9, 0),
            Tz::UTC
        ));
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
        assert!(validate(&json!({"cadence":"cron","expr":"0 9 * * 9"})).is_err());
        // dow > 7
        // Well-formed but impossible (Feb 31) → rejected; Feb 29 is fine.
        assert!(validate(&json!({"cadence":"cron","expr":"0 0 31 2 *"})).is_err());
        assert!(validate(&json!({"cadence":"cron","expr":"0 0 29 2 *"})).is_ok());
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
        assert!(is_due(
            &s,
            Some(utc(2026, 6, 25, 9, 0)),
            utc(2026, 6, 26, 9, 1),
            Tz::UTC
        ));
        // Now today 08:59 → not yet.
        assert!(!is_due(
            &s,
            Some(utc(2026, 6, 25, 9, 0)),
            utc(2026, 6, 26, 8, 59),
            Tz::UTC
        ));
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
        assert_eq!(
            sched.next_after(utc(2026, 6, 26, 10, 0), Tz::UTC).unwrap(),
            utc(2026, 6, 26, 17, 0)
        );
    }

    // --- DST (Asia/Jerusalem falls back Sun 2026-10-25 02:00 IDT→01:00 IST and
    // springs forward Fri 2026-03-27 02:00 IST→03:00 IDT; the EU on the 25th /
    // 29th at 01:00Z; New York falls back 2026-11-01 at 06:00Z) ---

    fn jlm() -> Tz {
        "Asia/Jerusalem".parse().unwrap()
    }

    #[test]
    fn cron_survives_the_fall_back_hour() {
        // Regression: the hour skip used to cross the ambiguous 01:00 and
        // `with_minute(0)` returned None — the schedule never fired again.
        let sched = cron::Schedule::parse("0 9 * * *").unwrap();
        assert_eq!(
            sched.next_after(utc(2026, 10, 24, 6, 0), jlm()),
            Some(utc(2026, 10, 25, 7, 0)) // 09:00 IST (UTC+2)
        );
        let s = json!({"cadence":"cron","expr":"0 9 * * *"});
        let last = Some(utc(2026, 10, 24, 6, 0));
        assert!(!is_due(&s, last, utc(2026, 10, 25, 6, 59), jlm()));
        assert!(is_due(&s, last, utc(2026, 10, 25, 7, 1), jlm()));
        assert_eq!(
            next_run(&s, utc(2026, 10, 24, 6, 0), jlm()),
            Some(utc(2026, 10, 25, 7, 0))
        );
    }

    #[test]
    fn cron_anchor_inside_the_repeated_hour() {
        // The cursor itself sits in the second 01:xx pass (with seconds).
        let sched = cron::Schedule::parse("0 9 * * *").unwrap();
        let after = Utc.with_ymd_and_hms(2026, 10, 24, 23, 10, 30).unwrap();
        assert_eq!(
            sched.next_after(after, jlm()),
            Some(utc(2026, 10, 25, 7, 0))
        );
    }

    #[test]
    fn cron_fixed_time_in_the_repeated_hour_fires_once() {
        let sched = cron::Schedule::parse("30 1 * * *").unwrap();
        // First pass: 01:30 IDT == 22:30Z.
        assert_eq!(
            sched.next_after(utc(2026, 10, 24, 22, 0), jlm()),
            Some(utc(2026, 10, 24, 22, 30))
        );
        // After it ran — even when the run finished inside the second pass —
        // the repeated 01:30 IST is skipped; next is tomorrow 01:30 IST.
        assert_eq!(
            sched.next_after(utc(2026, 10, 24, 22, 30), jlm()),
            Some(utc(2026, 10, 25, 23, 30))
        );
        assert_eq!(
            sched.next_after(utc(2026, 10, 24, 23, 10), jlm()),
            Some(utc(2026, 10, 25, 23, 30))
        );
    }

    #[test]
    fn cron_wildcard_hour_runs_in_real_time_through_fall_back() {
        let sched = cron::Schedule::parse("0 * * * *").unwrap();
        // 01:00 IDT (22:00Z) → the repeated 01:00 IST (23:00Z), an hour later.
        assert_eq!(
            sched.next_after(utc(2026, 10, 24, 22, 0), jlm()),
            Some(utc(2026, 10, 24, 23, 0))
        );
    }

    #[test]
    fn cron_time_in_spring_forward_gap_fires_right_after_it() {
        let sched = cron::Schedule::parse("30 2 * * *").unwrap();
        // 02:30 does not exist on 2026-03-27 → fires at the jump (03:00 IDT).
        assert_eq!(
            sched.next_after(utc(2026, 3, 26, 12, 0), jlm()),
            Some(utc(2026, 3, 27, 0, 0))
        );
        // …once: the next is 02:30 IDT the day after.
        assert_eq!(
            sched.next_after(utc(2026, 3, 27, 0, 0), jlm()),
            Some(utc(2026, 3, 27, 23, 30))
        );
        // A time outside the gap is unaffected; a wildcard-hour job just keeps
        // its real-time cadence (no extra catch-up fire).
        let nine = cron::Schedule::parse("0 9 * * *").unwrap();
        assert_eq!(
            nine.next_after(utc(2026, 3, 26, 12, 0), jlm()),
            Some(utc(2026, 3, 27, 6, 0))
        );
        let q = cron::Schedule::parse("*/15 * * * *").unwrap();
        assert_eq!(
            q.next_after(utc(2026, 3, 26, 23, 50), jlm()),
            Some(utc(2026, 3, 27, 0, 0))
        );
    }

    #[test]
    fn cron_europe_and_us_both_directions() {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        let sched = cron::Schedule::parse("30 2 * * *").unwrap();
        // Spring forward 2026-03-29: 02:30 is in the gap → at the jump.
        assert_eq!(
            sched.next_after(utc(2026, 3, 28, 12, 0), berlin),
            Some(utc(2026, 3, 29, 1, 0))
        );
        // Fall back 2026-10-25: the first 02:30 (CEST) only, then 02:30 CET
        // the next day.
        assert_eq!(
            sched.next_after(utc(2026, 10, 24, 12, 0), berlin),
            Some(utc(2026, 10, 25, 0, 30))
        );
        assert_eq!(
            sched.next_after(utc(2026, 10, 25, 0, 30), berlin),
            Some(utc(2026, 10, 26, 1, 30))
        );
        let london: Tz = "Europe/London".parse().unwrap();
        let nine = cron::Schedule::parse("0 9 * * *").unwrap();
        assert_eq!(
            nine.next_after(utc(2026, 10, 24, 8, 0), london),
            Some(utc(2026, 10, 25, 9, 0))
        );
        let ny: Tz = "America/New_York".parse().unwrap();
        assert_eq!(
            nine.next_after(utc(2026, 10, 31, 13, 0), ny),
            Some(utc(2026, 11, 1, 14, 0))
        );
    }

    #[test]
    fn cron_catches_up_once_after_downtime_across_dst() {
        let s = json!({"cadence":"cron","expr":"0 9 * * *"});
        // Down from before the fall-back until the 27th: due once…
        assert!(is_due(
            &s,
            Some(utc(2026, 10, 24, 6, 0)),
            utc(2026, 10, 27, 12, 0),
            jlm()
        ));
        // …and after that catch-up run the cursor moves on to tomorrow 09:00.
        let ran = Some(utc(2026, 10, 27, 12, 0));
        assert!(!is_due(&s, ran, utc(2026, 10, 27, 18, 0), jlm()));
        assert_eq!(
            next_run(&s, utc(2026, 10, 27, 12, 0), jlm()),
            Some(utc(2026, 10, 28, 7, 0))
        );
    }

    #[test]
    fn cron_first_fire_catch_up_uses_creation_time() {
        // Created Sunday; asleep through Monday 09:00; wakes at 10:00.
        let s = json!({"cadence":"cron","expr":"0 9 * * 1"});
        let created = Some(utc(2026, 6, 28, 12, 0));
        let now = utc(2026, 6, 29, 10, 0);
        assert!(!is_due(&s, None, now, Tz::UTC));
        assert!(is_due_since(&s, None, created, now, Tz::UTC));
        // Before the first fire → not due.
        assert!(!is_due_since(
            &s,
            None,
            created,
            utc(2026, 6, 29, 8, 0),
            Tz::UTC
        ));
        // The look-back is bounded: a months-old never-run monthly cron does
        // not fire on a random day.
        let monthly = json!({"cadence":"cron","expr":"0 9 1 * *"});
        assert!(!is_due_since(
            &monthly,
            None,
            Some(utc(2026, 5, 1, 0, 0)),
            utc(2026, 6, 20, 12, 0),
            Tz::UTC
        ));
    }

    #[test]
    fn daily_fires_once_in_the_repeated_hour() {
        let s = json!({"cadence":"daily","at":"01:30"});
        // The earlier 01:30 (IDT, 22:30Z) is the slot.
        assert!(is_due(
            &s,
            Some(utc(2026, 10, 23, 22, 31)),
            utc(2026, 10, 24, 22, 45),
            jlm()
        ));
        // Ran in the first pass → not again in the second pass.
        assert!(!is_due(
            &s,
            Some(utc(2026, 10, 24, 22, 31)),
            utc(2026, 10, 24, 23, 40),
            jlm()
        ));
    }

    #[test]
    fn daily_in_spring_forward_gap_fires_at_the_jump() {
        let s = json!({"cadence":"daily","at":"02:30"});
        assert_eq!(
            next_run(&s, utc(2026, 3, 26, 12, 0), jlm()),
            Some(utc(2026, 3, 27, 0, 0))
        );
        assert!(is_due(
            &s,
            Some(utc(2026, 3, 26, 0, 5)),
            utc(2026, 3, 27, 0, 1),
            jlm()
        ));
    }

    #[test]
    fn daily_and_weekly_next_run_keep_wall_clock_across_dst() {
        // Was `today + 1 day` in UTC → 06:00Z (08:00 IST), an hour early.
        let d = json!({"cadence":"daily","at":"09:00"});
        assert_eq!(
            next_run(&d, utc(2026, 10, 24, 7, 0), jlm()),
            Some(utc(2026, 10, 25, 7, 0))
        );
        // Sunday (weekday 6) 09:00, from after this Sunday's slot.
        let w = json!({"cadence":"weekly","at":"09:00","weekday":6});
        assert_eq!(
            next_run(&w, utc(2026, 10, 18, 7, 0), jlm()),
            Some(utc(2026, 10, 25, 7, 0))
        );
    }
}
