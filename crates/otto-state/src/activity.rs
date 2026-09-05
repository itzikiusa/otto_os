//! Agent activity repository: per-session live trail + normalized task tracker.
//!
//! Mirrors [`otto_core::domain::{TrailEvent, AgentTask}`]. ULID string PKs, UTC
//! RFC3339 timestamps, JSON in `detail_json`. Rows cascade away with their
//! session (FK `ON DELETE CASCADE`).

use std::collections::BTreeMap;

use chrono::Utc;
use otto_core::domain::{
    AgentTask, SessionActivitySummary, TaskStatus, TrailEvent, TrailKind, TrailLevel, TrailSource,
};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

/// Input for [`ActivityRepo::append_trail`]. `id`/`ts` are owned by the repo.
pub struct NewTrail {
    pub session_id: Id,
    pub workspace_id: Id,
    pub source: TrailSource,
    pub kind: TrailKind,
    pub level: TrailLevel,
    pub summary: String,
    pub detail: Option<serde_json::Value>,
}

/// Input for the task-tracker sync. `ext_id` is the provider-native id when one
/// exists (else `None`).
pub struct NewTask {
    pub ext_id: Option<String>,
    pub title: String,
    pub status: TaskStatus,
}

#[derive(Clone)]
pub struct ActivityRepo {
    pool: SqlitePool,
}

fn row_to_trail(r: &sqlx::sqlite::SqliteRow) -> Result<TrailEvent> {
    let detail = match r.get::<Option<String>, _>("detail_json") {
        Some(s) => Some(
            serde_json::from_str::<serde_json::Value>(&s)
                .map_err(|e| Error::Internal(format!("bad trail detail: {e}")))?,
        ),
        None => None,
    };
    Ok(TrailEvent {
        id: r.get("id"),
        session_id: r.get("session_id"),
        workspace_id: r.get("workspace_id"),
        ts: ts(&r.get::<String, _>("ts"))?,
        source: TrailSource::parse(&r.get::<String, _>("source"))
            .ok_or_else(|| Error::Internal("bad trail source".into()))?,
        kind: TrailKind::parse(&r.get::<String, _>("kind"))
            .ok_or_else(|| Error::Internal("bad trail kind".into()))?,
        level: TrailLevel::parse(&r.get::<String, _>("level")).unwrap_or(TrailLevel::Info),
        summary: r.get("summary"),
        detail,
    })
}

fn row_to_task(r: &sqlx::sqlite::SqliteRow) -> Result<AgentTask> {
    Ok(AgentTask {
        id: r.get("id"),
        session_id: r.get("session_id"),
        workspace_id: r.get("workspace_id"),
        ext_id: r.get("ext_id"),
        title: r.get("title"),
        status: TaskStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad task status".into()))?,
        position: r.get::<i64, _>("position"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        source: r.get("source"),
        description: r.get("description"),
        nudge_pending: r.get::<i64, _>("nudge_pending") != 0,
        nudged_at: match r.get::<Option<String>, _>("nudged_at") {
            Some(s) => Some(ts(&s)?),
            None => None,
        },
    })
}

/// Title match key for the task merge: case-folded, whitespace-collapsed.
/// A user-added "Fix the  Login bug" and the agent's later "fix the login bug"
/// are the same task and must keep one row.
pub fn normalize_title(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A task row still waiting for its nudge prompt (design §4.5).
#[derive(Debug, Clone)]
pub struct PendingNudge {
    pub task_id: Id,
    pub session_id: Id,
    pub workspace_id: Id,
    pub title: String,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
}

impl ActivityRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // --- trail --------------------------------------------------------------

    /// Append one trail entry and return it.
    pub async fn append_trail(&self, n: NewTrail) -> Result<TrailEvent> {
        let id = new_id();
        let now = fmt(Utc::now());
        let detail_json = match &n.detail {
            Some(v) => Some(
                serde_json::to_string(v)
                    .map_err(|e| Error::Internal(format!("encode trail detail: {e}")))?,
            ),
            None => None,
        };
        sqlx::query(
            "INSERT INTO agent_trail
                (id, session_id, workspace_id, ts, source, kind, level, summary, detail_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&n.session_id)
        .bind(&n.workspace_id)
        .bind(&now)
        .bind(n.source.as_str())
        .bind(n.kind.as_str())
        .bind(n.level.as_str())
        .bind(&n.summary)
        .bind(&detail_json)
        .execute(&self.pool)
        .await
        .map_err(dberr("append trail"))?;
        self.get_trail(&id).await
    }

    pub async fn get_trail(&self, id: &Id) -> Result<TrailEvent> {
        let r = sqlx::query("SELECT * FROM agent_trail WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("trail entry"))?;
        row_to_trail(&r)
    }

    /// Keep only the newest `keep_per_session` trail rows per session; delete
    /// the rest. Returns the number of rows pruned. Run periodically so
    /// long-lived sessions don't grow the trail unbounded.
    ///
    /// Deletes in small chunks: SQLite has ONE writer at a time, and a bulk
    /// DELETE over a large backlog held the write lock for seconds — every
    /// interactive write (create-session, trail ingest) queued behind it
    /// (observed as 3-6s "slow statement" warnings). Chunking yields the
    /// writer between batches so interactive statements interleave.
    pub async fn prune_trail(&self, keep_per_session: i64) -> Result<u64> {
        const CHUNK: i64 = 500;
        let mut total: u64 = 0;
        loop {
            let res = sqlx::query(
                "DELETE FROM agent_trail WHERE id IN (
                     SELECT id FROM (
                         SELECT id, ROW_NUMBER() OVER (
                             PARTITION BY session_id ORDER BY ts DESC, id DESC
                         ) AS rn
                         FROM agent_trail
                     ) WHERE rn > ? LIMIT ?
                 )",
            )
            .bind(keep_per_session)
            .bind(CHUNK)
            .execute(&self.pool)
            .await
            .map_err(dberr("prune trail"))?;
            let n = res.rows_affected();
            total += n;
            if n < CHUNK as u64 {
                return Ok(total);
            }
            // Brief yield so queued interactive writers acquire the lock
            // before the next batch.
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    /// Per-session roll-up for every session in `workspace_id` that has any
    /// tasks or trail — the multi-agent overview (sidebar chips).
    pub async fn workspace_summary(
        &self,
        workspace_id: &Id,
    ) -> Result<Vec<SessionActivitySummary>> {
        self.workspace_summary_inner(workspace_id, None).await
    }

    /// Per-session roll-up restricted to sessions owned by `user_id`. Used for
    /// non-admin callers so each user sees only their own sessions' activity in
    /// the multi-agent overview.
    pub async fn workspace_summary_for_user(
        &self,
        workspace_id: &Id,
        user_id: &Id,
    ) -> Result<Vec<SessionActivitySummary>> {
        self.workspace_summary_inner(workspace_id, Some(user_id)).await
    }

    /// Inner implementation: when `user_id` is `Some`, restricts the aggregate
    /// to sessions whose `session_id` is in the caller's own session set
    /// (`sessions.created_by = user_id`). When `None` the full workspace view
    /// is returned (admin / root path).
    async fn workspace_summary_inner(
        &self,
        workspace_id: &Id,
        user_id: Option<&Id>,
    ) -> Result<Vec<SessionActivitySummary>> {
        // Accumulate by session id (BTreeMap keeps a stable order).
        let mut map: BTreeMap<String, SessionActivitySummary> = BTreeMap::new();

        // Build the task query — join to sessions when a user filter is needed
        // so we only aggregate tasks for sessions the caller owns.
        let (task_rows, trail_rows) = if let Some(uid) = user_id {
            let task_rows = sqlx::query(
                "SELECT t.session_id, t.status, t.title FROM agent_tasks t
                 JOIN sessions s ON s.id = t.session_id
                 WHERE t.workspace_id = ? AND s.created_by = ?
                 ORDER BY t.session_id, t.position",
            )
            .bind(workspace_id)
            .bind(uid)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("summary tasks (user)"))?;

            let trail_rows = sqlx::query(
                "SELECT tr.session_id, MAX(tr.ts) AS last_ts FROM agent_trail tr
                 JOIN sessions s ON s.id = tr.session_id
                 WHERE tr.workspace_id = ? AND s.created_by = ?
                 GROUP BY tr.session_id",
            )
            .bind(workspace_id)
            .bind(uid)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("summary trail (user)"))?;

            (task_rows, trail_rows)
        } else {
            let task_rows = sqlx::query(
                "SELECT session_id, status, title FROM agent_tasks
                 WHERE workspace_id = ? ORDER BY session_id, position",
            )
            .bind(workspace_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("summary tasks"))?;

            let trail_rows = sqlx::query(
                "SELECT session_id, MAX(ts) AS last_ts FROM agent_trail
                 WHERE workspace_id = ? GROUP BY session_id",
            )
            .bind(workspace_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("summary trail"))?;

            (task_rows, trail_rows)
        };

        for r in &task_rows {
            let sid: String = r.get("session_id");
            let status: String = r.get("status");
            let title: String = r.get("title");
            let e = map.entry(sid.clone()).or_insert_with(|| SessionActivitySummary {
                session_id: sid,
                total: 0,
                done: 0,
                in_progress: None,
                last_ts: None,
            });
            e.total += 1;
            if status == "completed" {
                e.done += 1;
            }
            if status == "in_progress" && e.in_progress.is_none() {
                e.in_progress = Some(title);
            }
        }

        for r in &trail_rows {
            let sid: String = r.get("session_id");
            let last: Option<String> = r.get("last_ts");
            let last_ts = match last {
                Some(s) => Some(ts(&s)?),
                None => None,
            };
            let e = map.entry(sid.clone()).or_insert_with(|| SessionActivitySummary {
                session_id: sid,
                total: 0,
                done: 0,
                in_progress: None,
                last_ts: None,
            });
            e.last_ts = last_ts;
        }

        Ok(map.into_values().collect())
    }

    /// The most recent `limit` trail entries for a session, oldest→newest so the
    /// UI can append. (Query newest-first with LIMIT, then reverse.)
    pub async fn list_trail(&self, session_id: &Id, limit: i64) -> Result<Vec<TrailEvent>> {
        let rows = sqlx::query(
            "SELECT * FROM agent_trail WHERE session_id = ?
             ORDER BY ts DESC, id DESC LIMIT ?",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("trail"))?;
        let mut out: Vec<TrailEvent> = rows.iter().map(row_to_trail).collect::<Result<_>>()?;
        out.reverse();
        Ok(out)
    }

    // --- tasks --------------------------------------------------------------

    /// Tasks for a session in display order (position ASC).
    pub async fn list_tasks(&self, session_id: &Id) -> Result<Vec<AgentTask>> {
        let rows = sqlx::query(
            "SELECT * FROM agent_tasks WHERE session_id = ? ORDER BY position ASC, id ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("tasks"))?;
        rows.iter().map(row_to_task).collect()
    }

    /// Sync the provider's FULL plan (Claude's TodoWrite sends the complete list
    /// each call) into the session's task rows — design §4.5 merge semantics:
    ///
    /// * every existing row is matched to an incoming task by `ext_id` (when
    ///   both have one) else by normalized title, and UPDATED in place — ids
    ///   stay stable, no regenerate-on-sync;
    /// * unmatched incoming tasks are INSERTED as `source = 'agent'`;
    /// * unmatched `'agent'` rows are DELETED (the plan dropped them);
    /// * unmatched `'user'` rows (added from the board) are KEPT, positioned
    ///   after the plan, so a board task survives every plan update; a user
    ///   row the agent adopted takes the agent's status + `ext_id`.
    ///
    /// Returns the resulting list in order.
    pub async fn replace_tasks(
        &self,
        session_id: &Id,
        workspace_id: &Id,
        tasks: &[NewTask],
    ) -> Result<Vec<AgentTask>> {
        let now = fmt(Utc::now());
        let mut tx = self.pool.begin().await.map_err(dberr("tasks tx"))?;

        let existing = sqlx::query(
            "SELECT id, ext_id, title, source FROM agent_tasks WHERE session_id = ? ORDER BY position, id",
        )
        .bind(session_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(dberr("tasks prior"))?;
        struct Row {
            id: String,
            ext_id: Option<String>,
            norm: String,
            source: String,
        }
        let mut rows: Vec<Row> = existing
            .iter()
            .map(|r| Row {
                id: r.get("id"),
                ext_id: r.get("ext_id"),
                norm: normalize_title(&r.get::<String, _>("title")),
                source: r.get("source"),
            })
            .collect();
        let mut matched: Vec<bool> = vec![false; rows.len()];

        for (i, t) in tasks.iter().enumerate() {
            let norm = normalize_title(&t.title);
            let hit = rows
                .iter()
                .enumerate()
                .position(|(j, r)| {
                    !matched[j]
                        && ((t.ext_id.is_some() && r.ext_id.is_some() && r.ext_id == t.ext_id) || r.norm == norm)
                });
            match hit {
                Some(j) => {
                    matched[j] = true;
                    // A user row the agent adopted keeps its id + source but takes
                    // the agent's status and ext_id; an agent row is simply updated.
                    sqlx::query(
                        "UPDATE agent_tasks
                            SET title = ?, status = ?, position = ?, updated_at = ?,
                                ext_id = COALESCE(?, ext_id)
                          WHERE id = ?",
                    )
                    .bind(&t.title)
                    .bind(t.status.as_str())
                    .bind(i as i64)
                    .bind(&now)
                    .bind(&t.ext_id)
                    .bind(&rows[j].id)
                    .execute(&mut *tx)
                    .await
                    .map_err(dberr("update task"))?;
                }
                None => {
                    let id = new_id();
                    sqlx::query(
                        "INSERT INTO agent_tasks
                            (id, session_id, workspace_id, ext_id, title, status, position, created_at, updated_at, source)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'agent')",
                    )
                    .bind(&id)
                    .bind(session_id)
                    .bind(workspace_id)
                    .bind(&t.ext_id)
                    .bind(&t.title)
                    .bind(t.status.as_str())
                    .bind(i as i64)
                    .bind(&now)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await
                    .map_err(dberr("insert task"))?;
                }
            }
        }
        // Drop agent rows the plan no longer has; push surviving user rows after it.
        let mut tail = tasks.len() as i64;
        for (j, r) in rows.iter_mut().enumerate() {
            if matched[j] {
                continue;
            }
            if r.source == "agent" {
                sqlx::query("DELETE FROM agent_tasks WHERE id = ?")
                    .bind(&r.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(dberr("delete task"))?;
            } else {
                sqlx::query("UPDATE agent_tasks SET position = ? WHERE id = ?")
                    .bind(tail)
                    .bind(&r.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(dberr("reposition task"))?;
                tail += 1;
            }
        }

        tx.commit().await.map_err(dberr("tasks commit"))?;
        self.list_tasks(session_id).await
    }

    /// Incremental provider update (Claude `TaskCreate` / `TaskUpdate`, which
    /// carry a provider task id instead of the whole list): update the row with
    /// that `ext_id` — or, for a create, a same-titled row (a user task the agent
    /// picked up) — else insert a new `'agent'` row at the end. `status`
    /// `None` keeps the current one. Returns the resulting list.
    pub async fn upsert_task(
        &self,
        session_id: &Id,
        workspace_id: &Id,
        ext_id: &str,
        title: Option<&str>,
        status: Option<TaskStatus>,
    ) -> Result<Vec<AgentTask>> {
        let now = fmt(Utc::now());
        let existing = self.list_tasks(session_id).await?;
        let by_ext = existing.iter().find(|t| t.ext_id.as_deref() == Some(ext_id));
        let by_title = title.and_then(|t| {
            let n = normalize_title(t);
            existing.iter().find(|x| x.ext_id.is_none() && normalize_title(&x.title) == n)
        });
        match by_ext.or(by_title) {
            Some(row) => {
                sqlx::query(
                    "UPDATE agent_tasks
                        SET ext_id = ?, title = COALESCE(?, title), status = COALESCE(?, status), updated_at = ?
                      WHERE id = ?",
                )
                .bind(ext_id)
                .bind(title)
                .bind(status.map(|s| s.as_str()))
                .bind(&now)
                .bind(&row.id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update task"))?;
            }
            None => {
                let position = existing.iter().map(|t| t.position + 1).max().unwrap_or(0);
                sqlx::query(
                    "INSERT INTO agent_tasks
                        (id, session_id, workspace_id, ext_id, title, status, position, created_at, updated_at, source)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'agent')",
                )
                .bind(new_id())
                .bind(session_id)
                .bind(workspace_id)
                .bind(ext_id)
                .bind(title.unwrap_or("(task)"))
                .bind(status.unwrap_or(TaskStatus::Pending).as_str())
                .bind(position)
                .bind(&now)
                .bind(&now)
                .execute(&self.pool)
                .await
                .map_err(dberr("insert task"))?;
            }
        }
        self.list_tasks(session_id).await
    }

    /// Add a board task (`source = 'user'`, `nudge_pending = 1`) at the end of
    /// the list. Returns the new row.
    pub async fn insert_user_task(
        &self,
        session_id: &Id,
        workspace_id: &Id,
        title: &str,
        description: Option<&str>,
    ) -> Result<AgentTask> {
        let now = fmt(Utc::now());
        let id = new_id();
        let position: i64 = sqlx::query("SELECT COALESCE(MAX(position) + 1, 0) AS p FROM agent_tasks WHERE session_id = ?")
            .bind(session_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("task position"))?
            .get("p");
        sqlx::query(
            "INSERT INTO agent_tasks
                (id, session_id, workspace_id, ext_id, title, status, position, created_at, updated_at,
                 source, description, nudge_pending)
             VALUES (?, ?, ?, NULL, ?, 'pending', ?, ?, ?, 'user', ?, 1)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(workspace_id)
        .bind(title)
        .bind(position)
        .bind(&now)
        .bind(&now)
        .bind(description)
        .execute(&self.pool)
        .await
        .map_err(dberr("insert user task"))?;
        let r = sqlx::query("SELECT * FROM agent_tasks WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("task"))?;
        row_to_task(&r)
    }

    /// Every task still waiting for its nudge, oldest first (optionally for one
    /// session).
    pub async fn pending_nudges(&self, session_id: Option<&Id>) -> Result<Vec<PendingNudge>> {
        let rows = match session_id {
            Some(sid) => sqlx::query(
                "SELECT id, session_id, workspace_id, title, description, created_at FROM agent_tasks
                 WHERE nudge_pending = 1 AND session_id = ? ORDER BY created_at, id",
            )
            .bind(sid)
            .fetch_all(&self.pool)
            .await,
            None => sqlx::query(
                "SELECT id, session_id, workspace_id, title, description, created_at FROM agent_tasks
                 WHERE nudge_pending = 1 ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await,
        }
        .map_err(dberr("pending nudges"))?;
        rows.iter()
            .map(|r| {
                Ok(PendingNudge {
                    task_id: r.get("id"),
                    session_id: r.get("session_id"),
                    workspace_id: r.get("workspace_id"),
                    title: r.get("title"),
                    description: r.get("description"),
                    created_at: ts(&r.get::<String, _>("created_at"))?,
                })
            })
            .collect()
    }

    /// Atomically claim a pending nudge: flips `nudge_pending` 1 → 0 and stamps
    /// `nudged_at`; `Ok(true)` only for the one caller that won (two sweeps
    /// racing on the same row deliver it once). Undo with [`Self::unclaim_nudge`]
    /// when the submit fails.
    pub async fn claim_nudge(&self, task_id: &Id) -> Result<bool> {
        let now = fmt(Utc::now());
        let res = sqlx::query(
            "UPDATE agent_tasks SET nudge_pending = 0, nudged_at = ?, updated_at = ?
              WHERE id = ? AND nudge_pending = 1",
        )
        .bind(&now)
        .bind(&now)
        .bind(task_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("claim nudge"))?;
        Ok(res.rows_affected() == 1)
    }

    /// Put a claimed nudge back in the queue (the PTY write failed).
    pub async fn unclaim_nudge(&self, task_id: &Id) -> Result<()> {
        sqlx::query("UPDATE agent_tasks SET nudge_pending = 1, nudged_at = NULL WHERE id = ?")
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("unclaim nudge"))?;
        Ok(())
    }

    /// The nudge for `task_id` was submitted to the session.
    pub async fn mark_nudged(&self, task_id: &Id) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE agent_tasks SET nudge_pending = 0, nudged_at = ?, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(&now)
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("mark nudged"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn mk_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("sqlite");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    /// Seed a minimal user row.
    async fn seed_user(pool: &SqlitePool, user_id: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, 'x', ?, 0, ?)",
        )
        .bind(user_id)
        .bind(user_id)
        .bind(user_id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed user");
    }

    /// Seed a workspace row.
    async fn seed_workspace(pool: &SqlitePool, ws_id: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
             VALUES (?, 'ws', '/tmp', '{}', 0, ?)",
        )
        .bind(ws_id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed workspace");
    }

    /// Seed a session row owned by `created_by` and return its id.
    async fn seed_session(pool: &SqlitePool, ws_id: &str, created_by: &str) -> Id {
        let id = otto_core::new_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO sessions
                (id, workspace_id, kind, provider, title, status, cwd, created_by,
                 created_at, last_active_at, archived, meta_json)
             VALUES (?, ?, 'agent', 'shell', 't', 'running', '/tmp', ?, ?, ?, 0, '{}')
             ",
        )
        .bind(&id)
        .bind(ws_id)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed session");
        id
    }

    /// Seed a single task for the given session.
    async fn seed_task(pool: &SqlitePool, ws_id: &str, session_id: &str) {
        let task_id = otto_core::new_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO agent_tasks
                (id, session_id, workspace_id, ext_id, title, status, position, created_at, updated_at)
             VALUES (?, ?, ?, NULL, 'task', 'pending', 0, ?, ?)",
        )
        .bind(&task_id)
        .bind(session_id)
        .bind(ws_id)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed task");
    }

    /// Seed a trail entry for the given session.
    async fn seed_trail(pool: &SqlitePool, ws_id: &str, session_id: &str) {
        let trail_id = otto_core::new_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO agent_trail
                (id, session_id, workspace_id, ts, source, kind, level, summary)
             VALUES (?, ?, ?, ?, 'agent', 'session', 'info', 'test')",
        )
        .bind(&trail_id)
        .bind(session_id)
        .bind(ws_id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed trail");
    }

    /// #L18 — workspace_summary_for_user returns only the caller's sessions.
    ///
    /// Alice and Bob each own one session in the same workspace. When Bob
    /// queries with his own user id, only his session appears; Alice's is
    /// invisible.
    #[tokio::test]
    async fn summary_for_user_excludes_other_users_sessions() {
        let pool = mk_pool().await;
        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        seed_workspace(&pool, "ws1").await;

        let alice_sid = seed_session(&pool, "ws1", "alice").await;
        let bob_sid = seed_session(&pool, "ws1", "bob").await;

        seed_task(&pool, "ws1", &alice_sid).await;
        seed_task(&pool, "ws1", &bob_sid).await;
        seed_trail(&pool, "ws1", &alice_sid).await;
        seed_trail(&pool, "ws1", &bob_sid).await;

        let repo = ActivityRepo::new(pool.clone());

        // Bob's user-scoped view contains only his session.
        let bob_summary = repo
            .workspace_summary_for_user(&"ws1".into(), &"bob".into())
            .await
            .expect("bob summary");
        let bob_ids: Vec<&str> = bob_summary.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(bob_ids, vec![bob_sid.as_str()], "bob must only see his own session");
        assert!(
            !bob_ids.contains(&alice_sid.as_str()),
            "alice's session must not appear in bob's summary"
        );
    }

    /// The full workspace_summary (admin path) returns all sessions.
    #[tokio::test]
    async fn workspace_summary_returns_all_sessions() {
        let pool = mk_pool().await;
        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        seed_workspace(&pool, "ws1").await;

        let alice_sid = seed_session(&pool, "ws1", "alice").await;
        let bob_sid = seed_session(&pool, "ws1", "bob").await;

        seed_task(&pool, "ws1", &alice_sid).await;
        seed_task(&pool, "ws1", &bob_sid).await;

        let repo = ActivityRepo::new(pool.clone());
        let all = repo
            .workspace_summary(&"ws1".into())
            .await
            .expect("all summary");
        assert_eq!(all.len(), 2, "admin path must return both sessions");
    }

    /// workspace_summary_for_user with no sessions for the caller returns empty.
    #[tokio::test]
    async fn summary_for_user_empty_when_no_own_sessions() {
        let pool = mk_pool().await;
        seed_user(&pool, "alice").await;
        seed_user(&pool, "carol").await;
        seed_workspace(&pool, "ws1").await;

        let alice_sid = seed_session(&pool, "ws1", "alice").await;
        seed_task(&pool, "ws1", &alice_sid).await;

        let repo = ActivityRepo::new(pool.clone());
        // Carol has no sessions; her scoped summary should be empty.
        let carol = repo
            .workspace_summary_for_user(&"ws1".into(), &"carol".into())
            .await
            .expect("carol summary");
        assert!(carol.is_empty(), "carol with no sessions must get an empty summary");
    }

    fn nt(title: &str, status: TaskStatus, ext_id: Option<&str>) -> NewTask {
        NewTask {
            ext_id: ext_id.map(str::to_string),
            title: title.into(),
            status,
        }
    }

    /// Design §4.5: a plan sync replaces only `agent` rows, merges into `user`
    /// rows by normalized title / ext_id, and never regenerates ids.
    #[tokio::test]
    async fn replace_tasks_merges_user_rows_and_keeps_ids_stable() {
        let pool = mk_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let sid = seed_session(&pool, "ws1", "alice").await;
        let repo = ActivityRepo::new(pool.clone());
        let ws: Id = "ws1".into();

        let first = repo
            .replace_tasks(&sid, &ws, &[nt("design", TaskStatus::Completed, None), nt("build", TaskStatus::InProgress, None)])
            .await
            .unwrap();
        let build_id = first.iter().find(|t| t.title == "build").unwrap().id.clone();
        let user = repo.insert_user_task(&sid, &ws, "Fix the  Login bug", Some("see ticket")).await.unwrap();
        assert_eq!(user.source, "user");
        assert!(user.nudge_pending);
        assert_eq!(user.description.as_deref(), Some("see ticket"));

        // Plan update: design dropped, build completed, the agent adopted the
        // user's task (different casing) with an ext_id, plus a new one.
        let second = repo
            .replace_tasks(
                &sid,
                &ws,
                &[
                    nt("build", TaskStatus::Completed, None),
                    nt("fix the login bug", TaskStatus::InProgress, Some("7")),
                    nt("test", TaskStatus::Pending, None),
                ],
            )
            .await
            .unwrap();
        let titles: Vec<&str> = second.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["build", "fix the login bug", "test"], "design deleted, order = plan order");
        let build = second.iter().find(|t| t.title == "build").unwrap();
        assert_eq!(build.id, build_id, "agent row updated in place");
        assert_eq!(build.status, TaskStatus::Completed);
        let fix = second.iter().find(|t| t.ext_id.as_deref() == Some("7")).unwrap();
        assert_eq!(fix.id, user.id, "user row adopted, id stable");
        assert_eq!(fix.source, "user");
        assert_eq!(fix.status, TaskStatus::InProgress);
        assert_eq!(fix.description.as_deref(), Some("see ticket"));

        // A plan that drops the adopted task keeps the user row (after the plan).
        let third = repo.replace_tasks(&sid, &ws, &[nt("test", TaskStatus::Completed, None)]).await.unwrap();
        assert_eq!(third.len(), 2);
        assert_eq!(third[0].title, "test");
        assert_eq!(third[1].id, user.id);

        // Summary counts ALL rows (user-added tasks included).
        let sum = repo.workspace_summary(&ws).await.unwrap();
        assert_eq!(sum[0].total, 2);
        assert_eq!(sum[0].done, 1);

        // Nudge queue.
        let pending = repo.pending_nudges(Some(&sid)).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].task_id, user.id);
        // Atomic claim: exactly one winner; unclaim re-queues.
        assert!(repo.claim_nudge(&user.id).await.unwrap());
        assert!(!repo.claim_nudge(&user.id).await.unwrap());
        repo.unclaim_nudge(&user.id).await.unwrap();
        assert_eq!(repo.pending_nudges(Some(&sid)).await.unwrap().len(), 1);
        repo.mark_nudged(&user.id).await.unwrap();
        assert!(repo.pending_nudges(None).await.unwrap().is_empty());
        let after = repo.list_tasks(&sid).await.unwrap();
        assert!(after.iter().all(|t| !t.nudge_pending));
        assert!(after.iter().any(|t| t.nudged_at.is_some()));
    }

    /// `TaskCreate` / `TaskUpdate` update by ext_id (or adopt a same-titled row).
    #[tokio::test]
    async fn upsert_task_by_ext_id() {
        let pool = mk_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let sid = seed_session(&pool, "ws1", "alice").await;
        let repo = ActivityRepo::new(pool.clone());
        let ws: Id = "ws1".into();
        let user = repo.insert_user_task(&sid, &ws, "Write docs", None).await.unwrap();
        let l = repo.upsert_task(&sid, &ws, "1", Some("write docs"), None).await.unwrap();
        assert_eq!(l.len(), 1, "same-titled user row adopted");
        assert_eq!(l[0].id, user.id);
        assert_eq!(l[0].ext_id.as_deref(), Some("1"));
        let l = repo.upsert_task(&sid, &ws, "1", None, Some(TaskStatus::Completed)).await.unwrap();
        assert_eq!(l[0].status, TaskStatus::Completed);
        let l = repo.upsert_task(&sid, &ws, "2", Some("Ship it"), None).await.unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].source, "agent");
        assert_eq!(l[1].position, 1);
    }
}
