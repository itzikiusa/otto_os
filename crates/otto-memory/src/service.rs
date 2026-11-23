//! The memory service: save (with exact-dup NOOP + FTS-on-write), keyword
//! search (FTS5 → LIKE fallback, re-ranked), and the token-budgeted
//! `recall_brief`. Embeddings/vector recall were removed with Vault v3 — the
//! docs home lives in `otto-vault`; this layer is the agents' keyword memory.

use std::sync::atomic::{AtomicU8, Ordering};

use otto_core::Result;
use otto_state::memory::{ListFilter, SaveOutcome, SearchFilter};
use otto_state::DbPool;
use otto_state::MemoriesRepo;

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
    /// FTS5 availability (probed once): unknown/yes/no.
    fts: AtomicU8,
    /// Single-flight for the probe: `ensure_fts` reconciles the whole index
    /// under the write lock, so concurrent first callers wait for one run.
    fts_probe: tokio::sync::Mutex<()>,
    /// When the last probe failed TRANSIENTLY (S7-308): searches fall back to
    /// LIKE until [`FTS_RETRY_AFTER`] has passed, then probe again — a busy
    /// boot no longer pins the fallback for the daemon's lifetime.
    fts_failed_at: std::sync::Mutex<Option<std::time::Instant>>,
}

/// Back-off between FTS probes after a transient failure.
const FTS_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

impl MemoryService {
    /// Internal: assemble a service from its parts (all constructors funnel here
    /// so new fields are set in exactly one place).
    fn build(pool: DbPool, remote: Option<RemoteClient>) -> Self {
        Self {
            repo: MemoriesRepo::new(pool),
            remote,
            vault: None,
            fts: AtomicU8::new(FTS_UNKNOWN),
            fts_probe: tokio::sync::Mutex::new(()),
            fts_failed_at: std::sync::Mutex::new(None),
        }
    }

    /// Default production service: local SQLite, FTS5 keyword recall.
    pub fn with_defaults(pool: DbPool) -> Self {
        Self::build(pool, None)
    }

    /// Shared-backend service: forward all operations to a host Otto's memory API
    /// (one shared memory for the whole team). `pool` is kept only to satisfy the
    /// local repo handle (used by the graph endpoint); reads/writes go remote.
    pub fn remote(pool: DbPool, base_url: String, token: String) -> Self {
        Self::build(pool, Some(RemoteClient::new(base_url, token)))
    }

    /// Enable Obsidian-vault write-through: saved shared memories are also written as
    /// markdown notes under `root/<workspace>/`.
    pub fn with_vault(mut self, root: impl Into<std::path::PathBuf>) -> Self {
        self.vault = Some(crate::vault::VaultWriter::new(root));
        self
    }

    /// Re-index a (possibly externally edited / git-synced) vault directory into
    /// the store. Returns the number of notes ingested.
    pub async fn reindex_vault(&self, ws: &str, by: &str, dir: &std::path::Path) -> Result<usize> {
        // A directory walk + a read per note: off the async workers (S9-305).
        let owned = dir.to_path_buf();
        let notes = tokio::task::spawn_blocking(move || crate::vault::read_dir_notes(&owned))
            .await
            .map_err(|e| otto_core::Error::Internal(format!("vault reindex task: {e}")))??;
        let n = notes.len();
        self.save(ws, by, notes).await?;
        Ok(n)
    }

    pub fn repo(&self) -> &MemoriesRepo {
        &self.repo
    }

    pub(crate) fn remote_client(&self) -> Option<&RemoteClient> {
        self.remote.as_ref()
    }

    /// Raw pool access — used by governance operations that need to run SQL
    /// statements not yet exposed on `MemoriesRepo` (e.g. updating
    /// `provenance_json` for in-flight imports).
    pub fn pool(&self) -> &otto_state::DbPool {
        self.repo.pool()
    }

    /// Run the FTS probe + reconcile now (boot background task) so the first
    /// memory call after boot doesn't pay the O(n) reconcile under the write
    /// lock (S7-10). A remote-backed service has no local index to warm.
    pub async fn warm_fts(&self) -> bool {
        if self.remote.is_some() {
            return false;
        }
        self.fts_ready().await
    }

    /// Probe FTS5 once and cache the result; subsequent calls are a cheap atomic
    /// read. Returns whether FTS5-backed keyword search is available.
    async fn fts_ready(&self) -> bool {
        match self.fts.load(Ordering::Relaxed) {
            FTS_YES => true,
            FTS_NO => false,
            _ => {
                let _probe = self.fts_probe.lock().await;
                match self.fts.load(Ordering::Relaxed) {
                    FTS_YES => return true,
                    FTS_NO => return false,
                    _ => {}
                }
                let failed_recently = self
                    .fts_failed_at
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_some_and(|at| at.elapsed() < FTS_RETRY_AFTER);
                if failed_recently {
                    return false;
                }
                match self.repo.ensure_fts().await {
                    Ok(ok) => {
                        self.fts
                            .store(if ok { FTS_YES } else { FTS_NO }, Ordering::Relaxed);
                        ok
                    }
                    Err(e) => {
                        // Transient: stay UNKNOWN (LIKE for now), retry later.
                        tracing::warn!(error = %e, "memory FTS probe failed; retrying later");
                        *self.fts_failed_at.lock().unwrap_or_else(|p| p.into_inner()) =
                            Some(std::time::Instant::now());
                        false
                    }
                }
            }
        }
    }

    /// Persist memories, skipping exact duplicates (NOOP returns the existing row),
    /// indexing each new row into FTS on write.
    pub async fn save(&self, ws: &str, by: &str, items: Vec<NewMemory>) -> Result<Vec<Memory>> {
        if let Some(r) = &self.remote {
            return r.save(ws, items).await;
        }
        Ok(self
            .save_detailed(ws, by, items)
            .await?
            .into_iter()
            .map(|(m, _)| m)
            .collect())
    }

    /// [`Self::save`], reporting per item whether it was created, revived
    /// from an inactive duplicate, or already live. Each item is ONE write
    /// transaction covering the dedup lookup, the row and its FTS entry
    /// ([`MemoriesRepo::save_one`]). Local store only (a remote host reports
    /// no outcomes).
    pub async fn save_detailed(
        &self,
        ws: &str,
        by: &str,
        items: Vec<NewMemory>,
    ) -> Result<Vec<(Memory, SaveOutcome)>> {
        if self.remote.is_some() {
            return Err(otto_core::Error::Invalid(
                "detailed save must run on the memory host".into(),
            ));
        }
        let fts = self.fts_ready().await;
        let mut out = Vec::with_capacity(items.len());
        for nm in items {
            let (m, outcome) = self.repo.save_one(ws, by, nm, fts).await?;
            if m.visibility == "shared" {
                if let Some(v) = &self.vault {
                    // The row is already committed. Report export failure so
                    // callers can retry, including a deduplicated save. Missing
                    // exports are repaired without overwriting external edits.
                    let (writer, workspace, memory) = (v.clone(), ws.to_string(), m.clone());
                    tokio::task::spawn_blocking(move || {
                        writer.write_missing(&workspace, &memory, &[])
                    })
                    .await
                    .map_err(|e| {
                        otto_core::Error::Internal(format!(
                            "memory {} saved; vault export task failed: {e}; retry the same save",
                            m.id
                        ))
                    })?
                    .map_err(|e| {
                        otto_core::Error::Internal(format!(
                            "memory {} saved; vault export failed: {e}; retry the same save",
                            m.id
                        ))
                    })?;
                }
            }
            out.push((m, outcome));
        }
        Ok(out)
    }

    pub async fn get(&self, ws: &str, id: &str) -> Result<Memory> {
        if let Some(r) = &self.remote {
            return r.get(ws, id).await;
        }
        self.repo.get(ws, id).await
    }

    /// Resolve a row using the caller's visibility. Root/internal callers pass
    /// `None`; authenticated non-root callers must pass their actual user id.
    pub async fn get_visible(&self, ws: &str, id: &str, viewer: Option<&str>) -> Result<Memory> {
        let memory = self.get(ws, id).await?;
        if viewer.is_some_and(|viewer| memory.visibility != "shared" && memory.created_by != viewer)
        {
            return Err(otto_core::Error::NotFound("memory".into()));
        }
        Ok(memory)
    }

    pub async fn links_visible(
        &self,
        ws: &str,
        id: &str,
        viewer: Option<&str>,
    ) -> Result<Vec<MemoryLink>> {
        self.get_visible(ws, id, viewer).await?;
        if self.remote.is_none() {
            return self.repo.links_visible(ws, Some(id), viewer).await;
        }
        let mut visible = Vec::new();
        for link in self.links(ws, id).await? {
            let other = if link.src_id == id {
                &link.dst_id
            } else {
                &link.src_id
            };
            match self.get_visible(ws, other, viewer).await {
                Ok(_) => visible.push(link),
                Err(otto_core::Error::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(visible)
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
        let fts = self.fts_ready().await;
        self.repo.update_indexed(ws, id, p, fts).await
    }

    pub async fn forget(&self, ws: &str, id: &str) -> Result<()> {
        if let Some(r) = &self.remote {
            return r.forget(ws, id).await;
        }
        // The FTS index mirrors every memory row (search filters `active`,
        // and `include_inactive` recall still needs the text indexed), so a
        // forget leaves it — removing it here would just be re-added by the
        // startup reconcile.
        self.repo.forget(ws, id).await
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
            let created = self
                .save(ws, by, vec![crate::ingest::node_to_memory(collection, n)])
                .await?;
            if let Some(m) = created.into_iter().next() {
                map.insert(n.id.clone(), m.id);
            }
        }
        let mut edges = 0;
        for e in &g.edges {
            if let (Some(s), Some(t)) = (map.get(&e.source), map.get(&e.target)) {
                self.repo
                    .link(
                        s,
                        t,
                        e.rel.as_deref().unwrap_or("relates_to"),
                        1.0,
                        e.certainty.as_deref(),
                    )
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
        self.entity_graph_visible(ws, id, None).await
    }

    /// Visible memory graph (nodes + edges). Delegates to the remote host when
    /// configured so the graph view never silently splits onto local SQLite.
    pub async fn graph_visible(
        &self,
        ws: &str,
        collection: Option<&str>,
        viewer: Option<&str>,
    ) -> Result<crate::http::GraphData> {
        if let Some(r) = &self.remote {
            return r.graph(ws, collection).await;
        }
        let nodes: Vec<crate::http::GraphNode> = self
            .repo
            .graph_nodes_visible(ws, collection, viewer)
            .await?
            .into_iter()
            .map(|(id, label, kind, collection)| crate::http::GraphNode {
                id,
                label,
                kind,
                collection,
            })
            .collect();
        let ids: std::collections::HashSet<&str> =
            nodes.iter().map(|node| node.id.as_str()).collect();
        let edges = self
            .repo
            .links_visible(ws, None, viewer)
            .await?
            .into_iter()
            .filter(|edge| ids.contains(edge.src_id.as_str()) && ids.contains(edge.dst_id.as_str()))
            .collect();
        Ok(crate::http::GraphData { nodes, edges })
    }

    pub async fn entity_graph_visible(
        &self,
        ws: &str,
        id: &str,
        viewer: Option<&str>,
    ) -> Result<(Vec<MemoryLink>, Vec<Memory>)> {
        let links = self.links_visible(ws, id, viewer).await?;
        let mut neighbors = Vec::new();
        for l in &links {
            let other = if l.src_id == id { &l.dst_id } else { &l.src_id };
            match self.get_visible(ws, other, viewer).await {
                Ok(m) => neighbors.push(m),
                Err(otto_core::Error::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok((links, neighbors))
    }

    /// Keyword search (FTS5, → LIKE fallback), re-ranked, annotated with a
    /// structured "why selected" reason per hit. The legacy `semantic`/`hybrid`
    /// modes are accepted and execute the same keyword path (tolerant contract
    /// for existing callers; vectors are gone).
    pub async fn search(&self, ws: &str, mut q: MemoryQuery) -> Result<Vec<MemoryHit>> {
        q.k = if q.k == 0 { 20 } else { q.k.min(200) };
        if let Some(r) = &self.remote {
            return r.search(ws, &q).await;
        }
        let limit = q.k;
        let text = q.text.clone().unwrap_or_default();

        let has_post_filters = q.scope.is_some() || !q.tags.is_empty() || !q.entities.is_empty();
        let fetch_limit = if has_post_filters {
            (limit * 8).max(200)
        } else {
            limit * 4
        };
        let kf = SearchFilter {
            collection: q.collection.clone(),
            story_id: q.story_id.clone(),
            include_inactive: q.include_inactive,
            limit: fetch_limit as i64,
            kinds: q.kinds.clone(),
            viewer: q.viewer.clone(),
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
        // The search already returned full rows — rank them directly (this used
        // to re-`get` every hit: up to k*4 point queries per search, ~480 per
        // `recall_brief`).
        let mut hits: Vec<MemoryHit> = Vec::new();
        for (i, (m, _)) in kw.into_iter().enumerate() {
            let base = 1.0 / (1.0 + i as f32);
            if !q.include_inactive && !m.active {
                continue;
            }
            // Sharing: hide other users' private memories.
            if let Some(viewer) = &q.viewer {
                if m.visibility != "shared" && &m.created_by != viewer {
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
            if let Some(scope) = q.scope {
                if m.scope != scope {
                    continue;
                }
            }
            if !q.tags.is_empty()
                && !q
                    .tags
                    .iter()
                    .any(|tag| m.tags.iter().any(|mt| mt.eq_ignore_ascii_case(tag)))
            {
                continue;
            }
            if !q.entities.is_empty()
                && !q
                    .entities
                    .iter()
                    .any(|ent| m.entities.iter().any(|me| me.eq_ignore_ascii_case(ent)))
            {
                continue;
            }
            let scope_match =
                q.story_id.is_some() && q.story_id.as_deref() == m.story_id.as_deref();
            let sig = RerankSignals {
                recency_days: recency_days_now(&m.updated_at, &m.created_at),
                access_count: m.access_count,
                confidence: m.confidence,
                salience: m.salience,
                scope_match,
            };
            let score = rerank_score(base, &sig, q.recency_half_life_days.unwrap_or(30.0));

            // Explainability: why did this surface?
            let mut reasons: Vec<ContextReason> = Vec::new();
            if !text.trim().is_empty() {
                reasons.push(ContextReason::new(
                    "keyword",
                    format!("matched \"{}\"", text.trim()),
                    1.0,
                ));
            }
            if scope_match {
                reasons.push(ContextReason::new("scope", "same story", 0.15));
            }
            if reasons.is_empty() {
                reasons.push(ContextReason::new("keyword", "ranked by relevance", base));
            }
            let why: Vec<String> = reasons.iter().map(|r| r.detail.clone()).collect();

            hits.push(MemoryHit {
                memory: m,
                score,
                why,
                reasons,
            });
            if hits.len() >= limit * 3 {
                break;
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
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
    pub async fn recall_brief(
        &self,
        ws: &str,
        story: &str,
        opts: RecallOpts,
    ) -> Result<RecallBrief> {
        if let Some(r) = &self.remote {
            return r.recall_brief(ws, story, &opts).await;
        }
        let groups: &[(&str, &[&str])] = &[
            (
                "Constraints & Requirements",
                &[kind::CONSTRAINT, kind::REQUIREMENT],
            ),
            ("Decisions", &[kind::DECISION]),
            ("Key Facts", &[kind::FACT]),
            ("Answered Questions", &[kind::QA]),
            ("Learnings", &[kind::LEARNING]),
            ("Background", &[kind::SUMMARY, kind::SNAPSHOT]),
        ];
        let total = if opts.token_budget == 0 {
            2000
        } else {
            opts.token_budget
        };
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

    /// Recall active, caller-visible workspace memories matching `query`
    /// (recency-weighted, scope/tag-filtered via [`Self::search`]) and render a
    /// compact, byte-budgeted (`<= 2 KiB` default) `## Relevant Workspace Memory`
    /// markdown section with provenance citations. Returns an empty string when
    /// no memories match (strict no-op).
    pub async fn recall_context_brief(
        &self,
        ws: &str,
        query: MemoryQuery,
        max_bytes: Option<usize>,
    ) -> Result<String> {
        let hits = self.search(ws, query).await?;
        let budget = max_bytes.unwrap_or(crate::retrieve::MAX_CONTEXT_BRIEF_BYTES);
        Ok(crate::retrieve::format_hits_brief(&hits, budget))
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

/// Non-negative elapsed days since `updated_at` (falling back to `created_at`).
/// Returns `0.0` if neither timestamp parses or if the timestamp is in the future.
fn recency_days_now(updated_at: &str, created_at: &str) -> f32 {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    recency_days_at(updated_at, created_at, now_secs)
}

fn recency_days_at(updated_at: &str, created_at: &str, now_secs: f64) -> f32 {
    let Some(ts) =
        parse_rfc3339_unix_secs(updated_at).or_else(|| parse_rfc3339_unix_secs(created_at))
    else {
        return 0.0;
    };
    let elapsed = (now_secs - ts).max(0.0);
    (elapsed / 86_400.0) as f32
}

/// Minimal RFC3339 parser (`YYYY-MM-DD[T| ]HH:MM:SS[.frac](Z|+HH:MM|-HH:MM)`)
/// returning Unix epoch seconds as `f64`. Avoids pulling `chrono` into
/// non-dev dependencies.
fn parse_rfc3339_unix_secs(raw: &str) -> Option<f64> {
    let s = raw.trim();
    let b = s.as_bytes();
    if b.len() < 20 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    if *b.get(4)? != b'-' {
        return None;
    }
    let month: u32 = s.get(5..7)?.parse().ok()?;
    if *b.get(7)? != b'-' {
        return None;
    }
    let day: u32 = s.get(8..10)?.parse().ok()?;
    let sep = *b.get(10)?;
    if !matches!(sep, b'T' | b't' | b' ') {
        return None;
    }
    let hour: u32 = s.get(11..13)?.parse().ok()?;
    if *b.get(13)? != b':' {
        return None;
    }
    let minute: u32 = s.get(14..16)?.parse().ok()?;
    if *b.get(16)? != b':' {
        return None;
    }
    let second: u32 = s.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut rest = s.get(19..)?;
    let mut frac = 0.0f64;
    if let Some(after_dot) = rest.strip_prefix('.') {
        let digits_len = after_dot.bytes().take_while(|c| c.is_ascii_digit()).count();
        if digits_len == 0 {
            return None;
        }
        let frac_str = &after_dot[..digits_len.min(9)];
        let val: f64 = frac_str.parse().ok()?;
        frac = val / 10f64.powi(frac_str.len() as i32);
        rest = &after_dot[digits_len..];
    }
    let tz_offset_secs: i64 = if matches!(rest, "Z" | "z") {
        0
    } else if rest.len() == 6 && (rest.starts_with('+') || rest.starts_with('-')) {
        let sign = if rest.starts_with('+') { 1 } else { -1 };
        if rest.as_bytes()[3] != b':' {
            return None;
        }
        let oh: i64 = rest[1..3].parse().ok()?;
        let om: i64 = rest[4..6].parse().ok()?;
        if oh > 23 || om > 59 {
            return None;
        }
        sign * (oh * 3600 + om * 60)
    } else {
        return None;
    };
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second)
        - tz_offset_secs;
    Some(secs as f64 + frac)
}

/// Howard Hinnant's civil-to-days algorithm: days since 1970-01-01.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400) as u64;
    let m = u64::from(month);
    let d = u64::from(day);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recency_days_computes_real_elapsed_days_and_clamps_future() {
        let t0 = parse_rfc3339_unix_secs("2026-06-15T12:00:00Z").unwrap();
        // 10 days earlier
        let d = recency_days_at("2026-06-05T12:00:00Z", "2026-06-01T00:00:00Z", t0);
        assert!((d - 10.0).abs() < 0.01, "expected ~10 days, got {d}");
        // Falls back to created_at if updated_at is blank/invalid
        let d_fb = recency_days_at("", "2026-06-01T12:00:00Z", t0);
        assert!((d_fb - 14.0).abs() < 0.01, "expected ~14 days, got {d_fb}");
        // Timezone offset handled accurately
        let d_tz = recency_days_at("2026-06-05T14:00:00+02:00", "", t0);
        assert!((d_tz - 10.0).abs() < 0.01, "expected ~10 days, got {d_tz}");
        // Future timestamp clamps to 0.0
        assert_eq!(recency_days_at("2026-06-20T00:00:00Z", "", t0), 0.0);
        // Unparseable timestamps fall back safely to 0.0
        assert_eq!(recency_days_at("not-a-date", "also-bad", t0), 0.0);
    }

    #[tokio::test]
    async fn search_honors_scope_tags_entities_and_recency_ranking() {
        let (pool, ws, user) = crate::test_support::mem_pool().await;
        let svc = MemoryService::with_defaults(pool.clone());

        let mk = |title: &str, body: &str, scope: Scope, tags: Vec<&str>, entities: Vec<&str>| {
            NewMemory {
                collection: "product".into(),
                record_type: "item".into(),
                visibility: "shared".into(),
                scope,
                story_id: None,
                kind: kind::FACT.into(),
                title: title.into(),
                body: body.into(),
                entities: entities.into_iter().map(String::from).collect(),
                tags: tags.into_iter().map(String::from).collect(),
                source_kind: source::MANUAL.into(),
                source_ref: None,
                refs: vec![],
                confidence: Some(0.9),
                salience: Some(0.9),
            }
        };

        let saved = svc
            .save(
                &ws,
                &user,
                vec![
                    mk(
                        "Old note",
                        "settlement window runs nightly",
                        Scope::Workspace,
                        vec!["payments", "ops"],
                        vec!["ledger-svc"],
                    ),
                    mk(
                        "Story note",
                        "settlement window story override",
                        Scope::Story,
                        vec!["billing"],
                        vec!["invoice-svc"],
                    ),
                ],
            )
            .await
            .unwrap();

        // Filter by scope
        let story_hits = svc
            .search(
                &ws,
                MemoryQuery {
                    text: Some("settlement".into()),
                    scope: Some(Scope::Story),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(story_hits.len(), 1);
        assert_eq!(story_hits[0].memory.id, saved[1].id);

        // Filter by tags (case-insensitive)
        let tag_hits = svc
            .search(
                &ws,
                MemoryQuery {
                    text: Some("settlement".into()),
                    tags: vec!["PAYMENTS".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(tag_hits.len(), 1);
        assert_eq!(tag_hits[0].memory.id, saved[0].id);

        // Filter by entities
        let ent_hits = svc
            .search(
                &ws,
                MemoryQuery {
                    text: Some("settlement".into()),
                    entities: vec!["invoice-svc".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(ent_hits.len(), 1);
        assert_eq!(ent_hits[0].memory.id, saved[1].id);

        // Recency affects ranking: aging the same rank-0 memory lowers its score.
        let fresh_score = svc
            .search(
                &ws,
                MemoryQuery {
                    text: Some("settlement".into()),
                    tags: vec!["payments".into()],
                    recency_half_life_days: Some(30.0),
                    ..Default::default()
                },
            )
            .await
            .unwrap()[0]
            .score;
        sqlx::query("UPDATE memories SET updated_at = '2020-01-01T00:00:00Z' WHERE id = ?")
            .bind(&saved[0].id)
            .execute(&pool)
            .await
            .unwrap();
        let stale_score = svc
            .search(
                &ws,
                MemoryQuery {
                    text: Some("settlement".into()),
                    tags: vec!["payments".into()],
                    recency_half_life_days: Some(30.0),
                    ..Default::default()
                },
            )
            .await
            .unwrap()[0]
            .score;
        assert!(
            fresh_score > stale_score,
            "fresh memory ({fresh_score}) should score above stale memory ({stale_score})"
        );
    }

    #[tokio::test]
    async fn recall_context_brief_is_empty_noop_and_formats_provenance_within_budget() {
        let (pool, ws, user) = crate::test_support::mem_pool().await;
        let svc = MemoryService::with_defaults(pool);

        // Empty workspace ⇒ strict no-op.
        let empty = svc
            .recall_context_brief(&ws, MemoryQuery::default(), None)
            .await
            .unwrap();
        assert_eq!(empty, "");

        svc.save(
            &ws,
            &user,
            vec![NewMemory {
                collection: "product".into(),
                record_type: "item".into(),
                visibility: "shared".into(),
                scope: Scope::Workspace,
                story_id: None,
                kind: kind::DECISION.into(),
                title: "Auth cookies".into(),
                body: "Always set HttpOnly and SameSite=Lax".into(),
                entities: vec![],
                tags: vec!["security".into()],
                source_kind: source::MANUAL.into(),
                source_ref: Some("RFC-19".into()),
                refs: vec![],
                confidence: Some(0.95),
                salience: Some(0.9),
            }],
        )
        .await
        .unwrap();

        let md = svc
            .recall_context_brief(
                &ws,
                MemoryQuery {
                    tags: vec!["security".into()],
                    scope: Some(Scope::Workspace),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert!(md.starts_with("## Relevant Workspace Memory\n\n"));
        assert!(md.contains("[decision · workspace · "));
        assert!(md.contains("**Auth cookies**"));
        assert!(md.contains("(source: RFC-19)"));
        assert!(md.len() <= crate::retrieve::MAX_CONTEXT_BRIEF_BYTES);
    }
}
