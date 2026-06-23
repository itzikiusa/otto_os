//! Product story refinement thread + message repository.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

// ---------------------------------------------------------------------------
// Domain structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementThread {
    pub id: Id,
    pub story_id: Id,
    pub workspace_id: Id,
    pub discovery_run_id: Option<String>,
    pub cwd: String,
    pub title: String,
    pub status: String,
    pub model: Option<String>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementMessage {
    pub id: Id,
    pub thread_id: Id,
    pub role: String,
    pub body: String,
    pub meta_json: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Input structs
// ---------------------------------------------------------------------------

pub struct NewRefinementThread {
    pub story_id: Id,
    pub workspace_id: Id,
    pub discovery_run_id: Option<String>,
    pub cwd: String,
    pub title: String,
    pub model: Option<String>,
    pub created_by: Id,
}

pub struct NewRefinementMessage {
    pub thread_id: Id,
    pub role: String,
    pub body: String,
    pub meta_json: Option<String>,
}

// ---------------------------------------------------------------------------
// Row conversion
// ---------------------------------------------------------------------------

fn row_to_thread(r: &sqlx::sqlite::SqliteRow) -> Result<RefinementThread> {
    Ok(RefinementThread {
        id: r.get("id"),
        story_id: r.get("story_id"),
        workspace_id: r.get("workspace_id"),
        discovery_run_id: r.get("discovery_run_id"),
        cwd: r.get("cwd"),
        title: r.get("title"),
        status: r.get("status"),
        model: r.get("model"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_message(r: &sqlx::sqlite::SqliteRow) -> Result<RefinementMessage> {
    Ok(RefinementMessage {
        id: r.get("id"),
        thread_id: r.get("thread_id"),
        role: r.get("role"),
        body: r.get("body"),
        meta_json: r.get("meta_json"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repo
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ProductRefinementRepo {
    pool: SqlitePool,
}

impl ProductRefinementRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create a new refinement thread; `status` defaults to `'active'`.
    pub async fn create_thread(&self, t: NewRefinementThread) -> Result<RefinementThread> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_refinement_threads
             (id, story_id, workspace_id, discovery_run_id, cwd, title, status,
              model, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&t.story_id)
        .bind(&t.workspace_id)
        .bind(&t.discovery_run_id)
        .bind(&t.cwd)
        .bind(&t.title)
        .bind(&t.model)
        .bind(&t.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create refinement thread"))?;
        self.get_thread_required(&id).await
    }

    async fn get_thread_required(&self, id: &Id) -> Result<RefinementThread> {
        self.get_thread(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("refinement thread {id}")))
    }

    pub async fn get_thread(&self, id: &Id) -> Result<Option<RefinementThread>> {
        let row = sqlx::query("SELECT * FROM product_refinement_threads WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get refinement thread"))?;
        row.as_ref().map(row_to_thread).transpose()
    }

    /// List threads for a story, newest first.
    pub async fn list_threads_for_story(&self, story_id: &Id) -> Result<Vec<RefinementThread>> {
        let rows = sqlx::query(
            "SELECT * FROM product_refinement_threads WHERE story_id = ? ORDER BY created_at DESC",
        )
        .bind(story_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list refinement threads for story"))?;
        rows.iter().map(row_to_thread).collect()
    }

    /// Archive a thread: sets `status` to `'archived'` and bumps `updated_at`.
    pub async fn archive_thread(&self, id: &Id) -> Result<()> {
        let now = fmt(Utc::now());
        let result = sqlx::query(
            "UPDATE product_refinement_threads SET status = 'archived', updated_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("archive refinement thread"))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound(format!("refinement thread {id}")));
        }
        Ok(())
    }

    /// Create a new message in a thread.
    pub async fn create_message(&self, m: NewRefinementMessage) -> Result<RefinementMessage> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_refinement_messages
             (id, thread_id, role, body, meta_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&m.thread_id)
        .bind(&m.role)
        .bind(&m.body)
        .bind(&m.meta_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create refinement message"))?;
        self.get_message_required(&id).await
    }

    async fn get_message_required(&self, id: &Id) -> Result<RefinementMessage> {
        let row = sqlx::query("SELECT * FROM product_refinement_messages WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get refinement message"))?;
        row.as_ref()
            .map(row_to_message)
            .transpose()?
            .ok_or_else(|| Error::NotFound(format!("refinement message {id}")))
    }

    /// List messages for a thread, oldest first (chronological for replay).
    pub async fn list_messages(&self, thread_id: &Id) -> Result<Vec<RefinementMessage>> {
        let rows = sqlx::query(
            "SELECT * FROM product_refinement_messages WHERE thread_id = ? ORDER BY created_at ASC",
        )
        .bind(thread_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list refinement messages"))?;
        rows.iter().map(row_to_message).collect()
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
    async fn thread_and_message_roundtrip() {
        let pool = mem_pool().await;
        let repo = ProductRefinementRepo::new(pool);
        let t = repo.create_thread(NewRefinementThread{
            story_id:"s1".into(), workspace_id:"w1".into(), discovery_run_id:None,
            cwd:"/tmp/refine/t1".into(), title:"Sharpen ACs".into(), model:None,
            created_by:"u1".into() }).await.unwrap();
        assert_eq!(t.status, "active");
        assert_eq!(repo.get_thread(&t.id).await.unwrap().unwrap().title, "Sharpen ACs");
        assert_eq!(repo.list_threads_for_story(&"s1".into()).await.unwrap().len(), 1);

        let m1 = repo.create_message(NewRefinementMessage{ thread_id:t.id.clone(), role:"user".into(),
            body:"add edge cases".into(), meta_json:None }).await.unwrap();
        let _m2 = repo.create_message(NewRefinementMessage{ thread_id:t.id.clone(), role:"agent".into(),
            body:"done".into(), meta_json:Some(r#"{"story_updated":true,"version_no":3}"#.into()) }).await.unwrap();
        let msgs = repo.list_messages(&t.id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, m1.id);          // chronological (oldest first)
        assert_eq!(msgs[0].role, "user");

        repo.archive_thread(&t.id).await.unwrap();
        assert_eq!(repo.get_thread(&t.id).await.unwrap().unwrap().status, "archived");
    }
}
