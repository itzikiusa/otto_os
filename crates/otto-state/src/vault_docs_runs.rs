//! Vault docs-agent runs repository — the durable mirror of the in-memory
//! run registry in `otto-server/src/vault_docs_agent.rs`.
//!
//! One row per docs run AND per refine turn (`kind`). `payload` holds the full
//! `VaultDocsRun` JSON snapshot (the detail-rendering source of truth); the
//! flat columns exist only for listing/filtering. The orchestrator upserts a
//! fresh snapshot at every meaningful transition, so any row still
//! non-terminal at daemon startup was interrupted by a restart — that is the
//! whole recovery predicate (`list_unfinished`).

use otto_core::Result;
use sqlx::{Row, SqlitePool};

use crate::convert::dberr;

/// One persisted run/turn. Timestamps stay RFC3339 TEXT end-to-end — the
/// `VaultDocsRun` DTO itself carries string timestamps, so nothing converts.
#[derive(Debug, Clone)]
pub struct VaultDocsRunRow {
    pub id: String,
    pub vault_id: i64,
    pub ws_id: String,
    /// `docs` | `refine`.
    pub kind: String,
    /// `running | summarizing | reviewing | revising | done |
    /// done_with_findings | error | cancelled | interrupted`.
    pub state: String,
    pub prompt: String,
    pub target_dir: String,
    /// Refine turns: the note being edited (`""` for docs runs).
    pub note_path: String,
    /// Full `VaultDocsRun` JSON snapshot.
    pub payload: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub updated_at: String,
}

fn row_to_run(r: &sqlx::sqlite::SqliteRow) -> VaultDocsRunRow {
    VaultDocsRunRow {
        id: r.get("id"),
        vault_id: r.get("vault_id"),
        ws_id: r.get("ws_id"),
        kind: r.get("kind"),
        state: r.get("state"),
        prompt: r.get("prompt"),
        target_dir: r.get("target_dir"),
        note_path: r.get("note_path"),
        payload: r.get("payload"),
        started_at: r.get("started_at"),
        finished_at: r.get("finished_at"),
        updated_at: r.get("updated_at"),
    }
}

#[derive(Clone)]
pub struct VaultDocsRunsRepo {
    pool: SqlitePool,
}

impl VaultDocsRunsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Insert or refresh a run snapshot (`updated_at` stamped here).
    pub async fn upsert(&self, row: &VaultDocsRunRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO vault_docs_runs
               (id, vault_id, ws_id, kind, state, prompt, target_dir, note_path,
                payload, started_at, finished_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
               state = excluded.state,
               payload = excluded.payload,
               finished_at = excluded.finished_at,
               updated_at = excluded.updated_at",
        )
        .bind(&row.id)
        .bind(row.vault_id)
        .bind(&row.ws_id)
        .bind(&row.kind)
        .bind(&row.state)
        .bind(&row.prompt)
        .bind(&row.target_dir)
        .bind(&row.note_path)
        .bind(&row.payload)
        .bind(&row.started_at)
        .bind(&row.finished_at)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(dberr("upsert vault docs run"))?;
        Ok(())
    }

    pub async fn get(&self, id: &str) -> Result<Option<VaultDocsRunRow>> {
        let row = sqlx::query("SELECT * FROM vault_docs_runs WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get vault docs run"))?;
        Ok(row.as_ref().map(row_to_run))
    }

    /// Newest-first runs of one vault.
    pub async fn list_for_vault(&self, vault_id: i64, limit: i64) -> Result<Vec<VaultDocsRunRow>> {
        let rows = sqlx::query(
            "SELECT * FROM vault_docs_runs WHERE vault_id = ?1
             ORDER BY started_at DESC, id DESC LIMIT ?2",
        )
        .bind(vault_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list vault docs runs"))?;
        Ok(rows.iter().map(row_to_run).collect())
    }

    /// The newest refine turn recorded for one note — the refine-session
    /// registry's rehydration source after a daemon restart.
    pub async fn latest_refine_for_note(
        &self,
        vault_id: i64,
        note_path: &str,
    ) -> Result<Option<VaultDocsRunRow>> {
        let row = sqlx::query(
            "SELECT * FROM vault_docs_runs
             WHERE vault_id = ?1 AND kind = 'refine' AND note_path = ?2
             ORDER BY started_at DESC, id DESC LIMIT 1",
        )
        .bind(vault_id)
        .bind(note_path)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("latest refine for note"))?;
        Ok(row.as_ref().map(row_to_run))
    }

    /// Delete one run row (history cleanup). Returns whether a row existed.
    pub async fn delete(&self, id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM vault_docs_runs WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete vault docs run"))?;
        Ok(res.rows_affected() > 0)
    }

    /// Every non-terminal row — the startup interrupted-sweep input.
    pub async fn list_unfinished(&self) -> Result<Vec<VaultDocsRunRow>> {
        let rows = sqlx::query(
            "SELECT * FROM vault_docs_runs
             WHERE state IN ('running', 'summarizing', 'reviewing', 'revising')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list unfinished vault docs runs"))?;
        Ok(rows.iter().map(row_to_run).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    fn row(id: &str, vault_id: i64, kind: &str, state: &str, started_at: &str) -> VaultDocsRunRow {
        VaultDocsRunRow {
            id: id.into(),
            vault_id,
            ws_id: "ws1".into(),
            kind: kind.into(),
            state: state.into(),
            prompt: "p".into(),
            target_dir: String::new(),
            note_path: String::new(),
            payload: "{}".into(),
            started_at: started_at.into(),
            finished_at: None,
            updated_at: String::new(),
        }
    }

    #[tokio::test]
    async fn upsert_get_and_state_refresh() {
        let repo = VaultDocsRunsRepo::new(mem_pool().await);
        let mut r = row("r1", 1, "docs", "running", "2026-07-12T10:00:00Z");
        repo.upsert(&r).await.unwrap();
        assert_eq!(repo.get("r1").await.unwrap().unwrap().state, "running");

        r.state = "done".into();
        r.payload = r#"{"state":"done"}"#.into();
        r.finished_at = Some("2026-07-12T10:05:00Z".into());
        repo.upsert(&r).await.unwrap();
        let got = repo.get("r1").await.unwrap().unwrap();
        assert_eq!(got.state, "done");
        assert_eq!(got.payload, r#"{"state":"done"}"#);
        assert_eq!(got.finished_at.as_deref(), Some("2026-07-12T10:05:00Z"));
        assert!(repo.get("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_is_per_vault_newest_first_with_limit() {
        let repo = VaultDocsRunsRepo::new(mem_pool().await);
        repo.upsert(&row("a", 1, "docs", "done", "2026-07-12T10:00:00Z"))
            .await
            .unwrap();
        repo.upsert(&row("b", 1, "refine", "done", "2026-07-12T11:00:00Z"))
            .await
            .unwrap();
        repo.upsert(&row("c", 2, "docs", "done", "2026-07-12T12:00:00Z"))
            .await
            .unwrap();

        let v1 = repo.list_for_vault(1, 50).await.unwrap();
        assert_eq!(
            v1.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert_eq!(repo.list_for_vault(1, 1).await.unwrap().len(), 1);
        assert_eq!(repo.list_for_vault(2, 50).await.unwrap()[0].id, "c");
        assert!(repo.list_for_vault(3, 50).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unfinished_means_running_summarizing_reviewing_or_revising() {
        let repo = VaultDocsRunsRepo::new(mem_pool().await);
        for (id, state) in [
            ("r1", "running"),
            ("r2", "summarizing"),
            ("r3", "reviewing"),
            ("r4", "revising"),
            ("r5", "done"),
            ("r6", "done_with_findings"),
            ("r7", "error"),
            ("r8", "cancelled"),
            ("r9", "interrupted"),
        ] {
            repo.upsert(&row(id, 1, "docs", state, "2026-07-12T10:00:00Z"))
                .await
                .unwrap();
        }
        let mut ids: Vec<String> = repo
            .list_unfinished()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["r1", "r2", "r3", "r4"]);
    }
}
