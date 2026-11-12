//! The vault's full-text index, v2: tokens only, no second copy of the body.
//!
//! Layout (created at runtime by [`Store::ensure_fts`], derived like every
//! vault table, inside the main state database `otto.db`):
//!
//! - `vault_fts2` — a CONTENTLESS FTS5 table (`content=''`,
//!   `contentless_delete=1`): the inverted index plus per-row token counts
//!   (bm25 needs them), never the text. The v1 `vault_fts` stored every note
//!   body a second time (~78 MB of bodies at 10k notes).
//! - `vault_fts2_docs (rid, vault_id, path)` — which note each row is.
//!
//! Snippets are produced at query time for the returned page only: the
//! page's files are read (same 4 MiB body rule as indexing) into a scratch
//! in-memory FTS5 table with the same tokenizer, and FTS5's own `snippet()`
//! runs there — the snippet algorithm reads only the row and the query, so
//! the result is identical to `snippet()` over a stored body.
//!
//! Upgrade / rebuild: nothing is migrated in place. A v1 table is dropped on
//! the first probe and every note missing from v2 is filled by a background
//! backfill — bounded batches of [`BACKFILL_BATCH`] notes per transaction,
//! yielding between batches, logged with counts, resumable (the work list is
//! "notes without a row", so a restart picks up where it stopped). Search
//! keeps answering from what is indexed so far and says so
//! ([`SearchIndexStatus`] on the search page and the vault status).
//!
//! Space: dropping v1 puts its pages on `otto.db`'s freelist. This module
//! never VACUUMs the shared database; the daemon's hourly
//! `incremental_vacuum` (otto-state `maintenance.rs`) returns them to the OS
//! once the file is in incremental mode. Within the vault index, the merge
//! pass ([`Store::fts_merge`]) compacts segments and drops delete tombstones.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use otto_core::{Error, Result};
use sqlx::Connection;

use crate::engine::VaultEngine;
use crate::scan::MAX_FTS_BYTES;
use crate::store::{fts_put_conn, Store};
use crate::types::{SearchHit, SearchIndexStatus};

/// Notes per backfill transaction: one short write-lock hold each.
pub(crate) const BACKFILL_BATCH: i64 = 100;
/// A backfill that wrote at least this many rows ends with a merge pass.
const MERGE_AFTER_ROWS: i64 = 500;
/// Pages per `('merge', -N)` step (negative: every segment is eligible) and
/// the step cap of one pass, so a pass never holds the writer for long.
const MERGE_PAGES: i64 = 256;
const MERGE_MAX_STEPS: usize = 400;

/// Engine-side state of the full-text index.
#[derive(Default)]
pub(crate) struct SearchIndexes {
    /// Vaults this process found complete (nothing missing): no further
    /// counting per read.
    verified: Mutex<HashSet<i64>>,
    /// Running backfills → live progress.
    running: Mutex<HashMap<i64, Arc<Progress>>>,
    /// Vaults whose rows were cleared by a rebuild while a backfill was
    /// already running: that task makes another full pass before it ends
    /// (its path cursor may be past notes the clear just emptied). Read and
    /// written only under the `running` lock, so no request is lost.
    rerun: Mutex<HashSet<i64>>,
    /// The snippet scratch: a private in-memory SQLite connection, never the
    /// state database (no writer contention, no read-only reader issue).
    snip: tokio::sync::Mutex<Option<sqlx::SqliteConnection>>,
    /// Tests that drive the backfill by hand turn the read-path kick off.
    #[cfg(test)]
    pub(crate) no_auto_backfill: std::sync::atomic::AtomicBool,
}

#[derive(Default)]
pub(crate) struct Progress {
    indexed: AtomicI64,
    total: AtomicI64,
}

/// What one backfill pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BackfillReport {
    /// Rows written.
    pub filled: i64,
    /// Notes whose file could not be read (left to the scanner).
    pub skipped: i64,
    /// Transactions committed.
    pub batches: i64,
}

fn dberr(op: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| Error::Internal(format!("{op}: {e}"))
}

/// The body exactly as indexing sees it (`prepare.rs`): the whole file as
/// lossy UTF-8, or empty when it is over [`MAX_FTS_BYTES`]. `None` when the
/// file cannot be opened (gone, a symlink, unreadable).
pub(crate) fn read_body(root: &str, rel: &str) -> Option<String> {
    use std::io::Read;
    let path = std::path::Path::new(root).join(rel);
    let fd = rustix::fs::open(
        &path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let file = std::fs::File::from(fd);
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FTS_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FTS_BYTES {
        return Some(String::new());
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

impl SearchIndexes {
    /// FTS5 `snippet()` for each `(title, body)` against `expr`, computed in
    /// the scratch table (same tokenizer and arguments as the v1 query).
    /// `None` for a row the expression no longer matches (file changed).
    async fn snippets(
        &self,
        expr: &str,
        docs: &[(&str, &str)],
    ) -> std::result::Result<Vec<Option<String>>, sqlx::Error> {
        let mut guard = self.snip.lock().await;
        if guard.is_none() {
            let mut c = sqlx::SqliteConnection::connect("sqlite::memory:").await?;
            sqlx::query(
                "CREATE VIRTUAL TABLE s USING fts5(title, body, \
                 tokenize='unicode61 remove_diacritics 2')",
            )
            .execute(&mut c)
            .await?;
            *guard = Some(c);
        }
        let conn = guard.as_mut().expect("scratch connection just opened");
        let res = async {
            let mut tx = conn.begin().await?;
            for (i, (title, body)) in docs.iter().enumerate() {
                sqlx::query("INSERT INTO s (rowid, title, body) VALUES (?, ?, ?)")
                    .bind(i as i64)
                    .bind(*title)
                    .bind(*body)
                    .execute(&mut *tx)
                    .await?;
            }
            let rows: Vec<(i64, String)> = sqlx::query_as(
                "SELECT rowid, snippet(s, 1, '\u{2039}', '\u{203a}', '…', 14) FROM s WHERE s MATCH ?",
            )
            .bind(expr)
            .fetch_all(&mut *tx)
            .await?;
            // The page is scratch: rolling back leaves the table empty.
            tx.rollback().await?;
            Ok::<_, sqlx::Error>(rows)
        }
        .await;
        let rows = match res {
            Ok(rows) => rows,
            Err(e) => {
                *guard = None;
                return Err(e);
            }
        };
        let mut out = vec![None; docs.len()];
        for (rid, snip) in rows {
            if let Some(slot) = usize::try_from(rid).ok().and_then(|i| out.get_mut(i)) {
                *slot = Some(snip);
            }
        }
        Ok(out)
    }
}

impl Store {
    /// `(indexed, total)` notes of one vault.
    pub(crate) async fn fts_counts(
        &self,
        vault: i64,
    ) -> std::result::Result<(i64, i64), sqlx::Error> {
        sqlx::query_as(
            "SELECT (SELECT count(*) FROM vault_fts2_docs WHERE vault_id = ?), \
                    (SELECT count(*) FROM vault_notes WHERE vault_id = ?)",
        )
        .bind(vault)
        .bind(vault)
        .fetch_one(self.pool())
        .await
    }

    /// The next `limit` notes (by path, after `after`) with no index row.
    async fn fts_missing_batch(
        &self,
        vault: i64,
        after: &str,
        limit: i64,
    ) -> std::result::Result<Vec<(String, String)>, sqlx::Error> {
        sqlx::query_as(
            "SELECT n.path, n.title FROM vault_notes n WHERE n.vault_id = ? AND n.path > ? \
             AND NOT EXISTS (SELECT 1 FROM vault_fts2_docs d \
                             WHERE d.vault_id = n.vault_id AND d.path = n.path) \
             ORDER BY n.path LIMIT ?",
        )
        .bind(vault)
        .bind(after)
        .bind(limit)
        .fetch_all(self.pool())
        .await
    }

    /// Insert rows for notes that STILL have none, in one transaction. A
    /// scan or save that indexed a note after the batch was read wins (its
    /// row is newer than the body read here); a note removed meanwhile is
    /// skipped.
    async fn fts_fill(
        &self,
        vault: i64,
        docs: &[(String, String, String)],
    ) -> std::result::Result<i64, sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        let mut filled = 0;
        for (path, title, body) in docs {
            let (have, note): (bool, bool) = sqlx::query_as(
                "SELECT EXISTS(SELECT 1 FROM vault_fts2_docs WHERE vault_id = ? AND path = ?), \
                        EXISTS(SELECT 1 FROM vault_notes WHERE vault_id = ? AND path = ?)",
            )
            .bind(vault)
            .bind(path)
            .bind(vault)
            .bind(path)
            .fetch_one(&mut *tx)
            .await?;
            if have || !note {
                continue;
            }
            fts_put_conn(&mut tx, vault, path, title, body).await?;
            filled += 1;
        }
        tx.commit().await?;
        Ok(filled)
    }

    /// Hand unreadable notes to the scanner: a changed signature makes the
    /// next scan re-prepare (or prune) them.
    async fn fts_mark_rescan(
        &self,
        vault: i64,
        paths: &[String],
    ) -> std::result::Result<(), sqlx::Error> {
        if paths.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string(paths).unwrap_or_else(|_| "[]".into());
        sqlx::query(
            "UPDATE vault_notes SET mtime_ns = -1 WHERE vault_id = ? \
             AND path IN (SELECT value FROM json_each(?))",
        )
        .bind(vault)
        .bind(json)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Drop one vault's index rows (unregister, explicit rebuild).
    pub(crate) async fn fts_clear_vault(&self, vault: i64) -> std::result::Result<(), sqlx::Error> {
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "DELETE FROM vault_fts2 WHERE rowid IN \
             (SELECT rid FROM vault_fts2_docs WHERE vault_id = ?)",
        )
        .bind(vault)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM vault_fts2_docs WHERE vault_id = ?")
            .bind(vault)
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }

    /// Incremental segment merge (FTS5 `'merge'`, every segment eligible):
    /// the "optimize" for the vault index only, in bounded steps that each
    /// hold the writer briefly. Done when a step changes < 2 rows
    /// (`total_changes()` on the same connection, per the FTS5 docs).
    /// Returns the steps run.
    pub(crate) async fn fts_merge(&self) -> std::result::Result<usize, sqlx::Error> {
        for step in 1..=MERGE_MAX_STEPS {
            let mut conn = self.pool().acquire().await?;
            let before: i64 = sqlx::query_scalar("SELECT total_changes()")
                .fetch_one(&mut *conn)
                .await?;
            sqlx::query("INSERT INTO vault_fts2 (vault_fts2, rank) VALUES ('merge', ?)")
                .bind(-MERGE_PAGES)
                .execute(&mut *conn)
                .await?;
            let after: i64 = sqlx::query_scalar("SELECT total_changes()")
                .fetch_one(&mut *conn)
                .await?;
            drop(conn);
            if after - before < 2 {
                return Ok(step);
            }
            tokio::task::yield_now().await;
        }
        Ok(MERGE_MAX_STEPS)
    }
}

impl VaultEngine {
    /// `None` when the index is complete (or FTS5 is unavailable — search
    /// then uses title/path matching, a separate documented mode).
    pub(crate) async fn search_index_status(&self, id: i64) -> Option<SearchIndexStatus> {
        let running = self.search_index.running.lock().unwrap().get(&id).cloned();
        if let Some(p) = running {
            let (indexed, total) = (
                p.indexed.load(Ordering::Relaxed),
                p.total.load(Ordering::Relaxed),
            );
            if indexed < total {
                return Some(SearchIndexStatus {
                    state: "rebuilding".into(),
                    indexed,
                    total,
                });
            }
            // Counts not loaded yet, or nothing was missing (the common
            // verification pass): decide from the tables, never flash 0/0.
        }
        if !self.fts_known_ready() || self.search_index.verified.lock().unwrap().contains(&id) {
            return None;
        }
        let (indexed, total) = self.store().fts_counts(id).await.ok()?;
        if indexed >= total {
            return None;
        }
        Some(SearchIndexStatus {
            state: "pending".into(),
            indexed,
            total,
        })
    }

    /// Fire-and-forget backfill of the notes missing from the index (one
    /// per vault at a time; a no-op once this process found it complete).
    pub fn kick_search_backfill(self: &Arc<Self>, id: i64) {
        #[cfg(test)]
        if self.search_index.no_auto_backfill.load(Ordering::Relaxed) {
            return;
        }
        if !self.fts_known_ready() || self.search_index.verified.lock().unwrap().contains(&id) {
            return;
        }
        let progress = {
            let mut running = self.search_index.running.lock().unwrap();
            if running.contains_key(&id) {
                return;
            }
            let p = Arc::new(Progress::default());
            running.insert(id, p.clone());
            p
        };
        let eng = self.clone();
        tokio::spawn(async move {
            loop {
                let res = eng.backfill_with(id, &progress).await;
                {
                    // Decide under the `running` lock: a rebuild that lands
                    // after this check finds no task and starts its own.
                    let mut running = eng.search_index.running.lock().unwrap();
                    if eng.search_index.rerun.lock().unwrap().remove(&id) {
                        progress.indexed.store(0, Ordering::Relaxed);
                        progress.total.store(0, Ordering::Relaxed);
                        continue;
                    }
                    if res.is_ok() {
                        eng.search_index.verified.lock().unwrap().insert(id);
                    }
                    running.remove(&id);
                }
                if let Err(e) = res {
                    tracing::warn!(vault = id, error = %e, "vault search index: backfill failed; retried on the next read");
                }
                break;
            }
        });
    }

    /// Run the backfill inline (tests only; the background kick fills in place).
    #[cfg(test)]
    pub(crate) async fn run_search_backfill(&self, id: i64) -> Result<BackfillReport> {
        let progress = Progress::default();
        let report = self.backfill_with(id, &progress).await?;
        self.search_index.verified.lock().unwrap().insert(id);
        Ok(report)
    }

    async fn backfill_with(&self, id: i64, progress: &Progress) -> Result<BackfillReport> {
        let mut report = BackfillReport::default();
        if !self.fts_ready().await {
            return Ok(report);
        }
        let v = self.store().get_vault(id).await?;
        let (indexed, total) = self
            .store()
            .fts_counts(id)
            .await
            .map_err(dberr("vault.search_index.count"))?;
        progress.indexed.store(indexed, Ordering::Relaxed);
        progress.total.store(total, Ordering::Relaxed);
        if indexed >= total {
            return Ok(report);
        }
        tracing::info!(
            vault = id,
            indexed,
            total,
            "vault search index: backfilling notes missing from the full-text index"
        );
        let started = std::time::Instant::now();
        let mut after = String::new();
        loop {
            let batch = self
                .store()
                .fts_missing_batch(id, &after, BACKFILL_BATCH)
                .await
                .map_err(dberr("vault.search_index.batch"))?;
            let Some((last, _)) = batch.last() else {
                break;
            };
            after = last.clone();
            let root = v.root_path.clone();
            let read = tokio::task::spawn_blocking(move || {
                batch
                    .into_iter()
                    .map(|(path, title)| {
                        let body = read_body(&root, &path);
                        (path, title, body)
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|e| Error::Internal(format!("vault search index read: {e}")))?;
            let mut docs = Vec::with_capacity(read.len());
            let mut unreadable = Vec::new();
            for (path, title, body) in read {
                match body {
                    Some(body) => docs.push((path, title, body)),
                    None => unreadable.push(path),
                }
            }
            let filled = self
                .store()
                .fts_fill(id, &docs)
                .await
                .map_err(dberr("vault.search_index.fill"))?;
            self.store()
                .fts_mark_rescan(id, &unreadable)
                .await
                .map_err(dberr("vault.search_index.mark"))?;
            report.filled += filled;
            report.skipped += unreadable.len() as i64;
            report.batches += 1;
            progress.indexed.fetch_add(filled, Ordering::Relaxed);
            tokio::task::yield_now().await;
        }
        let merge_steps = if report.filled >= MERGE_AFTER_ROWS {
            self.store()
                .fts_merge()
                .await
                .map_err(dberr("vault.search_index.merge"))?
        } else {
            0
        };
        tracing::info!(
            vault = id,
            filled = report.filled,
            skipped = report.skipped,
            batches = report.batches,
            merge_steps,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "vault search index: backfill finished"
        );
        Ok(report)
    }

    /// `POST …/search-index` `{"action":"rebuild"}`: drop this vault's rows
    /// and rebuild them from the files in the background. Idempotent while
    /// a rebuild runs. `{"action":"optimize"}` runs the bounded merge pass.
    pub async fn search_index_action(
        self: &Arc<Self>,
        ws: &str,
        id: i64,
        action: &str,
    ) -> Result<SearchIndexStatus> {
        self.get_scoped(ws, id).await?;
        if !self.fts_ready().await {
            return Err(Error::Conflict(
                "Full-text search is unavailable here; search matches titles and paths only".into(),
            ));
        }
        match action {
            "rebuild" => {
                // Always clear and always refill: a backfill already running
                // (e.g. the quick verification pass after start) used to turn
                // the request into a silent no-op.
                self.search_index.verified.lock().unwrap().remove(&id);
                self.store()
                    .fts_clear_vault(id)
                    .await
                    .map_err(dberr("vault.search_index.clear"))?;
                tracing::info!(vault = id, "vault search index: rebuild requested");
                let queued = {
                    let running = self.search_index.running.lock().unwrap();
                    let busy = running.contains_key(&id);
                    if busy {
                        self.search_index.rerun.lock().unwrap().insert(id);
                    }
                    busy
                };
                if !queued {
                    self.kick_search_backfill(id);
                }
            }
            "optimize" => {
                let steps = self
                    .store()
                    .fts_merge()
                    .await
                    .map_err(dberr("vault.search_index.merge"))?;
                tracing::info!(vault = id, steps, "vault search index: optimized");
            }
            other => {
                return Err(Error::Invalid(format!(
                    "unknown search index action {other:?} (rebuild | optimize)"
                )))
            }
        }
        let (indexed, total) = self
            .store()
            .fts_counts(id)
            .await
            .map_err(dberr("vault.search_index.count"))?;
        Ok(self
            .search_index_status(id)
            .await
            .unwrap_or_else(|| SearchIndexStatus {
                state: "ready".into(),
                indexed,
                total,
            }))
    }

    /// Snippets for one page of FTS hits, from the note files.
    pub(crate) async fn fill_snippets(&self, root: &str, expr: &str, hits: &mut [SearchHit]) {
        let root = root.to_string();
        let paths: Vec<String> = hits.iter().map(|h| h.path.clone()).collect();
        let Ok(bodies) = tokio::task::spawn_blocking(move || {
            paths
                .iter()
                .map(|p| read_body(&root, p).unwrap_or_default())
                .collect::<Vec<_>>()
        })
        .await
        else {
            return;
        };
        let docs: Vec<(&str, &str)> = hits
            .iter()
            .zip(&bodies)
            .map(|(h, b)| (h.title.as_str(), b.as_str()))
            .collect();
        match self.search_index.snippets(expr, &docs).await {
            Ok(snips) => {
                for (hit, snip) in hits.iter_mut().zip(snips) {
                    hit.snippet = snip.unwrap_or_default();
                }
            }
            Err(e) => tracing::warn!(error = %e, "vault search snippets failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchReq;

    const NOTES: &[(&str, &str)] = &[
        (
            "deploy.md",
            "---\ntype: Runbook\ntitle: Deploy\n---\n# Deploy\nRoll the deployment with care. \
             Kafka consumers drain first, then the deployment proceeds region by region.",
        ),
        (
            "kafka/consumers.md",
            "# Consumers\nEvery consumer group commits offsets. A slow consumer delays the \
             deployment of downstream services. Café résumé naïve.",
        ),
        (
            "notes/long.md",
            "# Long\nalpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi \
             omicron pi rho sigma tau upsilon phi chi psi omega deployment alpha beta gamma",
        ),
        ("title-only.md", "# Deployment\nnothing else here"),
        ("unrelated.md", "# Other\nnothing to see"),
    ];

    async fn fixture() -> (Arc<VaultEngine>, tempfile::TempDir, i64) {
        let e = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
        let dir = tempfile::tempdir().unwrap();
        for (path, body) in NOTES {
            let abs = dir.path().join(path);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(abs, body).unwrap();
        }
        let id = e
            .store()
            .create_vault("ws", "Search", dir.path().to_str().unwrap(), true)
            .await
            .unwrap();
        e.scan(id).await.unwrap();
        (e, dir, id)
    }

    fn req(q: &str) -> SearchReq {
        SearchReq {
            query: q.into(),
            ..Default::default()
        }
    }

    /// The v1 layout and query, verbatim, over the same notes: the
    /// reference the v2 results must equal.
    async fn v1_reference(
        e: &VaultEngine,
        id: i64,
        root: &std::path::Path,
        expr: &str,
    ) -> Vec<(String, String, f32)> {
        let mut c = sqlx::SqliteConnection::connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE VIRTUAL TABLE vault_fts USING fts5(\
             vault_id UNINDEXED, path UNINDEXED, title, body, tokenize='unicode61 remove_diacritics 2')",
        )
        .execute(&mut c)
        .await
        .unwrap();
        for (path, _) in NOTES {
            let title = e.store().note_meta(id, path).await.unwrap().title;
            let body = read_body(root.to_str().unwrap(), path).unwrap();
            sqlx::query("INSERT INTO vault_fts (vault_id, path, title, body) VALUES (?,?,?,?)")
                .bind(id)
                .bind(*path)
                .bind(title)
                .bind(body)
                .execute(&mut c)
                .await
                .unwrap();
        }
        let rows: Vec<(String, String, f64)> = sqlx::query_as(
            "SELECT path, snippet(vault_fts, 3, '\u{2039}', '\u{203a}', '…', 14) AS snip, \
             bm25(vault_fts) AS rank FROM vault_fts \
             WHERE vault_fts.vault_id = ? AND vault_fts MATCH ? ORDER BY rank LIMIT 50",
        )
        .bind(id)
        .bind(expr)
        .fetch_all(&mut c)
        .await
        .unwrap();
        rows.into_iter()
            .map(|(p, s, r)| (p, s, -r as f32))
            .collect()
    }

    /// Results, order, snippets and scores equal the v1 index on a fixture
    /// vault — including diacritics, frontmatter and a title-only match.
    #[tokio::test]
    async fn search_results_and_snippets_equal_the_v1_index() {
        let (e, dir, id) = fixture().await;
        for q in [
            "deployment",
            "kafka consumer",
            "cafe",
            "resume",
            "omega",
            "deploy",
        ] {
            let expr = crate::engine::fts_expr(q);
            let want = v1_reference(&e, id, dir.path(), &expr).await;
            let page = e.search_page("ws", id, &req(q)).await.unwrap();
            assert!(page.index.is_none(), "{q}: complete index");
            let got: Vec<(String, String, f32)> = page
                .hits
                .iter()
                .map(|h| (h.path.clone(), h.snippet.clone(), h.score))
                .collect();
            assert!(!want.is_empty(), "{q}: the fixture must match something");
            assert_eq!(got.len(), want.len(), "{q}: {got:?} vs {want:?}");
            for ((gp, gs, gr), (wp, ws, wr)) in got.iter().zip(&want) {
                assert_eq!(gp, wp, "{q}: order");
                assert_eq!(gs, ws, "{q}: snippet of {gp}");
                assert!((gr - wr).abs() < 1e-4, "{q}: score of {gp}: {gr} vs {wr}");
            }
        }
    }

    /// The rebuild is bounded (≤ BACKFILL_BATCH notes per transaction),
    /// resumable (a second pass only fills what is still missing) and
    /// reports a partial index as partial until it completes.
    #[tokio::test]
    async fn rebuild_is_bounded_resumable_and_reports_partial() {
        let e = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
        let dir = tempfile::tempdir().unwrap();
        let n = BACKFILL_BATCH as usize * 2 + 7;
        for i in 0..n {
            std::fs::write(
                dir.path().join(format!("n{i:04}.md")),
                format!("# N{i}\nneedle{i} sharedword"),
            )
            .unwrap();
        }
        let id = e
            .store()
            .create_vault("ws", "Big", dir.path().to_str().unwrap(), true)
            .await
            .unwrap();
        e.scan(id).await.unwrap();
        e.search_index
            .no_auto_backfill
            .store(true, Ordering::Relaxed);
        // Lose the index (an upgrade from v1 starts here).
        e.store().fts_clear_vault(id).await.unwrap();
        let page = e.search_page("ws", id, &req("sharedword")).await.unwrap();
        let idx = page
            .index
            .expect("an empty index is reported, never silent");
        assert_eq!((idx.indexed, idx.total), (0, n as i64));
        assert_eq!(idx.state, "pending");
        // Simulate a stop after the first batch: fill exactly one batch.
        let batch = e
            .store()
            .fts_missing_batch(id, "", BACKFILL_BATCH)
            .await
            .unwrap();
        assert_eq!(batch.len(), BACKFILL_BATCH as usize, "bounded batch");
        let root = dir.path().to_str().unwrap().to_string();
        let docs: Vec<_> = batch
            .into_iter()
            .map(|(p, t)| {
                let b = read_body(&root, &p).unwrap();
                (p, t, b)
            })
            .collect();
        assert_eq!(e.store().fts_fill(id, &docs).await.unwrap(), BACKFILL_BATCH);
        let page = e.search_page("ws", id, &req("sharedword")).await.unwrap();
        let idx = page.index.expect("still partial");
        assert_eq!(idx.indexed, BACKFILL_BATCH);
        assert_eq!(page.hits.len(), 50, "answers from the indexed part");
        // A restarted engine resumes: only the remainder is written.
        let restarted = Arc::new(VaultEngine::new(e.store().pool().clone()));
        let report = restarted.run_search_backfill(id).await.unwrap();
        assert_eq!(report.filled, n as i64 - BACKFILL_BATCH);
        assert_eq!(report.batches, 2, "≤ BACKFILL_BATCH notes per transaction");
        let page = restarted
            .search_page("ws", id, &req(&format!("needle{}", n - 1)))
            .await
            .unwrap();
        assert!(page.index.is_none(), "complete");
        assert_eq!(page.hits.len(), 1);
        // Nothing left: a further pass writes nothing.
        assert_eq!(restarted.run_search_backfill(id).await.unwrap().filled, 0);
    }

    /// The explicit rebuild drops the vault's rows and rebuilds them in the
    /// background; optimize merges segments; unknown actions are refused.
    #[tokio::test]
    async fn explicit_rebuild_and_optimize() {
        let (e, _dir, id) = fixture().await;
        let st = e.search_index_action("ws", id, "rebuild").await.unwrap();
        assert!(st.indexed <= st.total);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while e.search_index_status(id).await.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("rebuild finishes");
        // Every note is back: a term from each body finds it.
        for (q, path) in [
            ("offsets", "kafka/consumers.md"),
            ("region", "deploy.md"),
            ("omicron", "notes/long.md"),
        ] {
            let hits = e.search("ws", id, &req(q)).await.unwrap();
            assert!(hits.iter().any(|h| h.path == path), "{q}: {hits:?}");
        }
        let st = e.search_index_action("ws", id, "optimize").await.unwrap();
        assert_eq!(st.state, "ready");
        assert_eq!(st.indexed, NOTES.len() as i64);
        assert!(matches!(
            e.search_index_action("ws", id, "drop").await,
            Err(Error::Invalid(_))
        ));
    }

    /// A rebuild requested while a backfill is already running (the quick
    /// verification pass after start) still clears the rows and queues a
    /// second full pass on that task — it used to be dropped silently.
    #[tokio::test]
    async fn rebuild_during_a_running_backfill_is_queued_not_dropped() {
        let (e, _dir, id) = fixture().await;
        e.search_index
            .running
            .lock()
            .unwrap()
            .insert(id, Arc::new(Progress::default()));
        e.search_index_action("ws", id, "rebuild").await.unwrap();
        assert!(e.search_index.rerun.lock().unwrap().contains(&id));
        assert_eq!(
            e.store().fts_counts(id).await.unwrap(),
            (0, NOTES.len() as i64),
            "the rows were cleared, not left as they were"
        );
    }

    /// A note whose file vanished before the backfill read it is left to
    /// the scanner (marked for rescan), not indexed empty.
    #[tokio::test]
    async fn unreadable_notes_are_handed_to_the_scanner() {
        let (e, dir, id) = fixture().await;
        e.search_index
            .no_auto_backfill
            .store(true, Ordering::Relaxed);
        e.store().fts_clear_vault(id).await.unwrap();
        std::fs::remove_file(dir.path().join("unrelated.md")).unwrap();
        let report = e.run_search_backfill(id).await.unwrap();
        assert_eq!(report.skipped, 1);
        assert_eq!(report.filled, NOTES.len() as i64 - 1);
        let sig: i64 =
            sqlx::query_scalar("SELECT mtime_ns FROM vault_notes WHERE vault_id = ? AND path = ?")
                .bind(id)
                .bind("unrelated.md")
                .fetch_one(e.store().pool())
                .await
                .unwrap();
        assert_eq!(sig, -1);
    }
}
