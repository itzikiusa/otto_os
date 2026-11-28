//! Scheduled active checks (Nuclei) run only under a person's explicit grant.
//!
//! A grant is a row that names everything it covers: the exact confirmed
//! target, the loopback addresses it resolved to, the exact schedule row, the
//! template policy compiled into this build and the person it runs as. It
//! expires after [`GRANT_DAYS`]. [`check`] compares it with live state every
//! time it is used — when a due job reaches the active part and again right
//! before Nuclei spawns, with the addresses the proxy will dial. Any mismatch
//! refuses the active part only; a pausing refusal ([`Refusal::pauses`]) marks
//! the grant invalid for good, so only a person can allow it again. The table
//! is new in this build, so no older build can read or move it (DATA-1).

use std::net::SocketAddr;

use chrono::{DateTime, Duration, Utc};
use otto_core::{cadence, Result};
use otto_state::DbPool;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::passive::Target;
use crate::schedule::ScheduleRow;
use crate::store::{db_error, encode};

/// A grant lasts this long; renewing is allowing again.
pub const GRANT_DAYS: i64 = 30;

/// Stable id of the active-check policy compiled into this build: the
/// template suites, excluded tags, severities and request rate Nuclei runs
/// with. Changing any of them changes the id, which pauses every grant.
pub fn template_policy() -> String {
    let mut hash = Sha256::new();
    for part in crate::dast::DEV_TEMPLATE_SUBDIRS {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    for part in [
        crate::dast::EXCLUDED_TAGS,
        crate::dast::SAFE_SEVERITIES,
        &crate::dast::RATE_LIMIT_PER_SEC.to_string(),
        "no-interactsh",
    ] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    format!("exposures-{}", &hex::encode(hash.finalize())[..12])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct ActiveGrant {
    pub granted_by: String,
    pub granted_at: String,
    pub expires_at: String,
    pub target_run_config_id: String,
    pub target_url: String,
    pub target_confirmed_at: String,
    #[sqlx(json)]
    pub addresses: Vec<String>,
    pub schedule_cron: String,
    pub schedule_timezone: String,
    pub schedule_armed_at: String,
    pub template_policy: String,
    pub invalidated_at: Option<String>,
    pub invalid_reason: Option<String>,
}

/// The exact identity a person is shown and allows. The server computes the
/// same value from live state ([`offer`]); a grant needs the two to be equal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantRequest {
    pub run_config_id: String,
    pub url: String,
    pub confirmed_at: String,
    pub addresses: Vec<String>,
    pub cron: String,
    pub timezone: String,
    pub armed_at: String,
    pub template_policy: String,
}

/// `GET .../security/dast`: the current grant, and what may be allowed now
/// (or the one condition that is missing).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ActiveSchedule {
    pub grant: Option<ActiveGrant>,
    pub offer: Option<GrantRequest>,
    pub blocked: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Expired(String),
    TargetChanged,
    ScheduleChanged,
    AddressChanged(String, String),
    UserChanged,
    NotLive,
    TemplatesChanged,
    Revoked,
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Self::Expired(on) => format!(
                "Your approval expired on {on}. Allow scheduled active checks again to continue."
            ),
            Self::TargetChanged => {
                "The audit target changed after you allowed scheduled active checks.".into()
            }
            Self::ScheduleChanged => {
                "The schedule changed after you allowed scheduled active checks.".into()
            }
            Self::AddressChanged(host, addr) => {
                format!("{host} now resolves to an address you did not approve ({addr}).")
            }
            Self::UserChanged => {
                "Scheduled active checks run only as the person who allowed them.".into()
            }
            Self::NotLive => {
                "The run configuration was not running, so active checks were skipped.".into()
            }
            Self::TemplatesChanged => {
                "This Otto version uses a different set of active checks. Review and allow again."
                    .into()
            }
            Self::Revoked => "Scheduled active checks were turned off.".into(),
        }
    }

    /// A stopped service or a revoked grant pauses nothing: the next run
    /// checks again. Every other refusal invalidates the grant for good.
    pub fn pauses(&self) -> bool {
        !matches!(self, Self::NotLive | Self::Revoked)
    }
}

/// Live state a grant is checked against.
pub struct Live<'a> {
    /// The confirmed passive target row.
    pub target: Option<&'a Target>,
    /// The URL the target's saved run configuration serves now.
    pub served_url: Option<&'a str>,
    pub running: bool,
    pub schedule: Option<&'a ScheduleRow>,
    /// The run-as person is enabled and still holds Agents: Edit.
    pub run_as_allowed: bool,
    /// The addresses the probe will dial; `None` before resolution.
    pub addresses: Option<&'a [SocketAddr]>,
    pub now: DateTime<Utc>,
}

fn parse(time: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(time)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

pub fn check(grant: &ActiveGrant, live: &Live) -> std::result::Result<(), Refusal> {
    let expires = parse(&grant.expires_at);
    if expires.is_none_or(|at| live.now >= at) {
        let on = expires.map_or_else(
            || grant.expires_at.clone(),
            |at| at.format("%-d %b %Y").to_string(),
        );
        return Err(Refusal::Expired(on));
    }
    if grant.template_policy != template_policy() {
        return Err(Refusal::TemplatesChanged);
    }
    let same_target = live.target.is_some_and(|t| {
        t.run_config_id == grant.target_run_config_id
            && t.url == grant.target_url
            && t.confirmed_at == grant.target_confirmed_at
    }) && live.served_url == Some(grant.target_url.as_str());
    if !same_target {
        return Err(Refusal::TargetChanged);
    }
    let Some(schedule) = live.schedule.filter(|s| {
        s.cron == grant.schedule_cron
            && s.timezone == grant.schedule_timezone
            && s.armed_at == grant.schedule_armed_at
    }) else {
        return Err(Refusal::ScheduleChanged);
    };
    if schedule.run_as != grant.granted_by || !live.run_as_allowed {
        return Err(Refusal::UserChanged);
    }
    if !live.running {
        return Err(Refusal::NotLive);
    }
    if let Some(bad) = live
        .addresses
        .unwrap_or_default()
        .iter()
        .find(|addr| !addr.ip().is_loopback() || !grant.addresses.contains(&addr.to_string()))
    {
        let host = Url::parse(&grant.target_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_default();
        return Err(Refusal::AddressChanged(host, bad.to_string()));
    }
    Ok(())
}

/// `localhost`, `127.0.0.0/8` or `::1`: the only hosts a schedule may probe.
pub fn loopback_host(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| {
        let host = url.host_str().unwrap_or_default();
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        bare.eq_ignore_ascii_case("localhost")
            || bare
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

/// No two of the next 50 runs closer than an hour.
pub fn cadence_ok(cron: &str, timezone: &str, now: DateTime<Utc>) -> bool {
    let spec = serde_json::json!({"cadence": "cron", "expr": cron});
    let tz = cadence::task_tz(timezone);
    let runs: Vec<_> = std::iter::successors(cadence::next_run(&spec, now, tz), |previous| {
        cadence::next_run(&spec, *previous, tz)
    })
    .take(51)
    .collect();
    runs.len() > 1
        && runs
            .windows(2)
            .all(|pair| pair[1] - pair[0] >= Duration::minutes(60))
}

/// What the person may allow now, or the one missing condition.
pub fn offer(
    target: &Target,
    served_url: Option<&str>,
    running: bool,
    schedule: Option<&ScheduleRow>,
    user_id: &str,
    addresses: &[SocketAddr],
    now: DateTime<Utc>,
) -> std::result::Result<GrantRequest, String> {
    if !target.scheduled {
        return Err("Include the passive audit in scheduled scans first.".into());
    }
    if !loopback_host(&target.url) || addresses.iter().any(|addr| !addr.ip().is_loopback()) {
        return Err("Only localhost targets can be scanned actively on a schedule. Run active scans on other targets manually.".into());
    }
    if served_url != Some(target.url.as_str()) {
        return Err("The run configuration's URL changed. Confirm the audit target again.".into());
    }
    let Some(schedule) = schedule else {
        return Err("Save a schedule first.".into());
    };
    if schedule.run_as != user_id {
        return Err(
            "Only the person this schedule runs as can allow scheduled active checks.".into(),
        );
    }
    if !cadence_ok(&schedule.cron, &schedule.timezone, now) {
        return Err("Scheduled active checks can run at most once an hour.".into());
    }
    if !running {
        return Err("Start the saved run configuration first.".into());
    }
    let mut addresses: Vec<String> = addresses.iter().map(SocketAddr::to_string).collect();
    addresses.sort();
    addresses.dedup();
    Ok(GrantRequest {
        run_config_id: target.run_config_id.clone(),
        url: target.url.clone(),
        confirmed_at: target.confirmed_at.clone(),
        addresses,
        cron: schedule.cron.clone(),
        timezone: schedule.timezone.clone(),
        armed_at: schedule.armed_at.clone(),
        template_policy: template_policy(),
    })
}

pub async fn get(pool: &DbPool, project: &str) -> Result<Option<ActiveGrant>> {
    sqlx::query_as(
        "SELECT granted_by, granted_at, expires_at, target_run_config_id, target_url,
                target_confirmed_at, addresses, schedule_cron, schedule_timezone,
                schedule_armed_at, template_policy, invalidated_at, invalid_reason
         FROM ide_security_active_grants WHERE project_id = ?",
    )
    .bind(project)
    .fetch_optional(pool)
    .await
    .map_err(db_error)
}

/// Stores a fresh grant for `request`, replacing any earlier one.
pub async fn grant(
    pool: &DbPool,
    project: &str,
    request: GrantRequest,
    granted_by: &str,
    now: DateTime<Utc>,
) -> Result<ActiveGrant> {
    let stamp = |t: DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let grant = ActiveGrant {
        granted_by: granted_by.into(),
        granted_at: stamp(now),
        expires_at: stamp(now + Duration::days(GRANT_DAYS)),
        target_run_config_id: request.run_config_id,
        target_url: request.url,
        target_confirmed_at: request.confirmed_at,
        addresses: request.addresses,
        schedule_cron: request.cron,
        schedule_timezone: request.timezone,
        schedule_armed_at: request.armed_at,
        template_policy: request.template_policy,
        invalidated_at: None,
        invalid_reason: None,
    };
    sqlx::query(
        "INSERT OR REPLACE INTO ide_security_active_grants (project_id, granted_by, granted_at,
            expires_at, target_run_config_id, target_url, target_confirmed_at, addresses,
            schedule_cron, schedule_timezone, schedule_armed_at, template_policy)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(project)
    .bind(&grant.granted_by)
    .bind(&grant.granted_at)
    .bind(&grant.expires_at)
    .bind(&grant.target_run_config_id)
    .bind(&grant.target_url)
    .bind(&grant.target_confirmed_at)
    .bind(encode(&grant.addresses)?)
    .bind(&grant.schedule_cron)
    .bind(&grant.schedule_timezone)
    .bind(&grant.schedule_armed_at)
    .bind(&grant.template_policy)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(grant)
}

pub async fn revoke(pool: &DbPool, project: &str) -> Result<bool> {
    let done = sqlx::query("DELETE FROM ide_security_active_grants WHERE project_id = ?")
        .bind(project)
        .execute(pool)
        .await
        .map_err(db_error)?;
    Ok(done.rows_affected() == 1)
}

/// Marks the grant issued at `granted_at` invalid; true only for the call
/// that paused it (so its audit row and notice are written once).
pub async fn pause(pool: &DbPool, project: &str, granted_at: &str, reason: &str) -> Result<bool> {
    let done = sqlx::query(
        "UPDATE ide_security_active_grants SET invalidated_at = ?, invalid_reason = ?
         WHERE project_id = ? AND granted_at = ? AND invalidated_at IS NULL",
    )
    .bind(crate::store::now())
    .bind(reason)
    .bind(project)
    .bind(granted_at)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(done.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::passive::{Candidate, ConfirmTarget};
    use crate::ScheduleInput;

    fn now() -> DateTime<Utc> {
        "2026-11-27T10:00:00Z".parse().unwrap()
    }

    fn target() -> Target {
        Target {
            run_config_id: "cfg".into(),
            url: "http://localhost:4200/".into(),
            allow_public: false,
            scheduled: true,
            confirmed_at: "2026-11-27T09:00:00.000Z".into(),
        }
    }

    fn schedule() -> ScheduleRow {
        ScheduleRow {
            cron: "0 2 * * 1-5".into(),
            timezone: "Europe/Berlin".into(),
            armed_at: "2026-11-27T09:30:00.000Z".into(),
            run_as: "person".into(),
        }
    }

    fn loopback() -> Vec<SocketAddr> {
        vec![
            "[::1]:4200".parse().unwrap(),
            "127.0.0.1:4200".parse().unwrap(),
        ]
    }

    fn granted() -> ActiveGrant {
        let request = offer(
            &target(),
            Some("http://localhost:4200/"),
            true,
            Some(&schedule()),
            "person",
            &loopback(),
            now(),
        )
        .unwrap();
        ActiveGrant {
            granted_by: "person".into(),
            granted_at: "2026-11-27T10:00:00.000Z".into(),
            expires_at: "2026-12-27T10:00:00.000Z".into(),
            target_run_config_id: request.run_config_id,
            target_url: request.url,
            target_confirmed_at: request.confirmed_at,
            addresses: request.addresses,
            schedule_cron: request.cron,
            schedule_timezone: request.timezone,
            schedule_armed_at: request.armed_at,
            template_policy: request.template_policy,
            invalidated_at: None,
            invalid_reason: None,
        }
    }

    #[test]
    fn every_part_of_the_grant_is_checked_against_live_state() {
        let grant = granted();
        let (target, schedule, addrs) = (target(), schedule(), loopback());
        let live = || Live {
            target: Some(&target),
            served_url: Some("http://localhost:4200/"),
            running: true,
            schedule: Some(&schedule),
            run_as_allowed: true,
            addresses: Some(&addrs),
            now: now(),
        };
        assert_eq!(check(&grant, &live()), Ok(()));

        let reconfirmed = Target {
            confirmed_at: "2026-11-28T09:00:00.000Z".into(),
            ..target.clone()
        };
        let other_config = Target {
            run_config_id: "other".into(),
            ..target.clone()
        };
        let rearmed = ScheduleRow {
            armed_at: "2026-11-28T09:30:00.000Z".into(),
            ..schedule.clone()
        };
        let retimed = ScheduleRow {
            cron: "0 3 * * 1-5".into(),
            ..schedule.clone()
        };
        let other_person = ScheduleRow {
            run_as: "someone-else".into(),
            ..schedule.clone()
        };
        let lan: Vec<SocketAddr> = vec!["192.168.1.10:4200".parse().unwrap()];
        let other_port: Vec<SocketAddr> = vec!["127.0.0.1:4300".parse().unwrap()];
        let cases: Vec<(Live, Refusal)> = vec![
            (
                Live {
                    now: "2026-12-27T10:00:00Z".parse().unwrap(),
                    ..live()
                },
                Refusal::Expired("27 Dec 2026".into()),
            ),
            (
                Live {
                    target: None,
                    ..live()
                },
                Refusal::TargetChanged,
            ),
            (
                Live {
                    target: Some(&reconfirmed),
                    ..live()
                },
                Refusal::TargetChanged,
            ),
            (
                Live {
                    target: Some(&other_config),
                    ..live()
                },
                Refusal::TargetChanged,
            ),
            (
                Live {
                    served_url: Some("http://localhost:4300/"),
                    ..live()
                },
                Refusal::TargetChanged,
            ),
            (
                Live {
                    schedule: None,
                    ..live()
                },
                Refusal::ScheduleChanged,
            ),
            (
                Live {
                    schedule: Some(&rearmed),
                    ..live()
                },
                Refusal::ScheduleChanged,
            ),
            (
                Live {
                    schedule: Some(&retimed),
                    ..live()
                },
                Refusal::ScheduleChanged,
            ),
            (
                Live {
                    schedule: Some(&other_person),
                    ..live()
                },
                Refusal::UserChanged,
            ),
            (
                Live {
                    run_as_allowed: false,
                    ..live()
                },
                Refusal::UserChanged,
            ),
            (
                Live {
                    running: false,
                    ..live()
                },
                Refusal::NotLive,
            ),
            (
                Live {
                    addresses: Some(&lan),
                    ..live()
                },
                Refusal::AddressChanged("localhost".into(), "192.168.1.10:4200".into()),
            ),
            (
                Live {
                    addresses: Some(&other_port),
                    ..live()
                },
                Refusal::AddressChanged("localhost".into(), "127.0.0.1:4300".into()),
            ),
        ];
        for (live, want) in cases {
            assert_eq!(check(&grant, &live), Err(want));
        }
        let old_policy = ActiveGrant {
            template_policy: "exposures-v0".into(),
            ..grant.clone()
        };
        assert_eq!(check(&old_policy, &live()), Err(Refusal::TemplatesChanged));
        // Only a stopped service or a revoke leaves the grant usable later.
        assert!(!Refusal::NotLive.pauses() && !Refusal::Revoked.pauses());
        assert!(Refusal::ScheduleChanged.pauses() && Refusal::TargetChanged.pauses());
    }

    #[test]
    fn the_template_policy_id_is_pinned_to_the_compiled_suites() {
        // Changing the suites, tags, severities or rate changes this id and so
        // pauses every grant: update it only together with the copy that tells
        // people what a grant allows.
        assert_eq!(template_policy(), "exposures-a3bc029970bb");
    }

    #[test]
    fn offers_only_loopback_targets_on_an_hourly_or_slower_schedule() {
        let (target, schedule) = (target(), schedule());
        let ok = |t: &Target, s: &ScheduleRow, addrs: &[SocketAddr]| {
            offer(
                t,
                Some(t.url.as_str()),
                true,
                Some(s),
                "person",
                addrs,
                now(),
            )
        };
        let request = ok(&target, &schedule, &loopback()).unwrap();
        assert_eq!(request.addresses, ["127.0.0.1:4200", "[::1]:4200"]);
        assert_eq!(request.armed_at, schedule.armed_at);

        let lan = Target {
            url: "http://192.168.1.10:8080/".into(),
            ..target.clone()
        };
        assert!(ok(&lan, &schedule, &[])
            .unwrap_err()
            .starts_with("Only localhost targets"));
        let rebound: Vec<SocketAddr> = vec!["10.0.0.5:4200".parse().unwrap()];
        assert!(ok(&target, &schedule, &rebound)
            .unwrap_err()
            .starts_with("Only localhost targets"));
        for cron in ["*/15 * * * *", "0,30 * * * *", "0 9,10 * * *"] {
            let fast = ScheduleRow {
                cron: cron.into(),
                ..schedule.clone()
            };
            if cron == "0 9,10 * * *" {
                assert!(ok(&target, &fast, &loopback()).is_ok(), "{cron}");
            } else {
                assert_eq!(
                    ok(&target, &fast, &loopback()).unwrap_err(),
                    "Scheduled active checks can run at most once an hour.",
                    "{cron}"
                );
            }
        }
        let passive_off = Target {
            scheduled: false,
            ..target.clone()
        };
        assert!(ok(&passive_off, &schedule, &loopback()).is_err());
        assert!(offer(
            &target,
            Some(target.url.as_str()),
            true,
            Some(&schedule),
            "someone-else",
            &loopback(),
            now()
        )
        .is_err());
        assert!(offer(
            &target,
            Some(target.url.as_str()),
            false,
            Some(&schedule),
            "person",
            &loopback(),
            now()
        )
        .is_err());
        assert!(offer(
            &target,
            Some(target.url.as_str()),
            true,
            None,
            "person",
            &loopback(),
            now()
        )
        .is_err());
        assert!(loopback_host("http://127.5.0.1:3000/") && loopback_host("http://[::1]:3000/"));
        assert!(
            !loopback_host("http://localhost.example.org/") && !loopback_host("http://10.0.0.2/")
        );
    }

    /// DATA-1: roll back to rc.5 (which never reads this table), let its own
    /// writers re-confirm the target and re-save the schedule, then come
    /// forward again: the grant must no longer match either.
    #[tokio::test]
    async fn a_rollback_writer_cannot_carry_a_grant_to_a_new_target_or_schedule() {
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO ide_named_projects (id, owner_user_id) VALUES ('p', 'person')")
            .execute(&pool)
            .await
            .unwrap();
        let candidate = |url: &str| Candidate {
            run_config_id: "cfg".into(),
            run_config_name: "Web".into(),
            url: url.into(),
            public: false,
        };
        let target = crate::passive::confirm(
            &pool,
            "p",
            candidate("http://127.0.0.1:4200/"),
            ConfirmTarget {
                run_config_id: "cfg".into(),
                url: "http://127.0.0.1:4200/".into(),
                allow_public: false,
                scheduled: true,
            },
        )
        .await
        .unwrap();
        crate::schedule::save(
            &pool,
            "p",
            "person",
            Some(ScheduleInput {
                cron: "0 2 * * *".into(),
                timezone: "UTC".into(),
            }),
        )
        .await
        .unwrap();
        let row = crate::schedule::row(&pool, "p").await.unwrap().unwrap();
        let addrs: Vec<SocketAddr> = vec!["127.0.0.1:4200".parse().unwrap()];
        let request = offer(
            &target,
            Some(target.url.as_str()),
            true,
            Some(&row),
            "person",
            &addrs,
            Utc::now(),
        )
        .unwrap();
        grant(&pool, "p", request, "person", Utc::now())
            .await
            .unwrap();

        async fn state(pool: &DbPool) -> (ActiveGrant, Target, ScheduleRow) {
            (
                get(pool, "p").await.unwrap().unwrap(),
                crate::passive::get(pool, "p").await.unwrap().unwrap(),
                crate::schedule::row(pool, "p").await.unwrap().unwrap(),
            )
        }
        let live = |target: &Target, schedule: &ScheduleRow, grant: &ActiveGrant| {
            let live = Live {
                target: Some(target),
                served_url: Some(target.url.as_str()),
                running: true,
                schedule: Some(schedule),
                run_as_allowed: true,
                addresses: Some(&addrs),
                now: Utc::now(),
            };
            check(grant, &live)
        };
        let (g, t, s) = state(&pool).await;
        assert_eq!(live(&t, &s, &g), Ok(()));

        // rc.5's schedule writer (schedule.rs at 1cc7d2465) re-saving a cron.
        sqlx::query(
            "INSERT INTO ide_security_schedules (project_id, cron, timezone, armed_at, armed_by)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(project_id) DO UPDATE SET cron = excluded.cron, timezone = excluded.timezone,
             armed_at = excluded.armed_at, armed_by = excluded.armed_by, last_slot = NULL
             WHERE cron != excluded.cron OR timezone != excluded.timezone OR armed_by != excluded.armed_by",
        )
        .bind("p")
        .bind("0 3 * * *")
        .bind("UTC")
        .bind("2030-01-01T00:00:00.000Z")
        .bind("person")
        .execute(&pool)
        .await
        .unwrap();
        let (g, t, s) = state(&pool).await;
        assert_eq!(live(&t, &s, &g), Err(Refusal::ScheduleChanged));

        // rc.5's confirmation writer (passive.rs at 1cc7d2465), confirming
        // the very same URL again: a new confirmation is a new identity.
        sqlx::query(
            "INSERT INTO ide_security_passive_targets (project_id, run_config_id, url, allow_public, scheduled, scheduled_active, confirmed_at)
             VALUES (?, ?, ?, ?, ?, 0, ?) ON CONFLICT(project_id) DO UPDATE SET run_config_id = excluded.run_config_id,
             url = excluded.url, allow_public = excluded.allow_public, scheduled = excluded.scheduled,
             scheduled_active = 0, confirmed_at = excluded.confirmed_at",
        )
        .bind("p")
        .bind("cfg")
        .bind(&g.target_url)
        .bind(false)
        .bind(true)
        .bind("2030-01-01T00:00:00.000Z")
        .execute(&pool)
        .await
        .unwrap();
        let (g, t, s) = state(&pool).await;
        assert_eq!(live(&t, &s, &g), Err(Refusal::TargetChanged));
    }

    #[tokio::test]
    async fn a_pause_is_recorded_once_and_a_revoke_removes_the_grant() {
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO ide_named_projects (id) VALUES ('p')")
            .execute(&pool)
            .await
            .unwrap();
        let g = granted();
        let request = GrantRequest {
            run_config_id: g.target_run_config_id.clone(),
            url: g.target_url.clone(),
            confirmed_at: g.target_confirmed_at.clone(),
            addresses: g.addresses.clone(),
            cron: g.schedule_cron.clone(),
            timezone: g.schedule_timezone.clone(),
            armed_at: g.schedule_armed_at.clone(),
            template_policy: g.template_policy.clone(),
        };
        let stored = grant(&pool, "p", request, "person", now()).await.unwrap();
        assert_eq!(stored.expires_at, "2026-12-27T10:00:00.000Z");
        assert_eq!(get(&pool, "p").await.unwrap(), Some(stored.clone()));
        let reason = Refusal::ScheduleChanged.message();
        assert!(pause(&pool, "p", &stored.granted_at, &reason)
            .await
            .unwrap());
        assert!(!pause(&pool, "p", &stored.granted_at, &reason)
            .await
            .unwrap());
        let paused = get(&pool, "p").await.unwrap().unwrap();
        assert_eq!(paused.invalid_reason.as_deref(), Some(reason.as_str()));
        assert!(revoke(&pool, "p").await.unwrap());
        assert_eq!(get(&pool, "p").await.unwrap(), None);
    }
}
