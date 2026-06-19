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
    })
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
    pub async fn prune_trail(&self, keep_per_session: i64) -> Result<u64> {
        let res = sqlx::query(
            "DELETE FROM agent_trail WHERE id IN (
                 SELECT id FROM (
                     SELECT id, ROW_NUMBER() OVER (
                         PARTITION BY session_id ORDER BY ts DESC, id DESC
                     ) AS rn
                     FROM agent_trail
                 ) WHERE rn > ?
             )",
        )
        .bind(keep_per_session)
        .execute(&self.pool)
        .await
        .map_err(dberr("prune trail"))?;
        Ok(res.rows_affected())
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

    /// Replace a session's whole task list (provider full-sync semantics — e.g.
    /// Claude's TodoWrite sends the complete list each call). Preserves
    /// `created_at` for tasks whose title is unchanged so age stays meaningful.
    /// Returns the resulting list in order.
    pub async fn replace_tasks(
        &self,
        session_id: &Id,
        workspace_id: &Id,
        tasks: &[NewTask],
    ) -> Result<Vec<AgentTask>> {
        let now = fmt(Utc::now());
        let mut tx = self.pool.begin().await.map_err(dberr("tasks tx"))?;

        // Snapshot existing created_at by title for age preservation.
        let prior = sqlx::query("SELECT title, created_at FROM agent_tasks WHERE session_id = ?")
            .bind(session_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(dberr("tasks prior"))?;
        let mut created_by_title = std::collections::HashMap::<String, String>::new();
        for r in &prior {
            created_by_title.insert(r.get::<String, _>("title"), r.get::<String, _>("created_at"));
        }

        sqlx::query("DELETE FROM agent_tasks WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("clear tasks"))?;

        for (i, t) in tasks.iter().enumerate() {
            let id = new_id();
            let created = created_by_title.get(&t.title).cloned().unwrap_or_else(|| now.clone());
            sqlx::query(
                "INSERT INTO agent_tasks
                    (id, session_id, workspace_id, ext_id, title, status, position, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(session_id)
            .bind(workspace_id)
            .bind(&t.ext_id)
            .bind(&t.title)
            .bind(t.status.as_str())
            .bind(i as i64)
            .bind(&created)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(dberr("insert task"))?;
        }

        tx.commit().await.map_err(dberr("tasks commit"))?;
        self.list_tasks(session_id).await
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
}
