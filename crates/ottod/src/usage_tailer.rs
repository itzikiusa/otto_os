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
//!     billing happens once.
//!   * **True-time stamping.** Claude events carry the transcript line's own
//!     `timestamp` (`UsageEvent.ts`), so history ingested late (the one-time
//!     rebuild below, or catch-up after daemon downtime) is dated when the API
//!     call actually happened, not when it was ingested. Codex lines carry no
//!     usable per-turn timestamp and keep the insert-time default; their
//!     pre-existing history is still seeded away at startup.
//!   * **One-time dedup rebuild.** The pre-dedup tailer counted every line, so
//!     stores it fed are inflated (~2.4× on real data). On first start after
//!     upgrade (marker `<data_dir>/usage_tailer_dedup_rebuild.done` absent) the
//!     tailer purges its own claude rows and re-derives them from the full
//!     transcripts — deduped, true-time-stamped. Delete-first + marker-last
//!     makes a crashed rebuild retry cleanly on the next start.
//!   * **Crash-resilient.** A bad file/line logs and is skipped; the loop never
//!     panics.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_state::{SessionsRepo, SqlitePool};
use otto_usage::{
    estimate_cost, parse_claude_line, parse_codex_line, parse_codex_session_meta,
    CodexCounterStore, CursorStore, SeenKeys, UsageEngine, UsageEvent, EXTERNAL_WORKSPACE,
};
use tokio::task::JoinHandle;

/// How often to scan for new transcript bytes.
const SCAN_INTERVAL: Duration = Duration::from_secs(20);

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

// ---------------------------------------------------------------------------
// Public handle
// ---------------------------------------------------------------------------

/// Handle returned by [`UsageTailer::start`]. Keep it alive for the process
/// lifetime; dropping it sets the cancel flag and stops the loop.
pub struct UsageTailerHandle {
    cancel: Arc<AtomicBool>,
    _task: JoinHandle<()>,
}

impl Drop for UsageTailerHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Tailer
// ---------------------------------------------------------------------------

pub struct UsageTailer {
    usage: Arc<UsageEngine>,
    pool: SqlitePool,
    /// Home directory — the root of `~/.claude` and `~/.codex`.
    home: PathBuf,
    /// Daemon data dir — holds the cursor/seen files and the rebuild marker.
    data_dir: PathBuf,
    cursors: CursorStore,
    /// Response-level dedup for claude lines (see module docs).
    seen: SeenKeys,
    /// Set when [`Self::scan_once`] recorded a new claude key, so the seen file
    /// is only rewritten when it actually changed.
    seen_dirty: bool,
    /// Session-wide cumulative-token baselines for Codex rollout snapshots.
    codex_counters: CodexCounterStore,
    codex_counters_dirty: bool,
}

/// One Otto session, projected for attribution.
#[derive(Clone)]
struct SessionRef {
    otto_session_id: String,
    workspace_id: String,
    provider: String,
}

/// Attribution indexes, rebuilt fresh each scan from the `sessions` table.
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
    pub fn new(usage: Arc<UsageEngine>, pool: SqlitePool, data_dir: PathBuf, home: PathBuf) -> Self {
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
            seen,
            seen_dirty: false,
            codex_counters,
            codex_counters_dirty: false,
        }
    }

    /// Spawn the background loop. Returns immediately.
    pub fn start(mut self) -> UsageTailerHandle {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_task = Arc::clone(&cancel);
        let task = tokio::spawn(async move {
            // One-time (marker-gated): purge the pre-dedup tailer's inflated
            // claude rows and re-derive them from the full transcripts.
            self.rebuild_claude_history().await;
            // Skip remaining pre-existing history (codex, and any claude file
            // the rebuild couldn't touch) so old turns aren't replayed with a
            // now() timestamp.
            self.seed_existing_files().await;
            loop {
                if cancel_task.load(Ordering::Relaxed) {
                    return;
                }
                if let Err(e) = self.scan_once().await {
                    tracing::warn!("usage tailer: scan failed: {e}");
                }
                // Sleep in short slices so cancellation is responsive.
                let mut slept = Duration::ZERO;
                while slept < SCAN_INTERVAL {
                    if cancel_task.load(Ordering::Relaxed) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    slept += Duration::from_millis(500);
                }
            }
        });
        UsageTailerHandle {
            cancel,
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
        let marker = self.data_dir.join("usage_tailer_dedup_rebuild.done");
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
        // Oldest-first so that, if the seen-set cap ever evicts, it evicts the
        // keys least likely to be replayed again.
        let mut files = self.claude_files();
        files.sort_by_key(|f| {
            std::fs::metadata(f)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });

        let mut events: Vec<UsageEvent> = Vec::new();
        let mut keys: Vec<String> = Vec::new();
        let mut key_set: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut offsets: Vec<(PathBuf, u64)> = Vec::new();
        let mut min_date: Option<String> = None;

        for file in files {
            let Some(size) = file_size(&file).await else {
                continue; // vanished mid-scan
            };
            if size == 0 {
                offsets.push((file, 0));
                continue;
            }
            let bytes = match read_range(&file, 0, size).await {
                Ok(b) => b,
                Err(e) => {
                    tracing::debug!("usage tailer: rebuild skipped {}: {e}", file.display());
                    continue;
                }
            };
            let Some(last_nl) = bytes.iter().rposition(|&b| b == b'\n') else {
                offsets.push((file, 0));
                continue;
            };
            let text = String::from_utf8_lossy(&bytes[..=last_nl]);
            let stem = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let sref = attr.by_provider_session.get(&stem);

            for line in text.lines() {
                let Some(parsed) = parse_claude_line(line) else {
                    continue;
                };
                if let Some(key) = &parsed.dedup_key {
                    if !key_set.insert(key.clone()) {
                        continue; // another line of an already-counted response
                    }
                    keys.push(key.clone());
                }
                if let Some(date) = parsed.timestamp.as_deref().and_then(|t| t.get(..10)) {
                    if min_date.as_deref().map(|m| date < m).unwrap_or(true) {
                        min_date = Some(date.to_string());
                    }
                }
                let (workspace_id, session_id) = match sref {
                    Some(s) => (s.workspace_id.clone(), s.otto_session_id.clone()),
                    None => (EXTERNAL_WORKSPACE.to_string(), stem.clone()),
                };
                let usage = parsed.usage;
                let cost = estimate_cost(
                    &usage.model,
                    usage.input,
                    usage.output,
                    usage.cache_read,
                    usage.cache_write,
                );
                events.push(UsageEvent {
                    ts: parsed.timestamp,
                    workspace_id,
                    session_id,
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
            offsets.push((file, last_nl as u64 + 1));
        }

        if events.is_empty() {
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
             re-ingesting {} deduped events from {} files",
            events.len(),
            offsets.len()
        );
        if let Err(e) = self.usage.purge_claude_tailer_rows(&min_date).await {
            tracing::warn!("usage tailer: dedup rebuild aborted (purge failed): {e}");
            return;
        }
        for chunk in events.chunks(5_000) {
            if let Err(e) = self.usage.insert_events(chunk).await {
                tracing::warn!(
                    "usage tailer: dedup rebuild insert failed (will retry next start): {e}"
                );
                return;
            }
        }
        for (f, off) in &offsets {
            self.cursors.set(f, *off);
        }
        for k in &keys {
            self.seen.insert(k);
        }
        if let Err(e) = self.cursors.save() {
            tracing::warn!("usage tailer: failed to persist cursors after rebuild: {e}");
        }
        if let Err(e) = self.seen.save() {
            tracing::warn!("usage tailer: failed to persist seen keys after rebuild: {e}");
        }
        if let Err(e) = std::fs::write(&marker, format!("rebuilt {} events\n", events.len())) {
            tracing::warn!("usage tailer: failed to write rebuild marker: {e}");
        }
        tracing::info!(
            "usage tailer: dedup rebuild complete — {} events re-ingested",
            events.len()
        );
    }

    /// Seed the cursor for every transcript file that isn't already tracked,
    /// setting it to the file's current size so existing history is skipped.
    /// Codex also records each session's latest cumulative snapshot: otherwise
    /// the first append after upgrading would look like an all-history delta.
    async fn seed_existing_files(&mut self) {
        let mut seeded = 0usize;
        for f in self.claude_files() {
            if self.cursors.contains(&f) {
                continue;
            }
            let size = file_size(&f).await.unwrap_or(0);
            self.cursors.set(&f, size);
            seeded += 1;
        }

        let mut codex_baselines = 0usize;
        let mut catchup_files = 0usize;
        let mut new_baseline_sessions = std::collections::HashSet::new();
        for f in self.codex_files() {
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
                    let size = file_size(&f).await.unwrap_or(0);
                    self.cursors.set(&f, size);
                    seeded += 1;
                }
            }
            if needs_baseline {
                new_baseline_sessions.insert(session_id.clone());
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

    /// One full scan: rebuild attribution, tail both providers, persist cursors.
    async fn scan_once(&mut self) -> Result<(), String> {
        let attr = self.build_attribution().await;

        for file in self.claude_files() {
            if let Err(e) = self.tail_claude_file(&file, &attr).await {
                tracing::debug!("usage tailer: claude file {} skipped: {e}", file.display());
            }
        }
        for file in self.codex_files() {
            if let Err(e) = self.tail_codex_file(&file, &attr).await {
                tracing::debug!("usage tailer: codex file {} skipped: {e}", file.display());
            }
        }

        let mut guards_persisted = true;
        if self.seen_dirty {
            match self.seen.save() {
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
        // Persist provider-level dedup baselines before advancing byte offsets.
        // If a guard write fails, replaying lines is safe in-process and safer
        // across restart than committing a cursor ahead of its dedup state.
        if guards_persisted {
            if let Err(e) = self.cursors.save() {
                tracing::warn!("usage tailer: failed to persist cursors: {e}");
            }
        }
        Ok(())
    }

    /// Rebuild the claude (by provider-session-id) and codex (by cwd)
    /// attribution indexes from the current `sessions` table.
    async fn build_attribution(&self) -> Attribution {
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

    /// All claude transcript files: `~/.claude/projects/*/*.jsonl`.
    fn claude_files(&self) -> Vec<PathBuf> {
        let root = self.home.join(".claude").join("projects");
        let mut out = Vec::new();
        for project in read_subdirs(&root) {
            out.extend(read_files_with_ext(&project, "jsonl"));
        }
        out
    }

    async fn tail_claude_file(&mut self, file: &Path, attr: &Attribution) -> Result<(), String> {
        let (chunk, new_offset) = match self.read_new_bytes(file).await? {
            Some(v) => v,
            None => return Ok(()),
        };

        // Filename stem is the CLI's session uuid (= provider_session_id).
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let sref = attr.by_provider_session.get(&stem);

        for line in chunk.lines() {
            let Some(parsed) = parse_claude_line(line) else {
                continue;
            };
            // One API response = many lines (content blocks, resume replays),
            // billed once — count only the first sighting of its key.
            if let Some(key) = &parsed.dedup_key {
                if !self.seen.insert(key) {
                    continue;
                }
                self.seen_dirty = true;
            }
            let (workspace_id, session_id) = match sref {
                Some(s) => (s.workspace_id.clone(), s.otto_session_id.clone()),
                None => (EXTERNAL_WORKSPACE.to_string(), stem.clone()),
            };
            let usage = parsed.usage;
            let cost = estimate_cost(
                &usage.model,
                usage.input,
                usage.output,
                usage.cache_read,
                usage.cache_write,
            );
            self.usage.record(UsageEvent {
                ts: parsed.timestamp,
                workspace_id,
                session_id,
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

        self.cursors.set(file, new_offset);
        Ok(())
    }

    // ── Codex ─────────────────────────────────────────────────────────────────

    /// All codex transcript files:
    /// `~/.codex/sessions/*/*/*/rollout-*.jsonl`.
    fn codex_files(&self) -> Vec<PathBuf> {
        let root = self.home.join(".codex").join("sessions");
        let mut out = Vec::new();
        for y in read_subdirs(&root) {
            for m in read_subdirs(&y) {
                for d in read_subdirs(&m) {
                    for f in read_files_with_ext(&d, "jsonl") {
                        if f
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("rollout-"))
                            .unwrap_or(false)
                        {
                            out.push(f);
                        }
                    }
                }
            }
        }
        out
    }

    async fn tail_codex_file(&mut self, file: &Path, attr: &Attribution) -> Result<(), String> {
        // The session_meta (id + cwd + model) lives on the first line; read it once
        // (cheaply, just the head) so we can attribute and price every turn.
        let meta = read_codex_meta(file).await;
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

        // Attribute by cwd → only when exactly one codex session matches.
        let sref = cwd.as_deref().and_then(|c| {
            attr.by_cwd.get(c).and_then(|sessions| {
                let codex: Vec<&SessionRef> =
                    sessions.iter().filter(|s| s.provider == "codex").collect();
                if codex.len() == 1 {
                    Some(codex[0].clone())
                } else {
                    None
                }
            })
        });

        let (chunk, new_offset) = match self.read_new_bytes(file).await? {
            Some(v) => v,
            None => return Ok(()),
        };

        for line in chunk.lines() {
            let Some(total) = parse_codex_line(line, &model) else {
                continue;
            };
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

        self.cursors.set(file, new_offset);
        Ok(())
    }

    // ── Shared I/O ──────────────────────────────────────────────────────────

    /// Read the new bytes of `file` from the persisted cursor to EOF, returning
    /// the complete-lines slice and the byte offset of the last consumed
    /// newline. Returns `Ok(None)` when there's nothing new (or only a partial
    /// trailing line). Handles truncation/rotation by resetting the cursor to 0.
    async fn read_new_bytes(&mut self, file: &Path) -> Result<Option<(String, u64)>, String> {
        let size = match file_size(file).await {
            Some(s) => s,
            None => return Ok(None), // file vanished mid-scan
        };
        let mut cursor = self.cursors.get(file).unwrap_or(0);
        if cursor > size {
            // Truncated / rotated under us — restart from the top.
            cursor = 0;
        }
        if size <= cursor {
            return Ok(None);
        }

        let bytes = read_range(file, cursor, size).await?;
        // Only consume up to the last newline; bytes after it are an incomplete
        // line still being written — leave them for the next scan.
        let last_nl = match bytes.iter().rposition(|&b| b == b'\n') {
            Some(pos) => pos,
            None => return Ok(None), // no complete line yet
        };
        let complete = &bytes[..=last_nl];
        let consumed = cursor + complete.len() as u64;
        let text = String::from_utf8_lossy(complete).into_owned();
        Ok(Some((text, consumed)))
    }
}

// ---------------------------------------------------------------------------
// Free helpers (filesystem, run on blocking threads to keep the loop snappy)
// ---------------------------------------------------------------------------

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

/// Files in `dir` with the given extension (non-recursive; empty on any error).
fn read_files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some(ext))
        .collect()
}

/// File size in bytes, or `None` if it can't be stat'd.
async fn file_size(file: &Path) -> Option<u64> {
    tokio::fs::metadata(file).await.ok().map(|m| m.len())
}

/// Read `file[start..end]` on a blocking thread (files can be large; we only
/// ever read the new slice, never the whole file).
async fn read_range(file: &Path, start: u64, end: u64) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let path = file.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let len = end.saturating_sub(start) as usize;
        let mut buf = vec![0u8; len];
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    })
    .await
    .map_err(|e| format!("join: {e}"))?
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
            if let Some(total) = text.lines().rev().find_map(|line| parse_codex_line(line, &model)) {
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
        let f = Path::new(
            "/x/rollout-2026-06-18T08-53-25-019ed94a-994a-7010-b01f-9b840c5b7068.jsonl",
        );
        assert_eq!(
            codex_thread_uuid(f),
            "019ed94a-994a-7010-b01f-9b840c5b7068"
        );
    }

    #[tokio::test]
    async fn read_last_codex_usage_finds_the_newest_cumulative_snapshot() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("otto-codex-tail-test-{nonce}"));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(
            "rollout-2026-07-12T20-00-00-019ed94a-994a-7010-b01f-9b840c5b7068.jsonl",
        );
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
