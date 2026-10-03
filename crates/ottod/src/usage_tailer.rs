//! Background usage tailer — mines *real* token usage from the agent CLIs'
//! on-disk transcript files and records it into the embedded ClickHouse usage
//! store via [`otto_usage::UsageEngine`].
//!
//! Why a tailer (rather than instrumenting the PTY): the CLIs already write
//! exact, per-turn token counts (input/output/cache) and the model id to JSONL
//! transcripts. Tailing those files is the single source of truth and survives
//! resumes, channel sessions, and restarts.
//!
//! Supported providers:
//!   * **Claude Code** — `~/.claude/projects/<enc_cwd>/<uuid>.jsonl`. Attributed
//!     by transcript filename stem (= `provider_session_id` in `sessions`).
//!     Subagent transcripts — `<enc_cwd>/<uuid>/subagents/agent-*.jsonl`, most
//!     of the claude files on a busy machine — are tailed too and attributed to
//!     their PARENT session `<uuid>` (r3-08-14). The global response-key seen
//!     set is what keeps them from double counting: a legacy in-file
//!     `isSidechain` copy of the same response carries the same key.
//!   * **Codex** — `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`.
//!     Attributed by `cwd` (from the file's `session_meta` line) → the unique
//!     codex session with that cwd, if exactly one.
//!   * **agy** — unsupported (token usage is encrypted on disk); logged once.
//!
//! Correctness invariants:
//!   * **No double-counting.** Provider-specific guards complement the per-file
//!     byte-offset cursor. Claude uses a persisted response-key seen set.
//!     Codex uses persisted cumulative counters keyed by session id, so repeated
//!     snapshots and resumes across rollout files emit only positive deltas.
//!     The cursor itself is persisted to `<data_dir>/usage_tailer.json`
//!     (atomic write), so no *line*
//!     is read twice — including across restarts. Only complete lines (up to
//!     the last `\n`) are consumed; a trailing partial line is left for the
//!     next scan. Claude's response keys are
//!     (`message.id:requestId`, `<data_dir>/usage_tailer_seen.json`), because
//!     one API *response* spans several transcript lines (one per content
//!     block, all repeating the same usage) and resumed sessions replay old
//!     lines into new files — so a response can arrive on many lines while
//!     billing happens once. Lines written while a response streams carry a
//!     PARTIAL `output_tokens`, so the response is held (per file) until its
//!     `stop_reason` line, the next response, or a scan with no growth, and
//!     counted once with its FINAL usage (`ResponseFolder`); a line arriving
//!     after that becomes a positive correction row. The persisted cursor
//!     never passes a held response, so a restart re-reads it.
//!   * **True-time stamping.** Claude events carry the transcript line's own
//!     `timestamp` (`UsageEvent.ts`), so history ingested late (the one-time
//!     rebuild below, or catch-up after daemon downtime) is dated when the API
//!     call actually happened, not when it was ingested. Codex lines carry no
//!     usable per-turn timestamp and keep the insert-time default; their
//!     pre-existing history is still seeded away at startup.
//!   * **One-time dedup rebuild.** The pre-dedup tailer counted every line, so
//!     stores it fed are inflated (~2.4× on real data). On first start after
//!     upgrade (marker `<data_dir>/usage_tailer_dedup_rebuild_v2.done` absent) the
//!     tailer purges its own claude rows and re-derives them from the full
//!     transcripts — deduped (FINAL usage per response), true-time-stamped,
//!     re-priced. Delete-first + marker-last
//!     makes a crashed rebuild retry cleanly on the next start.
//!   * **Crash-resilient.** A bad file/line logs and is skipped; the loop never
//!     panics.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_state::{DbPool, SessionsRepo};
use otto_usage::{
    estimate_cost, parse_claude_line, parse_codex_line, parse_codex_session_meta, ClaudeLine,
    CodexCounterStore, CursorStore, HeldResponse, ResponseFolder, SeenKeys, UsageEngine,
    UsageEvent, EXTERNAL_WORKSPACE,
};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Poll interval without a watcher, and the wake-up cadence while a streamed
/// response is held (its release needs a quiet period).
const SCAN_INTERVAL: Duration = Duration::from_secs(20);

/// Full listing cadence while the FSEvents watcher runs — the safety net for
/// anything the event stream missed (and where dead cursors are evicted).
const RECONCILE_INTERVAL: Duration = Duration::from_secs(600);

/// Coalesce a burst of FSEvents (an agent writing many lines) into one pass.
const FS_DEBOUNCE: Duration = Duration::from_secs(2);

/// A held response whose file stayed quiet this long is counted as final.
const HOLD_IDLE: Duration = Duration::from_secs(15);

/// Max bytes read (and held) per tail step; a big catch-up streams in windows.
const READ_WINDOW: u64 = 8 * 1024 * 1024;

/// Cursor log lines tolerated before the next persist compacts the map.
const CURSOR_LOG_COMPACT_AFTER: usize = 5_000;

/// Cap on the persisted claude response-key seen-set. Real transcripts produce
/// ~1.5k responses/day, so 100k keys ≈ two months of history — far beyond how
/// far back a resume replays — while keeping the JSON file a few MB.
const SEEN_KEYS_CAP: usize = 100_000;

/// Codex sessions are far less numerous than response ids; this covers years
/// of normal use while bounding the persisted cumulative-counter map.
const CODEX_COUNTERS_CAP: usize = 20_000;

/// Default model label for Codex turns when the rollout file carries no model.
/// `estimate_cost` prices this at the gpt tier (substring match on "codex").
const CODEX_FALLBACK_MODEL: &str = "codex";

/// Released claude responses remembered for late-line corrections (see
/// [`ResponseFolder`]). A response only straddles an idle release while it is
/// still streaming, so a few thousand recent keys is plenty.
const RECENT_RESPONSES_CAP: usize = 4_096;

/// Marker of the one-time claude history rebuild. `_v2`: the first rebuild
/// (marker `usage_tailer_dedup_rebuild.done`) kept the FIRST line per response
/// — a streamed response's partial `output_tokens` — and priced Opus 5.5 at
/// the Opus 4.x rate card, so it runs once more to keep the FINAL usage and
/// re-price every claude row.
const REBUILD_MARKER: &str = "usage_tailer_dedup_rebuild_v2.done";

/// Append-only seen-key log lines tolerated before the next persist compacts
/// them into the JSON array. ~5k keys ≈ a few days of active use; each append
/// is a few hundred bytes instead of a multi-MB rewrite.
const SEEN_LOG_COMPACT_AFTER: usize = 5_000;

// ---------------------------------------------------------------------------
// Public handle
// ---------------------------------------------------------------------------

/// Handle returned by [`UsageTailer::start`]. Keep it alive for the process
/// lifetime; dropping it sets the cancel flag and stops the loop.
pub struct UsageTailerHandle {
    cancel: Arc<AtomicBool>,
    wake: Arc<Notify>,
    _task: JoinHandle<()>,
}

impl Drop for UsageTailerHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // `notify_one` stores a permit if the loop isn't parked yet, so the
        // next sleep returns immediately either way.
        self.wake.notify_one();
    }
}

// ---------------------------------------------------------------------------
// Tailer
// ---------------------------------------------------------------------------

pub struct UsageTailer {
    usage: Arc<UsageEngine>,
    pool: DbPool,
    /// Home directory — the root of `~/.claude` and `~/.codex`.
    home: PathBuf,
    /// Daemon data dir — holds the cursor/seen files and the rebuild marker.
    data_dir: PathBuf,
    cursors: CursorStore,
    /// Size of each transcript when it was last tailed (memory only). Change
    /// detection compares against THIS, not the cursor: the cursor stops at
    /// the last `\n`, so a file ending in a partial line would otherwise look
    /// "grown" — and re-trigger the attribution query + a read — every scan.
    last_size: HashMap<PathBuf, u64>,
    /// Parsed `session_meta` per codex rollout. The first line is written once
    /// at rollout creation and never changes, so it is read at most once per
    /// file per daemon lifetime (and only once the file has new bytes).
    codex_meta: HashMap<PathBuf, otto_usage::CodexMeta>,
    /// Response-level dedup for claude lines (see module docs).
    seen: SeenKeys,
    /// Folds a streamed response's lines into its FINAL usage.
    folder: ResponseFolder,
    /// The response currently held back per claude file (still streaming).
    held: HashMap<PathBuf, HeldResponse>,
    /// Attribution of each held response's file: (workspace_id, session_id).
    held_attr: HashMap<PathBuf, (String, String)>,
    /// When each held response's file last grew; released after HOLD_IDLE.
    held_grew: HashMap<PathBuf, Instant>,
    /// Set when [`Self::scan_once`] recorded a new claude key, so the seen file
    /// is only rewritten when it actually changed.
    seen_dirty: bool,
    /// Session-wide cumulative-token baselines for Codex rollout snapshots.
    codex_counters: CodexCounterStore,
    codex_counters_dirty: bool,
    /// Attribution for the current pass, built on the first tail that
    /// actually has bytes (an idle pass never queries `sessions`).
    attr: Option<Attribution>,
    /// Work counters (perf guards in tests; cheap enough to keep always).
    stats: TailerStats,
}

/// How much work the tailer did — the unchanged-tree guard asserts these stay
/// flat across idle passes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct TailerStats {
    attribution_builds: u64,
    range_reads: u64,
    full_scans: u64,
}

/// Paths the FSEvents watcher saw change since the last pass.
#[derive(Default)]
struct DirtySet {
    paths: HashSet<PathBuf>,
    /// Kernel/user events were dropped (or the watcher errored): only a full
    /// listing can tell what changed.
    rescan: bool,
}

/// One Otto session, projected for attribution.
#[derive(Clone)]
struct SessionRef {
    otto_session_id: String,
    workspace_id: String,
    provider: String,
}

/// Attribution indexes, rebuilt from the `sessions` table on demand.
#[derive(Default)]
struct Attribution {
    /// claude: `provider_session_id` (= transcript filename stem) → session.
    by_provider_session: HashMap<String, SessionRef>,
    /// codex: `cwd` → all sessions in that directory (used only when unique).
    by_cwd: HashMap<String, Vec<SessionRef>>,
}

impl UsageTailer {
    /// Build the tailer. `data_dir` holds the persisted cursor file; `home` is
    /// the root for the `~/.claude` and `~/.codex` transcript trees.
    pub fn new(usage: Arc<UsageEngine>, pool: DbPool, data_dir: PathBuf, home: PathBuf) -> Self {
        let cursors = CursorStore::load(data_dir.join("usage_tailer.json"));
        let seen = SeenKeys::load(data_dir.join("usage_tailer_seen.json"), SEEN_KEYS_CAP);
        let codex_counters = CodexCounterStore::load(
            data_dir.join("usage_tailer_codex_totals.json"),
            CODEX_COUNTERS_CAP,
        );
        Self {
            usage,
            pool,
            home,
            data_dir,
            cursors,
            last_size: HashMap::new(),
            codex_meta: HashMap::new(),
            seen,
            folder: ResponseFolder::new(RECENT_RESPONSES_CAP),
            held: HashMap::new(),
            held_attr: HashMap::new(),
            held_grew: HashMap::new(),
            seen_dirty: false,
            codex_counters,
            codex_counters_dirty: false,
            attr: None,
            stats: TailerStats::default(),
        }
    }

    fn claude_root(&self) -> PathBuf {
        self.home.join(".claude").join("projects")
    }

    fn codex_root(&self) -> PathBuf {
        self.home.join(".codex").join("sessions")
    }

    /// Spawn the background loop. Returns immediately.
    ///
    /// Event-driven: an FSEvents watcher (notify) on the two transcript roots
    /// collects changed `*.jsonl` paths; the loop wakes on them (debounced),
    /// stats and tails only those. A full listing runs at boot, every
    /// RECONCILE_INTERVAL, and whenever the watcher reports dropped events.
    /// The 20 s timer is armed only while a streamed response is held (its
    /// release needs a quiet period). No watcher → the old 20 s full scan.
    pub fn start(mut self) -> UsageTailerHandle {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_task = Arc::clone(&cancel);
        let wake = Arc::new(Notify::new());
        let wake_task = Arc::clone(&wake);
        let task = tokio::spawn(async move {
            // One-time (marker-gated): purge the pre-dedup tailer's inflated
            // claude rows and re-derive them from the full transcripts.
            self.rebuild_claude_history().await;
            // Skip remaining pre-existing history (codex, and any claude file
            // the rebuild couldn't touch) so old turns aren't replayed with a
            // now() timestamp.
            self.seed_existing_files().await;

            let fs_wake = Arc::new(Notify::new());
            let dirty: Arc<std::sync::Mutex<DirtySet>> = Arc::default();
            let mut watch = TranscriptWatch::new(Arc::clone(&dirty), Arc::clone(&fs_wake));
            let mut last_full: Option<Instant> = None;
            loop {
                if cancel_task.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(w) = watch.as_mut() {
                    w.ensure_roots(&[self.claude_root(), self.codex_root()]);
                }
                let (paths, rescan) = {
                    let mut d = dirty.lock().unwrap_or_else(|e| e.into_inner());
                    (std::mem::take(&mut d.paths), std::mem::take(&mut d.rescan))
                };
                let full = watch.is_none()
                    || rescan
                    || last_full.is_none_or(|t| t.elapsed() >= RECONCILE_INTERVAL);
                let res = if full {
                    last_full = Some(Instant::now());
                    self.scan_once().await
                } else {
                    self.scan_paths(paths).await
                };
                if let Err(e) = res {
                    tracing::warn!("usage tailer: scan failed: {e}");
                }
                let timeout = if watch.is_none() || !self.held.is_empty() {
                    SCAN_INTERVAL
                } else {
                    RECONCILE_INTERVAL
                        .saturating_sub(last_full.map(|t| t.elapsed()).unwrap_or_default())
                };
                // One timer per pass; dropping the handle wakes us at once.
                tokio::select! {
                    _ = tokio::time::sleep(timeout) => {}
                    _ = fs_wake.notified() => {
                        // Debounce: an agent writes a burst of lines; take
                        // them in one pass.
                        tokio::select! {
                            _ = tokio::time::sleep(FS_DEBOUNCE) => {}
                            _ = wake_task.notified() => {
                                self.release_held(|_| true);
                                return;
                            }
                        }
                    }
                    _ = wake_task.notified() => {
                        // Count what is still held; its key is persisted only
                        // if a later persist runs, so a lost flush is re-read.
                        self.release_held(|_| true);
                        return;
                    }
                }
            }
        });
        UsageTailerHandle {
            cancel,
            wake,
            _task: task,
        }
    }

    /// One-time (marker-gated) correction pass over the claude transcripts.
    ///
    /// The pre-dedup tailer counted every assistant line, so a response whose
    /// content spans several lines — or is replayed into a resumed session's
    /// file — was recorded several times over (~2.4× inflation measured on real
    /// data). This pass re-derives claude usage from scratch:
    ///
    ///   1. parse every claude transcript from byte 0, deduped by response key,
    ///      each event stamped with its line's own timestamp (also *backfills*
    ///      history the old tailer skipped at seed time);
    ///   2. purge the old tailer rows — synchronously, bounded by the oldest
    ///      rebuilt date so rows whose transcripts were since deleted survive;
    ///   3. insert the rebuilt events, advance cursors to the parsed offsets,
    ///      fold the keys into the live seen-set, persist, write the marker.
    ///
    /// Ordering makes a crashed or failed rebuild safe to retry: the purge runs
    /// before the insert and its predicate also matches rebuilt rows (same
    /// dim-less claude completion shape), so a partial insert is swept up by
    /// the next attempt; the marker is only written after full success. On any
    /// error the in-memory cursors/seen stay untouched — the live loop keeps
    /// tailing appends from the old offsets (deduped) until the next daemon
    /// start retries.
    async fn rebuild_claude_history(&mut self) {
        let marker = self.data_dir.join(REBUILD_MARKER);
        if marker.exists() {
            return;
        }
        // The engine boots ClickHouse concurrently with us; give it a moment.
        // Not ready (or usage disabled) → skip without the marker so the next
        // start retries.
        if !self.usage.wait_ready(Duration::from_secs(90)).await {
            tracing::warn!("usage tailer: dedup rebuild skipped — usage engine not ready");
            return;
        }

        let attr = self.build_attribution().await;
        let attr: Arc<HashMap<String, (String, String)>> = Arc::new(
            attr.by_provider_session
                .into_iter()
                .map(|(k, s)| (k, (s.workspace_id, s.otto_session_id)))
                .collect(),
        );
        // Oldest-first so that, if the seen-set cap ever evicts, it evicts the
        // keys least likely to be replayed again.
        let home = self.home.clone();
        let files: Arc<Vec<PathBuf>> = Arc::new(
            tokio::task::spawn_blocking(move || {
                let mut files: Vec<(PathBuf, std::time::SystemTime)> = list_claude_files(&home)
                    .into_iter()
                    .map(|(f, _)| {
                        let m = std::fs::metadata(&f)
                            .and_then(|m| m.modified())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                        (f, m)
                    })
                    .collect();
                files.sort_by_key(|(_, m)| *m);
                files.into_iter().map(|(f, _)| f).collect()
            })
            .await
            .unwrap_or_default(),
        );

        // Pass 1 (blocking pool, streaming, O(1) memory): the oldest event
        // date bounds the purge, and the purge must precede every insert.
        let f1 = Arc::clone(&files);
        let (n_usage, min_date) = tokio::task::spawn_blocking(move || rebuild_min_date(&f1))
            .await
            .unwrap_or((0, None));

        if n_usage == 0 {
            // Fresh install / no transcripts: nothing to correct, and no date
            // to bound a purge by — just mark done.
            if let Err(e) = std::fs::write(&marker, b"no-events\n") {
                tracing::warn!("usage tailer: failed to write rebuild marker: {e}");
            }
            tracing::info!("usage tailer: dedup rebuild — no claude transcript usage found");
            return;
        }
        let Some(min_date) = min_date else {
            // Events exist but none carried a timestamp (never seen in real
            // transcripts): an unbounded purge is riskier than keeping the old
            // rows, and inserting without purging would double-count. Skip.
            tracing::warn!(
                "usage tailer: dedup rebuild skipped — no line timestamps to bound the purge"
            );
            if let Err(e) = std::fs::write(&marker, b"skipped-no-timestamps\n") {
                tracing::warn!("usage tailer: failed to write rebuild marker: {e}");
            }
            return;
        };

        tracing::info!(
            "usage tailer: dedup rebuild — purging claude tailer rows since {min_date}, \
             re-ingesting from {} files",
            files.len()
        );
        if let Err(e) = self.usage.purge_claude_tailer_rows(&min_date).await {
            tracing::warn!("usage tailer: dedup rebuild aborted (purge failed): {e}");
            return;
        }

        // Pass 2 (blocking pool, streaming): parse + dedup file by file and
        // hand finished events over in REBUILD_BATCH chunks, inserted here as
        // they arrive — peak memory is one file's open responses plus a batch
        // plus a 16-byte hash per response, not the whole history.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<UsageEvent>>(2);
        let f2 = Arc::clone(&files);
        let scan = tokio::task::spawn_blocking(move || rebuild_events(&f2, &attr, tx));
        let mut inserted = 0usize;
        let mut failed = false;
        while let Some(batch) = rx.recv().await {
            if let Err(e) = self.usage.insert_events(&batch).await {
                tracing::warn!(
                    "usage tailer: dedup rebuild insert failed (will retry next start): {e}"
                );
                failed = true;
                break;
            }
            inserted += batch.len();
        }
        drop(rx); // a failed insert stops the producer at its next send
        let out = match scan.await {
            Ok(out) => out,
            Err(e) => {
                tracing::warn!("usage tailer: dedup rebuild scan panicked: {e}");
                return;
            }
        };
        if failed || !out.completed {
            return;
        }
        for (f, off) in &out.offsets {
            self.cursors.set(f, *off);
        }
        for h in &out.keys {
            self.seen.insert_hash(*h);
        }
        // Newest keys stay remembered (files are oldest-first), so a response
        // still streaming during the rebuild is corrected, not lost.
        for (k, line) in &out.recent {
            self.folder.remember(k, line);
        }
        if let Err(e) = self.cursors.save() {
            tracing::warn!("usage tailer: failed to persist cursors after rebuild: {e}");
        }
        if let Err(e) = self.seen.save() {
            tracing::warn!("usage tailer: failed to persist seen keys after rebuild: {e}");
        }
        if let Err(e) = std::fs::write(&marker, format!("rebuilt {inserted} events\n")) {
            tracing::warn!("usage tailer: failed to write rebuild marker: {e}");
        }
        tracing::info!("usage tailer: dedup rebuild complete — {inserted} events re-ingested");
    }

    /// Seed the cursor for every transcript file that isn't already tracked,
    /// setting it to the file's current size so existing history is skipped.
    /// Codex also records each session's latest cumulative snapshot: otherwise
    /// the first append after upgrading would look like an all-history delta.
    async fn seed_existing_files(&mut self) {
        let mut seeded = 0usize;
        let (claude, codex) = self.list_files().await;
        for (f, size) in claude {
            if self.cursors.contains(&f) {
                continue;
            }
            self.cursors.set(&f, size);
            seeded += 1;
        }

        let mut codex_baselines = 0usize;
        let mut catchup_files = 0usize;
        let mut new_baseline_sessions = std::collections::HashSet::new();
        for (f, size) in codex {
            // Tracked rollout of a session with a baseline: nothing to seed,
            // so skip the 64 KiB head read (it ran for every rollout ever
            // written, every boot). The meta is read lazily once it grows.
            if self.cursors.contains(&f) && self.codex_counters.contains(&codex_thread_uuid(&f)) {
                continue;
            }
            let meta = read_codex_meta(&f).await;
            let session_id = meta
                .as_ref()
                .and_then(|m| m.session_id.clone())
                .unwrap_or_else(|| codex_thread_uuid(&f));
            let needs_baseline = new_baseline_sessions.contains(&session_id)
                || !self.codex_counters.contains(&session_id);
            if !self.cursors.contains(&f) {
                if !needs_baseline {
                    // A resume can create a new rollout while ottod is down.
                    // Read it from byte 0 against the persisted session total.
                    self.cursors.set(&f, 0);
                    catchup_files += 1;
                } else {
                    // Preserve the existing no-historical-backfill policy for
                    // sessions Otto has never observed.
                    self.cursors.set(&f, size);
                    seeded += 1;
                }
            }
            if needs_baseline {
                new_baseline_sessions.insert(session_id.clone());
            }
            if let Some(m) = &meta {
                self.codex_meta.insert(f.clone(), m.clone());
            }
            let model = meta
                .and_then(|m| m.model)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| CODEX_FALLBACK_MODEL.to_string());
            if new_baseline_sessions.contains(&session_id) {
                if let Some(total) = read_last_codex_usage(&f, &model).await {
                    self.codex_counters.seed(&session_id, &total);
                    codex_baselines += 1;
                }
            }
        }
        if seeded > 0 {
            if let Err(e) = self.cursors.save() {
                tracing::warn!("usage tailer: failed to persist seeded cursors: {e}");
            }
        }
        if codex_baselines > 0 {
            if let Err(e) = self.codex_counters.save() {
                tracing::warn!("usage tailer: failed to persist Codex baselines: {e}");
            }
        }
        tracing::info!(
            "usage tailer: seeded {seeded} pre-existing transcript file(s), \
             {codex_baselines} Codex cumulative baseline(s), \
             {catchup_files} resumed rollout(s) queued for catch-up"
        );
    }

    /// One full pass: list both trees (stat-first), tail what changed, evict
    /// cursors of deleted transcripts, persist.
    async fn scan_once(&mut self) -> Result<(), String> {
        self.stats.full_scans += 1;
        let (claude, codex) = self.list_files().await;
        self.tail_listed(&claude, &codex).await;
        self.evict_dead_cursors(&claude, &codex);
        self.persist();
        Ok(())
    }

    /// An event-driven pass over just the paths the watcher reported.
    async fn scan_paths(&mut self, paths: HashSet<PathBuf>) -> Result<(), String> {
        let claude_root = self.claude_root();
        let codex_root = self.codex_root();
        let (claude, codex) = tokio::task::spawn_blocking(move || {
            let mut claude = Vec::new();
            let mut codex = Vec::new();
            for p in paths {
                let is_claude = is_claude_transcript(&claude_root, &p);
                if !is_claude && !is_codex_rollout(&codex_root, &p) {
                    continue;
                }
                // Gone (deleted/renamed) → the reconcile pass evicts it.
                let Ok(md) = std::fs::metadata(&p) else {
                    continue;
                };
                if is_claude {
                    claude.push((p, md.len()));
                } else {
                    codex.push((p, md.len()));
                }
            }
            (claude, codex)
        })
        .await
        .unwrap_or_default();
        self.tail_listed(&claude, &codex).await;
        self.persist();
        Ok(())
    }

    /// Tail every listed file whose size moved since it was last tailed, then
    /// release held responses that have been quiet for HOLD_IDLE.
    async fn tail_listed(&mut self, claude: &[(PathBuf, u64)], codex: &[(PathBuf, u64)]) {
        self.attr = None; // rebuilt lazily, at most once per pass
        for (file, size) in claude {
            if self.last_size.get(file) == Some(size) {
                continue;
            }
            match self.tail_claude_file(file, *size).await {
                Ok(true) => {
                    if self.held.contains_key(file) {
                        self.held_grew.insert(file.clone(), Instant::now());
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::debug!("usage tailer: claude file {} skipped: {e}", file.display())
                }
            }
            self.last_size.insert(file.clone(), *size);
        }
        // A held response whose file wrote nothing for HOLD_IDLE is as final
        // as it will get — count it (a later line becomes a correction).
        let grew = std::mem::take(&mut self.held_grew);
        self.release_held(|f| grew.get(f).is_none_or(|t| t.elapsed() >= HOLD_IDLE));
        self.held_grew = grew;
        self.held_grew.retain(|f, _| self.held.contains_key(f));
        for (file, size) in codex {
            if self.last_size.get(file) == Some(size) {
                continue;
            }
            if let Err(e) = self.tail_codex_file(file, *size).await {
                tracing::debug!("usage tailer: codex file {} skipped: {e}", file.display());
            }
            self.last_size.insert(file.clone(), *size);
        }
        self.attr = None;
    }

    /// Persist whatever changed this scan, off the async worker. Provider-level
    /// dedup baselines are written before byte offsets: if a guard write fails,
    /// replaying lines is safe in-process and safer across restart than
    /// committing a cursor ahead of its dedup state.
    fn persist(&mut self) {
        blocking_io(|| {
            let mut guards_persisted = true;
            if self.seen_dirty || self.seen.has_pending() {
                // Append-only: only the keys new since the last write hit disk.
                match self.seen.append_pending(SEEN_LOG_COMPACT_AFTER) {
                    Ok(()) => self.seen_dirty = false,
                    Err(e) => {
                        guards_persisted = false;
                        tracing::warn!("usage tailer: failed to persist seen keys: {e}");
                    }
                }
            }
            if self.codex_counters_dirty {
                match self.codex_counters.save() {
                    Ok(()) => self.codex_counters_dirty = false,
                    Err(e) => {
                        guards_persisted = false;
                        tracing::warn!("usage tailer: failed to persist Codex counters: {e}");
                    }
                }
            }
            if guards_persisted && self.cursors.is_dirty() {
                // Never persist a cursor past a held (uncounted) response: a
                // restart re-reads it from its first line instead of losing it.
                // Append-only: just the moved cursors hit disk.
                let floors: HashMap<String, u64> = self
                    .held
                    .iter()
                    .map(|(f, h)| (f.to_string_lossy().into_owned(), h.start_offset))
                    .collect();
                let res = self.cursors.persist(CURSOR_LOG_COMPACT_AFTER, |k, off| {
                    floors.get(k).map_or(off, |&fl| fl.min(off))
                });
                if let Err(e) = res {
                    tracing::warn!("usage tailer: failed to persist cursors: {e}");
                }
            }
        });
    }

    /// Drop cursors (and cached codex metas / sizes) for transcripts that no
    /// longer exist — Claude prunes transcripts after ~30 days, which left over
    /// half the cursor map pointing at deleted files. A key missing from this
    /// scan's listing is only dropped after an explicit existence check, so a
    /// transient `read_dir` failure can never reset a live file's cursor.
    fn evict_dead_cursors(&mut self, claude: &[(PathBuf, u64)], codex: &[(PathBuf, u64)]) {
        let listed: HashSet<String> = claude
            .iter()
            .chain(codex.iter())
            .map(|(f, _)| f.to_string_lossy().into_owned())
            .collect();
        let candidates: Vec<String> = self
            .cursors
            .keys()
            .filter(|k| !listed.contains(*k))
            .map(str::to_string)
            .collect();
        if !candidates.is_empty() {
            let dead: HashSet<String> = blocking_io(|| {
                candidates
                    .into_iter()
                    .filter(|k| !Path::new(k).exists())
                    .collect()
            });
            if !dead.is_empty() {
                let removed = self.cursors.retain(|k| !dead.contains(k));
                tracing::debug!("usage tailer: evicted {removed} cursor(s) of deleted transcripts");
            }
        }
        if self.codex_meta.len() > codex.len() {
            let live: HashSet<&PathBuf> = codex.iter().map(|(f, _)| f).collect();
            self.codex_meta.retain(|f, _| live.contains(f));
        }
        if self.last_size.len() > claude.len() + codex.len() {
            let live: HashSet<&PathBuf> =
                claude.iter().chain(codex.iter()).map(|(f, _)| f).collect();
            self.last_size.retain(|f, _| live.contains(f));
        }
    }

    /// List both transcript trees (with sizes) in ONE blocking task.
    async fn list_files(&self) -> (Vec<(PathBuf, u64)>, Vec<(PathBuf, u64)>) {
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || (list_claude_files(&home), list_codex_files(&home)))
            .await
            .unwrap_or_default()
    }

    fn set_cursor(&mut self, file: &Path, offset: u64) {
        self.cursors.set_if_changed(file, offset);
    }

    /// The pass's attribution, querying `sessions` on first use only.
    async fn attribution(&mut self) -> &Attribution {
        if self.attr.is_none() {
            let a = self.build_attribution().await;
            self.attr = Some(a);
        }
        self.attr.get_or_insert_with(Attribution::default)
    }

    /// Rebuild the claude (by provider-session-id) and codex (by cwd)
    /// attribution indexes from the current `sessions` table.
    async fn build_attribution(&mut self) -> Attribution {
        self.stats.attribution_builds += 1;
        let repo = SessionsRepo::new(self.pool.clone());
        let rows = match repo.list_usage_attribution().await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("usage tailer: attribution query failed: {e}");
                return Attribution::default();
            }
        };
        let mut attr = Attribution::default();
        for r in rows {
            let sref = SessionRef {
                otto_session_id: r.id,
                workspace_id: r.workspace_id,
                provider: r.provider,
            };
            if let Some(psid) = r.provider_session_id {
                if !psid.is_empty() {
                    attr.by_provider_session.insert(psid, sref.clone());
                }
            }
            if !r.cwd.is_empty() {
                attr.by_cwd.entry(r.cwd).or_default().push(sref);
            }
        }
        attr
    }

    // ── Claude ────────────────────────────────────────────────────────────────

    /// Tail one claude transcript. Returns `Ok(true)` when it had new lines.
    /// Parsing runs on the blocking pool with the read; reads are capped at
    /// READ_WINDOW per call, so a large catch-up streams in windows.
    async fn tail_claude_file(&mut self, file: &Path, size: u64) -> Result<bool, String> {
        let mut any = false;
        loop {
            let Some((lines, new_offset, more)) =
                self.read_new_lines(file, size, parse_claude_line).await?
            else {
                return Ok(any);
            };
            any = true;

            // Filename stem is the CLI's session uuid (= provider_session_id);
            // a subagent transcript bills to its parent session.
            let stem = claude_session_stem(file);
            let ids = match self.attribution().await.by_provider_session.get(&stem) {
                Some(s) => (s.workspace_id.clone(), s.otto_session_id.clone()),
                None => (EXTERNAL_WORKSPACE.to_string(), stem.clone()),
            };

            // One API response = many lines (content blocks, streamed
            // partials, resume replays), billed once — the folder counts each
            // response once with its FINAL usage (see `ResponseFolder`).
            let mut held = self.held.remove(file);
            let mut out: Vec<ClaudeLine> = Vec::new();
            for (line_start, parsed) in lines {
                self.folder
                    .push(&mut held, parsed, line_start, &mut self.seen, &mut out);
            }
            if let Some(h) = held {
                self.held.insert(file.to_path_buf(), h);
                self.held_attr.insert(file.to_path_buf(), ids.clone());
            } else {
                self.held_attr.remove(file);
            }
            if !out.is_empty() {
                self.seen_dirty = true;
            }
            for line in out {
                self.record_claude(line, &ids);
            }
            self.set_cursor(file, new_offset);
            if !more {
                return Ok(true);
            }
        }
    }

    /// Count every held response whose file passes `which`.
    fn release_held(&mut self, which: impl Fn(&Path) -> bool) {
        let files: Vec<PathBuf> = self.held.keys().filter(|f| which(f)).cloned().collect();
        for f in files {
            let mut held = self.held.remove(&f);
            let ids = self
                .held_attr
                .remove(&f)
                .unwrap_or_else(|| (EXTERNAL_WORKSPACE.to_string(), claude_session_stem(&f)));
            let mut out = Vec::new();
            self.folder.release(&mut held, &mut self.seen, &mut out);
            if !out.is_empty() {
                self.seen_dirty = true;
            }
            // The persisted cursor floor moves on with the next save.
            self.cursors.mark_dirty(&f);
            for line in out {
                self.record_claude(line, &ids);
            }
        }
    }

    fn record_claude(&self, line: ClaudeLine, ids: &(String, String)) {
        let usage = line.usage;
        let cost = estimate_cost(
            &usage.model,
            usage.input,
            usage.output,
            usage.cache_read,
            usage.cache_write,
        );
        self.usage.record(UsageEvent {
            ts: line.timestamp,
            workspace_id: ids.0.clone(),
            session_id: ids.1.clone(),
            provider: "claude".to_string(),
            model: usage.model,
            kind: "completion".to_string(),
            input_tokens: usage.input,
            output_tokens: usage.output,
            cache_read_tokens: usage.cache_read,
            cache_write_tokens: usage.cache_write,
            cost_usd: cost,
            duration_ms: 0,
            ..Default::default()
        });
    }

    // ── Codex ─────────────────────────────────────────────────────────────────

    async fn tail_codex_file(&mut self, file: &Path, size: u64) -> Result<(), String> {
        // Growth check FIRST: an unchanged rollout costs nothing — no head read,
        // no 18 KB session_meta parse.
        let cursor = self.cursors.get(file).unwrap_or(0);
        if cursor == size {
            return Ok(());
        }
        // The session_meta (id + cwd + model) lives on the first line and never
        // changes; read it once per file (just the head) and cache it.
        let meta = match self.codex_meta.get(file) {
            Some(m) => Some(m.clone()),
            None => {
                let m = read_codex_meta(file).await;
                if let Some(m) = &m {
                    self.codex_meta.insert(file.to_path_buf(), m.clone());
                }
                m
            }
        };
        let cwd = meta.as_ref().and_then(|m| m.cwd.clone());
        let thread_uuid = codex_thread_uuid(file);
        let codex_session_id = meta
            .as_ref()
            .and_then(|m| m.session_id.clone())
            .unwrap_or_else(|| thread_uuid.clone());
        let model = meta
            .as_ref()
            .and_then(|m| m.model.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| CODEX_FALLBACK_MODEL.to_string());

        loop {
            let m = model.clone();
            let Some((totals, new_offset, more)) = self
                .read_new_lines(file, size, move |l| parse_codex_line(l, &m))
                .await?
            else {
                return Ok(());
            };
            // Attribute by cwd → only when exactly one codex session matches.
            let sref = match cwd.as_deref() {
                Some(c) => self.attribution().await.by_cwd.get(c).and_then(|sessions| {
                    let codex: Vec<&SessionRef> =
                        sessions.iter().filter(|s| s.provider == "codex").collect();
                    if codex.len() == 1 {
                        Some(codex[0].clone())
                    } else {
                        None
                    }
                }),
                None => None,
            };
            for (_, total) in totals {
                self.codex_counters_dirty = true;
                let Some(parsed) = self.codex_counters.apply(&codex_session_id, &total) else {
                    continue;
                };
                let (workspace_id, session_id) = match &sref {
                    Some(s) => (s.workspace_id.clone(), s.otto_session_id.clone()),
                    None => (EXTERNAL_WORKSPACE.to_string(), thread_uuid.clone()),
                };
                let cost = estimate_cost(
                    &parsed.model,
                    parsed.input,
                    parsed.output,
                    parsed.cache_read,
                    parsed.cache_write,
                );
                self.usage.record(UsageEvent {
                    workspace_id,
                    session_id,
                    provider: "codex".to_string(),
                    model: parsed.model,
                    kind: "completion".to_string(),
                    input_tokens: parsed.input,
                    output_tokens: parsed.output,
                    cache_read_tokens: parsed.cache_read,
                    cache_write_tokens: parsed.cache_write,
                    cost_usd: cost,
                    duration_ms: 0,
                    ..Default::default()
                });
            }
            self.set_cursor(file, new_offset);
            if !more {
                return Ok(());
            }
        }
    }

    // ── Shared I/O ──────────────────────────────────────────────────────────

    /// Read (and `parse`, on the blocking pool) the complete lines of `file`
    /// from its cursor, at most READ_WINDOW bytes per call. Returns each
    /// parsed line with its start offset, the offset just past the last
    /// consumed newline, and whether more bytes remain past the window.
    /// `Ok(None)` when there's nothing new (or only a partial trailing line).
    /// Handles truncation/rotation by restarting from 0. `size` comes from the
    /// pass's stat.
    async fn read_new_lines<T: Send + 'static>(
        &mut self,
        file: &Path,
        size: u64,
        parse: impl Fn(&str) -> Option<T> + Send + 'static,
    ) -> Result<Option<(Vec<(u64, T)>, u64, bool)>, String> {
        let mut cursor = self.cursors.get(file).unwrap_or(0);
        if cursor > size {
            // Truncated / rotated under us — restart from the top.
            cursor = 0;
        }
        if size <= cursor {
            return Ok(None);
        }
        self.stats.range_reads += 1;
        let path = file.to_path_buf();
        tokio::task::spawn_blocking(move || {
            read_lines_window(&path, cursor, size, READ_WINDOW, parse)
        })
        .await
        .map_err(|e| format!("join: {e}"))?
    }
}

/// FSEvents (notify) watcher over the transcript roots, feeding a dirty set.
struct TranscriptWatch {
    watcher: notify::RecommendedWatcher,
    watched: HashSet<PathBuf>,
}

impl TranscriptWatch {
    /// `None` when the platform watcher can't be created (→ 20 s polling).
    fn new(dirty: Arc<std::sync::Mutex<DirtySet>>, wake: Arc<Notify>) -> Option<Self> {
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let mut d = dirty.lock().unwrap_or_else(|e| e.into_inner());
            match res {
                Ok(ev) => {
                    if matches!(ev.kind, notify::EventKind::Access(_)) {
                        return;
                    }
                    if ev.need_rescan() {
                        d.rescan = true;
                    }
                    let mut hit = ev.need_rescan();
                    for p in ev.paths {
                        if p.extension().is_some_and(|x| x == "jsonl") {
                            d.paths.insert(p);
                            hit = true;
                        }
                    }
                    if !hit {
                        return;
                    }
                }
                Err(_) => d.rescan = true,
            }
            drop(d);
            wake.notify_one();
        })
        .map_err(|e| tracing::warn!("usage tailer: no FSEvents watcher ({e}); polling every 20 s"))
        .ok()?;
        Some(Self {
            watcher,
            watched: HashSet::new(),
        })
    }

    /// Watch every root that exists and isn't watched yet (a missing
    /// `~/.codex/sessions` is picked up once the CLI creates it).
    fn ensure_roots(&mut self, roots: &[PathBuf]) {
        use notify::Watcher;
        for root in roots {
            if self.watched.contains(root) || !root.is_dir() {
                continue;
            }
            match self.watcher.watch(root, notify::RecursiveMode::Recursive) {
                Ok(()) => {
                    self.watched.insert(root.clone());
                }
                Err(e) => tracing::warn!("usage tailer: cannot watch {}: {e}", root.display()),
            }
        }
    }
}

/// `<claude_root>/<project>/<sid>.jsonl` or
/// `<claude_root>/<project>/<sid>/subagents/<agent>.jsonl`.
fn is_claude_transcript(root: &Path, p: &Path) -> bool {
    let Ok(rel) = p.strip_prefix(root) else {
        return false;
    };
    if rel.extension().is_none_or(|x| x != "jsonl") {
        return false;
    }
    let parts: Vec<_> = rel.components().collect();
    match parts.len() {
        2 => true,
        4 => parts[2].as_os_str() == "subagents",
        _ => false,
    }
}

/// `<codex_root>/YYYY/MM/DD/rollout-*.jsonl`.
fn is_codex_rollout(root: &Path, p: &Path) -> bool {
    let Ok(rel) = p.strip_prefix(root) else {
        return false;
    };
    rel.components().count() == 4
        && rel.extension().is_some_and(|x| x == "jsonl")
        && rel
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rollout-"))
}

/// One bounded window of `file[cursor..size]`: complete lines only, parsed.
/// A single line longer than the window widens the read to the next newline
/// (or EOF), so a giant tool result can't wedge the cursor. Blocking.
#[allow(clippy::type_complexity)]
fn read_lines_window<T>(
    path: &Path,
    cursor: u64,
    size: u64,
    mut window: u64,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<(Vec<(u64, T)>, u64, bool)>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(cursor)).map_err(|e| e.to_string())?;
    let mut buf: Vec<u8> = Vec::new();
    let (last_nl, end) = loop {
        let end = size.min(cursor.saturating_add(window));
        let want = (end - cursor) as usize;
        let have = buf.len();
        buf.resize(want, 0);
        f.read_exact(&mut buf[have..]).map_err(|e| e.to_string())?;
        if let Some(pos) = buf.iter().rposition(|&b| b == b'\n') {
            break (pos, end);
        }
        if end >= size {
            return Ok(None); // no complete line yet
        }
        window = window.saturating_mul(2);
    };
    buf.truncate(last_nl + 1);
    let consumed = cursor + buf.len() as u64;
    // More only when the window — not a trailing partial line — stopped us.
    let more = end < size;
    // One copy at most: valid UTF-8 (the norm) is reused in place.
    let text = match String::from_utf8(buf) {
        Ok(t) => t,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    let mut out = Vec::new();
    let mut offset = cursor;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len() as u64;
        if let Some(v) = parse(line) {
            out.push((start, v));
        }
    }
    Ok(Some((out, consumed, more)))
}

// ---------------------------------------------------------------------------
// Free helpers (filesystem, run on blocking threads to keep the loop snappy)
// ---------------------------------------------------------------------------

/// Run short synchronous file I/O (state-file saves, existence checks) without
/// parking a runtime worker: `block_in_place` hands the worker's other tasks
/// off on the multi-thread runtime; on a current-thread runtime (unit tests)
/// it just runs inline.
fn blocking_io<R>(f: impl FnOnce() -> R) -> R {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

/// All claude transcript files with their sizes:
/// `<home>/.claude/projects/*/*.jsonl` plus subagent transcripts
/// `<home>/.claude/projects/*/*/subagents/*.jsonl`. Blocking — call off the
/// runtime.
fn list_claude_files(home: &Path) -> Vec<(PathBuf, u64)> {
    let root = home.join(".claude").join("projects");
    let mut out = Vec::new();
    for project in read_subdirs(&root) {
        out.extend(read_files_with_ext(&project, "jsonl", |_| true));
        for session in read_subdirs(&project) {
            out.extend(read_files_with_ext(
                &session.join("subagents"),
                "jsonl",
                |_| true,
            ));
        }
    }
    out
}

/// The claude session a transcript bills to: the file stem for a top-level
/// `<sid>.jsonl`, the parent `<sid>` for `<sid>/subagents/<agent>.jsonl`.
fn claude_session_stem(file: &Path) -> String {
    let parent = file.parent();
    if parent.and_then(|p| p.file_name()).and_then(|n| n.to_str()) == Some("subagents") {
        if let Some(sid) = parent.and_then(|p| p.parent()).and_then(|p| p.file_name()) {
            return sid.to_string_lossy().into_owned();
        }
    }
    file.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// All codex rollout files with their sizes:
/// `<home>/.codex/sessions/*/*/*/rollout-*.jsonl`. Blocking — call off the
/// runtime.
fn list_codex_files(home: &Path) -> Vec<(PathBuf, u64)> {
    let root = home.join(".codex").join("sessions");
    let mut out = Vec::new();
    for y in read_subdirs(&root) {
        for m in read_subdirs(&y) {
            for d in read_subdirs(&m) {
                out.extend(read_files_with_ext(&d, "jsonl", |n| {
                    n.starts_with("rollout-")
                }));
            }
        }
    }
    out
}

/// Immediate subdirectories of `dir` (empty on any error).
fn read_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect()
}

/// Files in `dir` with the given extension whose name passes `name_ok`, with
/// their sizes (non-recursive; empty on any error; unstat-able files skipped).
fn read_files_with_ext(
    dir: &Path,
    ext: &str,
    name_ok: impl Fn(&str) -> bool,
) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some(ext) {
                return None;
            }
            if !p
                .file_name()
                .and_then(|n| n.to_str())
                .map(&name_ok)
                .unwrap_or(false)
            {
                return None;
            }
            // `metadata` follows symlinks like the old `tokio::fs::metadata`.
            let len = std::fs::metadata(&p).ok()?.len();
            Some((p, len))
        })
        .collect()
}

/// Events handed from the rebuild scan to the inserter per batch.
const REBUILD_BATCH: usize = 5_000;

/// Stream `file`'s complete lines (up to its last `\n`) through `f(offset,
/// line)` with a bounded buffer. Returns the offset just past the last
/// newline (0 if none). Blocking.
fn for_each_complete_line(file: &Path, mut f: impl FnMut(u64, &str)) -> std::io::Result<u64> {
    use std::io::BufRead;
    let file = std::fs::File::open(file)?;
    let mut r = std::io::BufReader::with_capacity(256 * 1024, file);
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut offset = 0u64;
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            // EOF, or a trailing partial line still being written.
            return Ok(offset);
        }
        let line = String::from_utf8_lossy(&buf);
        f(offset, &line);
        offset += n as u64;
        if buf.capacity() > 8 * 1024 * 1024 {
            buf = Vec::with_capacity(64 * 1024); // drop a one-off giant line
        }
    }
}

/// Rebuild pass 1: how many usage lines exist and the oldest event date.
fn rebuild_min_date(files: &[PathBuf]) -> (usize, Option<String>) {
    let mut n = 0usize;
    let mut min_date: Option<String> = None;
    for file in files {
        let _ = for_each_complete_line(file, |_, line| {
            let Some(parsed) = parse_claude_line(line) else {
                return;
            };
            n += 1;
            if let Some(date) = parsed.timestamp.as_deref().and_then(|t| t.get(..10)) {
                if min_date.as_deref().map(|m| date < m).unwrap_or(true) {
                    min_date = Some(date.to_string());
                }
            }
        });
    }
    (n, min_date)
}

/// What rebuild pass 2 hands back to the tailer once every batch was sent.
#[derive(Default)]
struct RebuildOut {
    /// False when the inserter hung up (an insert failed) mid-scan.
    completed: bool,
    offsets: Vec<(PathBuf, u64)>,
    /// Every counted response key's hash, oldest first (for the seen set).
    keys: Vec<u128>,
    /// The newest counted responses (for late-line corrections).
    recent: std::collections::VecDeque<(String, ClaudeLine)>,
}

/// Rebuild pass 2: parse every file, fold each response's lines into its
/// FINAL (field-wise max) usage, count each response once across files, and
/// send the events in batches. Blocking.
///
/// A response's lines live in one file (replays into a resumed session carry
/// the same final usage), so open responses are folded per file and only a
/// 16-byte hash per counted key is kept across files.
fn rebuild_events(
    files: &[PathBuf],
    attr: &HashMap<String, (String, String)>,
    tx: tokio::sync::mpsc::Sender<Vec<UsageEvent>>,
) -> RebuildOut {
    let mut out = RebuildOut::default();
    let mut counted: HashSet<u128> = HashSet::new();
    let mut batch: Vec<UsageEvent> = Vec::with_capacity(REBUILD_BATCH);
    for file in files {
        let stem = claude_session_stem(file);
        let (workspace_id, session_id) = attr
            .get(&stem)
            .cloned()
            .unwrap_or_else(|| (EXTERNAL_WORKSPACE.to_string(), stem.clone()));
        // This file's responses in first-seen order; key hash → index.
        let mut lines: Vec<(Option<String>, ClaudeLine)> = Vec::new();
        let mut open: HashMap<u128, usize> = HashMap::new();
        let res = for_each_complete_line(file, |_, line| {
            let Some(parsed) = parse_claude_line(line) else {
                return;
            };
            match parsed.dedup_key.as_deref().map(otto_usage::seen_key_hash) {
                Some(h) if counted.contains(&h) => {} // replay of an earlier file
                Some(h) => match open.get(&h) {
                    Some(&i) => otto_usage::merge_max(&mut lines[i].1, &parsed),
                    None => {
                        open.insert(h, lines.len());
                        lines.push((parsed.dedup_key.clone(), parsed));
                    }
                },
                None => lines.push((None, parsed)),
            }
        });
        let end = match res {
            Ok(end) => end,
            Err(e) => {
                tracing::debug!("usage tailer: rebuild skipped {}: {e}", file.display());
                continue;
            }
        };
        out.offsets.push((file.clone(), end));
        for (key, line) in lines {
            if let Some(k) = &key {
                let h = otto_usage::seen_key_hash(k);
                counted.insert(h);
                out.keys.push(h);
                out.recent.push_back((k.clone(), line.clone()));
                if out.recent.len() > RECENT_RESPONSES_CAP {
                    out.recent.pop_front();
                }
            }
            let u = &line.usage;
            batch.push(UsageEvent {
                ts: line.timestamp.clone(),
                workspace_id: workspace_id.clone(),
                session_id: session_id.clone(),
                provider: "claude".to_string(),
                model: u.model.clone(),
                kind: "completion".to_string(),
                input_tokens: u.input,
                output_tokens: u.output,
                cache_read_tokens: u.cache_read,
                cache_write_tokens: u.cache_write,
                cost_usd: estimate_cost(&u.model, u.input, u.output, u.cache_read, u.cache_write),
                duration_ms: 0,
                ..Default::default()
            });
            if batch.len() >= REBUILD_BATCH {
                let full = std::mem::replace(&mut batch, Vec::with_capacity(REBUILD_BATCH));
                if tx.blocking_send(full).is_err() {
                    return out; // inserter failed: completed stays false
                }
            }
        }
    }
    if !batch.is_empty() && tx.blocking_send(batch).is_err() {
        return out;
    }
    out.completed = true;
    out
}

/// Read just the first line of a codex rollout file and parse its session_meta.
/// Reads a bounded head (the meta line is small) to avoid loading large files.
async fn read_codex_meta(file: &Path) -> Option<otto_usage::CodexMeta> {
    use std::io::Read;
    let path = file.to_path_buf();
    let head = tokio::task::spawn_blocking(move || -> Option<String> {
        let mut f = std::fs::File::open(&path).ok()?;
        // Session meta is the first line; 64 KiB is far more than enough.
        let mut buf = vec![0u8; 64 * 1024];
        let n = f.read(&mut buf).ok()?;
        buf.truncate(n);
        Some(String::from_utf8_lossy(&buf).into_owned())
    })
    .await
    .ok()??;
    let first = head.lines().next()?;
    parse_codex_session_meta(first)
}

/// Search backward through a bounded tail of a rollout for its newest
/// cumulative token snapshot. The first 512 KiB normally contains one; the
/// wider ceiling handles a large final tool result without loading multi-GB
/// rollouts in full.
async fn read_last_codex_usage(file: &Path, model: &str) -> Option<otto_usage::ParsedUsage> {
    use std::io::{Read, Seek, SeekFrom};
    const CHUNK_BYTES: u64 = 512 * 1024;
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    const OVERLAP: u64 = 16 * 1024;
    let path = file.to_path_buf();
    let model = model.to_string();
    tokio::task::spawn_blocking(move || -> Option<otto_usage::ParsedUsage> {
        let mut f = std::fs::File::open(&path).ok()?;
        let size = f.metadata().ok()?.len();
        let floor = size.saturating_sub(MAX_BYTES);
        let mut end = size;
        while end > floor {
            let start = end.saturating_sub(CHUNK_BYTES).max(floor);
            f.seek(SeekFrom::Start(start)).ok()?;
            let mut buf = vec![0u8; (end - start) as usize];
            f.read_exact(&mut buf).ok()?;
            let text = String::from_utf8_lossy(&buf);
            if let Some(total) = text
                .lines()
                .rev()
                .find_map(|line| parse_codex_line(line, &model))
            {
                return Some(total);
            }
            if start == floor {
                break;
            }
            end = start.saturating_add(OVERLAP);
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// Extract the thread uuid from a codex rollout filename:
/// `rollout-<ISO-ts>-<uuid>.jsonl`. The uuid is the tail after the timestamp;
/// since the ts itself contains `-`, we take the canonical 5-group uuid (last
/// 5 dash-separated segments of the stem). Falls back to the whole stem.
fn codex_thread_uuid(file: &Path) -> String {
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 5 {
        // Last 5 segments form the uuid (8-4-4-4-12).
        parts[parts.len() - 5..].join("-")
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_thread_uuid_from_rollout_filename() {
        let f =
            Path::new("/x/rollout-2026-06-18T08-53-25-019ed94a-994a-7010-b01f-9b840c5b7068.jsonl");
        assert_eq!(codex_thread_uuid(f), "019ed94a-994a-7010-b01f-9b840c5b7068");
    }

    #[test]
    fn listings_carry_sizes_and_filter_names() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("otto-usage-list-test-{nonce}"));
        let home = root.as_path();
        let day = home.join(".codex/sessions/2026/09/27");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-a.jsonl"), b"12345").unwrap();
        std::fs::write(day.join("other.jsonl"), b"x").unwrap();
        std::fs::write(day.join("rollout-b.txt"), b"x").unwrap();
        let proj = home.join(".claude/projects/-p");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("s.jsonl"), b"abc").unwrap();
        std::fs::write(proj.join("s.meta.json"), b"{}").unwrap();
        // r3-08-14: subagent transcripts live one level down; tool-results
        // and other session subdirs are not transcripts.
        let subs = proj.join("s/subagents");
        std::fs::create_dir_all(&subs).unwrap();
        std::fs::write(subs.join("agent-x.jsonl"), b"abcd").unwrap();
        std::fs::write(subs.join("agent-x.meta.json"), b"{}").unwrap();
        std::fs::create_dir_all(proj.join("s/tool-results")).unwrap();
        std::fs::write(proj.join("s/tool-results/t.jsonl"), b"z").unwrap();

        let codex = list_codex_files(home);
        assert_eq!(codex, vec![(day.join("rollout-a.jsonl"), 5)]);
        let mut claude = list_claude_files(home);
        claude.sort();
        assert_eq!(
            claude,
            // (Path order compares components: `s` sorts before `s.jsonl`.)
            vec![(subs.join("agent-x.jsonl"), 4), (proj.join("s.jsonl"), 3)]
        );
        // Missing trees are empty, not errors.
        assert!(list_codex_files(&home.join("nope")).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn subagent_transcripts_bill_to_their_parent_session() {
        assert_eq!(
            claude_session_stem(Path::new("/h/.claude/projects/-p/abc-123.jsonl")),
            "abc-123"
        );
        assert_eq!(
            claude_session_stem(Path::new(
                "/h/.claude/projects/-p/abc-123/subagents/agent-a1b2.jsonl"
            )),
            "abc-123"
        );
    }

    fn tmp_root(tag: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("otto-usage-{tag}-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn read_lines_window_streams_and_widens_for_a_giant_line() {
        let root = tmp_root("window");
        let f = root.join("t.jsonl");
        let giant = "x".repeat(50);
        std::fs::write(&f, format!("a\nbb\n{giant}\npartial")).unwrap();
        let size = std::fs::metadata(&f).unwrap().len();
        let parse = |l: &str| Some(l.trim_end().len());
        // Window 4: "a\nbb" → only "a\n" is complete; more remains.
        let (lines, off, more) = read_lines_window(&f, 0, size, 4, parse).unwrap().unwrap();
        assert_eq!((lines, off, more), (vec![(0, 1)], 2, true));
        // From 2 the window holds "bb\n" then the giant line forces widening.
        let (lines, off, more) = read_lines_window(&f, off, size, 4, parse).unwrap().unwrap();
        assert_eq!(lines, vec![(2, 2)]);
        let (lines, off2, _) = read_lines_window(&f, off, size, 4, parse).unwrap().unwrap();
        assert_eq!(lines, vec![(5, 50)]);
        assert_eq!(off2, 56);
        assert!(more);
        // Only a partial trailing line left → nothing to consume.
        assert!(read_lines_window(&f, off2, size, 4, parse)
            .unwrap()
            .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn watcher_paths_are_classified() {
        let c = Path::new("/h/.claude/projects");
        assert!(is_claude_transcript(
            c,
            Path::new("/h/.claude/projects/-p/s.jsonl")
        ));
        assert!(is_claude_transcript(
            c,
            Path::new("/h/.claude/projects/-p/s/subagents/a.jsonl")
        ));
        assert!(!is_claude_transcript(
            c,
            Path::new("/h/.claude/projects/-p/s/tool-results/t.jsonl")
        ));
        assert!(!is_claude_transcript(
            c,
            Path::new("/h/.claude/projects/-p/s.meta.json")
        ));
        assert!(!is_claude_transcript(c, Path::new("/elsewhere/-p/s.jsonl")));
        let x = Path::new("/h/.codex/sessions");
        assert!(is_codex_rollout(
            x,
            Path::new("/h/.codex/sessions/2026/10/03/rollout-a.jsonl")
        ));
        assert!(!is_codex_rollout(
            x,
            Path::new("/h/.codex/sessions/2026/10/03/other.jsonl")
        ));
        assert!(!is_codex_rollout(
            x,
            Path::new("/h/.codex/sessions/2026/10/rollout-a.jsonl")
        ));
    }

    fn assistant_line(id: &str, out: u64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-10-03T10:00:00Z","requestId":"r","message":{{"id":"{id}","model":"claude-opus-4-1","stop_reason":"end_turn","usage":{{"input_tokens":1,"output_tokens":{out}}}}}}}"#
        )
    }

    /// Perf guard (U2/U10): on a realistic tree (2k claude + 1.4k codex
    /// files, 10 ending in a partial line), an unchanged-tree pass must not
    /// query `sessions` or read any file — the old `has_new` compared sizes
    /// against the newline-bounded cursor, so the partial-line files rebuilt
    /// attribution and re-read every 20 s forever.
    #[tokio::test]
    async fn unchanged_tree_pass_does_no_attribution_or_reads() {
        let root = tmp_root("idle");
        let home = root.join("home");
        for p in 0..100 {
            let proj = home.join(format!(".claude/projects/-p{p}"));
            std::fs::create_dir_all(&proj).unwrap();
            for f in 0..20 {
                std::fs::write(proj.join(format!("s{f}.jsonl")), "{}\n").unwrap();
            }
        }
        let day = home.join(".codex/sessions/2026/10/03");
        std::fs::create_dir_all(&day).unwrap();
        for f in 0..1400 {
            std::fs::write(day.join(format!("rollout-{f}.jsonl")), "{}\n").unwrap();
        }
        let pool = otto_state::open(&root.join("t.db")).await.unwrap();
        let usage = UsageEngine::start(
            otto_usage::UsageConfig {
                enabled: false,
                ..Default::default()
            },
            root.join("usage"),
        )
        .await;
        let mut t = UsageTailer::new(usage, pool, root.join("data"), home.clone());
        t.seed_existing_files().await;
        // 10 transcripts grow a partial (newline-less) line after the seed.
        for p in 0..10 {
            let f = home.join(format!(".claude/projects/-p{p}/s0.jsonl"));
            std::fs::write(&f, "{}\n{\"type\":\"assist").unwrap();
        }
        t.scan_once().await.unwrap();
        let after_first = t.stats;
        assert_eq!(
            after_first.attribution_builds, 0,
            "no complete line → no query"
        );
        assert_eq!(after_first.range_reads, 10);

        // Unchanged tree: zero reads, zero attribution — full or event pass.
        t.scan_once().await.unwrap();
        t.scan_paths(HashSet::new()).await.unwrap();
        assert_eq!(t.stats.range_reads, after_first.range_reads);
        assert_eq!(t.stats.attribution_builds, 0);

        // A real usage line → exactly one read and one attribution query.
        let f = home.join(".claude/projects/-p50/s3.jsonl");
        std::fs::write(&f, format!("{{}}\n{}\n", assistant_line("m1", 7))).unwrap();
        t.scan_paths(HashSet::from([f.clone()])).await.unwrap();
        assert_eq!(t.stats.range_reads, after_first.range_reads + 1);
        assert_eq!(t.stats.attribution_builds, 1);
        assert_eq!(
            t.cursors.get(&f),
            std::fs::metadata(&f).ok().map(|m| m.len())
        );
        assert!(t.seen.contains("m1:r"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rebuild_streams_dedups_and_keeps_final_usage() {
        let root = tmp_root("rebuild");
        let a = root.join("a.jsonl");
        let b = root.join("b.jsonl");
        // a: a streamed response (partial then final) + a key-less line.
        let partial =
            assistant_line("m1", 3).replace(r#""stop_reason":"end_turn""#, r#""stop_reason":null"#);
        std::fs::write(
            &a,
            format!(
                "{partial}\n{{\"type\":\"user\"}}\n{}\n",
                assistant_line("m1", 9)
            ),
        )
        .unwrap();
        // b: replays m1 (resume) and adds m2, then a partial trailing line.
        std::fs::write(
            &b,
            format!(
                "{}\n{}\n{{\"x",
                assistant_line("m1", 9),
                assistant_line("m2", 4)
            ),
        )
        .unwrap();
        let files = vec![a.clone(), b.clone()];
        assert_eq!(rebuild_min_date(&files), (4, Some("2026-10-03".into())));
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        let attr = HashMap::new();
        let out = tokio::task::spawn_blocking(move || rebuild_events(&files, &attr, tx))
            .await
            .unwrap();
        let mut events = Vec::new();
        while let Some(batch) = rx.recv().await {
            events.extend(batch);
        }
        assert!(out.completed);
        let outs: Vec<u64> = events.iter().map(|e| e.output_tokens).collect();
        assert_eq!(outs, vec![9, 4], "m1 once with its FINAL usage, m2 once");
        assert_eq!(out.keys.len(), 2);
        let b_len = std::fs::metadata(&b).unwrap().len();
        assert_eq!(
            out.offsets[1],
            (b.clone(), b_len - 3),
            "stops before the partial line"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fsevents_watcher_reports_new_transcripts() {
        let root = tmp_root("watch");
        let root = std::fs::canonicalize(&root).unwrap();
        let dirty: Arc<std::sync::Mutex<DirtySet>> = Arc::default();
        let wake = Arc::new(Notify::new());
        let Some(mut w) = TranscriptWatch::new(Arc::clone(&dirty), wake) else {
            return; // no platform watcher: the 20 s fallback covers it
        };
        w.ensure_roots(std::slice::from_ref(&root));
        std::thread::sleep(Duration::from_millis(300));
        let f = root.join("s.jsonl");
        std::fs::write(&f, "{}\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            {
                let d = dirty.lock().unwrap();
                if d.paths.contains(&f) || d.rescan {
                    break;
                }
            }
            assert!(Instant::now() < deadline, "no FSEvent for {}", f.display());
            std::thread::sleep(Duration::from_millis(50));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn read_last_codex_usage_finds_the_newest_cumulative_snapshot() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("otto-codex-tail-test-{nonce}"));
        std::fs::create_dir_all(&dir).unwrap();
        let file =
            dir.join("rollout-2026-07-12T20-00-00-019ed94a-994a-7010-b01f-9b840c5b7068.jsonl");
        let old = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":5}}}}"#;
        let new = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":160,"cached_input_tokens":120,"output_tokens":9,"reasoning_output_tokens":4}}}}"#;
        let padding = "tool output\n".repeat(60_000);
        std::fs::write(&file, format!("{old}\n{padding}{new}\n")).unwrap();

        let got = read_last_codex_usage(&file, "gpt-5-codex").await.unwrap();
        assert_eq!(got.input, 40);
        assert_eq!(got.cache_read, 120);
        assert_eq!(got.output, 9, "reasoning is already included in output");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
