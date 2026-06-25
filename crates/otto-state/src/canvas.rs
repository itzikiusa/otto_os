//! Canvas Studio scene repository.
//!
//! A scene is one portable JSON document (`doc_json` — nodes/edges/slides/
//! appState; the rich schema lives in the UI `types.ts`). The Rust side treats
//! the document as opaque text and only owns the metadata (title, workspace,
//! optional story link, timestamps) needed for listing and access control.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

// ---------------------------------------------------------------------------
// Domain structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasScene {
    pub id: Id,
    pub workspace_id: Id,
    pub story_id: Option<Id>,
    pub title: String,
    pub doc_json: String,
    pub thumbnail: Option<String>,
    /// Which agent drives this scene's "Ask AI" turns (default `"claude"`).
    pub provider: String,
    /// Folder path used to group scenes in the UI (e.g. `"Platform/Staging"`).
    /// `None` = root/ungrouped.
    pub section: Option<String>,
    /// The managed Otto session backing this scene's "Ask AI" (resumable in
    /// Agents). `None` until the first assist turn creates it.
    pub session_id: Option<Id>,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Lightweight row for scene lists (omits the potentially-large `doc_json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasSceneSummary {
    pub id: Id,
    pub workspace_id: Id,
    pub story_id: Option<Id>,
    pub title: String,
    pub thumbnail: Option<String>,
    /// Folder path used to group scenes in the UI. `None` = root/ungrouped.
    pub section: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Input structs
// ---------------------------------------------------------------------------

pub struct NewScene {
    pub workspace_id: Id,
    pub story_id: Option<Id>,
    pub title: String,
    pub doc_json: String,
    /// Which agent drives "Ask AI" for this scene (default `"claude"`).
    pub provider: String,
    /// Optional folder path used to group scenes in the UI.
    pub section: Option<String>,
    pub created_by: Id,
}

/// Partial update — `None` fields are left unchanged.
#[derive(Default)]
pub struct SceneUpdate {
    pub title: Option<String>,
    pub doc_json: Option<String>,
    pub thumbnail: Option<String>,
    pub provider: Option<String>,
    pub section: Option<String>,
    /// Link/relink this scene to a product story (COALESCE — keeps prior on None).
    pub story_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Row conversion
// ---------------------------------------------------------------------------

fn row_to_scene(r: &sqlx::sqlite::SqliteRow) -> Result<CanvasScene> {
    Ok(CanvasScene {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        story_id: r.get("story_id"),
        title: r.get("title"),
        doc_json: r.get("doc_json"),
        thumbnail: r.get("thumbnail"),
        provider: r.get("provider"),
        section: r.get("section"),
        session_id: r.get("session_id"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_summary(r: &sqlx::sqlite::SqliteRow) -> Result<CanvasSceneSummary> {
    Ok(CanvasSceneSummary {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        story_id: r.get("story_id"),
        title: r.get("title"),
        thumbnail: r.get("thumbnail"),
        section: r.get("section"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Repo
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct CanvasRepo {
    pool: SqlitePool,
}

impl CanvasRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, r: NewScene) -> Result<CanvasScene> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO canvas_scenes
             (id, workspace_id, story_id, title, doc_json, thumbnail,
              provider, section, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&r.workspace_id)
        .bind(&r.story_id)
        .bind(&r.title)
        .bind(&r.doc_json)
        .bind(&r.provider)
        .bind(&r.section)
        .bind(&r.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create canvas scene"))?;
        self.get_required(&id).await
    }

    async fn get_required(&self, id: &Id) -> Result<CanvasScene> {
        self.get(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("canvas scene {id}")))
    }

    pub async fn get(&self, id: &Id) -> Result<Option<CanvasScene>> {
        let row = sqlx::query("SELECT * FROM canvas_scenes WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get canvas scene"))?;
        row.as_ref().map(row_to_scene).transpose()
    }

    /// List scenes for a workspace, most-recently-updated first.
    pub async fn list_for_workspace(&self, ws: &Id) -> Result<Vec<CanvasSceneSummary>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, story_id, title, thumbnail, section, created_at, updated_at
             FROM canvas_scenes WHERE workspace_id = ? ORDER BY updated_at DESC",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list canvas scenes for workspace"))?;
        rows.iter().map(row_to_summary).collect()
    }

    /// List scenes linked to a product story, most-recently-updated first.
    pub async fn list_for_story(&self, story_id: &Id) -> Result<Vec<CanvasSceneSummary>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, story_id, title, thumbnail, section, created_at, updated_at
             FROM canvas_scenes WHERE story_id = ? ORDER BY updated_at DESC",
        )
        .bind(story_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list canvas scenes for story"))?;
        rows.iter().map(row_to_summary).collect()
    }

    /// List a user's scenes across ALL workspaces — Canvas is a global tool, so
    /// you see your scenes regardless of the active workspace.
    pub async fn list_for_user(&self, user_id: &Id) -> Result<Vec<CanvasSceneSummary>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, story_id, title, thumbnail, section, created_at, updated_at
             FROM canvas_scenes WHERE created_by = ? ORDER BY updated_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list canvas scenes for user"))?;
        rows.iter().map(row_to_summary).collect()
    }

    /// Partial update — `None` fields keep their current value via COALESCE.
    pub async fn update(&self, id: &Id, patch: SceneUpdate) -> Result<CanvasScene> {
        let now = fmt(Utc::now());
        let result = sqlx::query(
            "UPDATE canvas_scenes
             SET title = COALESCE(?, title),
                 doc_json = COALESCE(?, doc_json),
                 thumbnail = COALESCE(?, thumbnail),
                 provider = COALESCE(?, provider),
                 section = COALESCE(?, section),
                 story_id = COALESCE(?, story_id),
                 updated_at = ?
             WHERE id = ?",
        )
        .bind(&patch.title)
        .bind(&patch.doc_json)
        .bind(&patch.thumbnail)
        .bind(&patch.provider)
        .bind(&patch.section)
        .bind(&patch.story_id)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update canvas scene"))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound(format!("canvas scene {id}")));
        }
        self.get_required(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM canvas_scenes WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete canvas scene"))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound(format!("canvas scene {id}")));
        }
        Ok(())
    }

    /// Link the managed session backing this scene's Ask-AI (set on first use).
    pub async fn set_session(&self, id: &Id, session_id: &Id) -> Result<()> {
        let now = fmt(Utc::now());
        let result = sqlx::query(
            "UPDATE canvas_scenes SET session_id = ?, updated_at = ? WHERE id = ?",
        )
        .bind(session_id)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set canvas scene session"))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound(format!("canvas scene {id}")));
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
    async fn create_get_list_update_delete_roundtrip() {
        let pool = mem_pool().await;
        let repo = CanvasRepo::new(pool);

        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: Some("s1".into()),
                title: "My Scene".into(),
                doc_json: r#"{"schema":1,"nodes":[],"edges":[],"slides":[]}"#.into(),
                provider: "claude".into(),
                section: Some("Platform/Staging".into()),
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        assert_eq!(scene.title, "My Scene");
        assert!(scene.thumbnail.is_none());
        assert_eq!(scene.provider, "claude");
        assert_eq!(scene.section.as_deref(), Some("Platform/Staging"));

        // summary carries the section so the list can group
        let story_summaries = repo.list_for_story(&"s1".into()).await.unwrap();
        assert_eq!(story_summaries[0].section.as_deref(), Some("Platform/Staging"));

        // partial update of provider; section kept via COALESCE
        let prov = repo
            .update(
                &scene.id,
                SceneUpdate { provider: Some("codex".into()), ..Default::default() },
            )
            .await
            .unwrap();
        assert_eq!(prov.provider, "codex");
        assert_eq!(prov.section.as_deref(), Some("Platform/Staging"));

        // list_for_workspace / list_for_story see it
        let ws_list = repo.list_for_workspace(&"w1".into()).await.unwrap();
        assert_eq!(ws_list.len(), 1);
        let story_list = repo.list_for_story(&"s1".into()).await.unwrap();
        assert_eq!(story_list.len(), 1);

        // partial update: only title; doc_json untouched
        let updated = repo
            .update(
                &scene.id,
                SceneUpdate { title: Some("Renamed".into()), ..Default::default() },
            )
            .await
            .unwrap();
        assert_eq!(updated.title, "Renamed");
        assert_eq!(updated.doc_json, scene.doc_json);

        // update doc + thumbnail
        let updated2 = repo
            .update(
                &scene.id,
                SceneUpdate {
                    title: None,
                    doc_json: Some(r#"{"schema":1,"nodes":[{"id":"n1"}],"edges":[],"slides":[]}"#.into()),
                    thumbnail: Some("data:image/png;base64,AAAA".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated2.title, "Renamed"); // unchanged
        assert!(updated2.doc_json.contains("n1"));
        assert_eq!(updated2.thumbnail.as_deref(), Some("data:image/png;base64,AAAA"));

        // delete then get is None
        repo.delete(&scene.id).await.unwrap();
        assert!(repo.get(&scene.id).await.unwrap().is_none());

        // update / delete on a missing id → NotFound (not panic)
        let missing: Id = "nope".into();
        assert!(matches!(
            repo.update(&missing, SceneUpdate::default()).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(repo.delete(&missing).await, Err(Error::NotFound(_))));
    }
}
