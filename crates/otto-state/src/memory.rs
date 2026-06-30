//! Memory layer repository — items + chunks, vectors, and the link graph.
//!
//! Persistence DTOs live here (otto-state cannot depend on otto-memory); the
//! `otto-memory` crate re-exports them. Keyword search uses LIKE (always
//! available, instant at single-user scale); vectors are stored as little-endian
//! f32 BLOBs and searched in-process by the caller's `VectorIndex`.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use otto_core::{new_id, Error, Result};

use crate::convert::{dberr, fmt};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Workspace,
    Story,
    Entity,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::Workspace => "workspace",
            Scope::Story => "story",
            Scope::Entity => "entity",
        }
    }
    pub fn parse(s: &str) -> Scope {
        match s {
            "story" => Scope::Story,
            "entity" => Scope::Entity,
            _ => Scope::Workspace,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryRef {
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub workspace_id: String,
    pub collection: String,
    pub record_type: String,
    pub scope: Scope,
    pub story_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub entities: Vec<String>,
    pub tags: Vec<String>,
    pub source_kind: String,
    pub source_ref: Option<String>,
    pub refs: Vec<MemoryRef>,
    pub confidence: f32,
    pub salience: f32,
    pub content_hash: String,
    pub active: bool,
    pub superseded_by: Option<String>,
    pub version: i64,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_accessed_at: Option<String>,
    pub access_count: i64,
    pub expires_at: Option<String>,
    /// `shared` (all workspace members) or `private` (creator-only).
    pub visibility: String,
    // -- lifecycle governance (0056_memory_lifecycle.sql) --
    /// Lifecycle state: `suggested` | `accepted` | `stale` | `contradicted`.
    pub state: String,
    /// JSON provenance record — op + source ids (merge/split/import).
    pub provenance_json: Option<String>,
    /// Unix epoch seconds; set when the memory is soft-deleted. `None` = live.
    pub forgotten_at: Option<i64>,
    /// Random token required to undo a forget. Cleared after undo or permanent
    /// delete.
    pub undo_token: Option<String>,
}

/// A governed-import batch: one AGENTS.md / CLAUDE.md / .cursorrules import.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GovernedImport {
    pub id: String,
    pub workspace_id: String,
    pub kind: String,
    pub label: String,
    pub memory_ids: Vec<String>,
    pub imported_by: String,
    pub imported_at: String,
    pub reverted_at: Option<String>,
}

fn default_collection() -> String {
    "product".into()
}
fn default_record_type() -> String {
    "item".into()
}
fn default_visibility() -> String {
    "shared".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewMemory {
    #[serde(default = "default_collection")]
    pub collection: String,
    #[serde(default = "default_record_type")]
    pub record_type: String,
    pub scope: Scope,
    #[serde(default)]
    pub story_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source_kind: String,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub refs: Vec<MemoryRef>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub salience: Option<f32>,
    /// `shared` (default — all workspace members) or `private` (creator-only).
    #[serde(default = "default_visibility")]
    pub visibility: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemoryPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub entities: Option<Vec<String>>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub salience: Option<f32>,
    #[serde(default)]
    pub active: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryLink {
    pub src_id: String,
    pub dst_id: String,
    pub rel: String,
    pub weight: f32,
    #[serde(default)]
    pub certainty: Option<String>,
}

#[derive(Default)]
pub struct ListFilter {
    pub collection: Option<String>,
    pub kind: Option<String>,
    pub story_id: Option<String>,
    pub tag: Option<String>,
    pub include_inactive: bool,
    pub limit: i64,
    /// When set, restrict to memories visible to this user id (shared, or their
    /// own private). `None` sees everything (internal/system callers).
    pub viewer: Option<String>,
}

#[derive(Default, Clone)]
pub struct SearchFilter {
    pub collection: Option<String>,
    pub story_id: Option<String>,
    pub include_inactive: bool,
    pub limit: i64,
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn vec_str(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}
fn vec_refs(s: &str) -> Vec<MemoryRef> {
    serde_json::from_str(s).unwrap_or_default()
}
fn jstr<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

fn row_to_memory(r: &sqlx::sqlite::SqliteRow) -> Result<Memory> {
    Ok(Memory {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        collection: r.get("collection"),
        record_type: r.get("record_type"),
        scope: Scope::parse(&r.get::<String, _>("scope")),
        story_id: r.get("story_id"),
        kind: r.get("kind"),
        title: r.get("title"),
        body: r.get("body"),
        entities: vec_str(&r.get::<String, _>("entities_json")),
        tags: vec_str(&r.get::<String, _>("tags_json")),
        source_kind: r.get("source_kind"),
        source_ref: r.get("source_ref"),
        refs: vec_refs(&r.get::<String, _>("refs_json")),
        confidence: r.get::<f64, _>("confidence") as f32,
        salience: r.get::<f64, _>("salience") as f32,
        content_hash: r.get("content_hash"),
        active: r.get::<i64, _>("active") != 0,
        superseded_by: r.get("superseded_by"),
        version: r.get("version"),
        created_by: r.get("created_by"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
        last_accessed_at: r.get("last_accessed_at"),
        access_count: r.get("access_count"),
        expires_at: r.get("expires_at"),
        visibility: r.get("visibility"),
        // lifecycle governance — columns added in 0056_memory_lifecycle.sql; may
        // be absent in older rows (SQLite returns the DEFAULT in that case).
        state: r.try_get("state").unwrap_or_else(|_| "accepted".into()),
        provenance_json: r.try_get("provenance_json").unwrap_or(None),
        forgotten_at: r.try_get("forgotten_at").unwrap_or(None),
        undo_token: r.try_get("undo_token").unwrap_or(None),
    })
}

// ---------------------------------------------------------------------------
// Repo
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct MemoriesRepo {
    pool: SqlitePool,
}

impl MemoriesRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Raw pool access — for callers that need to run statements not yet exposed
    /// on the repo (e.g. the governance service updating provenance_json).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Normalized SHA-256 of the body — for exact-duplicate detection.
    pub fn content_hash(body: &str) -> String {
        let norm = body
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let mut h = Sha256::new();
        h.update(norm.as_bytes());
        format!("{:x}", h.finalize())
    }

    pub async fn create(&self, ws: &str, by: &str, nm: NewMemory) -> Result<Memory> {
        let id = new_id();
        let now = fmt(Utc::now());
        let hash = Self::content_hash(&nm.body);
        sqlx::query(
            "INSERT INTO memories (id,workspace_id,collection,record_type,scope,story_id,kind,title,body,\
             entities_json,tags_json,source_kind,source_ref,refs_json,confidence,salience,visibility,content_hash,\
             active,version,created_by,created_at,updated_at,access_count) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,1,1,?,?,?,0)",
        )
        .bind(&id)
        .bind(ws)
        .bind(&nm.collection)
        .bind(&nm.record_type)
        .bind(nm.scope.as_str())
        .bind(&nm.story_id)
        .bind(&nm.kind)
        .bind(&nm.title)
        .bind(&nm.body)
        .bind(jstr(&nm.entities))
        .bind(jstr(&nm.tags))
        .bind(&nm.source_kind)
        .bind(&nm.source_ref)
        .bind(jstr(&nm.refs))
        .bind(nm.confidence.unwrap_or(0.7) as f64)
        .bind(nm.salience.unwrap_or(0.5) as f64)
        .bind(if nm.visibility.is_empty() { "shared" } else { &nm.visibility })
        .bind(&hash)
        .bind(by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.create"))?;
        self.get(ws, &id).await
    }

    pub async fn get(&self, ws: &str, id: &str) -> Result<Memory> {
        let r = sqlx::query("SELECT * FROM memories WHERE id = ? AND workspace_id = ?")
            .bind(id)
            .bind(ws)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("memory"))?;
        row_to_memory(&r)
    }

    pub async fn list(&self, ws: &str, f: &ListFilter) -> Result<Vec<Memory>> {
        let mut sql = String::from("SELECT * FROM memories WHERE workspace_id = ?");
        if !f.include_inactive {
            sql.push_str(" AND active = 1");
        }
        if f.collection.is_some() {
            sql.push_str(" AND collection = ?");
        }
        if f.kind.is_some() {
            sql.push_str(" AND kind = ?");
        }
        if f.story_id.is_some() {
            sql.push_str(" AND story_id = ?");
        }
        if f.tag.is_some() {
            sql.push_str(" AND tags_json LIKE ?");
        }
        if f.viewer.is_some() {
            sql.push_str(" AND (visibility = 'shared' OR created_by = ?)");
        }
        sql.push_str(" ORDER BY updated_at DESC LIMIT ?");
        let mut q = sqlx::query(&sql).bind(ws);
        if let Some(c) = &f.collection {
            q = q.bind(c);
        }
        if let Some(k) = &f.kind {
            q = q.bind(k);
        }
        if let Some(s) = &f.story_id {
            q = q.bind(s);
        }
        if let Some(t) = &f.tag {
            q = q.bind(format!("%\"{t}\"%"));
        }
        if let Some(v) = &f.viewer {
            q = q.bind(v);
        }
        q = q.bind(if f.limit > 0 { f.limit } else { 100 });
        let rows = q.fetch_all(&self.pool).await.map_err(dberr("memory.list"))?;
        rows.iter().map(row_to_memory).collect()
    }

    /// Keyword search: prefilter rows that contain any query term (SQL LIKE), then
    /// rank by matched-term count. Returns (memory, score) best-first.
    pub async fn search_keyword(
        &self,
        ws: &str,
        query: &str,
        f: &SearchFilter,
    ) -> Result<Vec<(Memory, f32)>> {
        let terms: Vec<String> = query
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| s.len() >= 2)
            .map(|s| s.to_string())
            .collect();
        let mut sql = String::from("SELECT * FROM memories WHERE workspace_id = ?");
        if !f.include_inactive {
            sql.push_str(" AND active = 1");
        }
        if f.collection.is_some() {
            sql.push_str(" AND collection = ?");
        }
        if f.story_id.is_some() {
            sql.push_str(" AND story_id = ?");
        }
        if !terms.is_empty() {
            sql.push_str(" AND (");
            for (i, _) in terms.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" OR ");
                }
                sql.push_str("lower(title || ' ' || body) LIKE ?");
            }
            sql.push(')');
        }
        sql.push_str(" LIMIT 2000");
        let mut q = sqlx::query(&sql).bind(ws);
        if let Some(c) = &f.collection {
            q = q.bind(c);
        }
        if let Some(s) = &f.story_id {
            q = q.bind(s);
        }
        for t in &terms {
            q = q.bind(format!("%{t}%"));
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("memory.search_keyword"))?;
        let mut scored: Vec<(Memory, f32)> = rows
            .iter()
            .filter_map(|r| row_to_memory(r).ok())
            .map(|m| {
                let hay = format!("{} {}", m.title, m.body).to_lowercase();
                let hits = terms.iter().filter(|t| hay.contains(t.as_str())).count();
                (m, hits as f32)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let lim = if f.limit > 0 { f.limit as usize } else { 50 };
        scored.truncate(lim);
        Ok(scored)
    }

    pub async fn find_by_hash(
        &self,
        ws: &str,
        collection: &str,
        scope: Scope,
        story: Option<&str>,
        hash: &str,
    ) -> Result<Option<Memory>> {
        let r = sqlx::query(
            "SELECT * FROM memories WHERE workspace_id = ? AND collection = ? AND scope = ? \
             AND IFNULL(story_id,'') = IFNULL(?,'') AND content_hash = ? AND active = 1",
        )
        .bind(ws)
        .bind(collection)
        .bind(scope.as_str())
        .bind(story)
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("memory.find_by_hash"))?;
        match r {
            Some(row) => Ok(Some(row_to_memory(&row)?)),
            None => Ok(None),
        }
    }

    pub async fn update(&self, ws: &str, id: &str, p: MemoryPatch) -> Result<Memory> {
        let cur = self.get(ws, id).await?;
        let title = p.title.unwrap_or(cur.title);
        let body = p.body.unwrap_or(cur.body);
        let tags = p.tags.unwrap_or(cur.tags);
        let entities = p.entities.unwrap_or(cur.entities);
        let confidence = p.confidence.unwrap_or(cur.confidence);
        let salience = p.salience.unwrap_or(cur.salience);
        let active = p.active.unwrap_or(cur.active);
        let hash = Self::content_hash(&body);
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE memories SET title=?, body=?, tags_json=?, entities_json=?, confidence=?, \
             salience=?, active=?, content_hash=?, version=version+1, updated_at=? \
             WHERE id=? AND workspace_id=?",
        )
        .bind(&title)
        .bind(&body)
        .bind(jstr(&tags))
        .bind(jstr(&entities))
        .bind(confidence as f64)
        .bind(salience as f64)
        .bind(active as i64)
        .bind(&hash)
        .bind(&now)
        .bind(id)
        .bind(ws)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.update"))?;
        self.get(ws, id).await
    }

    pub async fn forget(&self, ws: &str, id: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE memories SET active=0, updated_at=? WHERE id=? AND workspace_id=?")
            .bind(&now)
            .bind(id)
            .bind(ws)
            .execute(&self.pool)
            .await
            .map_err(dberr("memory.forget"))?;
        Ok(())
    }

    pub async fn supersede(&self, ws: &str, old: &str, new: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE memories SET active=0, superseded_by=?, updated_at=? WHERE id=? AND workspace_id=?",
        )
        .bind(new)
        .bind(&now)
        .bind(old)
        .bind(ws)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.supersede"))?;
        Ok(())
    }

    pub async fn bump_access(&self, ws: &str, ids: &[String]) -> Result<()> {
        let now = fmt(Utc::now());
        for id in ids {
            let _ = sqlx::query(
                "UPDATE memories SET access_count=access_count+1, last_accessed_at=? \
                 WHERE id=? AND workspace_id=?",
            )
            .bind(&now)
            .bind(id)
            .bind(ws)
            .execute(&self.pool)
            .await;
        }
        Ok(())
    }

    // -- vectors --

    pub async fn put_vector(&self, id: &str, model: &str, dim: usize, v: &[f32]) -> Result<()> {
        let blob: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO memory_vectors(memory_id,model_id,dim,embedding,embedded_at) \
             VALUES(?,?,?,?,?) ON CONFLICT(memory_id) DO UPDATE SET \
             model_id=excluded.model_id, dim=excluded.dim, embedding=excluded.embedding, \
             embedded_at=excluded.embedded_at",
        )
        .bind(id)
        .bind(model)
        .bind(dim as i64)
        .bind(blob)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.put_vector"))?;
        Ok(())
    }

    pub async fn all_vectors(&self, ws: &str, model: &str) -> Result<Vec<(String, Vec<f32>)>> {
        let rows = sqlx::query(
            "SELECT v.memory_id AS mid, v.embedding AS emb FROM memory_vectors v \
             JOIN memories m ON m.id = v.memory_id \
             WHERE m.workspace_id = ? AND m.active = 1 AND v.model_id = ?",
        )
        .bind(ws)
        .bind(model)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("memory.all_vectors"))?;
        Ok(rows
            .iter()
            .map(|r| {
                let b: Vec<u8> = r.get("emb");
                let v = b
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                (r.get::<String, _>("mid"), v)
            })
            .collect())
    }

    // -- links / graph --

    pub async fn link(
        &self,
        src: &str,
        dst: &str,
        rel: &str,
        weight: f32,
        certainty: Option<&str>,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT OR REPLACE INTO memory_links(src_id,dst_id,rel,weight,certainty,created_at) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(src)
        .bind(dst)
        .bind(rel)
        .bind(weight as f64)
        .bind(certainty)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.link"))?;
        Ok(())
    }

    pub async fn links_of(&self, ws: &str, id: &str) -> Result<Vec<MemoryLink>> {
        let rows = sqlx::query(
            "SELECT l.src_id, l.dst_id, l.rel, l.weight, l.certainty FROM memory_links l \
             JOIN memories m ON m.id = l.src_id \
             WHERE (l.src_id = ? OR l.dst_id = ?) AND m.workspace_id = ?",
        )
        .bind(id)
        .bind(id)
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("memory.links_of"))?;
        Ok(rows.iter().map(row_to_link).collect())
    }

    /// Graph nodes for a workspace (optionally a single collection): (id,title,kind,collection).
    pub async fn graph_nodes(
        &self,
        ws: &str,
        collection: Option<&str>,
    ) -> Result<Vec<(String, String, String, String)>> {
        let mut sql =
            String::from("SELECT id, title, kind, collection FROM memories WHERE workspace_id = ? AND active = 1");
        if collection.is_some() {
            sql.push_str(" AND collection = ?");
        }
        sql.push_str(" LIMIT 5000");
        let mut q = sqlx::query(&sql).bind(ws);
        if let Some(c) = collection {
            q = q.bind(c);
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("memory.graph_nodes"))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("id"),
                    r.get::<String, _>("title"),
                    r.get::<String, _>("kind"),
                    r.get::<String, _>("collection"),
                )
            })
            .collect())
    }

    /// Map `source_ref → memory id` for a given `source_kind` (used to resolve
    /// imported graph edges to the memory rows their nodes became).
    pub async fn ids_by_source_ref(
        &self,
        ws: &str,
        source_kind: &str,
    ) -> Result<std::collections::HashMap<String, String>> {
        let rows = sqlx::query(
            "SELECT id, source_ref FROM memories WHERE workspace_id = ? AND source_kind = ? \
             AND source_ref IS NOT NULL AND active = 1",
        )
        .bind(ws)
        .bind(source_kind)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("memory.ids_by_source_ref"))?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let sr: Option<String> = r.get("source_ref");
                sr.map(|s| (s, r.get::<String, _>("id")))
            })
            .collect())
    }

    pub async fn all_links(&self, ws: &str) -> Result<Vec<MemoryLink>> {
        let rows = sqlx::query(
            "SELECT l.src_id, l.dst_id, l.rel, l.weight, l.certainty FROM memory_links l \
             JOIN memories m ON m.id = l.src_id WHERE m.workspace_id = ?",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("memory.all_links"))?;
        Ok(rows.iter().map(row_to_link).collect())
    }

    // -- governance (0056_memory_lifecycle.sql) ---------------------------------

    /// Transition a memory's lifecycle state. Valid transitions are enforced by
    /// the service layer; the repo blindly writes the requested state.
    pub async fn set_state(&self, ws: &str, id: &str, state: &str) -> Result<Memory> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE memories SET state=?, updated_at=? WHERE id=? AND workspace_id=?",
        )
        .bind(state)
        .bind(&now)
        .bind(id)
        .bind(ws)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.set_state"))?;
        self.get(ws, id).await
    }

    /// Soft-delete a memory: set `active=0`, `forgotten_at`, and mint an opaque
    /// `undo_token`. Returns the token so the caller can hand it to the client.
    pub async fn soft_forget(&self, ws: &str, id: &str) -> Result<String> {
        // Verify the row exists in this workspace first (returns NotFound if absent).
        let _ = self.get(ws, id).await?;
        let now = fmt(Utc::now());
        let epoch = Utc::now().timestamp();
        // Random 32-byte hex token — sufficient entropy, no external crate needed.
        let token = {
            use std::fmt::Write as _;
            let mut raw = [0u8; 32];
            // Use the standard library's available entropy source on stable Rust.
            // Deterministic-safe: SHA-256 over (id + workspace + epoch nanos).
            let seed = format!(
                "{}{}{}{}",
                id,
                ws,
                epoch,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos())
                    .unwrap_or(42)
            );
            // SHA-256 via sha2 (already a workspace dep).
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(seed.as_bytes());
            raw.copy_from_slice(&hash);
            let mut s = String::with_capacity(64);
            for b in &raw {
                let _ = write!(s, "{:02x}", b);
            }
            s
        };
        sqlx::query(
            "UPDATE memories SET active=0, forgotten_at=?, undo_token=?, \
             state='stale', updated_at=? WHERE id=? AND workspace_id=?",
        )
        .bind(epoch)
        .bind(&token)
        .bind(&now)
        .bind(id)
        .bind(ws)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.soft_forget"))?;
        Ok(token)
    }

    /// Undo a soft-delete: restore `active=1`, clear `forgotten_at` and
    /// `undo_token`, set state back to `accepted`. Returns the restored memory.
    pub async fn undo_forget(&self, ws: &str, undo_token: &str) -> Result<Memory> {
        // Locate the row by its undo token, scoped to the workspace.
        let row = sqlx::query(
            "SELECT id FROM memories WHERE undo_token=? AND workspace_id=?",
        )
        .bind(undo_token)
        .bind(ws)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("memory.undo_forget.find"))?
        .ok_or_else(|| Error::NotFound("undo token not found or already used".into()))?;
        let id: String = row.get("id");
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE memories SET active=1, forgotten_at=NULL, undo_token=NULL, \
             state='accepted', updated_at=? WHERE id=? AND workspace_id=?",
        )
        .bind(&now)
        .bind(&id)
        .bind(ws)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.undo_forget"))?;
        self.get(ws, &id).await
    }

    /// Record merge provenance on a memory (the newly created merged row). Also
    /// marks all source memories as `contradicted` + sets their `superseded_by`.
    pub async fn record_merge(
        &self,
        ws: &str,
        merged_id: &str,
        source_ids: &[String],
        provenance_json: &str,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE memories SET provenance_json=?, updated_at=? WHERE id=? AND workspace_id=?",
        )
        .bind(provenance_json)
        .bind(&now)
        .bind(merged_id)
        .bind(ws)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.record_merge.provenance"))?;
        for src in source_ids {
            sqlx::query(
                "UPDATE memories SET active=0, state='contradicted', superseded_by=?, \
                 updated_at=? WHERE id=? AND workspace_id=?",
            )
            .bind(merged_id)
            .bind(&now)
            .bind(src)
            .bind(ws)
            .execute(&self.pool)
            .await
            .map_err(dberr("memory.record_merge.source"))?;
        }
        Ok(())
    }

    /// Record split provenance on a set of child memories and mark the parent
    /// `contradicted` + point it at the first child (as `superseded_by`).
    pub async fn record_split(
        &self,
        ws: &str,
        parent_id: &str,
        child_ids: &[String],
        provenance_json: &str,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        for child in child_ids {
            sqlx::query(
                "UPDATE memories SET provenance_json=?, updated_at=? \
                 WHERE id=? AND workspace_id=?",
            )
            .bind(provenance_json)
            .bind(&now)
            .bind(child)
            .bind(ws)
            .execute(&self.pool)
            .await
            .map_err(dberr("memory.record_split.child"))?;
        }
        let first_child = child_ids.first().map(String::as_str).unwrap_or("");
        sqlx::query(
            "UPDATE memories SET active=0, state='contradicted', superseded_by=?, \
             updated_at=? WHERE id=? AND workspace_id=?",
        )
        .bind(first_child)
        .bind(&now)
        .bind(parent_id)
        .bind(ws)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.record_split.parent"))?;
        Ok(())
    }

    // -- governed import ---------------------------------------------------------

    /// Persist a governed-import record (after the memories themselves are created).
    pub async fn create_governed_import(
        &self,
        ws: &str,
        kind: &str,
        label: &str,
        memory_ids: &[String],
        by: &str,
    ) -> Result<GovernedImport> {
        let id = new_id();
        let now = fmt(Utc::now());
        let ids_json = jstr(&memory_ids);
        sqlx::query(
            "INSERT INTO governed_imports \
             (id,workspace_id,kind,label,memory_ids_json,imported_by,imported_at) \
             VALUES (?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(ws)
        .bind(kind)
        .bind(label)
        .bind(&ids_json)
        .bind(by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("governed_import.create"))?;
        Ok(GovernedImport {
            id,
            workspace_id: ws.into(),
            kind: kind.into(),
            label: label.into(),
            memory_ids: memory_ids.to_vec(),
            imported_by: by.into(),
            imported_at: now,
            reverted_at: None,
        })
    }

    /// List governed imports for a workspace (most recent first).
    pub async fn list_governed_imports(&self, ws: &str) -> Result<Vec<GovernedImport>> {
        let rows = sqlx::query(
            "SELECT * FROM governed_imports WHERE workspace_id=? ORDER BY imported_at DESC",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("governed_import.list"))?;
        rows.iter().map(row_to_governed_import).collect()
    }
}

fn row_to_link(r: &sqlx::sqlite::SqliteRow) -> MemoryLink {
    MemoryLink {
        src_id: r.get("src_id"),
        dst_id: r.get("dst_id"),
        rel: r.get("rel"),
        weight: r.get::<f64, _>("weight") as f32,
        certainty: r.get("certainty"),
    }
}

fn row_to_governed_import(r: &sqlx::sqlite::SqliteRow) -> Result<GovernedImport> {
    let ids_json: String = r.get("memory_ids_json");
    let memory_ids: Vec<String> = serde_json::from_str(&ids_json).unwrap_or_default();
    Ok(GovernedImport {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        kind: r.get("kind"),
        label: r.get("label"),
        memory_ids,
        imported_by: r.get("imported_by"),
        imported_at: r.get("imported_at"),
        reverted_at: r.get("reverted_at"),
    })
}

// ---------------------------------------------------------------------------
// FTS5 keyword index (Vault v2). A standalone, app-maintained FTS5 table —
// `memories_fts(mid, ws, title, body)`. Created at runtime (not in a migration)
// so a SQLite build without FTS5 degrades to the existing LIKE search instead of
// aborting migrations. Callers check `ensure_fts()` once and fall back to
// `search_keyword` when it returns `false`.
// ---------------------------------------------------------------------------

/// Tokenize free text into a safe FTS5 MATCH expression: each ≥2-char alnum term
/// is quoted (so `:`/`-`/`*`/`(` can't be a MATCH syntax error) and OR-joined,
/// matching `search_keyword`'s any-term semantics. `None` when there are no terms.
fn fts_match_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(|s| format!("\"{s}\""))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

impl MemoriesRepo {
    /// Create the FTS5 index if this SQLite build supports it, returning whether
    /// FTS5 is available. Idempotent; backfills any not-yet-indexed memories on
    /// first creation. A build without FTS5 returns `Ok(false)` (→ LIKE fallback)
    /// rather than erroring.
    pub async fn ensure_fts(&self) -> Result<bool> {
        let created = sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(\
             mid UNINDEXED, ws UNINDEXED, title, body, tokenize='porter unicode61')",
        )
        .execute(&self.pool)
        .await;
        if created.is_err() {
            return Ok(false);
        }
        // Backfill ONCE, only when the index is empty — a full O(n) INSERT…SELECT.
        // (The previous per-row `WHERE NOT EXISTS` scanned the UNINDEXED `mid`
        // column for every memory → O(n²), seconds on a large vault. fts_index
        // keeps it in sync on writes, so a one-time bulk fill is sufficient.)
        let empty = sqlx::query("SELECT COUNT(*) AS c FROM memories_fts")
            .fetch_one(&self.pool)
            .await
            .map(|r| r.get::<i64, _>("c") == 0)
            .unwrap_or(false);
        if empty {
            let _ = sqlx::query(
                "INSERT INTO memories_fts (mid, ws, title, body) \
                 SELECT id, workspace_id, title, body FROM memories",
            )
            .execute(&self.pool)
            .await;
        }
        Ok(true)
    }

    /// Upsert a memory's text into the FTS index (delete-then-insert; FTS5
    /// external tables have no UPSERT). Silent no-op when FTS5 is unavailable.
    pub async fn fts_index(&self, mid: &str, ws: &str, title: &str, body: &str) -> Result<()> {
        let _ = sqlx::query("DELETE FROM memories_fts WHERE mid = ?")
            .bind(mid)
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("INSERT INTO memories_fts (mid, ws, title, body) VALUES (?,?,?,?)")
            .bind(mid)
            .bind(ws)
            .bind(title)
            .bind(body)
            .execute(&self.pool)
            .await;
        Ok(())
    }

    /// Count active vectors for (workspace, model) — the cheap signature the
    /// HNSW index uses to decide between its exact path and a cached ANN graph.
    pub async fn count_vectors(&self, ws: &str, model: &str) -> Result<usize> {
        let r = sqlx::query(
            "SELECT COUNT(*) AS c FROM memory_vectors v \
             JOIN memories m ON m.id = v.memory_id \
             WHERE m.workspace_id = ? AND m.active = 1 AND v.model_id = ?",
        )
        .bind(ws)
        .bind(model)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("memory.count_vectors"))?;
        Ok(r.get::<i64, _>("c").max(0) as usize)
    }

    /// Remove a memory from the FTS index.
    pub async fn fts_remove(&self, mid: &str) -> Result<()> {
        let _ = sqlx::query("DELETE FROM memories_fts WHERE mid = ?")
            .bind(mid)
            .execute(&self.pool)
            .await;
        Ok(())
    }

    /// FTS5 keyword search ranked by bm25 (lower = better). Mirrors
    /// `search_keyword`'s filters. Returns (memory, score) best-first; an empty
    /// query yields no matches.
    pub async fn search_fts(
        &self,
        ws: &str,
        query: &str,
        f: &SearchFilter,
    ) -> Result<Vec<(Memory, f32)>> {
        let Some(mq) = fts_match_query(query) else {
            return Ok(vec![]);
        };
        let mut sql = String::from(
            "SELECT m.*, bm25(memories_fts) AS rank FROM memories_fts \
             JOIN memories m ON m.id = memories_fts.mid \
             WHERE memories_fts MATCH ? AND memories_fts.ws = ?",
        );
        if !f.include_inactive {
            sql.push_str(" AND m.active = 1");
        }
        if f.collection.is_some() {
            sql.push_str(" AND m.collection = ?");
        }
        if f.story_id.is_some() {
            sql.push_str(" AND m.story_id = ?");
        }
        sql.push_str(" ORDER BY rank ASC LIMIT ?");
        let mut q = sqlx::query(&sql).bind(&mq).bind(ws);
        if let Some(c) = &f.collection {
            q = q.bind(c);
        }
        if let Some(s) = &f.story_id {
            q = q.bind(s);
        }
        let lim = if f.limit > 0 { f.limit } else { 50 };
        q = q.bind(lim);
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("memory.search_fts"))?;
        let n = rows.len();
        Ok(rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| row_to_memory(r).ok().map(|m| (m, (n - i) as f32)))
            .collect())
    }
}
