//! The memory service: save (with exact-dup NOOP + FTS-on-write), keyword
//! search (FTS5 → LIKE fallback, re-ranked), and the token-budgeted
//! `recall_brief`. Embeddings/vector recall were removed with Vault v3 — the
//! docs home lives in `otto-vault`; this layer is the agents' keyword memory.

use std::sync::atomic::{AtomicU8, Ordering};

use otto_core::Result;
use otto_state::memory::{ListFilter, SearchFilter};
use otto_state::MemoriesRepo;
use sqlx::SqlitePool;

use crate::remote::RemoteClient;
use crate::retrieve::{rerank_score, RerankSignals};
use crate::types::*;

// FTS availability tri-state.
const FTS_UNKNOWN: u8 = 0;
const FTS_YES: u8 = 1;
const FTS_NO: u8 = 2;

pub struct MemoryService {
    repo: MemoriesRepo,
    /// When set, every operation forwards to a shared host Otto instead of the
    /// local SQLite — this is how a team shares one memory across machines.
    remote: Option<RemoteClient>,
    /// When set, saved memories are also written through to an Obsidian-compatible
    /// markdown vault (git-shareable; the file-based path to a shared vault).
    vault: Option<crate::vault::VaultWriter>,
    /// FTS5 availability (lazily probed once): unknown/yes/no.
    fts: AtomicU8,
}

impl MemoryService {
    /// Internal: assemble a service from its parts (all constructors funnel here
    /// so new fields are set in exactly one place).
    fn build(pool: SqlitePool, remote: Option<RemoteClient>) -> Self {
        Self {
            repo: MemoriesRepo::new(pool),
            remote,
            vault: None,
            fts: AtomicU8::new(FTS_UNKNOWN),
        }
    }

    /// Keyword-only service (there is no other kind anymore).
    pub fn new_keyword_only(pool: SqlitePool) -> Self {
        Self::build(pool, None)
    }

    /// Default production service: local SQLite, FTS5 keyword recall.
    pub fn with_defaults(pool: SqlitePool) -> Self {
        Self::build(pool, None)
    }

    /// Shared-backend service: forward all operations to a host Otto's memory API
    /// (one shared memory for the whole team). `pool` is kept only to satisfy the
    /// local repo handle (used by the graph endpoint); reads/writes go remote.
    pub fn remote(pool: SqlitePool, base_url: String, token: String) -> Self {
        Self::build(pool, Some(RemoteClient::new(base_url, token)))
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

    /// Keep the FTS index in sync for a saved/updated memory.
    async fn fts_index_one(&self, m: &Memory) {
        if self.fts_ready().await {
            let _ = self.repo.fts_index(&m.id, &m.workspace_id, &m.title, &m.body).await;
        }
    }

    /// Persist memories, skipping exact duplicates (NOOP returns the existing row),
    /// indexing each new row into FTS on write.
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
            self.fts_index_one(&m).await;
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
        self.fts_index_one(&m).await;
        Ok(m)
    }

    pub async fn forget(&self, ws: &str, id: &str) -> Result<()> {
        if let Some(r) = &self.remote {
            return r.forget(ws, id).await;
        }
        self.repo.forget(ws, id).await?;
        let _ = self.repo.fts_remove(id).await;
        Ok(())
    }

    pub async fn links(&self, ws: &str, id: &str) -> Result<Vec<MemoryLink>> {
        if let Some(r) = &self.remote {
            return r.links(ws, id).await;
        }
        self.repo.links_of(ws, id).await
    }

    // -- collections: docs ingestion + graph import/traversal --

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

    /// Keyword search (FTS5, → LIKE fallback), re-ranked, annotated with a
    /// structured "why selected" reason per hit. The legacy `semantic`/`hybrid`
    /// modes are accepted and execute the same keyword path (tolerant contract
    /// for existing callers; vectors are gone).
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
        // FTS5 for a real query; the LIKE path otherwise (an empty query must
        // still return the filtered set — recall_brief relies on this).
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

        let fused: Vec<(String, f32)> = kw_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), 1.0 / (1.0 + i as f32)))
            .collect();

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
            if !text.trim().is_empty() {
                reasons.push(ContextReason::new("keyword", format!("matched \"{}\"", text.trim()), 1.0));
            }
            if scope_match {
                reasons.push(ContextReason::new("scope", "same story", 0.15));
            }
            if reasons.is_empty() {
                reasons.push(ContextReason::new("keyword", "ranked by relevance", base));
            }
            let why: Vec<String> = reasons.iter().map(|r| r.detail.clone()).collect();

            hits.push(MemoryHit { memory: m, score, why, reasons });
            if hits.len() >= limit * 3 {
                break;
            }
        }
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        // Bump access counts in the BACKGROUND — it's non-critical and a write,
        // so it must never make a search wait on the SQLite write lock.
        let ids: Vec<String> = hits.iter().map(|h| h.memory.id.clone()).collect();
        if !ids.is_empty() {
            let repo = self.repo.clone();
            let ws_owned = ws.to_string();
            tokio::spawn(async move {
                let _ = repo.bump_access(&ws_owned, &ids).await;
            });
        }
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
                mode: SearchMode::Keyword,
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

fn est_tokens(s: &str) -> usize {
    (s.split_whitespace().count() * 4) / 3 + 1
}

/// Defang untrusted-derived text so role markers / code fences can't act as
/// instructions when the brief is composed into a prompt.
fn fence_inline(s: &str) -> String {
    s.replace('`', "ʼ").replace('\n', " ")
}
