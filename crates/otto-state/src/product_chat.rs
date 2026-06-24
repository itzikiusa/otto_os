//! Discovery-chat repository.
//!
//! A discovery chat is a lightweight, interactive conversation attached to a
//! product story (works even on an empty/Untitled draft) to help with early
//! discovery and research BEFORE a story is written. Distinct from the
//! heavyweight swarm discovery runs (`product_discovery`) and from per-version
//! refinement threads (`product_refinement`).

use chrono::{DateTime, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

// ---------------------------------------------------------------------------
// Domain structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryChat {
    pub id: Id,
    pub story_id: Id,
    pub workspace_id: Id,
    pub cwd: String,
    pub title: String,
    pub status: String,
    pub model: Option<String>,
    /// The managed Otto session backing this chat (visible/resumable in Agents).
    /// `None` until the first turn creates it.
    pub session_id: Option<Id>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryChatMessage {
    pub id: Id,
    pub chat_id: Id,
    pub role: String,
    pub body: String,
    pub actions_json: Option<String>,
    pub meta_json: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Input structs
// ---------------------------------------------------------------------------

pub struct NewDiscoveryChat {
    pub story_id: Id,
    pub workspace_id: Id,
    pub cwd: String,
    pub title: String,
    pub model: Option<String>,
    pub created_by: Id,
}

pub struct NewDiscoveryChatMessage {
    pub chat_id: Id,
    pub role: String,
    pub body: String,
    pub actions_json: Option<String>,
    pub meta_json: Option<String>,
}

// ---------------------------------------------------------------------------
// Row conversion
// ---------------------------------------------------------------------------

fn row_to_chat(r: &sqlx::sqlite::SqliteRow) -> Result<DiscoveryChat> {
    Ok(DiscoveryChat {
        id: r.get("id"),
        story_id: r.get("story_id"),
        workspace_id: r.get("workspace_id"),
        cwd: r.get("cwd"),
        title: r.get("title"),
        status: r.get("status"),
        model: r.get("model"),
        session_id: r.get("session_id"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_message(r: &sqlx::sqlite::SqliteRow) -> Result<DiscoveryChatMessage> {
    Ok(DiscoveryChatMessage {
        id: r.get("id"),
        chat_id: r.get("chat_id"),
        role: r.get("role"),
        body: r.get("body"),
        actions_json: r.get("actions_json"),
        meta_json: r.get("meta_json"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repo
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DiscoveryChatRepo {
    pool: SqlitePool,
}

impl DiscoveryChatRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_chat(&self, r: NewDiscoveryChat) -> Result<DiscoveryChat> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_discovery_chats
             (id, story_id, workspace_id, cwd, title, status, model,
              created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&r.story_id)
        .bind(&r.workspace_id)
        .bind(&r.cwd)
        .bind(&r.title)
        .bind(&r.model)
        .bind(&r.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create discovery chat"))?;
        self.get_chat_required(&id).await
    }

    async fn get_chat_required(&self, id: &Id) -> Result<DiscoveryChat> {
        self.get_chat(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("discovery chat {id}")))
    }

    pub async fn get_chat(&self, id: &Id) -> Result<Option<DiscoveryChat>> {
        let row = sqlx::query("SELECT * FROM product_discovery_chats WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get discovery chat"))?;
        row.as_ref().map(row_to_chat).transpose()
    }

    /// List chats for a story, newest first.
    pub async fn list_for_story(&self, story_id: &Id) -> Result<Vec<DiscoveryChat>> {
        let rows = sqlx::query(
            "SELECT * FROM product_discovery_chats WHERE story_id = ? ORDER BY created_at DESC",
        )
        .bind(story_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list discovery chats for story"))?;
        rows.iter().map(row_to_chat).collect()
    }

    pub async fn get_messages(&self, chat_id: &Id) -> Result<Vec<DiscoveryChatMessage>> {
        let rows = sqlx::query(
            "SELECT * FROM product_discovery_chat_messages
             WHERE chat_id = ? ORDER BY created_at ASC",
        )
        .bind(chat_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list discovery chat messages"))?;
        rows.iter().map(row_to_message).collect()
    }

    pub async fn add_message(&self, r: NewDiscoveryChatMessage) -> Result<DiscoveryChatMessage> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO product_discovery_chat_messages
             (id, chat_id, role, body, actions_json, meta_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&r.chat_id)
        .bind(&r.role)
        .bind(&r.body)
        .bind(&r.actions_json)
        .bind(&r.meta_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add discovery chat message"))?;
        // bump the chat's updated_at so lists sort sensibly
        let _ = sqlx::query("UPDATE product_discovery_chats SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(&r.chat_id)
            .execute(&self.pool)
            .await;
        self.get_message_required(&id).await
    }

    async fn get_message_required(&self, id: &Id) -> Result<DiscoveryChatMessage> {
        let row = sqlx::query("SELECT * FROM product_discovery_chat_messages WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get discovery chat message"))?;
        row.as_ref()
            .map(row_to_message)
            .transpose()?
            .ok_or_else(|| Error::NotFound(format!("discovery chat message {id}")))
    }

    pub async fn set_status(&self, id: &Id, status: &str) -> Result<DiscoveryChat> {
        let now = fmt(Utc::now());
        let result = sqlx::query(
            "UPDATE product_discovery_chats SET status = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set discovery chat status"))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound(format!("discovery chat {id}")));
        }
        self.get_chat_required(id).await
    }

    /// Link the managed session backing this chat (set once, on the first turn).
    pub async fn set_session(&self, id: &Id, session_id: &Id) -> Result<()> {
        let now = fmt(Utc::now());
        let result = sqlx::query(
            "UPDATE product_discovery_chats SET session_id = ?, updated_at = ? WHERE id = ?",
        )
        .bind(session_id)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set discovery chat session"))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound(format!("discovery chat {id}")));
        }
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
    async fn chat_message_roundtrip() {
        let pool = mem_pool().await;
        let repo = DiscoveryChatRepo::new(pool);

        let chat = repo
            .create_chat(NewDiscoveryChat {
                story_id: "s1".into(),
                workspace_id: "w1".into(),
                cwd: "/tmp".into(),
                title: "Discovery".into(),
                model: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        assert_eq!(chat.status, "active");

        let list = repo.list_for_story(&"s1".into()).await.unwrap();
        assert_eq!(list.len(), 1);

        repo.add_message(NewDiscoveryChatMessage {
            chat_id: chat.id.clone(),
            role: "user".into(),
            body: "help me scope X".into(),
            actions_json: None,
            meta_json: Some(r#"{"ctx":"..."}"#.into()),
        })
        .await
        .unwrap();
        let agent = repo
            .add_message(NewDiscoveryChatMessage {
                chat_id: chat.id.clone(),
                role: "agent".into(),
                body: "here's a plan".into(),
                actions_json: Some(r#"[{"type":"add_questions"}]"#.into()),
                meta_json: None,
            })
            .await
            .unwrap();
        assert_eq!(agent.role, "agent");

        let msgs = repo.get_messages(&chat.id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user"); // ascending by created_at
        assert!(msgs[1].actions_json.is_some());

        let archived = repo.set_status(&chat.id, "archived").await.unwrap();
        assert_eq!(archived.status, "archived");
    }
}
