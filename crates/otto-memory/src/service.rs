//! The memory service: save (with exact-dup NOOP + embed-on-write), hybrid
//! search (keyword ⊕ vector, fused by RRF, re-ranked), and the token-budgeted
//! `recall_brief`.

use std::sync::Arc;

use otto_core::Result;
use otto_state::memory::{ListFilter, SearchFilter};
use otto_state::MemoriesRepo;
use sqlx::SqlitePool;

use crate::embed::{Embedder, StubEmbedder};
use crate::index::{BruteForceIndex, VectorIndex};
use crate::retrieve::{rerank_score, rrf_fuse, RerankSignals};
use crate::types::*;

const STUB_DIM: usize = 256;

pub struct MemoryService {
    repo: MemoriesRepo,
    embedder: Option<Arc<dyn Embedder>>,
    index: Option<Arc<dyn VectorIndex>>,
}

impl MemoryService {
    pub fn new(
        pool: SqlitePool,
        embedder: Arc<dyn Embedder>,
        index: Arc<dyn VectorIndex>,
    ) -> Self {
        Self {
            repo: MemoriesRepo::new(pool),
            embedder: Some(embedder),
            index: Some(index),
        }
    }

    /// Build a service from any embedder, wiring a brute-force index keyed to the
    /// embedder's model id (one-liner for real local/remote embedders).
    pub fn with_embedder(pool: SqlitePool, embedder: Arc<dyn Embedder>) -> Self {
        let index: Arc<dyn VectorIndex> =
            Arc::new(BruteForceIndex::new(pool.clone(), embedder.model_id().to_string()));
        Self::new(pool, embedder, index)
    }

    /// Keyword-only service (no embeddings) — used where vectors aren't wanted.
    pub fn new_keyword_only(pool: SqlitePool) -> Self {
        Self {
            repo: MemoriesRepo::new(pool),
            embedder: None,
            index: None,
        }
    }

    /// Default production service: stub embedder + brute-force index. Real
    /// embedders (fastembed/OpenAI/Voyage) are feature-gated and swap in here.
    pub fn with_defaults(pool: SqlitePool) -> Self {
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder::new(STUB_DIM));
        let index: Arc<dyn VectorIndex> =
            Arc::new(BruteForceIndex::new(pool.clone(), "stub-v1".into()));
        Self::new(pool, embedder, index)
    }

    pub fn repo(&self) -> &MemoriesRepo {
        &self.repo
    }

    async fn embed_one(&self, m: &Memory) {
        if let Some(e) = &self.embedder {
            let text = format!("{}\n{}", m.title, m.body);
            if let Ok(v) = e.embed(&[text]).await {
                if let Some(v0) = v.into_iter().next() {
                    let _ = self.repo.put_vector(&m.id, e.model_id(), e.dim(), &v0).await;
                }
            }
        }
    }

    /// Persist memories, skipping exact duplicates (NOOP returns the existing row),
    /// and embedding each new row on write.
    pub async fn save(&self, ws: &str, by: &str, items: Vec<NewMemory>) -> Result<Vec<Memory>> {
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
            out.push(m);
        }
        Ok(out)
    }

    pub async fn get(&self, ws: &str, id: &str) -> Result<Memory> {
        self.repo.get(ws, id).await
    }

    pub async fn list(&self, ws: &str, f: ListFilter) -> Result<Vec<Memory>> {
        self.repo.list(ws, &f).await
    }

    pub async fn update(&self, ws: &str, id: &str, p: MemoryPatch) -> Result<Memory> {
        let m = self.repo.update(ws, id, p).await?;
        self.embed_one(&m).await;
        Ok(m)
    }

    pub async fn forget(&self, ws: &str, id: &str) -> Result<()> {
        self.repo.forget(ws, id).await
    }

    pub async fn links(&self, ws: &str, id: &str) -> Result<Vec<MemoryLink>> {
        self.repo.links_of(ws, id).await
    }

    /// Hybrid search: keyword (LIKE) ⊕ vector KNN, fused by RRF, then re-ranked.
    pub async fn search(&self, ws: &str, q: MemoryQuery) -> Result<Vec<MemoryHit>> {
        let limit = if q.k == 0 { 20 } else { q.k };
        let text = q.text.clone().unwrap_or_default();

        let kf = SearchFilter {
            collection: q.collection.clone(),
            story_id: q.story_id.clone(),
            include_inactive: q.include_inactive,
            limit: (limit * 4) as i64,
        };
        let kw = self.repo.search_keyword(ws, &text, &kf).await?;
        let kw_ids: Vec<String> = kw.iter().map(|(m, _)| m.id.clone()).collect();

        let mut sem_ids: Vec<String> = Vec::new();
        if q.mode != SearchMode::Keyword && !text.is_empty() {
            if let (Some(e), Some(idx)) = (&self.embedder, &self.index) {
                if let Ok(qv) = e.embed(std::slice::from_ref(&text)).await {
                    if let Some(v0) = qv.into_iter().next() {
                        sem_ids = idx
                            .knn(ws, &v0, limit * 4)
                            .await?
                            .into_iter()
                            .map(|(id, _)| id)
                            .collect();
                    }
                }
            }
        }

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
            hits.push(MemoryHit {
                memory: m,
                score,
                why: vec![],
            });
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
