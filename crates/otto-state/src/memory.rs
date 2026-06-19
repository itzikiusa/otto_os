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

use otto_core::{new_id, Result};

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
}

fn default_collection() -> String {
    "product".into()
}
fn default_record_type() -> String {
    "item".into()
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
             entities_json,tags_json,source_kind,source_ref,refs_json,confidence,salience,content_hash,\
             active,version,created_by,created_at,updated_at,access_count) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,1,1,?,?,?,0)",
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
