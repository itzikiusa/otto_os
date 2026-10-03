//! Persisted Proof Pack snapshots — the exportable evidence bundle for a review.
//! Namespaced `review_proof_packs` to avoid colliding with a parallel
//! `feat/proof-packs` branch's generic ProofPack. The live pack is assembled on
//! demand by the server; export persists a snapshot here (for audit/share).

use crate::DbPool;
use chrono::Utc;
use sqlx::Row;

use crate::convert::{dberr, fmt};
use otto_core::finding::ReviewProofPackExport;
use otto_core::{new_id, Result};

/// Export snapshots kept per review; each export stores the whole rendered
/// markdown, so an unbounded history grew forever (PP-08).
pub const KEEP_EXPORTS_PER_REVIEW: u32 = 5;

#[derive(Clone)]
pub struct ReviewProofPacksRepo {
    pool: DbPool,
}

impl ReviewProofPacksRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    pub async fn create(
        &self,
        review_id: &str,
        workspace_id: &str,
        format: &str,
        content: &str,
        summary_json: &str,
        created_by: &str,
    ) -> Result<ReviewProofPackExport> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO review_proof_packs \
             (id, review_id, workspace_id, format, content, summary_json, created_by, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(review_id)
        .bind(workspace_id)
        .bind(format)
        .bind(content)
        .bind(summary_json)
        .bind(created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create proof pack"))?;
        // Best-effort: a failed prune never fails the export.
        if let Err(e) = self.prune(review_id, KEEP_EXPORTS_PER_REVIEW).await {
            tracing::warn!("review proof pack prune failed: {e}");
        }
        Ok(ReviewProofPackExport {
            id,
            review_id: review_id.to_string(),
            format: format.to_string(),
            markdown: content.to_string(),
            created_at: now,
        })
    }

    /// Keep only the newest `keep` export snapshots of a review. Returns how
    /// many were deleted. Walks `idx_review_proof_packs_review`.
    pub async fn prune(&self, review_id: &str, keep: u32) -> Result<u64> {
        let r = sqlx::query(
            "DELETE FROM review_proof_packs WHERE review_id = ? AND id NOT IN ( \
               SELECT id FROM review_proof_packs WHERE review_id = ? \
               ORDER BY created_at DESC, id DESC LIMIT ?)",
        )
        .bind(review_id)
        .bind(review_id)
        .bind(keep.max(1) as i64)
        .execute(&self.pool)
        .await
        .map_err(dberr("prune review proof packs"))?;
        Ok(r.rows_affected())
    }

    /// Snapshots for a review, newest first.
    pub async fn list_for_review(&self, review_id: &str) -> Result<Vec<ReviewProofPackExport>> {
        let rows = sqlx::query(
            "SELECT id, review_id, format, content, created_at FROM review_proof_packs \
             WHERE review_id = ? ORDER BY created_at DESC",
        )
        .bind(review_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list proof packs"))?;
        rows.iter()
            .map(|r| {
                Ok(ReviewProofPackExport {
                    id: r.get("id"),
                    review_id: r.get("review_id"),
                    format: r
                        .try_get("format")
                        .unwrap_or_else(|_| "markdown".to_string()),
                    markdown: r.try_get("content").unwrap_or_default(),
                    created_at: r.try_get("created_at").unwrap_or_default(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
    }

    #[tokio::test]
    async fn create_and_list_snapshots() {
        let repo = ReviewProofPacksRepo::new(mem_pool().await);
        let pack = repo
            .create(
                "rev1",
                "ws1",
                "markdown",
                "# Proof Pack\n...",
                "{\"total\":3}",
                "u1",
            )
            .await
            .unwrap();
        assert_eq!(pack.format, "markdown");
        assert!(pack.markdown.contains("Proof Pack"));
        let list = repo.list_for_review("rev1").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].review_id, "rev1");
    }

    #[tokio::test]
    async fn proof_exports_keep_only_the_newest_per_review() {
        let repo = ReviewProofPacksRepo::new(mem_pool().await);
        let mut last = String::new();
        for i in 0..(KEEP_EXPORTS_PER_REVIEW + 3) {
            last = repo
                .create("rev1", "ws1", "markdown", &format!("# {i}"), "{}", "u1")
                .await
                .unwrap()
                .id;
        }
        repo.create("rev2", "ws1", "markdown", "# other", "{}", "u1")
            .await
            .unwrap();
        let list = repo.list_for_review("rev1").await.unwrap();
        assert_eq!(list.len(), KEEP_EXPORTS_PER_REVIEW as usize);
        assert!(list.iter().any(|e| e.id == last));
        assert_eq!(repo.list_for_review("rev2").await.unwrap().len(), 1);
    }
}
