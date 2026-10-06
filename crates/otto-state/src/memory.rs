//! Memory layer repository — items + chunks, vectors, and the link graph.
//!
//! Persistence DTOs live here (otto-state cannot depend on otto-memory); the
//! `otto-memory` crate re-exports them. Keyword search uses LIKE (always
//! available, instant at single-user scale); vectors are stored as little-endian
//! f32 BLOBs and searched in-process by the caller's `VectorIndex`.

use crate::DbPool;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;

use otto_core::{new_id, Error, Result};

use crate::convert::{dberr, dberr_unique, fmt};

/// How long a soft-forget's undo token stays valid (from `forgotten_at`).
pub const UNDO_FORGET_WINDOW_SECS: i64 = 30 * 24 * 3600;

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
    /// delete. NEVER serialized: it is handed out only by `forget` — in every
    /// list/get JSON it would let any Viewer read the secret (S7-307).
    #[serde(default, skip_serializing)]
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
    /// Restrict to these `kind`s (empty = any). Applied in SQL, BEFORE the
    /// LIMIT — filtering after it let a section come back empty even though
    /// matching memories existed.
    pub kinds: Vec<String>,
    /// Hide OTHER users' private memories (non-`private` rows, or this user's
    /// own). `None` sees everything.
    pub viewer: Option<String>,
}

impl SearchFilter {
    /// SQL for the `kinds`/`viewer` predicates (`col` = column prefix, e.g.
    /// `"m."`), bound by [`Self::bind_extra`] in the same order.
    fn extra_sql(&self, col: &str) -> String {
        let mut sql = String::new();
        if !self.kinds.is_empty() {
            let marks = vec!["?"; self.kinds.len()].join(",");
            sql.push_str(&format!(" AND {col}kind IN ({marks})"));
        }
        if self.viewer.is_some() {
            sql.push_str(&format!(
                " AND ({col}visibility != 'private' OR {col}created_by = ?)"
            ));
        }
        sql
    }

    fn bind_extra<'q>(
        &'q self,
        mut q: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>,
    ) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments> {
        for k in &self.kinds {
            q = q.bind(k);
        }
        if let Some(v) = &self.viewer {
            q = q.bind(v);
        }
        q
    }
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

/// What [`MemoriesRepo::save_one`] did with an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// A new row was inserted.
    Created,
    /// An inactive (forgotten / merged / superseded) duplicate was revived.
    Reactivated,
    /// A live duplicate already existed; it is returned unchanged.
    Existing,
}

impl SaveOutcome {
    /// True when the save produced a row that was not live before (created or
    /// reactivated) — what callers mean by "imported".
    pub fn is_new(self) -> bool {
        !matches!(self, SaveOutcome::Existing)
    }
}

/// The dedup unique index (`idx_memories_dedup`) covers inactive rows too; a
/// write that collides with it is a caller-facing 409, not a 500.
const DUPLICATE_MEMORY: &str =
    "a memory with the same content already exists in this collection and scope";

async fn insert_conn(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    ws: &str,
    by: &str,
    nm: &NewMemory,
) -> Result<()> {
    let now = fmt(Utc::now());
    let hash = MemoriesRepo::content_hash(&nm.body);
    sqlx::query(
        "INSERT INTO memories (id,workspace_id,collection,record_type,scope,story_id,kind,title,body,\
         entities_json,tags_json,source_kind,source_ref,refs_json,confidence,salience,visibility,content_hash,\
         active,version,created_by,created_at,updated_at,access_count) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,1,1,?,?,?,0)",
    )
    .bind(id)
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
    .execute(&mut *conn)
    .await
    .map_err(dberr_unique("memory.create", DUPLICATE_MEMORY))?;
    Ok(())
}

async fn get_conn(conn: &mut sqlx::SqliteConnection, ws: &str, id: &str) -> Result<Memory> {
    let r = sqlx::query("SELECT * FROM memories WHERE id = ? AND workspace_id = ?")
        .bind(id)
        .bind(ws)
        .fetch_one(&mut *conn)
        .await
        .map_err(dberr("memory"))?;
    row_to_memory(&r)
}

#[derive(Clone)]
pub struct MemoriesRepo {
    pool: DbPool,
}

impl MemoriesRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    /// Raw pool access — for callers that need to run statements not yet exposed
    /// on the repo (e.g. the governance service updating provenance_json).
    pub fn pool(&self) -> &DbPool {
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
        hex::encode(h.finalize())
    }

    /// Insert one memory row (no dedup lookup, no FTS). A row whose content
    /// matches an existing one (active or not — the dedup index covers both)
    /// is a 409 Conflict, not a 500; [`Self::save_one`] is the dedup-aware path.
    pub async fn create(&self, ws: &str, by: &str, nm: NewMemory) -> Result<Memory> {
        let mut tx = self.pool.begin().await.map_err(dberr("memory.create"))?;
        let id = new_id();
        insert_conn(&mut tx, &id, ws, by, &nm).await?;
        let m = get_conn(&mut tx, ws, &id).await?;
        tx.commit().await.map_err(dberr("memory.create"))?;
        Ok(m)
    }

    /// Save one memory atomically, deduplicating by normalized content hash
    /// within its collection/scope/story — ONE `BEGIN IMMEDIATE` transaction
    /// covering the lookup, the row write and (when `fts`) its FTS row + rowid
    /// map entry, so the index can't silently drift from the table:
    ///
    /// - a live duplicate is returned unchanged ([`SaveOutcome::Existing`]);
    /// - a forgotten / merged / superseded duplicate is REACTIVATED (the
    ///   dedup unique index covers inactive rows, so a plain INSERT 500'd and
    ///   left a batch import half-applied) — [`SaveOutcome::Reactivated`];
    /// - otherwise a new row is inserted ([`SaveOutcome::Created`]).
    ///
    /// The FTS write runs under a savepoint: if it fails the memory still
    /// commits and the next [`Self::ensure_fts`] reconcile repairs the index.
    pub async fn save_one(
        &self,
        ws: &str,
        by: &str,
        nm: NewMemory,
        fts: bool,
    ) -> Result<(Memory, SaveOutcome)> {
        let hash = Self::content_hash(&nm.body);
        let mut tx = self.pool.begin().await.map_err(dberr("memory.save"))?;
        let found: Option<(String, i64)> = sqlx::query_as(
            "SELECT id, active FROM memories WHERE workspace_id = ? AND collection = ? AND scope = ? \
             AND IFNULL(story_id,'') = IFNULL(?,'') AND content_hash = ?",
        )
        .bind(ws)
        .bind(&nm.collection)
        .bind(nm.scope.as_str())
        .bind(&nm.story_id)
        .bind(&hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(dberr("memory.save.find"))?;
        let (id, outcome) = match found {
            Some((id, active)) if active != 0 => {
                let m = get_conn(&mut tx, ws, &id).await?;
                tx.rollback().await.map_err(dberr("memory.save"))?;
                return Ok((m, SaveOutcome::Existing));
            }
            Some((id, _)) => {
                sqlx::query(
                    "UPDATE memories SET active=1, superseded_by=NULL, forgotten_at=NULL, \
                     undo_token=NULL, state='accepted', version=version+1, updated_at=? \
                     WHERE id=? AND workspace_id=?",
                )
                .bind(fmt(Utc::now()))
                .bind(&id)
                .bind(ws)
                .execute(&mut *tx)
                .await
                .map_err(dberr("memory.save.reactivate"))?;
                (id, SaveOutcome::Reactivated)
            }
            None => {
                let id = new_id();
                insert_conn(&mut tx, &id, ws, by, &nm).await?;
                (id, SaveOutcome::Created)
            }
        };
        let m = get_conn(&mut tx, ws, &id).await?;
        if fts {
            fts_put_guarded(&mut tx, &m.id, &m.workspace_id, &m.title, &m.body).await;
        }
        tx.commit().await.map_err(dberr("memory.save"))?;
        Ok((m, outcome))
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
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(ws);
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
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("memory.list"))?;
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
        sql.push_str(&f.extra_sql(""));
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
        let lim = if f.limit > 0 { f.limit as usize } else { 50 };
        if terms.is_empty() {
            // No terms: every row scores 0 — return the newest `lim`, not an
            // arbitrary 2 000-row slice decoded only to be truncated.
            sql.push_str(&format!(" ORDER BY updated_at DESC LIMIT {lim}"));
        } else {
            // Candidates for the term-hit ranking below; bounded relative to
            // the page (was a flat 2 000 rows / ~3 MB decoded per miss).
            sql.push_str(&format!(" LIMIT {}", (lim * 10).clamp(100, 1000)));
        }
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(ws);
        if let Some(c) = &f.collection {
            q = q.bind(c);
        }
        if let Some(s) = &f.story_id {
            q = q.bind(s);
        }
        q = f.bind_extra(q);
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
        self.update_indexed(ws, id, p, false).await
    }

    /// [`Self::update`] in one `BEGIN IMMEDIATE` transaction that also
    /// refreshes the memory's FTS row when `fts` (see [`Self::save_one`]). An
    /// edit that makes the body collide with another memory of the same
    /// collection/scope (live or not) is a 409 Conflict.
    pub async fn update_indexed(
        &self,
        ws: &str,
        id: &str,
        p: MemoryPatch,
        fts: bool,
    ) -> Result<Memory> {
        let mut tx = self.pool.begin().await.map_err(dberr("memory.update"))?;
        let cur = get_conn(&mut tx, ws, id).await?;
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
        .execute(&mut *tx)
        .await
        .map_err(dberr_unique("memory.update", DUPLICATE_MEMORY))?;
        let m = get_conn(&mut tx, ws, id).await?;
        if fts {
            fts_put_guarded(&mut tx, &m.id, &m.workspace_id, &m.title, &m.body).await;
        }
        tx.commit().await.map_err(dberr("memory.update"))?;
        Ok(m)
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

    /// Count a recall of each hit. ONE statement for the whole hit list (it
    /// used to be one autocommit UPDATE per hit — a `recall_brief` took the
    /// write lock up to ~48 times). An id listed twice counts once.
    pub async fn bump_access(&self, ws: &str, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids_json = serde_json::to_string(ids)
            .map_err(|e| Error::Internal(format!("memory.bump_access: {e}")))?;
        sqlx::query(
            "UPDATE memories SET access_count=access_count+1, last_accessed_at=? \
             WHERE workspace_id=? AND id IN (SELECT value FROM json_each(?))",
        )
        .bind(fmt(Utc::now()))
        .bind(ws)
        .bind(ids_json)
        .execute(&self.pool)
        .await
        .map_err(dberr("memory.bump_access"))?;
        Ok(())
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
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(ws);
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
        sqlx::query("UPDATE memories SET state=?, updated_at=? WHERE id=? AND workspace_id=?")
            .bind(state)
            .bind(&now)
            .bind(id)
            .bind(ws)
            .execute(&self.pool)
            .await
            .map_err(dberr("memory.set_state"))?;
        self.get(ws, id).await
    }

    /// An unguessable single-use undo token: SHA-256 over three ULIDs (each
    /// carrying 80 bits from the thread CSPRNG — 240 random bits in all). It
    /// used to hash only (id, ws, epoch, nanos), which an observer of the
    /// forget could reconstruct (S7-06).
    fn mint_undo_token() -> String {
        use sha2::{Digest, Sha256};
        let seed = format!("{}{}{}", new_id(), new_id(), new_id());
        hex::encode(Sha256::digest(seed.as_bytes()))
    }

    /// Soft-delete a memory: set `active=0`, `forgotten_at`, and mint an opaque
    /// `undo_token`. Returns the token so the caller can hand it to the client.
    pub async fn soft_forget(&self, ws: &str, id: &str) -> Result<String> {
        // Verify the row exists in this workspace first (returns NotFound if absent).
        let _ = self.get(ws, id).await?;
        let now = fmt(Utc::now());
        let epoch = Utc::now().timestamp();
        let token = Self::mint_undo_token();
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
        self.undo_forget_scoped(ws, None, undo_token).await
    }

    /// [`Self::undo_forget`] pinned to memory `id` when given: the
    /// `/memory/{mid}/forget/undo` route restores ONLY `mid`, never whatever
    /// row the token names (S7-307).
    pub async fn undo_forget_scoped(
        &self,
        ws: &str,
        id: Option<&str>,
        undo_token: &str,
    ) -> Result<Memory> {
        // Locate the row by its undo token, scoped to the workspace.
        // Tokens expire with the undo window (measured from `forgotten_at`).
        let oldest = Utc::now().timestamp() - UNDO_FORGET_WINDOW_SECS;
        let row = sqlx::query(
            "SELECT id FROM memories WHERE undo_token=? AND workspace_id=? \
             AND forgotten_at IS NOT NULL AND forgotten_at >= ? AND (? IS NULL OR id = ?)",
        )
        .bind(undo_token)
        .bind(ws)
        .bind(oldest)
        .bind(id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("memory.undo_forget.find"))?
        .ok_or_else(|| Error::NotFound("undo token not found, expired or already used".into()))?;
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

    /// Record merge provenance on a memory (the merged row). Also marks every
    /// OTHER source memory `contradicted` + sets its `superseded_by`. A source
    /// that IS the merged row (the merged text deduplicated onto it) keeps its
    /// active state — deactivating it would make the merged knowledge vanish
    /// (S7-02). One write transaction: provenance and sources move together.
    pub async fn record_merge(
        &self,
        ws: &str,
        merged_id: &str,
        source_ids: &[String],
        provenance_json: &str,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("memory.record_merge"))?;
        sqlx::query(
            "UPDATE memories SET provenance_json=?, updated_at=? WHERE id=? AND workspace_id=?",
        )
        .bind(provenance_json)
        .bind(&now)
        .bind(merged_id)
        .bind(ws)
        .execute(&mut *tx)
        .await
        .map_err(dberr("memory.record_merge.provenance"))?;
        for src in source_ids.iter().filter(|s| s.as_str() != merged_id) {
            sqlx::query(
                "UPDATE memories SET active=0, state='contradicted', superseded_by=?, \
                 updated_at=? WHERE id=? AND workspace_id=?",
            )
            .bind(merged_id)
            .bind(&now)
            .bind(src)
            .bind(ws)
            .execute(&mut *tx)
            .await
            .map_err(dberr("memory.record_merge.source"))?;
        }
        tx.commit().await.map_err(dberr("memory.record_merge"))
    }

    /// Record split provenance on a set of child memories and mark the parent
    /// `contradicted` + point it at the first child (as `superseded_by`). The
    /// parent never supersedes itself: a child id equal to the parent is
    /// refused (S7-02). One write transaction.
    pub async fn record_split(
        &self,
        ws: &str,
        parent_id: &str,
        child_ids: &[String],
        provenance_json: &str,
    ) -> Result<()> {
        if child_ids.is_empty() || child_ids.iter().any(|c| c == parent_id) {
            return Err(Error::Invalid(
                "a split part may not equal the parent memory".into(),
            ));
        }
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("memory.record_split"))?;
        for child in child_ids {
            sqlx::query(
                "UPDATE memories SET provenance_json=?, updated_at=? \
                 WHERE id=? AND workspace_id=?",
            )
            .bind(provenance_json)
            .bind(&now)
            .bind(child)
            .bind(ws)
            .execute(&mut *tx)
            .await
            .map_err(dberr("memory.record_split.child"))?;
        }
        sqlx::query(
            "UPDATE memories SET active=0, state='contradicted', superseded_by=?, \
             updated_at=? WHERE id=? AND workspace_id=?",
        )
        .bind(&child_ids[0])
        .bind(&now)
        .bind(parent_id)
        .bind(ws)
        .execute(&mut *tx)
        .await
        .map_err(dberr("memory.record_split.parent"))?;
        tx.commit().await.map_err(dberr("memory.record_split"))
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

/// Upsert one memory's FTS row through the `mid → rowid` map on `conn`
/// (an open transaction).
async fn fts_put_conn(
    conn: &mut sqlx::SqliteConnection,
    mid: &str,
    ws: &str,
    title: &str,
    body: &str,
) -> std::result::Result<(), sqlx::Error> {
    let rid: Option<i64> = sqlx::query_scalar("SELECT rid FROM memories_fts_ids WHERE mid = ?")
        .bind(mid)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(rid) = rid {
        let n = sqlx::query("UPDATE memories_fts SET ws = ?, title = ?, body = ? WHERE rowid = ?")
            .bind(ws)
            .bind(title)
            .bind(body)
            .bind(rid)
            .execute(&mut *conn)
            .await?
            .rows_affected();
        if n > 0 {
            return Ok(());
        }
    }
    let ins = sqlx::query("INSERT INTO memories_fts (mid, ws, title, body) VALUES (?,?,?,?)")
        .bind(mid)
        .bind(ws)
        .bind(title)
        .bind(body)
        .execute(&mut *conn)
        .await?;
    sqlx::query("INSERT OR REPLACE INTO memories_fts_ids (mid, rid) VALUES (?, ?)")
        .bind(mid)
        .bind(ins.last_insert_rowid())
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Delete one memory's FTS row + map entry on `conn` (an open transaction).
async fn fts_del_conn(
    conn: &mut sqlx::SqliteConnection,
    mid: &str,
) -> std::result::Result<(), sqlx::Error> {
    let rid: Option<i64> = sqlx::query_scalar("SELECT rid FROM memories_fts_ids WHERE mid = ?")
        .bind(mid)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(rid) = rid {
        sqlx::query("DELETE FROM memories_fts WHERE rowid = ?")
            .bind(rid)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM memories_fts_ids WHERE mid = ?")
            .bind(mid)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// [`fts_put_conn`] under a SAVEPOINT inside the caller's write transaction:
/// the index write commits WITH the memory row, and an index failure rolls
/// back only itself (the memory still saves; [`MemoriesRepo::reconcile_fts`]
/// repairs the gap on the next start) instead of failing the user's save.
async fn fts_put_guarded(
    conn: &mut sqlx::SqliteConnection,
    mid: &str,
    ws: &str,
    title: &str,
    body: &str,
) {
    if sqlx::query("SAVEPOINT memory_fts")
        .execute(&mut *conn)
        .await
        .is_err()
    {
        return;
    }
    if let Err(e) = fts_put_conn(conn, mid, ws, title, body).await {
        tracing::warn!(memory = mid, error = %e, "memory FTS write failed; reconciled on next start");
        let _ = sqlx::query("ROLLBACK TO memory_fts")
            .execute(&mut *conn)
            .await;
    }
    let _ = sqlx::query("RELEASE memory_fts").execute(&mut *conn).await;
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
        if let Err(e) = created {
            // Only a build WITHOUT FTS5 is a permanent "no" (→ LIKE fallback).
            // Anything else (SQLITE_BUSY past the busy timeout while boot
            // tasks hold the writer, an IO hiccup) is transient: an `Err` the
            // caller retries, never a cached "unavailable" (S7-308).
            if e.to_string().contains("no such module") {
                return Ok(false);
            }
            return Err(dberr("memory.ensure_fts")(e));
        }
        // Without the map every index write would fail: transient, retried.
        self.ensure_fts_map()
            .await
            .map_err(dberr("memory.ensure_fts_map"))?;
        // Once per daemon: repair whatever drifted while the index was
        // written outside the memory's own transaction (older builds), or a
        // savepointed FTS write failed. A failed repair leaves FTS usable.
        match self.reconcile_fts().await {
            Ok(0) => {}
            Ok(n) => tracing::info!(repaired = n, "memory FTS index reconciled"),
            Err(e) => tracing::warn!(error = %e, "memory FTS reconcile failed"),
        }
        Ok(true)
    }

    /// The `mid → FTS rowid` map every index write goes through (perf
    /// r3-01-04): `mid` is an UNINDEXED FTS column, so `DELETE … WHERE mid = ?`
    /// scanned the whole index on every memory upsert (~10 ms at 5k memories,
    /// holding the writer). Derived data, created at runtime like the index.
    ///
    /// Backfill ONCE: an empty index is bulk-filled from `memories` (a full
    /// O(n) INSERT…SELECT; the old per-row `WHERE NOT EXISTS` was O(n²)); an
    /// index built before the map existed gets its map in one scan, keeping the
    /// newest row per memory and dropping duplicates.
    async fn ensure_fts_map(&self) -> std::result::Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='memories_fts_ids' AND type='table')",
        )
        .fetch_one(&mut *tx)
        .await?;
        if !exists {
            sqlx::query(
                "CREATE TABLE memories_fts_ids (mid TEXT PRIMARY KEY, rid INTEGER NOT NULL)",
            )
            .execute(&mut *tx)
            .await?;
            let empty: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM memories_fts)")
                .fetch_one(&mut *tx)
                .await?;
            if empty {
                sqlx::query(
                    "INSERT INTO memories_fts (mid, ws, title, body) \
                     SELECT id, workspace_id, title, body FROM memories",
                )
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query(
                "INSERT OR REPLACE INTO memories_fts_ids (mid, rid) \
                 SELECT mid, rowid FROM memories_fts ORDER BY rowid",
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM memories_fts WHERE rowid NOT IN (SELECT rid FROM memories_fts_ids)",
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }

    /// Upsert a memory's text into the FTS index: an UPDATE by rowid through
    /// the map when indexed, else INSERT + map row — one transaction (it used
    /// to be two autocommit writes around a whole-index scan). Silent no-op
    /// when FTS5 is unavailable. Saves/updates index inside their own
    /// transaction ([`Self::save_one`]); this is for out-of-band callers.
    pub async fn fts_index(&self, mid: &str, ws: &str, title: &str, body: &str) -> Result<()> {
        let put = async {
            let mut tx = self.pool.begin().await?;
            fts_put_conn(&mut tx, mid, ws, title, body).await?;
            tx.commit().await
        };
        let _: std::result::Result<(), sqlx::Error> = put.await;
        Ok(())
    }

    /// Remove a memory from the FTS index (by rowid through the map; the old
    /// scan only when the map was never built on this DB).
    pub async fn fts_remove(&self, mid: &str) -> Result<()> {
        let del = async {
            let mut tx = self.pool.begin().await?;
            fts_del_conn(&mut tx, mid).await?;
            tx.commit().await
        };
        let res: std::result::Result<(), sqlx::Error> = del.await;
        if res.is_err() {
            let _ = sqlx::query("DELETE FROM memories_fts WHERE mid = ?")
                .bind(mid)
                .execute(&self.pool)
                .await;
        }
        Ok(())
    }

    /// Repair drift between `memories` and the FTS index (+ its rowid map),
    /// in ONE `BEGIN IMMEDIATE` transaction. The index mirrors EVERY memory
    /// row (search filters `active` itself), so this:
    ///
    /// 1. drops map rows pointing at a vanished FTS row, and FTS/map rows of
    ///    memories that no longer exist (hard deletes, workspace cascades);
    /// 2. indexes every memory missing from the map (a crash or a failed FTS
    ///    write left it unindexed — search would silently never find it);
    /// 3. rewrites FTS rows whose text no longer matches the memory.
    ///
    /// Returns how many rows were repaired. Runs once per daemon from
    /// [`Self::ensure_fts`]; O(n) over `memories`, cheap at memory-store scale.
    pub async fn reconcile_fts(&self) -> std::result::Result<u64, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let mut fixed = 0;
        fixed += sqlx::query(
            "DELETE FROM memories_fts_ids WHERE rid NOT IN (SELECT rowid FROM memories_fts)",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        fixed += sqlx::query(
            "DELETE FROM memories_fts WHERE rowid IN (SELECT rid FROM memories_fts_ids \
             WHERE mid NOT IN (SELECT id FROM memories))",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query("DELETE FROM memories_fts_ids WHERE mid NOT IN (SELECT id FROM memories)")
            .execute(&mut *tx)
            .await?;
        // Unmapped FTS rows are unreachable through the map (a pre-map crash
        // remnant): drop them so a re-index can't leave a ghost duplicate.
        fixed += sqlx::query(
            "DELETE FROM memories_fts WHERE rowid NOT IN (SELECT rid FROM memories_fts_ids)",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        // FTS5 hands out rowid = max+1, so everything above the current max
        // was inserted by the backfill below.
        let max: i64 = sqlx::query_scalar("SELECT IFNULL(MAX(rowid), 0) FROM memories_fts")
            .fetch_one(&mut *tx)
            .await?;
        let missing = sqlx::query(
            "INSERT INTO memories_fts (mid, ws, title, body) \
             SELECT id, workspace_id, title, body FROM memories \
             WHERE id NOT IN (SELECT mid FROM memories_fts_ids)",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if missing > 0 {
            sqlx::query(
                "INSERT OR REPLACE INTO memories_fts_ids (mid, rid) \
                 SELECT mid, rowid FROM memories_fts WHERE rowid > ?",
            )
            .bind(max)
            .execute(&mut *tx)
            .await?;
        }
        fixed += missing;
        let stale: Vec<(i64, String, String, String)> = sqlx::query_as(
            "SELECT i.rid, m.workspace_id, m.title, m.body FROM memories m \
             JOIN memories_fts_ids i ON i.mid = m.id JOIN memories_fts f ON f.rowid = i.rid \
             WHERE f.ws IS NOT m.workspace_id OR f.title IS NOT m.title OR f.body IS NOT m.body",
        )
        .fetch_all(&mut *tx)
        .await?;
        for (rid, ws, title, body) in &stale {
            sqlx::query("UPDATE memories_fts SET ws = ?, title = ?, body = ? WHERE rowid = ?")
                .bind(ws)
                .bind(title)
                .bind(body)
                .bind(rid)
                .execute(&mut *tx)
                .await?;
        }
        fixed += stale.len() as u64;
        tx.commit().await?;
        Ok(fixed)
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
        sql.push_str(&f.extra_sql("m."));
        sql.push_str(" ORDER BY rank ASC LIMIT ?");
        let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(&mq)
            .bind(ws);
        if let Some(c) = &f.collection {
            q = q.bind(c);
        }
        if let Some(s) = &f.story_id {
            q = q.bind(s);
        }
        q = f.bind_extra(q);
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

#[cfg(test)]
mod fts_map_tests {
    use super::*;

    async fn rows(r: &MemoriesRepo) -> Vec<(String, String)> {
        sqlx::query_as("SELECT mid, body FROM memories_fts ORDER BY mid")
            .fetch_all(&r.pool)
            .await
            .unwrap()
    }

    /// S7-308: a transient failure (here: the pool is gone) is an `Err` the
    /// service retries, not `Ok(false)` — that cached "FTS unavailable" for
    /// the daemon's lifetime and silently degraded every search to LIKE.
    #[tokio::test]
    async fn transient_fts_failure_is_an_error_not_unavailable() {
        let pool = crate::db::test_pool().await;
        let r = MemoriesRepo::new(pool.clone());
        pool.close().await;
        assert!(r.ensure_fts().await.is_err());
    }

    /// r3-01-04: an upsert replaces the memory's FTS row in place through the
    /// indexed `mid → rowid` map (no whole-index scan), removal deletes it.
    #[tokio::test]
    async fn upsert_and_remove_go_through_the_rowid_map() {
        let r = MemoriesRepo::new(crate::db::test_pool().await);
        assert!(r.ensure_fts().await.unwrap());
        r.fts_index("m1", "ws", "T", "first").await.unwrap();
        r.fts_index("m2", "ws", "T", "other").await.unwrap();
        r.fts_index("m1", "ws", "T", "second").await.unwrap();
        assert_eq!(
            rows(&r).await,
            vec![
                ("m1".to_string(), "second".to_string()),
                ("m2".to_string(), "other".to_string())
            ]
        );
        r.fts_remove("m1").await.unwrap();
        assert_eq!(
            rows(&r).await,
            vec![("m2".to_string(), "other".to_string())]
        );
        let plan: Vec<(i64, i64, i64, String)> =
            sqlx::query_as("EXPLAIN QUERY PLAN SELECT rid FROM memories_fts_ids WHERE mid = 'm2'")
                .fetch_all(&r.pool)
                .await
                .unwrap();
        assert!(
            plan.iter()
                .any(|(.., d)| d.contains("INDEX") || d.contains("PRIMARY KEY")),
            "indexed lookup, got {plan:?}"
        );
    }

    /// An index built before the map: `ensure_fts_map` builds the map once
    /// and keeps only the newest row per memory. (Driven directly: the
    /// `ensure_fts` reconcile that follows would also prune these rows, since
    /// no `memories` row `m` exists — asserted last.)
    #[tokio::test]
    async fn legacy_index_gets_its_map_built_and_deduplicated() {
        let r = MemoriesRepo::new(crate::db::test_pool().await);
        sqlx::query(
            "CREATE VIRTUAL TABLE memories_fts USING fts5(\
             mid UNINDEXED, ws UNINDEXED, title, body, tokenize='porter unicode61')",
        )
        .execute(&r.pool)
        .await
        .unwrap();
        for body in ["old", "new"] {
            sqlx::query(
                "INSERT INTO memories_fts (mid, ws, title, body) VALUES ('m', 'ws', 'T', ?)",
            )
            .bind(body)
            .execute(&r.pool)
            .await
            .unwrap();
        }
        r.ensure_fts_map().await.unwrap();
        assert_eq!(rows(&r).await, vec![("m".to_string(), "new".to_string())]);
        r.fts_index("m", "ws", "T", "newer").await.unwrap();
        assert_eq!(rows(&r).await, vec![("m".to_string(), "newer".to_string())]);
        // The index mirrors `memories`: a row for a memory that no longer
        // exists is an orphan the startup reconcile removes (map row too).
        assert!(r.ensure_fts().await.unwrap());
        assert_eq!(rows(&r).await, vec![]);
        let mapped: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories_fts_ids")
            .fetch_one(&r.pool)
            .await
            .unwrap();
        assert_eq!(mapped, 0);
    }
}
