//! History index repository (design §4.6): one row per transcript file the
//! background scan found under `~/.claude/projects` / `~/.codex/sessions`.
//! Metadata only — the conversation itself is always re-read from disk.

use chrono::Utc;
use otto_core::{Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptIndexRow {
    pub path: String,
    /// `claude` | `codex`.
    pub provider: String,
    pub provider_session_id: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub first_prompt: Option<String>,
    pub started_at: Option<String>,
    pub last_active_at: Option<String>,
    /// File mtime, unix seconds.
    pub mtime: i64,
    pub size: i64,
    /// `None` when the peek skipped the middle of a big file.
    pub turns: Option<i64>,
    pub indexed_at: String,
}

fn row(r: &sqlx::sqlite::SqliteRow) -> TranscriptIndexRow {
    TranscriptIndexRow {
        path: r.get("path"),
        provider: r.get("provider"),
        provider_session_id: r.get("provider_session_id"),
        cwd: r.get("cwd"),
        title: r.get("title"),
        first_prompt: r.get("first_prompt"),
        started_at: r.get("started_at"),
        last_active_at: r.get("last_active_at"),
        mtime: r.get("mtime"),
        size: r.get("size"),
        turns: r.get("turns"),
        indexed_at: r.get("indexed_at"),
    }
}

#[derive(Clone)]
pub struct TranscriptIndexRepo {
    pool: SqlitePool,
}

impl TranscriptIndexRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// `(mtime, size)` of every indexed path — the rescan's skip-unchanged set.
    pub async fn stamps(&self) -> Result<std::collections::HashMap<String, (i64, i64)>> {
        let rows = sqlx::query("SELECT path, mtime, size FROM transcript_index")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("transcript index"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>("path"), (r.get::<i64, _>("mtime"), r.get::<i64, _>("size"))))
            .collect())
    }

    /// Insert or refresh one row (`indexed_at` is set here).
    pub async fn upsert(&self, r: &TranscriptIndexRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO transcript_index
                (path, provider, provider_session_id, cwd, title, first_prompt, started_at,
                 last_active_at, mtime, size, turns, indexed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(path) DO UPDATE SET
                provider = excluded.provider,
                provider_session_id = excluded.provider_session_id,
                cwd = excluded.cwd,
                title = COALESCE(excluded.title, transcript_index.title),
                first_prompt = COALESCE(excluded.first_prompt, transcript_index.first_prompt),
                started_at = COALESCE(excluded.started_at, transcript_index.started_at),
                last_active_at = COALESCE(excluded.last_active_at, transcript_index.last_active_at),
                mtime = excluded.mtime,
                size = excluded.size,
                turns = excluded.turns,
                indexed_at = excluded.indexed_at",
        )
        .bind(&r.path)
        .bind(&r.provider)
        .bind(&r.provider_session_id)
        .bind(&r.cwd)
        .bind(&r.title)
        .bind(&r.first_prompt)
        .bind(&r.started_at)
        .bind(&r.last_active_at)
        .bind(r.mtime)
        .bind(r.size)
        .bind(r.turns)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("upsert transcript index"))?;
        Ok(())
    }

    pub async fn get(&self, path: &str) -> Result<Option<TranscriptIndexRow>> {
        let r = sqlx::query("SELECT * FROM transcript_index WHERE path = ?")
            .bind(path)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("transcript index"))?;
        Ok(r.as_ref().map(row))
    }

    /// Every row, most recently active first.
    pub async fn list(&self) -> Result<Vec<TranscriptIndexRow>> {
        let rows = sqlx::query("SELECT * FROM transcript_index ORDER BY last_active_at DESC, path")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("transcript index"))?;
        Ok(rows.iter().map(row).collect())
    }

    /// Newest `limit` rows active strictly before `before` (RFC3339 text
    /// compare — the History page cursor).
    pub async fn list_page(&self, before: Option<&str>, limit: i64) -> Result<Vec<TranscriptIndexRow>> {
        let rows = sqlx::query(
            "SELECT * FROM transcript_index
              WHERE (? IS NULL OR last_active_at < ?)
              ORDER BY last_active_at DESC, path LIMIT ?",
        )
        .bind(before)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("transcript index"))?;
        Ok(rows.iter().map(row).collect())
    }

    /// Drop rows whose file is gone (`paths` = what the scan actually saw).
    pub async fn retain(&self, paths: &std::collections::HashSet<String>) -> Result<u64> {
        let rows = sqlx::query("SELECT path FROM transcript_index")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("transcript index"))?;
        let mut removed = 0;
        for r in rows {
            let p: String = r.get("path");
            if !paths.contains(&p) {
                sqlx::query("DELETE FROM transcript_index WHERE path = ?")
                    .bind(&p)
                    .execute(&self.pool)
                    .await
                    .map_err(dberr("delete transcript index"))?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Rows for the given provider session ids (the claimed-set join).
    pub async fn by_provider_session_ids(&self, ids: &[Id]) -> Result<Vec<TranscriptIndexRow>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(200) {
            let marks = vec!["?"; chunk.len()].join(",");
            let sql = format!("SELECT * FROM transcript_index WHERE provider_session_id IN ({marks})");
            let mut q = sqlx::query(&sql);
            for id in chunk {
                q = q.bind(id);
            }
            let rows = q.fetch_all(&self.pool).await.map_err(dberr("transcript index"))?;
            out.extend(rows.iter().map(row));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn mk_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().in_memory(true))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn upsert_get_list_retain() {
        let repo = TranscriptIndexRepo::new(mk_pool().await);
        let a = TranscriptIndexRow {
            path: "/h/.claude/projects/-x/a.jsonl".into(),
            provider: "claude".into(),
            provider_session_id: Some("a".into()),
            cwd: Some("/x".into()),
            title: Some("A".into()),
            first_prompt: Some("hi".into()),
            started_at: Some("2026-01-01T00:00:00Z".into()),
            last_active_at: Some("2026-01-02T00:00:00Z".into()),
            mtime: 10,
            size: 100,
            turns: Some(3),
            indexed_at: String::new(),
        };
        repo.upsert(&a).await.unwrap();
        let mut b = a.clone();
        b.path = "/h/.codex/sessions/2026/01/01/rollout-x-b.jsonl".into();
        b.provider = "codex".into();
        b.provider_session_id = Some("b".into());
        b.last_active_at = Some("2026-01-03T00:00:00Z".into());
        repo.upsert(&b).await.unwrap();
        // Refresh keeps a title the re-peek could not see.
        let mut a2 = a.clone();
        a2.title = None;
        a2.mtime = 11;
        repo.upsert(&a2).await.unwrap();
        let got = repo.get(&a.path).await.unwrap().unwrap();
        assert_eq!(got.title.as_deref(), Some("A"));
        assert_eq!(got.mtime, 11);
        assert_eq!(repo.stamps().await.unwrap()[&a.path], (11, 100));
        let list = repo.list().await.unwrap();
        assert_eq!(list[0].path, b.path, "newest first");
        let page = repo.list_page(Some("2026-01-03T00:00:00Z"), 10).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].path, a.path);
        assert_eq!(repo.list_page(None, 1).await.unwrap().len(), 1);
        assert_eq!(repo.by_provider_session_ids(&["b".to_string()]).await.unwrap().len(), 1);
        let keep: std::collections::HashSet<String> = [b.path.clone()].into_iter().collect();
        assert_eq!(repo.retain(&keep).await.unwrap(), 1);
        assert!(repo.get(&a.path).await.unwrap().is_none());
    }
}
