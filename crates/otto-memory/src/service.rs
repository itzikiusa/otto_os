//! The memory service: save (with exact-dup NOOP + embed-on-write), hybrid
//! search (keyword ⊕ vector, fused by RRF, re-ranked), and the token-budgeted
//! `recall_brief`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use otto_core::Result;
use otto_state::memory::{ListFilter, SearchFilter};
use otto_state::{CodeIndexRepo, MemoriesRepo};
use sqlx::SqlitePool;

use crate::backends::{QdrantIndex, SurrealClient};
use crate::embed::{Embedder, LocalCodeEmbedder};
use crate::index::{HnswIndex, VectorIndex};
use crate::remote::RemoteClient;
use crate::retrieve::{rerank_score, rrf_fuse, RerankSignals};
use crate::types::*;

/// Per-workspace remote backends (Vault v2 layering). Absent → the workspace
/// uses the local SQLite + HNSW path.
#[derive(Default, Clone)]
struct WsBackends {
    /// Remote vector engine (used for the semantic branch + write-through).
    qdrant: Option<Arc<QdrantIndex>>,
    /// Remote graph engine (code graph is mirrored here for traversal).
    surreal: Option<Arc<SurrealClient>>,
}

// FTS availability tri-state.
const FTS_UNKNOWN: u8 = 0;
const FTS_YES: u8 = 1;
const FTS_NO: u8 = 2;
/// Batch size when re-embedding memories so a remote provider round-trips are
/// amortized instead of one HTTP call per memory.
const REINDEX_BATCH: usize = 64;

pub struct MemoryService {
    repo: MemoriesRepo,
    /// Code-intelligence store (symbol index + dependency graph + repo state).
    code: CodeIndexRepo,
    /// Interior-mutable so the active embedder/index can be swapped at runtime
    /// (the `PUT /memory/embedder` endpoint) without rebuilding the whole
    /// service. The Arc is cloned out under a short read lock before any await.
    embedder: RwLock<Option<Arc<dyn Embedder>>>,
    index: RwLock<Option<Arc<dyn VectorIndex>>>,
    /// When set, every operation forwards to a shared host Otto instead of the
    /// local SQLite — this is how a team shares one memory across machines.
    remote: Option<RemoteClient>,
    /// When set, saved memories are also written through to an Obsidian-compatible
    /// markdown vault (git-shareable; the file-based path to a shared vault).
    vault: Option<crate::vault::VaultWriter>,
    /// Per-workspace remote backends (Qdrant vector / SurrealDB graph). Loaded
    /// from config at boot + on change; empty → local-only.
    backends: RwLock<HashMap<String, WsBackends>>,
    /// FTS5 availability (lazily probed once): unknown/yes/no.
    fts: AtomicU8,
}

impl MemoryService {
    /// Internal: assemble a service from its parts (all constructors funnel here
    /// so new fields are set in exactly one place).
    fn build(
        pool: SqlitePool,
        embedder: Option<Arc<dyn Embedder>>,
        index: Option<Arc<dyn VectorIndex>>,
        remote: Option<RemoteClient>,
    ) -> Self {
        Self {
            repo: MemoriesRepo::new(pool.clone()),
            code: CodeIndexRepo::new(pool),
            embedder: RwLock::new(embedder),
            index: RwLock::new(index),
            remote,
            vault: None,
            backends: RwLock::new(HashMap::new()),
            fts: AtomicU8::new(FTS_UNKNOWN),
        }
    }

    pub fn new(
        pool: SqlitePool,
        embedder: Arc<dyn Embedder>,
        index: Arc<dyn VectorIndex>,
    ) -> Self {
        Self::build(pool, Some(embedder), Some(index), None)
    }

    /// Build a service from any embedder, wiring an HNSW index keyed to the
    /// embedder's model id (one-liner for real local/remote embedders).
    pub fn with_embedder(pool: SqlitePool, embedder: Arc<dyn Embedder>) -> Self {
        let index: Arc<dyn VectorIndex> =
            Arc::new(HnswIndex::new(pool.clone(), embedder.model_id().to_string()));
        Self::new(pool, embedder, index)
    }

    /// Keyword-only service (no embeddings) — used where vectors aren't wanted.
    pub fn new_keyword_only(pool: SqlitePool) -> Self {
        Self::build(pool, None, None, None)
    }

    /// Default production service (Vault v2): the code-aware local embedder + an
    /// HNSW ANN index (exact below threshold). Real neural embedders
    /// (Ollama/OpenAI/Voyage) swap in via `set_embedder`.
    pub fn with_defaults(pool: SqlitePool) -> Self {
        let embedder: Arc<dyn Embedder> = Arc::new(LocalCodeEmbedder::default());
        let index: Arc<dyn VectorIndex> =
            Arc::new(HnswIndex::new(pool.clone(), embedder.model_id().to_string()));
        Self::new(pool, embedder, index)
    }

    /// Shared-backend service: forward all operations to a host Otto's memory API
    /// (one shared memory for the whole team). `pool` is kept only to satisfy the
    /// local repo handle (used by the graph endpoint); reads/writes go remote.
    pub fn remote(pool: SqlitePool, base_url: String, token: String) -> Self {
        Self::build(pool, None, None, Some(RemoteClient::new(base_url, token)))
    }

    // -- runtime embedder management (settings/keychain-driven) --------------

    /// Clone the active embedder out from under the read lock (drop the guard
    /// before any await).
    fn current_embedder(&self) -> Option<Arc<dyn Embedder>> {
        self.embedder.read().ok().and_then(|g| g.clone())
    }

    /// Clone the active index out from under the read lock.
    fn current_index(&self) -> Option<Arc<dyn VectorIndex>> {
        self.index.read().ok().and_then(|g| g.clone())
    }

    /// Swap the active embedder at runtime (e.g. from `PUT /memory/embedder`).
    /// The vector index is rebuilt to match the embedder's `model_id` so KNN
    /// only scores vectors produced by the active model. `None` disables vector
    /// search (keyword-only). Existing vectors at other models are simply
    /// ignored until a `reindex` re-embeds them under the new model.
    pub fn set_embedder(&self, embedder: Option<Arc<dyn Embedder>>) {
        let index: Option<Arc<dyn VectorIndex>> = embedder.as_ref().map(|e| {
            Arc::new(HnswIndex::new(
                self.repo.pool().clone(),
                e.model_id().to_string(),
            )) as Arc<dyn VectorIndex>
        });
        if let Ok(mut g) = self.embedder.write() {
            *g = embedder;
        }
        if let Ok(mut g) = self.index.write() {
            *g = index;
        }
    }

    /// `(model_id, dim)` of the active embedder, or `None` when keyword-only.
    pub fn embedder_status(&self) -> Option<(String, usize)> {
        self.embedder
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|e| (e.model_id().to_string(), e.dim())))
    }

    /// Re-embed a workspace's memories under the active embedder, skipping rows
    /// already embedded at the current model (idempotent). Batched to amortize
    /// remote round-trips. Returns the number of memories (re)embedded. No-op
    /// (returns 0) when keyword-only or when forwarding to a remote backend.
    pub async fn reindex(&self, ws: &str) -> Result<usize> {
        if self.remote.is_some() {
            return Ok(0);
        }
        let Some(e) = self.current_embedder() else {
            return Ok(0);
        };
        let model = e.model_id().to_string();
        // Memories already embedded at the active model — skip them.
        let already: std::collections::HashSet<String> = self
            .repo
            .all_vectors(ws, &model)
            .await?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let filter = ListFilter {
            collection: None,
            kind: None,
            story_id: None,
            tag: None,
            include_inactive: true,
            limit: 100_000,
            viewer: None,
        };
        let memories = self.repo.list(ws, &filter).await?;
        let pending: Vec<&Memory> = memories.iter().filter(|m| !already.contains(&m.id)).collect();
        let mut count = 0usize;
        for chunk in pending.chunks(REINDEX_BATCH) {
            let texts: Vec<String> = chunk
                .iter()
                .map(|m| format!("{}\n{}", m.title, m.body))
                .collect();
            if let Ok(vecs) = e.embed(&texts).await {
                for (m, v) in chunk.iter().zip(vecs) {
                    if self
                        .repo
                        .put_vector(&m.id, e.model_id(), e.dim(), &v)
                        .await
                        .is_ok()
                    {
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    /// Enable Obsidian-vault write-through: saved memories are also written as
    /// markdown notes under `root/<workspace>/`.
    pub fn with_vault(mut self, root: impl Into<std::path::PathBuf>) -> Self {
        self.vault = Some(crate::vault::VaultWriter::new(root));
        self
    }

    /// Re-index a (possibly externally edited / git-synced) vault directory into
    /// the store. Returns the number of notes ingested.
    pub async fn reindex_vault(
        &self,
        ws: &str,
        by: &str,
        dir: &std::path::Path,
    ) -> Result<usize> {
        let notes = crate::vault::read_dir_notes(dir)?;
        let n = notes.len();
        self.save(ws, by, notes).await?;
        Ok(n)
    }

    pub fn repo(&self) -> &MemoriesRepo {
        &self.repo
    }

    /// Raw pool access — used by governance operations that need to run SQL
    /// statements not yet exposed on `MemoriesRepo` (e.g. updating
    /// `provenance_json` for in-flight imports).
    pub fn pool(&self) -> &sqlx::SqlitePool {
        self.repo.pool()
    }

    // -- Vault v2 plumbing: FTS availability + per-workspace backends ---------

    /// Probe FTS5 once and cache the result; subsequent calls are a cheap atomic
    /// read. Returns whether FTS5-backed keyword search is available.
    async fn fts_ready(&self) -> bool {
        match self.fts.load(Ordering::Relaxed) {
            FTS_YES => true,
            FTS_NO => false,
            _ => {
                let ok = self.repo.ensure_fts().await.unwrap_or(false);
                self.fts.store(if ok { FTS_YES } else { FTS_NO }, Ordering::Relaxed);
                ok
            }
        }
    }

    /// Clone the workspace's configured remote backends (cheap Arcs).
    fn ws_backends(&self, ws: &str) -> WsBackends {
        self.backends
            .read()
            .ok()
            .and_then(|g| g.get(ws).cloned())
            .unwrap_or_default()
    }

    /// Register (or clear) a workspace's Qdrant vector backend at runtime.
    pub fn set_qdrant(&self, ws: &str, qdrant: Option<Arc<QdrantIndex>>) {
        if let Ok(mut g) = self.backends.write() {
            g.entry(ws.to_string()).or_default().qdrant = qdrant;
        }
    }

    /// Register (or clear) a workspace's SurrealDB graph backend at runtime.
    pub fn set_surreal(&self, ws: &str, surreal: Option<Arc<SurrealClient>>) {
        if let Ok(mut g) = self.backends.write() {
            g.entry(ws.to_string()).or_default().surreal = surreal;
        }
    }

    /// Access the code-intelligence repo (symbols + graph + repo state).
    pub fn code(&self) -> &CodeIndexRepo {
        &self.code
    }

    async fn embed_one(&self, m: &Memory) {
        if let Some(e) = self.current_embedder() {
            let text = format!("{}\n{}", m.title, m.body);
            if let Ok(v) = e.embed(&[text]).await {
                if let Some(v0) = v.into_iter().next() {
                    let _ = self.repo.put_vector(&m.id, e.model_id(), e.dim(), &v0).await;
                    // Write-through to the workspace's remote vector layer.
                    if let Some(q) = self.ws_backends(&m.workspace_id).qdrant {
                        let _ = q.ensure_collection(e.dim()).await;
                        let _ = q.upsert(&m.id, &v0).await;
                    }
                }
            }
        }
        // Keep the FTS index in sync.
        if self.fts_ready().await {
            let _ = self.repo.fts_index(&m.id, &m.workspace_id, &m.title, &m.body).await;
        }
    }

    /// Persist memories, skipping exact duplicates (NOOP returns the existing row),
    /// and embedding each new row on write.
    pub async fn save(&self, ws: &str, by: &str, items: Vec<NewMemory>) -> Result<Vec<Memory>> {
        if let Some(r) = &self.remote {
            return r.save(ws, items).await;
        }
        let mut out = Vec::with_capacity(items.len());
        for nm in items {
            let hash = MemoriesRepo::content_hash(&nm.body);
            if let Some(ex) = self
                .repo
                .find_by_hash(ws, &nm.collection, nm.scope, nm.story_id.as_deref(), &hash)
                .await?
            {
                out.push(ex);
                continue;
            }
            let m = self.repo.create(ws, by, nm).await?;
            self.embed_one(&m).await;
            if let Some(v) = &self.vault {
                let _ = v.write(ws, &m, &[]);
            }
            out.push(m);
        }
        Ok(out)
    }

    pub async fn get(&self, ws: &str, id: &str) -> Result<Memory> {
        if let Some(r) = &self.remote {
            return r.get(ws, id).await;
        }
        self.repo.get(ws, id).await
    }

    pub async fn list(&self, ws: &str, f: ListFilter) -> Result<Vec<Memory>> {
        if let Some(r) = &self.remote {
            return r.list(ws, &f).await;
        }
        self.repo.list(ws, &f).await
    }

    pub async fn update(&self, ws: &str, id: &str, p: MemoryPatch) -> Result<Memory> {
        if let Some(r) = &self.remote {
            return r.update(ws, id, &p).await;
        }
        let m = self.repo.update(ws, id, p).await?;
        self.embed_one(&m).await;
        Ok(m)
    }

    pub async fn forget(&self, ws: &str, id: &str) -> Result<()> {
        if let Some(r) = &self.remote {
            return r.forget(ws, id).await;
        }
        self.repo.forget(ws, id).await?;
        let _ = self.repo.fts_remove(id).await;
        if let Some(q) = self.ws_backends(ws).qdrant {
            let _ = q.delete(id).await;
        }
        Ok(())
    }

    pub async fn links(&self, ws: &str, id: &str) -> Result<Vec<MemoryLink>> {
        if let Some(r) = &self.remote {
            return r.links(ws, id).await;
        }
        self.repo.links_of(ws, id).await
    }

    // -- collections: code/docs ingestion + graph import/traversal --

    /// Chunk text into a collection (e.g. `code`/`docs`) and store as `chunk`
    /// records. Returns the number of chunks created.
    pub async fn ingest_text(
        &self,
        ws: &str,
        by: &str,
        collection: &str,
        path: &str,
        content: &str,
    ) -> Result<usize> {
        let chunks = crate::ingest::chunk_text(collection, path, content, 40, 8);
        let n = chunks.len();
        self.save(ws, by, chunks).await?;
        Ok(n)
    }

    /// Import a graphify `graph.json`: nodes → `entity` memories, edges → links
    /// (with graphify's certainty tag). Runs on the store-owning instance.
    pub async fn import_graph(
        &self,
        ws: &str,
        by: &str,
        collection: &str,
        g: crate::ingest::GraphifyGraph,
    ) -> Result<crate::ingest::ImportStats> {
        if self.remote.is_some() {
            return Err(otto_core::Error::Invalid(
                "graph import must run on the memory host".into(),
            ));
        }
        let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for n in &g.nodes {
            let created = self.save(ws, by, vec![crate::ingest::node_to_memory(collection, n)]).await?;
            if let Some(m) = created.into_iter().next() {
                map.insert(n.id.clone(), m.id);
            }
        }
        let mut edges = 0;
        for e in &g.edges {
            if let (Some(s), Some(t)) = (map.get(&e.source), map.get(&e.target)) {
                self.repo
                    .link(s, t, e.rel.as_deref().unwrap_or("relates_to"), 1.0, e.certainty.as_deref())
                    .await?;
                edges += 1;
            }
        }
        Ok(crate::ingest::ImportStats {
            nodes: map.len(),
            edges,
        })
    }

    /// An entity's immediate neighborhood: its links + the memories they connect.
    pub async fn entity_graph(&self, ws: &str, id: &str) -> Result<(Vec<MemoryLink>, Vec<Memory>)> {
        let links = self.repo.links_of(ws, id).await?;
        let mut neighbors = Vec::new();
        for l in &links {
            let other = if l.src_id == id { &l.dst_id } else { &l.src_id };
            if let Ok(m) = self.repo.get(ws, other).await {
                neighbors.push(m);
            }
        }
        Ok((links, neighbors))
    }

    /// Hybrid search: keyword (FTS5, → LIKE fallback) ⊕ vector ANN (local HNSW or
    /// the workspace's Qdrant), fused by RRF, re-ranked, and annotated with a
    /// structured "why selected" reason per hit (Vault v2 explainability).
    pub async fn search(&self, ws: &str, q: MemoryQuery) -> Result<Vec<MemoryHit>> {
        if let Some(r) = &self.remote {
            return r.search(ws, &q).await;
        }
        let limit = if q.k == 0 { 20 } else { q.k };
        let text = q.text.clone().unwrap_or_default();

        let kf = SearchFilter {
            collection: q.collection.clone(),
            story_id: q.story_id.clone(),
            include_inactive: q.include_inactive,
            limit: (limit * 4) as i64,
        };
        // Keyword branch: FTS5 for a real query; the LIKE path otherwise (an empty
        // query must still return the filtered set — recall_brief relies on this).
        let kw = if !text.trim().is_empty() && self.fts_ready().await {
            let fts = self.repo.search_fts(ws, &text, &kf).await?;
            if fts.is_empty() {
                // bm25 can miss on stemming/tokenization edge cases — fall back to LIKE.
                self.repo.search_keyword(ws, &text, &kf).await?
            } else {
                fts
            }
        } else {
            self.repo.search_keyword(ws, &text, &kf).await?
        };
        let kw_ids: Vec<String> = kw.iter().map(|(m, _)| m.id.clone()).collect();
        let kw_set: std::collections::HashSet<&String> = kw_ids.iter().collect();

        // Semantic branch: the workspace's Qdrant vector layer if configured,
        // else the local HNSW/exact index.
        let mut sem: Vec<(String, f32)> = Vec::new();
        if q.mode != SearchMode::Keyword && !text.is_empty() {
            let backend_index: Option<Arc<dyn VectorIndex>> = self
                .ws_backends(ws)
                .qdrant
                .map(|q| q as Arc<dyn VectorIndex>)
                .or_else(|| self.current_index());
            if let (Some(e), Some(idx)) = (self.current_embedder(), backend_index) {
                if let Ok(qv) = e.embed(std::slice::from_ref(&text)).await {
                    if let Some(v0) = qv.into_iter().next() {
                        sem = idx.knn(ws, &v0, limit * 4).await.unwrap_or_default();
                    }
                }
            }
        }
        let sem_ids: Vec<String> = sem.iter().map(|(id, _)| id.clone()).collect();
        let sem_score: std::collections::HashMap<&String, f32> =
            sem.iter().map(|(id, s)| (id, *s)).collect();

        let fused: Vec<(String, f32)> = match q.mode {
            SearchMode::Keyword => kw_ids
                .iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), 1.0 / (1.0 + i as f32)))
                .collect(),
            SearchMode::Semantic => sem_ids
                .iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), 1.0 / (1.0 + i as f32)))
                .collect(),
            SearchMode::Hybrid => rrf_fuse(&kw_ids, &sem_ids, 60.0),
        };

        let mut hits: Vec<MemoryHit> = Vec::new();
        for (id, base) in fused.into_iter() {
            let Ok(m) = self.repo.get(ws, &id).await else {
                continue;
            };
            if !q.include_inactive && !m.active {
                continue;
            }
            // Sharing: hide other users' private memories.
            if let Some(viewer) = &q.viewer {
                if m.visibility == "private" && &m.created_by != viewer {
                    continue;
                }
            }
            if let Some(sid) = &q.story_id {
                if m.story_id.as_deref() != Some(sid.as_str()) {
                    continue;
                }
            }
            if let Some(c) = &q.collection {
                if &m.collection != c {
                    continue;
                }
            }
            if !q.kinds.is_empty() && !q.kinds.contains(&m.kind) {
                continue;
            }
            let scope_match = q.story_id.is_some() && q.story_id.as_deref() == m.story_id.as_deref();
            let sig = RerankSignals {
                recency_days: 0.0,
                access_count: m.access_count,
                confidence: m.confidence,
                salience: m.salience,
                scope_match,
            };
            let score = rerank_score(base, &sig, q.recency_half_life_days.unwrap_or(30.0));

            // Explainability: why did this surface?
            let mut reasons: Vec<ContextReason> = Vec::new();
            if let Some(sim) = sem_score.get(&id) {
                reasons.push(ContextReason::new("vector", format!("semantic similarity {sim:.2}"), *sim));
            }
            if kw_set.contains(&id) {
                reasons.push(ContextReason::new("keyword", format!("matched \"{}\"", text.trim()), 1.0));
            }
            if scope_match {
                reasons.push(ContextReason::new("scope", "same story", 0.15));
            }
            if reasons.is_empty() {
                reasons.push(ContextReason::new("hybrid", "ranked by combined relevance", base));
            }
            let why: Vec<String> = reasons.iter().map(|r| r.detail.clone()).collect();

            hits.push(MemoryHit { memory: m, score, why, reasons });
            if hits.len() >= limit * 3 {
                break;
            }
        }
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        let ids: Vec<String> = hits.iter().map(|h| h.memory.id.clone()).collect();
        let _ = self.repo.bump_access(ws, &ids).await;
        Ok(hits)
    }

    /// Assemble a compact, token-budgeted background brief for a story.
    pub async fn recall_brief(&self, ws: &str, story: &str, opts: RecallOpts) -> Result<RecallBrief> {
        if let Some(r) = &self.remote {
            return r.recall_brief(ws, story, &opts).await;
        }
        let groups: &[(&str, &[&str])] = &[
            ("Constraints & Requirements", &[kind::CONSTRAINT, kind::REQUIREMENT]),
            ("Decisions", &[kind::DECISION]),
            ("Key Facts", &[kind::FACT]),
            ("Answered Questions", &[kind::QA]),
            ("Learnings", &[kind::LEARNING]),
            ("Background", &[kind::SUMMARY, kind::SNAPSHOT]),
        ];
        let total = if opts.token_budget == 0 { 2000 } else { opts.token_budget };
        let mut budget = total;
        let mut sections = Vec::new();
        let mut used = Vec::new();
        for (heading, kinds) in groups {
            let q = MemoryQuery {
                text: opts.focus.clone(),
                story_id: Some(story.to_string()),
                kinds: kinds.iter().map(|s| s.to_string()).collect(),
                k: 8,
                mode: SearchMode::Hybrid,
                viewer: opts.viewer.clone(),
                ..Default::default()
            };
            let hits = self.search(ws, q).await?;
            let mut body = String::new();
            let mut refs = Vec::new();
            for h in hits {
                let cost = est_tokens(&h.memory.body);
                if cost > budget {
                    continue;
                }
                budget -= cost;
                body.push_str(&format!("- {}\n", fence_inline(&h.memory.body)));
                refs.extend(h.memory.refs.clone());
                used.push(h.memory.id);
            }
            if !body.is_empty() {
                sections.push(BriefSection {
                    heading: heading.to_string(),
                    body_md: body,
                    refs,
                });
            }
        }
        Ok(RecallBrief {
            story_id: story.to_string(),
            token_estimate: total.saturating_sub(budget),
            sections,
            used,
        })
    }
}

// ===========================================================================
// Vault v2 — code intelligence: repo indexing, symbol/graph queries, the Repo
// Brain context assembly, and docs linked into the dependency graph.
// ===========================================================================

/// Cap on embedded code chunks per index pass (a huge repo can't run away).
const MAX_CODE_CHUNKS: usize = 6000;

/// Max nodes returned by the graph endpoints — keeps the force-graph view smooth
/// on large repos (the UI also offers filters). Most-connected nodes are kept.
const GRAPH_NODE_CAP: usize = 800;

/// Pick the `cap` highest-degree node ids from an edge list. Orphans (degree 0)
/// are included only to fill remaining slots after connected nodes.
fn top_by_degree(
    node_ids: impl Iterator<Item = String>,
    edges: impl Iterator<Item = (String, String)>,
    cap: usize,
) -> std::collections::HashSet<String> {
    let mut degree: HashMap<String, usize> = HashMap::new();
    for id in node_ids {
        degree.entry(id).or_insert(0);
    }
    for (s, d) in edges {
        if let Some(v) = degree.get_mut(&s) {
            *v += 1;
        }
        if let Some(v) = degree.get_mut(&d) {
            *v += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = degree.into_iter().collect();
    ranked.sort_by_key(|x| std::cmp::Reverse(x.1));
    ranked.into_iter().take(cap).map(|(id, _)| id).collect()
}

impl MemoryService {
    /// Index a repository on disk into the Vault: tree-sitter symbol extraction
    /// with dependency-graph heuristics (HTTP/DB/import/call edges), embeddings
    /// of the source files into the `code` collection, and a rolled-up index
    /// state. Mirrors the graph to SurrealDB when the workspace has a graph backend.
    pub async fn index_repo(
        &self,
        ws: &str,
        by: &str,
        root: &std::path::Path,
        name: Option<&str>,
    ) -> Result<IndexResult> {
        if self.remote.is_some() {
            return Err(otto_core::Error::Invalid(
                "repo indexing must run on the memory host".into(),
            ));
        }
        let root = root
            .canonicalize()
            .map_err(|e| otto_core::Error::Invalid(format!("repo path: {e}")))?;
        let root_str = root.to_string_lossy().to_string();
        let display_name = name
            .map(|s| s.to_string())
            .or_else(|| root.file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_else(|| root_str.clone());
        let repo_id = self.code.upsert_repo(ws, &root_str, &display_name).await?;
        self.code
            .set_repo_state(&repo_id, "indexing", None, None, None)
            .await?;
        // Clean re-scan: drop prior symbols + graph for this repo.
        self.code.clear_repo(&repo_id).await?;

        // Scan off the async runtime (CPU-bound parse + walk).
        let scan_root = root.clone();
        let scan = tokio::task::spawn_blocking(move || {
            crate::code_scan::scan_repo(&scan_root, &crate::code_scan::ScanOptions::default())
        })
        .await
        .map_err(|e| otto_core::Error::Internal(format!("scan join: {e}")))?;

        // Symbols.
        let syms: Vec<otto_state::CodeSymbol> = scan
            .symbols
            .iter()
            .map(|s| otto_state::CodeSymbol {
                id: String::new(),
                workspace_id: ws.to_string(),
                repo_id: repo_id.clone(),
                name: s.name.clone(),
                kind: s.kind.clone(),
                lang: s.lang.clone(),
                file: s.file.clone(),
                line: s.line as i64,
                signature: s.signature.clone(),
            })
            .collect();
        let symbol_count = self.code.insert_symbols(ws, &repo_id, &syms).await?;

        // Graph: upsert nodes (natural key → id), then edges.
        let mut key_to_id: HashMap<crate::code_scan::NodeKey, String> = HashMap::new();
        for n in &scan.nodes {
            let id = self
                .code
                .upsert_node(
                    ws,
                    &otto_state::NewNode {
                        repo_id: Some(repo_id.clone()),
                        kind: n.key.kind.clone(),
                        key: n.key.key.clone(),
                        label: n.label.clone(),
                        file: n.file.clone(),
                        line: n.line.map(|l| l as i64),
                        meta_json: n.meta_json.clone(),
                    },
                )
                .await?;
            key_to_id.insert(n.key.clone(), id);
        }
        let mut edge_count = 0usize;
        for e in &scan.edges {
            if let (Some(s), Some(d)) = (key_to_id.get(&e.src), key_to_id.get(&e.dst)) {
                self.code
                    .upsert_edge(
                        ws,
                        &otto_state::NewEdge {
                            repo_id: Some(repo_id.clone()),
                            src_id: s.clone(),
                            dst_id: d.clone(),
                            rel: e.rel.clone(),
                            detail: e.detail.clone(),
                            weight: 1.0,
                            file: e.file.clone(),
                            line: e.line.map(|l| l as i64),
                        },
                    )
                    .await?;
                edge_count += 1;
            }
        }

        // Embed source files into the `code` collection (bounded).
        let mut chunk_count = 0usize;
        for (path, content) in &scan.texts {
            if chunk_count >= MAX_CODE_CHUNKS {
                break;
            }
            chunk_count += self
                .ingest_text(ws, by, "code", path, content)
                .await
                .unwrap_or(0);
        }

        // Mirror the graph to SurrealDB if configured (rich remote traversal).
        if let Some(sr) = self.ws_backends(ws).surreal {
            if let Ok(g) = self.code.graph(ws, Some(&repo_id)).await {
                let _ = sr.mirror_graph(&g.nodes, &g.edges).await;
            }
        }

        let head = crate::git_context::repo_context(&root, 1).await.head;
        self.code
            .set_repo_state(
                &repo_id,
                "ready",
                None,
                head.as_deref(),
                Some((
                    scan.files as i64,
                    symbol_count as i64,
                    edge_count as i64,
                    chunk_count as i64,
                )),
            )
            .await?;

        Ok(IndexResult {
            repo_id,
            files: scan.files,
            symbols: symbol_count,
            edges: edge_count,
            chunks: chunk_count,
        })
    }

    pub async fn list_repos(&self, ws: &str) -> Result<Vec<otto_state::CodeRepo>> {
        self.code.list_repos(ws).await
    }

    /// The unified Vault graph (knowledge memories + their links, plus the code
    /// dependency graph) for the Obsidian-style full graph view. When `repo_id`
    /// is given, the code side is scoped to that repo; the knowledge side is the
    /// whole workspace.
    pub async fn full_graph(&self, ws: &str, repo_id: Option<&str>) -> Result<FullGraph> {
        let mut nodes: Vec<FullGraphNode> = Vec::new();
        let mut edges: Vec<FullGraphEdge> = Vec::new();

        // Knowledge side.
        for (id, label, kind, _collection) in self.repo.graph_nodes(ws, None).await.unwrap_or_default() {
            nodes.push(FullGraphNode { id, label, kind, group: "knowledge".into(), file: None, line: None });
        }
        for l in self.repo.all_links(ws).await.unwrap_or_default() {
            edges.push(FullGraphEdge { src: l.src_id, dst: l.dst_id, rel: l.rel, detail: String::new() });
        }

        // Code side.
        let cg = self.code.graph(ws, repo_id).await.unwrap_or_default();
        for n in cg.nodes {
            nodes.push(FullGraphNode { id: n.id, label: n.label, kind: n.kind, group: "code".into(), file: n.file, line: n.line });
        }
        for e in cg.edges {
            edges.push(FullGraphEdge { src: e.src_id, dst: e.dst_id, rel: e.rel, detail: e.detail });
        }
        // Cap for a responsive force-graph: keep the most-connected nodes.
        if nodes.len() > GRAPH_NODE_CAP {
            let keep = top_by_degree(
                nodes.iter().map(|n| n.id.clone()),
                edges.iter().map(|e| (e.src.clone(), e.dst.clone())),
                GRAPH_NODE_CAP,
            );
            nodes.retain(|n| keep.contains(&n.id));
            edges.retain(|e| keep.contains(&e.src) && keep.contains(&e.dst));
        }
        Ok(FullGraph { nodes, edges })
    }

    pub async fn search_symbols(
        &self,
        ws: &str,
        query: &str,
        repo_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<otto_state::CodeSymbol>> {
        self.code.search_symbols(ws, query, repo_id, limit).await
    }

    pub async fn code_graph(&self, ws: &str, repo_id: Option<&str>) -> Result<otto_state::CodeGraph> {
        let mut g = self.code.graph(ws, repo_id).await?;
        if g.nodes.len() > GRAPH_NODE_CAP {
            let keep = top_by_degree(
                g.nodes.iter().map(|n| n.id.clone()),
                g.edges.iter().map(|e| (e.src_id.clone(), e.dst_id.clone())),
                GRAPH_NODE_CAP,
            );
            g.nodes.retain(|n| keep.contains(&n.id));
            g.edges.retain(|e| keep.contains(&e.src_id) && keep.contains(&e.dst_id));
        }
        Ok(g)
    }

    pub async fn code_neighborhood(
        &self,
        ws: &str,
        node_id: &str,
        depth: usize,
    ) -> Result<otto_state::CodeGraph> {
        self.code.neighborhood(ws, node_id, depth).await
    }

    /// Resolve a graph node id by natural key (kind, key) within a repo.
    pub async fn code_node_id(
        &self,
        ws: &str,
        repo_id: Option<&str>,
        kind: &str,
        key: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .code
            .find_node(ws, repo_id, kind, key)
            .await?
            .map(|n| n.id))
    }

    /// Create/refresh a documentation note (collection `docs`) and link it into
    /// the code graph as a `doc` node with `documents` edges to the given code
    /// nodes (by id). Returns the created memory.
    pub async fn upsert_doc(
        &self,
        ws: &str,
        by: &str,
        repo_id: Option<&str>,
        title: &str,
        body: &str,
        documents: &[String],
    ) -> Result<Memory> {
        let nm = NewMemory {
            collection: "docs".into(),
            record_type: "item".into(),
            scope: Scope::Workspace,
            story_id: None,
            kind: kind::SUMMARY.into(),
            title: title.into(),
            body: body.into(),
            entities: vec![],
            tags: vec!["doc".into()],
            source_kind: source::MANUAL.into(),
            source_ref: None,
            refs: vec![],
            confidence: Some(0.9),
            salience: Some(0.8),
            visibility: "shared".into(),
        };
        let m = self.save(ws, by, vec![nm]).await?.into_iter().next().ok_or_else(|| {
            otto_core::Error::Internal("doc save returned nothing".into())
        })?;
        // Represent the doc in the dependency graph + link it to code nodes.
        let doc_node = self
            .code
            .upsert_node(
                ws,
                &otto_state::NewNode {
                    repo_id: repo_id.map(|s| s.to_string()),
                    kind: "doc".into(),
                    key: m.id.clone(),
                    label: title.to_string(),
                    file: None,
                    line: None,
                    meta_json: "{}".into(),
                },
            )
            .await?;
        for target in documents {
            self.code
                .upsert_edge(
                    ws,
                    &otto_state::NewEdge {
                        repo_id: repo_id.map(|s| s.to_string()),
                        src_id: doc_node.clone(),
                        dst_id: target.clone(),
                        rel: "documents".into(),
                        detail: String::new(),
                        weight: 1.0,
                        file: None,
                        line: None,
                    },
                )
                .await?;
        }
        Ok(m)
    }

    /// Assemble the **Repo Brain** for an agent: relevant knowledge/docs (hybrid
    /// recall), relevant symbols, the dependency neighborhood around the best
    /// match, and git/test context — each annotated with why it was selected,
    /// rendered to a compact markdown block.
    pub async fn repo_brain(
        &self,
        ws: &str,
        focus: &str,
        cwd: Option<&std::path::Path>,
        budget: usize,
    ) -> Result<RepoBrain> {
        let budget = if budget == 0 { 1800 } else { budget };
        let mut sections: Vec<BriefSection> = Vec::new();
        let mut reasons: Vec<ContextReason> = Vec::new();
        let mut used = 0usize;

        // 0) Indexed repos + their key external dependencies (always-on overview).
        // Scope to the repo matching `cwd` when one is given.
        let repos = self.code.list_repos(ws).await.unwrap_or_default();
        if !repos.is_empty() {
            let cwd_str = cwd.map(|p| p.to_string_lossy().to_string());
            let active = cwd_str.as_ref().and_then(|c| {
                repos.iter().find(|r| c == &r.root || c.starts_with(&format!("{}/", r.root)))
            });
            let mut body = String::new();
            for r in &repos {
                let mark = if active.map(|a| a.id == r.id).unwrap_or(false) { " ◀ this session" } else { "" };
                body.push_str(&format!(
                    "- **{}** — {} symbols, {} edges, {} files{}\n",
                    r.name, r.symbols, r.edges, r.files, mark
                ));
            }
            // Key external dependencies (service / db_table nodes) of the active
            // repo — a cheap targeted query, not the full graph (which is huge).
            if let Some(a) = active {
                if let Ok(deps) = self.code.dependency_labels(&a.id).await {
                    let mut services: Vec<String> =
                        deps.into_iter().map(|(label, kind)| format!("{label} ({kind})")).collect();
                    services.sort();
                    services.dedup();
                    if !services.is_empty() {
                        // Cap so the injected brain stays compact (token budget).
                        let total = services.len();
                        services.truncate(24);
                        let more = if total > 24 { format!(" (+{} more)", total - 24) } else { String::new() };
                        body.push_str(&format!("- key dependencies: {}{}\n", services.join(", "), more));
                    }
                }
                reasons.push(ContextReason::new("graph", format!("indexed repo {}", a.name), 0.8));
            }
            sections.push(BriefSection {
                heading: "Indexed repos".into(),
                body_md: body,
                refs: vec![],
            });
        }

        // 1) Relevant knowledge + docs (hybrid recall).
        let hits = self
            .search(
                ws,
                MemoryQuery {
                    text: if focus.is_empty() { None } else { Some(focus.to_string()) },
                    k: 6,
                    mode: SearchMode::Hybrid,
                    ..Default::default()
                },
            )
            .await
            .unwrap_or_default();
        if !hits.is_empty() {
            let mut body = String::new();
            for h in hits.iter().take(6) {
                if used >= budget {
                    break;
                }
                let why = h.why.first().cloned().unwrap_or_default();
                body.push_str(&format!("- {} _( {} )_\n", fence_inline(&h.memory.title), why));
                reasons.extend(h.reasons.clone());
                used += est_tokens(&h.memory.title);
            }
            if !body.is_empty() {
                sections.push(BriefSection {
                    heading: "Relevant knowledge".into(),
                    body_md: body,
                    refs: vec![],
                });
            }
        }

        // 2) Relevant symbols — match any focus term (split the phrase), best
        // (shortest-name) first, deduped by name+file.
        let mut syms: Vec<otto_state::CodeSymbol> = Vec::new();
        let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        let mut query_terms: Vec<String> = focus
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() >= 3)
            .map(|s| s.to_string())
            .collect();
        if !focus.trim().is_empty() {
            query_terms.push(focus.to_string());
        }
        for term in &query_terms {
            for s in self.search_symbols(ws, term, None, 8).await.unwrap_or_default() {
                if seen.insert((s.name.clone(), s.file.clone())) {
                    syms.push(s);
                }
            }
            if syms.len() >= 8 {
                break;
            }
        }
        if !syms.is_empty() {
            let mut body = String::new();
            for s in &syms {
                body.push_str(&format!("- `{}` ({}) — {}:{}\n", s.name, s.kind, s.file, s.line));
            }
            reasons.push(ContextReason::new("symbol", format!("{} matching symbols", syms.len()), 0.7));
            sections.push(BriefSection {
                heading: "Relevant symbols".into(),
                body_md: body,
                refs: vec![],
            });
        }

        // 3) Dependency neighborhood around the best-matching symbol.
        if let Some(best) = syms.first() {
            let key = format!("{}#{}", best.file, best.name);
            if let Some(nid) = self.code_node_id(ws, Some(&best.repo_id), "symbol", &key).await? {
                if let Ok(g) = self.code.neighborhood(ws, &nid, 2).await {
                    if !g.edges.is_empty() {
                        let id_label: HashMap<String, String> =
                            g.nodes.iter().map(|n| (n.id.clone(), n.label.clone())).collect();
                        let mut body = String::new();
                        for e in g.edges.iter().take(20) {
                            let s = id_label.get(&e.src_id).cloned().unwrap_or_default();
                            let d = id_label.get(&e.dst_id).cloned().unwrap_or_default();
                            let det = if e.detail.is_empty() { String::new() } else { format!(" ({})", e.detail) };
                            body.push_str(&format!("- {s} —{}→ {d}{det}\n", e.rel));
                        }
                        reasons.push(ContextReason::new("graph", format!("dependency neighborhood of {}", best.name), 0.6));
                        sections.push(BriefSection {
                            heading: format!("Dependencies of {}", best.name),
                            body_md: body,
                            refs: vec![],
                        });
                    }
                }

                // 4) git + test context for the focus file.
                if let Some(root) = cwd {
                    let gc = crate::git_context::file_context(root, &best.file, 5).await;
                    if !gc.commits.is_empty() || !gc.blame.is_empty() {
                        let mut body = String::new();
                        if let Some(last) = &gc.last_change {
                            body.push_str(&format!("- last changed: {last}\n"));
                        }
                        for c in gc.commits.iter().take(3) {
                            body.push_str(&format!("- {} {} ({})\n", c.hash, fence_inline(&c.subject), c.author));
                        }
                        if let Some(top) = gc.blame.first() {
                            body.push_str(&format!("- top author: {} ({} lines)\n", top.author, top.lines));
                        }
                        reasons.push(ContextReason::new("recent", format!("git history for {}", best.file), 0.5));
                        sections.push(BriefSection {
                            heading: "Recent changes".into(),
                            body_md: body,
                            refs: vec![],
                        });
                    }
                    let tests: Vec<String> = crate::test_map::tests_for_source(&best.file)
                        .into_iter()
                        .filter(|t| root.join(t).exists())
                        .collect();
                    if !tests.is_empty() {
                        reasons.push(ContextReason::new("test", format!("tests for {}", best.file), 0.5));
                        sections.push(BriefSection {
                            heading: "Tests".into(),
                            body_md: tests.iter().map(|t| format!("- {t}\n")).collect(),
                            refs: vec![],
                        });
                    }
                }
            }
        }

        let markdown = render_brain(focus, &sections);
        Ok(RepoBrain {
            focus: focus.to_string(),
            sections,
            reasons,
            token_estimate: used,
            markdown,
        })
    }
}

/// Render the Repo Brain sections into a compact markdown block.
fn render_brain(focus: &str, sections: &[BriefSection]) -> String {
    if sections.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Repo Brain (Vault)\n");
    if !focus.is_empty() {
        out.push_str(&format!("_Focus: {}_\n", fence_inline(focus)));
    }
    for s in sections {
        out.push_str(&format!("\n### {}\n{}", s.heading, s.body_md));
    }
    out.push_str("\n_Source: Otto Vault — items chosen by hybrid recall + the code dependency graph._\n");
    out
}

/// The Repo Brain source consumed by `otto-context`'s spawn hook, so EVERY agent
/// session gets the workspace's repo brain. Synchronous (the hook is sync); it
/// bridges to the async `repo_brain` on the multi-thread daemon runtime. A
/// missing runtime / empty brain yields `None` (nothing injected).
impl otto_context::RepoBrainSource for MemoryService {
    fn brain_markdown(&self, workspace_id: &str, cwd: &str, focus: &str) -> Option<String> {
        let handle = tokio::runtime::Handle::try_current().ok()?;
        let cwd_path = std::path::Path::new(cwd);
        // Spawn injection must never be broken by recall — bridge sync→async on
        // the multi-thread runtime, and catch any panic (returns None → nothing
        // injected) so a bad brain can't abort a session spawn.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tokio::task::block_in_place(|| {
                handle.block_on(self.repo_brain(workspace_id, focus, Some(cwd_path), 1200))
            })
        }));
        let brain = result.ok()?.ok()?;
        let md = brain.markdown.trim().to_string();
        if md.is_empty() {
            None
        } else {
            Some(md)
        }
    }
}

fn est_tokens(s: &str) -> usize {
    (s.split_whitespace().count() * 4) / 3 + 1
}

/// Defang untrusted-derived text so role markers / code fences can't act as
/// instructions when the brief is composed into a prompt.
fn fence_inline(s: &str) -> String {
    s.replace('`', "ʼ").replace('\n', " ")
}
