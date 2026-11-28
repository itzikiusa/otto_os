use std::future::Future;

use chrono::{DateTime, Utc};
use otto_core::domain::{Capability, Feature, User};
use otto_core::{cadence, Error, Result};
use otto_state::{DbPool, GrantsRepo, UsersRepo};
use serde_json::{json, Value};

use crate::{store, ScanSummary, Schedule, ScheduleInput};

#[derive(sqlx::FromRow)]
struct ScheduledProject {
    project_id: String,
    user_id: String,
    cron: String,
    timezone: String,
    armed_at: String,
    last_slot: Option<String>,
}

impl ScheduledProject {
    async fn settle(&self, pool: &DbPool, fire: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "UPDATE ide_security_schedules SET last_slot = ?
             WHERE project_id = ? AND armed_at = ? AND cron = ? AND timezone = ?",
        )
        .bind(fire.to_rfc3339())
        .bind(&self.project_id)
        .bind(&self.armed_at)
        .bind(&self.cron)
        .bind(&self.timezone)
        .execute(pool)
        .await
        .map_err(store::db_error)?;
        Ok(())
    }

    fn occurrence(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let parse = |value: &str| {
            DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|t| t.with_timezone(&Utc))
        };
        let cursor = self.last_slot.as_deref().and_then(parse);
        let armed = parse(&self.armed_at);
        let spec = spec(&self.cron);
        let timezone = cadence::task_tz(&self.timezone);
        cadence::is_due_since(&spec, cursor, armed, now, timezone)
            .then(|| cadence::served_occurrence(&spec, cursor, armed, now, timezone))
    }
}

fn spec(cron: &str) -> Value {
    json!({"cadence": "cron", "expr": cron})
}

pub fn preview(input: ScheduleInput, now: DateTime<Utc>) -> Result<Schedule> {
    let cron = input.cron.trim().to_string();
    let timezone = input.timezone.trim().to_string();
    if cron.len() > 128 || timezone.parse::<chrono_tz::Tz>().is_err() {
        return Err(Error::Invalid(
            "Enter a five-field cron and an IANA timezone.".into(),
        ));
    }
    let spec = spec(&cron);
    cadence::validate(&spec)?;
    Ok(view(cron, timezone, now))
}

fn view(cron: String, timezone: String, now: DateTime<Utc>) -> Schedule {
    let tz = cadence::task_tz(&timezone);
    let spec = spec(&cron);
    let next_runs = std::iter::successors(cadence::next_run(&spec, now, tz), |previous| {
        cadence::next_run(&spec, *previous, tz)
    })
    .take(5)
    .map(|time| time.to_rfc3339())
    .collect();
    Schedule {
        cron,
        timezone,
        next_runs,
    }
}

pub async fn get(pool: &DbPool, project: &str) -> Result<Option<Schedule>> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT s.cron, s.timezone FROM ide_security_schedules s
         JOIN ide_named_projects p ON p.id = s.project_id
         WHERE p.id = ? AND (p.owner_user_id != '' OR s.armed_by != '')",
    )
    .bind(project)
    .fetch_optional(pool)
    .await
    .map_err(store::db_error)?;
    Ok(row.map(|(cron, timezone)| view(cron, timezone, Utc::now())))
}

/// The exact saved schedule row a scheduled active grant binds to, with the
/// person it runs as (the project owner, else whoever saved it).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ScheduleRow {
    pub cron: String,
    pub timezone: String,
    pub armed_at: String,
    pub run_as: String,
}

pub async fn row(pool: &DbPool, project: &str) -> Result<Option<ScheduleRow>> {
    sqlx::query_as(
        "SELECT s.cron, s.timezone, s.armed_at,
                COALESCE(NULLIF(p.owner_user_id, ''), s.armed_by) AS run_as
         FROM ide_security_schedules s JOIN ide_named_projects p ON p.id = s.project_id
         WHERE p.id = ? AND (p.owner_user_id != '' OR s.armed_by != '')",
    )
    .bind(project)
    .fetch_optional(pool)
    .await
    .map_err(store::db_error)
}

pub async fn save(
    pool: &DbPool,
    project: &str,
    user_id: &str,
    input: Option<ScheduleInput>,
) -> Result<Option<Schedule>> {
    if let Some(input) = input {
        let schedule = preview(input, Utc::now())?;
        sqlx::query(
            "INSERT INTO ide_security_schedules (project_id, cron, timezone, armed_at, armed_by)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(project_id) DO UPDATE SET cron = excluded.cron, timezone = excluded.timezone,
             armed_at = excluded.armed_at, armed_by = excluded.armed_by, last_slot = NULL
             WHERE cron != excluded.cron OR timezone != excluded.timezone OR armed_by != excluded.armed_by",
        ).bind(project).bind(&schedule.cron).bind(&schedule.timezone).bind(store::now()).bind(user_id)
            .execute(pool).await.map_err(store::db_error)?;
        Ok(Some(schedule))
    } else {
        sqlx::query("DELETE FROM ide_security_schedules WHERE project_id = ?")
            .bind(project)
            .execute(pool)
            .await
            .map_err(store::db_error)?;
        Ok(None)
    }
}

async fn due(pool: &DbPool, now: DateTime<Utc>) -> Result<Vec<(ScheduledProject, DateTime<Utc>)>> {
    let rows = sqlx::query_as::<_, ScheduledProject>(
        "SELECT s.project_id, COALESCE(NULLIF(p.owner_user_id, ''), s.armed_by) AS user_id,
                s.cron, s.timezone, s.armed_at, s.last_slot
         FROM ide_security_schedules s JOIN ide_named_projects p ON p.id = s.project_id
         WHERE p.owner_user_id != '' OR s.armed_by != ''",
    )
    .fetch_all(pool)
    .await
    .map_err(store::db_error)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.occurrence(now).map(|fire| (row, fire)))
        .collect())
}

pub async fn tick<F, Fut>(pool: &DbPool, now: DateTime<Utc>, enqueue: F) -> Result<()>
where
    F: Fn(User, String) -> Fut,
    Fut: Future<Output = Result<ScanSummary>>,
{
    let users = UsersRepo::new(pool.clone());
    let grants = GrantsRepo::new(pool.clone());
    for (schedule, fire) in due(pool, now).await? {
        let user = match users.get(&schedule.user_id).await {
            Ok(user) => user,
            Err(Error::NotFound(_)) => continue,
            Err(error) => return Err(error),
        };
        if user.disabled || grants.capability_of(&user, Feature::Agents).await? < Capability::Edit {
            continue;
        }
        let project = schedule.project_id.clone();
        match enqueue(user, project.clone()).await {
            Ok(_) => schedule.settle(pool, fire).await?,
            Err(Error::Conflict(_)) => {}
            Err(error) => tracing::warn!(project, %error, "scheduled security scan deferred"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unidentified_legacy_schedule_is_not_presented_as_armed() {
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO ide_named_projects (id) VALUES ('legacy')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO ide_security_schedules (project_id, cron, timezone, armed_at)
             VALUES ('legacy', '*/5 * * * *', 'UTC', '2026-11-27T05:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let now = "2026-11-27T05:05:00Z".parse().unwrap();
        assert!(due(&pool, now).await.unwrap().is_empty());
        assert!(get(&pool, "legacy").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn tick_runs_legacy_as_the_configuring_user_and_keeps_existing_project_owners() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let pool = otto_state::db::test_pool().await;
        let owner = UsersRepo::new(pool.clone())
            .create("owner", "unused", "Owner", true)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ide_named_projects (id, owner_user_id) VALUES ('legacy', ''), ('owned', ?)")
            .bind(&owner.id).execute(&pool).await.unwrap();
        save(
            &pool,
            "legacy",
            &owner.id,
            Some(ScheduleInput {
                cron: "*/5 * * * *".into(),
                timezone: "UTC".into(),
            }),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE ide_security_schedules SET armed_at = '2026-11-27T05:00:00Z'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO ide_security_schedules (project_id, cron, timezone, armed_at)
            VALUES ('owned', '*/5 * * * *', 'UTC', '2026-11-27T05:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(get(&pool, "legacy").await.unwrap().is_some());

        let now = "2026-11-27T13:34:00Z".parse().unwrap();
        let count = AtomicUsize::new(0);
        let dispatch = |user: User, project: String| {
            assert_eq!(user.id, owner.id);
            count.fetch_add(1, Ordering::Relaxed);
            let pool = &pool;
            async move { store::insert(pool, &project, crate::ScanTrigger::Manual).await }
        };
        tick(&pool, now, dispatch).await.unwrap();
        tick(&pool, now, dispatch).await.unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 2);
        for project in ["legacy", "owned"] {
            let scan = store::list(&pool, project).await.unwrap();
            assert_eq!(scan.scans.len(), 1);
            assert_eq!(scan.scans[0].status, crate::ScanStatus::Queued);
        }
    }

    #[test]
    fn wake_uses_only_the_latest_missed_cron_slot() {
        let now = "2026-11-27T13:34:00Z".parse::<DateTime<Utc>>().unwrap();
        let mut row = ScheduledProject {
            project_id: "p".into(),
            user_id: "u".into(),
            cron: "*/15 * * * *".into(),
            timezone: "UTC".into(),
            armed_at: "2026-11-27T05:00:00Z".into(),
            last_slot: None,
        };
        let slot = row.occurrence(now).unwrap();
        assert_eq!(slot.to_rfc3339(), "2026-11-27T13:30:00+00:00");
        row.last_slot = Some(slot.to_rfc3339());
        assert!(row.occurrence(now).is_none());
        let preview = preview(
            ScheduleInput {
                cron: row.cron,
                timezone: row.timezone,
            },
            now,
        )
        .unwrap();
        assert_eq!(preview.next_runs[0], "2026-11-27T13:45:00+00:00");
    }
}
