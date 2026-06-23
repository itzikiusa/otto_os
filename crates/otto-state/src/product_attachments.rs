//! Product story attachment repository: files/images attached to stories.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

// ---------------------------------------------------------------------------
// Domain structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductAttachment {
    pub id: Id,
    pub story_id: Id,
    pub workspace_id: Id,
    pub filename: String,
    pub mime: String,
    pub size_bytes: i64,
    pub sha256: Option<String>,
    pub storage_path: String,
    pub kind: String,
    pub source: String,
    pub meta_json: Option<String>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Input structs
// ---------------------------------------------------------------------------

pub struct NewAttachment {
    pub story_id: Id,
    pub workspace_id: Id,
    pub filename: String,
    pub mime: String,
    pub size_bytes: i64,
    pub sha256: Option<String>,
    pub storage_path: String,
    pub kind: String,
    pub source: String,
    pub meta_json: Option<String>,
    pub created_by: Id,
}

#[derive(Default)]
pub struct AttachmentPatch {
    pub kind: Option<String>,
    pub filename: Option<String>,
}

// ---------------------------------------------------------------------------
// Row conversion
// ---------------------------------------------------------------------------

fn row_to_attachment(r: &sqlx::sqlite::SqliteRow) -> Result<ProductAttachment> {
    Ok(ProductAttachment {
        id: r.get("id"),
        story_id: r.get("story_id"),
        workspace_id: r.get("workspace_id"),
        filename: r.get("filename"),
        mime: r.get("mime"),
        size_bytes: r.get("size_bytes"),
        sha256: r.get("sha256"),
        storage_path: r.get("storage_path"),
        kind: r.get("kind"),
        source: r.get("source"),
        meta_json: r.get("meta_json"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repo
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ProductAttachmentRepo {
    pool: SqlitePool,
}

impl ProductAttachmentRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, a: NewAttachment) -> Result<ProductAttachment> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_attachments
             (id, story_id, workspace_id, filename, mime, size_bytes, sha256,
              storage_path, kind, source, meta_json, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.story_id)
        .bind(&a.workspace_id)
        .bind(&a.filename)
        .bind(&a.mime)
        .bind(a.size_bytes)
        .bind(&a.sha256)
        .bind(&a.storage_path)
        .bind(&a.kind)
        .bind(&a.source)
        .bind(&a.meta_json)
        .bind(&a.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create attachment"))?;
        self.get_required(&id).await
    }

    /// Fetch by id, returning `Err(NotFound)` when the row is absent.
    async fn get_required(&self, id: &Id) -> Result<ProductAttachment> {
        self.get(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("attachment {id}")))
    }

    pub async fn get(&self, id: &Id) -> Result<Option<ProductAttachment>> {
        let row = sqlx::query("SELECT * FROM product_attachments WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get attachment"))?;
        row.as_ref().map(row_to_attachment).transpose()
    }

    pub async fn list_for_story(&self, story_id: &Id) -> Result<Vec<ProductAttachment>> {
        let rows =
            sqlx::query("SELECT * FROM product_attachments WHERE story_id = ? ORDER BY created_at ASC")
                .bind(story_id)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("list attachments for story"))?;
        rows.iter().map(row_to_attachment).collect()
    }

    pub async fn update(&self, id: &Id, p: AttachmentPatch) -> Result<ProductAttachment> {
        let existing = self.get_required(id).await?;
        let kind = p.kind.as_deref().unwrap_or(&existing.kind);
        let filename = p.filename.as_deref().unwrap_or(&existing.filename);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE product_attachments SET kind = ?, filename = ?, updated_at = ? WHERE id = ?",
        )
        .bind(kind)
        .bind(filename)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update attachment"))?;
        self.get_required(id).await
    }

    /// Delete the attachment row. Returns `Ok(())` even if the row was absent.
    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM product_attachments WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete attachment"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn create_get_list_update_delete_roundtrip() {
        let pool = mem_pool().await;
        let repo = ProductAttachmentRepo::new(pool);
        let a = repo
            .create(NewAttachment {
                story_id: "s1".into(),
                workspace_id: "w1".into(),
                filename: "x.png".into(),
                mime: "image/png".into(),
                size_bytes: 10,
                sha256: None,
                storage_path: "product/attachments/s1/a1.png".into(),
                kind: "image".into(),
                source: "user".into(),
                meta_json: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        assert_eq!(repo.get(&a.id).await.unwrap().unwrap().filename, "x.png");
        assert_eq!(repo.list_for_story(&"s1".into()).await.unwrap().len(), 1);
        let upd = repo
            .update(
                &a.id,
                AttachmentPatch {
                    kind: Some("mockup".into()),
                    filename: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(upd.kind, "mockup");
        repo.delete(&a.id).await.unwrap();
        assert!(repo.get(&a.id).await.unwrap().is_none());
    }
}
