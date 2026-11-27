use chrono::{DateTime, Utc};
use otto_core::{cadence, Error, Result};
use otto_state::DbPool;
use serde_json::{json, Value};

use crate::{store, Schedule, ScheduleInput};

#[derive(sqlx::FromRow)]
pub struct ScheduledProject {
    pub project_id: String,
    pub owner_user_id: String,
    cron: String,
    timezone: String,
    armed_at: String,
    last_slot: Option<String>,
}

impl ScheduledProject {
    pub async fn settle(&self, pool: &DbPool, fire: DateTime<Utc>) -> Result<()> {
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
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT cron, timezone FROM ide_security_schedules WHERE project_id = ?")
            .bind(project)
            .fetch_optional(pool)
            .await
            .map_err(store::db_error)?;
    Ok(row.map(|(cron, timezone)| view(cron, timezone, Utc::now())))
}

pub async fn save(
    pool: &DbPool,
    project: &str,
    input: Option<ScheduleInput>,
) -> Result<Option<Schedule>> {
    if let Some(input) = input {
        let schedule = preview(input, Utc::now())?;
        sqlx::query(
            "INSERT INTO ide_security_schedules (project_id, cron, timezone, armed_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(project_id) DO UPDATE SET cron = excluded.cron, timezone = excluded.timezone,
             armed_at = excluded.armed_at, last_slot = NULL
             WHERE cron != excluded.cron OR timezone != excluded.timezone",
        ).bind(project).bind(&schedule.cron).bind(&schedule.timezone).bind(store::now())
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

pub async fn due(
    pool: &DbPool,
    now: DateTime<Utc>,
) -> Result<Vec<(ScheduledProject, DateTime<Utc>)>> {
    let rows = sqlx::query_as::<_, ScheduledProject>(
        "SELECT s.project_id, p.owner_user_id, s.cron, s.timezone, s.armed_at, s.last_slot
         FROM ide_security_schedules s JOIN ide_named_projects p ON p.id = s.project_id",
    )
    .fetch_all(pool)
    .await
    .map_err(store::db_error)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.occurrence(now).map(|fire| (row, fire)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_uses_only_the_latest_missed_cron_slot() {
        let now = "2026-11-27T13:34:00Z".parse::<DateTime<Utc>>().unwrap();
        let mut row = ScheduledProject {
            project_id: "p".into(),
            owner_user_id: "u".into(),
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
