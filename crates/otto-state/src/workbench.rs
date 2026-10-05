//! Workbench: per-user scratch files with a FULL, append-only edit history.
//!
//! Users create them with `POST /workspaces/{ws}/workbench/docs` and autosave
//! with `PATCH …/docs/{id}`; every content change lands in
//! `workbench_revisions` (see migration 0165).
//!
//! History rules, enforced here rather than trusted to the caller:
//! - **Coalescing, never loss.** Rapid autosaves fold into ONE `auto` revision
//!   per ~[`COALESCE_SECS`] burst: a save inside the window overwrites the
//!   burst's revision (so it always holds the newest content), a save after it
//!   — or a `checkpoint` (⌘S) — appends a new one. The doc row's content hash
//!   always equals its newest revision's, so the latest content is in history.
//! - **Append-only.** Nothing but [`WorkbenchRepo::purge`] (the explicit
//!   permanent delete of a TRASHED doc) removes a revision. There is no FK
//!   cascade into these tables and the retention pruner does not list them.
//! - **Atomic.** Every mutation runs in one `BEGIN IMMEDIATE` transaction, so
//!   concurrent autosaves from two windows serialize with contiguous seqs and
//!   a crash leaves either the old or the new state, never a half write.
//! - **Content-addressed.** Text lives once in `workbench_blobs` keyed by its
//!   sha256; a blob is dropped only when no revision or doc references it.

use crate::DbPool;
use chrono::{DateTime, Duration, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::convert::{dberr, fmt, ts};

/// Autosaves within this many seconds of a burst's first save fold into it.
pub const COALESCE_SECS: i64 = 60;
/// Largest doc content accepted (UTF-8 bytes).
pub const MAX_CONTENT_BYTES: usize = 5 * 1024 * 1024;
/// Largest asset accepted (bytes).
pub const MAX_ASSET_BYTES: usize = 20 * 1024 * 1024;
/// Line diffs give up on an exact script past this many edits (falls back to
/// "replace the changed middle"), bounding CPU/memory on huge rewrites.
const MAX_DIFF_EDITS: usize = 2_000;
/// Lines per side a diff will consider at all.
const MAX_DIFF_LINES: usize = 20_000;

// ---------------------------------------------------------------------------
// Wire types (mirrored by ui/src/lib/api/types.ts "Workbench")
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkbenchDoc {
    pub id: Id,
    pub workspace_id: Id,
    pub owner_id: Id,
    pub name: String,
    pub language: String,
    pub pinned: bool,
    pub folder: String,
    pub tags: Vec<String>,
    pub size: i64,
    pub rev: i64,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbenchDocFull {
    #[serde(flatten)]
    pub doc: WorkbenchDoc,
    pub content: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NewWorkbenchDoc {
    pub name: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub pinned: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkbenchPatch {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub pinned: Option<bool>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub checkpoint: bool,
    #[serde(default)]
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkbenchRevision {
    pub seq: i64,
    pub kind: String,
    pub content_hash: String,
    pub size: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub saves: i64,
    pub restored_from: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbenchRevisionDetail {
    pub doc_id: Id,
    #[serde(flatten)]
    pub revision: WorkbenchRevision,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkbenchDiffLine {
    pub op: String,
    pub text: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbenchDiff {
    pub doc_id: Id,
    pub from: i64,
    pub to: Option<i64>,
    pub added: usize,
    pub removed: usize,
    pub lines: Vec<WorkbenchDiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkbenchAsset {
    pub id: Id,
    pub mime: String,
    pub size: i64,
    pub sha256: String,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

fn check_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(invalid("name is required"));
    }
    if n.chars().count() > 200 {
        return Err(invalid("name is too long (max 200 characters)"));
    }
    Ok(n.to_string())
}

fn check_language(lang: &str) -> Result<String> {
    let l = lang.trim().to_ascii_lowercase();
    if l.is_empty()
        || l.len() > 32
        || !l
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_+.".contains(c))
    {
        return Err(invalid("language must be 1-32 chars of [a-z0-9-_+.]"));
    }
    Ok(l)
}

fn check_folder(folder: &str) -> Result<String> {
    let f = folder.trim().trim_matches('/');
    if f.len() > 300 || f.split('/').any(|seg| seg == "..") {
        return Err(invalid("invalid folder"));
    }
    Ok(f.to_string())
}

fn check_tags(tags: &[String]) -> Result<String> {
    if tags.len() > 32 || tags.iter().any(|t| t.chars().count() > 64) {
        return Err(invalid("too many or too long tags (max 32 × 64 chars)"));
    }
    let clean: Vec<&str> = tags
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();
    serde_json::to_string(&clean).map_err(|e| Error::Internal(e.to_string()))
}

fn check_content(content: &str) -> Result<()> {
    if content.len() > MAX_CONTENT_BYTES {
        return Err(invalid(format!(
            "content too large ({} bytes, max {MAX_CONTENT_BYTES})",
            content.len()
        )));
    }
    Ok(())
}

fn row_to_doc(r: &sqlx::sqlite::SqliteRow) -> Result<WorkbenchDoc> {
    let tags: Vec<String> =
        serde_json::from_str(&r.get::<String, _>("tags_json")).unwrap_or_default();
    let deleted: Option<String> = r.get("deleted_at");
    Ok(WorkbenchDoc {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        owner_id: r.get("owner_id"),
        name: r.get("name"),
        language: r.get("language"),
        pinned: r.get::<i64, _>("pinned") != 0,
        folder: r.get("folder"),
        tags,
        size: r.get("size"),
        rev: r.get("rev"),
        content_hash: r.get("content_hash"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        deleted_at: deleted.as_deref().map(ts).transpose()?,
    })
}

fn row_to_rev(r: &sqlx::sqlite::SqliteRow) -> Result<WorkbenchRevision> {
    Ok(WorkbenchRevision {
        seq: r.get("seq"),
        kind: r.get("kind"),
        content_hash: r.get("content_hash"),
        size: r.get("size"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
        saves: r.get("saves"),
        restored_from: r.get("restored_from"),
    })
}

type Tx = sqlx::Transaction<'static, sqlx::Sqlite>;

async fn put_blob(tx: &mut Tx, content: &str) -> Result<String> {
    let hash = sha256_hex(content.as_bytes());
    sqlx::query("INSERT OR IGNORE INTO workbench_blobs (hash, content, size) VALUES (?, ?, ?)")
        .bind(&hash)
        .bind(content)
        .bind(content.len() as i64)
        .execute(&mut **tx)
        .await
        .map_err(dberr("store workbench blob"))?;
    Ok(hash)
}

/// Drop `hash` from the blob store iff nothing references it any more.
async fn gc_blob(tx: &mut Tx, hash: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM workbench_blobs WHERE hash = ?1
           AND NOT EXISTS (SELECT 1 FROM workbench_revisions WHERE content_hash = ?1)
           AND NOT EXISTS (SELECT 1 FROM workbench_docs WHERE content_hash = ?1)",
    )
    .bind(hash)
    .execute(&mut **tx)
    .await
    .map_err(dberr("gc workbench blob"))?;
    Ok(())
}

async fn doc_in_tx(tx: &mut Tx, ws: &Id, owner: &Id, id: &Id) -> Result<WorkbenchDoc> {
    let row = sqlx::query(
        "SELECT * FROM workbench_docs WHERE id = ? AND workspace_id = ? AND owner_id = ?",
    )
    .bind(id)
    .bind(ws)
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await
    .map_err(dberr("get workbench doc"))?
    .ok_or_else(|| Error::NotFound(format!("workbench doc {id}")))?;
    row_to_doc(&row)
}

/// Record `content` as a revision of `doc_id` (coalescing per the module
/// rules) and point the doc row at it. Returns the resulting newest seq, or
/// `None` when the content is unchanged (nothing recorded).
async fn record_revision(
    tx: &mut Tx,
    doc: &WorkbenchDoc,
    content: &str,
    kind: &str,
    restored_from: Option<i64>,
    now: DateTime<Utc>,
) -> Result<Option<i64>> {
    let hash = sha256_hex(content.as_bytes());
    if hash == doc.content_hash && kind != "restore" {
        // ⌘S on already-autosaved content SEALS the open burst: the latest
        // `auto` revision becomes a `checkpoint` (it already holds exactly
        // this content), so the next autosave starts a fresh revision. On a
        // non-`auto` latest (or a plain autosave) nothing is recorded.
        if kind == "checkpoint" {
            let sealed = sqlx::query(
                "UPDATE workbench_revisions SET kind = 'checkpoint', updated_at = ?
                 WHERE doc_id = ? AND kind = 'auto'
                   AND seq = (SELECT MAX(seq) FROM workbench_revisions WHERE doc_id = ?)",
            )
            .bind(fmt(now))
            .bind(&doc.id)
            .bind(&doc.id)
            .execute(&mut **tx)
            .await
            .map_err(dberr("seal workbench burst"))?;
            if sealed.rows_affected() > 0 {
                return Ok(Some(doc.rev));
            }
        }
        return Ok(None);
    }
    put_blob(tx, content).await?;
    let latest =
        sqlx::query("SELECT * FROM workbench_revisions WHERE doc_id = ? ORDER BY seq DESC LIMIT 1")
            .bind(&doc.id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(dberr("latest workbench revision"))?;
    let latest = latest.as_ref().map(row_to_rev).transpose()?;
    let size = content.len() as i64;
    let now_s = fmt(now);

    let coalesce = matches!(&latest, Some(l)
        if kind == "auto" && l.kind == "auto" && l.created_at > now - Duration::seconds(COALESCE_SECS));
    let seq = if let (true, Some(l)) = (coalesce, &latest) {
        sqlx::query(
            "UPDATE workbench_revisions SET content_hash = ?, size = ?, updated_at = ?, saves = saves + 1
             WHERE doc_id = ? AND seq = ?",
        )
        .bind(&hash)
        .bind(size)
        .bind(&now_s)
        .bind(&doc.id)
        .bind(l.seq)
        .execute(&mut **tx)
        .await
        .map_err(dberr("coalesce workbench revision"))?;
        l.seq
    } else {
        let seq = latest.as_ref().map(|l| l.seq).unwrap_or(0) + 1;
        sqlx::query(
            "INSERT INTO workbench_revisions
               (doc_id, seq, kind, content_hash, size, created_at, updated_at, saves, restored_from)
             VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?)",
        )
        .bind(&doc.id)
        .bind(seq)
        .bind(kind)
        .bind(&hash)
        .bind(size)
        .bind(&now_s)
        .bind(&now_s)
        .bind(restored_from)
        .execute(&mut **tx)
        .await
        .map_err(dberr("append workbench revision"))?;
        seq
    };
    sqlx::query("UPDATE workbench_docs SET content_hash = ?, size = ?, rev = ?, updated_at = ? WHERE id = ?")
        .bind(&hash)
        .bind(size)
        .bind(seq)
        .bind(&now_s)
        .bind(&doc.id)
        .execute(&mut **tx)
        .await
        .map_err(dberr("update workbench doc content"))?;
    if let (true, Some(l)) = (coalesce, &latest) {
        if l.content_hash != hash {
            gc_blob(tx, &l.content_hash).await?;
        }
    }
    Ok(Some(seq))
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct WorkbenchRepo {
    pool: DbPool,
}

impl WorkbenchRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        Self { pool: pool.into() }
    }

    async fn begin(&self) -> Result<Tx> {
        self.pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(dberr("lock workbench"))
    }

    /// The caller's docs in `ws`: live ones (pinned first, newest first) or,
    /// with `trash`, the soft-deleted ones (most recently trashed first).
    pub async fn list(&self, ws: &Id, owner: &Id, trash: bool) -> Result<Vec<WorkbenchDoc>> {
        let sql = if trash {
            "SELECT * FROM workbench_docs WHERE workspace_id = ? AND owner_id = ?
               AND deleted_at IS NOT NULL ORDER BY deleted_at DESC"
        } else {
            "SELECT * FROM workbench_docs WHERE workspace_id = ? AND owner_id = ?
               AND deleted_at IS NULL ORDER BY pinned DESC, updated_at DESC"
        };
        let rows = sqlx::query(sql)
            .bind(ws)
            .bind(owner)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list workbench docs"))?;
        rows.iter().map(row_to_doc).collect()
    }

    pub async fn create(
        &self,
        ws: &Id,
        owner: &Id,
        req: NewWorkbenchDoc,
    ) -> Result<WorkbenchDocFull> {
        self.create_at(ws, owner, req, Utc::now()).await
    }

    pub async fn create_at(
        &self,
        ws: &Id,
        owner: &Id,
        req: NewWorkbenchDoc,
        now: DateTime<Utc>,
    ) -> Result<WorkbenchDocFull> {
        let name = check_name(&req.name)?;
        let language = check_language(req.language.as_deref().unwrap_or("auto"))?;
        let folder = check_folder(req.folder.as_deref().unwrap_or(""))?;
        let tags = check_tags(req.tags.as_deref().unwrap_or(&[]))?;
        let content = req.content.unwrap_or_default();
        check_content(&content)?;
        let id = new_id();
        let now_s = fmt(now);
        let mut tx = self.begin().await?;
        let hash = put_blob(&mut tx, &content).await?;
        sqlx::query(
            "INSERT INTO workbench_docs (id, workspace_id, owner_id, name, language, pinned, folder,
               tags_json, content_hash, size, rev, created_at, updated_at, deleted_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, NULL)",
        )
        .bind(&id)
        .bind(ws)
        .bind(owner)
        .bind(&name)
        .bind(&language)
        .bind(req.pinned.unwrap_or(false) as i64)
        .bind(&folder)
        .bind(&tags)
        .bind(&hash)
        .bind(content.len() as i64)
        .bind(&now_s)
        .bind(&now_s)
        .execute(&mut *tx)
        .await
        .map_err(dberr("create workbench doc"))?;
        sqlx::query(
            "INSERT INTO workbench_revisions
               (doc_id, seq, kind, content_hash, size, created_at, updated_at, saves, restored_from)
             VALUES (?, 1, 'create', ?, ?, ?, ?, 1, NULL)",
        )
        .bind(&id)
        .bind(&hash)
        .bind(content.len() as i64)
        .bind(&now_s)
        .bind(&now_s)
        .execute(&mut *tx)
        .await
        .map_err(dberr("create workbench revision"))?;
        let doc = doc_in_tx(&mut tx, ws, owner, &id).await?;
        tx.commit()
            .await
            .map_err(dberr("commit workbench create"))?;
        Ok(WorkbenchDocFull { doc, content })
    }

    pub async fn get_meta(&self, ws: &Id, owner: &Id, id: &Id) -> Result<WorkbenchDoc> {
        let row = sqlx::query(
            "SELECT * FROM workbench_docs WHERE id = ? AND workspace_id = ? AND owner_id = ?",
        )
        .bind(id)
        .bind(ws)
        .bind(owner)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get workbench doc"))?
        .ok_or_else(|| Error::NotFound(format!("workbench doc {id}")))?;
        row_to_doc(&row)
    }

    pub async fn get(&self, ws: &Id, owner: &Id, id: &Id) -> Result<WorkbenchDocFull> {
        let doc = self.get_meta(ws, owner, id).await?;
        let content = self.blob(&doc.content_hash).await?;
        Ok(WorkbenchDocFull { doc, content })
    }

    async fn blob(&self, hash: &str) -> Result<String> {
        sqlx::query_scalar("SELECT content FROM workbench_blobs WHERE hash = ?")
            .bind(hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("read workbench blob"))?
            .ok_or_else(|| Error::Internal(format!("workbench blob {hash} missing")))
    }

    pub async fn update(
        &self,
        ws: &Id,
        owner: &Id,
        id: &Id,
        patch: WorkbenchPatch,
    ) -> Result<WorkbenchDoc> {
        self.update_at(ws, owner, id, patch, Utc::now()).await
    }

    /// Autosave / rename / pin / retag. `content` records a revision
    /// (coalesced unless `checkpoint`); metadata never does. A trashed doc
    /// rejects edits (restore it first) so the trash view stays truthful.
    pub async fn update_at(
        &self,
        ws: &Id,
        owner: &Id,
        id: &Id,
        patch: WorkbenchPatch,
        now: DateTime<Utc>,
    ) -> Result<WorkbenchDoc> {
        let name = patch.name.as_deref().map(check_name).transpose()?;
        let language = patch.language.as_deref().map(check_language).transpose()?;
        let folder = patch.folder.as_deref().map(check_folder).transpose()?;
        let tags = patch.tags.as_deref().map(check_tags).transpose()?;
        if let Some(c) = &patch.content {
            check_content(c)?;
        }
        let mut tx = self.begin().await?;
        let doc = doc_in_tx(&mut tx, ws, owner, id).await?;
        if doc.deleted_at.is_some() {
            return Err(Error::Conflict(
                "doc is in the trash — restore it first".into(),
            ));
        }
        let meta_changed = name.is_some()
            || language.is_some()
            || folder.is_some()
            || tags.is_some()
            || patch.pinned.is_some();
        if meta_changed {
            sqlx::query(
                "UPDATE workbench_docs SET
                   name = COALESCE(?, name), language = COALESCE(?, language),
                   folder = COALESCE(?, folder), tags_json = COALESCE(?, tags_json),
                   pinned = COALESCE(?, pinned), updated_at = ?
                 WHERE id = ?",
            )
            .bind(name)
            .bind(language)
            .bind(folder)
            .bind(tags)
            .bind(patch.pinned.map(|p| p as i64))
            .bind(fmt(now))
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("update workbench doc"))?;
        }
        if let Some(content) = &patch.content {
            let kind = if patch.checkpoint {
                "checkpoint"
            } else {
                "auto"
            };
            record_revision(&mut tx, &doc, content, kind, None, now).await?;
        }
        let out = doc_in_tx(&mut tx, ws, owner, id).await?;
        tx.commit()
            .await
            .map_err(dberr("commit workbench update"))?;
        Ok(out)
    }

    /// Soft delete: move to the trash. History is untouched.
    pub async fn trash(&self, ws: &Id, owner: &Id, id: &Id) -> Result<WorkbenchDoc> {
        self.set_deleted(ws, owner, id, Some(fmt(Utc::now()))).await
    }

    pub async fn restore(&self, ws: &Id, owner: &Id, id: &Id) -> Result<WorkbenchDoc> {
        self.set_deleted(ws, owner, id, None).await
    }

    async fn set_deleted(
        &self,
        ws: &Id,
        owner: &Id,
        id: &Id,
        at: Option<String>,
    ) -> Result<WorkbenchDoc> {
        let mut tx = self.begin().await?;
        doc_in_tx(&mut tx, ws, owner, id).await?;
        sqlx::query("UPDATE workbench_docs SET deleted_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("trash workbench doc"))?;
        let out = doc_in_tx(&mut tx, ws, owner, id).await?;
        tx.commit().await.map_err(dberr("commit workbench trash"))?;
        Ok(out)
    }

    /// Permanently delete a TRASHED doc with its whole history. Refused
    /// (`Conflict`) for a live doc — trash it first. The ONLY path that
    /// removes workbench revisions.
    pub async fn purge(&self, ws: &Id, owner: &Id, id: &Id) -> Result<()> {
        let mut tx = self.begin().await?;
        let doc = doc_in_tx(&mut tx, ws, owner, id).await?;
        if doc.deleted_at.is_none() {
            return Err(Error::Conflict(
                "only a trashed doc can be permanently deleted".into(),
            ));
        }
        let hashes: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT content_hash FROM workbench_revisions WHERE doc_id = ?",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await
        .map_err(dberr("purge workbench hashes"))?;
        sqlx::query("DELETE FROM workbench_revisions WHERE doc_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("purge workbench revisions"))?;
        sqlx::query("DELETE FROM workbench_docs WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("purge workbench doc"))?;
        for h in hashes.iter().chain(std::iter::once(&doc.content_hash)) {
            gc_blob(&mut tx, h).await?;
        }
        tx.commit().await.map_err(dberr("commit workbench purge"))?;
        Ok(())
    }

    /// First bounded metadata page, newest first. Content remains available
    /// by explicit revision lookup regardless of this listing window.
    pub async fn list_revisions(
        &self,
        ws: &Id,
        owner: &Id,
        id: &Id,
    ) -> Result<Vec<WorkbenchRevision>> {
        self.list_revisions_page(ws, owner, id, 100, None).await
    }

    /// Keyset metadata page over the existing (doc_id, seq) primary key.
    pub async fn list_revisions_page(
        &self,
        ws: &Id,
        owner: &Id,
        id: &Id,
        limit: i64,
        before_seq: Option<i64>,
    ) -> Result<Vec<WorkbenchRevision>> {
        self.get_meta(ws, owner, id).await?;
        let sql = if before_seq.is_some() {
            "SELECT * FROM workbench_revisions WHERE doc_id = ? AND seq < ? ORDER BY seq DESC LIMIT ?"
        } else {
            "SELECT * FROM workbench_revisions WHERE doc_id = ? ORDER BY seq DESC LIMIT ?"
        };
        let mut query = sqlx::query(sql).bind(id);
        if let Some(before) = before_seq {
            query = query.bind(before);
        }
        let rows = query
            .bind(limit.clamp(1, 200))
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list workbench revisions"))?;
        rows.iter().map(row_to_rev).collect()
    }

    pub async fn get_revision(
        &self,
        ws: &Id,
        owner: &Id,
        id: &Id,
        seq: i64,
    ) -> Result<WorkbenchRevisionDetail> {
        self.get_meta(ws, owner, id).await?;
        let row = sqlx::query("SELECT * FROM workbench_revisions WHERE doc_id = ? AND seq = ?")
            .bind(id)
            .bind(seq)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("get workbench revision"))?
            .ok_or_else(|| Error::NotFound(format!("revision {seq}")))?;
        let revision = row_to_rev(&row)?;
        let content = self.blob(&revision.content_hash).await?;
        Ok(WorkbenchRevisionDetail {
            doc_id: id.clone(),
            revision,
            content,
        })
    }

    /// Make revision `seq`'s content current again — appended as a new
    /// `restore` revision; nothing is overwritten.
    pub async fn restore_revision(
        &self,
        ws: &Id,
        owner: &Id,
        id: &Id,
        seq: i64,
    ) -> Result<WorkbenchDocFull> {
        let old = self.get_revision(ws, owner, id, seq).await?;
        let mut tx = self.begin().await?;
        let doc = doc_in_tx(&mut tx, ws, owner, id).await?;
        if doc.deleted_at.is_some() {
            return Err(Error::Conflict(
                "doc is in the trash — restore it first".into(),
            ));
        }
        record_revision(
            &mut tx,
            &doc,
            &old.content,
            "restore",
            Some(seq),
            Utc::now(),
        )
        .await?;
        let doc = doc_in_tx(&mut tx, ws, owner, id).await?;
        tx.commit()
            .await
            .map_err(dberr("commit workbench restore"))?;
        Ok(WorkbenchDocFull {
            doc,
            content: old.content,
        })
    }

    /// Line diff `from` → `to` (`None` = current content).
    pub async fn diff(
        &self,
        ws: &Id,
        owner: &Id,
        id: &Id,
        from: i64,
        to: Option<i64>,
    ) -> Result<WorkbenchDiff> {
        let a = self.get_revision(ws, owner, id, from).await?.content;
        let b = match to {
            Some(seq) => self.get_revision(ws, owner, id, seq).await?.content,
            None => self.get(ws, owner, id).await?.content,
        };
        let lines = tokio::task::spawn_blocking(move || line_diff(&a, &b))
            .await
            .map_err(|e| Error::Internal(format!("diff task: {e}")))?;
        let added = lines.iter().filter(|l| l.op == "add").count();
        let removed = lines.iter().filter(|l| l.op == "del").count();
        Ok(WorkbenchDiff {
            doc_id: id.clone(),
            from,
            to,
            added,
            removed,
            lines,
        })
    }

    pub async fn put_asset(
        &self,
        ws: &Id,
        owner: &Id,
        mime: &str,
        data: &[u8],
    ) -> Result<WorkbenchAsset> {
        if data.is_empty() || data.len() > MAX_ASSET_BYTES {
            return Err(invalid(format!(
                "asset must be 1..={MAX_ASSET_BYTES} bytes"
            )));
        }
        let sha = sha256_hex(data);
        // Same bytes from the same owner → the existing asset (idempotent paste).
        if let Some(r) = sqlx::query(
            "SELECT id, mime, size, sha256, created_at FROM workbench_assets
             WHERE workspace_id = ? AND owner_id = ? AND sha256 = ? LIMIT 1",
        )
        .bind(ws)
        .bind(owner)
        .bind(&sha)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("find workbench asset"))?
        {
            return row_to_asset(&r);
        }
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO workbench_assets (id, workspace_id, owner_id, mime, size, sha256, data, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(ws)
        .bind(owner)
        .bind(mime)
        .bind(data.len() as i64)
        .bind(&sha)
        .bind(data)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("store workbench asset"))?;
        Ok(WorkbenchAsset {
            id,
            mime: mime.to_string(),
            size: data.len() as i64,
            sha256: sha,
            created_at: ts(&now)?,
        })
    }

    pub async fn get_asset(
        &self,
        ws: &Id,
        owner: &Id,
        id: &Id,
    ) -> Result<(WorkbenchAsset, Vec<u8>)> {
        let r = sqlx::query(
            "SELECT * FROM workbench_assets WHERE id = ? AND workspace_id = ? AND owner_id = ?",
        )
        .bind(id)
        .bind(ws)
        .bind(owner)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("get workbench asset"))?
        .ok_or_else(|| Error::NotFound(format!("workbench asset {id}")))?;
        Ok((row_to_asset(&r)?, r.get("data")))
    }

    /// Find a live doc by exact name (MCP `workbench_write` by name).
    pub async fn find_by_name(
        &self,
        ws: &Id,
        owner: &Id,
        name: &str,
    ) -> Result<Option<WorkbenchDoc>> {
        let row = sqlx::query(
            "SELECT * FROM workbench_docs WHERE workspace_id = ? AND owner_id = ? AND name = ?
               AND deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(ws)
        .bind(owner)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("find workbench doc"))?;
        row.as_ref().map(row_to_doc).transpose()
    }
}

fn row_to_asset(r: &sqlx::sqlite::SqliteRow) -> Result<WorkbenchAsset> {
    Ok(WorkbenchAsset {
        id: r.get("id"),
        mime: r.get("mime"),
        size: r.get("size"),
        sha256: r.get("sha256"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

// ---------------------------------------------------------------------------
// Line diff (Myers O(ND), bounded)
// ---------------------------------------------------------------------------

fn dl(op: &str, text: &str, old_line: Option<usize>, new_line: Option<usize>) -> WorkbenchDiffLine {
    WorkbenchDiffLine {
        op: op.to_string(),
        text: text.to_string(),
        old_line,
        new_line,
    }
}

/// Line diff of `a` → `b`. Exact (shortest edit script) up to
/// [`MAX_DIFF_EDITS`] edits / [`MAX_DIFF_LINES`] lines; beyond that the
/// changed middle (after the common prefix/suffix) is reported as one
/// delete-block + one add-block — still correct, just not minimal.
pub fn line_diff(a: &str, b: &str) -> Vec<WorkbenchDiffLine> {
    let a: Vec<&str> = a.lines().collect();
    let b: Vec<&str> = b.lines().collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let max_suf = a.len().min(b.len()) - pre;
    let suf = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(max_suf)
        .take_while(|(x, y)| x == y)
        .count();
    let am = &a[pre..a.len() - suf];
    let bm = &b[pre..b.len() - suf];

    let mut out = Vec::with_capacity(a.len().max(b.len()));
    for (i, l) in a[..pre].iter().enumerate() {
        out.push(dl("eq", l, Some(i + 1), Some(i + 1)));
    }
    let mid = if am.len() <= MAX_DIFF_LINES && bm.len() <= MAX_DIFF_LINES {
        myers(am, bm)
    } else {
        None
    };
    // ops over the middle: (op, ai, bi) with 0-based middle indexes.
    let ops = mid.unwrap_or_else(|| {
        let mut v: Vec<(u8, usize, usize)> = (0..am.len()).map(|i| (b'd', i, 0)).collect();
        v.extend((0..bm.len()).map(|j| (b'a', 0, j)));
        v
    });
    for (op, i, j) in ops {
        match op {
            b'e' => out.push(dl("eq", am[i], Some(pre + i + 1), Some(pre + j + 1))),
            b'd' => out.push(dl("del", am[i], Some(pre + i + 1), None)),
            _ => out.push(dl("add", bm[j], None, Some(pre + j + 1))),
        }
    }
    for k in 0..suf {
        let ai = a.len() - suf + k;
        let bi = b.len() - suf + k;
        out.push(dl("eq", a[ai], Some(ai + 1), Some(bi + 1)));
    }
    out
}

/// Myers shortest edit script; `None` when it needs more than
/// [`MAX_DIFF_EDITS`] edits. Ops are `(b'e'|b'd'|b'a', a_idx, b_idx)`.
fn myers(a: &[&str], b: &[&str]) -> Option<Vec<(u8, usize, usize)>> {
    let n = a.len() as isize;
    let m = b.len() as isize;
    // trace[d][k + d] = furthest x on diagonal k after d edits.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = None;
    let max_d = (n + m).min(MAX_DIFF_EDITS as isize);
    for d in 0..=max_d {
        let mut v = vec![0isize; (2 * d + 1) as usize];
        let mut k = -d;
        while k <= d {
            let mut x = if d == 0 {
                0
            } else {
                let prev = &trace[(d - 1) as usize];
                let at = |kk: isize| prev[(kk + d - 1) as usize];
                if k == -d || (k != d && at(k - 1) < at(k + 1)) {
                    at(k + 1)
                } else {
                    at(k - 1) + 1
                }
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[(k + d) as usize] = x;
            if x >= n && y >= m {
                found = Some(d);
            }
            k += 2;
        }
        trace.push(v);
        if found.is_some() {
            break;
        }
    }
    let dmax = found?;
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..=dmax).rev() {
        let prev = &trace[(d - 1) as usize];
        let at = |kk: isize| prev[(kk + d - 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = at(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            ops.push((b'e', x as usize, y as usize));
        }
        if x == prev_x {
            y -= 1;
            ops.push((b'a', x as usize, y as usize));
        } else {
            x -= 1;
            ops.push((b'd', x as usize, y as usize));
        }
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        ops.push((b'e', x as usize, y as usize));
    }
    ops.reverse();
    Some(ops)
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
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
    }

    fn ids() -> (Id, Id) {
        ("ws1".into(), "u1".into())
    }

    fn content(c: &str) -> WorkbenchPatch {
        WorkbenchPatch {
            content: Some(c.into()),
            ..Default::default()
        }
    }

    async fn new_doc(repo: &WorkbenchRepo, now: DateTime<Utc>) -> WorkbenchDocFull {
        let (ws, u) = ids();
        repo.create_at(
            &ws,
            &u,
            NewWorkbenchDoc {
                name: "brands.sql".into(),
                content: Some("select 1".into()),
                ..Default::default()
            },
            now,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn autosaves_coalesce_within_a_burst_and_keep_latest() {
        let repo = WorkbenchRepo::new(mem_pool().await);
        let (ws, u) = ids();
        let t0 = Utc::now() - Duration::minutes(10);
        let d = new_doc(&repo, t0).await;
        // rev 1 = create; the first autosave starts burst rev 2.
        for (i, s) in [1, 10, 30, 59].iter().enumerate() {
            let doc = repo
                .update_at(
                    &ws,
                    &u,
                    &d.doc.id,
                    content(&format!("v{i}")),
                    t0 + Duration::seconds(*s),
                )
                .await
                .unwrap();
            assert_eq!(doc.rev, 2, "save {i} folds into the burst");
        }
        let revs = repo.list_revisions(&ws, &u, &d.doc.id).await.unwrap();
        assert_eq!(revs.len(), 2);
        assert_eq!(revs[0].saves, 4);
        assert_eq!(
            repo.get_revision(&ws, &u, &d.doc.id, 2)
                .await
                .unwrap()
                .content,
            "v3"
        );
        // Past the window → a new revision.
        let doc = repo
            .update_at(
                &ws,
                &u,
                &d.doc.id,
                content("v4"),
                t0 + Duration::seconds(70),
            )
            .await
            .unwrap();
        assert_eq!(doc.rev, 3);
        // Latest content always equals the doc's current content.
        let full = repo.get(&ws, &u, &d.doc.id).await.unwrap();
        assert_eq!(full.content, "v4");
        let latest = repo.get_revision(&ws, &u, &d.doc.id, 3).await.unwrap();
        assert_eq!(latest.content, full.content);
        // The overwritten intermediate blobs were collected; live ones remain.
        let blobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workbench_blobs")
            .fetch_one(&repo.pool)
            .await
            .unwrap();
        assert_eq!(blobs, 3, "select 1, v3, v4");
    }

    #[tokio::test]
    async fn checkpoint_forces_a_new_revision_and_noop_saves_record_nothing() {
        let repo = WorkbenchRepo::new(mem_pool().await);
        let (ws, u) = ids();
        let t0 = Utc::now();
        let d = new_doc(&repo, t0).await;
        repo.update_at(&ws, &u, &d.doc.id, content("a"), t0)
            .await
            .unwrap();
        let mut p = content("b");
        p.checkpoint = true;
        let doc = repo.update_at(&ws, &u, &d.doc.id, p, t0).await.unwrap();
        assert_eq!(doc.rev, 3);
        // An autosave right after a checkpoint does not fold into it.
        let doc = repo
            .update_at(&ws, &u, &d.doc.id, content("c"), t0)
            .await
            .unwrap();
        assert_eq!(doc.rev, 4);
        // Same content again → nothing recorded.
        let doc = repo
            .update_at(&ws, &u, &d.doc.id, content("c"), t0)
            .await
            .unwrap();
        assert_eq!(doc.rev, 4);
        // Metadata alone never creates a revision.
        let doc = repo
            .update_at(
                &ws,
                &u,
                &d.doc.id,
                WorkbenchPatch {
                    name: Some("renamed.sql".into()),
                    pinned: Some(true),
                    tags: Some(vec!["db".into()]),
                    ..Default::default()
                },
                t0,
            )
            .await
            .unwrap();
        assert_eq!(
            (doc.rev, doc.name.as_str(), doc.pinned),
            (4, "renamed.sql", true)
        );
        assert_eq!(doc.tags, vec!["db".to_string()]);
    }

    #[tokio::test]
    async fn identical_checkpoint_seals_the_autosave_burst() {
        let repo = WorkbenchRepo::new(mem_pool().await);
        let (ws, u) = ids();
        let t0 = Utc::now();
        let d = new_doc(&repo, t0).await;
        let doc = repo
            .update_at(&ws, &u, &d.doc.id, content("draft"), t0)
            .await
            .unwrap();
        assert_eq!(doc.rev, 2);
        // ⌘S with the already-autosaved buffer: no new revision, burst sealed.
        let mut p = content("draft");
        p.checkpoint = true;
        let doc = repo
            .update_at(&ws, &u, &d.doc.id, p.clone(), t0 + Duration::seconds(5))
            .await
            .unwrap();
        assert_eq!(doc.rev, 2);
        let revs = repo.list_revisions(&ws, &u, &d.doc.id).await.unwrap();
        assert_eq!(revs[0].kind, "checkpoint");
        // A second identical ⌘S is a no-op.
        let doc = repo
            .update_at(&ws, &u, &d.doc.id, p, t0 + Duration::seconds(6))
            .await
            .unwrap();
        assert_eq!(doc.rev, 2);
        // The next autosave (still inside 60 s) starts a fresh revision.
        let doc = repo
            .update_at(
                &ws,
                &u,
                &d.doc.id,
                content("draft 2"),
                t0 + Duration::seconds(10),
            )
            .await
            .unwrap();
        assert_eq!(doc.rev, 3);
        assert_eq!(
            repo.get_revision(&ws, &u, &d.doc.id, 2)
                .await
                .unwrap()
                .content,
            "draft"
        );
    }

    #[tokio::test]
    async fn trash_restore_and_permanent_delete() {
        let repo = WorkbenchRepo::new(mem_pool().await);
        let (ws, u) = ids();
        let d = new_doc(&repo, Utc::now()).await;
        repo.update(&ws, &u, &d.doc.id, content("x")).await.unwrap();
        // Purge refused while live.
        assert!(matches!(
            repo.purge(&ws, &u, &d.doc.id).await,
            Err(Error::Conflict(_))
        ));
        let t = repo.trash(&ws, &u, &d.doc.id).await.unwrap();
        assert!(t.deleted_at.is_some());
        assert!(repo.list(&ws, &u, false).await.unwrap().is_empty());
        assert_eq!(repo.list(&ws, &u, true).await.unwrap().len(), 1);
        // History survives the trash; edits are refused until restored.
        assert_eq!(
            repo.list_revisions(&ws, &u, &d.doc.id).await.unwrap().len(),
            2
        );
        assert!(repo.update(&ws, &u, &d.doc.id, content("y")).await.is_err());
        repo.restore(&ws, &u, &d.doc.id).await.unwrap();
        assert_eq!(repo.list(&ws, &u, false).await.unwrap().len(), 1);
        // Trash again, then purge removes doc + revisions + blobs.
        repo.trash(&ws, &u, &d.doc.id).await.unwrap();
        repo.purge(&ws, &u, &d.doc.id).await.unwrap();
        assert!(matches!(
            repo.get(&ws, &u, &d.doc.id).await,
            Err(Error::NotFound(_))
        ));
        for t in ["workbench_revisions", "workbench_blobs", "workbench_docs"] {
            let n: i64 =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {t}")))
                    .fetch_one(&repo.pool)
                    .await
                    .unwrap();
            assert_eq!(n, 0, "{t} emptied by purge");
        }
    }

    #[tokio::test]
    async fn docs_are_scoped_per_owner_and_workspace() {
        let repo = WorkbenchRepo::new(mem_pool().await);
        let d = new_doc(&repo, Utc::now()).await;
        let other: Id = "u2".into();
        assert!(repo.get(&"ws1".into(), &other, &d.doc.id).await.is_err());
        assert!(repo
            .get(&"ws2".into(), &"u1".into(), &d.doc.id)
            .await
            .is_err());
        assert!(repo
            .list(&"ws1".into(), &other, false)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn restore_revision_appends_and_shares_the_blob() {
        let repo = WorkbenchRepo::new(mem_pool().await);
        let (ws, u) = ids();
        let t0 = Utc::now() - Duration::minutes(5);
        let d = new_doc(&repo, t0).await;
        repo.update_at(
            &ws,
            &u,
            &d.doc.id,
            content("second"),
            t0 + Duration::seconds(1),
        )
        .await
        .unwrap();
        let r = repo.restore_revision(&ws, &u, &d.doc.id, 1).await.unwrap();
        assert_eq!(r.content, "select 1");
        assert_eq!(r.doc.rev, 3);
        let revs = repo.list_revisions(&ws, &u, &d.doc.id).await.unwrap();
        assert_eq!(revs[0].kind, "restore");
        assert_eq!(revs[0].restored_from, Some(1));
        assert_eq!(revs[0].content_hash, revs[2].content_hash);
        // Diff old → current is empty of changes; 2 → current shows the swap.
        let df = repo.diff(&ws, &u, &d.doc.id, 2, None).await.unwrap();
        assert_eq!((df.added, df.removed), (1, 1));
    }

    #[tokio::test]
    async fn concurrent_autosaves_serialize_without_loss() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::db::open(&dir.path().join("wb.db")).await.unwrap();
        let repo = WorkbenchRepo::new(pool);
        let (ws, u) = ids();
        let d = new_doc(&repo, Utc::now()).await;
        let mut tasks = Vec::new();
        for i in 0..16 {
            let repo = repo.clone();
            let (ws, u, id) = (ws.clone(), u.clone(), d.doc.id.clone());
            tasks.push(tokio::spawn(async move {
                let mut p = content(&format!("write {i}"));
                p.checkpoint = i % 2 == 0;
                repo.update(&ws, &u, &id, p).await.unwrap()
            }));
        }
        let mut writes = Vec::new();
        for t in tasks {
            writes.push(t.await.unwrap());
        }
        let revs = repo.list_revisions(&ws, &u, &d.doc.id).await.unwrap();
        // Contiguous seqs, newest first, ending at 1.
        for (i, r) in revs.iter().enumerate() {
            assert_eq!(r.seq as usize, revs.len() - i);
        }
        let full = repo.get(&ws, &u, &d.doc.id).await.unwrap();
        assert!(full.content.starts_with("write "));
        assert_eq!(full.doc.rev, revs[0].seq);
        assert_eq!(full.doc.content_hash, revs[0].content_hash);
        // Every checkpoint survived as its own revision.
        assert!(revs.iter().filter(|r| r.kind == "checkpoint").count() >= 8);
        assert_eq!(writes.len(), 16);
    }

    #[tokio::test]
    async fn retention_never_touches_workbench_history() {
        let pool = mem_pool().await;
        let repo = WorkbenchRepo::new(pool.clone());
        let (ws, u) = ids();
        let ancient = Utc::now() - Duration::days(3650);
        let d = new_doc(&repo, ancient).await;
        repo.update_at(
            &ws,
            &u,
            &d.doc.id,
            content("old"),
            ancient + Duration::minutes(5),
        )
        .await
        .unwrap();
        // The tightest policy the pruner accepts.
        let policy: crate::RetentionPolicy = serde_json::from_value(serde_json::json!({
            "enabled": true
        }))
        .unwrap();
        crate::RetentionRepo::new(pool.clone())
            .prune(&policy)
            .await
            .unwrap();
        assert_eq!(
            repo.list_revisions(&ws, &u, &d.doc.id).await.unwrap().len(),
            2
        );
        // And the pruner's source names no workbench table at all.
        for src in [
            include_str!("retention.rs"),
            include_str!("retention/runs.rs"),
        ] {
            assert!(
                !src.contains("workbench"),
                "retention must never list workbench tables"
            );
        }
    }

    #[test]
    fn line_diff_is_minimal_and_numbered() {
        let d = line_diff("a\nb\nc\nd", "a\nx\nc\nd\ne");
        let ops: Vec<(&str, &str)> = d.iter().map(|l| (l.op.as_str(), l.text.as_str())).collect();
        assert_eq!(
            ops,
            vec![
                ("eq", "a"),
                ("del", "b"),
                ("add", "x"),
                ("eq", "c"),
                ("eq", "d"),
                ("add", "e")
            ]
        );
        assert_eq!(d[2].new_line, Some(2));
        assert_eq!(d[1].old_line, Some(2));
        assert_eq!(d[5].new_line, Some(5));
        assert!(line_diff("same\n", "same\n").iter().all(|l| l.op == "eq"));
        let all_new = line_diff("", "x\ny");
        assert_eq!(all_new.iter().filter(|l| l.op == "add").count(), 2);
        // Interleaved changes.
        let d = line_diff("1\n2\n3\n4\n5\n6", "1\n3\n4\nX\n6\n7");
        let rebuilt: Vec<&str> = d
            .iter()
            .filter(|l| l.op != "del")
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(rebuilt, vec!["1", "3", "4", "X", "6", "7"]);
        let old: Vec<&str> = d
            .iter()
            .filter(|l| l.op != "add")
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(old, vec!["1", "2", "3", "4", "5", "6"]);
    }

    #[tokio::test]
    async fn assets_dedupe_per_owner() {
        let repo = WorkbenchRepo::new(mem_pool().await);
        let (ws, u) = ids();
        let a = repo
            .put_asset(&ws, &u, "image/png", b"\x89PNG....")
            .await
            .unwrap();
        let b = repo
            .put_asset(&ws, &u, "image/png", b"\x89PNG....")
            .await
            .unwrap();
        assert_eq!(a.id, b.id);
        let (meta, bytes) = repo.get_asset(&ws, &u, &a.id).await.unwrap();
        assert_eq!((meta.mime.as_str(), bytes.len()), ("image/png", 8));
        assert!(repo.get_asset(&ws, &"u2".into(), &a.id).await.is_err());
    }
}
