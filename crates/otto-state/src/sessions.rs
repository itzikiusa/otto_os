//! Sessions repository.

use chrono::Utc;
use otto_core::domain::{Session, SessionKind, SessionStatus};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

#[derive(Clone)]
pub struct SessionsRepo {
    pool: SqlitePool,
}

/// Minimal read-only projection used by the usage tailer to attribute on-disk
/// transcript turns back to Otto sessions. Deliberately *not* filtered by
/// status or `archived`: analysis/agent sessions finish quickly (status
/// `exited`) yet their transcripts keep growing as the user resumes them, and
/// usage from those turns still belongs to the original workspace/session.
#[derive(Debug, Clone)]
pub struct UsageAttrRow {
    pub id: String,
    pub workspace_id: String,
    pub provider: String,
    pub cwd: String,
    /// The CLI's own session uuid (= Claude transcript filename stem). `None`
    /// for sessions that never got a provider id.
    pub provider_session_id: Option<String>,
}

/// Insert payload for a new session row.
pub struct NewSession {
    pub workspace_id: Id,
    pub kind: SessionKind,
    pub provider: String,
    pub title: String,
    pub cwd: String,
    pub provider_session_id: Option<String>,
    pub connection_id: Option<Id>,
    pub created_by: Id,
    pub meta: serde_json::Value,
}

fn row_to_session(r: &sqlx::sqlite::SqliteRow) -> Result<Session> {
    Ok(Session {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        kind: SessionKind::parse(&r.get::<String, _>("kind"))
            .ok_or_else(|| Error::Internal("bad session kind".into()))?,
        provider: r.get("provider"),
        title: r.get("title"),
        status: SessionStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad session status".into()))?,
        cwd: r.get("cwd"),
        provider_session_id: r.get("provider_session_id"),
        connection_id: r.get("connection_id"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        last_active_at: ts(&r.get::<String, _>("last_active_at"))?,
        archived: r.get::<i64, _>("archived") != 0,
        meta: json(&r.get::<String, _>("meta_json"))?,
    })
}

impl SessionsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, s: NewSession) -> Result<Session> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd,
                                   provider_session_id, connection_id, created_by,
                                   created_at, last_active_at, meta_json)
             VALUES (?, ?, ?, ?, ?, 'running', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&s.workspace_id)
        .bind(s.kind.as_str())
        .bind(&s.provider)
        .bind(&s.title)
        .bind(&s.cwd)
        .bind(&s.provider_session_id)
        .bind(&s.connection_id)
        .bind(&s.created_by)
        .bind(&now)
        .bind(&now)
        .bind(s.meta.to_string())
        .execute(&self.pool)
        .await
        .map_err(dberr("create session"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<Session> {
        let r = sqlx::query("SELECT * FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("session"))?;
        row_to_session(&r)
    }

    pub async fn list_by_workspace(&self, ws: &Id) -> Result<Vec<Session>> {
        let rows = sqlx::query("SELECT * FROM sessions WHERE workspace_id = ? ORDER BY created_at")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Sessions of a workspace **owned by** `user_id` (the `created_by` creator).
    ///
    /// Used to owner-scope the session list for non-admin callers so user A's
    /// sessions never appear in user B's list (leak #L1). Admins/root keep the
    /// unfiltered [`list_by_workspace`].
    pub async fn list_by_workspace_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions WHERE workspace_id = ? AND created_by = ? ORDER BY created_at",
        )
        .bind(ws)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Sessions that should be revived or marked reconnectable on daemon boot.
    ///
    /// Includes exited agent sessions that have a `provider_session_id` — those
    /// can be resumed with `--resume` even after the daemon restarts.
    pub async fn list_all_restorable(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE archived = 0 \
               AND (status != 'exited' \
                    OR (kind = 'agent' AND provider_session_id IS NOT NULL))",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    pub async fn update_status(&self, id: &Id, status: SessionStatus) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = ?, last_active_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session status"))?;
        Ok(())
    }

    pub async fn set_provider_session(&self, id: &Id, provider_session_id: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET provider_session_id = ? WHERE id = ?")
            .bind(provider_session_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session"))?;
        Ok(())
    }

    pub async fn set_title(&self, id: &Id, title: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET title = ? WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session"))?;
        Ok(())
    }

    pub async fn set_meta(&self, id: &Id, meta: &serde_json::Value) -> Result<()> {
        sqlx::query("UPDATE sessions SET meta_json = ? WHERE id = ?")
            .bind(meta.to_string())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session meta"))?;
        Ok(())
    }

    pub async fn set_archived(&self, id: &Id, archived: bool) -> Result<()> {
        sqlx::query("UPDATE sessions SET archived = ? WHERE id = ?")
            .bind(archived as i64)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("archive session"))?;
        Ok(())
    }

    pub async fn touch(&self, id: &Id) -> Result<()> {
        sqlx::query("UPDATE sessions SET last_active_at = ? WHERE id = ?")
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("touch session"))?;
        Ok(())
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete session"))?;
        Ok(())
    }

    /// Unarchived sessions active at or after `since` (RFC3339), newest first.
    pub async fn list_active_since(&self, ws: &Id, since: &str) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE workspace_id = ? AND archived = 0 AND last_active_at >= ? \
             ORDER BY last_active_at DESC",
        )
        .bind(ws)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Non-archived, channel-spawned agent sessions idle longer than `max_idle`
    /// — used to auto-archive stale ticket/chat sessions so they don't pile up
    /// in the sidebar. Oldest first.
    pub async fn list_idle_channel_sessions(
        &self,
        max_idle: std::time::Duration,
    ) -> Result<Vec<Session>> {
        let cutoff = fmt(
            Utc::now() - chrono::Duration::from_std(max_idle).unwrap_or_else(|_| chrono::Duration::zero()),
        );
        let before = cutoff.as_str();
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE archived = 0 AND kind = 'agent' AND last_active_at < ? \
               AND json_extract(meta_json, '$.source') = 'channel' \
             ORDER BY last_active_at",
        )
        .bind(before)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("idle channel sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Archived, channel-spawned agent sessions whose last activity is older
    /// than `max_age` — used to permanently delete closed ticket/chat sessions
    /// so the DB doesn't grow without bound at ticketing volume. Oldest first.
    pub async fn list_archived_channel_sessions_older_than(
        &self,
        max_age: std::time::Duration,
    ) -> Result<Vec<Session>> {
        let cutoff = fmt(
            Utc::now()
                - chrono::Duration::from_std(max_age).unwrap_or_else(|_| chrono::Duration::zero()),
        );
        let before = cutoff.as_str();
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE archived = 1 AND kind = 'agent' AND last_active_at < ? \
               AND json_extract(meta_json, '$.source') = 'channel' \
             ORDER BY last_active_at",
        )
        .bind(before)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("old archived channel sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Agent sessions that are candidates for the existence-check pruner:
    /// non-running (status `exited` or `reconnectable`) agent sessions that
    /// carry a `provider_session_id` (so they could in principle be resumed).
    ///
    /// Includes archived rows — an archived session whose provider transcript
    /// is gone is also un-resumable and should be cleaned up. The pruner then
    /// verifies each against the provider's on-disk transcript before deleting;
    /// rows it cannot verify are kept.
    pub async fn list_prunable_agent_sessions(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE kind = 'agent' \
               AND provider_session_id IS NOT NULL \
               AND status IN ('exited', 'reconnectable')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("prunable agent sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// All sessions projected to the fields the usage tailer needs to attribute
    /// on-disk transcript turns. Read-only and unfiltered (see [`UsageAttrRow`]).
    pub async fn list_usage_attribution(&self) -> sqlx::Result<Vec<UsageAttrRow>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, provider, cwd, provider_session_id FROM sessions",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| UsageAttrRow {
                id: r.get("id"),
                workspace_id: r.get("workspace_id"),
                provider: r.get("provider"),
                cwd: r.get("cwd"),
                provider_session_id: r.get("provider_session_id"),
            })
            .collect())
    }

    /// Count of sessions in a workspace for a provider (for "claude #N" titles).
    pub async fn count_by_provider(&self, ws: &Id, provider: &str) -> Result<i64> {
        let r = sqlx::query(
            "SELECT COUNT(*) AS n FROM sessions WHERE workspace_id = ? AND provider = ?",
        )
        .bind(ws)
        .bind(provider)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("count sessions"))?;
        Ok(r.get("n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

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

    async fn seed_user_ws(pool: &SqlitePool) -> (String, String) {
        let user = new_id();
        let ws = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
            .bind(&user).bind("u").bind("x").bind("U").bind(&now)
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
            .bind(&ws).bind("w").bind("/tmp").bind(&now)
            .execute(pool).await.unwrap();
        (user, ws)
    }

    async fn insert_session(
        pool: &SqlitePool,
        ws: &str,
        user: &str,
        last_active: &str,
        meta: &str,
        archived: i64,
    ) -> String {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd,
                                   created_by, created_at, last_active_at, archived, meta_json)
             VALUES (?, ?, 'agent', 'claude', 't', 'idle', '/tmp', ?, ?, ?, ?, ?)",
        )
        .bind(&id).bind(ws).bind(user).bind(&now).bind(last_active).bind(archived).bind(meta)
        .execute(pool).await.unwrap();
        id
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_session_full(
        pool: &SqlitePool,
        ws: &str,
        user: &str,
        provider: &str,
        status: &str,
        provider_session_id: Option<&str>,
        archived: i64,
    ) -> String {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd,
                                   provider_session_id, created_by, created_at, last_active_at,
                                   archived, meta_json)
             VALUES (?, ?, 'agent', ?, 't', ?, '/tmp', ?, ?, ?, ?, ?, '{}')",
        )
        .bind(&id)
        .bind(ws)
        .bind(provider)
        .bind(status)
        .bind(provider_session_id)
        .bind(user)
        .bind(&now)
        .bind(&now)
        .bind(archived)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn prunable_agent_sessions_query_filters_correctly() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());

        // Matches: exited + has provider_session_id.
        let exited = insert_session_full(&pool, &ws, &user, "claude", "exited", Some("sid-1"), 0).await;
        // Matches: reconnectable + has provider_session_id (archived still counts).
        let recon = insert_session_full(&pool, &ws, &user, "claude", "reconnectable", Some("sid-2"), 1).await;
        // Excluded: still running.
        insert_session_full(&pool, &ws, &user, "claude", "running", Some("sid-3"), 0).await;
        // Excluded: working.
        insert_session_full(&pool, &ws, &user, "claude", "working", Some("sid-4"), 0).await;
        // Excluded: exited but no provider_session_id (can't be resumed/verified).
        insert_session_full(&pool, &ws, &user, "shell", "exited", None, 0).await;

        let got = repo.list_prunable_agent_sessions().await.unwrap();
        let mut ids: Vec<&str> = got.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        let mut want = vec![exited.as_str(), recon.as_str()];
        want.sort();
        assert_eq!(ids, want);
    }

    #[tokio::test]
    async fn list_usage_attribution_returns_all_sessions_unfiltered() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());

        // A live claude session with a provider id.
        let live = insert_session_full(&pool, &ws, &user, "claude", "running", Some("psid-1"), 0).await;
        // An exited+archived codex session with NO provider id — must still be
        // returned (analysis sessions finish fast but transcripts keep growing).
        let exited = insert_session_full(&pool, &ws, &user, "codex", "exited", None, 1).await;

        let got = repo.list_usage_attribution().await.unwrap();
        let mut ids: Vec<&str> = got.iter().map(|r| r.id.as_str()).collect();
        ids.sort();
        let mut want = vec![live.as_str(), exited.as_str()];
        want.sort();
        assert_eq!(ids, want);

        let live_row = got.iter().find(|r| r.id == live).unwrap();
        assert_eq!(live_row.provider, "claude");
        assert_eq!(live_row.workspace_id, ws);
        assert_eq!(live_row.provider_session_id.as_deref(), Some("psid-1"));

        let exited_row = got.iter().find(|r| r.id == exited).unwrap();
        assert_eq!(exited_row.provider, "codex");
        assert_eq!(exited_row.provider_session_id, None);
    }

    /// Seed an extra user into an existing pool and return its id.
    async fn seed_extra_user(pool: &SqlitePool, username: &str) -> String {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
            .bind(&id).bind(username).bind("x").bind(username).bind(&now)
            .execute(pool).await.unwrap();
        id
    }

    #[tokio::test]
    async fn list_by_workspace_for_user_filters_by_owner() {
        let pool = mem_pool().await;
        let (alice, ws) = seed_user_ws(&pool).await;
        let bob = seed_extra_user(&pool, "bob").await;
        let repo = SessionsRepo::new(pool.clone());

        // Two sessions for alice, one for bob — same workspace.
        let a1 = insert_session_full(&pool, &ws, &alice, "claude", "running", None, 0).await;
        let a2 = insert_session_full(&pool, &ws, &alice, "shell", "running", None, 0).await;
        let b1 = insert_session_full(&pool, &ws, &bob, "claude", "running", None, 0).await;

        // alice sees only her two; bob sees only his one.
        let alice_ids: std::collections::HashSet<String> = repo
            .list_by_workspace_for_user(&ws, &alice)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(
            alice_ids,
            [a1.clone(), a2.clone()].into_iter().collect(),
            "alice must see only her own sessions"
        );
        assert!(!alice_ids.contains(&b1), "alice must not see bob's session");

        let bob_ids: Vec<String> = repo
            .list_by_workspace_for_user(&ws, &bob)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(bob_ids, vec![b1.clone()], "bob sees only his own");

        // The unscoped list still returns all three (admin/root path).
        let all = repo.list_by_workspace(&ws).await.unwrap();
        assert_eq!(all.len(), 3, "unscoped list returns every session");
    }

    #[tokio::test]
    async fn idle_channel_sessions_query_filters_correctly() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());

        let old = fmt(Utc::now() - ChronoDuration::hours(20));
        let recent = fmt(Utc::now());
        // The only row that should match: old + channel + not archived.
        let idle = insert_session(&pool, &ws, &user, &old, r#"{"source":"channel","channel":"telegram"}"#, 0).await;
        // Excluded: recent channel session.
        insert_session(&pool, &ws, &user, &recent, r#"{"source":"channel"}"#, 0).await;
        // Excluded: old but not a channel session.
        insert_session(&pool, &ws, &user, &old, "{}", 0).await;
        // Excluded: old channel session but already archived.
        insert_session(&pool, &ws, &user, &old, r#"{"source":"channel"}"#, 1).await;

        let got = repo
            .list_idle_channel_sessions(std::time::Duration::from_secs(12 * 60 * 60))
            .await
            .unwrap();
        let ids: Vec<&str> = got.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec![idle.as_str()]);
    }
}
