//! Agent Swarm repository: swarms, agents, projects, tasks, runs, board
//! messages. Self-contained module (row structs serve directly as API DTOs).
//! See docs/superpowers/specs/2026-06-18-agent-swarm-design.md.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

// --- Domain ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Swarm {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub description: String,
    pub preset_slug: Option<String>,
    pub status: String,
    pub config: Value,
    /// Budget guardrails (D3). `None` on any field = unlimited for that dimension.
    /// Lifetime run cap for the swarm (counts all `swarm_runs` rows).
    pub max_total_runs: Option<i64>,
    /// Wall-clock budget in seconds, measured from `created_at`.
    pub max_runtime_secs: Option<i64>,
    /// Summed cost cap in USD (best-effort/soft — cost may be 0 until usage
    /// attribution lands).
    pub max_cost_usd: Option<f64>,
    /// Per-task attempt ceiling before a task is marked `blocked`.
    pub max_attempts: Option<i64>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmAgent {
    pub id: Id,
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub title: String,
    pub reports_to: Option<Id>,
    pub provider: String,
    pub model: Option<String>,
    pub soul_name: Option<String>,
    pub soul_md: Option<String>,
    pub specialization: String,
    pub scope_md: String,
    pub skills: Value,
    pub schedule: Option<Value>,
    pub cwd_mode: Option<String>,
    pub avatar: String,
    pub status: String,
    pub order_idx: i64,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmProject {
    pub id: Id,
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub description: String,
    pub repo_path: Option<String>,
    pub goal_md: Option<String>,
    pub status: String,
    pub order_idx: i64,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmTask {
    pub id: Id,
    pub project_id: Id,
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub title: String,
    pub description: String,
    pub assignee_agent_id: Option<Id>,
    pub status: String,
    pub priority: String,
    pub parent_task_id: Option<Id>,
    pub depends_on: Value,
    pub labels: Value,
    pub result_ref: Option<String>,
    pub delegated: bool,
    pub order_idx: i64,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmRun {
    pub id: Id,
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub agent_id: Id,
    pub session_id: Option<Id>,
    pub kind: String,
    pub trigger: String,
    pub status: String,
    pub attempt: i64,
    pub summary: Option<String>,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub tokens_input: Option<i64>,
    pub tokens_output: Option<i64>,
    pub cost_usd: Option<f64>,
    pub enqueued_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmMessage {
    pub id: Id,
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub run_id: Option<Id>,
    pub author_agent_id: Option<Id>,
    pub author_user_id: Option<Id>,
    pub to_agent_id: Option<Id>,
    pub kind: String,
    pub body: String,
    pub meta: Value,
    pub created_at: DateTime<Utc>,
}

// --- Inputs ----------------------------------------------------------------

pub struct NewSwarm {
    pub workspace_id: Id,
    pub name: String,
    pub description: String,
    pub preset_slug: Option<String>,
    pub config: Value,
    /// Budget guardrails (None = unlimited for that dimension).
    pub max_total_runs: Option<i64>,
    pub max_runtime_secs: Option<i64>,
    pub max_cost_usd: Option<f64>,
    pub max_attempts: Option<i64>,
    pub created_by: Id,
}

#[derive(Default)]
pub struct SwarmPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub config: Option<Value>,
    /// Budget guardrails. `Some(None)` clears (→ unlimited), `Some(Some(v))` sets.
    pub max_total_runs: Option<Option<i64>>,
    pub max_runtime_secs: Option<Option<i64>>,
    pub max_cost_usd: Option<Option<f64>>,
    pub max_attempts: Option<Option<i64>>,
}

pub struct NewAgent {
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub title: String,
    pub reports_to: Option<Id>,
    pub provider: String,
    pub model: Option<String>,
    pub soul_name: Option<String>,
    pub soul_md: Option<String>,
    pub specialization: String,
    pub scope_md: String,
    pub skills: Value,
    pub schedule: Option<Value>,
    pub cwd_mode: Option<String>,
    pub avatar: String,
    pub order_idx: i64,
    pub created_by: Id,
}

#[derive(Default)]
pub struct AgentPatch {
    pub name: Option<String>,
    pub title: Option<String>,
    pub reports_to: Option<Option<Id>>,
    pub provider: Option<String>,
    pub model: Option<Option<String>>,
    pub soul_name: Option<Option<String>>,
    pub soul_md: Option<Option<String>>,
    pub specialization: Option<String>,
    pub scope_md: Option<String>,
    pub skills: Option<Value>,
    pub schedule: Option<Option<Value>>,
    pub cwd_mode: Option<Option<String>>,
    pub avatar: Option<String>,
    pub status: Option<String>,
    pub order_idx: Option<i64>,
}

pub struct NewProject {
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub description: String,
    pub repo_path: Option<String>,
    pub goal_md: Option<String>,
    pub order_idx: i64,
    pub created_by: Id,
}

#[derive(Default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub repo_path: Option<Option<String>>,
    pub goal_md: Option<Option<String>>,
    pub status: Option<String>,
    pub order_idx: Option<i64>,
}

pub struct NewTask {
    pub project_id: Id,
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub title: String,
    pub description: String,
    pub assignee_agent_id: Option<Id>,
    pub status: String,
    pub priority: String,
    pub parent_task_id: Option<Id>,
    pub depends_on: Value,
    pub labels: Value,
    pub order_idx: i64,
    pub created_by: Id,
}

#[derive(Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub assignee_agent_id: Option<Option<Id>>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub depends_on: Option<Value>,
    pub labels: Option<Value>,
    pub result_ref: Option<Option<String>>,
    pub delegated: Option<bool>,
    pub order_idx: Option<i64>,
}

pub struct NewRun {
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub agent_id: Id,
    pub kind: String,
    pub trigger: String,
}

#[derive(Default)]
pub struct RunPatch {
    pub session_id: Option<Option<Id>>,
    pub status: Option<String>,
    pub attempt: Option<i64>,
    pub summary: Option<Option<String>>,
    pub result: Option<Option<Value>>,
    pub error: Option<Option<String>>,
    pub tokens_input: Option<Option<i64>>,
    pub tokens_output: Option<Option<i64>>,
    pub cost_usd: Option<Option<f64>>,
    pub started_at: Option<Option<DateTime<Utc>>>,
    pub finished_at: Option<Option<DateTime<Utc>>>,
}

pub struct NewMessage {
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub run_id: Option<Id>,
    pub author_agent_id: Option<Id>,
    pub author_user_id: Option<Id>,
    pub to_agent_id: Option<Id>,
    pub kind: String,
    pub body: String,
    pub meta: Value,
}

/// Filters for the runs list/kanban feed.
#[derive(Default)]
pub struct RunFilter {
    pub swarm_id: Option<Id>,
    pub project_id: Option<Id>,
    pub agent_id: Option<Id>,
    pub status: Option<String>,
}

// --- Row mappers -----------------------------------------------------------

fn opt_json(s: Option<String>) -> Result<Option<Value>> {
    match s {
        Some(t) => Ok(Some(json(&t)?)),
        None => Ok(None),
    }
}

fn opt_ts(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    match s {
        Some(t) => Ok(Some(ts(&t)?)),
        None => Ok(None),
    }
}

fn row_to_swarm(r: &sqlx::sqlite::SqliteRow) -> Result<Swarm> {
    Ok(Swarm {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        description: r.get("description"),
        preset_slug: r.get("preset_slug"),
        status: r.get("status"),
        config: json(&r.get::<String, _>("config_json"))?,
        max_total_runs: r.get("max_total_runs"),
        max_runtime_secs: r.get("max_runtime_secs"),
        max_cost_usd: r.get("max_cost_usd"),
        max_attempts: r.get("max_attempts"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_agent(r: &sqlx::sqlite::SqliteRow) -> Result<SwarmAgent> {
    Ok(SwarmAgent {
        id: r.get("id"),
        swarm_id: r.get("swarm_id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        title: r.get("title"),
        reports_to: r.get("reports_to"),
        provider: r.get("provider"),
        model: r.get("model"),
        soul_name: r.get("soul_name"),
        soul_md: r.get("soul_md"),
        specialization: r.get("specialization"),
        scope_md: r.get("scope_md"),
        skills: json(&r.get::<String, _>("skills_json"))?,
        schedule: opt_json(r.get::<Option<String>, _>("schedule_json"))?,
        cwd_mode: r.get("cwd_mode"),
        avatar: r.get("avatar"),
        status: r.get("status"),
        order_idx: r.get("order_idx"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_project(r: &sqlx::sqlite::SqliteRow) -> Result<SwarmProject> {
    Ok(SwarmProject {
        id: r.get("id"),
        swarm_id: r.get("swarm_id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        description: r.get("description"),
        repo_path: r.get("repo_path"),
        goal_md: r.get("goal_md"),
        status: r.get("status"),
        order_idx: r.get("order_idx"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_task(r: &sqlx::sqlite::SqliteRow) -> Result<SwarmTask> {
    Ok(SwarmTask {
        id: r.get("id"),
        project_id: r.get("project_id"),
        swarm_id: r.get("swarm_id"),
        workspace_id: r.get("workspace_id"),
        title: r.get("title"),
        description: r.get("description"),
        assignee_agent_id: r.get("assignee_agent_id"),
        status: r.get("status"),
        priority: r.get("priority"),
        parent_task_id: r.get("parent_task_id"),
        depends_on: json(&r.get::<String, _>("depends_on_json"))?,
        labels: json(&r.get::<String, _>("labels_json"))?,
        result_ref: r.get("result_ref"),
        delegated: r.get::<i64, _>("delegated") != 0,
        order_idx: r.get("order_idx"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_run(r: &sqlx::sqlite::SqliteRow) -> Result<SwarmRun> {
    Ok(SwarmRun {
        id: r.get("id"),
        swarm_id: r.get("swarm_id"),
        workspace_id: r.get("workspace_id"),
        project_id: r.get("project_id"),
        task_id: r.get("task_id"),
        agent_id: r.get("agent_id"),
        session_id: r.get("session_id"),
        kind: r.get("kind"),
        trigger: r.get("trigger"),
        status: r.get("status"),
        attempt: r.get("attempt"),
        summary: r.get("summary"),
        result: opt_json(r.get::<Option<String>, _>("result_json"))?,
        error: r.get("error"),
        tokens_input: r.get("tokens_input"),
        tokens_output: r.get("tokens_output"),
        cost_usd: r.get("cost_usd"),
        enqueued_at: ts(&r.get::<String, _>("enqueued_at"))?,
        started_at: opt_ts(r.get::<Option<String>, _>("started_at"))?,
        finished_at: opt_ts(r.get::<Option<String>, _>("finished_at"))?,
    })
}

fn row_to_message(r: &sqlx::sqlite::SqliteRow) -> Result<SwarmMessage> {
    Ok(SwarmMessage {
        id: r.get("id"),
        swarm_id: r.get("swarm_id"),
        workspace_id: r.get("workspace_id"),
        project_id: r.get("project_id"),
        task_id: r.get("task_id"),
        run_id: r.get("run_id"),
        author_agent_id: r.get("author_agent_id"),
        author_user_id: r.get("author_user_id"),
        to_agent_id: r.get("to_agent_id"),
        kind: r.get("kind"),
        body: r.get("body"),
        meta: json(&r.get::<String, _>("meta_json"))?,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

// --- Repo ------------------------------------------------------------------

#[derive(Clone)]
pub struct SwarmRepo {
    pool: SqlitePool,
}

impl SwarmRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- Swarms -------------------------------------------------------------

    pub async fn list_swarms(&self, ws: &Id) -> Result<Vec<Swarm>> {
        let rows = sqlx::query("SELECT * FROM swarms WHERE workspace_id = ? ORDER BY created_at DESC")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list swarms"))?;
        rows.iter().map(row_to_swarm).collect()
    }

    pub async fn list_all_active_swarms(&self) -> Result<Vec<Swarm>> {
        let rows = sqlx::query("SELECT * FROM swarms WHERE status = 'active'")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list active swarms"))?;
        rows.iter().map(row_to_swarm).collect()
    }

    pub async fn get_swarm(&self, id: &Id) -> Result<Swarm> {
        let row = sqlx::query("SELECT * FROM swarms WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get swarm"))?;
        row_to_swarm(&row)
    }

    pub async fn create_swarm(&self, s: NewSwarm) -> Result<Swarm> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO swarms (id, workspace_id, name, description, preset_slug, status,
                                 config_json, max_total_runs, max_runtime_secs, max_cost_usd,
                                 max_attempts, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'paused', ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&s.workspace_id)
        .bind(&s.name)
        .bind(&s.description)
        .bind(&s.preset_slug)
        .bind(s.config.to_string())
        .bind(s.max_total_runs)
        .bind(s.max_runtime_secs)
        .bind(s.max_cost_usd)
        .bind(s.max_attempts)
        .bind(&s.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create swarm"))?;
        self.get_swarm(&id).await
    }

    pub async fn update_swarm(&self, id: &Id, p: SwarmPatch) -> Result<Swarm> {
        let cur = self.get_swarm(id).await?;
        let name = p.name.unwrap_or(cur.name);
        let description = p.description.unwrap_or(cur.description);
        let status = p.status.unwrap_or(cur.status);
        let config = p.config.unwrap_or(cur.config).to_string();
        let max_total_runs = p.max_total_runs.unwrap_or(cur.max_total_runs);
        let max_runtime_secs = p.max_runtime_secs.unwrap_or(cur.max_runtime_secs);
        let max_cost_usd = p.max_cost_usd.unwrap_or(cur.max_cost_usd);
        let max_attempts = p.max_attempts.unwrap_or(cur.max_attempts);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarms SET name = ?, description = ?, status = ?, config_json = ?,
                max_total_runs = ?, max_runtime_secs = ?, max_cost_usd = ?, max_attempts = ?,
                updated_at = ? WHERE id = ?",
        )
        .bind(&name)
        .bind(&description)
        .bind(&status)
        .bind(&config)
        .bind(max_total_runs)
        .bind(max_runtime_secs)
        .bind(max_cost_usd)
        .bind(max_attempts)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update swarm"))?;
        self.get_swarm(id).await
    }

    pub async fn set_swarm_status(&self, id: &Id, status: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE swarms SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set swarm status"))?;
        Ok(())
    }

    pub async fn delete_swarm(&self, id: &Id) -> Result<()> {
        // Cascade-delete children (no FK cascade in the schema).
        for tbl in [
            "swarm_messages",
            "swarm_runs",
            "swarm_tasks",
            "swarm_projects",
            "swarm_agents",
        ] {
            sqlx::query(&format!("DELETE FROM {tbl} WHERE swarm_id = ?"))
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("delete swarm children"))?;
        }
        sqlx::query("DELETE FROM swarms WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete swarm"))?;
        Ok(())
    }

    // -- Agents -------------------------------------------------------------

    pub async fn list_agents(&self, swarm_id: &Id) -> Result<Vec<SwarmAgent>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_agents WHERE swarm_id = ? ORDER BY order_idx, created_at",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list agents"))?;
        rows.iter().map(row_to_agent).collect()
    }

    pub async fn list_scheduled_agents(&self) -> Result<Vec<SwarmAgent>> {
        let rows = sqlx::query(
            "SELECT a.* FROM swarm_agents a JOIN swarms s ON s.id = a.swarm_id
             WHERE a.status = 'active' AND s.status = 'active' AND a.schedule_json IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list scheduled agents"))?;
        rows.iter().map(row_to_agent).collect()
    }

    pub async fn get_agent(&self, id: &Id) -> Result<SwarmAgent> {
        let row = sqlx::query("SELECT * FROM swarm_agents WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get agent"))?;
        row_to_agent(&row)
    }

    pub async fn create_agent(&self, a: NewAgent) -> Result<SwarmAgent> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO swarm_agents (id, swarm_id, workspace_id, name, title, reports_to,
                provider, model, soul_name, soul_md, specialization, scope_md, skills_json,
                schedule_json, cwd_mode, avatar, status, order_idx, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.swarm_id)
        .bind(&a.workspace_id)
        .bind(&a.name)
        .bind(&a.title)
        .bind(&a.reports_to)
        .bind(&a.provider)
        .bind(&a.model)
        .bind(&a.soul_name)
        .bind(&a.soul_md)
        .bind(&a.specialization)
        .bind(&a.scope_md)
        .bind(a.skills.to_string())
        .bind(a.schedule.as_ref().map(|v| v.to_string()))
        .bind(&a.cwd_mode)
        .bind(&a.avatar)
        .bind(a.order_idx)
        .bind(&a.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create agent"))?;
        self.get_agent(&id).await
    }

    pub async fn update_agent(&self, id: &Id, p: AgentPatch) -> Result<SwarmAgent> {
        let cur = self.get_agent(id).await?;
        let name = p.name.unwrap_or(cur.name);
        let title = p.title.unwrap_or(cur.title);
        let reports_to = p.reports_to.unwrap_or(cur.reports_to);
        let provider = p.provider.unwrap_or(cur.provider);
        let model = p.model.unwrap_or(cur.model);
        let soul_name = p.soul_name.unwrap_or(cur.soul_name);
        let soul_md = p.soul_md.unwrap_or(cur.soul_md);
        let specialization = p.specialization.unwrap_or(cur.specialization);
        let scope_md = p.scope_md.unwrap_or(cur.scope_md);
        let skills = p.skills.unwrap_or(cur.skills).to_string();
        let schedule = p.schedule.unwrap_or(cur.schedule).map(|v| v.to_string());
        let cwd_mode = p.cwd_mode.unwrap_or(cur.cwd_mode);
        let avatar = p.avatar.unwrap_or(cur.avatar);
        let status = p.status.unwrap_or(cur.status);
        let order_idx = p.order_idx.unwrap_or(cur.order_idx);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarm_agents SET name = ?, title = ?, reports_to = ?, provider = ?, model = ?,
                soul_name = ?, soul_md = ?, specialization = ?, scope_md = ?, skills_json = ?,
                schedule_json = ?, cwd_mode = ?, avatar = ?, status = ?, order_idx = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&title)
        .bind(&reports_to)
        .bind(&provider)
        .bind(&model)
        .bind(&soul_name)
        .bind(&soul_md)
        .bind(&specialization)
        .bind(&scope_md)
        .bind(&skills)
        .bind(&schedule)
        .bind(&cwd_mode)
        .bind(&avatar)
        .bind(&status)
        .bind(order_idx)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update agent"))?;
        self.get_agent(id).await
    }

    pub async fn delete_agent(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM swarm_agents WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete agent"))?;
        Ok(())
    }

    // -- Projects -----------------------------------------------------------

    pub async fn list_projects(&self, swarm_id: &Id) -> Result<Vec<SwarmProject>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_projects WHERE swarm_id = ? ORDER BY order_idx, created_at",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list projects"))?;
        rows.iter().map(row_to_project).collect()
    }

    pub async fn get_project(&self, id: &Id) -> Result<SwarmProject> {
        let row = sqlx::query("SELECT * FROM swarm_projects WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get project"))?;
        row_to_project(&row)
    }

    pub async fn create_project(&self, p: NewProject) -> Result<SwarmProject> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO swarm_projects (id, swarm_id, workspace_id, name, description, repo_path,
                goal_md, status, order_idx, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&p.swarm_id)
        .bind(&p.workspace_id)
        .bind(&p.name)
        .bind(&p.description)
        .bind(&p.repo_path)
        .bind(&p.goal_md)
        .bind(p.order_idx)
        .bind(&p.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create project"))?;
        self.get_project(&id).await
    }

    pub async fn update_project(&self, id: &Id, p: ProjectPatch) -> Result<SwarmProject> {
        let cur = self.get_project(id).await?;
        let name = p.name.unwrap_or(cur.name);
        let description = p.description.unwrap_or(cur.description);
        let repo_path = p.repo_path.unwrap_or(cur.repo_path);
        let goal_md = p.goal_md.unwrap_or(cur.goal_md);
        let status = p.status.unwrap_or(cur.status);
        let order_idx = p.order_idx.unwrap_or(cur.order_idx);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarm_projects SET name = ?, description = ?, repo_path = ?, goal_md = ?,
                status = ?, order_idx = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&name)
        .bind(&description)
        .bind(&repo_path)
        .bind(&goal_md)
        .bind(&status)
        .bind(order_idx)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update project"))?;
        self.get_project(id).await
    }

    pub async fn delete_project(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM swarm_tasks WHERE project_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete project tasks"))?;
        sqlx::query("DELETE FROM swarm_projects WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete project"))?;
        Ok(())
    }

    // -- Tasks --------------------------------------------------------------

    pub async fn list_tasks(&self, project_id: &Id) -> Result<Vec<SwarmTask>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_tasks WHERE project_id = ? ORDER BY order_idx, created_at",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list tasks"))?;
        rows.iter().map(row_to_task).collect()
    }

    pub async fn list_tasks_for_swarm(&self, swarm_id: &Id) -> Result<Vec<SwarmTask>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_tasks WHERE swarm_id = ? ORDER BY order_idx, created_at",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list swarm tasks"))?;
        rows.iter().map(row_to_task).collect()
    }

    pub async fn get_task(&self, id: &Id) -> Result<SwarmTask> {
        let row = sqlx::query("SELECT * FROM swarm_tasks WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get task"))?;
        row_to_task(&row)
    }

    pub async fn create_task(&self, t: NewTask) -> Result<SwarmTask> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO swarm_tasks (id, project_id, swarm_id, workspace_id, title, description,
                assignee_agent_id, status, priority, parent_task_id, depends_on_json, labels_json,
                delegated, order_idx, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&t.project_id)
        .bind(&t.swarm_id)
        .bind(&t.workspace_id)
        .bind(&t.title)
        .bind(&t.description)
        .bind(&t.assignee_agent_id)
        .bind(&t.status)
        .bind(&t.priority)
        .bind(&t.parent_task_id)
        .bind(t.depends_on.to_string())
        .bind(t.labels.to_string())
        .bind(t.order_idx)
        .bind(&t.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create task"))?;
        self.get_task(&id).await
    }

    pub async fn update_task(&self, id: &Id, p: TaskPatch) -> Result<SwarmTask> {
        let cur = self.get_task(id).await?;
        let title = p.title.unwrap_or(cur.title);
        let description = p.description.unwrap_or(cur.description);
        let assignee = p.assignee_agent_id.unwrap_or(cur.assignee_agent_id);
        let status = p.status.unwrap_or(cur.status);
        let priority = p.priority.unwrap_or(cur.priority);
        let depends_on = p.depends_on.unwrap_or(cur.depends_on).to_string();
        let labels = p.labels.unwrap_or(cur.labels).to_string();
        let result_ref = p.result_ref.unwrap_or(cur.result_ref);
        let delegated = p.delegated.unwrap_or(cur.delegated);
        let order_idx = p.order_idx.unwrap_or(cur.order_idx);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarm_tasks SET title = ?, description = ?, assignee_agent_id = ?, status = ?,
                priority = ?, depends_on_json = ?, labels_json = ?, result_ref = ?, delegated = ?,
                order_idx = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&title)
        .bind(&description)
        .bind(&assignee)
        .bind(&status)
        .bind(&priority)
        .bind(&depends_on)
        .bind(&labels)
        .bind(&result_ref)
        .bind(i64::from(delegated))
        .bind(order_idx)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update task"))?;
        self.get_task(id).await
    }

    pub async fn delete_task(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM swarm_tasks WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete task"))?;
        Ok(())
    }

    /// Tasks ready to run: status='todo' with all dependencies done.
    pub async fn ready_tasks(&self, swarm_id: &Id) -> Result<Vec<SwarmTask>> {
        let all = self.list_tasks_for_swarm(swarm_id).await?;
        let done: std::collections::HashSet<&str> = all
            .iter()
            .filter(|t| t.status == "done")
            .map(|t| t.id.as_str())
            .collect();
        let mut ready: Vec<SwarmTask> = all
            .iter()
            .filter(|t| t.status == "todo")
            .filter(|t| {
                t.depends_on
                    .as_array()
                    .map(|deps| {
                        deps.iter()
                            .filter_map(|d| d.as_str())
                            .all(|d| done.contains(d))
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        // Priority order then manual order.
        let rank = |p: &str| match p {
            "urgent" => 0,
            "high" => 1,
            "medium" => 2,
            _ => 3,
        };
        ready.sort_by(|a, b| {
            rank(&a.priority)
                .cmp(&rank(&b.priority))
                .then(a.order_idx.cmp(&b.order_idx))
        });
        Ok(ready)
    }

    /// Are all child tasks of a parent complete (done/cancelled)?
    pub async fn children_complete(&self, parent_id: &Id) -> Result<bool> {
        let rows = sqlx::query(
            "SELECT status FROM swarm_tasks WHERE parent_task_id = ?",
        )
        .bind(parent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("children complete"))?;
        if rows.is_empty() {
            return Ok(false);
        }
        Ok(rows
            .iter()
            .all(|r| matches!(r.get::<String, _>("status").as_str(), "done" | "cancelled")))
    }

    // -- Runs ---------------------------------------------------------------

    pub async fn create_run(&self, r: NewRun) -> Result<SwarmRun> {
        let id = new_id();
        let now = fmt(Utc::now());
        // 0-based attempt index for the task: number of prior runs against it. Used
        // by the coordinator's per-task attempt ceiling (D3).
        let attempt = match &r.task_id {
            Some(tid) => self.task_run_count(tid).await.unwrap_or(0),
            None => 0,
        };
        sqlx::query(
            "INSERT INTO swarm_runs (id, swarm_id, workspace_id, project_id, task_id, agent_id,
                kind, trigger, status, attempt, enqueued_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?)",
        )
        .bind(&id)
        .bind(&r.swarm_id)
        .bind(&r.workspace_id)
        .bind(&r.project_id)
        .bind(&r.task_id)
        .bind(&r.agent_id)
        .bind(&r.kind)
        .bind(&r.trigger)
        .bind(attempt)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create run"))?;
        self.get_run(&id).await
    }

    pub async fn get_run(&self, id: &Id) -> Result<SwarmRun> {
        let row = sqlx::query("SELECT * FROM swarm_runs WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get run"))?;
        row_to_run(&row)
    }

    pub async fn update_run(&self, id: &Id, p: RunPatch) -> Result<SwarmRun> {
        let cur = self.get_run(id).await?;
        let session_id = p.session_id.unwrap_or(cur.session_id);
        let status = p.status.unwrap_or(cur.status);
        let attempt = p.attempt.unwrap_or(cur.attempt);
        let summary = p.summary.unwrap_or(cur.summary);
        let result = p.result.unwrap_or(cur.result).map(|v| v.to_string());
        let error = p.error.unwrap_or(cur.error);
        let tokens_input = p.tokens_input.unwrap_or(cur.tokens_input);
        let tokens_output = p.tokens_output.unwrap_or(cur.tokens_output);
        let cost_usd = p.cost_usd.unwrap_or(cur.cost_usd);
        let started_at = p.started_at.unwrap_or(cur.started_at).map(fmt);
        let finished_at = p.finished_at.unwrap_or(cur.finished_at).map(fmt);
        sqlx::query(
            "UPDATE swarm_runs SET session_id = ?, status = ?, attempt = ?, summary = ?,
                result_json = ?, error = ?, tokens_input = ?, tokens_output = ?, cost_usd = ?,
                started_at = ?, finished_at = ? WHERE id = ?",
        )
        .bind(&session_id)
        .bind(&status)
        .bind(attempt)
        .bind(&summary)
        .bind(&result)
        .bind(&error)
        .bind(tokens_input)
        .bind(tokens_output)
        .bind(cost_usd)
        .bind(&started_at)
        .bind(&finished_at)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update run"))?;
        self.get_run(id).await
    }

    pub async fn running_count(&self, swarm_id: &Id) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS n FROM swarm_runs WHERE swarm_id = ? AND status IN ('running','waiting')",
        )
        .bind(swarm_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("running count"))?;
        Ok(row.get::<i64, _>("n"))
    }

    /// In-flight runs (queued + running + waiting) — used for the parallel cap.
    pub async fn active_run_count(&self, swarm_id: &Id) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS n FROM swarm_runs WHERE swarm_id = ?
             AND status IN ('queued','running','waiting')",
        )
        .bind(swarm_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("active run count"))?;
        Ok(row.get::<i64, _>("n"))
    }

    /// Lifetime run count for a swarm (all `swarm_runs` rows, any status). Drives
    /// the `max_total_runs` budget.
    pub async fn total_run_count(&self, swarm_id: &Id) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM swarm_runs WHERE swarm_id = ?")
            .bind(swarm_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("total run count"))?;
        Ok(row.get::<i64, _>("n"))
    }

    /// Summed cost (USD) across a swarm's runs. Best-effort/soft — `cost_usd` may
    /// be 0/NULL until usage attribution lands. Drives the `max_cost_usd` budget.
    pub async fn total_cost(&self, swarm_id: &Id) -> Result<f64> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(cost_usd), 0.0) AS c FROM swarm_runs WHERE swarm_id = ?",
        )
        .bind(swarm_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("total cost"))?;
        Ok(row.get::<f64, _>("c"))
    }

    /// How many runs have been started for a task (its attempt count). Drives the
    /// per-task `max_attempts` ceiling so a task isn't re-queued forever.
    pub async fn task_run_count(&self, task_id: &Id) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM swarm_runs WHERE task_id = ?")
            .bind(task_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("task run count"))?;
        Ok(row.get::<i64, _>("n"))
    }

    pub async fn agent_has_active_run(&self, agent_id: &Id) -> Result<bool> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS n FROM swarm_runs WHERE agent_id = ? AND status IN ('queued','running','waiting')",
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("agent active run"))?;
        Ok(row.get::<i64, _>("n") > 0)
    }

    pub async fn list_runs(&self, f: &RunFilter) -> Result<Vec<SwarmRun>> {
        let mut sql = String::from("SELECT * FROM swarm_runs WHERE 1=1");
        if f.swarm_id.is_some() {
            sql.push_str(" AND swarm_id = ?");
        }
        if f.project_id.is_some() {
            sql.push_str(" AND project_id = ?");
        }
        if f.agent_id.is_some() {
            sql.push_str(" AND agent_id = ?");
        }
        if f.status.is_some() {
            sql.push_str(" AND status = ?");
        }
        sql.push_str(" ORDER BY enqueued_at DESC LIMIT 500");
        let mut q = sqlx::query(&sql);
        if let Some(v) = &f.swarm_id {
            q = q.bind(v);
        }
        if let Some(v) = &f.project_id {
            q = q.bind(v);
        }
        if let Some(v) = &f.agent_id {
            q = q.bind(v);
        }
        if let Some(v) = &f.status {
            q = q.bind(v);
        }
        let rows = q.fetch_all(&self.pool).await.map_err(dberr("list runs"))?;
        rows.iter().map(row_to_run).collect()
    }

    pub async fn runs_for_swarm(&self, swarm_id: &Id, limit: i64) -> Result<Vec<SwarmRun>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_runs WHERE swarm_id = ? ORDER BY enqueued_at DESC LIMIT ?",
        )
        .bind(swarm_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("runs for swarm"))?;
        rows.iter().map(row_to_run).collect()
    }

    /// Mark all non-terminal runs of a swarm as stopped (abort).
    pub async fn stop_active_runs(&self, swarm_id: &Id) -> Result<Vec<Id>> {
        let rows = sqlx::query(
            "SELECT id FROM swarm_runs WHERE swarm_id = ? AND status IN ('queued','running','waiting')",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("select active runs"))?;
        let ids: Vec<Id> = rows.iter().map(|r| r.get::<Id, _>("id")).collect();
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarm_runs SET status = 'stopped', finished_at = ?
             WHERE swarm_id = ? AND status IN ('queued','running','waiting')",
        )
        .bind(&now)
        .bind(swarm_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("stop active runs"))?;
        Ok(ids)
    }

    // -- Board (messages) ---------------------------------------------------

    pub async fn create_message(&self, m: NewMessage) -> Result<SwarmMessage> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO swarm_messages (id, swarm_id, workspace_id, project_id, task_id, run_id,
                author_agent_id, author_user_id, to_agent_id, kind, body, meta_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&m.swarm_id)
        .bind(&m.workspace_id)
        .bind(&m.project_id)
        .bind(&m.task_id)
        .bind(&m.run_id)
        .bind(&m.author_agent_id)
        .bind(&m.author_user_id)
        .bind(&m.to_agent_id)
        .bind(&m.kind)
        .bind(&m.body)
        .bind(m.meta.to_string())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create message"))?;
        self.get_message(&id).await
    }

    pub async fn get_message(&self, id: &Id) -> Result<SwarmMessage> {
        let row = sqlx::query("SELECT * FROM swarm_messages WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get message"))?;
        row_to_message(&row)
    }

    pub async fn list_board(
        &self,
        swarm_id: &Id,
        project_id: Option<&Id>,
        task_id: Option<&Id>,
        limit: i64,
    ) -> Result<Vec<SwarmMessage>> {
        let mut sql = String::from("SELECT * FROM swarm_messages WHERE swarm_id = ?");
        if project_id.is_some() {
            sql.push_str(" AND project_id = ?");
        }
        if task_id.is_some() {
            sql.push_str(" AND task_id = ?");
        }
        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        let mut q = sqlx::query(&sql).bind(swarm_id);
        if let Some(v) = project_id {
            q = q.bind(v);
        }
        if let Some(v) = task_id {
            q = q.bind(v);
        }
        q = q.bind(limit);
        let rows = q.fetch_all(&self.pool).await.map_err(dberr("list board"))?;
        rows.iter().map(row_to_message).collect()
    }

    /// Recent board messages addressed to an agent or @all (for turn context).
    pub async fn board_for_agent(
        &self,
        swarm_id: &Id,
        agent_id: &Id,
        limit: i64,
    ) -> Result<Vec<SwarmMessage>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_messages WHERE swarm_id = ?
             AND (to_agent_id IS NULL OR to_agent_id = ?)
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(swarm_id)
        .bind(agent_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("board for agent"))?;
        rows.iter().map(row_to_message).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    /// A task created with the given `status` (no swarm/project FKs needed — the
    /// swarm tables carry no FK constraints).
    fn new_task(swarm: &Id, status: &str) -> NewTask {
        NewTask {
            project_id: new_id(),
            swarm_id: swarm.clone(),
            workspace_id: new_id(),
            title: "hand-added".into(),
            description: String::new(),
            assignee_agent_id: None,
            status: status.into(),
            priority: "medium".into(),
            parent_task_id: None,
            depends_on: json!([]),
            labels: json!([]),
            order_idx: 0,
            created_by: new_id(),
        }
    }

    /// D2 regression: a hand-added task defaults to "todo" (set by
    /// `SwarmService::create_task`), and `ready_tasks` MUST pick it up so the
    /// swarm actually runs it. An explicitly-"backlog" task stays unscheduled.
    #[tokio::test]
    async fn ready_tasks_picks_up_todo_excludes_backlog() {
        let pool = mem_pool().await;
        let repo = SwarmRepo::new(pool);
        let swarm = new_id();

        let todo = repo.create_task(new_task(&swarm, "todo")).await.unwrap();
        assert_eq!(todo.status, "todo");
        let _backlog = repo.create_task(new_task(&swarm, "backlog")).await.unwrap();

        let ready = repo.ready_tasks(&swarm).await.unwrap();
        assert_eq!(ready.len(), 1, "only the todo task should be schedulable");
        assert_eq!(ready[0].id, todo.id);
        assert_eq!(ready[0].status, "todo");
    }

    /// A "todo" task whose dependency isn't done yet is NOT ready; once the
    /// dependency is done it becomes ready (guards that the default status
    /// doesn't bypass the dependency gate).
    #[tokio::test]
    async fn ready_tasks_respects_dependencies() {
        let pool = mem_pool().await;
        let repo = SwarmRepo::new(pool);
        let swarm = new_id();

        let dep = repo.create_task(new_task(&swarm, "todo")).await.unwrap();
        let mut blocked = new_task(&swarm, "todo");
        blocked.depends_on = json!([dep.id]);
        let blocked = repo.create_task(blocked).await.unwrap();

        // Both are "todo", but `blocked` waits on `dep`.
        let ready = repo.ready_tasks(&swarm).await.unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, dep.id);

        // Finish the dependency → the blocked task becomes ready.
        repo.update_task(
            &dep.id,
            TaskPatch { status: Some("done".into()), ..Default::default() },
        )
        .await
        .unwrap();
        let ready = repo.ready_tasks(&swarm).await.unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, blocked.id);
    }

    fn new_swarm(budget: (Option<i64>, Option<i64>, Option<f64>, Option<i64>)) -> NewSwarm {
        NewSwarm {
            workspace_id: new_id(),
            name: "budget-swarm".into(),
            description: String::new(),
            preset_slug: None,
            config: json!({}),
            max_total_runs: budget.0,
            max_runtime_secs: budget.1,
            max_cost_usd: budget.2,
            max_attempts: budget.3,
            created_by: new_id(),
        }
    }

    /// D3: budget columns round-trip through create/get/update (migration 0032),
    /// and `update_swarm` can clear a budget to NULL (unlimited).
    #[tokio::test]
    async fn swarm_budget_columns_roundtrip() {
        let pool = mem_pool().await;
        let repo = SwarmRepo::new(pool);

        let s = repo
            .create_swarm(new_swarm((Some(300), Some(14400), None, Some(3))))
            .await
            .unwrap();
        assert_eq!(s.max_total_runs, Some(300));
        assert_eq!(s.max_runtime_secs, Some(14400));
        assert_eq!(s.max_cost_usd, None);
        assert_eq!(s.max_attempts, Some(3));

        // Raise one budget and clear another to unlimited.
        let s = repo
            .update_swarm(
                &s.id,
                SwarmPatch {
                    max_total_runs: Some(Some(500)),
                    max_attempts: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(s.max_total_runs, Some(500));
        assert_eq!(s.max_attempts, None, "cleared → unlimited");
        assert_eq!(s.max_runtime_secs, Some(14400), "untouched budget unchanged");
    }

    /// D3: the budget query building blocks. `total_run_count`/`total_cost` sum a
    /// swarm's runs; `task_run_count` (and `create_run`'s `attempt`) count per task.
    #[tokio::test]
    async fn budget_run_queries_and_attempt_numbering() {
        let pool = mem_pool().await;
        let repo = SwarmRepo::new(pool);
        let swarm = repo
            .create_swarm(new_swarm((Some(10), None, Some(100.0), Some(3))))
            .await
            .unwrap();
        let task = repo.create_task(new_task(&swarm.id, "todo")).await.unwrap();

        assert_eq!(repo.total_run_count(&swarm.id).await.unwrap(), 0);
        assert_eq!(repo.task_run_count(&task.id).await.unwrap(), 0);

        let mk_run = || NewRun {
            swarm_id: swarm.id.clone(),
            workspace_id: swarm.workspace_id.clone(),
            project_id: Some(task.project_id.clone()),
            task_id: Some(task.id.clone()),
            agent_id: new_id(),
            kind: "task".into(),
            trigger: "coordinator".into(),
        };

        // First run for the task → attempt 0.
        let r0 = repo.create_run(mk_run()).await.unwrap();
        assert_eq!(r0.attempt, 0, "first run is attempt 0");
        // Second run for the same task → attempt 1 (prior run count).
        let r1 = repo.create_run(mk_run()).await.unwrap();
        assert_eq!(r1.attempt, 1, "attempt = prior run count");

        assert_eq!(repo.total_run_count(&swarm.id).await.unwrap(), 2);
        assert_eq!(repo.task_run_count(&task.id).await.unwrap(), 2);

        // Cost sums (best-effort): set cost on the runs and confirm SUM.
        repo.update_run(&r0.id, RunPatch { cost_usd: Some(Some(1.50)), ..Default::default() })
            .await
            .unwrap();
        repo.update_run(&r1.id, RunPatch { cost_usd: Some(Some(2.25)), ..Default::default() })
            .await
            .unwrap();
        let spent = repo.total_cost(&swarm.id).await.unwrap();
        assert!((spent - 3.75).abs() < 1e-9, "summed cost = 3.75, got {spent}");
    }
}
