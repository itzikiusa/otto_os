//! Persistence for the **Otto Assistant** (migration `XXXX_assistant.sql`).
//!
//! Every row is per user (`owner_user_id`): the by-id reads take the owner and
//! answer `NotFound` for another user's row, so a route can never leak one
//! user's thread/task to another (IDOR guard at the storage edge). Pure
//! storage — routing, session driving, memory and cadence math live in
//! `otto_server::assistant`. Ids are ULIDs, so `id` order is creation order
//! (the turn index pages on it).

use chrono::Utc;
use otto_core::{new_id, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

// --- Wire rows (mirrored in ui/src/lib/api/types.ts `// ── Otto Assistant`) --

/// One assistant thread. `status` is not stored — the server derives it from
/// the backing session (`asleep | idle | working`) before a row goes out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantThread {
    pub id: String,
    #[serde(skip)]
    pub owner_user_id: String,
    pub space_slot: Option<i64>,
    pub title: String,
    pub provider: String,
    pub model: Option<String>,
    pub account_id: Option<String>,
    pub route_pinned: bool,
    pub session_id: Option<String>,
    pub incognito: bool,
    pub failover_choice: String,
    #[serde(default = "asleep")]
    pub status: String,
    pub last_turn_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn asleep() -> String {
    "asleep".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantAttachment {
    pub id: String,
    pub name: String,
    pub path: String,
    pub mime: String,
    pub size: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantTurn {
    pub id: String,
    pub thread_id: String,
    pub role: String,
    pub kind: String,
    pub text: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub route_reason: Option<String>,
    pub session_id: Option<String>,
    pub attachments: Vec<AssistantAttachment>,
    pub data: Option<Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantTask {
    pub id: String,
    #[serde(skip)]
    pub owner_user_id: String,
    pub thread_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub title: String,
    pub detail: String,
    pub origin: String,
    pub run_at: Option<String>,
    pub timezone: String,
    /// Cadence spec for reminders (`{cadence:"once", run_at}`) — internal.
    #[serde(skip)]
    pub schedule: Option<Value>,
    pub schedule_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_run_id: Option<String>,
    pub needs_you: Option<Value>,
    pub result: Option<Value>,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

/// Stored router settings (the server fills defaults for missing keys).
#[derive(Debug, Clone, Default)]
pub struct AssistantRoutingRow {
    pub rules: Value,
    pub auto_failover: bool,
    pub memory_approval: bool,
    pub limits: Value,
    pub updated_at: Option<String>,
}

// --- Inputs ------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct NewThread {
    pub owner_user_id: String,
    pub title: String,
    pub space_slot: Option<i64>,
    pub provider: String,
    pub model: Option<String>,
    pub account_id: Option<String>,
    pub route_pinned: bool,
    pub incognito: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NewTurn {
    pub thread_id: String,
    pub role: String,
    pub kind: String,
    pub text: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub route_reason: Option<String>,
    pub session_id: Option<String>,
    pub attachments: Vec<AssistantAttachment>,
    pub data: Option<Value>,
    /// Transcript turn id of an indexed reply (idempotent re-index).
    pub source_ref: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub owner_user_id: String,
    pub thread_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub title: String,
    pub detail: String,
    pub origin: String,
    pub run_at: Option<String>,
    pub timezone: String,
    pub schedule: Option<Value>,
    pub agent_id: Option<String>,
    pub needs_you: Option<Value>,
}

/// Task states that end a task (stamp `finished_at`).
pub const TERMINAL_STATES: [&str; 3] = ["done", "failed", "cancelled"];

/// Every valid task state.
pub const TASK_STATES: [&str; 6] = ["queued", "running", "needs_you", "done", "failed", "cancelled"];

// --- Row mapping ---------------------------------------------------------------

fn opt_json(r: &sqlx::sqlite::SqliteRow, col: &str) -> Option<Value> {
    r.get::<Option<String>, _>(col)
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn row_to_thread(r: &sqlx::sqlite::SqliteRow) -> AssistantThread {
    AssistantThread {
        id: r.get("id"),
        owner_user_id: r.get("owner_user_id"),
        space_slot: r.get("space_slot"),
        title: r.get("title"),
        provider: r.get("provider"),
        model: r.get("model"),
        account_id: r.get("account_id"),
        route_pinned: r.get::<i64, _>("route_pinned") != 0,
        session_id: r.get("session_id"),
        incognito: r.get::<i64, _>("incognito") != 0,
        failover_choice: r.get("failover_choice"),
        status: asleep(),
        last_turn_at: r.get("last_turn_at"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn row_to_turn(r: &sqlx::sqlite::SqliteRow) -> AssistantTurn {
    let attachments_raw: String = r.get("attachments_json");
    AssistantTurn {
        id: r.get("id"),
        thread_id: r.get("thread_id"),
        role: r.get("role"),
        kind: r.get("kind"),
        text: r.get("text"),
        provider: r.get("provider"),
        model: r.get("model"),
        route_reason: r.get("route_reason"),
        session_id: r.get("session_id"),
        attachments: serde_json::from_str(&attachments_raw).unwrap_or_default(),
        data: opt_json(r, "data_json"),
        created_at: r.get("created_at"),
    }
}

fn row_to_task(r: &sqlx::sqlite::SqliteRow) -> AssistantTask {
    AssistantTask {
        id: r.get("id"),
        owner_user_id: r.get("owner_user_id"),
        thread_id: r.get("thread_id"),
        kind: r.get("kind"),
        state: r.get("state"),
        title: r.get("title"),
        detail: r.get("detail"),
        origin: r.get("origin"),
        run_at: r.get("run_at"),
        timezone: r.get("timezone"),
        schedule: opt_json(r, "schedule_json"),
        schedule_id: r.get("schedule_id"),
        agent_id: r.get("agent_id"),
        agent_run_id: r.get("agent_run_id"),
        needs_you: opt_json(r, "needs_you_json"),
        result: opt_json(r, "result_json"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
        finished_at: r.get("finished_at"),
    }
}

fn row_to_attachment(r: &sqlx::sqlite::SqliteRow) -> AssistantAttachment {
    AssistantAttachment {
        id: r.get("id"),
        name: r.get("name"),
        path: r.get("path"),
        mime: r.get("mime"),
        size: r.get("size"),
    }
}

fn check_slot(slot: Option<i64>) -> Result<()> {
    match slot {
        Some(s) if !(1..=4).contains(&s) => Err(Error::Invalid(
            "space_slot must be 1..4 (or null)".into(),
        )),
        _ => Ok(()),
    }
}

// --- Repo ------------------------------------------------------------------------

#[derive(Clone)]
pub struct AssistantRepo {
    pool: SqlitePool,
}

impl AssistantRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- Threads ------------------------------------------------------------------

    /// Create a thread. A requested `space_slot` is taken over: whichever of
    /// the owner's threads held it is unslotted first (the slot is unique).
    pub async fn create_thread(&self, t: NewThread) -> Result<AssistantThread> {
        check_slot(t.space_slot)?;
        let id = new_id();
        let now = fmt(Utc::now());
        if let Some(slot) = t.space_slot {
            self.release_slot(&t.owner_user_id, slot).await?;
        }
        sqlx::query(
            "INSERT INTO assistant_threads (id, owner_user_id, space_slot, title, provider, model, \
             account_id, route_pinned, incognito, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&t.owner_user_id)
        .bind(t.space_slot)
        .bind(&t.title)
        .bind(&t.provider)
        .bind(&t.model)
        .bind(&t.account_id)
        .bind(t.route_pinned as i64)
        .bind(t.incognito as i64)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create assistant thread"))?;
        self.get_thread(&t.owner_user_id, &id).await
    }

    async fn release_slot(&self, owner: &str, slot: i64) -> Result<()> {
        sqlx::query(
            "UPDATE assistant_threads SET space_slot = NULL WHERE owner_user_id = ? AND space_slot = ?",
        )
        .bind(owner)
        .bind(slot)
        .execute(&self.pool)
        .await
        .map_err(dberr("release assistant space slot"))?;
        Ok(())
    }

    /// The owner's thread (another user's id answers `NotFound`).
    pub async fn get_thread(&self, owner: &str, id: &str) -> Result<AssistantThread> {
        let r = sqlx::query("SELECT * FROM assistant_threads WHERE id = ? AND owner_user_id = ?")
            .bind(id)
            .bind(owner)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("assistant thread"))?;
        Ok(row_to_thread(&r))
    }

    /// Owner-less read for daemon-internal callers (tick, reply indexer).
    pub async fn get_thread_any(&self, id: &str) -> Result<AssistantThread> {
        let r = sqlx::query("SELECT * FROM assistant_threads WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("assistant thread"))?;
        Ok(row_to_thread(&r))
    }

    /// Slotted threads first (by slot), then most recently updated.
    pub async fn list_threads(&self, owner: &str) -> Result<Vec<AssistantThread>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_threads WHERE owner_user_id = ? \
             ORDER BY (space_slot IS NULL), space_slot, updated_at DESC",
        )
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list assistant threads"))?;
        Ok(rows.iter().map(row_to_thread).collect())
    }

    /// Rename and/or (un)slot. `space_slot: Some(None)` unslots.
    pub async fn update_thread(
        &self,
        owner: &str,
        id: &str,
        title: Option<String>,
        space_slot: Option<Option<i64>>,
    ) -> Result<AssistantThread> {
        self.get_thread(owner, id).await?;
        if let Some(slot) = space_slot {
            check_slot(slot)?;
            if let Some(s) = slot {
                self.release_slot(owner, s).await?;
            }
            sqlx::query("UPDATE assistant_threads SET space_slot = ? WHERE id = ?")
                .bind(slot)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("slot assistant thread"))?;
        }
        sqlx::query(
            "UPDATE assistant_threads SET title = COALESCE(?, title), updated_at = ? WHERE id = ?",
        )
        .bind(title)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update assistant thread"))?;
        self.get_thread(owner, id).await
    }

    /// Point the thread at a (new) backing session and the route it runs.
    pub async fn set_thread_session(
        &self,
        id: &str,
        session_id: Option<&str>,
        provider: &str,
        model: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE assistant_threads SET session_id = ?, provider = ?, model = ?, account_id = ?, \
             updated_at = ? WHERE id = ?",
        )
        .bind(session_id)
        .bind(provider)
        .bind(model)
        .bind(account_id)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set assistant thread session"))?;
        Ok(())
    }

    /// Set / clear the explicit pin. A pin also records the pinned route so
    /// the next turn uses it (the backing session switches at that turn).
    pub async fn set_thread_pin(
        &self,
        id: &str,
        pinned: bool,
        provider: Option<&str>,
        model: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE assistant_threads SET route_pinned = ?, \
             provider = COALESCE(?, provider), \
             model = CASE WHEN ? THEN ? ELSE model END, \
             account_id = CASE WHEN ? THEN ? ELSE account_id END, \
             updated_at = ? WHERE id = ?",
        )
        .bind(pinned as i64)
        .bind(provider)
        .bind(pinned as i64)
        .bind(model)
        .bind(pinned as i64)
        .bind(account_id)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("pin assistant thread"))?;
        Ok(())
    }

    pub async fn set_failover_choice(&self, id: &str, choice: &str) -> Result<()> {
        if !matches!(choice, "ask" | "switch" | "stay") {
            return Err(Error::Invalid(format!("failover_choice '{choice}'")));
        }
        sqlx::query("UPDATE assistant_threads SET failover_choice = ?, updated_at = ? WHERE id = ?")
            .bind(choice)
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set assistant failover choice"))?;
        Ok(())
    }

    pub async fn delete_thread(&self, owner: &str, id: &str) -> Result<AssistantThread> {
        let t = self.get_thread(owner, id).await?;
        sqlx::query("DELETE FROM assistant_threads WHERE id = ? AND owner_user_id = ?")
            .bind(id)
            .bind(owner)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete assistant thread"))?;
        Ok(t)
    }

    /// The thread a session backs, if any (by `session_id`).
    pub async fn thread_by_session(&self, session_id: &str) -> Result<Option<AssistantThread>> {
        let r = sqlx::query("SELECT * FROM assistant_threads WHERE session_id = ? LIMIT 1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("assistant thread by session"))?;
        Ok(r.as_ref().map(row_to_thread))
    }

    /// Incognito threads idle since before `cutoff` (RFC3339) — deleted by the tick.
    pub async fn expired_incognito(&self, cutoff: &str) -> Result<Vec<AssistantThread>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_threads WHERE incognito = 1 \
             AND COALESCE(last_turn_at, created_at) < ?",
        )
        .bind(cutoff)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("expired incognito threads"))?;
        Ok(rows.iter().map(row_to_thread).collect())
    }

    // -- Turns ------------------------------------------------------------------------

    /// Append a turn and bump the thread's `last_turn_at` / `updated_at`.
    pub async fn add_turn(&self, t: NewTurn) -> Result<AssistantTurn> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO assistant_turns (id, thread_id, role, kind, text, provider, model, \
             route_reason, session_id, attachments_json, data_json, source_ref, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&t.thread_id)
        .bind(&t.role)
        .bind(if t.kind.is_empty() { "message" } else { t.kind.as_str() })
        .bind(&t.text)
        .bind(&t.provider)
        .bind(&t.model)
        .bind(&t.route_reason)
        .bind(&t.session_id)
        .bind(serde_json::to_string(&t.attachments).unwrap_or_else(|_| "[]".into()))
        .bind(t.data.as_ref().map(|v| v.to_string()))
        .bind(&t.source_ref)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add assistant turn"))?;
        sqlx::query("UPDATE assistant_threads SET last_turn_at = ?, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(&now)
            .bind(&t.thread_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("touch assistant thread"))?;
        self.get_turn(&id).await
    }

    pub async fn get_turn(&self, id: &str) -> Result<AssistantTurn> {
        let r = sqlx::query("SELECT * FROM assistant_turns WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("assistant turn"))?;
        Ok(row_to_turn(&r))
    }

    /// Index a reply turn by its transcript `source_ref`. Returns the row when
    /// it was inserted or its text changed (a growing turn), `None` when the
    /// index already held exactly this text — so callers emit only real changes.
    pub async fn upsert_indexed_turn(&self, t: NewTurn) -> Result<Option<AssistantTurn>> {
        let Some(source) = t.source_ref.clone() else {
            return self.add_turn(t).await.map(Some);
        };
        let existing = sqlx::query(
            "SELECT id, text FROM assistant_turns WHERE thread_id = ? AND source_ref = ?",
        )
        .bind(&t.thread_id)
        .bind(&source)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("find indexed assistant turn"))?;
        match existing {
            None => self.add_turn(t).await.map(Some),
            Some(row) => {
                let id: String = row.get("id");
                let text: String = row.get("text");
                if text == t.text {
                    return Ok(None);
                }
                sqlx::query("UPDATE assistant_turns SET text = ?, model = COALESCE(?, model) WHERE id = ?")
                    .bind(&t.text)
                    .bind(&t.model)
                    .bind(&id)
                    .execute(&self.pool)
                    .await
                    .map_err(dberr("update indexed assistant turn"))?;
                self.get_turn(&id).await.map(Some)
            }
        }
    }

    /// Up to `limit` turns before `before` (a turn id; `None` = the newest),
    /// returned oldest first.
    pub async fn list_turns(
        &self,
        thread_id: &str,
        before: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AssistantTurn>> {
        let limit = limit.clamp(1, 500);
        let rows = sqlx::query(
            "SELECT * FROM assistant_turns WHERE thread_id = ? AND (? IS NULL OR id < ?) \
             ORDER BY id DESC LIMIT ?",
        )
        .bind(thread_id)
        .bind(before)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list assistant turns"))?;
        let mut out: Vec<AssistantTurn> = rows.iter().map(row_to_turn).collect();
        out.reverse();
        Ok(out)
    }

    // -- Attachments ------------------------------------------------------------------

    pub async fn add_attachment(
        &self,
        owner: &str,
        thread_id: &str,
        a: &AssistantAttachment,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO assistant_attachments (id, thread_id, owner_user_id, name, path, mime, size, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&a.id)
        .bind(thread_id)
        .bind(owner)
        .bind(&a.name)
        .bind(&a.path)
        .bind(&a.mime)
        .bind(a.size)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("add assistant attachment"))?;
        Ok(())
    }

    /// The owner's attachments of `thread_id` among `ids` (unknown ids are
    /// skipped; another thread's or user's never match).
    pub async fn attachments(
        &self,
        owner: &str,
        thread_id: &str,
        ids: &[String],
    ) -> Result<Vec<AssistantAttachment>> {
        let mut out = Vec::new();
        for id in ids.iter().take(20) {
            let r = sqlx::query(
                "SELECT * FROM assistant_attachments WHERE id = ? AND owner_user_id = ? AND thread_id = ?",
            )
            .bind(id)
            .bind(owner)
            .bind(thread_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("assistant attachment"))?;
            if let Some(r) = r {
                out.push(row_to_attachment(&r));
            }
        }
        Ok(out)
    }

    // -- Tasks --------------------------------------------------------------------------

    pub async fn create_task(&self, t: NewTask) -> Result<AssistantTask> {
        if !TASK_STATES.contains(&t.state.as_str()) {
            return Err(Error::Invalid(format!("task state '{}'", t.state)));
        }
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO assistant_tasks (id, owner_user_id, thread_id, kind, state, title, detail, \
             origin, run_at, timezone, schedule_json, agent_id, needs_you_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&t.owner_user_id)
        .bind(&t.thread_id)
        .bind(&t.kind)
        .bind(&t.state)
        .bind(&t.title)
        .bind(&t.detail)
        .bind(if t.origin.is_empty() { "thread" } else { t.origin.as_str() })
        .bind(&t.run_at)
        .bind(&t.timezone)
        .bind(t.schedule.as_ref().map(|v| v.to_string()))
        .bind(&t.agent_id)
        .bind(t.needs_you.as_ref().map(|v| v.to_string()))
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create assistant task"))?;
        self.get_task_any(&id).await
    }

    pub async fn get_task(&self, owner: &str, id: &str) -> Result<AssistantTask> {
        let r = sqlx::query("SELECT * FROM assistant_tasks WHERE id = ? AND owner_user_id = ?")
            .bind(id)
            .bind(owner)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("assistant task"))?;
        Ok(row_to_task(&r))
    }

    pub async fn get_task_any(&self, id: &str) -> Result<AssistantTask> {
        let r = sqlx::query("SELECT * FROM assistant_tasks WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("assistant task"))?;
        Ok(row_to_task(&r))
    }

    /// Newest first. `state` / `thread_id` filter when given.
    pub async fn list_tasks(
        &self,
        owner: &str,
        state: Option<&str>,
        thread_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AssistantTask>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_tasks WHERE owner_user_id = ? \
             AND (? IS NULL OR state = ?) AND (? IS NULL OR thread_id = ?) \
             ORDER BY id DESC LIMIT ?",
        )
        .bind(owner)
        .bind(state)
        .bind(state)
        .bind(thread_id)
        .bind(thread_id)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list assistant tasks"))?;
        Ok(rows.iter().map(row_to_task).collect())
    }

    /// The needs-you queue, oldest first.
    pub async fn needs_you(&self, owner: &str) -> Result<Vec<AssistantTask>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_tasks WHERE owner_user_id = ? AND state = 'needs_you' ORDER BY id",
        )
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("assistant needs-you"))?;
        Ok(rows.iter().map(row_to_task).collect())
    }

    pub async fn count_needs_you(&self, owner: &str) -> Result<i64> {
        let r = sqlx::query(
            "SELECT COUNT(*) AS n FROM assistant_tasks WHERE owner_user_id = ? AND state = 'needs_you'",
        )
        .bind(owner)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("count assistant needs-you"))?;
        Ok(r.get::<i64, _>("n"))
    }

    /// Move a task to `state`. `needs_you: Some(v)` replaces the payload
    /// (`Some(Value::Null)` clears it); `result: Some(v)` replaces the result.
    /// Terminal states stamp `finished_at`.
    pub async fn set_task_state(
        &self,
        id: &str,
        state: &str,
        needs_you: Option<Value>,
        result: Option<Value>,
    ) -> Result<AssistantTask> {
        if !TASK_STATES.contains(&state) {
            return Err(Error::Invalid(format!("task state '{state}'")));
        }
        let now = fmt(Utc::now());
        let finished = TERMINAL_STATES.contains(&state).then(|| now.clone());
        let ny_set = needs_you.is_some() as i64;
        let ny = needs_you.filter(|v| !v.is_null()).map(|v| v.to_string());
        sqlx::query(
            "UPDATE assistant_tasks SET state = ?, \
             needs_you_json = CASE WHEN ? THEN ? ELSE needs_you_json END, \
             result_json = COALESCE(?, result_json), \
             finished_at = COALESCE(?, finished_at), updated_at = ? WHERE id = ?",
        )
        .bind(state)
        .bind(ny_set)
        .bind(ny)
        .bind(result.map(|v| v.to_string()))
        .bind(finished)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set assistant task state"))?;
        self.get_task_any(id).await
    }

    /// Compare-and-swap a task's state (`from` → `to`); true when this caller
    /// won. The reminder tick claims `queued → running` with it so a reminder
    /// never fires twice.
    pub async fn claim_task(&self, id: &str, from: &str, to: &str) -> Result<bool> {
        let r = sqlx::query(
            "UPDATE assistant_tasks SET state = ?, updated_at = ? WHERE id = ? AND state = ?",
        )
        .bind(to)
        .bind(fmt(Utc::now()))
        .bind(id)
        .bind(from)
        .execute(&self.pool)
        .await
        .map_err(dberr("claim assistant task"))?;
        Ok(r.rows_affected() == 1)
    }

    pub async fn set_task_agent_run(&self, id: &str, agent_id: &str, run_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE assistant_tasks SET agent_id = ?, agent_run_id = ?, updated_at = ? WHERE id = ?",
        )
        .bind(agent_id)
        .bind(run_id)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set assistant task run"))?;
        Ok(())
    }

    /// Queued reminders whose `run_at` is at or before `now` (RFC3339 UTC).
    pub async fn due_reminders(&self, now: &str) -> Result<Vec<AssistantTask>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_tasks WHERE kind = 'reminder' AND state = 'queued' \
             AND run_at IS NOT NULL AND run_at <= ? ORDER BY run_at LIMIT 100",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("due assistant reminders"))?;
        Ok(rows.iter().map(row_to_task).collect())
    }

    /// Running delegations with a Personal Agent run to watch.
    pub async fn running_delegations(&self) -> Result<Vec<AssistantTask>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_tasks WHERE kind = 'delegation' AND state = 'running' \
             AND agent_run_id IS NOT NULL LIMIT 200",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("running assistant delegations"))?;
        Ok(rows.iter().map(row_to_task).collect())
    }

    /// Open approval items (synced against the MCP approvals queue by the tick).
    pub async fn open_approvals(&self) -> Result<Vec<AssistantTask>> {
        let rows = sqlx::query(
            "SELECT * FROM assistant_tasks WHERE kind = 'approval' AND state = 'needs_you' LIMIT 200",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("open assistant approvals"))?;
        Ok(rows.iter().map(row_to_task).collect())
    }

    // -- Routing ------------------------------------------------------------------------

    pub async fn routing(&self, owner: &str) -> Result<Option<AssistantRoutingRow>> {
        let r = sqlx::query("SELECT * FROM assistant_routing WHERE owner_user_id = ?")
            .bind(owner)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("assistant routing"))?;
        Ok(r.map(|r| {
            let rules: String = r.get("rules_json");
            let limits: String = r.get("limits_json");
            AssistantRoutingRow {
                rules: serde_json::from_str(&rules).unwrap_or(Value::Null),
                auto_failover: r.get::<i64, _>("auto_failover") != 0,
                memory_approval: r.get::<i64, _>("memory_approval") != 0,
                limits: serde_json::from_str(&limits).unwrap_or(Value::Null),
                updated_at: Some(r.get("updated_at")),
            }
        }))
    }

    /// Upsert the settings half (limits are written by [`Self::set_limits`]).
    pub async fn put_routing(
        &self,
        owner: &str,
        rules: &Value,
        auto_failover: bool,
        memory_approval: bool,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO assistant_routing (owner_user_id, rules_json, auto_failover, memory_approval, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(owner_user_id) DO UPDATE SET rules_json = excluded.rules_json, \
             auto_failover = excluded.auto_failover, memory_approval = excluded.memory_approval, \
             updated_at = excluded.updated_at",
        )
        .bind(owner)
        .bind(rules.to_string())
        .bind(auto_failover as i64)
        .bind(memory_approval as i64)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("put assistant routing"))?;
        Ok(())
    }

    pub async fn set_limits(&self, owner: &str, limits: &Value) -> Result<()> {
        sqlx::query(
            "INSERT INTO assistant_routing (owner_user_id, limits_json, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT(owner_user_id) DO UPDATE SET limits_json = excluded.limits_json",
        )
        .bind(owner)
        .bind(limits.to_string())
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("set assistant limits"))?;
        Ok(())
    }

    // -- Principals + grants (minimal; the Permissions tab is a later phase) --------

    /// The owner's assistant principal id, created on first use.
    pub async fn assistant_principal(&self, owner: &str) -> Result<String> {
        sqlx::query(
            "INSERT OR IGNORE INTO agent_principals (id, kind, owner_user_id, ref_id, name, created_at) \
             VALUES (?, 'assistant', ?, '', 'Otto', ?)",
        )
        .bind(new_id())
        .bind(owner)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("ensure assistant principal"))?;
        let r = sqlx::query(
            "SELECT id FROM agent_principals WHERE kind = 'assistant' AND owner_user_id = ? AND ref_id = ''",
        )
        .bind(owner)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("assistant principal"))?;
        Ok(r.get("id"))
    }

    pub async fn set_grant(
        &self,
        principal_id: &str,
        resource_kind: &str,
        resource: &str,
        mode: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO agent_grants (principal_id, resource_kind, resource, mode, created_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(principal_id, resource_kind, resource) DO UPDATE SET mode = excluded.mode",
        )
        .bind(principal_id)
        .bind(resource_kind)
        .bind(resource)
        .bind(mode)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("set agent grant"))?;
        Ok(())
    }

    pub async fn grant_mode(
        &self,
        principal_id: &str,
        resource_kind: &str,
        resource: &str,
    ) -> Result<Option<String>> {
        let r = sqlx::query(
            "SELECT mode FROM agent_grants WHERE principal_id = ? AND resource_kind = ? AND resource = ?",
        )
        .bind(principal_id)
        .bind(resource_kind)
        .bind(resource)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("agent grant"))?;
        Ok(r.map(|r| r.get("mode")))
    }

    // -- Personal-agent lookup for delegation -----------------------------------------

    /// `(agent_id, workspace_id)` of every Personal Agent named `name`
    /// (case-insensitive). The caller role-checks each workspace.
    pub async fn personal_agents_named(&self, name: &str) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id FROM personal_agents WHERE lower(name) = lower(?) LIMIT 20",
        )
        .bind(name.trim())
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("personal agents by name"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>("id"), r.get::<String, _>("workspace_id")))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn repo() -> AssistantRepo {
        AssistantRepo::new(crate::db::test_pool().await)
    }

    fn thread(owner: &str, slot: Option<i64>) -> NewThread {
        NewThread {
            owner_user_id: owner.into(),
            title: "Personal".into(),
            space_slot: slot,
            provider: "claude".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn another_users_thread_is_not_found() {
        let r = repo().await;
        let t = r.create_thread(thread("u1", None)).await.unwrap();
        assert!(r.get_thread("u1", &t.id).await.is_ok());
        assert!(matches!(
            r.get_thread("u2", &t.id).await,
            Err(Error::NotFound(_))
        ));
        assert!(r.list_threads("u2").await.unwrap().is_empty());
        assert!(r.delete_thread("u2", &t.id).await.is_err());
    }

    #[tokio::test]
    async fn a_space_slot_moves_to_the_newest_claim() {
        let r = repo().await;
        let a = r.create_thread(thread("u1", Some(1))).await.unwrap();
        let b = r.create_thread(thread("u1", Some(1))).await.unwrap();
        assert_eq!(r.get_thread("u1", &a.id).await.unwrap().space_slot, None);
        assert_eq!(r.get_thread("u1", &b.id).await.unwrap().space_slot, Some(1));
        // Another user's slot 1 is independent.
        let c = r.create_thread(thread("u2", Some(1))).await.unwrap();
        assert_eq!(c.space_slot, Some(1));
        assert_eq!(r.get_thread("u1", &b.id).await.unwrap().space_slot, Some(1));
        // Out-of-range slots are refused; unslotting works.
        assert!(r.create_thread(thread("u1", Some(5))).await.is_err());
        let b2 = r.update_thread("u1", &b.id, None, Some(None)).await.unwrap();
        assert_eq!(b2.space_slot, None);
        // Slotted threads list first.
        let _ = r.update_thread("u1", &a.id, None, Some(Some(2))).await.unwrap();
        let list = r.list_threads("u1").await.unwrap();
        assert_eq!(list[0].id, a.id);
    }

    #[tokio::test]
    async fn indexed_turns_are_idempotent_and_page_oldest_first() {
        let r = repo().await;
        let t = r.create_thread(thread("u1", None)).await.unwrap();
        let user = r
            .add_turn(NewTurn {
                thread_id: t.id.clone(),
                role: "user".into(),
                text: "hi".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let reply = |text: &str| NewTurn {
            thread_id: t.id.clone(),
            role: "assistant".into(),
            text: text.into(),
            provider: Some("claude".into()),
            source_ref: Some("req-1".into()),
            ..Default::default()
        };
        assert!(r.upsert_indexed_turn(reply("Hel")).await.unwrap().is_some());
        // Same text again: no change, nothing to emit.
        assert!(r.upsert_indexed_turn(reply("Hel")).await.unwrap().is_none());
        // The turn grew: updated in place (same row).
        let grown = r.upsert_indexed_turn(reply("Hello!")).await.unwrap().unwrap();
        let turns = r.list_turns(&t.id, None, 100).await.unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, user.id);
        assert_eq!(turns[1].id, grown.id);
        assert_eq!(turns[1].text, "Hello!");
        // Paging before the reply returns only the user turn.
        let page = r.list_turns(&t.id, Some(&grown.id), 100).await.unwrap();
        assert_eq!(page.len(), 1);
        // The thread's last_turn_at moved.
        assert!(r.get_thread("u1", &t.id).await.unwrap().last_turn_at.is_some());
    }

    #[tokio::test]
    async fn needs_you_queue_counts_and_clears() {
        let r = repo().await;
        let t = r
            .create_task(NewTask {
                owner_user_id: "u1".into(),
                kind: "approval".into(),
                state: "needs_you".into(),
                title: "Post to #general".into(),
                needs_you: Some(json!({"kind":"approval","prompt":"ok?"})),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(r.count_needs_you("u1").await.unwrap(), 1);
        assert_eq!(r.count_needs_you("u2").await.unwrap(), 0);
        assert_eq!(r.needs_you("u1").await.unwrap()[0].id, t.id);
        let done = r
            .set_task_state(&t.id, "done", Some(Value::Null), Some(json!({"decision":"approved"})))
            .await
            .unwrap();
        assert!(done.needs_you.is_none());
        assert!(done.finished_at.is_some());
        assert_eq!(done.result.unwrap()["decision"], "approved");
        assert_eq!(r.count_needs_you("u1").await.unwrap(), 0);
        assert!(r.set_task_state(&t.id, "bogus", None, None).await.is_err());
        assert!(r.get_task("u2", &t.id).await.is_err());
    }

    #[tokio::test]
    async fn due_reminders_are_claimed_once() {
        let r = repo().await;
        let t = r
            .create_task(NewTask {
                owner_user_id: "u1".into(),
                kind: "reminder".into(),
                state: "queued".into(),
                title: "Call Dana".into(),
                run_at: Some("2026-09-24T15:00:00+00:00".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(r.due_reminders("2026-09-24T14:59:00+00:00").await.unwrap().is_empty());
        assert_eq!(r.due_reminders("2026-09-24T15:00:00+00:00").await.unwrap().len(), 1);
        assert!(r.claim_task(&t.id, "queued", "running").await.unwrap());
        assert!(!r.claim_task(&t.id, "queued", "running").await.unwrap());
        assert!(r.due_reminders("2026-09-25T00:00:00+00:00").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn routing_upserts_and_grants_are_per_principal() {
        let r = repo().await;
        assert!(r.routing("u1").await.unwrap().is_none());
        r.set_limits("u1", &json!([{"provider":"claude"}])).await.unwrap();
        r.put_routing("u1", &json!({"targets":{}}), true, false).await.unwrap();
        let row = r.routing("u1").await.unwrap().unwrap();
        assert!(row.auto_failover);
        assert_eq!(row.limits[0]["provider"], "claude");
        let p1 = r.assistant_principal("u1").await.unwrap();
        assert_eq!(p1, r.assistant_principal("u1").await.unwrap());
        let p2 = r.assistant_principal("u2").await.unwrap();
        assert_ne!(p1, p2);
        r.set_grant(&p1, "tool_destination", "telegram|me", "allow").await.unwrap();
        assert_eq!(
            r.grant_mode(&p1, "tool_destination", "telegram|me").await.unwrap().as_deref(),
            Some("allow")
        );
        assert!(r.grant_mode(&p2, "tool_destination", "telegram|me").await.unwrap().is_none());
    }
}
