//! Agent Swarm repository: swarms, agents, projects, tasks, runs, board
//! messages. Self-contained module (row structs serve directly as API DTOs).
//! See docs/superpowers/specs/2026-06-18-agent-swarm-design.md.

use crate::DbPool;
use chrono::{DateTime, Utc};
use otto_core::{new_id, Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;

use crate::convert::{dberr, dberr_unique, fmt, json, ts};

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
    /// Budget guardrails (D3/D8). All nullable = unlimited. The Coordinator
    /// pauses the swarm (with `pause_reason`) when any is exceeded.
    pub max_total_runs: Option<i64>,
    pub max_cost_usd: Option<f64>,
    pub max_runtime_secs: Option<i64>,
    /// Per-task attempt ceiling (default 3).
    pub max_attempts: i64,
    /// When the swarm last went active — anchors the wall-clock runtime budget.
    pub run_started_at: Option<DateTime<Utc>>,
    /// Why the Coordinator auto-paused (budget/limit reason); else `None`.
    pub pause_reason: Option<String>,
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
    /// Source Product story this project was created from (Plan → Swarm), if any.
    pub story_id: Option<Id>,
    /// Project-level skill set, layered on team (swarm) + per-agent skills.
    /// `[{name, must_use?}]` (same shape as agent skills).
    pub skills: Value,
    /// Dedicated swarm branch agent worktrees are based on + merged into. Pinned
    /// once at first worktree creation; NEVER the user's working branch.
    pub integration_branch: Option<String>,
    /// Channel a webhook/Slack-launched project replies/escalates back to.
    pub origin_channel: Option<String>,
    pub origin_chat: Option<String>,
    pub origin_thread: Option<String>,
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
    /// How many turns the Coordinator has (re-)queued for this task. Compared
    /// against the swarm's `max_attempts` ceiling to stop infinite re-runs.
    pub attempts: i64,
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

/// A goal the leader verifies for a task/project, or a swarm-level standing
/// (always-applied) goal template. See docs/design/swarm-enhancements-2026-06-26.md §3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmGoal {
    pub id: Id,
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub kind: String, // explicit | standing
    pub title: String,
    pub description: String,
    pub metric: Option<String>,
    pub comparator: Option<String>,
    pub target_value: Option<f64>,
    pub block_value: Option<f64>,
    pub verify_cmd: Option<String>,
    pub max_retries: i64,
    pub blocking: bool,
    pub status: String, // pending|verifying|passed|warned|unmet|skipped|error
    pub verdict: Option<Value>,
    pub iterations: i64,
    pub order_idx: i64,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Binds a workspace channel (Slack/Telegram/webhook) to a swarm so an inbound
/// message launches the team. See design §4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmChannelTrigger {
    pub id: Id,
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub channel: String, // slack | telegram | webhook
    pub match_chat: String,
    pub keyword: String,
    pub repo_path: Option<String>,
    pub auto_start: bool,
    pub reply: bool,
    pub enabled: bool,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// --- Inputs ----------------------------------------------------------------

pub struct NewSwarm {
    pub workspace_id: Id,
    pub name: String,
    pub description: String,
    pub preset_slug: Option<String>,
    pub config: Value,
    pub max_total_runs: Option<i64>,
    pub max_cost_usd: Option<f64>,
    pub max_runtime_secs: Option<i64>,
    pub max_attempts: Option<i64>,
    pub created_by: Id,
}

#[derive(Default)]
pub struct SwarmPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub config: Option<Value>,
    // Budget guardrails. Outer Option = "present in patch"; inner = the value
    // (NULL clears the limit → unlimited).
    pub max_total_runs: Option<Option<i64>>,
    pub max_cost_usd: Option<Option<f64>>,
    pub max_runtime_secs: Option<Option<i64>>,
    pub max_attempts: Option<i64>,
    /// The person whose edit this is when it rewrites the swarm's prompt text
    /// (its description): stamped as `prompt_updated_by` / `_at` IN THE SAME
    /// UPDATE (access-control-6-02, migration 0196). `None` keeps the stamp.
    pub prompt_updated_by: Option<Id>,
}

/// Accumulated spend/run-count for a swarm (for the budget check + API surface).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SwarmSpend {
    pub total_runs: i64,
    pub cost_usd: f64,
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

/// Keys of an agent schedule that define WHEN it fires. A change to any of
/// them (or a resume) re-arms the schedule; a directive-only edit does not.
const AGENT_SCHEDULE_KEYS: [&str; 6] =
    ["cadence", "every_min", "at", "weekday", "expr", "timezone"];

/// Merge an incoming agent schedule with the stored one. The server owns the
/// two cursor keys inside `schedule_json`:
///   * `last_run` — the scheduler's fired cursor. The editor rebuilds the
///     schedule from its form fields, so a save used to drop it and the agent
///     fired again on the next tick (duplicate run). Always carried forward.
///   * `armed_at` — the instant the schedule was (re)armed: created, resumed,
///     or given a new cadence/timezone. The scheduler uses
///     `max(last_run, armed_at)` as its cursor, so a new `daily 09:00` saved at
///     15:00 waits for tomorrow instead of firing at once.
///
/// A client-supplied `last_run` / `armed_at` is ignored.
pub fn merge_agent_schedule(cur: Option<&Value>, incoming: Value, now: &str) -> Value {
    let mut next = incoming;
    let Some(obj) = next.as_object_mut() else {
        return next;
    };
    obj.remove("last_run");
    obj.remove("armed_at");
    let enabled = |v: &Value| v.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let new_enabled = obj.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let rearm = match cur.filter(|c| c.is_object()) {
        None => true,
        Some(c) => {
            (!enabled(c) && new_enabled)
                || AGENT_SCHEDULE_KEYS.iter().any(|k| c.get(k) != obj.get(*k))
        }
    };
    if let Some(c) = cur {
        if let Some(l) = c.get("last_run").filter(|v| !v.is_null()) {
            obj.insert("last_run".into(), l.clone());
        }
    }
    let armed = if rearm {
        Some(Value::String(now.to_string()))
    } else {
        cur.and_then(|c| c.get("armed_at"))
            .filter(|v| !v.is_null())
            .cloned()
    };
    if let Some(a) = armed {
        obj.insert("armed_at".into(), a);
    }
    next
}

pub struct NewProject {
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub description: String,
    pub repo_path: Option<String>,
    pub goal_md: Option<String>,
    pub story_id: Option<Id>,
    pub order_idx: i64,
    pub created_by: Id,
}

#[derive(Default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub repo_path: Option<Option<String>>,
    pub goal_md: Option<Option<String>>,
    pub story_id: Option<Option<Id>>,
    pub skills: Option<Value>,
    pub integration_branch: Option<Option<String>>,
    pub origin_channel: Option<Option<String>>,
    pub origin_chat: Option<Option<String>>,
    pub origin_thread: Option<Option<String>>,
    pub status: Option<String>,
    pub order_idx: Option<i64>,
    /// The person whose edit rewrites the project's prompt text (name,
    /// description, goal): stamped in the same UPDATE (access-control-6-02).
    pub prompt_updated_by: Option<Id>,
}

pub struct NewGoal {
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub metric: Option<String>,
    pub comparator: Option<String>,
    pub target_value: Option<f64>,
    pub block_value: Option<f64>,
    pub verify_cmd: Option<String>,
    pub max_retries: i64,
    pub blocking: bool,
    pub order_idx: i64,
    pub created_by: Id,
}

#[derive(Default)]
pub struct GoalPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub metric: Option<Option<String>>,
    pub comparator: Option<Option<String>>,
    pub target_value: Option<Option<f64>>,
    pub block_value: Option<Option<f64>>,
    pub verify_cmd: Option<Option<String>>,
    pub max_retries: Option<i64>,
    pub blocking: Option<bool>,
    pub status: Option<String>,
    pub verdict: Option<Option<Value>>,
    pub iterations: Option<i64>,
    pub order_idx: Option<i64>,
    /// The person rewriting the goal's text / verify command: stamped as the
    /// goal's `updated_by` AND as its owner's prompt editor (task, project or
    /// swarm) in one transaction (concurrency-async-6-03, data-integrity-6-02).
    pub updated_by: Option<Id>,
}

pub struct NewTrigger {
    pub swarm_id: Id,
    pub workspace_id: Id,
    pub channel: String,
    pub match_chat: String,
    pub keyword: String,
    pub repo_path: Option<String>,
    pub auto_start: bool,
    pub reply: bool,
    pub enabled: bool,
    pub created_by: Id,
}

#[derive(Default)]
pub struct TriggerPatch {
    pub channel: Option<String>,
    pub match_chat: Option<String>,
    pub keyword: Option<String>,
    pub repo_path: Option<Option<String>>,
    pub auto_start: Option<bool>,
    pub reply: Option<bool>,
    pub enabled: Option<bool>,
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
    /// The person whose edit rewrites the task's prompt (title, description,
    /// labels): stamped as `prompt_updated_by` / `_at` in the same UPDATE as
    /// the content, so the text never lands without its editor
    /// (data-integrity-6-02). `None` keeps the stamp.
    pub prompt_updated_by: Option<Id>,
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
    /// The workspace the runs belong to. The per-workspace list route MUST
    /// set it — without it the query spans every workspace's runs.
    pub workspace_id: Option<Id>,
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
        max_cost_usd: r.get("max_cost_usd"),
        max_runtime_secs: r.get("max_runtime_secs"),
        max_attempts: r.get("max_attempts"),
        run_started_at: opt_ts(r.get::<Option<String>, _>("run_started_at"))?,
        pause_reason: r.get("pause_reason"),
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
        story_id: r.get("story_id"),
        skills: json(&r.get::<String, _>("skills_json"))?,
        integration_branch: r.get("integration_branch"),
        origin_channel: r.get("origin_channel"),
        origin_chat: r.get("origin_chat"),
        origin_thread: r.get("origin_thread"),
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
        attempts: r.get("attempts"),
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

/// Whose prompt text a [`SwarmRepo::prompt_edit`] stamp belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptScope {
    Task,
    Project,
    Swarm,
}

/// Stamp `user` as `scope`'s prompt editor at `now`, inside `tx`.
async fn stamp_prompt_editor(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    scope: PromptScope,
    id: &str,
    user: &str,
    now: &str,
) -> Result<()> {
    let sql = match scope {
        PromptScope::Task => {
            "UPDATE swarm_tasks SET prompt_updated_by = ?, prompt_updated_at = ? WHERE id = ?"
        }
        PromptScope::Project => {
            "UPDATE swarm_projects SET prompt_updated_by = ?, prompt_updated_at = ? WHERE id = ?"
        }
        PromptScope::Swarm => {
            "UPDATE swarms SET prompt_updated_by = ?, prompt_updated_at = ? WHERE id = ?"
        }
    };
    sqlx::query(sql)
        .bind(user)
        .bind(now)
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(dberr("stamp prompt editor"))?;
    Ok(())
}

/// A person's goal write: the goal's `updated_by`, plus its owner's prompt
/// editor — the task for a task goal, else the project for a project goal,
/// else the swarm for a standing template — inside the goal's transaction.
async fn stamp_goal_editor(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    goal_id: &str,
    task_id: Option<&Id>,
    project_id: Option<&Id>,
    swarm_id: Option<&Id>,
    user: &str,
) -> Result<()> {
    let now = fmt(Utc::now());
    sqlx::query("UPDATE swarm_goals SET updated_by = ? WHERE id = ?")
        .bind(user)
        .bind(goal_id)
        .execute(&mut **tx)
        .await
        .map_err(dberr("set goal editor"))?;
    let owner = match (task_id, project_id, swarm_id) {
        (Some(t), _, _) => Some((PromptScope::Task, t)),
        (None, Some(p), _) => Some((PromptScope::Project, p)),
        (None, None, Some(s)) => Some((PromptScope::Swarm, s)),
        (None, None, None) => None,
    };
    if let Some((scope, id)) = owner {
        stamp_prompt_editor(tx, scope, id, user, &now).await?;
    }
    Ok(())
}

fn row_to_goal(r: &sqlx::sqlite::SqliteRow) -> Result<SwarmGoal> {
    Ok(SwarmGoal {
        id: r.get("id"),
        swarm_id: r.get("swarm_id"),
        workspace_id: r.get("workspace_id"),
        project_id: r.get("project_id"),
        task_id: r.get("task_id"),
        kind: r.get("kind"),
        title: r.get("title"),
        description: r.get("description"),
        metric: r.get("metric"),
        comparator: r.get("comparator"),
        target_value: r.get("target_value"),
        block_value: r.get("block_value"),
        verify_cmd: r.get("verify_cmd"),
        max_retries: r.get("max_retries"),
        blocking: r.get::<i64, _>("blocking") != 0,
        status: r.get("status"),
        verdict: opt_json(r.get::<Option<String>, _>("verdict_json"))?,
        iterations: r.get("iterations"),
        order_idx: r.get("order_idx"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_trigger(r: &sqlx::sqlite::SqliteRow) -> Result<SwarmChannelTrigger> {
    Ok(SwarmChannelTrigger {
        id: r.get("id"),
        swarm_id: r.get("swarm_id"),
        workspace_id: r.get("workspace_id"),
        channel: r.get("channel"),
        match_chat: r.get("match_chat"),
        keyword: r.get("keyword"),
        repo_path: r.get("repo_path"),
        auto_start: r.get::<i64, _>("auto_start") != 0,
        reply: r.get::<i64, _>("reply") != 0,
        enabled: r.get::<i64, _>("enabled") != 0,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

// --- Repo ------------------------------------------------------------------

#[derive(Clone)]
pub struct SwarmRepo {
    pool: DbPool,
}

impl SwarmRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    // -- Swarms -------------------------------------------------------------

    pub async fn list_swarms(&self, ws: &Id) -> Result<Vec<Swarm>> {
        let rows =
            sqlx::query("SELECT * FROM swarms WHERE workspace_id = ? ORDER BY created_at DESC")
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
                                 config_json, max_total_runs, max_cost_usd, max_runtime_secs,
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
        .bind(s.max_cost_usd)
        .bind(s.max_runtime_secs)
        .bind(s.max_attempts.unwrap_or(3))
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
        let max_cost_usd = p.max_cost_usd.unwrap_or(cur.max_cost_usd);
        let max_runtime_secs = p.max_runtime_secs.unwrap_or(cur.max_runtime_secs);
        let max_attempts = p.max_attempts.unwrap_or(cur.max_attempts);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarms SET name = ?, description = ?, status = ?, config_json = ?,
                max_total_runs = ?, max_cost_usd = ?, max_runtime_secs = ?, max_attempts = ?,
                updated_at = ?,
                prompt_updated_by = COALESCE(?, prompt_updated_by),
                prompt_updated_at = CASE WHEN ? IS NULL THEN prompt_updated_at ELSE ? END
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&description)
        .bind(&status)
        .bind(&config)
        .bind(max_total_runs)
        .bind(max_cost_usd)
        .bind(max_runtime_secs)
        .bind(max_attempts)
        .bind(&now)
        .bind(&p.prompt_updated_by)
        .bind(&p.prompt_updated_by)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update swarm"))?;
        self.get_swarm(id).await
    }

    /// Set a swarm's lifecycle status. Manages the budget bookkeeping too:
    /// going `active` anchors the wall-clock runtime budget (`run_started_at`)
    /// if it isn't already set and clears any prior `pause_reason`; leaving
    /// `active` (pause/abort) clears the anchor so a later resume restarts the
    /// clock.
    pub async fn set_swarm_status(&self, id: &Id, status: &str) -> Result<()> {
        let now = fmt(Utc::now());
        if status == "active" {
            sqlx::query(
                "UPDATE swarms SET status = 'active', pause_reason = NULL,
                    run_started_at = COALESCE(run_started_at, ?), updated_at = ? WHERE id = ?",
            )
            .bind(&now)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set swarm status"))?;
        } else {
            sqlx::query(
                "UPDATE swarms SET status = ?, run_started_at = NULL, updated_at = ? WHERE id = ?",
            )
            .bind(status)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set swarm status"))?;
        }
        Ok(())
    }

    /// Auto-pause a swarm because a budget/limit was hit. Records a human-facing
    /// `pause_reason` and clears the wall-clock anchor (a manual resume restarts
    /// the clock — the user is expected to raise the budget first).
    pub async fn pause_swarm_with_reason(&self, id: &Id, reason: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarms SET status = 'paused', pause_reason = ?, run_started_at = NULL,
                updated_at = ? WHERE id = ?",
        )
        .bind(reason)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("pause swarm with reason"))?;
        Ok(())
    }

    /// Accumulated spend + total run count for a swarm — the basis for the
    /// `max_total_runs` / `max_cost_usd` budget checks and the API surface.
    /// `cost_usd` sums the per-run backfilled cost (NULLs count as 0).
    /// A scheduled row stopped before a session does not count, and retention
    /// does not roll that row into `pruned_runs`.
    pub async fn swarm_spend(&self, swarm_id: &Id) -> Result<SwarmSpend> {
        // + the rollup of runs the retention job pruned (migration 0161), so
        // pruning never refunds a lifetime budget.
        let counts = not_unstarted_scheduled_stop("");
        let sql = format!(
            "SELECT (SELECT COUNT(*) FROM swarm_runs WHERE swarm_id = ?1 AND {counts})
                    + COALESCE((SELECT pruned_runs FROM swarms WHERE id = ?1), 0) AS n,
                    COALESCE((SELECT SUM(cost_usd) FROM swarm_runs WHERE swarm_id = ?1 AND {counts}), 0.0)
                      + COALESCE((SELECT pruned_cost_usd FROM swarms WHERE id = ?1), 0.0) AS spend"
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(swarm_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("swarm spend"))?;
        Ok(SwarmSpend {
            total_runs: row.get::<i64, _>("n"),
            cost_usd: row.get::<f64, _>("spend"),
        })
    }

    pub async fn delete_swarm(&self, id: &Id) -> Result<()> {
        // Cascade-delete children (no FK cascade in the schema) in ONE write
        // transaction: a failure midway (e.g. SQLITE_BUSY) used to leave the
        // swarm row behind with its runs and tasks already gone. Goals and
        // channel triggers carry `swarm_id` too — left behind, every inbound
        // channel message kept matching the orphaned triggers.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("delete swarm begin"))?;
        for tbl in [
            "swarm_messages",
            "swarm_goals",
            "swarm_channel_triggers",
            "swarm_runs",
            "swarm_tasks",
            "swarm_projects",
            "swarm_agents",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DELETE FROM {tbl} WHERE swarm_id = ?"
            )))
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete swarm children"))?;
        }
        sqlx::query("DELETE FROM swarms WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete swarm"))?;
        tx.commit().await.map_err(dberr("delete swarm commit"))?;
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
        .bind(
            a.schedule
                .clone()
                .map(|v| merge_agent_schedule(None, v, &Utc::now().to_rfc3339()).to_string()),
        )
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
        // Read-modify-write under the write lock: the scheduler advances the
        // schedule cursor concurrently, and the merge must see the stored one.
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin agent edit"))?;
        let row = sqlx::query("SELECT * FROM swarm_agents WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("get agent for edit"))?;
        let cur = row_to_agent(&row)?;
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
        let schedule = match p.schedule {
            None => cur.schedule,
            Some(None) => None,
            Some(Some(v)) => Some(merge_agent_schedule(
                cur.schedule.as_ref(),
                v,
                &Utc::now().to_rfc3339(),
            )),
        }
        .map(|v| v.to_string());
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
        .execute(&mut *tx)
        .await
        .map_err(dberr("update agent"))?;
        tx.commit().await.map_err(dberr("commit agent edit"))?;
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
        let row = sqlx::query("SELECT * FROM swarm_projects WHERE id = ? AND swarm_id IS NOT NULL")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get project"))?;
        row_to_project(&row)
    }

    /// The swarm project created from a given Product story (Plan → Swarm
    /// back-link), if any. Most-recently-created wins if the story was sent more
    /// than once. Used by the story view's "linked swarm project" badge.
    pub async fn project_for_story(&self, story_id: &Id) -> Result<Option<SwarmProject>> {
        let row = sqlx::query(
            "SELECT * FROM swarm_projects WHERE story_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(story_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("project for story"))?;
        row.as_ref().map(row_to_project).transpose()
    }

    pub async fn create_project(&self, p: NewProject) -> Result<SwarmProject> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO swarm_projects (id, swarm_id, workspace_id, name, description, repo_path,
                goal_md, story_id, status, order_idx, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&p.swarm_id)
        .bind(&p.workspace_id)
        .bind(&p.name)
        .bind(&p.description)
        .bind(&p.repo_path)
        .bind(&p.goal_md)
        .bind(&p.story_id)
        .bind(p.order_idx)
        .bind(&p.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        // A project may carry a `story_id`, which is unique among non-NULL rows
        // (migration 0037): two concurrent Plan→Swarm hand-offs racing past the
        // pre-check both insert here, and the loser must see a Conflict (not a
        // generic 500) so it can fall back to the existing linked project.
        .map_err(dberr_unique(
            "create project",
            "a swarm project is already linked to this story",
        ))?;
        self.get_project(&id).await
    }

    pub async fn update_project(&self, id: &Id, p: ProjectPatch) -> Result<SwarmProject> {
        let cur = self.get_project(id).await?;
        let name = p.name.unwrap_or(cur.name);
        let description = p.description.unwrap_or(cur.description);
        let repo_path = p.repo_path.unwrap_or(cur.repo_path);
        let goal_md = p.goal_md.unwrap_or(cur.goal_md);
        let story_id = p.story_id.unwrap_or(cur.story_id);
        let skills = p.skills.unwrap_or(cur.skills);
        let integration_branch = p.integration_branch.unwrap_or(cur.integration_branch);
        let origin_channel = p.origin_channel.unwrap_or(cur.origin_channel);
        let origin_chat = p.origin_chat.unwrap_or(cur.origin_chat);
        let origin_thread = p.origin_thread.unwrap_or(cur.origin_thread);
        let status = p.status.unwrap_or(cur.status);
        let order_idx = p.order_idx.unwrap_or(cur.order_idx);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarm_projects SET name = ?, description = ?, repo_path = ?, goal_md = ?,
                story_id = ?, skills_json = ?, integration_branch = ?, origin_channel = ?,
                origin_chat = ?, origin_thread = ?, status = ?, order_idx = ?, updated_at = ?,
                context_version = context_version + 1,
                prompt_updated_by = COALESCE(?, prompt_updated_by),
                prompt_updated_at = CASE WHEN ? IS NULL THEN prompt_updated_at ELSE ? END
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&description)
        .bind(&repo_path)
        .bind(&goal_md)
        .bind(&story_id)
        .bind(skills.to_string())
        .bind(&integration_branch)
        .bind(&origin_channel)
        .bind(&origin_chat)
        .bind(&origin_thread)
        .bind(&status)
        .bind(order_idx)
        .bind(&now)
        .bind(&p.prompt_updated_by)
        .bind(&p.prompt_updated_by)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        // Same unique `story_id` index as `create_project`: two swarm agents
        // racing to mint the project's epic both call `update_project(story_id)`
        // — the loser must see a Conflict so it can re-read the project and file
        // under the winner's epic instead of surfacing a 500.
        .map_err(dberr_unique(
            "update project",
            "a swarm project is already linked to this story",
        ))?;
        self.get_project(id).await
    }

    /// Link `story_id` to project `id` ONLY if the project has no story yet.
    /// `Ok(true)` = linked by this call; `Ok(false)` = someone else linked first
    /// (re-read the project and use theirs). This is the epic-tree ingest race
    /// guard: two agents of the same project racing to mint its epic must not
    /// last-writer-win each other, and a story already rooting ANOTHER project
    /// surfaces as `Conflict` via the unique index.
    pub async fn link_story_if_unlinked(&self, id: &Id, story_id: &Id) -> Result<bool> {
        let now = fmt(Utc::now());
        let res = sqlx::query(
            "UPDATE swarm_projects SET story_id = ?, updated_at = ?
             WHERE id = ? AND story_id IS NULL",
        )
        .bind(story_id)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr_unique(
            "link project story",
            "a swarm project is already linked to this story",
        ))?;
        Ok(res.rows_affected() == 1)
    }

    pub async fn delete_project(&self, id: &Id) -> Result<()> {
        // A removed project takes its board history with it: tasks, their and
        // the project's goals, AND the project-scoped feed. Runs stay — they
        // are the spend/audit record. One transaction, so a failure midway
        // never leaves a project whose tasks are already gone.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("delete project begin"))?;
        sqlx::query(
            "DELETE FROM swarm_goals WHERE project_id = ?
                OR task_id IN (SELECT id FROM swarm_tasks WHERE project_id = ?)",
        )
        .bind(id)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("delete project goals"))?;
        sqlx::query("DELETE FROM swarm_tasks WHERE project_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete project tasks"))?;
        sqlx::query("DELETE FROM swarm_messages WHERE project_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete project messages"))?;
        sqlx::query("DELETE FROM swarm_projects WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete project"))?;
        tx.commit().await.map_err(dberr("delete project commit"))?;
        Ok(())
    }

    /// Clear a project's board: delete every task and every project-scoped feed
    /// message, keeping the project itself (and the runs — spend history).
    /// Returns (tasks_deleted, messages_deleted).
    pub async fn clear_project_board(&self, project_id: &Id) -> Result<(u64, u64)> {
        // Task goals go with their tasks (no FK cascades on `swarm_goals.task_id`,
        // so they were orphaned for good); project-level goals stay with the
        // project. One transaction, mirroring `delete_project`.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("clear project begin"))?;
        sqlx::query(
            "DELETE FROM swarm_goals
              WHERE task_id IN (SELECT id FROM swarm_tasks WHERE project_id = ?)",
        )
        .bind(project_id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("clear project task goals"))?;
        let tasks = sqlx::query("DELETE FROM swarm_tasks WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("clear project tasks"))?
            .rows_affected();
        let msgs = sqlx::query("DELETE FROM swarm_messages WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("clear project messages"))?
            .rows_affected();
        tx.commit().await.map_err(dberr("clear project commit"))?;
        Ok((tasks, msgs))
    }

    /// Stop every queued/running/waiting run tied to `project_id` (board clear /
    /// project delete): agents must not keep working — and later repopulate —
    /// a board the operator just wiped. Returns the stopped run ids.
    pub async fn stop_active_runs_for_project(&self, project_id: &Id) -> Result<Vec<Id>> {
        let rows = sqlx::query(
            "SELECT id FROM swarm_runs WHERE project_id = ? AND status IN ('queued','running','waiting')",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("select active project runs"))?;
        let ids: Vec<Id> = rows.iter().map(|r| r.get::<Id, _>("id")).collect();
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarm_runs SET status = 'stopped', finished_at = ?
             WHERE project_id = ? AND status IN ('queued','running','waiting')",
        )
        .bind(&now)
        .bind(project_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("stop active project runs"))?;
        Ok(ids)
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

    /// Number of tasks in a swarm (index-only count; perf §15 F4).
    pub async fn count_tasks(&self, swarm_id: &Id) -> Result<i64> {
        sqlx::query_scalar("SELECT COUNT(*) FROM swarm_tasks WHERE swarm_id = ?")
            .bind(swarm_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("count swarm tasks"))
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
                order_idx = ?, updated_at = ?,
                prompt_updated_by = COALESCE(?, prompt_updated_by),
                prompt_updated_at = CASE WHEN ? IS NULL THEN prompt_updated_at ELSE ? END
             WHERE id = ?",
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
        .bind(&p.prompt_updated_by)
        .bind(&p.prompt_updated_by)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update task"))?;
        self.get_task(id).await
    }

    /// Compare-and-set a task's status: flips it to `status` only while it is
    /// one of `expected`; `false` = nothing written (someone moved it). A
    /// verification controller finishing minutes later must not overwrite an
    /// operator's cancel / back-to-todo with `done` (S4-07).
    pub async fn set_task_status_if(
        &self,
        id: &Id,
        expected: &[&str],
        status: &str,
    ) -> Result<bool> {
        let placeholders = vec!["?"; expected.len().max(1)].join(",");
        let sql = format!(
            "UPDATE swarm_tasks SET status = ?, updated_at = ? WHERE id = ? AND status IN ({placeholders})"
        );
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(status)
            .bind(fmt(Utc::now()))
            .bind(id);
        if expected.is_empty() {
            q = q.bind("");
        }
        for st in expected {
            q = q.bind(*st);
        }
        let res = q
            .execute(&self.pool)
            .await
            .map_err(dberr("set task status if"))?;
        Ok(res.rows_affected() > 0)
    }

    /// Atomically claim a `todo` task for a turn: flip it to `in_progress` (and
    /// assign `assignee` when the task has no assignee yet) ONLY if it is still
    /// `todo`. Returns whether this caller won the claim — the coordinator's
    /// old check-then-act (`agent_has_active_run` → bump → unconditional
    /// `update_task`) let two overlapping ticks (a restarted coordinator whose
    /// old loop was mid-tick) dispatch the same task twice.
    pub async fn claim_task(&self, id: &Id, assignee: Option<&Id>) -> Result<bool> {
        let now = fmt(Utc::now());
        let res = sqlx::query(
            "UPDATE swarm_tasks
             SET status = 'in_progress',
                 assignee_agent_id = COALESCE(assignee_agent_id, ?),
                 updated_at = ?
             WHERE id = ? AND status = 'todo'",
        )
        .bind(assignee.map(|a| a.as_str()))
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("claim task"))?;
        Ok(res.rows_affected() == 1)
    }

    /// Give back one attempt (never below 0) — for a turn that was cut short
    /// by a swarm PAUSE, which must not count against `max_attempts` (pausing
    /// twice used to move a task to `blocked`).
    pub async fn refund_task_attempt(&self, id: &Id) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarm_tasks SET attempts = MAX(attempts - 1, 0), updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("refund task attempt"))?;
        Ok(())
    }

    pub async fn delete_task(&self, id: &Id) -> Result<()> {
        // The task's goals (and their verdicts) go with it: `swarm_goals.task_id`
        // has no FK, and nothing lists or prunes a goal of a deleted task.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("delete task begin"))?;
        sqlx::query("DELETE FROM swarm_goals WHERE task_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete task goals"))?;
        sqlx::query("DELETE FROM swarm_tasks WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete task"))?;
        tx.commit().await.map_err(dberr("delete task commit"))?;
        Ok(())
    }

    /// True when `id` names a person (a `users` row) — how the runtime tells a
    /// human task originator from an agent id in `swarm_tasks.created_by`
    /// (agent-created handoffs/reviews stamp the agent's id there).
    pub async fn user_exists(&self, id: &str) -> Result<bool> {
        let hit: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("user exists"))?;
        Ok(hit.is_some())
    }

    /// Record `user` as the last person who changed what a turn on this task
    /// executes — its title, description or labels, or one of its goals
    /// (access-control-5-01, migration 0195). `acting_user` prefers this
    /// person over the task's originator.
    pub async fn set_task_prompt_editor(&self, id: &Id, user: &str) -> Result<()> {
        sqlx::query(
            "UPDATE swarm_tasks SET prompt_updated_by = ?, prompt_updated_at = ? WHERE id = ?",
        )
        .bind(user)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set task prompt editor"))?;
        Ok(())
    }

    /// The last person who rewrote the prompt text of a task, project or
    /// swarm, with when (access-control-6-02, migration 0196). `None` =
    /// nobody since it was created. A task stamped by the 0195 build has no
    /// time: it reads as the task's `updated_at`.
    pub async fn prompt_edit(
        &self,
        scope: PromptScope,
        id: &Id,
    ) -> Result<Option<(Id, DateTime<Utc>)>> {
        let sql = match scope {
            PromptScope::Task => {
                "SELECT prompt_updated_by, COALESCE(prompt_updated_at, updated_at)
                 FROM swarm_tasks WHERE id = ?"
            }
            PromptScope::Project => {
                "SELECT prompt_updated_by, prompt_updated_at FROM swarm_projects WHERE id = ?"
            }
            PromptScope::Swarm => {
                "SELECT prompt_updated_by, prompt_updated_at FROM swarms WHERE id = ?"
            }
        };
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("prompt editor"))?;
        Ok(row.and_then(|(who, at)| {
            let who = who.filter(|u| !u.is_empty())?;
            let at = at
                .and_then(|a| DateTime::parse_from_rfc3339(&a).ok())
                .map(|a| a.with_timezone(&Utc))
                .unwrap_or(DateTime::<Utc>::MIN_UTC);
            Some((who, at))
        }))
    }

    /// The last person who rewrote this task's prompt or goals (`None` = nobody
    /// since it was created, or a row written before migration 0195).
    pub async fn task_prompt_editor(&self, id: &Id) -> Result<Option<Id>> {
        let who: Option<Option<String>> =
            sqlx::query_scalar("SELECT prompt_updated_by FROM swarm_tasks WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(dberr("task prompt editor"))?;
        Ok(who.flatten().filter(|u| !u.is_empty()))
    }

    /// Record the last person who edited a goal (`swarm_goals.updated_by`,
    /// migration 0195; `created_by` already names its author).
    pub async fn set_goal_editor(&self, id: &Id, user: &str) -> Result<()> {
        sqlx::query("UPDATE swarm_goals SET updated_by = ? WHERE id = ?")
            .bind(user)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set goal editor"))?;
        Ok(())
    }

    /// Increment a task's attempt counter and return the new value. Called when
    /// the Coordinator (re-)queues a turn so `route_result` can enforce the
    /// per-task `max_attempts` ceiling.
    pub async fn bump_task_attempt(&self, id: &Id) -> Result<i64> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE swarm_tasks SET attempts = attempts + 1, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("bump task attempt"))?;
        let row = sqlx::query("SELECT attempts FROM swarm_tasks WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get task attempts"))?;
        Ok(row.get::<i64, _>("attempts"))
    }

    /// Tasks ready to run: status='todo' with all dependencies done.
    pub async fn ready_tasks(&self, swarm_id: &Id) -> Result<Vec<SwarmTask>> {
        // The coordinator calls this every tick per active swarm. Load only the
        // `todo` rows (full), and just the ids of `done` ones for the dependency
        // check — never every task's description/labels (backlog B6 / SE-10).
        let rows = sqlx::query(
            "SELECT * FROM swarm_tasks WHERE swarm_id = ? AND status = 'todo' \
             ORDER BY order_idx, created_at",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("ready tasks"))?;
        let todo = rows.iter().map(row_to_task).collect::<Result<Vec<_>>>()?;
        let has_deps = todo.iter().any(|t| {
            t.depends_on
                .as_array()
                .is_some_and(|d| d.iter().any(|v| v.is_string()))
        });
        let done: std::collections::HashSet<String> = if has_deps {
            sqlx::query_scalar::<_, String>(
                "SELECT id FROM swarm_tasks WHERE swarm_id = ? AND status = 'done'",
            )
            .bind(swarm_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("ready tasks deps"))?
            .into_iter()
            .collect()
        } else {
            std::collections::HashSet::new()
        };
        let mut ready: Vec<SwarmTask> = todo
            .into_iter()
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
        let rows = sqlx::query("SELECT status FROM swarm_tasks WHERE parent_task_id = ?")
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

    /// Reserve one agent and one capacity slot atomically against the current
    /// lifecycle state. An active swarm is admitted only when `pause_reason`
    /// is null or `""` (a whitespace reason is not empty). `allow_paused`
    /// still admits an ordinary pause (`status = paused` and an empty reason)
    /// and still refuses a nonempty reason and an aborted swarm.
    pub async fn reserve_run(&self, r: NewRun, allow_paused: bool) -> Result<SwarmRun> {
        let id = new_id();
        let mut conn = self.pool.acquire().await.map_err(dberr("reserve run"))?;
        if reserve_run_on(&mut conn, &id, &r, allow_paused).await? == 0 {
            return Err(otto_core::Error::Conflict(
                "swarm stopped, paused with a reason, agent busy, or run capacity exhausted".into(),
            ));
        }
        drop(conn);
        self.get_run(&id).await
    }

    /// Admit one scheduled-agent tick: the run reservation and the cursor
    /// advance commit together, and only while the stored schedule is still
    /// the one the scheduler judged due (`seen`). Only `$.last_run` is written
    /// (`json_set`), so a concurrent edit is never reverted to the scheduler's
    /// stale copy — the old full-row `update_agent` write did exactly that.
    /// `Ok(None)` = the schedule changed under the tick (it is re-judged next
    /// scan); `Err(Conflict)` = swarm not runnable, agent busy, or no capacity.
    /// A refused admission does not advance `$.last_run`.
    pub async fn reserve_scheduled_run(
        &self,
        r: NewRun,
        seen: &Value,
        fired_at: &str,
    ) -> Result<Option<SwarmRun>> {
        let id = new_id();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin scheduled swarm run"))?;
        let raw: Option<String> =
            sqlx::query_scalar("SELECT schedule_json FROM swarm_agents WHERE id = ?")
                .bind(&r.agent_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(dberr("read agent schedule"))?
                .flatten();
        let Some(raw) = raw else { return Ok(None) };
        if serde_json::from_str::<Value>(&raw).ok().as_ref() != Some(seen) {
            return Ok(None);
        }
        if reserve_run_on(&mut tx, &id, &r, false).await? == 0 {
            return Err(otto_core::Error::Conflict(
                "swarm stopped, paused with a reason, agent busy, or run capacity exhausted".into(),
            ));
        }
        let advanced = sqlx::query(
            "UPDATE swarm_agents SET schedule_json = json_set(schedule_json, '$.last_run', ?)
             WHERE id = ? AND schedule_json = ?",
        )
        .bind(fired_at)
        .bind(&r.agent_id)
        .bind(&raw)
        .execute(&mut *tx)
        .await
        .map_err(dberr("advance agent schedule"))?;
        if advanced.rows_affected() == 0 {
            return Ok(None);
        }
        tx.commit()
            .await
            .map_err(dberr("commit scheduled swarm run"))?;
        self.get_run(&id).await.map(Some)
    }

    /// Undo a scheduled reservation that never started a turn.
    ///
    /// `reserve_scheduled_run` commits `$.last_run` with the queued row. For
    /// `once`, that cursor is the already-fired flag, so leaving it consumes
    /// the only shot. For a recurring cadence it is the last served slot, so
    /// leaving it skips the current one. This stops the row only while it is
    /// still `queued`, `kind` and `trigger` are `scheduled`, and `session_id`
    /// is null, then puts `$.last_run` back to `previous_last_run` (removing
    /// the key when that was absent) only when the stored cursor is still
    /// `fired_at`. A session, any other status, or a cursor that has moved is
    /// left unchanged: the turn may have started, or a later edit owns the
    /// schedule. Other schedule keys are not written. Returns whether the
    /// unstarted row was stopped.
    pub async fn release_unstarted_scheduled_occurrence(
        &self,
        run_id: &str,
        agent_id: &str,
        fired_at: &str,
        previous_last_run: Option<&str>,
    ) -> Result<bool> {
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin unstarted scheduled release"))?;
        let stopped = sqlx::query(
            "UPDATE swarm_runs SET status = 'stopped', error = ?, finished_at = ?
             WHERE id = ? AND agent_id = ? AND status = 'queued' AND session_id IS NULL
               AND kind = 'scheduled' AND trigger = 'scheduled'",
        )
        .bind(UNSTARTED_SCHEDULED_STOP)
        .bind(&now)
        .bind(run_id)
        .bind(agent_id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("stop unstarted scheduled run"))?;
        if stopped.rows_affected() != 1 {
            tx.commit()
                .await
                .map_err(dberr("commit unstarted scheduled release"))?;
            return Ok(false);
        }
        let rewind = match previous_last_run {
            Some(previous) => sqlx::query(
                "UPDATE swarm_agents
                 SET schedule_json = json_set(schedule_json, '$.last_run', ?)
                 WHERE id = ? AND json_extract(schedule_json, '$.last_run') = ?",
            )
            .bind(previous)
            .bind(agent_id)
            .bind(fired_at)
            .execute(&mut *tx)
            .await
            .map_err(dberr("restore scheduled cursor"))?,
            None => sqlx::query(
                "UPDATE swarm_agents
                 SET schedule_json = json_remove(schedule_json, '$.last_run')
                 WHERE id = ? AND json_extract(schedule_json, '$.last_run') = ?",
            )
            .bind(agent_id)
            .bind(fired_at)
            .execute(&mut *tx)
            .await
            .map_err(dberr("clear scheduled cursor"))?,
        };
        if rewind.rows_affected() != 1 {
            tracing::debug!(
                run = %run_id,
                agent = %agent_id,
                "unstarted scheduled run stopped; cursor had already moved"
            );
        }
        tx.commit()
            .await
            .map_err(dberr("commit unstarted scheduled release"))?;
        Ok(true)
    }

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

    /// Just a run's `status` (`None` = no such run). The readiness / submit
    /// waits poll this every 250 ms for minutes; a full [`Self::get_run`]
    /// decoded `result_json`/`summary`/`error` on every poll to read one
    /// column.
    pub async fn run_status(&self, id: &Id) -> Result<Option<String>> {
        sqlx::query_scalar("SELECT status FROM swarm_runs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get run status"))
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

    /// [`update_run`] as a compare-and-set: applies only while the run's
    /// status is one of `expected`, returning `None` (nothing written)
    /// otherwise. A turn that finishes after the operator stopped it (or a
    /// pause cut it) must not overwrite `stopped` with `done` — nor a Stop
    /// clobber a run that already finished.
    pub async fn update_run_if_status(
        &self,
        id: &Id,
        expected: &[&str],
        p: RunPatch,
    ) -> Result<Option<SwarmRun>> {
        let cur = self.get_run(id).await?;
        if !expected.contains(&cur.status.as_str()) {
            return Ok(None);
        }
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
        let placeholders = vec!["?"; expected.len()].join(",");
        let sql = format!(
            "UPDATE swarm_runs SET session_id = ?, status = ?, attempt = ?, summary = ?,
                result_json = ?, error = ?, tokens_input = ?, tokens_output = ?, cost_usd = ?,
                started_at = ?, finished_at = ? WHERE id = ? AND status IN ({placeholders})"
        );
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
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
            .bind(id);
        for st in expected {
            q = q.bind(*st);
        }
        let res = q
            .execute(&self.pool)
            .await
            .map_err(dberr("update run (guarded)"))?;
        if res.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_run(id).await.map(Some)
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

    /// Lifetime run count for a swarm. A scheduled row stopped before a session
    /// does not count. Drives the `max_total_runs` budget together with
    /// [`Self::swarm_spend`], which also adds runs retention has pruned.
    pub async fn total_run_count(&self, swarm_id: &Id) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(*) AS n FROM swarm_runs WHERE swarm_id = ? AND {}",
            not_unstarted_scheduled_stop("")
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(swarm_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("total run count"))?;
        Ok(row.get::<i64, _>("n"))
    }

    /// Summed cost (USD) across a swarm's runs. Best-effort/soft — `cost_usd` may
    /// be 0/NULL until usage attribution lands. Drives the `max_cost_usd` budget.
    pub async fn total_cost(&self, swarm_id: &Id) -> Result<f64> {
        let sql = format!(
            "SELECT COALESCE(SUM(cost_usd), 0.0) AS c FROM swarm_runs WHERE swarm_id = ? AND {}",
            not_unstarted_scheduled_stop("")
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
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

    /// Agents of a swarm with a live (`queued`/`running`/`waiting`) run — the
    /// coordinator / scheduler / utilization read this ONCE per pass instead
    /// of an [`Self::agent_has_active_run`] COUNT per ready task or agent.
    /// One range read on `idx_swarm_runs_swarm(swarm_id, status)`.
    pub async fn busy_agents(&self, swarm_id: &Id) -> Result<std::collections::HashSet<Id>> {
        let ids: Vec<Id> = sqlx::query_scalar(
            "SELECT DISTINCT agent_id FROM swarm_runs \
             WHERE swarm_id = ? AND status IN ('queued','running','waiting')",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("busy agents"))?;
        Ok(ids.into_iter().collect())
    }

    /// Task count per status for a swarm (`COUNT … GROUP BY status`) — no
    /// task row is decoded.
    pub async fn task_status_counts(
        &self,
        swarm_id: &Id,
    ) -> Result<std::collections::HashMap<String, i64>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT status, COUNT(*) FROM swarm_tasks WHERE swarm_id = ? GROUP BY status",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("task status counts"))?;
        Ok(rows.into_iter().collect())
    }

    pub async fn list_runs(&self, f: &RunFilter) -> Result<Vec<SwarmRun>> {
        self.list_runs_projected(f, false).await
    }

    /// Retire task-owned verifier/fixer turns before releasing the lifecycle gate.
    pub async fn stop_verification_runs(&self, task_id: &Id) -> Result<()> {
        sqlx::query("UPDATE swarm_runs SET status='stopped', finished_at=?, error='task changed'
                     WHERE task_id=? AND kind IN ('verify','fix') AND status IN ('queued','running','waiting')")
            .bind(Utc::now().to_rfc3339()).bind(task_id).execute(&self.pool).await
            .map_err(dberr("stop task verification runs"))?;
        Ok(())
    }

    /// Successful turns whose local task routing was interrupted. These retain
    /// their task claim during restart recovery: replay the durable result, never
    /// the agent's work. Only the latest task run can own that claim.
    pub async fn pending_task_results(&self, swarm_id: &Id) -> Result<Vec<SwarmRun>> {
        let rows = sqlx::query(
            "SELECT r.* FROM swarm_runs r JOIN swarm_tasks t ON t.id = r.task_id
             WHERE t.swarm_id = ? AND t.status IN ('in_progress','in_review')
               AND r.id = (SELECT latest.id FROM swarm_runs latest WHERE latest.task_id = t.id
                           ORDER BY latest.enqueued_at DESC, latest.rowid DESC LIMIT 1)
               AND r.status = 'done' AND r.kind IN ('task','planning')
               AND json_valid(r.result_json)
               AND json_extract(r.result_json, '$.status') IN ('done','blocked','needs_review','in_progress')
               AND COALESCE(json_extract(r.result_json, '$._routing_complete'), 0) = 0",
        ).bind(swarm_id).fetch_all(&self.pool).await.map_err(dberr("pending task results"))?;
        rows.iter().map(row_to_run).collect()
    }

    /// Checkpoint result application without replacing usage or other result
    /// metadata that may have been backfilled concurrently.
    pub async fn finish_task_routing(&self, run_id: &Id) -> Result<()> {
        sqlx::query(
            "UPDATE swarm_runs SET result_json = json_set(result_json, '$._routing_complete', 1)
                     WHERE id = ? AND status = 'done' AND json_valid(result_json)",
        )
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("finish task routing"))?;
        Ok(())
    }

    /// The run list WITHOUT `result_json` (except `kind='recruit'`, whose
    /// Runs-list "Hire" button reads the proposal). A run's result blob is
    /// ~8 KB and the list is 500 rows, reloaded on every open/resync/run —
    /// clients that need one run's result read `GET /swarm/runs/{rid}`
    /// (backlog B6 / SE-09).
    pub async fn list_runs_lite(&self, f: &RunFilter) -> Result<Vec<SwarmRun>> {
        self.list_runs_projected(f, true).await
    }

    async fn list_runs_projected(&self, f: &RunFilter, lite: bool) -> Result<Vec<SwarmRun>> {
        let mut sql = String::from(if lite {
            "SELECT id, swarm_id, workspace_id, project_id, task_id, agent_id, session_id, \
             kind, trigger, status, attempt, summary, \
             CASE WHEN kind = 'recruit' THEN result_json END AS result_json, \
             error, tokens_input, tokens_output, cost_usd, enqueued_at, started_at, \
             finished_at FROM swarm_runs WHERE 1=1"
        } else {
            "SELECT * FROM swarm_runs WHERE 1=1"
        });
        if f.workspace_id.is_some() {
            sql.push_str(" AND workspace_id = ?");
        }
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
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()));
        if let Some(v) = &f.workspace_id {
            q = q.bind(v);
        }
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

    /// `task_id → session_id` of each task's most recent run that has a
    /// session — the only thing the dependency graph needs from runs. Two
    /// narrow columns via the `(swarm_id, enqueued_at)` index instead of 500
    /// full rows with their `result_json` parsed (backlog B6 / SE-06).
    pub async fn latest_task_sessions(
        &self,
        swarm_id: &Id,
    ) -> Result<std::collections::HashMap<String, String>> {
        // One row per task (perf §15 F11): SQLite's bare-column rule takes
        // `session_id` from the row holding `MAX(enqueued_at)`.
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT task_id, session_id, MAX(enqueued_at) FROM swarm_runs \
             WHERE swarm_id = ? AND task_id IS NOT NULL AND session_id IS NOT NULL \
             GROUP BY task_id",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("latest task sessions"))?;
        Ok(rows.into_iter().map(|(tid, sid, _)| (tid, sid)).collect())
    }

    /// Fail every swarm run left in a non-terminal state (daemon-startup
    /// recovery): a run's background task dies with the process, so an orphaned
    /// `queued`/`running`/`waiting` row would otherwise permanently consume the
    /// parallel cap (and block its agent's "one turn at a time" gate). Mirrors
    /// the review/skill-eval recovery. Returns the number of rows updated.
    /// Mark non-terminal runs as `stopped` (NOT `error`) on a daemon restart —
    /// they were interrupted by the restart, not a real agent failure, and the
    /// coordinator re-runs the task. The note is kept in `error` for context.
    ///
    /// The turn's `route_result` died with the process too, so its task would
    /// sit `in_progress` forever (`ready_tasks` only selects `todo`) — the
    /// promised re-run never happened and its dependants stayed blocked. Tasks
    /// left `in_progress` with no active run go back to `todo`; a delegated
    /// parent with children still open is legitimately `in_progress` and
    /// stays.
    pub async fn fail_running(&self, error: &str) -> Result<u64> {
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("begin swarm recovery"))?;
        // A turn the restart cut short is not the task's fault: give back the
        // attempt it was charged at dispatch (floor 0, like a pause refund) in
        // the same transaction that stops the run, or every daemon restart
        // walked a task toward its `max_attempts` ceiling. Only the turn kinds
        // that were charged (`task` / `planning`): a cut-short verify or fix
        // turn charged nothing, and refunding it handed the task a free retry.
        sqlx::query(
            "UPDATE swarm_tasks SET attempts = MAX(attempts - 1, 0), updated_at = ?
             WHERE id IN (SELECT task_id FROM swarm_runs
                          WHERE task_id IS NOT NULL
                            AND kind IN ('task','planning')
                            AND status IN ('queued','running','waiting'))",
        )
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(dberr("refund interrupted swarm attempts"))?;
        let res = sqlx::query(
            "UPDATE swarm_runs SET status = 'stopped', error = ?, finished_at = ?
             WHERE status IN ('queued','running','waiting')",
        )
        .bind(error)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(dberr("fail running swarm runs"))?;
        tx.commit().await.map_err(dberr("commit swarm recovery"))?;
        let reset = sqlx::query(
            "UPDATE swarm_tasks SET status = 'todo', updated_at = ?
             WHERE status = 'in_progress'
               AND NOT EXISTS (SELECT 1 FROM swarm_runs r
                               WHERE r.task_id = swarm_tasks.id
                                 AND r.status IN ('queued','running','waiting'))
               AND NOT EXISTS (SELECT 1 FROM swarm_tasks c
                               WHERE c.parent_task_id = swarm_tasks.id
                                 AND c.status NOT IN ('done','cancelled'))
               AND NOT EXISTS (SELECT 1 FROM swarm_runs r
                               WHERE r.id = (SELECT latest.id FROM swarm_runs latest
                                             WHERE latest.task_id = swarm_tasks.id
                                             ORDER BY latest.enqueued_at DESC, latest.rowid DESC LIMIT 1)
                                 AND r.status = 'done' AND r.kind IN ('task','planning')
                                 AND json_valid(r.result_json)
                                 AND json_extract(r.result_json, '$.status') IN ('done','blocked','needs_review','in_progress')
                                 AND COALESCE(json_extract(r.result_json, '$._routing_complete'), 0) = 0)",
        )
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("reset stranded swarm tasks"))?;
        if reset.rows_affected() > 0 {
            tracing::info!(
                "swarm recovery: {} task(s) left in_progress by the restart are back in todo",
                reset.rows_affected()
            );
        }
        Ok(res.rows_affected())
    }

    /// Stop every in-flight run of a swarm with `reason` in `error` (a pause
    /// writes "paused" so the task routing knows the turn was cut short on
    /// purpose, not failed). Returns the stopped run ids.
    pub async fn stop_active_runs_with_reason(
        &self,
        swarm_id: &Id,
        reason: &str,
    ) -> Result<Vec<Id>> {
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
            "UPDATE swarm_runs SET status = 'stopped', error = ?, finished_at = ?
             WHERE swarm_id = ? AND status IN ('queued','running','waiting')",
        )
        .bind(reason)
        .bind(&now)
        .bind(swarm_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("stop active runs"))?;
        Ok(ids)
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
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(swarm_id);
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

    // -- Goals --------------------------------------------------------------

    pub async fn create_goal(&self, g: NewGoal) -> Result<SwarmGoal> {
        self.create_goal_by(g, None).await
    }

    /// [`Self::create_goal`] for a person's edit: the goal row, its
    /// `updated_by` and its owner's prompt editor (task, else project) commit
    /// in ONE transaction, so a goal never lands without the identity its
    /// verification turn must act as (concurrency-async-6-03).
    pub async fn create_goal_by(&self, g: NewGoal, editor: Option<&str>) -> Result<SwarmGoal> {
        let id = new_id();
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("create goal tx"))?;
        sqlx::query(
            "INSERT INTO swarm_goals (id, swarm_id, workspace_id, project_id, task_id, kind, title,
                description, metric, comparator, target_value, block_value, verify_cmd, max_retries,
                blocking, status, iterations, order_idx, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', 0, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&g.swarm_id)
        .bind(&g.workspace_id)
        .bind(&g.project_id)
        .bind(&g.task_id)
        .bind(&g.kind)
        .bind(&g.title)
        .bind(&g.description)
        .bind(&g.metric)
        .bind(&g.comparator)
        .bind(g.target_value)
        .bind(g.block_value)
        .bind(&g.verify_cmd)
        .bind(g.max_retries)
        .bind(g.blocking as i64)
        .bind(g.order_idx)
        .bind(&g.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(dberr("create goal"))?;
        if let Some(user) = editor {
            stamp_goal_editor(
                &mut tx,
                &id,
                g.task_id.as_ref(),
                g.project_id.as_ref(),
                None,
                user,
            )
            .await?;
        }
        tx.commit().await.map_err(dberr("create goal commit"))?;
        self.get_goal(&id).await
    }

    pub async fn get_goal(&self, id: &Id) -> Result<SwarmGoal> {
        let row = sqlx::query("SELECT * FROM swarm_goals WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get goal"))?;
        row_to_goal(&row)
    }

    /// Explicit goals attached to a task, ordered.
    pub async fn list_goals_for_task(&self, task_id: &Id) -> Result<Vec<SwarmGoal>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_goals WHERE task_id = ? ORDER BY order_idx, created_at",
        )
        .bind(task_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list task goals"))?;
        rows.iter().map(row_to_goal).collect()
    }

    /// Explicit goals attached to a project (not a specific task), ordered.
    pub async fn list_goals_for_project(&self, project_id: &Id) -> Result<Vec<SwarmGoal>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_goals WHERE project_id = ? AND task_id IS NULL
             ORDER BY order_idx, created_at",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list project goals"))?;
        rows.iter().map(row_to_goal).collect()
    }

    /// Swarm-level standing-goal templates (task_id IS NULL, project_id IS NULL).
    pub async fn list_standing_goals(&self, swarm_id: &Id) -> Result<Vec<SwarmGoal>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_goals WHERE swarm_id = ? AND kind = 'standing'
             AND task_id IS NULL AND project_id IS NULL ORDER BY order_idx, created_at",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list standing goals"))?;
        rows.iter().map(row_to_goal).collect()
    }

    pub async fn update_goal(&self, id: &Id, p: GoalPatch) -> Result<SwarmGoal> {
        let cur = self.get_goal(id).await?;
        let title = p.title.unwrap_or(cur.title);
        let description = p.description.unwrap_or(cur.description);
        let metric = p.metric.unwrap_or(cur.metric);
        let comparator = p.comparator.unwrap_or(cur.comparator);
        let target_value = p.target_value.unwrap_or(cur.target_value);
        let block_value = p.block_value.unwrap_or(cur.block_value);
        let verify_cmd = p.verify_cmd.unwrap_or(cur.verify_cmd);
        let max_retries = p.max_retries.unwrap_or(cur.max_retries);
        let blocking = p.blocking.unwrap_or(cur.blocking);
        let status = p.status.unwrap_or(cur.status);
        let verdict = p.verdict.unwrap_or(cur.verdict);
        let iterations = p.iterations.unwrap_or(cur.iterations);
        let order_idx = p.order_idx.unwrap_or(cur.order_idx);
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("update goal tx"))?;
        sqlx::query(
            "UPDATE swarm_goals SET title = ?, description = ?, metric = ?, comparator = ?,
                target_value = ?, block_value = ?, verify_cmd = ?, max_retries = ?, blocking = ?,
                status = ?, verdict_json = ?, iterations = ?, order_idx = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(&description)
        .bind(&metric)
        .bind(&comparator)
        .bind(target_value)
        .bind(block_value)
        .bind(&verify_cmd)
        .bind(max_retries)
        .bind(blocking as i64)
        .bind(&status)
        .bind(verdict.map(|v| v.to_string()))
        .bind(iterations)
        .bind(order_idx)
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("update goal"))?;
        if let Some(user) = p.updated_by.as_deref() {
            // A standing template (no task, no project) is the swarm's text.
            let swarm =
                (cur.task_id.is_none() && cur.project_id.is_none()).then_some(&cur.swarm_id);
            stamp_goal_editor(
                &mut tx,
                id,
                cur.task_id.as_ref(),
                cur.project_id.as_ref(),
                swarm,
                user,
            )
            .await?;
        }
        tx.commit().await.map_err(dberr("update goal commit"))?;
        self.get_goal(id).await
    }

    /// Replace a swarm's standing-goal templates in ONE transaction (S4-24):
    /// concurrent PUTs serialize on SQLite's write lock instead of
    /// interleaving their delete/insert into a duplicated set, and a failed
    /// insert rolls the whole replace back (the caller sees the error).
    pub async fn replace_standing_goals(
        &self,
        swarm_id: &Id,
        goals: Vec<NewGoal>,
    ) -> Result<Vec<SwarmGoal>> {
        self.replace_standing_goals_by(swarm_id, goals, None).await
    }

    /// [`Self::replace_standing_goals`] for a person's edit: the swarm's
    /// prompt editor is stamped in the same transaction (the standing goals'
    /// `verify_cmd`s run in every task's verification turn —
    /// access-control-6-02).
    pub async fn replace_standing_goals_by(
        &self,
        swarm_id: &Id,
        goals: Vec<NewGoal>,
        editor: Option<&str>,
    ) -> Result<Vec<SwarmGoal>> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("standing goals tx"))?;
        sqlx::query(
            "DELETE FROM swarm_goals WHERE swarm_id = ? AND kind = 'standing'
             AND task_id IS NULL AND project_id IS NULL",
        )
        .bind(swarm_id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("clear standing goals"))?;
        let now = fmt(Utc::now());
        for g in goals {
            sqlx::query(
                "INSERT INTO swarm_goals (id, swarm_id, workspace_id, project_id, task_id, kind, title,
                    description, metric, comparator, target_value, block_value, verify_cmd, max_retries,
                    blocking, status, iterations, order_idx, created_by, created_at, updated_at)
                 VALUES (?, ?, ?, NULL, NULL, 'standing', ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', 0, ?, ?, ?, ?)",
            )
            .bind(new_id())
            .bind(swarm_id)
            .bind(&g.workspace_id)
            .bind(&g.title)
            .bind(&g.description)
            .bind(&g.metric)
            .bind(&g.comparator)
            .bind(g.target_value)
            .bind(g.block_value)
            .bind(&g.verify_cmd)
            .bind(g.max_retries)
            .bind(g.blocking as i64)
            .bind(g.order_idx)
            .bind(&g.created_by)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(dberr("insert standing goal"))?;
        }
        if let Some(user) = editor {
            stamp_prompt_editor(&mut tx, PromptScope::Swarm, swarm_id, user, &now).await?;
        }
        tx.commit().await.map_err(dberr("standing goals commit"))?;
        self.list_standing_goals(swarm_id).await
    }

    pub async fn delete_goal(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM swarm_goals WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete goal"))?;
        Ok(())
    }

    // -- Channel triggers ---------------------------------------------------

    pub async fn create_trigger(&self, t: NewTrigger) -> Result<SwarmChannelTrigger> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO swarm_channel_triggers (id, swarm_id, workspace_id, channel, match_chat,
                keyword, repo_path, auto_start, reply, enabled, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&t.swarm_id)
        .bind(&t.workspace_id)
        .bind(&t.channel)
        .bind(&t.match_chat)
        .bind(&t.keyword)
        .bind(&t.repo_path)
        .bind(t.auto_start as i64)
        .bind(t.reply as i64)
        .bind(t.enabled as i64)
        .bind(&t.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create trigger"))?;
        self.get_trigger(&id).await
    }

    pub async fn get_trigger(&self, id: &Id) -> Result<SwarmChannelTrigger> {
        let row = sqlx::query("SELECT * FROM swarm_channel_triggers WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get trigger"))?;
        row_to_trigger(&row)
    }

    pub async fn list_triggers(&self, swarm_id: &Id) -> Result<Vec<SwarmChannelTrigger>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_channel_triggers WHERE swarm_id = ? ORDER BY created_at",
        )
        .bind(swarm_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list triggers"))?;
        rows.iter().map(row_to_trigger).collect()
    }

    /// Find an enabled trigger for an inbound message: matches workspace + channel,
    /// and (if set) the specific chat. Keyword matching is done by the caller.
    /// Most-recently-created wins.
    pub async fn find_triggers(
        &self,
        workspace_id: &Id,
        channel: &str,
    ) -> Result<Vec<SwarmChannelTrigger>> {
        let rows = sqlx::query(
            "SELECT * FROM swarm_channel_triggers WHERE workspace_id = ? AND channel = ?
             AND enabled = 1 ORDER BY created_at DESC",
        )
        .bind(workspace_id)
        .bind(channel)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("find triggers"))?;
        rows.iter().map(row_to_trigger).collect()
    }

    pub async fn update_trigger(&self, id: &Id, p: TriggerPatch) -> Result<SwarmChannelTrigger> {
        let cur = self.get_trigger(id).await?;
        let channel = p.channel.unwrap_or(cur.channel);
        let match_chat = p.match_chat.unwrap_or(cur.match_chat);
        let keyword = p.keyword.unwrap_or(cur.keyword);
        let repo_path = p.repo_path.unwrap_or(cur.repo_path);
        let auto_start = p.auto_start.unwrap_or(cur.auto_start);
        let reply = p.reply.unwrap_or(cur.reply);
        let enabled = p.enabled.unwrap_or(cur.enabled);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE swarm_channel_triggers SET channel = ?, match_chat = ?, keyword = ?,
                repo_path = ?, auto_start = ?, reply = ?, enabled = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&channel)
        .bind(&match_chat)
        .bind(&keyword)
        .bind(&repo_path)
        .bind(auto_start as i64)
        .bind(reply as i64)
        .bind(enabled as i64)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update trigger"))?;
        self.get_trigger(id).await
    }

    pub async fn delete_trigger(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM swarm_channel_triggers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete trigger"))?;
        Ok(())
    }
}

/// Error stored on a scheduled row that was reserved and then stopped before
/// any session. Admission, spend, and retention do not treat it as a consumed
/// lifetime run. A session, or any other error, still counts.
pub(crate) const UNSTARTED_SCHEDULED_STOP: &str = "stopped before the turn started";

/// SQL predicate: `alias` (or the bare column when empty) is a run that still
/// spends the lifetime budget. `error IS` keeps a NULL error, and any other
/// text, inside the count. Only the exact never-started marker is refunded.
pub(crate) fn not_unstarted_scheduled_stop(alias: &str) -> String {
    let prefix = if alias.is_empty() {
        String::new()
    } else {
        format!("{alias}.")
    };
    format!(
        "NOT ({prefix}status = 'stopped' AND {prefix}session_id IS NULL AND {prefix}kind = 'scheduled' AND {prefix}trigger = 'scheduled' AND {prefix}error IS '{UNSTARTED_SCHEDULED_STOP}')"
    )
}

/// The atomic gated INSERT behind [`SwarmRepo::reserve_run`] (rows affected:
/// 0 = refused). Shared so a caller can run it inside its own transaction.
async fn reserve_run_on(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    r: &NewRun,
    allow_paused: bool,
) -> Result<u64> {
    let budget_runs = not_unstarted_scheduled_stop("");
    let sql = format!(
            "INSERT INTO swarm_runs (id, swarm_id, workspace_id, project_id, task_id, agent_id,
                kind, trigger, status, attempt, enqueued_at)
             SELECT ?, s.id, ?, ?, ?, ?, ?, ?, 'queued',
                (SELECT COUNT(*) FROM swarm_runs WHERE task_id = ?), ?
             FROM swarms s WHERE s.id = ? AND s.workspace_id = ?
               AND COALESCE(s.pause_reason, '') = ''
               AND (s.status = 'active' OR (? AND s.status = 'paused'))
               AND NOT EXISTS (SELECT 1 FROM swarm_runs WHERE agent_id = ? AND status IN ('queued','running','waiting'))
               AND (SELECT COUNT(*) FROM swarm_runs WHERE swarm_id = s.id AND status IN ('queued','running','waiting'))
                   < MAX(1, COALESCE(json_extract(s.config_json, '$.max_parallel_sessions'), 4))
               AND (s.max_total_runs IS NULL OR (
                    (SELECT COUNT(*) FROM swarm_runs WHERE swarm_id = s.id AND {budget_runs})
                    + COALESCE(s.pruned_runs, 0)
               ) < s.max_total_runs)"
        );
    let inserted = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .bind(&r.workspace_id)
        .bind(&r.project_id)
        .bind(&r.task_id)
        .bind(&r.agent_id)
        .bind(&r.kind)
        .bind(&r.trigger)
        .bind(&r.task_id)
        .bind(fmt(Utc::now()))
        .bind(&r.swarm_id)
        .bind(&r.workspace_id)
        .bind(allow_paused)
        .bind(&r.agent_id)
        .execute(&mut *conn)
        .await
        .map_err(dberr("reserve run"))?;
    Ok(inserted.rows_affected())
}

#[cfg(test)]
#[path = "swarm/tests.rs"]
mod tests;
