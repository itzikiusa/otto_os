//! Persistence for the **dynamic model catalog** (`provider_models`).
//!
//! Rows are discovered at runtime by `otto-server`'s model-catalog refresher
//! (CLI probe / docs scrape / models.dev fallback) or added manually by the
//! user. The refresh contract is deliberately conservative:
//!
//!   * [`ProviderModelsRepo::upsert_batch`] replaces a provider's NON-manual
//!     rows atomically and is only called after a *successful* fetch — a failed
//!     source chain never touches the table, so the last good list survives
//!     outages (staleness is surfaced via `fetched_at` instead).
//!   * Manual rows (`source = 'manual'`) are user-owned and survive refreshes.
//!   * Order of appearance is meaningful (docs pages list the newest models
//!     first), so all reads ORDER BY the implicit rowid.

use chrono::Utc;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

/// One catalog entry: a model id a provider's CLI accepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModel {
    pub provider: String,
    pub model_id: String,
    pub label: String,
    pub notes: String,
    /// 'cli' | 'scrape' | 'catalog' | 'manual'.
    pub source: String,
    /// RFC3339 timestamp of the fetch (or of the manual add).
    pub fetched_at: String,
}

#[derive(Clone)]
pub struct ProviderModelsRepo {
    pool: SqlitePool,
}

fn row_to_model(r: &sqlx::sqlite::SqliteRow) -> ProviderModel {
    ProviderModel {
        provider: r.get("provider"),
        model_id: r.get("model_id"),
        label: r.get("label"),
        notes: r.get("notes"),
        source: r.get("source"),
        fetched_at: r.get("fetched_at"),
    }
}

impl ProviderModelsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Atomically replace `provider`'s fetched rows with `models` (id, label)
    /// pairs from `source` ('cli' | 'scrape' | 'catalog'). Manual rows are left
    /// untouched; a manual row also shadows an incoming fetched duplicate (the
    /// user's label/notes win). Callers MUST only invoke this on a successful
    /// fetch — an empty `models` is rejected so a botched parse can't blank the
    /// catalog.
    pub async fn upsert_batch(
        &self,
        provider: &str,
        source: &str,
        models: &[(String, String)],
    ) -> Result<()> {
        if models.is_empty() {
            return Err(Error::Invalid(format!(
                "refusing to replace '{provider}' models with an empty list"
            )));
        }
        let now = fmt(Utc::now());
        let mut tx = self.pool.begin().await.map_err(dberr("begin models upsert"))?;
        sqlx::query("DELETE FROM provider_models WHERE provider = ? AND source != 'manual'")
            .bind(provider)
            .execute(&mut *tx)
            .await
            .map_err(dberr("clear fetched models"))?;
        for (id, label) in models {
            // INSERT OR IGNORE: a manual row with the same id keeps the user's
            // label/notes (the PK collision is the shadowing rule, not an error).
            sqlx::query(
                "INSERT OR IGNORE INTO provider_models
                     (provider, model_id, label, notes, source, fetched_at)
                 VALUES (?, ?, ?, '', ?, ?)",
            )
            .bind(provider)
            .bind(id)
            .bind(label)
            .bind(source)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(dberr("insert model"))?;
        }
        tx.commit().await.map_err(dberr("commit models upsert"))
    }

    /// Every row, grouped by insert order within provider (rowid preserves the
    /// docs' order of appearance).
    pub async fn list_all(&self) -> Result<Vec<ProviderModel>> {
        let rows = sqlx::query("SELECT * FROM provider_models ORDER BY provider, rowid")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list provider models"))?;
        Ok(rows.iter().map(row_to_model).collect())
    }

    /// One provider's rows, in order of appearance.
    pub async fn list_for(&self, provider: &str) -> Result<Vec<ProviderModel>> {
        let rows = sqlx::query("SELECT * FROM provider_models WHERE provider = ? ORDER BY rowid")
            .bind(provider)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list provider models"))?;
        Ok(rows.iter().map(row_to_model).collect())
    }

    /// Add (or overwrite) a user-managed manual entry.
    pub async fn add_manual(
        &self,
        provider: &str,
        model_id: &str,
        label: &str,
        notes: &str,
    ) -> Result<()> {
        let id = model_id.trim();
        if id.is_empty() {
            return Err(Error::Invalid("model id is empty".into()));
        }
        sqlx::query(
            "INSERT INTO provider_models (provider, model_id, label, notes, source, fetched_at)
             VALUES (?, ?, ?, ?, 'manual', ?)
             ON CONFLICT(provider, model_id) DO UPDATE SET
                 label = excluded.label, notes = excluded.notes,
                 source = 'manual', fetched_at = excluded.fetched_at",
        )
        .bind(provider)
        .bind(id)
        .bind(if label.trim().is_empty() { id } else { label })
        .bind(notes)
        .bind(fmt(Utc::now()))
        .execute(&self.pool)
        .await
        .map_err(dberr("add manual model"))?;
        Ok(())
    }

    /// Delete one entry (manual or fetched — a fetched one comes back on the
    /// next refresh, which is the honest behavior).
    pub async fn delete(&self, provider: &str, model_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM provider_models WHERE provider = ? AND model_id = ?")
            .bind(provider)
            .bind(model_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete model"))?;
        Ok(())
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

    fn pairs(ids: &[&str]) -> Vec<(String, String)> {
        ids.iter().map(|i| (i.to_string(), i.to_string())).collect()
    }

    #[tokio::test]
    async fn upsert_replaces_fetched_preserves_manual_and_order() {
        let repo = ProviderModelsRepo::new(mem_pool().await);
        repo.add_manual("claude", "my-alias", "My Alias", "hand-added").await.unwrap();
        repo.upsert_batch("claude", "scrape", &pairs(&["claude-fable-5", "claude-opus-5"]))
            .await
            .unwrap();
        // Second refresh replaces the fetched set; manual row survives.
        repo.upsert_batch("claude", "scrape", &pairs(&["claude-opus-5", "claude-sonnet-5"]))
            .await
            .unwrap();
        let rows = repo.list_for("claude").await.unwrap();
        let ids: Vec<&str> = rows.iter().map(|m| m.model_id.as_str()).collect();
        // manual first (oldest rowid), then fetch order of appearance.
        assert_eq!(ids, vec!["my-alias", "claude-opus-5", "claude-sonnet-5"]);
        assert_eq!(rows[0].source, "manual");
    }

    #[tokio::test]
    async fn manual_shadows_fetched_duplicate() {
        let repo = ProviderModelsRepo::new(mem_pool().await);
        repo.add_manual("codex", "gpt-5.4", "my gpt", "pinned label").await.unwrap();
        repo.upsert_batch("codex", "scrape", &pairs(&["gpt-5.4", "gpt-5.5"])).await.unwrap();
        let rows = repo.list_for("codex").await.unwrap();
        let mine = rows.iter().find(|m| m.model_id == "gpt-5.4").unwrap();
        assert_eq!(mine.source, "manual");
        assert_eq!(mine.label, "my gpt");
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn empty_batch_rejected_last_good_kept() {
        let repo = ProviderModelsRepo::new(mem_pool().await);
        repo.upsert_batch("agy", "cli", &pairs(&["gemini-3.1-pro-high"])).await.unwrap();
        assert!(repo.upsert_batch("agy", "cli", &[]).await.is_err());
        assert_eq!(repo.list_for("agy").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_and_list_all_grouping() {
        let repo = ProviderModelsRepo::new(mem_pool().await);
        repo.upsert_batch("claude", "scrape", &pairs(&["claude-fable-5"])).await.unwrap();
        repo.upsert_batch("agy", "cli", &pairs(&["gemini-3.1-pro-high"])).await.unwrap();
        repo.delete("claude", "claude-fable-5").await.unwrap();
        let all = repo.list_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].provider, "agy");
    }
}
