//! Product mockup annotation repository: pinned annotations on attachment mockups.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

// ---------------------------------------------------------------------------
// Domain structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockupAnnotation {
    pub id: Id,
    pub attachment_id: Id,
    pub story_id: Id,
    pub workspace_id: Id,
    pub x_pct: f64,
    pub y_pct: f64,
    pub body: String,
    pub resolved: bool,
    pub author_id: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Input structs
// ---------------------------------------------------------------------------

pub struct NewAnnotation {
    pub attachment_id: Id,
    pub story_id: Id,
    pub workspace_id: Id,
    pub x_pct: f64,
    pub y_pct: f64,
    pub body: String,
    pub author_id: Id,
}

#[derive(Default)]
pub struct AnnotationPatch {
    pub body: Option<String>,
    pub resolved: Option<bool>,
}

// ---------------------------------------------------------------------------
// Row conversion
// ---------------------------------------------------------------------------

fn row_to_annotation(r: &sqlx::sqlite::SqliteRow) -> Result<MockupAnnotation> {
    Ok(MockupAnnotation {
        id: r.get("id"),
        attachment_id: r.get("attachment_id"),
        story_id: r.get("story_id"),
        workspace_id: r.get("workspace_id"),
        x_pct: r.get("x_pct"),
        y_pct: r.get("y_pct"),
        body: r.get("body"),
        resolved: r.get::<i64, _>("resolved") != 0,
        author_id: r.get("author_id"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repo
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ProductMockupRepo {
    pool: SqlitePool,
}

impl ProductMockupRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, a: NewAnnotation) -> Result<MockupAnnotation> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_mockup_annotations
             (id, attachment_id, story_id, workspace_id, x_pct, y_pct, body, resolved,
              author_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&a.attachment_id)
        .bind(&a.story_id)
        .bind(&a.workspace_id)
        .bind(a.x_pct)
        .bind(a.y_pct)
        .bind(&a.body)
        .bind(&a.author_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create annotation"))?;
        self.get(&id).await.map(|opt| opt.expect("just inserted"))
    }

    pub async fn get(&self, id: &Id) -> Result<Option<MockupAnnotation>> {
        let row = sqlx::query("SELECT * FROM product_mockup_annotations WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get annotation"))?;
        row.as_ref().map(row_to_annotation).transpose()
    }

    pub async fn list_for_attachment(&self, attachment_id: &Id) -> Result<Vec<MockupAnnotation>> {
        let rows = sqlx::query(
            "SELECT * FROM product_mockup_annotations WHERE attachment_id = ? ORDER BY created_at ASC",
        )
        .bind(attachment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list annotations for attachment"))?;
        rows.iter().map(row_to_annotation).collect()
    }

    pub async fn update(&self, id: &Id, p: AnnotationPatch) -> Result<MockupAnnotation> {
        let now = fmt(Utc::now());
        if let Some(body) = &p.body {
            sqlx::query(
                "UPDATE product_mockup_annotations SET body = ?, updated_at = ? WHERE id = ?",
            )
            .bind(body)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update annotation body"))?;
        }
        if let Some(resolved) = p.resolved {
            sqlx::query(
                "UPDATE product_mockup_annotations SET resolved = ?, updated_at = ? WHERE id = ?",
            )
            .bind(resolved as i64)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update annotation resolved"))?;
        }
        self.get(id)
            .await
            .map(|opt| opt.expect("annotation not found after update"))
    }

    /// Delete the annotation row. Returns `Ok(())` even if absent.
    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM product_mockup_annotations WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete annotation"))?;
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
    async fn create_list_update_delete_roundtrip() {
        let pool = mem_pool().await;
        let repo = ProductMockupRepo::new(pool);

        let ann = repo
            .create(NewAnnotation {
                attachment_id: "att1".into(),
                story_id: "s1".into(),
                workspace_id: "w1".into(),
                x_pct: 0.25,
                y_pct: 0.75,
                body: "Fix this button".into(),
                author_id: "u1".into(),
            })
            .await
            .unwrap();

        assert!((ann.x_pct - 0.25).abs() < f64::EPSILON);
        assert!((ann.y_pct - 0.75).abs() < f64::EPSILON);
        assert!(!ann.resolved);

        // list returns it
        let list = repo.list_for_attachment(&"att1".into()).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, ann.id);

        // update body + resolved
        let upd = repo
            .update(
                &ann.id,
                AnnotationPatch {
                    body: Some("Updated note".into()),
                    resolved: Some(true),
                },
            )
            .await
            .unwrap();
        assert_eq!(upd.body, "Updated note");
        assert!(upd.resolved);

        // delete removes
        repo.delete(&ann.id).await.unwrap();
        assert!(repo.get(&ann.id).await.unwrap().is_none());
    }
}
