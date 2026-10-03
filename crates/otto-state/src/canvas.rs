//! Canvas Studio scene repository.
//!
//! A scene is one portable JSON document (`doc_json` — nodes/edges/slides/
//! appState; the rich schema lives in the UI `types.ts`). The Rust side treats
//! the document as opaque text and only owns the metadata (title, workspace,
//! optional story link, timestamps) needed for listing and access control.

use crate::DbPool;
use chrono::{DateTime, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;

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
    /// The scene's source format (`mermaid` | `excalidraw` | `d2`) — the
    /// trigger-maintained `format` column (migration 0143) mirrors `doc_json`'s
    /// `format`, so list views show a format chip without parsing documents. `None` for docs that predate/omit `format`
    /// (treated as `mermaid` by convention on the UI side).
    pub format: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// One entry of a scene's version history (migration 0152) — the document as
/// it was just before a change. List rows omit the document itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasSceneVersion {
    pub id: Id,
    pub scene_id: Id,
    /// `agent` (before an Ask AI commit) | `user` (throttled save snapshot) |
    /// `restore` (the state a restore replaced).
    pub origin: String,
    pub created_by: Option<Id>,
    /// The snapshot's source format, when its doc carries one.
    pub format: Option<String>,
    /// Byte length of the snapshotted document.
    pub size: i64,
    pub created_at: DateTime<Utc>,
}

/// User saves snapshot at most once per this many seconds (10 minutes).
pub const USER_SNAPSHOT_EVERY_SECS: i64 = 600;

/// How many versions a scene keeps (oldest pruned first).
pub const SCENE_VERSIONS_KEPT: i64 = 30;

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
    pool: DbPool,
}

impl CanvasRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    /// Create a scene. Inline Excalidraw images in `doc_json` are moved into
    /// the content-addressed `canvas_files` table first (the stored doc keeps
    /// `otto-canvas-file:<sha>` refs; see [`Self::apply_update`]).
    pub async fn create(&self, r: NewScene) -> Result<CanvasScene> {
        let id = new_id();
        let now = fmt(Utc::now());
        let prep = prepare_doc(r.doc_json).await?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("create canvas scene tx"))?;
        insert_files(&mut tx, &prep.files, &now).await?;
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
        .bind(&prep.doc)
        .bind(&r.provider)
        .bind(&r.section)
        .bind(&r.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(dberr("create canvas scene"))?;
        sync_scene_refs(&mut tx, &id, &prep.refs).await?;
        tx.commit()
            .await
            .map_err(dberr("create canvas scene commit"))?;
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
                    format, created_at, updated_at
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
                    format, created_at, updated_at
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
                    format, created_at, updated_at
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
    /// The owning workspace of a scene, without loading its document (SD-22:
    /// access checks on PUT/DELETE used to `SELECT *` the whole doc).
    pub async fn workspace_of(&self, id: &Id) -> Result<Option<Id>> {
        sqlx::query_scalar("SELECT workspace_id FROM canvas_scenes WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("canvas scene workspace"))
    }

    /// One scene's list row (no `doc_json`).
    pub async fn summary(&self, id: &Id) -> Result<CanvasSceneSummary> {
        let row = sqlx::query(
            "SELECT id, workspace_id, story_id, title, thumbnail, section,
                    format, created_at, updated_at
             FROM canvas_scenes WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get canvas scene summary"))?
        .ok_or_else(|| Error::NotFound(format!("canvas scene {id}")))?;
        row_to_summary(&row)
    }

    pub async fn update(&self, id: &Id, patch: SceneUpdate) -> Result<CanvasScene> {
        self.apply_update(id, patch).await?;
        self.get_required(id).await
    }

    /// [`Self::update`] answering with the list row instead of echoing the
    /// whole document back (SD-22: the save round-trip moved the doc 3×).
    pub async fn update_summary(&self, id: &Id, patch: SceneUpdate) -> Result<CanvasSceneSummary> {
        self.apply_update(id, patch).await?;
        self.summary(id).await
    }

    /// The write behind [`Self::update`]. A new document is externalized
    /// first (R2): inline Excalidraw `files[*].dataURL` payloads go to
    /// `canvas_files` once and the stored doc keeps `otto-canvas-file:<sha>`
    /// refs — so a board with a pasted 3 MB screenshot is stored (and, once
    /// the client sends refs back, autosaved) as a few KB. Docs without inline
    /// files are only byte-scanned (no JSON parse). Files, the row and the
    /// scene's ref set (`canvas_scene_files`, the live GC root) change in ONE
    /// transaction, so a concurrent GC can never drop a file mid-save.
    async fn apply_update(&self, id: &Id, mut patch: SceneUpdate) -> Result<()> {
        let now = fmt(Utc::now());
        let prep = match patch.doc_json.take() {
            Some(doc) => Some(prepare_doc(doc).await?),
            None => None,
        };
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("update canvas scene tx"))?;
        if let Some(p) = &prep {
            insert_files(&mut tx, &p.files, &now).await?;
        }
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
        .bind(prep.as_ref().map(|p| p.doc.as_str()))
        .bind(&patch.thumbnail)
        .bind(&patch.provider)
        .bind(&patch.section)
        .bind(&patch.story_id)
        .bind(&now)
        .bind(id)
        .bind(patch.expect_updated_at.map(fmt))
        .bind(patch.expect_updated_at.map(fmt))
        .execute(&mut *tx)
        .await
        .map_err(dberr("update canvas scene"))?;
        if result.rows_affected() == 0 {
            drop(tx);
            // Distinguish "gone" from "changed under us".
            return match self.workspace_of(id).await? {
                Some(_) => Err(Error::Conflict(format!(
                    "canvas scene {id} changed since the edit began"
                ))),
                None => Err(Error::NotFound(format!("canvas scene {id}"))),
            };
        }
        let dropped = match &prep {
            Some(p) => sync_scene_refs(&mut tx, id, &p.refs).await?,
            None => 0,
        };
        tx.commit()
            .await
            .map_err(dberr("update canvas scene commit"))?;
        if dropped > 0 {
            self.gc_files().await?;
        }
        Ok(())
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        // Explicit child delete first (independent of the foreign_keys pragma,
        // which isn't guaranteed on every pool — see other repos' convention).
        sqlx::query("DELETE FROM canvas_scene_refs WHERE scene_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete canvas scene refs"))?;
        let refs = sqlx::query(
            "DELETE FROM canvas_version_files WHERE version_id IN (
                 SELECT id FROM canvas_scene_versions WHERE scene_id = ?)",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("delete canvas version files"))?
        .rows_affected();
        sqlx::query("DELETE FROM canvas_scene_versions WHERE scene_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete canvas scene versions"))?;
        let live = sqlx::query("DELETE FROM canvas_scene_files WHERE scene_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete canvas scene files"))?
            .rows_affected();
        if refs + live > 0 {
            self.gc_files().await?;
        }
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

    /// Snapshot the scene's CURRENT document into its version history (C5).
    /// Skipped — returning `None` — when the newest version already holds this
    /// exact document, or when `throttle_secs` is set and a snapshot of the same
    /// `origin` is younger than it (user saves: at most one per window). Prunes
    /// to the newest [`SCENE_VERSIONS_KEPT`].
    ///
    /// `format` / `size` are recorded once here (migration 0159) so
    /// [`Self::list_versions`] never parses a document. A plain doc is copied
    /// inside SQLite (it never travels through Rust); an Excalidraw doc with
    /// pasted images has its base64 `files` moved into the content-addressed
    /// `canvas_files` table first, so N versions of a board with one 3 MB
    /// screenshot store the screenshot once, not N times.
    pub async fn snapshot(
        &self,
        scene_id: &Id,
        origin: &str,
        created_by: Option<&Id>,
        throttle_secs: Option<i64>,
    ) -> Result<Option<Id>> {
        if let Some(window) = throttle_secs {
            let cutoff = fmt(Utc::now() - chrono::Duration::seconds(window));
            let recent: Option<String> = sqlx::query_scalar(
                "SELECT id FROM canvas_scene_versions
                 WHERE scene_id = ? AND origin = ? AND created_at > ? LIMIT 1",
            )
            .bind(scene_id)
            .bind(origin)
            .bind(&cutoff)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("canvas version throttle"))?;
            if recent.is_some() {
                return Ok(None);
            }
        }
        // Does the doc carry inline Excalidraw files? `instr` is a byte scan —
        // no JSON parse — and `NULL` means the scene is gone.
        let has_files: Option<bool> = sqlx::query_scalar(
            "SELECT instr(doc_json, 'dataURL') > 0 FROM canvas_scenes WHERE id = ?",
        )
        .bind(scene_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("canvas snapshot probe"))?;
        let Some(has_files) = has_files else {
            return Ok(None);
        };
        let vid = new_id();
        let now = fmt(Utc::now());
        let inserted = if has_files {
            self.snapshot_externalized(scene_id, &vid, origin, created_by, &now)
                .await?
        } else {
            sqlx::query(
                "INSERT INTO canvas_scene_versions
                 (id, scene_id, doc_json, origin, created_by, created_at, format, size)
                 SELECT ?, s.id, s.doc_json, ?, ?, ?,
                        CASE WHEN json_valid(s.doc_json) THEN json_extract(s.doc_json, '$.format') END,
                        length(s.doc_json)
                 FROM canvas_scenes s
                 WHERE s.id = ?
                   AND s.doc_json IS NOT (
                       SELECT v.doc_json FROM canvas_scene_versions v
                       WHERE v.scene_id = s.id ORDER BY v.rowid DESC LIMIT 1)",
            )
            .bind(&vid)
            .bind(origin)
            .bind(created_by)
            .bind(&now)
            .bind(scene_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("snapshot canvas scene"))?
            .rows_affected()
                > 0
        };
        if !inserted {
            return Ok(None);
        }
        self.prune_versions(scene_id).await?;
        Ok(Some(vid))
    }

    /// The Excalidraw-with-images snapshot path: externalize `files` off the
    /// runtime, then insert files + version + refs in one transaction.
    async fn snapshot_externalized(
        &self,
        scene_id: &Id,
        vid: &Id,
        origin: &str,
        created_by: Option<&Id>,
        now: &str,
    ) -> Result<bool> {
        let doc: Option<String> =
            sqlx::query_scalar("SELECT doc_json FROM canvas_scenes WHERE id = ?")
                .bind(scene_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(dberr("canvas snapshot doc"))?;
        let Some(doc) = doc else {
            return Ok(false);
        };
        let size = doc.len() as i64;
        // A live doc is normally already externalized (refs only — a byte
        // scan); a legacy doc not saved since 0163 still carries inline files.
        let ext = prepare_doc(doc).await?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("canvas snapshot tx"))?;
        insert_files(&mut tx, &ext.files, now).await?;
        let res = sqlx::query(
            "INSERT INTO canvas_scene_versions
             (id, scene_id, doc_json, origin, created_by, created_at, format, size)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6,
                    COALESCE(?7, CASE WHEN json_valid(?3) THEN json_extract(?3, '$.format') END),
                    ?8
             WHERE ?9 IS NOT (
                 SELECT v.doc_json FROM canvas_scene_versions v
                 WHERE v.scene_id = ?10 ORDER BY v.rowid DESC LIMIT 1)",
        )
        .bind(vid)
        .bind(scene_id)
        .bind(&ext.doc)
        .bind(origin)
        .bind(created_by)
        .bind(now)
        .bind(&ext.format)
        .bind(size)
        .bind(&ext.doc)
        .bind(scene_id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("snapshot canvas scene"))?;
        if res.rows_affected() == 0 {
            // Unchanged since the newest version: drop the transaction (the
            // INSERT OR IGNOREd files, if new, go with it).
            return Ok(false);
        }
        for sha in &ext.refs {
            sqlx::query(
                "INSERT OR IGNORE INTO canvas_version_files (version_id, sha256) VALUES (?, ?)",
            )
            .bind(vid)
            .bind(sha)
            .execute(&mut *tx)
            .await
            .map_err(dberr("insert canvas version file"))?;
        }
        tx.commit().await.map_err(dberr("canvas snapshot commit"))?;
        Ok(true)
    }

    /// Keep the newest [`SCENE_VERSIONS_KEPT`] versions; drop the file refs of
    /// pruned versions and garbage-collect files nothing references any more.
    async fn prune_versions(&self, scene_id: &Id) -> Result<()> {
        let refs = sqlx::query(
            "DELETE FROM canvas_version_files WHERE version_id IN (
                 SELECT id FROM canvas_scene_versions
                 WHERE scene_id = ? AND rowid NOT IN (
                     SELECT rowid FROM canvas_scene_versions WHERE scene_id = ?
                     ORDER BY rowid DESC LIMIT ?))",
        )
        .bind(scene_id)
        .bind(scene_id)
        .bind(SCENE_VERSIONS_KEPT)
        .execute(&self.pool)
        .await
        .map_err(dberr("prune canvas version files"))?
        .rows_affected();
        sqlx::query(
            "DELETE FROM canvas_scene_versions
             WHERE scene_id = ? AND rowid NOT IN (
                 SELECT rowid FROM canvas_scene_versions WHERE scene_id = ?
                 ORDER BY rowid DESC LIMIT ?)",
        )
        .bind(scene_id)
        .bind(scene_id)
        .bind(SCENE_VERSIONS_KEPT)
        .execute(&self.pool)
        .await
        .map_err(dberr("prune canvas scene versions"))?;
        if refs > 0 {
            self.gc_files().await?;
        }
        Ok(())
    }

    /// Delete `canvas_files` rows no version AND no live scene references
    /// (both index-backed).
    async fn gc_files(&self) -> Result<u64> {
        Ok(sqlx::query(
            "DELETE FROM canvas_files
              WHERE NOT EXISTS (
                    SELECT 1 FROM canvas_version_files r WHERE r.sha256 = canvas_files.sha256)
                AND NOT EXISTS (
                    SELECT 1 FROM canvas_scene_files l WHERE l.sha256 = canvas_files.sha256)",
        )
        .execute(&self.pool)
        .await
        .map_err(dberr("gc canvas files"))?
        .rows_affected())
    }

    /// Total bytes held by the content-addressed file store + how many files —
    /// a storage-size gauge for the canvas history.
    pub async fn file_store_stats(&self) -> Result<(i64, i64)> {
        let row =
            sqlx::query("SELECT COUNT(*) AS n, COALESCE(SUM(size), 0) AS bytes FROM canvas_files")
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("canvas file stats"))?;
        Ok((row.get("n"), row.get("bytes")))
    }

    /// A scene's version history, newest first (no documents). Reads the
    /// `format` / `size` columns recorded at snapshot time — no JSON parsing.
    pub async fn list_versions(&self, scene_id: &Id) -> Result<Vec<CanvasSceneVersion>> {
        let rows = sqlx::query(
            "SELECT id, scene_id, origin, created_by, created_at,
                    COALESCE(size, 0) AS size, format
             FROM canvas_scene_versions WHERE scene_id = ?
             ORDER BY rowid DESC",
        )
        .bind(scene_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list canvas scene versions"))?;
        rows.iter()
            .map(|r| {
                Ok(CanvasSceneVersion {
                    id: r.get("id"),
                    scene_id: r.get("scene_id"),
                    origin: r.get("origin"),
                    created_by: r.get("created_by"),
                    format: r.get("format"),
                    size: r.get("size"),
                    created_at: ts(&r.get::<String, _>("created_at"))?,
                })
            })
            .collect()
    }

    /// One version's document (scoped to its scene so a version id can't be
    /// replayed onto another scene), with externalized Excalidraw files
    /// rehydrated back to their inline `dataURL`s.
    pub async fn version_doc(&self, scene_id: &Id, version_id: &Id) -> Result<Option<String>> {
        let doc: Option<String> = sqlx::query_scalar(
            "SELECT doc_json FROM canvas_scene_versions WHERE scene_id = ? AND id = ?",
        )
        .bind(scene_id)
        .bind(version_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get canvas scene version"))?;
        let Some(doc) = doc else {
            return Ok(None);
        };
        if !doc.contains(FILE_REF_PREFIX) {
            return Ok(Some(doc));
        }
        let rows = sqlx::query(
            "SELECT f.sha256, f.data_url FROM canvas_version_files r
             JOIN canvas_files f ON f.sha256 = r.sha256
             WHERE r.version_id = ?",
        )
        .bind(version_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("get canvas version files"))?;
        let files: std::collections::HashMap<String, String> = rows
            .iter()
            .map(|r| (r.get("sha256"), r.get("data_url")))
            .collect();
        let out = tokio::task::spawn_blocking(move || rehydrate_files(&doc, &files))
            .await
            .map_err(|e| Error::Internal(format!("canvas rehydrate: {e}")))?;
        Ok(Some(out))
    }

    /// The scene's document with its externalized files put back inline —
    /// for API callers that want a self-contained doc (export, duplicate,
    /// agents); the Canvas editor asks for refs (`?files=ref`) and fetches
    /// each file once from `GET /canvas/files/{sha}` instead.
    pub async fn rehydrate_live(&self, mut scene: CanvasScene) -> Result<CanvasScene> {
        if !scene.doc_json.contains(FILE_REF_PREFIX) {
            return Ok(scene);
        }
        let rows = sqlx::query(
            "SELECT f.sha256, f.data_url FROM canvas_scene_files r
             JOIN canvas_files f ON f.sha256 = r.sha256
             WHERE r.scene_id = ?",
        )
        .bind(&scene.id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("get canvas scene files"))?;
        let files: std::collections::HashMap<String, String> = rows
            .iter()
            .map(|r| (r.get("sha256"), r.get("data_url")))
            .collect();
        let doc = std::mem::take(&mut scene.doc_json);
        scene.doc_json = tokio::task::spawn_blocking(move || rehydrate_files(&doc, &files))
            .await
            .map_err(|e| Error::Internal(format!("canvas rehydrate: {e}")))?;
        Ok(scene)
    }

    /// One content-addressed file (`mime`, `data_url`) and the workspaces of
    /// every scene that references it, live or in history — the caller must
    /// be able to view one of them (`GET /canvas/files/{sha}`).
    pub async fn file_with_workspaces(
        &self,
        sha: &str,
    ) -> Result<Option<(String, String, Vec<Id>)>> {
        let row = sqlx::query("SELECT mime, data_url FROM canvas_files WHERE sha256 = ?")
            .bind(sha)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get canvas file"))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let ws: Vec<Id> = sqlx::query_scalar(
            "SELECT DISTINCT s.workspace_id FROM canvas_scenes s
              WHERE s.id IN (SELECT scene_id FROM canvas_scene_files WHERE sha256 = ?1)
                 OR s.id IN (SELECT v.scene_id FROM canvas_scene_versions v
                               JOIN canvas_version_files r ON r.version_id = v.id
                              WHERE r.sha256 = ?1)",
        )
        .bind(sha)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("canvas file workspaces"))?;
        Ok(Some((row.get("mime"), row.get("data_url"), ws)))
    }

    /// Link the managed session backing this scene's Ask-AI (set on first use).
    pub async fn set_session(&self, id: &Id, session_id: &Id) -> Result<()> {
        let now = fmt(Utc::now());
        let result =
            sqlx::query("UPDATE canvas_scenes SET session_id = ?, updated_at = ? WHERE id = ?")
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
// Excalidraw file externalization (migration 0159)
// ---------------------------------------------------------------------------

/// Marker a version doc stores in place of an externalized file's `dataURL`.
pub const FILE_REF_PREFIX: &str = "otto-canvas-file:";

struct ExtFile {
    sha256: String,
    mime: String,
    data_url: String,
}

struct Externalized {
    doc: String,
    format: Option<String>,
    files: Vec<ExtFile>,
    /// Every file ref the output doc holds (new and pre-existing), sorted.
    refs: Vec<String>,
}

/// Does `doc` carry an inline Excalidraw file (`"dataURL": "data:…"`, in the
/// outer doc or JSON-escaped inside `source`)? A byte scan — no parse.
fn has_inline_files(doc: &str) -> bool {
    doc.match_indices("dataURL").any(|(i, m)| {
        let rest = doc[i + m.len()..].trim_start_matches(['\\', '"', ':', ' ']);
        rest.starts_with("data:")
    })
}

/// The `otto-canvas-file:<sha256>` refs in `doc`, sorted + deduped. A byte
/// scan — no parse.
fn scan_refs(doc: &str) -> Vec<String> {
    let mut out: Vec<String> = doc
        .match_indices(FILE_REF_PREFIX)
        .filter_map(|(i, m)| doc.get(i + m.len()..i + m.len() + 64))
        .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Externalize a doc's inline files (parse on the blocking pool), or — the
/// common case once a scene has been saved since 0163 — just byte-scan its refs.
async fn prepare_doc(doc: String) -> Result<Externalized> {
    if !has_inline_files(&doc) {
        let refs = scan_refs(&doc);
        return Ok(Externalized {
            doc,
            format: None,
            files: Vec::new(),
            refs,
        });
    }
    tokio::task::spawn_blocking(move || externalize_files(&doc))
        .await
        .map_err(|e| Error::Internal(format!("canvas externalize: {e}")))
}

async fn insert_files(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    files: &[ExtFile],
    now: &str,
) -> Result<()> {
    for f in files {
        sqlx::query(
            "INSERT OR IGNORE INTO canvas_files (sha256, mime, data_url, size, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&f.sha256)
        .bind(&f.mime)
        .bind(&f.data_url)
        .bind(f.data_url.len() as i64)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(dberr("insert canvas file"))?;
    }
    Ok(())
}

/// Make `canvas_scene_files` for `scene_id` equal `refs` (sorted). Reads the
/// tiny current set first, so an autosave that changed no image writes
/// nothing. Returns how many refs were dropped (→ the caller GCs).
async fn sync_scene_refs(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    scene_id: &str,
    refs: &[String],
) -> Result<u64> {
    let mut have: Vec<String> = sqlx::query_scalar(
        "SELECT sha256 FROM canvas_scene_files WHERE scene_id = ? ORDER BY sha256",
    )
    .bind(scene_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(dberr("canvas scene files"))?;
    have.sort();
    if have == refs {
        return Ok(0);
    }
    let want: std::collections::HashSet<&str> = refs.iter().map(String::as_str).collect();
    let mut dropped = 0;
    for sha in have.iter().filter(|h| !want.contains(h.as_str())) {
        dropped += sqlx::query("DELETE FROM canvas_scene_files WHERE scene_id = ? AND sha256 = ?")
            .bind(scene_id)
            .bind(sha)
            .execute(&mut **tx)
            .await
            .map_err(dberr("drop canvas scene file"))?
            .rows_affected();
    }
    for sha in refs {
        sqlx::query("INSERT OR IGNORE INTO canvas_scene_files (scene_id, sha256) VALUES (?, ?)")
            .bind(scene_id)
            .bind(sha)
            .execute(&mut **tx)
            .await
            .map_err(dberr("add canvas scene file"))?;
    }
    Ok(dropped)
}

/// Move an Excalidraw doc's inline `files[*].dataURL` payloads out into
/// content-addressed entries. Any doc that isn't a parseable Excalidraw scene
/// with inline files is returned unchanged (with its `format`, if any).
fn externalize_files(doc: &str) -> Externalized {
    use sha2::{Digest, Sha256};
    let unchanged = |format: Option<String>| Externalized {
        doc: doc.to_string(),
        format,
        files: Vec::new(),
        refs: scan_refs(doc),
    };
    let Ok(mut outer) = serde_json::from_str::<serde_json::Value>(doc) else {
        return unchanged(None);
    };
    let format = outer
        .get("format")
        .and_then(|f| f.as_str())
        .map(str::to_string);
    if format.as_deref() != Some("excalidraw") {
        return unchanged(format);
    }
    let Some(src) = outer.get("source").and_then(|s| s.as_str()) else {
        return unchanged(format);
    };
    let Ok(mut scene) = serde_json::from_str::<serde_json::Value>(src) else {
        return unchanged(format);
    };
    let mut files = Vec::new();
    if let Some(map) = scene.get_mut("files").and_then(|f| f.as_object_mut()) {
        for entry in map.values_mut() {
            let Some(obj) = entry.as_object_mut() else {
                continue;
            };
            let Some(data_url) = obj.get("dataURL").and_then(|d| d.as_str()) else {
                continue;
            };
            if data_url.starts_with(FILE_REF_PREFIX) {
                continue;
            }
            let sha256 = hex::encode(Sha256::digest(data_url.as_bytes()));
            let mime = obj
                .get("mimeType")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            files.push(ExtFile {
                sha256: sha256.clone(),
                mime,
                data_url: data_url.to_string(),
            });
            obj.insert(
                "dataURL".into(),
                serde_json::Value::String(format!("{FILE_REF_PREFIX}{sha256}")),
            );
        }
    }
    if files.is_empty() {
        return unchanged(format);
    }
    outer["source"] = serde_json::Value::String(scene.to_string());
    let doc = outer.to_string();
    let refs = scan_refs(&doc);
    Externalized {
        doc,
        format,
        files,
        refs,
    }
}

/// Inverse of [`externalize_files`]: put each referenced file's `dataURL`
/// back. A ref whose file is missing is left as the marker (never a crash).
fn rehydrate_files(doc: &str, files: &std::collections::HashMap<String, String>) -> String {
    let Ok(mut outer) = serde_json::from_str::<serde_json::Value>(doc) else {
        return doc.to_string();
    };
    let Some(src) = outer.get("source").and_then(|s| s.as_str()) else {
        return doc.to_string();
    };
    let Ok(mut scene) = serde_json::from_str::<serde_json::Value>(src) else {
        return doc.to_string();
    };
    if let Some(map) = scene.get_mut("files").and_then(|f| f.as_object_mut()) {
        for entry in map.values_mut() {
            let Some(obj) = entry.as_object_mut() else {
                continue;
            };
            let sha = obj
                .get("dataURL")
                .and_then(|d| d.as_str())
                .and_then(|d| d.strip_prefix(FILE_REF_PREFIX))
                .map(str::to_string);
            if let Some(data) = sha.and_then(|s| files.get(&s)) {
                obj.insert("dataURL".into(), serde_json::Value::String(data.clone()));
            }
        }
    }
    outer["source"] = serde_json::Value::String(scene.to_string());
    outer.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
    }

    #[tokio::test]
    async fn versions_snapshot_dedupe_throttle_prune_and_cascade() {
        let repo = CanvasRepo::new(mem_pool().await);
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "V".into(),
                doc_json: r#"{"format":"d2","source":"a"}"#.into(),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        let u = Id::from("u1".to_string());
        let first = repo
            .snapshot(&scene.id, "agent", Some(&u), None)
            .await
            .unwrap();
        assert!(first.is_some());
        // Same document again → deduped.
        assert!(repo
            .snapshot(&scene.id, "agent", None, None)
            .await
            .unwrap()
            .is_none());

        let set = |src: &str| SceneUpdate {
            doc_json: Some(format!(r#"{{"format":"d2","source":"{src}"}}"#)),
            ..Default::default()
        };
        repo.update(&scene.id, set("b")).await.unwrap();
        let window = Some(600);
        assert!(repo
            .snapshot(&scene.id, "user", None, window)
            .await
            .unwrap()
            .is_some());
        repo.update(&scene.id, set("c")).await.unwrap();
        // A user snapshot inside the window is throttled.
        assert!(repo
            .snapshot(&scene.id, "user", None, window)
            .await
            .unwrap()
            .is_none());

        let list = repo.list_versions(&scene.id).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].origin, "user");
        assert_eq!(list[0].format.as_deref(), Some("d2"));
        let doc = repo
            .version_doc(&scene.id, &list[1].id)
            .await
            .unwrap()
            .unwrap();
        assert!(doc.contains(r#""source":"a""#));
        // Scoped: a version id never resolves under another scene.
        assert!(repo
            .version_doc(&Id::from("other".to_string()), &list[1].id)
            .await
            .unwrap()
            .is_none());

        // Prune keeps the newest SCENE_VERSIONS_KEPT.
        for i in 0..(SCENE_VERSIONS_KEPT + 5) {
            repo.update(&scene.id, set(&format!("n{i}"))).await.unwrap();
            repo.snapshot(&scene.id, "agent", None, None).await.unwrap();
        }
        let list = repo.list_versions(&scene.id).await.unwrap();
        assert_eq!(list.len() as i64, SCENE_VERSIONS_KEPT);

        repo.delete(&scene.id).await.unwrap();
        assert!(repo.list_versions(&scene.id).await.unwrap().is_empty());
    }

    fn excali_doc(images: &[(&str, &str)], label: &str) -> String {
        let files: serde_json::Map<String, serde_json::Value> = images
            .iter()
            .map(|(id, data)| {
                (
                    id.to_string(),
                    serde_json::json!({"id": id, "mimeType": "image/png", "dataURL": data}),
                )
            })
            .collect();
        let scene = serde_json::json!({
            "type": "excalidraw", "elements": [{"id": label}], "files": files,
        });
        serde_json::json!({
            "type": "otto-canvas", "version": 1, "format": "excalidraw",
            "source": scene.to_string(),
        })
        .to_string()
    }

    #[tokio::test]
    async fn excalidraw_images_are_stored_once_and_rehydrated_on_restore() {
        let repo = CanvasRepo::new(mem_pool().await);
        let img = format!("data:image/png;base64,{}", "A".repeat(200_000));
        let doc0 = excali_doc(&[("f1", &img)], "e0");
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "X".into(),
                doc_json: doc0.clone(),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        // 10 versions of the same board, each with the same 200 KB image.
        for i in 0..10 {
            repo.snapshot(&scene.id, "agent", None, None)
                .await
                .unwrap()
                .unwrap();
            let next = SceneUpdate {
                doc_json: Some(excali_doc(&[("f1", &img)], &format!("e{}", i + 1))),
                ..Default::default()
            };
            repo.update(&scene.id, next).await.unwrap();
        }
        // Unchanged doc → deduped even on the externalized path.
        repo.snapshot(&scene.id, "agent", None, None)
            .await
            .unwrap()
            .unwrap();
        assert!(repo
            .snapshot(&scene.id, "agent", None, None)
            .await
            .unwrap()
            .is_none());

        let (n, bytes) = repo.file_store_stats().await.unwrap();
        assert_eq!(n, 1, "one image stored once");
        assert_eq!(bytes, img.len() as i64);
        let stored: i64 = sqlx::query_scalar(
            "SELECT SUM(length(doc_json)) FROM canvas_scene_versions WHERE scene_id = ?",
        )
        .bind(&scene.id)
        .fetch_one(&repo.pool)
        .await
        .unwrap();
        assert!(
            stored < 20_000,
            "versions hold refs, not base64 ({stored} B)"
        );

        let list = repo.list_versions(&scene.id).await.unwrap();
        assert_eq!(list.len(), 11);
        assert_eq!(list[0].format.as_deref(), Some("excalidraw"));
        // The live doc is stored externalized since 0163, so a snapshot of it
        // is small too.
        assert!(list[0].size < 20_000, "size is the stored (ref) doc length");
        // The oldest version rehydrates to the original document's content.
        let oldest = repo
            .version_doc(&scene.id, &list[10].id)
            .await
            .unwrap()
            .unwrap();
        assert!(oldest.contains(&img));
        assert!(!oldest.contains(FILE_REF_PREFIX));
        let v: serde_json::Value = serde_json::from_str(&oldest).unwrap();
        let inner: serde_json::Value = serde_json::from_str(v["source"].as_str().unwrap()).unwrap();
        assert_eq!(inner["elements"][0]["id"], "e0");

        // Deleting the scene garbage-collects the file.
        repo.delete(&scene.id).await.unwrap();
        assert_eq!(repo.file_store_stats().await.unwrap().0, 0);
    }

    #[test]
    fn inline_and_ref_scans_need_no_parse() {
        let img = "data:image/png;base64,AAAA";
        let doc = excali_doc(&[("f1", img)], "e");
        assert!(has_inline_files(&doc), "escaped inside `source`");
        assert!(has_inline_files(r#"{"dataURL": "data:x"}"#));
        assert!(!has_inline_files(r#"{"text":"dataURL is data: here"}"#));
        let sha = "a".repeat(64);
        let refd = excali_doc(&[("f1", &format!("{FILE_REF_PREFIX}{sha}"))], "e");
        assert!(!has_inline_files(&refd));
        assert_eq!(scan_refs(&refd), vec![sha.clone()]);
        assert!(scan_refs(&format!("{FILE_REF_PREFIX}xyz")).is_empty());
    }

    /// R2: the LIVE doc keeps refs, so an autosave that sends refs back is a
    /// few KB however big the pasted image; every read path still works.
    #[tokio::test]
    async fn live_doc_externalizes_images_and_ref_saves_stay_small() {
        let repo = CanvasRepo::new(mem_pool().await);
        let img = format!("data:image/png;base64,{}", "C".repeat(3 * 1024 * 1024));
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "Live".into(),
                doc_json: excali_doc(&[("f1", &img)], "e0"),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        // Stored with a ref, not the 3 MB payload.
        assert!(scene.doc_json.len() < 10_000, "{}", scene.doc_json.len());
        let shas = scan_refs(&scene.doc_json);
        assert_eq!(shas.len(), 1);
        let sha = shas[0].clone();
        assert_eq!(repo.file_store_stats().await.unwrap().0, 1);
        // The editor's next autosaves send the ref back: small bodies, no new
        // file rows, and the history still rehydrates.
        let ref_url = format!("{FILE_REF_PREFIX}{sha}");
        for i in 1..=5 {
            let body = excali_doc(&[("f1", &ref_url)], &format!("e{i}"));
            assert!(body.len() < 200_000, "autosave body {} B", body.len());
            repo.snapshot(&scene.id, "agent", None, None).await.unwrap();
            repo.update_summary(
                &scene.id,
                SceneUpdate {
                    doc_json: Some(body),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        assert_eq!(repo.file_store_stats().await.unwrap().0, 1);
        let versions = repo.list_versions(&scene.id).await.unwrap();
        assert_eq!(versions[0].format.as_deref(), Some("excalidraw"));
        let old = repo
            .version_doc(&scene.id, &versions[0].id)
            .await
            .unwrap()
            .unwrap();
        assert!(old.contains(&img), "a version rehydrates from its refs");
        // Self-contained read for export / duplicate / agents.
        let live = repo.get(&scene.id).await.unwrap().unwrap();
        assert!(!live.doc_json.contains(&img));
        let full = repo.rehydrate_live(live).await.unwrap();
        assert!(full.doc_json.contains(&img));
        assert!(!full.doc_json.contains(FILE_REF_PREFIX));
        // The file route's access data.
        let (mime, data, ws) = repo.file_with_workspaces(&sha).await.unwrap().unwrap();
        assert_eq!(
            (mime.as_str(), data.len(), ws),
            ("image/png", img.len(), vec!["w1".to_string()])
        );
        // Removing the image from the live doc keeps the file while history
        // cites it; deleting the scene collects it.
        repo.update(
            &scene.id,
            SceneUpdate {
                doc_json: Some(excali_doc(&[], "gone")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(repo.file_store_stats().await.unwrap().0, 1);
        repo.delete(&scene.id).await.unwrap();
        assert_eq!(repo.file_store_stats().await.unwrap().0, 0);
        assert!(repo.file_with_workspaces(&sha).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn live_only_file_is_collected_once_the_doc_drops_it() {
        let repo = CanvasRepo::new(mem_pool().await);
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "Drop".into(),
                doc_json: excali_doc(&[("f", "data:image/png;base64,only")], "a"),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        assert_eq!(repo.file_store_stats().await.unwrap().0, 1);
        repo.update(
            &scene.id,
            SceneUpdate {
                doc_json: Some(excali_doc(&[], "b")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(repo.file_store_stats().await.unwrap().0, 0);
    }

    #[tokio::test]
    async fn pruning_versions_garbage_collects_unreferenced_files() {
        let repo = CanvasRepo::new(mem_pool().await);
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "G".into(),
                doc_json: excali_doc(&[("f", "data:image/png;base64,first")], "a"),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        repo.snapshot(&scene.id, "agent", None, None)
            .await
            .unwrap()
            .unwrap();
        // Every later version uses a different image; the first one ages out.
        for i in 0..SCENE_VERSIONS_KEPT {
            let next = SceneUpdate {
                doc_json: Some(excali_doc(
                    &[("f", &format!("data:image/png;base64,img{i}"))],
                    &format!("b{i}"),
                )),
                ..Default::default()
            };
            repo.update(&scene.id, next).await.unwrap();
            repo.snapshot(&scene.id, "agent", None, None)
                .await
                .unwrap()
                .unwrap();
        }
        let (n, _) = repo.file_store_stats().await.unwrap();
        assert_eq!(
            n, SCENE_VERSIONS_KEPT,
            "the pruned version's file was collected"
        );
    }

    #[tokio::test]
    async fn list_versions_reads_columns_not_documents() {
        let repo = CanvasRepo::new(mem_pool().await);
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "L".into(),
                doc_json: r#"{"format":"d2","source":"abc"}"#.into(),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        repo.snapshot(&scene.id, "agent", None, None)
            .await
            .unwrap()
            .unwrap();
        // Corrupt the stored document: a list that parsed it would now
        // report no format / a different size.
        sqlx::query("UPDATE canvas_scene_versions SET doc_json = 'x' WHERE scene_id = ?")
            .bind(&scene.id)
            .execute(&repo.pool)
            .await
            .unwrap();
        let list = repo.list_versions(&scene.id).await.unwrap();
        assert_eq!(list[0].format.as_deref(), Some("d2"));
        assert_eq!(
            list[0].size,
            r#"{"format":"d2","source":"abc"}"#.len() as i64
        );
    }

    /// Budget bench: a 5 MB Excalidraw scene — save, snapshot and list the
    /// history. `cargo test -p otto-state --lib canvas_large_scene -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn canvas_large_scene_budget() {
        let repo = CanvasRepo::new(mem_pool().await);
        let img = format!("data:image/png;base64,{}", "B".repeat(5 * 1024 * 1024));
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "Big".into(),
                doc_json: excali_doc(&[("f1", &img)], "e"),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        let t = std::time::Instant::now();
        for i in 0..SCENE_VERSIONS_KEPT {
            let next = SceneUpdate {
                doc_json: Some(excali_doc(&[("f1", &img)], &format!("e{i}"))),
                ..Default::default()
            };
            repo.update_summary(&scene.id, next).await.unwrap();
            repo.snapshot(&scene.id, "agent", None, None).await.unwrap();
        }
        let per_save = t.elapsed() / SCENE_VERSIONS_KEPT as u32;
        let t = std::time::Instant::now();
        let list = repo.list_versions(&scene.id).await.unwrap();
        let list_ms = t.elapsed();
        let (_, bytes) = repo.file_store_stats().await.unwrap();
        println!("5 MB scene: save+snapshot {per_save:?}/op, list_versions {list_ms:?}, file store {bytes} B");
        assert_eq!(list.len() as i64, SCENE_VERSIONS_KEPT);
        assert!(
            list_ms < std::time::Duration::from_millis(100),
            "{list_ms:?}"
        );
        assert!(
            per_save < std::time::Duration::from_millis(300),
            "{per_save:?}"
        );
        assert!(bytes < 6 * 1024 * 1024, "image stored once");
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
        assert_eq!(
            story_summaries[0].section.as_deref(),
            Some("Platform/Staging")
        );

        // partial update of provider; section kept via COALESCE
        let prov = repo
            .update(
                &scene.id,
                SceneUpdate {
                    provider: Some("codex".into()),
                    ..Default::default()
                },
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
                SceneUpdate {
                    title: Some("Renamed".into()),
                    ..Default::default()
                },
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
                    doc_json: Some(
                        r#"{"schema":1,"nodes":[{"id":"n1"}],"edges":[],"slides":[]}"#.into(),
                    ),
                    thumbnail: Some("data:image/png;base64,AAAA".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated2.title, "Renamed"); // unchanged
        assert!(updated2.doc_json.contains("n1"));
        assert_eq!(
            updated2.thumbnail.as_deref(),
            Some("data:image/png;base64,AAAA")
        );

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
        assert!(
            matches!(conflicted, Err(Error::Conflict(_))),
            "{conflicted:?}"
        );
        let still = repo.get(&scene.id).await.unwrap().unwrap();
        assert!(
            still.doc_json.contains("n1"),
            "doc untouched after conflict"
        );
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
        assert!(matches!(
            repo.delete(&missing).await,
            Err(Error::NotFound(_))
        ));
    }

    // -----------------------------------------------------------------------
    // Session ↔ scene refs
    // -----------------------------------------------------------------------

    async fn seed_user(pool: &DbPool, user_id: &str) {
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

    async fn seed_workspace(pool: &DbPool, ws_id: &str) {
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

    async fn seed_session(pool: &DbPool, ws_id: &str, created_by: &str) -> Id {
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
        repo.add_ref(&scene.id, &sid, &"w1".into(), &"u1".into())
            .await
            .unwrap();
        repo.add_ref(&scene.id, &sid, &"w1".into(), &"u1".into())
            .await
            .unwrap();

        let refs = repo.list_refs_for_session(&sid).await.unwrap();
        assert_eq!(refs.len(), 1, "idempotent add must not duplicate the ref");
        assert_eq!(refs[0].id, scene.id);
        assert_eq!(
            refs[0].format.as_deref(),
            Some("d2"),
            "format is pulled from doc_json"
        );

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
        repo.add_ref(&scene.id, &sid, &"w1".into(), &"u1".into())
            .await
            .unwrap();
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

    // SD-22: the list's `format` is a trigger-maintained column; PUT/DELETE
    // access checks and the summary answer never load the document.
    #[tokio::test]
    async fn format_column_follows_the_document_and_summary_skips_it() {
        let pool = mem_pool().await;
        let repo = CanvasRepo::new(pool.clone());
        let scene = repo
            .create(NewScene {
                workspace_id: "w1".into(),
                story_id: None,
                title: "D".into(),
                doc_json: r#"{"type":"otto-canvas","format":"d2","source":"a -> b"}"#.into(),
                provider: "claude".into(),
                section: None,
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        let list = repo.list_for_workspace(&"w1".into()).await.unwrap();
        assert_eq!(list[0].format.as_deref(), Some("d2"), "insert trigger");

        let row = repo
            .update_summary(
                &scene.id,
                SceneUpdate {
                    doc_json: Some(
                        r#"{"type":"otto-canvas","format":"excalidraw","source":"{}"}"#.into(),
                    ),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(row.format.as_deref(), Some("excalidraw"), "update trigger");
        assert_eq!(row.id, scene.id);

        // A title-only update keeps the format; malformed JSON clears it
        // instead of failing the write.
        repo.update(
            &scene.id,
            SceneUpdate {
                title: Some("T".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            repo.summary(&scene.id).await.unwrap().format.as_deref(),
            Some("excalidraw")
        );
        repo.update(
            &scene.id,
            SceneUpdate {
                doc_json: Some("not json".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(repo.summary(&scene.id).await.unwrap().format, None);

        assert_eq!(
            repo.workspace_of(&scene.id).await.unwrap().as_deref(),
            Some("w1")
        );
        assert_eq!(repo.workspace_of(&"missing".into()).await.unwrap(), None);
        assert!(matches!(
            repo.update_summary(&"missing".into(), SceneUpdate::default())
                .await,
            Err(Error::NotFound(_))
        ));
    }
}
