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

use std::collections::HashMap;

use crate::DbPool;
use chrono::Utc;
use otto_core::{new_id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;

use crate::convert::{dberr, fmt, json};

/// Process cache of [`PersonalAgentsRepo::autonomy`] keyed by (database
/// handle, agent id) — perf N4. Capped (cleared when full: it is a hit-rate
/// aid, not state) and generation-checked so a read that overlapped an
/// invalidation never re-inserts the old config.
mod autonomy_cache {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    use super::AgentAutonomy;

    const CAP: usize = 512;
    type Key = (u64, String);

    static GENERATION: AtomicU64 = AtomicU64::new(0);

    fn map() -> &'static Mutex<HashMap<Key, AgentAutonomy>> {
        static MAP: OnceLock<Mutex<HashMap<Key, AgentAutonomy>>> = OnceLock::new();
        MAP.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(super) fn generation() -> u64 {
        GENERATION.load(Ordering::Acquire)
    }

    pub(super) fn get(key: &Key) -> Option<AgentAutonomy> {
        map().lock().ok()?.get(key).cloned()
    }

    /// Cache `cfg` unless an invalidation happened since `generation` was
    /// read (the row may have changed under the read).
    pub(super) fn put(key: Key, cfg: AgentAutonomy, generation: u64) {
        let Ok(mut m) = map().lock() else { return };
        if GENERATION.load(Ordering::Acquire) != generation {
            return;
        }
        if m.len() >= CAP && !m.contains_key(&key) {
            m.clear();
        }
        m.insert(key, cfg);
    }

    pub(super) fn invalidate(pool: u64, agent_id: &str) {
        GENERATION.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut m) = map().lock() {
            m.remove(&(pool, agent_id.to_string()));
        }
    }
}

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
    /// When the schedule was last (re)armed — created, resumed (it or its
    /// agent), or given a new cadence/timezone. The due check never looks
    /// before it. `None` on pre-0147 rows.
    #[serde(default)]
    pub armed_at: Option<String>,
    /// The schedule's own permission set (0153): `read_only` confines its runs
    /// (no mutating otto.* tools, no sends, writes only inside the agent
    /// folder); `directed` runs under the normal approval + auto-approve rules.
    #[serde(default = "default_permission")]
    pub permission: String,
    pub created_at: String,
    pub updated_at: String,
}

fn default_permission() -> String {
    "directed".into()
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
    /// The permission mode the run executed under (0153): `proactive` (a
    /// standing-goal run — always read-only, feed only, never delivered),
    /// `directed` (Run now / delegation / chat-initiated) or `scheduled`.
    #[serde(default = "default_permission")]
    pub mode: String,
    /// The standing goal a proactive run worked on.
    #[serde(default)]
    pub goal_id: Option<String>,
    /// True when the run's session was confined read-only.
    #[serde(default)]
    pub read_only: bool,
    pub created_at: String,
}

/// Per-agent autonomy config (`personal_agent_autonomy.config_json`, 0153):
/// the proactive budget, standing goals, custom rules and the primary flag.
/// Every field defaults, so a missing row (or an older document) reads as
/// "proactive off, no goals, no rules, not primary".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentAutonomy {
    pub proactive: ProactiveConfig,
    pub goals: Vec<StandingGoal>,
    pub rules: Vec<AgentRule>,
    /// The workspace's primary assistant ("your agent") — at most one per
    /// workspace (the route clears the flag on every other agent).
    pub primary: bool,
}

/// Proactive mode: the agent works its standing goals in the background,
/// STRICTLY read-only, within a daily budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProactiveConfig {
    pub enabled: bool,
    /// Proactive runs per day across all goals (1..=24).
    pub runs_per_day: u32,
    /// Wall-clock cap per proactive run, in minutes (1..=60).
    pub max_minutes: u32,
}

impl Default for ProactiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            runs_per_day: 4,
            max_minutes: 15,
        }
    }
}

/// A goal the agent works on continuously (proactive runs), producing findings
/// into its feed rather than acting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StandingGoal {
    pub id: String,
    pub text: String,
    pub enabled: bool,
    /// Server-maintained: when a proactive run last worked this goal.
    pub last_run_at: Option<String>,
}

impl Default for StandingGoal {
    fn default() -> Self {
        Self {
            id: String::new(),
            text: String::new(),
            enabled: true,
            last_run_at: None,
        }
    }
}

/// A plain-language rule, injected into the agent's instructions and — where
/// it is expressible — enforced at the tool layer (`enforce`, derived by the
/// server from the text; never trusted from the client).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentRule {
    pub id: String,
    pub text: String,
    pub enforce: Option<RuleEnforcement>,
}

/// The enforceable part of a rule: `approval` (force a human approval) or
/// `deny` (refuse) for a MUTATING tool call whose tool name or arguments
/// mention any of `terms`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuleEnforcement {
    pub kind: String,
    pub terms: Vec<String>,
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

/// A room's message volume and recency, for the rooms list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoomActivity {
    pub message_count: i64,
    /// `created_at` of the newest message (`None` for an empty room).
    pub last_message_at: Option<String>,
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
        armed_at: r.get("armed_at"),
        permission: r
            .try_get::<String, _>("permission")
            .unwrap_or_else(|_| default_permission()),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    })
}

/// Longest run `summary` (in characters) the Activity feed shows; the feed
/// projection reads one more so the clip can mark the cut.
pub const FEED_SUMMARY_CHARS: usize = 280;

/// Every `personal_agent_runs` column [`row_to_run`] reads, with `summary`
/// cut to [`FEED_SUMMARY_CHARS`] + 1 characters (perf N5).
const FEED_RUN_COLS: &str = "id, agent_id, schedule_id, workspace_id, status, trigger, \
     started_at, finished_at, substr(summary, 1, 281) AS summary, report_path, report_rel, \
     delivered, delivery_error, error, session_id, report_hash, attempts, skipped_delivery, \
     mode, goal_id, read_only, created_at";

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
        mode: r
            .try_get::<String, _>("mode")
            .unwrap_or_else(|_| default_permission()),
        goal_id: r.try_get("goal_id").unwrap_or(None),
        read_only: r.try_get::<i64, _>("read_only").unwrap_or(0) != 0,
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
    pool: DbPool,
}

impl PersonalAgentsRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
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

    /// User-maintained context is independent of filesystem memory and persona.
    pub async fn context(&self, id: &str) -> Result<(String, String)> {
        let row =
            sqlx::query("SELECT content, version FROM personal_agent_context WHERE agent_id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(dberr("get agent context"))?;
        Ok(row
            .map(|r| (r.get("content"), r.get("version")))
            .unwrap_or_else(|| (String::new(), "missing".into())))
    }

    /// One atomic compare-and-swap; concurrent editors never silently overwrite.
    pub async fn save_context(&self, id: &str, expected: &str, content: &str) -> Result<String> {
        if content.len() > 1024 * 1024 {
            return Err(otto_core::Error::Invalid(
                "agent context exceeds 1 MiB".into(),
            ));
        }
        let version = new_id();
        let result = sqlx::query(
            "INSERT INTO personal_agent_context (agent_id, content, version, updated_at) \
             SELECT ?, ?, ?, ? WHERE ? = 'missing' OR EXISTS \
             (SELECT 1 FROM personal_agent_context WHERE agent_id = ? AND version = ?) \
             ON CONFLICT(agent_id) DO UPDATE SET content = excluded.content, version = excluded.version, \
             updated_at = excluded.updated_at WHERE personal_agent_context.version = ?"
        ).bind(id).bind(content).bind(&version).bind(fmt(Utc::now()))
            .bind(expected).bind(id).bind(expected).bind(expected)
            .execute(&self.pool).await.map_err(dberr("save agent context"))?;
        if result.rows_affected() == 0 {
            return Err(otto_core::Error::Conflict(
                "Context changed since you opened it. Reload and reconcile your edits.".into(),
            ));
        }
        Ok(version)
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
        // The autonomy row cascaded with the agent.
        autonomy_cache::invalidate(self.pool.id(), id);
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
        self.create_schedule_once(s, None, "directed").await
    }

    /// A retry of the same create returns its first committed schedule.
    pub async fn create_schedule_once(
        &self,
        s: NewAgentSchedule,
        request_key: Option<&str>,
        permission: &str,
    ) -> Result<PersonalAgentSchedule> {
        if request_key.is_some_and(|k| k.is_empty() || k.len() > 128 || !k.is_ascii()) {
            return Err(otto_core::Error::Invalid(
                "idempotency_key must contain 1–128 ASCII characters".into(),
            ));
        }
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO personal_agent_schedules (id, agent_id, schedule_json, timezone, \
             directive, enabled, created_at, updated_at, armed_at, request_key, permission) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(agent_id, request_key) DO NOTHING",
        )
        .bind(&id)
        .bind(&s.agent_id)
        .bind(s.schedule.to_string())
        .bind(&s.timezone)
        .bind(&s.directive)
        .bind(s.enabled as i64)
        .bind(&now)
        .bind(&now)
        // Armed at creation: the first fire is the next one after now.
        .bind(&now)
        .bind(request_key)
        .bind(permission)
        .execute(&self.pool)
        .await
        .map_err(dberr("create personal agent schedule"))?;
        let saved = if let Some(key) = request_key {
            let row = sqlx::query(
                "SELECT * FROM personal_agent_schedules WHERE agent_id = ? AND request_key = ?",
            )
            .bind(&s.agent_id)
            .bind(key)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("schedule retry"))?;
            row_to_schedule(&row)?
        } else {
            self.get_schedule(&id).await?
        };
        if saved.schedule != s.schedule
            || saved.timezone != s.timezone
            || saved.directive != s.directive
            || saved.permission != permission
        {
            return Err(otto_core::Error::Conflict(
                "idempotency_key was already used for different schedule settings".into(),
            ));
        }
        Ok(saved)
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
    pub async fn list_enabled_schedules(
        &self,
    ) -> Result<Vec<(PersonalAgentSchedule, PersonalAgent)>> {
        // Perf W3: TWO set queries whatever N is (it was 1 + 2N per minute),
        // reusing the row parsers. A row that fails to parse — or an agent
        // deleted between the reads — is skipped, never failing the tick.
        let sched_rows = sqlx::query(
            "SELECT s.* FROM personal_agent_schedules s \
             JOIN personal_agents a ON a.id = s.agent_id \
             WHERE s.enabled = 1 AND a.enabled = 1",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list enabled personal agent schedules"))?;
        if sched_rows.is_empty() {
            return Ok(vec![]);
        }
        let agent_rows = sqlx::query(
            "SELECT a.* FROM personal_agents a WHERE a.enabled = 1 AND EXISTS \
             (SELECT 1 FROM personal_agent_schedules s WHERE s.agent_id = a.id AND s.enabled = 1)",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list enabled personal agents"))?;
        let agents: std::collections::HashMap<String, PersonalAgent> = agent_rows
            .iter()
            .filter_map(|r| row_to_agent(r).ok())
            .map(|a| (a.id.clone(), a))
            .collect();
        Ok(sched_rows
            .iter()
            .filter_map(|r| row_to_schedule(r).ok())
            .filter_map(|s| agents.get(&s.agent_id).cloned().map(|a| (s, a)))
            .collect())
    }

    pub async fn update_schedule(
        &self,
        id: &str,
        p: AgentSchedulePatch,
    ) -> Result<PersonalAgentSchedule> {
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

    /// Re-arm one schedule at `at` (see `PersonalAgentSchedule::armed_at`);
    /// `reset_once` also forgets a fired `once` (its cursor is the fired flag).
    pub async fn rearm_schedule(&self, id: &str, at: &str, reset_once: bool) -> Result<()> {
        sqlx::query(
            "UPDATE personal_agent_schedules SET armed_at = ?, \
             last_run_at = CASE WHEN ? THEN NULL ELSE last_run_at END WHERE id = ?",
        )
        .bind(at)
        .bind(reset_once)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("rearm personal agent schedule"))?;
        Ok(())
    }

    /// Re-arm every schedule of an agent at `at` — the agent was resumed, so
    /// what its schedules missed while it was paused is not caught up.
    pub async fn rearm_agent_schedules(&self, agent_id: &str, at: &str) -> Result<()> {
        sqlx::query("UPDATE personal_agent_schedules SET armed_at = ? WHERE agent_id = ?")
            .bind(at)
            .bind(agent_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("rearm personal agent schedules"))?;
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
        self.create_run_configured(&new_id(), r, "directed", false, None)
            .await
    }

    /// The engine reserves cancellation for `id` before this atomic insert.
    pub async fn create_run_configured(
        &self,
        id: &str,
        r: NewAgentRun,
        mode: &str,
        read_only: bool,
        goal_id: Option<&str>,
    ) -> Result<PersonalAgentRun> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO personal_agent_runs (id, agent_id, schedule_id, workspace_id, status, \
             trigger, started_at, summary, delivered, created_at, mode, read_only, goal_id) \
             VALUES (?, ?, ?, ?, 'running', ?, ?, '', 0, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&r.agent_id)
        .bind(&r.schedule_id)
        .bind(&r.workspace_id)
        .bind(&r.trigger)
        .bind(&now)
        .bind(&now)
        .bind(mode)
        .bind(read_only as i64)
        .bind(goal_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("create personal agent run"))?;
        self.get_run(id).await
    }

    /// Settle a run. A `None` session id keeps the one recorded when the
    /// run's session opened — a failed run is when the user needs it most.
    pub async fn finish_run(&self, run_id: &str, f: FinishAgentRun) -> Result<()> {
        sqlx::query(
            "UPDATE personal_agent_runs SET status = ?, summary = ?, report_path = ?, \
             report_rel = ?, delivered = ?, delivery_error = ?, error = ?, \
             session_id = COALESCE(?, session_id), \
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

    /// Record the permission mode (and standing goal) a run executes under.
    pub async fn set_run_mode(
        &self,
        run_id: &str,
        mode: &str,
        read_only: bool,
        goal_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE personal_agent_runs SET mode = ?, read_only = ?, goal_id = ? WHERE id = ?",
        )
        .bind(mode)
        .bind(read_only as i64)
        .bind(goal_id)
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set personal agent run mode"))?;
        Ok(())
    }

    /// Proactive runs started at/after `since` (RFC3339), per agent, for
    /// every agent in `agent_ids` in ONE statement (perf N6: the proactive
    /// tick used to ask per agent per minute). Agents without such a run are
    /// absent (count 0). Walks `idx_par_agent` per listed agent.
    pub async fn count_proactive_runs_since(
        &self,
        agent_ids: &[&str],
        since: &str,
    ) -> Result<HashMap<String, i64>> {
        if agent_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let q = format!(
            "SELECT agent_id, COUNT(*) AS n FROM personal_agent_runs \
             WHERE agent_id IN ({}) AND mode = 'proactive' AND started_at >= ? \
             GROUP BY agent_id",
            vec!["?"; agent_ids.len()].join(",")
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(q.as_str()));
        for id in agent_ids {
            query = query.bind(*id);
        }
        let rows = query
            .bind(since)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("count proactive personal agent runs"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>("agent_id"), r.get::<i64, _>("n")))
            .collect())
    }

    /// Runs of `mode` started at/after `since` (RFC3339) — the proactive
    /// daily budget counter.
    pub async fn count_runs_since(&self, agent_id: &str, mode: &str, since: &str) -> Result<i64> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS n FROM personal_agent_runs \
             WHERE agent_id = ? AND mode = ? AND started_at >= ?",
        )
        .bind(agent_id)
        .bind(mode)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("count personal agent runs"))?;
        Ok(row.get("n"))
    }

    /// Set one schedule's permission set (`read_only` | `directed`).
    pub async fn set_schedule_permission(&self, id: &str, permission: &str) -> Result<()> {
        if !matches!(permission, "read_only" | "directed") {
            return Err(otto_core::Error::Invalid(format!(
                "schedule permission must be read_only or directed, got {permission}"
            )));
        }
        sqlx::query("UPDATE personal_agent_schedules SET permission = ? WHERE id = ?")
            .bind(permission)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set personal agent schedule permission"))?;
        Ok(())
    }

    /// Enabled agents whose autonomy config turns proactive mode on, with that
    /// config — the proactive scheduler's scan.
    pub async fn list_proactive(&self) -> Result<Vec<(PersonalAgent, AgentAutonomy)>> {
        let rows = sqlx::query(
            "SELECT a.*, x.config_json AS autonomy_json FROM personal_agents a \
             JOIN personal_agent_autonomy x ON x.agent_id = a.id \
             WHERE a.enabled = 1 AND json_valid(x.config_json) \
               AND json_extract(x.config_json, '$.proactive.enabled') = 1",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list proactive personal agents"))?;
        let mut out = Vec::with_capacity(rows.len());
        for r in &rows {
            let cfg: AgentAutonomy =
                serde_json::from_str(&r.get::<String, _>("autonomy_json")).unwrap_or_default();
            out.push((row_to_agent(r)?, cfg));
        }
        Ok(out)
    }

    /// The agent's autonomy config (defaults when no row exists).
    ///
    /// Cached per database handle (perf N4): the governed pipeline asks for an
    /// agent's rules on EVERY tool call its sessions make, and the config only
    /// changes through [`Self::save_autonomy`] / [`Self::delete`] (the row
    /// cascades with its agent), which invalidate it. A read that raced a
    /// write never caches its (possibly stale) answer.
    pub async fn autonomy(&self, agent_id: &str) -> Result<AgentAutonomy> {
        let key = (self.pool.id(), agent_id.to_string());
        let generation = autonomy_cache::generation();
        if let Some(hit) = autonomy_cache::get(&key) {
            return Ok(hit);
        }
        let row = sqlx::query("SELECT config_json FROM personal_agent_autonomy WHERE agent_id = ?")
            .bind(agent_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get agent autonomy"))?;
        let cfg: AgentAutonomy = row
            .and_then(|r| serde_json::from_str(&r.get::<String, _>("config_json")).ok())
            .unwrap_or_default();
        autonomy_cache::put(key, cfg.clone(), generation);
        Ok(cfg)
    }

    /// Replace the agent's autonomy config.
    pub async fn save_autonomy(&self, agent_id: &str, cfg: &AgentAutonomy) -> Result<()> {
        let body = serde_json::to_string(cfg)
            .map_err(|e| otto_core::Error::Internal(format!("autonomy json: {e}")))?;
        sqlx::query(
            "INSERT INTO personal_agent_autonomy (agent_id, config_json, updated_at) \
             VALUES (?, ?, ?) ON CONFLICT(agent_id) DO UPDATE SET \
             config_json = excluded.config_json, updated_at = excluded.updated_at",
        )
        .bind(agent_id)
        .bind(body)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("save agent autonomy"))?;
        autonomy_cache::invalidate(self.pool.id(), agent_id);
        Ok(())
    }

    /// "Reset agent": drop its schedules and run history and forget its chat
    /// session id. Memory files and the chat session itself are handled by the
    /// server (filesystem + session manager). Returns the removed report paths.
    pub async fn reset_agent(&self, agent_id: &str) -> Result<Vec<String>> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("reset agent: begin"))?;
        let paths: Vec<String> = sqlx::query(
            "SELECT report_path FROM personal_agent_runs WHERE agent_id = ? AND report_path IS NOT NULL",
        )
        .bind(agent_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(dberr("reset agent: runs"))?
        .iter()
        .map(|r| r.get::<String, _>("report_path"))
        .collect();
        for sql in [
            "DELETE FROM personal_agent_runs WHERE agent_id = ?",
            "DELETE FROM personal_agent_schedules WHERE agent_id = ?",
            "UPDATE personal_agents SET chat_session_id = NULL WHERE id = ?",
        ] {
            sqlx::query(sql)
                .bind(agent_id)
                .execute(&mut *tx)
                .await
                .map_err(dberr("reset agent"))?;
        }
        tx.commit().await.map_err(dberr("reset agent: commit"))?;
        Ok(paths)
    }

    pub async fn get_run(&self, run_id: &str) -> Result<PersonalAgentRun> {
        let row = sqlx::query("SELECT * FROM personal_agent_runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("personal agent run not found"))?;
        row_to_run(&row)
    }

    /// The agent's newest `running` run, if any (the activity feed's "Now"
    /// without listing history — perf W4). `summary` is cut in SQL to
    /// [`FEED_SUMMARY_CHARS`] + 1 characters (perf N5) — enough for the
    /// feed's clip to know it was longer.
    pub async fn running_run(&self, agent_id: &str) -> Result<Option<PersonalAgentRun>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {FEED_RUN_COLS} FROM personal_agent_runs \
             WHERE agent_id = ? AND status = 'running' ORDER BY started_at DESC LIMIT 1"
        )))
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("running personal agent run"))?;
        row.as_ref().map(row_to_run).transpose()
    }

    /// [`Self::list_runs`] for the Activity feed (perf N5): the same rows,
    /// with `summary` cut in SQL to [`FEED_SUMMARY_CHARS`] + 1 characters, so
    /// a long report summary is not read and copied only to be clipped.
    pub async fn list_runs_for_feed(
        &self,
        agent_id: &str,
        limit: i64,
    ) -> Result<Vec<PersonalAgentRun>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {FEED_RUN_COLS} FROM personal_agent_runs \
             WHERE agent_id = ? ORDER BY started_at DESC LIMIT ?"
        )))
        .bind(agent_id)
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list personal agent runs (feed)"))?;
        rows.iter().map(row_to_run).collect()
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

    /// Notification baseline from the latest successful non-proactive run.
    /// A failed/absent delivery invalidates the baseline instead of falling back
    /// to an older report. Unchanged skips carry it forward across history pruning;
    /// proactive feed-only runs never establish an outward-delivery baseline.
    pub async fn last_ok_report_hash(
        &self,
        agent_id: &str,
        exclude_run: &str,
    ) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT CASE WHEN delivery_error IS NULL AND (delivered = 1 OR skipped_delivery = 1) \
             THEN report_hash ELSE NULL END AS report_hash FROM personal_agent_runs \
             WHERE agent_id = ? AND status = 'ok' AND mode != 'proactive' \
             AND id != ? ORDER BY started_at DESC, id DESC LIMIT 1",
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
    pool: DbPool,
}

impl AgentRoomsRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    pub async fn create(
        &self,
        workspace_id: &str,
        name: &str,
        created_by: Option<&str>,
    ) -> Result<AgentRoom> {
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
        let rows =
            sqlx::query("SELECT * FROM agent_rooms WHERE workspace_id = ? ORDER BY created_at ASC")
                .bind(ws)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("list agent rooms"))?;
        rows.iter().map(row_to_room).collect()
    }

    /// The rooms `agent_id` is a member of (oldest room first) — what the
    /// agent is told about in its instructions.
    pub async fn list_for_agent(&self, agent_id: &str) -> Result<Vec<AgentRoom>> {
        let rows = sqlx::query(
            "SELECT r.* FROM agent_rooms r \
               JOIN agent_room_members m ON m.room_id = r.id \
             WHERE m.agent_id = ? ORDER BY r.created_at ASC",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list agent rooms for agent"))?;
        rows.iter().map(row_to_room).collect()
    }

    /// Member agent ids of every room in workspace `ws`, keyed by room id
    /// (join order) — one query for the whole rooms list instead of one per
    /// room.
    pub async fn members_by_workspace(&self, ws: &str) -> Result<HashMap<String, Vec<String>>> {
        let rows = sqlx::query(
            "SELECT m.room_id, m.agent_id FROM agent_room_members m \
               JOIN agent_rooms r ON r.id = m.room_id \
             WHERE r.workspace_id = ? ORDER BY m.created_at ASC",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list agent room members by workspace"))?;
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for r in &rows {
            out.entry(r.get("room_id"))
                .or_default()
                .push(r.get("agent_id"));
        }
        Ok(out)
    }

    /// Message count + newest message time of every room in workspace `ws`
    /// that has messages, keyed by room id. Reads the activity denormalized
    /// onto `agent_rooms` (migration 0165, kept by [`Self::add_message`]) —
    /// one indexed pass over the workspace's rooms, never over their messages.
    pub async fn activity_by_workspace(&self, ws: &str) -> Result<HashMap<String, RoomActivity>> {
        let rows = sqlx::query(
            "SELECT id, message_count, last_message_at FROM agent_rooms \
              WHERE workspace_id = ? AND message_count > 0",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("agent room activity"))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("id"),
                    RoomActivity {
                        message_count: r.get("message_count"),
                        last_message_at: r.get("last_message_at"),
                    },
                )
            })
            .collect())
    }

    /// Re-derive the denormalized `message_count` / `last_message_at` of every
    /// room whose stored count drifted from its rows — call after deleting
    /// messages outside [`Self::add_message`] (retention pruning). Returns the
    /// number of rooms corrected.
    pub async fn recount_activity(&self) -> Result<u64> {
        let res = sqlx::query(
            "UPDATE agent_rooms \
                SET message_count = (SELECT COUNT(*) FROM agent_room_messages m \
                                      WHERE m.room_id = agent_rooms.id), \
                    last_message_at = (SELECT MAX(m.created_at) FROM agent_room_messages m \
                                        WHERE m.room_id = agent_rooms.id) \
              WHERE message_count != (SELECT COUNT(*) FROM agent_room_messages m \
                                       WHERE m.room_id = agent_rooms.id)",
        )
        .execute(&self.pool)
        .await
        .map_err(dberr("recount agent room activity"))?;
        Ok(res.rows_affected())
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
        let row =
            sqlx::query("SELECT 1 AS x FROM agent_room_members WHERE room_id = ? AND agent_id = ?")
                .bind(room_id)
                .bind(agent_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(dberr("check agent room membership"))?;
        Ok(row.is_some())
    }

    // -- Messages ------------------------------------------------------------

    /// Append a message and bump the room's denormalized activity in the same
    /// transaction (the rooms list reads `agent_rooms.message_count` /
    /// `last_message_at` instead of scanning messages).
    pub async fn add_message(&self, m: NewRoomMessage) -> Result<AgentRoomMessage> {
        let id = new_id();
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("add agent room message tx"))?;
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
        .execute(&mut *tx)
        .await
        .map_err(dberr("add agent room message"))?;
        sqlx::query(
            "UPDATE agent_rooms SET message_count = message_count + 1, last_message_at = ? \
              WHERE id = ?",
        )
        .bind(&now)
        .bind(&m.room_id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("bump agent room activity"))?;
        tx.commit()
            .await
            .map_err(dberr("add agent room message commit"))?;
        Ok(AgentRoomMessage {
            id,
            room_id: m.room_id,
            author_kind: m.author_kind,
            author_id: m.author_id,
            text: m.text,
            created_at: now,
        })
    }

    /// The insertion `rowid` of message `id` IN room `room_id` (`None` when
    /// unknown here) — the paging cursor, resolved once so the page read is
    /// a plain `(room_id, rowid)` range scan on `idx_arm_room_seq`.
    async fn cursor_seq(&self, room_id: &str, id: &str) -> Result<Option<i64>> {
        let row = sqlx::query(
            "SELECT rowid AS seq FROM agent_room_messages WHERE id = ? AND room_id = ?",
        )
        .bind(id)
        .bind(room_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("agent room message cursor"))?;
        Ok(row.map(|r| r.get("seq")))
    }

    /// Chronological page: messages after the `after` message, oldest first,
    /// capped at `limit`. `after = None` starts from the beginning. Ordering and
    /// the cursor use the table's monotonic `rowid` (insertion order) rather than
    /// the ULID `id` — two messages minted in the same millisecond tie on the
    /// ULID timestamp and would otherwise sort by their random suffix, i.e.
    /// non-deterministically. The cursor is looked up IN THIS ROOM: a cursor
    /// from another room used to shift the page by that room's rowid; now it
    /// is unknown here and the read starts from the beginning.
    pub async fn list_messages(
        &self,
        room_id: &str,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AgentRoomMessage>> {
        let from = match after {
            Some(a) => self.cursor_seq(room_id, a).await?.unwrap_or(0),
            None => 0,
        };
        let rows = sqlx::query(
            "SELECT * FROM agent_room_messages \
             WHERE room_id = ? AND rowid > ? \
             ORDER BY rowid ASC LIMIT ?",
        )
        .bind(room_id)
        .bind(from)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list agent room messages"))?;
        rows.iter().map(row_to_message).collect()
    }

    /// The `limit` messages immediately BEFORE `before` (or the room's newest
    /// `limit` when `before` is `None`), returned oldest first. Lets a client
    /// open a busy room on its tail and page backwards, instead of walking
    /// forward from the first message (backlog B6 / SA-08).
    pub async fn list_messages_before(
        &self,
        room_id: &str,
        before: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AgentRoomMessage>> {
        // Two plain statements instead of one `? IS NULL OR …` predicate, so
        // the planner sees a bound range and walks `idx_arm_room_seq`
        // backwards from the cursor (no temp B-tree over the whole room).
        let upto = match before {
            // An unknown cursor reads nothing (it is not in this room).
            Some(b) => match self.cursor_seq(room_id, b).await? {
                Some(seq) => seq,
                None => return Ok(Vec::new()),
            },
            None => i64::MAX,
        };
        let mut rows = sqlx::query(
            "SELECT * FROM agent_room_messages \
              WHERE room_id = ? AND rowid < ? \
              ORDER BY rowid DESC LIMIT ?",
        )
        .bind(room_id)
        .bind(upto)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list agent room messages before"))?;
        rows.reverse();
        rows.iter().map(row_to_message).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn pool() -> DbPool {
        crate::db::test_pool().await
    }

    async fn seed_ws(pool: &DbPool, id: &str) {
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
    async fn schedule_create_retry_after_lost_response_is_idempotent() {
        let p = pool().await;
        seed_ws(&p, "retry-ws").await;
        let repo = PersonalAgentsRepo::new(p);
        let agent = repo.create(new_agent("retry-ws", "Recap")).await.unwrap();
        let input = || NewAgentSchedule {
            agent_id: agent.id.clone(),
            schedule: json!({"cadence":"interval","every_min":60}),
            timezone: "UTC".into(),
            directive: "recap".into(),
            enabled: true,
        };
        let (a, b) = tokio::join!(
            repo.create_schedule_once(input(), Some("template"), "read_only"),
            repo.create_schedule_once(input(), Some("template"), "read_only")
        );
        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.permission, "read_only");
        assert_eq!(repo.list_schedules(&agent.id).await.unwrap().len(), 1);
        let mut changed = input();
        changed.directive = "different".into();
        assert!(repo
            .create_schedule_once(changed, Some("template"), "read_only")
            .await
            .is_err());
        assert_eq!(repo.list_schedules(&agent.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn user_context_compare_and_swap_preserves_concurrent_edits() {
        let pool = pool().await;
        seed_ws(&pool, "w").await;
        let repo = PersonalAgentsRepo::new(pool);
        let agent = repo
            .create(NewPersonalAgent::defaults("w".into(), "Context".into()))
            .await
            .unwrap();
        assert_eq!(
            repo.context(&agent.id).await.unwrap(),
            (String::new(), "missing".into())
        );
        let version = repo
            .save_context(&agent.id, "missing", "user context")
            .await
            .unwrap();
        assert!(repo
            .save_context(&agent.id, "missing", "stale")
            .await
            .is_err());
        assert_eq!(
            repo.context(&agent.id).await.unwrap(),
            ("user context".into(), version)
        );
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
        assert_eq!(
            repo.get(&a.id).await.unwrap().chat_session_id.as_deref(),
            Some("sess-9")
        );
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
        repo.set_schedule_runtime(
            &fast.id,
            Some("2026-09-01T10:00:00+00:00"),
            Some("2026-09-01T10:15:00+00:00"),
        )
        .await
        .unwrap();
        let daily2 = repo.get_schedule(&daily.id).await.unwrap();
        let fast2 = repo.get_schedule(&fast.id).await.unwrap();
        assert!(daily2.last_run_at.is_none(), "sibling cursor untouched");
        assert!(fast2.last_run_at.is_some());
        assert_eq!(repo.list_schedules(&a.id).await.unwrap().len(), 2);
        // Enabled tick sees both; disabling the agent hides both.
        assert_eq!(repo.list_enabled_schedules().await.unwrap().len(), 2);
        repo.update(
            &a.id,
            PersonalAgentPatch {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(repo.list_enabled_schedules().await.unwrap().len(), 0);
    }

    /// Perf W3/W12 budget: the scheduler's per-minute scan is a constant
    /// TWO statements whatever the number of schedules (it was 1 + 2N), and
    /// returns each schedule paired with its own agent.
    #[tokio::test]
    async fn schedule_scan_is_two_queries_for_any_n() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        for i in 0..6 {
            let a = repo
                .create(new_agent("ws1", &format!("Agent {i}")))
                .await
                .unwrap();
            for j in 0..2 {
                repo.create_schedule(NewAgentSchedule {
                    agent_id: a.id.clone(),
                    schedule: json!({"cadence":"interval","every_min": 15 + j}),
                    timezone: "UTC".into(),
                    directive: format!("d{i}-{j}"),
                    enabled: true,
                })
                .await
                .unwrap();
            }
        }
        let probe = p.statement_probe();
        probe.reset();
        let pairs = repo.list_enabled_schedules().await.unwrap();
        assert_eq!(pairs.len(), 12);
        assert!(pairs.iter().all(|(s, a)| s.agent_id == a.id));
        let stmts = probe.take();
        assert_eq!(stmts.len(), 2, "tick scan budget: {stmts:?}");
        // Nothing enabled → a single statement.
        sqlx::query("UPDATE personal_agents SET enabled = 0")
            .execute(&p)
            .await
            .unwrap();
        probe.reset();
        assert!(repo.list_enabled_schedules().await.unwrap().is_empty());
        assert_eq!(probe.take().len(), 1);
    }

    /// Perf N4: the governed pipeline's per-call autonomy read is served from
    /// the cache, and a save / delete is visible at once.
    #[tokio::test]
    async fn autonomy_is_cached_and_invalidated_on_save_and_delete() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        let a = repo.create(new_agent("ws1", "Cached")).await.unwrap();
        let probe = p.statement_probe();
        assert!(repo.autonomy(&a.id).await.unwrap().rules.is_empty());
        probe.reset();
        for _ in 0..10 {
            repo.autonomy(&a.id).await.unwrap();
        }
        assert!(probe.take().is_empty(), "served from the cache");
        let cfg = AgentAutonomy {
            rules: vec![AgentRule {
                id: "r1".into(),
                text: "Ask before prod".into(),
                enforce: None,
            }],
            ..Default::default()
        };
        // A fresh repo handle on the same database sees the save.
        repo.save_autonomy(&a.id, &cfg).await.unwrap();
        let other = PersonalAgentsRepo::new(p.clone());
        assert_eq!(other.autonomy(&a.id).await.unwrap().rules.len(), 1);
        repo.delete(&a.id).await.unwrap();
        assert!(repo.autonomy(&a.id).await.unwrap().rules.is_empty());
        // Another database never sees this one's entry.
        let p2 = pool().await;
        assert!(PersonalAgentsRepo::new(p2)
            .autonomy(&a.id)
            .await
            .unwrap()
            .rules
            .is_empty());
    }

    /// Perf N6: the proactive tick's budget counts are ONE statement for any
    /// number of agents, counting only proactive runs inside the window.
    #[tokio::test]
    async fn proactive_counts_are_one_query_for_any_n() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        let mut ids = Vec::new();
        for i in 0..5 {
            let a = repo
                .create(new_agent("ws1", &format!("P{i}")))
                .await
                .unwrap();
            for j in 0..i {
                let r = repo
                    .create_run(NewAgentRun {
                        agent_id: a.id.clone(),
                        schedule_id: None,
                        workspace_id: "ws1".into(),
                        trigger: "proactive".into(),
                    })
                    .await
                    .unwrap();
                let mode = if j == 0 { "directed" } else { "proactive" };
                repo.set_run_mode(&r.id, mode, true, None).await.unwrap();
            }
            ids.push(a.id);
        }
        let since = (Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let probe = p.statement_probe();
        probe.reset();
        let counts = repo
            .count_proactive_runs_since(&refs, &since)
            .await
            .unwrap();
        assert_eq!(probe.take().len(), 1, "one statement for 5 agents");
        for (i, id) in ids.iter().enumerate() {
            let want = i.saturating_sub(1) as i64; // the first run is directed
            assert_eq!(counts.get(id).copied().unwrap_or(0), want, "agent {i}");
            assert_eq!(
                repo.count_runs_since(id, "proactive", &since)
                    .await
                    .unwrap(),
                want,
                "matches the per-agent count"
            );
        }
        let future = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        assert!(repo
            .count_proactive_runs_since(&refs, &future)
            .await
            .unwrap()
            .is_empty());
        assert!(repo
            .count_proactive_runs_since(&[], &since)
            .await
            .unwrap()
            .is_empty());
    }

    /// Perf N5: the feed projection reads every column `row_to_run` needs and
    /// cuts the summary in SQL, one character past the feed's clip.
    #[tokio::test]
    async fn feed_projection_cuts_the_summary_in_sql() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        let a = repo.create(new_agent("ws1", "Feed")).await.unwrap();
        let r = repo
            .create_run(NewAgentRun {
                agent_id: a.id.clone(),
                schedule_id: None,
                workspace_id: "ws1".into(),
                trigger: "manual".into(),
            })
            .await
            .unwrap();
        sqlx::query("UPDATE personal_agent_runs SET summary = ? WHERE id = ?")
            .bind("é".repeat(5_000))
            .bind(&r.id)
            .execute(&p)
            .await
            .unwrap();
        assert!(FEED_RUN_COLS.contains(&format!("substr(summary, 1, {})", FEED_SUMMARY_CHARS + 1)));
        let cur = repo.running_run(&a.id).await.unwrap().unwrap();
        assert_eq!(cur.summary.chars().count(), FEED_SUMMARY_CHARS + 1);
        let feed = repo.list_runs_for_feed(&a.id, 20).await.unwrap();
        assert_eq!(feed[0].summary.chars().count(), FEED_SUMMARY_CHARS + 1);
        let full = repo.list_runs(&a.id, 20).await.unwrap();
        assert_eq!(full[0].summary.chars().count(), 5_000);
        let (mut f, mut g) = (feed[0].clone(), full[0].clone());
        f.summary.clear();
        g.summary.clear();
        assert_eq!(
            serde_json::to_value(f).unwrap(),
            serde_json::to_value(g).unwrap(),
            "every other field identical"
        );
    }

    #[tokio::test]
    async fn schedules_are_armed_at_creation_and_rearmable() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        let a = repo.create(new_agent("ws1", "Recap")).await.unwrap();
        let s = repo
            .create_schedule(NewAgentSchedule {
                agent_id: a.id.clone(),
                schedule: json!({"cadence":"once","run_at":"2026-09-01T10:00:00Z"}),
                timezone: "UTC".into(),
                directive: "once".into(),
                enabled: true,
            })
            .await
            .unwrap();
        assert_eq!(s.armed_at.as_deref(), Some(s.created_at.as_str()));
        repo.set_schedule_runtime(&s.id, Some("2026-09-01T10:00:05+00:00"), None)
            .await
            .unwrap();
        repo.rearm_agent_schedules(&a.id, "2026-09-02T00:00:00+00:00")
            .await
            .unwrap();
        let got = repo.get_schedule(&s.id).await.unwrap();
        assert_eq!(got.armed_at.as_deref(), Some("2026-09-02T00:00:00+00:00"));
        assert!(got.last_run_at.is_some(), "an agent resume keeps cursors");
        repo.rearm_schedule(&s.id, "2026-09-03T00:00:00+00:00", true)
            .await
            .unwrap();
        let got = repo.get_schedule(&s.id).await.unwrap();
        assert!(
            got.last_run_at.is_none(),
            "a re-timed once forgets it fired"
        );
    }

    #[tokio::test]
    async fn failed_run_keeps_its_live_session() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        let a = repo.create(new_agent("ws1", "Recap")).await.unwrap();
        let r = repo
            .create_run(NewAgentRun {
                agent_id: a.id.clone(),
                schedule_id: None,
                workspace_id: "ws1".into(),
                trigger: "manual".into(),
            })
            .await
            .unwrap();
        repo.set_run_session(&r.id, "sess-1").await.unwrap();
        repo.finish_run(
            &r.id,
            FinishAgentRun {
                status: "error".into(),
                error: Some("agent run failed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let got = repo.list_runs(&a.id, 1).await.unwrap().remove(0);
        assert_eq!(got.status, "error");
        assert_eq!(got.session_id.as_deref(), Some("sess-1"));
    }

    #[tokio::test]
    async fn notification_baseline_requires_successful_delivery() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let repo = PersonalAgentsRepo::new(p.clone());
        let a = repo.create(new_agent("ws1", "Recap")).await.unwrap();
        for (i, delivered, error, skipped, mode) in [
            (0, false, Some("offline"), false, "directed"),
            (1, false, None, true, "proactive"),
            (2, false, None, false, "directed"),
            (3, true, None, false, "directed"),
            (4, false, Some("offline"), false, "directed"),
            (5, false, None, true, "proactive"),
            (6, true, None, false, "directed"),
            (7, false, None, true, "directed"),
        ] {
            let r = repo
                .create_run(NewAgentRun {
                    agent_id: a.id.clone(),
                    schedule_id: None,
                    workspace_id: "ws1".into(),
                    trigger: "manual".into(),
                })
                .await
                .unwrap();
            repo.finish_run(
                &r.id,
                FinishAgentRun {
                    status: "ok".into(),
                    delivered,
                    delivery_error: error.map(str::to_string),
                    skipped_delivery: skipped,
                    report_hash: Some(format!("h{i}")),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            sqlx::query("UPDATE personal_agent_runs SET started_at = ?, mode = ? WHERE id = ?")
                .bind(format!("2026-10-08T00:00:0{i}Z"))
                .bind(mode)
                .bind(&r.id)
                .execute(&p)
                .await
                .unwrap();
            let hash = repo.last_ok_report_hash(&a.id, "next").await.unwrap();
            let expected = match i {
                3 => Some("h3"),
                6 => Some("h6"),
                7 => Some("h7"),
                _ => None,
            };
            assert_eq!(
                hash.as_deref(),
                expected,
                "run {i} cannot stand in for delivered content"
            );
        }
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
                    delivered: true,
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
        assert_eq!(
            rooms.list_members(&room.id).await.unwrap(),
            vec![a.id.clone()]
        );

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
        let page = rooms
            .list_messages(&room.id, Some(&m1.id), 50)
            .await
            .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, m2.id);
        // Backwards paging: the tail (newest `limit`, oldest first), then the
        // page before a cursor; an unknown cursor reads nothing.
        let tail = rooms.list_messages_before(&room.id, None, 1).await.unwrap();
        assert_eq!(tail.iter().map(|m| &m.id).collect::<Vec<_>>(), vec![&m2.id]);
        let both = rooms
            .list_messages_before(&room.id, None, 50)
            .await
            .unwrap();
        assert_eq!(
            both.iter().map(|m| &m.id).collect::<Vec<_>>(),
            vec![&m1.id, &m2.id]
        );
        let older = rooms
            .list_messages_before(&room.id, Some(&m2.id), 50)
            .await
            .unwrap();
        assert_eq!(
            older.iter().map(|m| &m.id).collect::<Vec<_>>(),
            vec![&m1.id]
        );
        assert!(rooms
            .list_messages_before(&room.id, Some(&m1.id), 50)
            .await
            .unwrap()
            .is_empty());

        // Cursors are scoped to their room: another room's message id is an
        // unknown cursor here (forward → from the start, backward → nothing).
        let other = rooms.create("ws1", "other", None).await.unwrap();
        let foreign = rooms
            .add_message(NewRoomMessage {
                room_id: other.id.clone(),
                author_kind: "user".into(),
                author_id: "u1".into(),
                text: "elsewhere".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            rooms
                .list_messages(&room.id, Some(&foreign.id), 50)
                .await
                .unwrap()
                .len(),
            2,
            "a foreign cursor must not skip this room's messages"
        );
        assert!(rooms
            .list_messages_before(&room.id, Some(&foreign.id), 50)
            .await
            .unwrap()
            .is_empty());

        // The rooms-list aggregates: members and activity per room in one
        // query each; an empty room has no activity row.
        rooms.add_member(&other.id, &b.id).await.unwrap();
        let members = rooms.members_by_workspace("ws1").await.unwrap();
        assert_eq!(members[&room.id], vec![a.id.clone()]);
        assert_eq!(members[&other.id], vec![b.id.clone()]);
        let empty = rooms.create("ws1", "quiet", None).await.unwrap();
        let activity = rooms.activity_by_workspace("ws1").await.unwrap();
        assert_eq!(activity[&room.id].message_count, 2);
        assert_eq!(
            activity[&room.id].last_message_at.as_deref(),
            Some(m2.created_at.as_str())
        );
        assert_eq!(activity[&other.id].message_count, 1);
        assert!(!activity.contains_key(&empty.id));
        // An agent's own rooms, oldest first.
        let a_rooms = rooms.list_for_agent(&a.id).await.unwrap();
        assert_eq!(
            a_rooms.iter().map(|r| &r.id).collect::<Vec<_>>(),
            vec![&room.id]
        );
        rooms.add_member(&empty.id, &a.id).await.unwrap();
        assert_eq!(rooms.list_for_agent(&a.id).await.unwrap().len(), 2);
        assert!(rooms.list_for_agent("nobody").await.unwrap().is_empty());

        // Deleting an agent cascades its membership but keeps its messages
        // (the transcript stays user-visible).
        agents.delete(&a.id).await.unwrap();
        assert!(!rooms.is_member(&room.id, &a.id).await.unwrap());
        assert_eq!(
            rooms.list_messages(&room.id, None, 50).await.unwrap().len(),
            2
        );
        // Deleting the room cascades its messages.
        rooms.delete(&room.id).await.unwrap();
        assert!(rooms.get(&room.id).await.is_err());
    }

    #[tokio::test]
    async fn autonomy_permissions_modes_and_reset_round_trip() {
        let pool = pool().await;
        seed_ws(&pool, "w1").await;
        let repo = PersonalAgentsRepo::new(pool.clone());
        let a = repo.create(new_agent("w1", "Scout")).await.unwrap();

        // No row ⇒ every default (proactive off, budget 4/day, 15 min).
        let cfg = repo.autonomy(&a.id).await.unwrap();
        assert_eq!(cfg, AgentAutonomy::default());
        assert!(!cfg.proactive.enabled);
        assert_eq!(cfg.proactive.runs_per_day, 4);

        let mut next = cfg.clone();
        next.proactive.enabled = true;
        next.goals.push(StandingGoal {
            id: "g1".into(),
            text: "Watch CI".into(),
            ..Default::default()
        });
        next.rules.push(AgentRule {
            id: "r1".into(),
            text: "Ask before touching prod".into(),
            enforce: Some(RuleEnforcement {
                kind: "approval".into(),
                terms: vec!["prod".into()],
            }),
        });
        repo.save_autonomy(&a.id, &next).await.unwrap();
        assert_eq!(repo.autonomy(&a.id).await.unwrap(), next);
        let pro = repo.list_proactive().await.unwrap();
        assert_eq!(pro.len(), 1);
        assert_eq!(pro[0].0.id, a.id);
        assert_eq!(pro[0].1.goals.len(), 1);

        let s = repo
            .create_schedule(NewAgentSchedule {
                agent_id: a.id.clone(),
                schedule: json!({"cadence":"interval","every_min":60}),
                timezone: "UTC".into(),
                directive: "d".into(),
                enabled: true,
            })
            .await
            .unwrap();
        assert_eq!(s.permission, "directed");
        repo.set_schedule_permission(&s.id, "read_only")
            .await
            .unwrap();
        assert_eq!(
            repo.get_schedule(&s.id).await.unwrap().permission,
            "read_only"
        );
        assert!(repo.set_schedule_permission(&s.id, "root").await.is_err());

        let run = repo
            .create_run(NewAgentRun {
                agent_id: a.id.clone(),
                schedule_id: None,
                workspace_id: "w1".into(),
                trigger: "proactive".into(),
            })
            .await
            .unwrap();
        assert_eq!(run.mode, "directed");
        repo.set_run_mode(&run.id, "proactive", true, Some("g1"))
            .await
            .unwrap();
        let got = repo.get_run(&run.id).await.unwrap();
        assert_eq!(got.mode, "proactive");
        assert_eq!(got.goal_id.as_deref(), Some("g1"));
        assert!(got.read_only);
        let since = "2000-01-01T00:00:00Z";
        assert_eq!(
            repo.count_runs_since(&a.id, "proactive", since)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            repo.count_runs_since(&a.id, "directed", since)
                .await
                .unwrap(),
            0
        );

        repo.set_chat_session(&a.id, Some("s1")).await.unwrap();
        repo.reset_agent(&a.id).await.unwrap();
        assert!(repo.list_runs(&a.id, 10).await.unwrap().is_empty());
        assert!(repo.list_schedules(&a.id).await.unwrap().is_empty());
        assert!(repo.get(&a.id).await.unwrap().chat_session_id.is_none());
    }

    /// R5: the denormalized room activity tracks `add_message`, and
    /// `recount_activity` heals drift after an out-of-band delete (retention).
    #[tokio::test]
    async fn room_activity_is_denormalized_and_recountable() {
        let p = pool().await;
        seed_ws(&p, "ws1").await;
        let rooms = AgentRoomsRepo::new(p.clone());
        let room = rooms.create("ws1", "r", None).await.unwrap();
        let empty = rooms.create("ws1", "empty", None).await.unwrap();
        let mut last = None;
        for i in 0..3 {
            last = Some(
                rooms
                    .add_message(NewRoomMessage {
                        room_id: room.id.clone(),
                        author_kind: "user".into(),
                        author_id: "u1".into(),
                        text: format!("m{i}"),
                    })
                    .await
                    .unwrap(),
            );
        }
        let last = last.unwrap();
        // The returned message is the stored row.
        let stored = rooms.list_messages_before(&room.id, None, 1).await.unwrap();
        assert_eq!(stored[0].id, last.id);
        assert_eq!(stored[0].created_at, last.created_at);
        let act = rooms.activity_by_workspace("ws1").await.unwrap();
        assert_eq!(act[&room.id].message_count, 3);
        assert_eq!(
            act[&room.id].last_message_at.as_deref(),
            Some(&*last.created_at)
        );
        assert!(!act.contains_key(&empty.id), "empty rooms are omitted");
        assert_eq!(rooms.recount_activity().await.unwrap(), 0, "no drift");

        sqlx::query("DELETE FROM agent_room_messages WHERE text = 'm0'")
            .execute(p.writer())
            .await
            .unwrap();
        assert_eq!(rooms.recount_activity().await.unwrap(), 1);
        let act = rooms.activity_by_workspace("ws1").await.unwrap();
        assert_eq!(act[&room.id].message_count, 2);
        assert_eq!(
            act[&room.id].last_message_at.as_deref(),
            Some(&*last.created_at)
        );
    }

    async fn plan(p: &DbPool, sql: &str) -> String {
        let rows: Vec<(i64, i64, i64, String)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
                .fetch_all(p.writer())
                .await
                .unwrap();
        rows.into_iter()
            .map(|r| r.3)
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// R1 / M1: the room tail, the before-cursor page and the after-cursor
    /// page are range scans on `idx_arm_room_seq` — no temp B-tree sort over
    /// the room — and the rooms-list activity never touches the messages.
    #[tokio::test]
    async fn room_reads_use_the_rowid_index() {
        let p = pool().await;
        for sql in [
            "SELECT * FROM agent_room_messages WHERE room_id = 'r' AND rowid > 5 \
             ORDER BY rowid ASC LIMIT 100",
            "SELECT * FROM agent_room_messages WHERE room_id = 'r' AND rowid < 9223372036854775807 \
             ORDER BY rowid DESC LIMIT 100",
        ] {
            let got = plan(&p, sql).await;
            assert!(got.contains("idx_arm_room_seq"), "{sql}: {got}");
            assert!(!got.contains("TEMP B-TREE"), "{sql}: {got}");
        }
        let cursor = plan(
            &p,
            "SELECT rowid AS seq FROM agent_room_messages WHERE id = 'x' AND room_id = 'r'",
        )
        .await;
        assert!(
            cursor.contains("INDEX") || cursor.contains("PRIMARY KEY"),
            "{cursor}"
        );
        let act = plan(
            &p,
            "SELECT id, message_count, last_message_at FROM agent_rooms \
              WHERE workspace_id = 'w' AND message_count > 0",
        )
        .await;
        assert!(act.contains("idx_agent_rooms_ws"), "{act}");
        assert!(!act.contains("agent_room_messages"), "{act}");
    }
}
