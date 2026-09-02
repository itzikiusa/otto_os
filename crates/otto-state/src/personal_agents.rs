//! Persistence for **Personal Agents** (migration `0112_personal_agents.sql`).
//!
//! Four surfaces: `personal_agents` (the named persistent agent), 1..N
//! `personal_agent_schedules` per agent (each with its OWN `last_run_at` cursor,
//! advanced by the engine on run completion), `personal_agent_runs` (one row per
//! execution, mirroring `scheduled_task_runs`), and the rooms trio
//! (`agent_rooms` / `agent_room_members` / `agent_room_messages`) — the only
//! agent-to-agent transport, fully persisted and user-visible. Pure storage —
//! cadence/cursor math, persona materialization and report I/O live in
//! `otto_server`. Message ids are ULIDs, so lexicographic `id > after` paging is
//! chronological.

use chrono::Utc;
use otto_core::{new_id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json};

// --- Domain --------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalAgent {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub avatar: String,
    pub soul_md: String,
    pub provider: String,
    /// Empty string = provider default model.
    pub model: String,
    /// Empty string = `data_dir/personal/<agent-id>/` (resolved by the engine).
    pub cwd: String,
    pub browser: bool,
    /// `{type: none|slack|telegram|email|webhook, chat_id?/to?/url?/subject?}` —
    /// same shape as a scheduled task's destination.
    pub delivery: Value,
    pub enabled: bool,
    /// The agent's single interactive chat session (output-only; set by the
    /// chat-session route).
    pub chat_session_id: Option<String>,
    pub created_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalAgentSchedule {
    pub id: String,
    pub agent_id: String,
    /// Existing cadence format: `{cadence: interval|daily|weekly|cron, …}`.
    pub schedule: Value,
    pub timezone: String,
    /// The run's task prompt for this schedule.
    pub directive: String,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalAgentRun {
    pub id: String,
    pub agent_id: String,
    pub schedule_id: Option<String>,
    pub workspace_id: String,
    pub status: String,
    pub trigger: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub summary: String,
    pub report_path: Option<String>,
    pub report_rel: Option<String>,
    pub delivered: bool,
    pub delivery_error: Option<String>,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub report_hash: Option<String>,
    pub attempts: i64,
    pub skipped_delivery: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRoom {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub created_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRoomMessage {
    pub id: String,
    pub room_id: String,
    /// `agent` (author_id = personal_agents.id) or `user` (author_id = users.id).
    pub author_kind: String,
    pub author_id: String,
    pub text: String,
    pub created_at: String,
}

// --- Inputs --------------------------------------------------------------

/// Fields for creating an agent. `delivery` defaults to `{"type":"none"}`.
#[derive(Clone, Debug)]
pub struct NewPersonalAgent {
    pub workspace_id: String,
    pub name: String,
    pub avatar: String,
    pub soul_md: String,
    pub provider: String,
    pub model: String,
    pub cwd: String,
    pub browser: bool,
    pub delivery: Value,
    pub enabled: bool,
    pub created_by: Option<String>,
}

impl NewPersonalAgent {
    /// Construct with every optional field at its default.
    pub fn defaults(workspace_id: String, name: String) -> Self {
        Self {
            workspace_id,
            name,
            avatar: String::new(),
            soul_md: String::new(),
            provider: "claude".into(),
            model: String::new(),
            cwd: String::new(),
            browser: false,
            delivery: serde_json::json!({"type": "none"}),
            enabled: true,
            created_by: None,
        }
    }
}

/// Partial update — every `Some` field is written (`None` leaves it unchanged).
#[derive(Clone, Debug, Default)]
pub struct PersonalAgentPatch {
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub soul_md: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub browser: Option<bool>,
    pub delivery: Option<Value>,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct NewAgentSchedule {
    pub agent_id: String,
    pub schedule: Value,
    pub timezone: String,
    pub directive: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct AgentSchedulePatch {
    pub schedule: Option<Value>,
    pub timezone: Option<String>,
    pub directive: Option<String>,
    pub enabled: Option<bool>,
}

/// Fields for opening a run row (status starts `running`).
#[derive(Clone, Debug)]
pub struct NewAgentRun {
    pub agent_id: String,
    pub schedule_id: Option<String>,
    pub workspace_id: String,
    pub trigger: String,
}

/// Terminal state for a run — filled by the engine once the agent completes
/// (success or failure) and delivery has been attempted.
#[derive(Clone, Debug, Default)]
pub struct FinishAgentRun {
    pub status: String,
    pub summary: String,
    pub report_path: Option<String>,
    pub report_rel: Option<String>,
    pub delivered: bool,
    pub delivery_error: Option<String>,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub report_hash: Option<String>,
    pub attempts: i64,
    pub skipped_delivery: bool,
}

#[derive(Clone, Debug)]
pub struct NewRoomMessage {
    pub room_id: String,
    pub author_kind: String,
    pub author_id: String,
    pub text: String,
}

// --- Row mapping ---------------------------------------------------------

fn row_to_agent(r: &sqlx::sqlite::SqliteRow) -> Result<PersonalAgent> {
    let delivery_raw: String = r.get("delivery_json");
    Ok(PersonalAgent {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        avatar: r.get("avatar"),
        soul_md: r.get("soul_md"),
        provider: r.get("provider"),
        model: r.get("model"),
        cwd: r.get("cwd"),
        browser: r.get::<i64, _>("browser") != 0,
        delivery: json(&delivery_raw).unwrap_or(Value::Null),
        enabled: r.get::<i64, _>("enabled") != 0,
        chat_session_id: r.get("chat_session_id"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

fn row_to_schedule(r: &sqlx::sqlite::SqliteRow) -> Result<PersonalAgentSchedule> {
    let sched_raw: String = r.get("schedule_json");
    Ok(PersonalAgentSchedule {
        id: r.get("id"),
        agent_id: r.get("agent_id"),
        schedule: json(&sched_raw).unwrap_or(Value::Null),
        timezone: r.get("timezone"),
        directive: r.get("directive"),
        enabled: r.get::<i64, _>("enabled") != 0,
        last_run_at: r.get("last_run_at"),
        next_run_at: r.get("next_run_at"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

fn row_to_run(r: &sqlx::sqlite::SqliteRow) -> Result<PersonalAgentRun> {
    Ok(PersonalAgentRun {
        id: r.get("id"),
        agent_id: r.get("agent_id"),
        schedule_id: r.get("schedule_id"),
        workspace_id: r.get("workspace_id"),
        status: r.get("status"),
        trigger: r.get("trigger"),
        started_at: r.get("started_at"),
        finished_at: r.get("finished_at"),
        summary: r.get("summary"),
        report_path: r.get("report_path"),
        report_rel: r.get("report_rel"),
        delivered: r.get::<i64, _>("delivered") != 0,
        delivery_error: r.get("delivery_error"),
        error: r.get("error"),
        session_id: r.get("session_id"),
        report_hash: r.get("report_hash"),
        attempts: r.get("attempts"),
        skipped_delivery: r.get::<i64, _>("skipped_delivery") != 0,
        created_at: r.get("created_at"),
    })
}

fn row_to_room(r: &sqlx::sqlite::SqliteRow) -> Result<AgentRoom> {
    Ok(AgentRoom {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

fn row_to_message(r: &sqlx::sqlite::SqliteRow) -> Result<AgentRoomMessage> {
    Ok(AgentRoomMessage {
        id: r.get("id"),
        room_id: r.get("room_id"),
        author_kind: r.get("author_kind"),
        author_id: r.get("author_id"),
        text: r.get("text"),
        created_at: r.get("created_at"),
    })
}

// --- Agents + schedules + runs -------------------------------------------

#[derive(Clone)]
pub struct PersonalAgentsRepo {
    pool: SqlitePool,
}

impl PersonalAgentsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- Agents ------------------------------------------------------------

    pub async fn create(&self, a: NewPersonalAgent) -> Result<PersonalAgent> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO personal_agents (id, workspace_id, name, avatar, soul_md, provider, \
             model, cwd, browser, delivery_json, enabled, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.workspace_id)
        .bind(&a.name)
        .bind(&a.avatar)
        .bind(&a.soul_md)
        .bind(&a.provider)
        .bind(&a.model)
        .bind(&a.cwd)
        .bind(a.browser as i64)
        .bind(a.delivery.to_string())
        .bind(a.enabled as i64)
        .bind(&a.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create personal agent"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &str) -> Result<PersonalAgent> {
        let row = sqlx::query("SELECT * FROM personal_agents WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("personal agent not found"))?;
        row_to_agent(&row)
    }

    pub async fn list_by_workspace(&self, ws: &str) -> Result<Vec<PersonalAgent>> {
        let rows = sqlx::query(
            "SELECT * FROM personal_agents WHERE workspace_id = ? ORDER BY created_at ASC",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list personal agents"))?;
        rows.iter().map(row_to_agent).collect()
    }

    pub async fn update(&self, id: &str, p: PersonalAgentPatch) -> Result<PersonalAgent> {
        sqlx::query(
            "UPDATE personal_agents SET \
               name = COALESCE(?, name), \
               avatar = COALESCE(?, avatar), \
               soul_md = COALESCE(?, soul_md), \
               provider = COALESCE(?, provider), \
               model = COALESCE(?, model), \
               cwd = COALESCE(?, cwd), \
               browser = COALESCE(?, browser), \
               delivery_json = COALESCE(?, delivery_json), \
               enabled = COALESCE(?, enabled), \
               updated_at = ? \
             WHERE id = ?",
        )
        .bind(p.name)
        .bind(p.avatar)
        .bind(p.soul_md)
        .bind(p.provider)
        .bind(p.model)
        .bind(p.cwd)
        .bind(p.browser.map(|b| b as i64))
        .bind(p.delivery.map(|v| v.to_string()))
        .bind(p.enabled.map(|b| b as i64))
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update personal agent"))?;
        self.get(id).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM personal_agents WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete personal agent"))?;
        Ok(())
    }

    /// Pin (or clear) the agent's single interactive chat session.
    pub async fn set_chat_session(&self, id: &str, session_id: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE personal_agents SET chat_session_id = ?, updated_at = ? WHERE id = ?")
            .bind(session_id)
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set personal agent chat session"))?;
        Ok(())
    }

    // -- Schedules ----------------------------------------------------------

    pub async fn create_schedule(&self, s: NewAgentSchedule) -> Result<PersonalAgentSchedule> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO personal_agent_schedules (id, agent_id, schedule_json, timezone, \
             directive, enabled, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&s.agent_id)
        .bind(s.schedule.to_string())
        .bind(&s.timezone)
        .bind(&s.directive)
        .bind(s.enabled as i64)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create personal agent schedule"))?;
        self.get_schedule(&id).await
    }

    pub async fn get_schedule(&self, id: &str) -> Result<PersonalAgentSchedule> {
        let row = sqlx::query("SELECT * FROM personal_agent_schedules WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("personal agent schedule not found"))?;
        row_to_schedule(&row)
    }

    pub async fn list_schedules(&self, agent_id: &str) -> Result<Vec<PersonalAgentSchedule>> {
        let rows = sqlx::query(
            "SELECT * FROM personal_agent_schedules WHERE agent_id = ? ORDER BY created_at ASC",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list personal agent schedules"))?;
        rows.iter().map(row_to_schedule).collect()
    }

    /// Every enabled schedule of every enabled agent — the scheduler's tick
    /// query. Returns (schedule, agent) pairs so the tick needn't re-fetch.
    pub async fn list_enabled_schedules(&self) -> Result<Vec<(PersonalAgentSchedule, PersonalAgent)>> {
        let rows = sqlx::query(
            "SELECT s.id AS s_id, a.id AS a_id FROM personal_agent_schedules s \
             JOIN personal_agents a ON a.id = s.agent_id \
             WHERE s.enabled = 1 AND a.enabled = 1",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list enabled personal agent schedules"))?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let sid: String = r.get("s_id");
            let aid: String = r.get("a_id");
            out.push((self.get_schedule(&sid).await?, self.get(&aid).await?));
        }
        Ok(out)
    }

    pub async fn update_schedule(&self, id: &str, p: AgentSchedulePatch) -> Result<PersonalAgentSchedule> {
        sqlx::query(
            "UPDATE personal_agent_schedules SET \
               schedule_json = COALESCE(?, schedule_json), \
               timezone = COALESCE(?, timezone), \
               directive = COALESCE(?, directive), \
               enabled = COALESCE(?, enabled), \
               updated_at = ? \
             WHERE id = ?",
        )
        .bind(p.schedule.map(|v| v.to_string()))
        .bind(p.timezone)
        .bind(p.directive)
        .bind(p.enabled.map(|b| b as i64))
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update personal agent schedule"))?;
        self.get_schedule(id).await
    }

    pub async fn delete_schedule(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM personal_agent_schedules WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete personal agent schedule"))?;
        Ok(())
    }

    /// Advance a schedule's cursor + display field after a run completes.
    pub async fn set_schedule_runtime(
        &self,
        id: &str,
        last_run_at: Option<&str>,
        next_run_at: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE personal_agent_schedules SET last_run_at = COALESCE(?, last_run_at), \
             next_run_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(last_run_at)
        .bind(next_run_at)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set personal agent schedule runtime"))?;
        Ok(())
    }

    // -- Runs ----------------------------------------------------------------

    pub async fn create_run(&self, r: NewAgentRun) -> Result<PersonalAgentRun> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO personal_agent_runs (id, agent_id, schedule_id, workspace_id, status, \
             trigger, started_at, summary, delivered, created_at) \
             VALUES (?, ?, ?, ?, 'running', ?, ?, '', 0, ?)",
        )
        .bind(&id)
        .bind(&r.agent_id)
        .bind(&r.schedule_id)
        .bind(&r.workspace_id)
        .bind(&r.trigger)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create personal agent run"))?;
        self.get_run(&id).await
    }

    pub async fn finish_run(&self, run_id: &str, f: FinishAgentRun) -> Result<()> {
        sqlx::query(
            "UPDATE personal_agent_runs SET status = ?, summary = ?, report_path = ?, \
             report_rel = ?, delivered = ?, delivery_error = ?, error = ?, session_id = ?, \
             report_hash = ?, attempts = ?, skipped_delivery = ?, finished_at = ? WHERE id = ?",
        )
        .bind(&f.status)
        .bind(&f.summary)
        .bind(&f.report_path)
        .bind(&f.report_rel)
        .bind(f.delivered as i64)
        .bind(&f.delivery_error)
        .bind(&f.error)
        .bind(&f.session_id)
        .bind(&f.report_hash)
        .bind(f.attempts.max(1))
        .bind(f.skipped_delivery as i64)
        .bind(fmt(Utc::now()))
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("finish personal agent run"))?;
        Ok(())
    }

    /// Persist the live session id as soon as the run's agent session is
    /// created, so the UI can Open the run live.
    pub async fn set_run_session(&self, run_id: &str, session_id: &str) -> Result<()> {
        sqlx::query("UPDATE personal_agent_runs SET session_id = ? WHERE id = ?")
            .bind(session_id)
            .bind(run_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set personal agent run session"))?;
        Ok(())
    }

    pub async fn get_run(&self, run_id: &str) -> Result<PersonalAgentRun> {
        let row = sqlx::query("SELECT * FROM personal_agent_runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("personal agent run not found"))?;
        row_to_run(&row)
    }

    pub async fn list_runs(&self, agent_id: &str, limit: i64) -> Result<Vec<PersonalAgentRun>> {
        let rows = sqlx::query(
            "SELECT * FROM personal_agent_runs WHERE agent_id = ? ORDER BY started_at DESC LIMIT ?",
        )
        .bind(agent_id)
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list personal agent runs"))?;
        rows.iter().map(row_to_run).collect()
    }

    /// The report hash of the most recent successful run for an agent (excluding
    /// a given run id) — backs notify-on-change change detection.
    pub async fn last_ok_report_hash(&self, agent_id: &str, exclude_run: &str) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT report_hash FROM personal_agent_runs WHERE agent_id = ? AND status = 'ok' \
             AND id != ? AND report_hash IS NOT NULL ORDER BY started_at DESC LIMIT 1",
        )
        .bind(agent_id)
        .bind(exclude_run)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("last ok personal agent report hash"))?;
        Ok(row.and_then(|r| r.get::<Option<String>, _>("report_hash")))
    }

    /// Delete all but the most-recent `keep` runs for an agent. Returns the
    /// `report_path`s of deleted rows so the caller can unlink the report files.
    pub async fn prune_runs(&self, agent_id: &str, keep: i64) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT id, report_path FROM personal_agent_runs WHERE agent_id = ? \
             ORDER BY started_at DESC LIMIT -1 OFFSET ?",
        )
        .bind(agent_id)
        .bind(keep.max(0))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("select prunable personal agent runs"))?;
        let mut paths = Vec::new();
        for r in &rows {
            let id: String = r.get("id");
            if let Some(p) = r.get::<Option<String>, _>("report_path") {
                paths.push(p);
            }
            let _ = sqlx::query("DELETE FROM personal_agent_runs WHERE id = ?")
                .bind(&id)
                .execute(&self.pool)
                .await
                .map_err(dberr("prune personal agent run"))?;
        }
        Ok(paths)
    }

    /// Mark every still-`running` run as `error` — called once at scheduler
    /// start to clear zombie rows left by a daemon restart. Returns the count.
    pub async fn reap_running(&self) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE personal_agent_runs SET status = 'error', \
             error = 'interrupted by daemon restart', finished_at = ? WHERE status = 'running'",
        )
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("reap running personal agent runs"))?;
        Ok(res.rows_affected())
    }
}

// --- Rooms ----------------------------------------------------------------

#[derive(Clone)]
pub struct AgentRoomsRepo {
    pool: SqlitePool,
}

impl AgentRoomsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, workspace_id: &str, name: &str, created_by: Option<&str>) -> Result<AgentRoom> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO agent_rooms (id, workspace_id, name, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(name)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create agent room"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &str) -> Result<AgentRoom> {
        let row = sqlx::query("SELECT * FROM agent_rooms WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("agent room not found"))?;
        row_to_room(&row)
    }

    pub async fn list_by_workspace(&self, ws: &str) -> Result<Vec<AgentRoom>> {
        let rows = sqlx::query("SELECT * FROM agent_rooms WHERE workspace_id = ? ORDER BY created_at ASC")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list agent rooms"))?;
        rows.iter().map(row_to_room).collect()
    }

    pub async fn rename(&self, id: &str, name: &str) -> Result<AgentRoom> {
        sqlx::query("UPDATE agent_rooms SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("rename agent room"))?;
        self.get(id).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM agent_rooms WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete agent room"))?;
        Ok(())
    }

    // -- Membership ----------------------------------------------------------

    /// Idempotent add (INSERT OR IGNORE — re-adding a member is a no-op).
    pub async fn add_member(&self, room_id: &str, agent_id: &str) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO agent_room_members (room_id, agent_id, created_at) VALUES (?, ?, ?)",
        )
        .bind(room_id)
        .bind(agent_id)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("add agent room member"))?;
        Ok(())
    }

    pub async fn remove_member(&self, room_id: &str, agent_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM agent_room_members WHERE room_id = ? AND agent_id = ?")
            .bind(room_id)
            .bind(agent_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("remove agent room member"))?;
        Ok(())
    }

    pub async fn list_members(&self, room_id: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT agent_id FROM agent_room_members WHERE room_id = ? ORDER BY created_at ASC",
        )
        .bind(room_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list agent room members"))?;
        Ok(rows.iter().map(|r| r.get("agent_id")).collect())
    }

    /// The room-tool membership check: may this agent post/read here?
    pub async fn is_member(&self, room_id: &str, agent_id: &str) -> Result<bool> {
        let row = sqlx::query("SELECT 1 AS x FROM agent_room_members WHERE room_id = ? AND agent_id = ?")
            .bind(room_id)
            .bind(agent_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("check agent room membership"))?;
        Ok(row.is_some())
    }

    // -- Messages ------------------------------------------------------------

    pub async fn add_message(&self, m: NewRoomMessage) -> Result<AgentRoomMessage> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO agent_room_messages (id, room_id, author_kind, author_id, text, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&m.room_id)
        .bind(&m.author_kind)
        .bind(&m.author_id)
        .bind(&m.text)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add agent room message"))?;
        let row = sqlx::query("SELECT * FROM agent_room_messages WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("agent room message not found"))?;
        row_to_message(&row)
    }

    /// Chronological page: messages after the `after` message, oldest first,
    /// capped at `limit`. `after = None` starts from the beginning. Ordering and
    /// the cursor use the table's monotonic `rowid` (insertion order) rather than
    /// the ULID `id` — two messages minted in the same millisecond tie on the
    /// ULID timestamp and would otherwise sort by their random suffix, i.e.
    /// non-deterministically.
    pub async fn list_messages(
        &self,
        room_id: &str,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AgentRoomMessage>> {
        let rows = sqlx::query(
            "SELECT * FROM agent_room_messages \
             WHERE room_id = ? \
               AND rowid > COALESCE((SELECT rowid FROM agent_room_messages WHERE id = ?), 0) \
             ORDER BY rowid ASC LIMIT ?",
        )
        .bind(room_id)
        .bind(after.unwrap_or(""))
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list agent room messages"))?;
        rows.iter().map(row_to_message).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn pool() -> SqlitePool {
        crate::db::test_pool().await
    }

    async fn seed_ws(pool: &SqlitePool, id: &str) {
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
            .bind(id)
            .bind("ws")
            .bind("/tmp/ws")
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
    }

    fn new_agent(ws: &str, name: &str) -> NewPersonalAgent {
        NewPersonalAgent {
            soul_md: "# You are Testy".into(),
            created_by: Some("u1".into()),
            ..NewPersonalAgent::defaults(ws.into(), name.into())
        }
    }

    #[tokio::test]
    async fn agent_crud_roundtrip() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        let a = repo.create(new_agent("ws1", "Recap")).await.unwrap();
        assert_eq!(a.name, "Recap");
        assert_eq!(a.delivery["type"], "none");
        assert!(!a.browser);
        let upd = repo
            .update(
                &a.id,
                PersonalAgentPatch {
                    name: Some("Recap 2".into()),
                    browser: Some(true),
                    model: Some("opus".into()),
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(upd.name, "Recap 2");
        assert!(upd.browser);
        assert_eq!(upd.model, "opus");
        assert!(!upd.enabled);
        assert_eq!(repo.list_by_workspace("ws1").await.unwrap().len(), 1);
        repo.set_chat_session(&a.id, Some("sess-9")).await.unwrap();
        assert_eq!(repo.get(&a.id).await.unwrap().chat_session_id.as_deref(), Some("sess-9"));
        repo.delete(&a.id).await.unwrap();
        assert!(repo.get(&a.id).await.is_err());
    }

    #[tokio::test]
    async fn schedules_have_independent_cursors() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        let a = repo.create(new_agent("ws1", "Recap")).await.unwrap();
        let daily = repo
            .create_schedule(NewAgentSchedule {
                agent_id: a.id.clone(),
                schedule: json!({"cadence":"daily","at":"09:00"}),
                timezone: "UTC".into(),
                directive: "daily recap".into(),
                enabled: true,
            })
            .await
            .unwrap();
        let fast = repo
            .create_schedule(NewAgentSchedule {
                agent_id: a.id.clone(),
                schedule: json!({"cadence":"interval","every_min":15}),
                timezone: "UTC".into(),
                directive: "needs attention?".into(),
                enabled: true,
            })
            .await
            .unwrap();
        repo.set_schedule_runtime(&fast.id, Some("2026-09-01T10:00:00+00:00"), Some("2026-09-01T10:15:00+00:00"))
            .await
            .unwrap();
        let daily2 = repo.get_schedule(&daily.id).await.unwrap();
        let fast2 = repo.get_schedule(&fast.id).await.unwrap();
        assert!(daily2.last_run_at.is_none(), "sibling cursor untouched");
        assert!(fast2.last_run_at.is_some());
        assert_eq!(repo.list_schedules(&a.id).await.unwrap().len(), 2);
        // Enabled tick sees both; disabling the agent hides both.
        assert_eq!(repo.list_enabled_schedules().await.unwrap().len(), 2);
        repo.update(&a.id, PersonalAgentPatch { enabled: Some(false), ..Default::default() })
            .await
            .unwrap();
        assert_eq!(repo.list_enabled_schedules().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn runs_finish_prune_reap() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        let a = repo.create(new_agent("ws1", "Recap")).await.unwrap();
        for i in 0..5 {
            let r = repo
                .create_run(NewAgentRun {
                    agent_id: a.id.clone(),
                    schedule_id: None,
                    workspace_id: "ws1".into(),
                    trigger: "manual".into(),
                })
                .await
                .unwrap();
            assert_eq!(r.status, "running");
            repo.finish_run(
                &r.id,
                FinishAgentRun {
                    status: "ok".into(),
                    summary: format!("run {i}"),
                    report_path: Some(format!("/x/{i}.md")),
                    report_hash: Some(format!("h{i}")),
                    attempts: 1,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        let h = repo.last_ok_report_hash(&a.id, "other").await.unwrap();
        assert_eq!(h.as_deref(), Some("h4"));
        let deleted = repo.prune_runs(&a.id, 2).await.unwrap();
        assert_eq!(deleted.len(), 3);
        assert_eq!(repo.list_runs(&a.id, 100).await.unwrap().len(), 2);
        // reap flips a fresh running row to error.
        let r = repo
            .create_run(NewAgentRun {
                agent_id: a.id.clone(),
                schedule_id: None,
                workspace_id: "ws1".into(),
                trigger: "schedule".into(),
            })
            .await
            .unwrap();
        assert_eq!(repo.reap_running().await.unwrap(), 1);
        assert_eq!(repo.get_run(&r.id).await.unwrap().status, "error");
    }

    #[tokio::test]
    async fn rooms_membership_and_messages() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let agents = PersonalAgentsRepo::new(p.clone());
        let rooms = AgentRoomsRepo::new(p.clone());
        let a = agents.create(new_agent("ws1", "A")).await.unwrap();
        let b = agents.create(new_agent("ws1", "B")).await.unwrap();
        let room = rooms.create("ws1", "standup", Some("u1")).await.unwrap();
        rooms.add_member(&room.id, &a.id).await.unwrap();
        rooms.add_member(&room.id, &a.id).await.unwrap(); // idempotent
        assert!(rooms.is_member(&room.id, &a.id).await.unwrap());
        assert!(!rooms.is_member(&room.id, &b.id).await.unwrap());
        assert_eq!(rooms.list_members(&room.id).await.unwrap(), vec![a.id.clone()]);

        let m1 = rooms
            .add_message(NewRoomMessage {
                room_id: room.id.clone(),
                author_kind: "agent".into(),
                author_id: a.id.clone(),
                text: "hello".into(),
            })
            .await
            .unwrap();
        let m2 = rooms
            .add_message(NewRoomMessage {
                room_id: room.id.clone(),
                author_kind: "user".into(),
                author_id: "u1".into(),
                text: "hi".into(),
            })
            .await
            .unwrap();
        let all = rooms.list_messages(&room.id, None, 50).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, m1.id);
        let page = rooms.list_messages(&room.id, Some(&m1.id), 50).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, m2.id);

        // Deleting an agent cascades its membership but keeps its messages
        // (the transcript stays user-visible).
        agents.delete(&a.id).await.unwrap();
        assert!(!rooms.is_member(&room.id, &a.id).await.unwrap());
        assert_eq!(rooms.list_messages(&room.id, None, 50).await.unwrap().len(), 2);
        // Deleting the room cascades its messages.
        rooms.delete(&room.id).await.unwrap();
        assert!(rooms.get(&room.id).await.is_err());
    }
}
