//! Workbench archive: the scratch docs Otto kept in the DB before scratches
//! became real files (the Scratches project of the IDE).
//!
//! The tables of migrations 0165/0184 (`workbench_docs`, `workbench_blobs`,
//! `workbench_revisions`, `workbench_assets`) are never written or dropped any
//! more: they are the archive, and an older build still reads them after a
//! rollback. The only thing the current build does with them is copy each live
//! doc into its owner's Scratches folder ONCE — [`WorkbenchArchive::unexported`]
//! lists what is still to copy, [`WorkbenchArchive::mark_exported`] records
//! where a doc went (migration 0219), so the copy is idempotent.

use crate::DbPool;
use otto_core::Result;
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::convert::dberr;

/// Hex SHA-256 of CONTENT, used as an identity/change-detection digest (the
/// correlation digest of a long tool argument in the MCP audit log). It never
/// hashes a password, token or other secret for storage or comparison — a
/// credential would need a keyed or password hash instead (CodeQL
/// `rust/weak-sensitive-data-hashing` is a false positive here; the name makes
/// the purpose explicit).
pub fn content_digest_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A live (not trashed) doc that has not been copied to the Scratches folder.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchivedDoc {
    pub id: String,
    pub name: String,
    /// `auto` or an explicit language id (`json`, `sql`, `md`, …, `image`).
    pub language: String,
    /// `/`-separated sub-folder (`''` = top level).
    pub folder: String,
    /// The text, or an `image` doc's picture bytes.
    pub bytes: Vec<u8>,
    /// The picture's MIME type for an `image` doc, `None` for text.
    pub mime: Option<String>,
}

#[derive(Clone)]
pub struct WorkbenchArchive {
    pool: DbPool,
}

impl WorkbenchArchive {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        Self { pool: pool.into() }
    }

    /// Every live doc of `owner`, from EVERY workspace, not exported yet —
    /// oldest first, so a name clash keeps the older doc's plain name. An
    /// `image` doc carries its asset's bytes (it is skipped when the asset is
    /// gone: there is nothing to copy).
    pub async fn unexported(&self, owner: &str) -> Result<Vec<ArchivedDoc>> {
        let rows = sqlx::query(
            "SELECT d.id, d.name, d.language, d.folder, b.content, a.mime, a.data \
             FROM workbench_docs d \
             JOIN workbench_blobs b ON b.hash = d.content_hash \
             LEFT JOIN workbench_assets a \
               ON d.language = 'image' AND a.id = TRIM(b.content) AND a.owner_id = d.owner_id \
             WHERE d.owner_id = ? AND d.deleted_at IS NULL AND d.exported_path IS NULL \
             ORDER BY d.created_at, d.id",
        )
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list unexported workbench docs"))?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let language: String = r.get("language");
                let mime: Option<String> = r.get("mime");
                let bytes = if language == "image" {
                    r.get::<Option<Vec<u8>>, _>("data")?
                } else {
                    r.get::<String, _>("content").into_bytes()
                };
                Some(ArchivedDoc {
                    id: r.get("id"),
                    name: r.get("name"),
                    language,
                    folder: r.get("folder"),
                    bytes,
                    mime,
                })
            })
            .collect())
    }

    /// Record that doc `id` now lives at `path` (relative to the Scratches folder).
    pub async fn mark_exported(&self, id: &str, path: &str) -> Result<()> {
        sqlx::query("UPDATE workbench_docs SET exported_path = ? WHERE id = ?")
            .bind(path)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("mark workbench doc exported"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
    }

    async fn insert(
        pool: &DbPool,
        id: &str,
        owner: &str,
        lang: &str,
        content: &str,
        trashed: bool,
    ) {
        let hash = content_digest_hex(content.as_bytes());
        sqlx::query("INSERT OR IGNORE INTO workbench_blobs (hash, content, size) VALUES (?, ?, ?)")
            .bind(&hash)
            .bind(content)
            .bind(content.len() as i64)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO workbench_docs (id, workspace_id, owner_id, name, language, folder, \
             content_hash, size, created_at, updated_at, deleted_at) \
             VALUES (?, 'ws', ?, ?, ?, 'a/b', ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(owner)
        .bind(format!("doc-{id}"))
        .bind(lang)
        .bind(&hash)
        .bind(content.len() as i64)
        .bind(format!("2026-01-01T00:00:0{id}Z"))
        .bind("2026-01-01T00:00:00Z")
        .bind(trashed.then_some("2026-02-01T00:00:00Z"))
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn lists_live_unexported_docs_of_one_owner_until_marked() {
        let pool = mem_pool().await;
        insert(&pool, "1", "alice", "sql", "SELECT 1", false).await;
        insert(&pool, "2", "alice", "md", "# gone", true).await;
        insert(&pool, "3", "bob", "md", "# bob", false).await;
        insert(&pool, "4", "alice", "image", "asset-1", false).await;
        sqlx::query(
            "INSERT INTO workbench_assets (id, workspace_id, owner_id, mime, size, sha256, data, created_at) \
             VALUES ('asset-1', 'ws', 'alice', 'image/png', 3, 'x', X'010203', 'now')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let repo = WorkbenchArchive::new(pool.clone());

        let docs = repo.unexported("alice").await.unwrap();
        let ids: Vec<&str> = docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["1", "4"], "trashed + other owners' docs are skipped");
        assert_eq!(docs[0].bytes, b"SELECT 1");
        assert_eq!(
            (docs[0].folder.as_str(), docs[0].mime.as_deref()),
            ("a/b", None)
        );
        assert_eq!(
            (docs[1].bytes.as_slice(), docs[1].mime.as_deref()),
            (&[1u8, 2, 3][..], Some("image/png"))
        );

        repo.mark_exported("1", "a/b/doc-1.sql").await.unwrap();
        let left: Vec<String> = repo
            .unexported("alice")
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(left, ["4"]);
    }
}
