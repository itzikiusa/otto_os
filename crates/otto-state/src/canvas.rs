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
    /// The scene's source format (`mermaid` | `excalidraw` | `d2`), pulled out of
    /// `doc_json` via `json_extract` so list views can show a format chip without
    /// fetching the full document. `None` for docs that predate/omit `format`
    /// (treated as `mermaid` by convention on the UI side).
    pub format: Option<String>,
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
    /// Optimistic-concurrency guard: when set, the UPDATE only applies if the
    /// row's `updated_at` still equals this stamp; otherwise `Error::Conflict`.
    /// The agent-turn commit uses it so a long turn can't silently clobber
    /// edits the user saved while the turn ran (last-write-wins lost update).
    pub expect_updated_at: Option<DateTime<Utc>>,
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
        format: r.get("format"),
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
            "SELECT id, workspace_id, story_id, title, thumbnail, section,
                    json_extract(doc_json, '$.format') AS format, created_at, updated_at
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
            "SELECT id, workspace_id, story_id, title, thumbnail, section,
                    json_extract(doc_json, '$.format') AS format, created_at, updated_at
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
            "SELECT id, workspace_id, story_id, title, thumbnail, section,
                    json_extract(doc_json, '$.format') AS format, created_at, updated_at
             FROM canvas_scenes WHERE created_by = ? ORDER BY updated_at DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list canvas scenes for user"))?;
        rows.iter().map(row_to_summary).collect()
    }

    // -----------------------------------------------------------------------
    // Session ↔ scene references (canvas_scene_refs)
    // -----------------------------------------------------------------------

    /// Reference a scene from a session (idempotent — re-adding an existing ref
    /// is a no-op, not a conflict).
    pub async fn add_ref(
        &self,
        scene_id: &Id,
        session_id: &Id,
        workspace_id: &Id,
        user_id: &Id,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO canvas_scene_refs
             (scene_id, session_id, workspace_id, created_by, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (scene_id, session_id) DO NOTHING",
        )
        .bind(scene_id)
        .bind(session_id)
        .bind(workspace_id)
        .bind(user_id)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add canvas scene ref"))?;
        Ok(())
    }

    /// Remove a scene reference from a session. A missing ref is a silent no-op
    /// (detaching something already detached is not an error).
    pub async fn remove_ref(&self, scene_id: &Id, session_id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM canvas_scene_refs WHERE scene_id = ? AND session_id = ?")
            .bind(scene_id)
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("remove canvas scene ref"))?;
        Ok(())
    }

    /// List the scenes referenced by a session, most-recently-updated first.
    pub async fn list_refs_for_session(&self, session_id: &Id) -> Result<Vec<CanvasSceneSummary>> {
        let rows = sqlx::query(
            "SELECT s.id, s.workspace_id, s.story_id, s.title, s.thumbnail, s.section,
                    json_extract(s.doc_json, '$.format') AS format, s.created_at, s.updated_at
             FROM canvas_scenes s
             JOIN canvas_scene_refs r ON r.scene_id = s.id
             WHERE r.session_id = ?
             ORDER BY s.updated_at DESC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list canvas refs for session"))?;
        rows.iter().map(row_to_summary).collect()
    }

    /// Partial update — `None` fields keep their current value via COALESCE.
    /// With `expect_updated_at` set, the write applies only when the row is
    /// unchanged since that stamp; a concurrent edit yields `Error::Conflict`
    /// instead of a silent last-write-wins clobber.
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
             WHERE id = ? AND (? IS NULL OR updated_at = ?)",
        )
        .bind(&patch.title)
        .bind(&patch.doc_json)
        .bind(&patch.thumbnail)
        .bind(&patch.provider)
        .bind(&patch.section)
        .bind(&patch.story_id)
        .bind(&now)
        .bind(id)
        .bind(patch.expect_updated_at.map(fmt))
        .bind(patch.expect_updated_at.map(fmt))
        .execute(&self.pool)
        .await
        .map_err(dberr("update canvas scene"))?;
        if result.rows_affected() == 0 {
            // Distinguish "gone" from "changed under us".
            return match self.get(id).await? {
                Some(_) => Err(Error::Conflict(format!(
                    "canvas scene {id} changed since the edit began"
                ))),
                None => Err(Error::NotFound(format!("canvas scene {id}"))),
            };
        }
        self.get_required(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        // Explicit child delete first (independent of the foreign_keys pragma,
        // which isn't guaranteed on every pool — see other repos' convention).
        sqlx::query("DELETE FROM canvas_scene_refs WHERE scene_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete canvas scene refs"))?;
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

        // Optimistic guard: a STALE expect_updated_at (from before updated2's
        // write) must Conflict — not silently clobber the newer doc.
        let stale = updated.updated_at;
        let conflicted = repo
            .update(
                &scene.id,
                SceneUpdate {
                    doc_json: Some(r#"{"schema":1,"nodes":[],"edges":[],"slides":[]}"#.into()),
                    expect_updated_at: Some(stale),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(conflicted, Err(Error::Conflict(_))), "{conflicted:?}");
        let still = repo.get(&scene.id).await.unwrap().unwrap();
        assert!(still.doc_json.contains("n1"), "doc untouched after conflict");
        // The FRESH stamp applies cleanly.
        let ok = repo
            .update(
                &scene.id,
                SceneUpdate {
                    doc_json: Some(r#"{"schema":1,"nodes":[],"edges":[],"slides":[]}"#.into()),
                    expect_updated_at: Some(still.updated_at),
                    ..Default::default()
                },
            )
            .await;
        assert!(ok.is_ok(), "{ok:?}");

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

    // -----------------------------------------------------------------------
    // Session ↔ scene refs
    // -----------------------------------------------------------------------

    async fn seed_user(pool: &SqlitePool, user_id: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, 'x', ?, 0, ?)",
        )
        .bind(user_id)
        .bind(user_id)
        .bind(user_id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed user");
    }

    async fn seed_workspace(pool: &SqlitePool, ws_id: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
             VALUES (?, 'ws', '/tmp', '{}', 0, ?)",
        )
        .bind(ws_id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed workspace");
    }

    async fn seed_session(pool: &SqlitePool, ws_id: &str, created_by: &str) -> Id {
        let id = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO sessions
                (id, workspace_id, kind, provider, title, status, cwd, created_by,
                 created_at, last_active_at, meta_json)
             VALUES (?, ?, 'agent', 'shell', 't', 'running', '/tmp', ?, ?, ?, '{}')",
        )
        .bind(&id)
        .bind(ws_id)
        .bind(created_by)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed session");
        id
    }

    #[tokio::test]
    async fn add_ref_is_idempotent_lists_and_removes() {
        let pool = mem_pool().await;
        seed_user(&pool, "u1").await;
        seed_workspace(&pool, "w1").await;
        let sid = seed_session(&pool, "w1", "u1").await;

        let repo = CanvasRepo::new(pool);
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "Referenced Scene".into(),
                doc_json: r#"{"type":"otto-canvas","version":1,"format":"d2","source":""}"#.into(),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();

        // No refs yet.
        assert!(repo.list_refs_for_session(&sid).await.unwrap().is_empty());

        // Add twice — idempotent, not a conflict error.
        repo.add_ref(&scene.id, &sid, &"w1".into(), &"u1".into()).await.unwrap();
        repo.add_ref(&scene.id, &sid, &"w1".into(), &"u1".into()).await.unwrap();

        let refs = repo.list_refs_for_session(&sid).await.unwrap();
        assert_eq!(refs.len(), 1, "idempotent add must not duplicate the ref");
        assert_eq!(refs[0].id, scene.id);
        assert_eq!(refs[0].format.as_deref(), Some("d2"), "format is pulled from doc_json");

        // Remove — list goes back to empty.
        repo.remove_ref(&scene.id, &sid).await.unwrap();
        assert!(repo.list_refs_for_session(&sid).await.unwrap().is_empty());

        // Removing an already-removed ref is a silent no-op.
        repo.remove_ref(&scene.id, &sid).await.unwrap();
    }

    #[tokio::test]
    async fn deleting_a_scene_cascades_its_refs() {
        let pool = mem_pool().await;
        seed_user(&pool, "u1").await;
        seed_workspace(&pool, "w1").await;
        let sid = seed_session(&pool, "w1", "u1").await;

        let repo = CanvasRepo::new(pool.clone());
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "Doomed Scene".into(),
                doc_json: r#"{"schema":1}"#.into(),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        repo.add_ref(&scene.id, &sid, &"w1".into(), &"u1".into()).await.unwrap();
        assert_eq!(repo.list_refs_for_session(&sid).await.unwrap().len(), 1);

        repo.delete(&scene.id).await.unwrap();

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM canvas_scene_refs WHERE scene_id = ?")
                .bind(&scene.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0, "deleting a scene must cascade its refs");
    }
}
