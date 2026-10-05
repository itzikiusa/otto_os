//! [`UsageEngine`] — the façade the daemon talks to. Owns the ClickHouse
//! handle, a background batch-writer for usage events, the live config, and all
//! the aggregate queries the dashboard reads.
//!
//! The ClickHouse handle + writer live behind an `RwLock` ([`Inner`]) so the
//! engine can be (re)initialized at runtime — e.g. right after the wizard
//! installs or updates the `clickhouse` binary — without a daemon restart.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use otto_core::Result;
use tokio::sync::mpsc;

use crate::clickhouse::ClickHouse;
use crate::metrics::{Metric, MetricsSampler};
use crate::schema;
use crate::types::{
    AttributionDimension, AttributionRow, DailyModelUsage, DailyUsage, FeatureUsage, ForecastReq,
    ForecastResp, MetricPoint, ModelUsage, MonthlyUsage, ProviderUsage, SessionTotals,
    SessionUsage, TokenTotals, UsageConfig, UsageEvent, UsageReport, UsageStatus, UsageSummary,
};

/// Flush the usage buffer at least this often. Every non-empty flush is one
/// INSERT = one new MergeTree part, and each part is later merged into the
/// month's big part — at 15 s the October partition was rewritten 1345 times
/// in three days (S9-01: ~200 rows merged per row inserted). 90 s cuts that
/// ~6×; a dashboard READ asks the writer to flush right away (`flush_req`), so
/// someone watching the Usage page still sees fresh numbers. Shutdown still
/// flushes when the channel closes.
const FLUSH_INTERVAL: Duration = Duration::from_secs(90);
/// A usage flush is BACKGROUND work (S9-02): it never wakes a parked server
/// nor resets its idle clock, or any always-on agent (workflows, bridges,
/// scheduled tasks) kept ClickHouse up 24/7. While parked, events stay
/// buffered until a foreground request wakes the server — or until the oldest
/// has waited this long (or RETAIN_MAX fills), when the writer does one
/// bounded `wake_background` (same policy as telemetry's MAX_DEFER).
const MAX_DEFER: Duration = Duration::from_secs(30 * 60);

/// How long a `session_totals` rollup is reused (see `totals_cache`). Shorter
/// than the writer's flush interval, so it never hides a flush for long.
const SESSION_TOTALS_TTL: Duration = Duration::from_secs(5);

/// `((days, otto_only), usage generation, computed at, rows)`.
type SessionTotalsMemo = (
    (u32, bool),
    u64,
    std::time::Instant,
    Arc<Vec<SessionTotals>>,
);

/// `system_metrics` samples are buffered in memory and inserted together this
/// often (R1a): one part per 5 min instead of one per sample (+ its merges).
/// `metrics()` serves the unflushed tail from memory, so sparklines stay live.
/// The insert is BACKGROUND work: it never wakes an idle-stopped server nor
/// resets its idle clock — while parked the samples stay buffered (bounded by
/// METRICS_BUF_MAX) and go in right after the next foreground wake.
const METRICS_FLUSH_EVERY: Duration = Duration::from_secs(300);
/// Cap on buffered samples across failed flushes (oldest dropped first).
const METRICS_BUF_MAX: usize = 2_000;
/// The sampler runs only while something needs it (R1b): a live session, a
/// recent `/usage/metrics` reader, or usage recorded within this window.
const SAMPLER_WANTED_FOR: Duration = Duration::from_secs(600);
/// Idle-stop the ClickHouse server after this long with no request (R1d).
const IDLE_STOP_AFTER: Duration = Duration::from_secs(15 * 60);
/// How often the idle-stop check runs (a cheap lock + clock read).
const IDLE_CHECK_EVERY: Duration = Duration::from_secs(60);

/// Buffered metrics samples awaiting their batched insert.
#[derive(Default)]
struct MetricsBuf {
    rows: Vec<(chrono::DateTime<chrono::Utc>, Metric)>,
    /// When the oldest buffered sample arrived (the flush clock).
    since: Option<std::time::Instant>,
}
/// …or sooner once this many events are buffered (size-based flush: a burst
/// still becomes one part per ~2k rows, not one per 200).
const FLUSH_BATCH: usize = 2_000;
/// Default cap on the session leaderboard.
const SESSION_LIMIT: u32 = 50;
/// Self-heal signal (perf2/03 N5): an insert path that failed against a DEAD
/// server child raises it, and the watcher task wakes on the `Notify` —
/// event-driven, so an idle daemon has no periodic heal wake-up (it used to
/// poll every 5 s). `store` keeps the old `AtomicBool` call shape so raise
/// sites read the same; storing `true` also wakes the watcher (a permit is
/// kept when nobody waits, so a raise is never lost).
#[derive(Default)]
struct HealSignal {
    flag: AtomicBool,
    notify: tokio::sync::Notify,
}

impl HealSignal {
    fn store(&self, v: bool, order: Ordering) {
        self.flag.store(v, order);
        if v {
            self.notify.notify_one();
        }
    }

    fn swap(&self, v: bool, order: Ordering) -> bool {
        self.flag.swap(v, order)
    }
}
/// How long a measured on-disk size is reused by [`UsageEngine::status`].
const DISK_SIZE_TTL: Duration = Duration::from_secs(300);
/// Cap on events retained across failed flushes while the server is down —
/// beyond this the OLDEST buffered events are dropped (bounded memory beats a
/// perfect record during an outage; the tailer re-derives transcript rows).
const RETAIN_MAX: usize = 20_000;

/// Swappable runtime state: the live ClickHouse handle, the writer channel, and
/// the resolved binary path (kept even when disabled, for status reporting).
#[derive(Default)]
struct Inner {
    ch: Option<Arc<ClickHouse>>,
    tx: Option<mpsc::UnboundedSender<UsageEvent>>,
    bin_path: Option<PathBuf>,
}

pub struct UsageEngine {
    inner: RwLock<Inner>,
    config: RwLock<UsageConfig>,
    data_dir: PathBuf,
    /// Serializes (re)initialization so two `reinit` calls can't both try to
    /// start a server on the same (dir-locked) data dir.
    reinit_lock: tokio::sync::Mutex<()>,
    /// Set by an insert path that failed against a DEAD server child (killed /
    /// crashed out from under us). The self-heal watcher (spawned in
    /// [`Self::start`]) drains it and restarts the server via [`Self::reinit`],
    /// so usage tracking recovers without a daemon restart.
    heal: Arc<HealSignal>,
    /// Last measured ClickHouse on-disk size + when (see [`DISK_SIZE_TTL`]).
    disk_cache: std::sync::Mutex<Option<(std::time::Instant, u64)>>,
    /// Last `session_totals(days, otto_only)` result for SESSION_TOTALS_TTL:
    /// one Usage page open runs it for the summary's top sessions + feature
    /// breakdown and again for budgets a few ms later. An async mutex held
    /// across the uncached query makes it SINGLE-FLIGHT (R5): budgets racing
    /// the summary wait for the one scan instead of starting a second.
    totals_cache: tokio::sync::Mutex<Option<SessionTotalsMemo>>,
    /// `system_metrics` samples not yet inserted (see METRICS_FLUSH_EVERY).
    metrics_buf: std::sync::Mutex<MetricsBuf>,
    /// Unix seconds of the last `metrics()` read / `record()` — gates the
    /// sampler (see [`Self::sampler_wanted`]).
    last_metrics_read: AtomicU64,
    last_record: AtomicU64,
    /// Idle-stop window in seconds (0 = never stop); tests shorten it.
    idle_stop_secs: Arc<AtomicU64>,
    /// Wakes the idle-stop task when the window changes.
    idle_stop_changed: Arc<tokio::sync::Notify>,
    /// Signalled by the ClickHouse handle after a lazy restart: the wake
    /// flusher then writes the metrics held back while it was parked.
    ch_woke: Arc<tokio::sync::Notify>,
    /// Bumped after every successful usage-event insert. Spend only changes
    /// when it moves, so budget checks with an unchanged generation can be
    /// skipped (see [`Self::usage_generation`]) and the totals memo never
    /// serves a rollup computed before a flush.
    usage_gen: Arc<AtomicU64>,
    /// Asks the usage writer to flush its buffer now (S9-01): poked by every
    /// dashboard read ([`Self::ch_read`]) and after a lazy restart, so the
    /// 90 s flush interval never shows a watcher stale numbers for long.
    flush_req: Arc<tokio::sync::Notify>,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Wake the heal watcher and wake flusher so they see the engine is gone and exit.
impl Drop for UsageEngine {
    fn drop(&mut self) {
        self.heal.notify.notify_one();
        // Ends the wake flusher too (its upgrade fails).
        self.ch_woke.notify_one();
    }
}

/// Which sessions a read covers: every recorded session (root), or only the
/// listed session ids (a non-root caller's own sessions).
#[derive(Debug, Clone, Copy, Default)]
pub enum UsageScope<'a> {
    #[default]
    All,
    Sessions(&'a [String]),
}

impl UsageScope<'_> {
    /// SQL fragment narrowing an aggregation to the scope (`AND …` or empty).
    fn filter(&self) -> String {
        match self {
            Self::All => String::new(),
            Self::Sessions([]) => "AND 0".to_string(),
            Self::Sessions(ids) => {
                let list: Vec<String> = ids.iter().map(|i| format!("'{}'", ch_string(i))).collect();
                format!("AND session_id IN ({})", list.join(","))
            }
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Sessions(_) => "own",
        }
    }
}

impl UsageEngine {
    /// Build the engine and kick off ClickHouse bring-up **in the background**.
    /// Returns immediately so a slow first boot (attaching a large pre-existing
    /// dataset) never blocks daemon startup; the engine reports
    /// `available() == false` until the server is healthy + the schema is ready.
    /// Never fails — on any problem it stays a degraded (no-op) engine that can be
    /// revived later via [`Self::reinit`].
    pub async fn start(config: UsageConfig, data_dir: PathBuf) -> Arc<Self> {
        let engine = Arc::new(Self {
            inner: RwLock::new(Inner::default()),
            config: RwLock::new(config.clone()),
            data_dir,
            reinit_lock: tokio::sync::Mutex::new(()),
            heal: Arc::new(HealSignal::default()),
            disk_cache: std::sync::Mutex::new(None),
            totals_cache: tokio::sync::Mutex::new(None),
            metrics_buf: std::sync::Mutex::new(MetricsBuf::default()),
            last_metrics_read: AtomicU64::new(0),
            last_record: AtomicU64::new(0),
            idle_stop_secs: Arc::new(AtomicU64::new(IDLE_STOP_AFTER.as_secs())),
            idle_stop_changed: Arc::new(tokio::sync::Notify::new()),
            ch_woke: Arc::new(tokio::sync::Notify::new()),
            usage_gen: Arc::new(AtomicU64::new(0)),
            flush_req: Arc::new(tokio::sync::Notify::new()),
        });
        let bg = Arc::clone(&engine);
        tokio::spawn(async move {
            bg.reinit(config).await;
        });
        // Self-heal watcher: when an insert path flags the server child as dead
        // (see `heal`), restart it. Weak so the watcher never keeps a dropped
        // engine alive; exits with it.
        let weak: Weak<Self> = Arc::downgrade(&engine);
        let heal = Arc::clone(&engine.heal);
        tokio::spawn(async move {
            loop {
                // No timer: sleeps until a raise (or the engine's Drop) wakes it.
                heal.notify.notified().await;
                let Some(engine) = weak.upgrade() else { break };
                if engine.heal.swap(false, Ordering::SeqCst) {
                    tracing::warn!(
                        "usage: clickhouse server died under the engine — restarting it"
                    );
                    let cfg = engine.config();
                    engine.reinit(cfg).await;
                }
            }
        });
        // Wake flusher: metrics are background writes that never wake a parked
        // server, so they wait for a foreground request to wake it and go in
        // right after. No timer — sleeps until a wake (or exits with the engine).
        let weak: Weak<Self> = Arc::downgrade(&engine);
        let woke = Arc::clone(&engine.ch_woke);
        tokio::spawn(async move {
            loop {
                woke.notified().await;
                let Some(engine) = weak.upgrade() else { break };
                // Usage events held while parked go in too.
                engine.flush_req.notify_one();
                match engine.flush_metrics().await {
                    Ok(0) => {}
                    Ok(n) => tracing::debug!("usage: wrote {n} metrics held while idle-stopped"),
                    Err(e) => tracing::debug!("usage: post-wake metrics flush failed: {e}"),
                }
            }
        });
        engine
    }

    /// (Re)initialize the ClickHouse handle from `config`. The persistent server
    /// locks the data dir, so this STOPS any previous server first (drop the
    /// writer channel → ends its task; shutdown the old server → releases the
    /// lock) BEFORE starting a fresh one. Serialized + safe to call repeatedly.
    pub async fn reinit(&self, config: UsageConfig) {
        let _g = self.reinit_lock.lock().await;
        let ch_dir = self.data_dir.join("clickhouse");
        let bin_path = ClickHouse::locate(config.clickhouse_path.as_deref());

        // Stop the previous server FIRST (outside the new-server start) so the
        // dir lock is released before we bind a new one.
        let prev = {
            let mut inner = self.inner.write().expect("usage inner lock");
            inner.tx = None; // dropping the sender ends the writer task
            inner.ch.take()
        };
        if let Some(old) = prev {
            old.shutdown().await;
        }

        let (ch, tx) = if config.enabled {
            match &bin_path {
                Some(bin) => match ClickHouse::start(bin.clone(), ch_dir.clone()).await {
                    Ok(server) => {
                        let ch = Arc::new(server);
                        match ch.exec(&schema::schema_sql(config.retention_days)).await {
                            Ok(()) => {
                                // Additive migration: add work-attribution columns to
                                // tables created before B1. IF NOT EXISTS makes this
                                // idempotent; errors are warnings so a quirky build
                                // doesn't prevent the engine from starting.
                                if let Err(e) = ch.exec(&schema::add_workref_columns_sql()).await {
                                    tracing::warn!(
                                        "usage: add workref columns failed (non-fatal): {e}"
                                    );
                                }
                                // Only alter when the window changed: each MODIFY
                                // TTL used to queue a part-rewriting mutation on
                                // every boot and self-heal.
                                if let Err(e) = ensure_ttl(&ch, config.retention_days).await {
                                    tracing::warn!("usage: modify ttl failed (non-fatal): {e}");
                                }
                                // One-time: tables created before the DDL had
                                // PARTITION BY are rebuilt partitioned — BEFORE
                                // the writer starts and before `available()`, so
                                // nothing inserts mid-copy and the tailer's
                                // rebuild purge only touches affected months.
                                ensure_partitioned(&ch, config.retention_days).await;
                                if let Err(e) = ch.exec(schema::parts_lifetime_sql()).await {
                                    tracing::warn!(
                                        "usage: old_parts_lifetime setting failed (non-fatal): {e}"
                                    );
                                }
                                ch.set_wake_hook(Arc::clone(&self.ch_woke));
                                let (tx, rx) = mpsc::unbounded_channel();
                                spawn_writer(
                                    Arc::clone(&ch),
                                    rx,
                                    Arc::clone(&self.heal),
                                    Arc::clone(&self.usage_gen),
                                    Arc::clone(&self.flush_req),
                                );
                                spawn_idle_stopper(
                                    Arc::downgrade(&ch),
                                    Arc::clone(&self.idle_stop_secs),
                                    Arc::clone(&self.idle_stop_changed),
                                );
                                tracing::info!(
                                    "usage: clickhouse server ready at {} (binary {})",
                                    ch.data_dir().display(),
                                    bin.display()
                                );
                                (Some(ch), Some(tx))
                            }
                            Err(e) => {
                                tracing::warn!("usage: clickhouse schema init failed: {e}");
                                ch.shutdown().await;
                                (None, None)
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("usage: clickhouse server start failed: {e}");
                        (None, None)
                    }
                },
                None => {
                    tracing::info!("usage: clickhouse binary not found — usage tracking disabled");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        let became_available = ch.is_some();
        let ch_for_compact = ch.clone();
        *self.config.write().expect("usage config lock") = config.clone();
        *self.inner.write().expect("usage inner lock") = Inner { ch, tx, bin_path };

        // One-time compaction + retention enforcement of a pre-existing bloated
        // dataset (e.g. the 25k-part, 1.4 GB dir from the old per-query model).
        // Marker-guarded so it runs at most once; background so it never blocks.
        if became_available {
            if let Some(ch) = ch_for_compact {
                let retention = config.retention_days;
                tokio::spawn(async move {
                    maybe_compact(ch, ch_dir, retention).await;
                });
            }
        }
    }

    /// Test seam: raise the self-heal signal as a failed insert would.
    #[cfg(test)]
    fn raise_heal(&self) {
        self.heal.store(true, Ordering::SeqCst);
    }

    /// Stop the ClickHouse server cleanly (SIGTERM → flush). Called from the
    /// daemon's graceful-shutdown path so the dir lock is released and the next
    /// daemon start doesn't have to reclaim an orphan.
    pub async fn shutdown(&self) {
        // Buffered metric samples go in before the server stops — even when
        // it is idle-stopped (the one place a metrics write may wake it).
        if let Err(e) = self.flush_metrics_with(true).await {
            tracing::debug!("usage: final metrics flush failed: {e}");
        }
        let ch = {
            let mut inner = self.inner.write().expect("usage inner lock");
            inner.tx = None;
            inner.ch.take()
        };
        if let Some(ch) = ch {
            ch.shutdown().await;
        }
    }

    /// True when usage tracking is live (server up + schema created).
    pub fn available(&self) -> bool {
        self.inner.read().expect("usage inner lock").ch.is_some()
    }

    /// Poll until the engine is available (server up + schema ready) or `timeout`
    /// elapses; returns the final availability. Useful right after [`Self::start`],
    /// whose ClickHouse bring-up runs asynchronously so the daemon never blocks.
    pub async fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.available() {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return self.available();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn ch(&self) -> Option<Arc<ClickHouse>> {
        self.inner.read().expect("usage inner lock").ch.clone()
    }

    /// [`Self::ch`] for a dashboard READ of `usage_events`: also asks the
    /// writer to flush what it buffered, so a watcher (who keeps the server
    /// awake anyway) sees fresh rows by the next poll.
    fn ch_read(&self) -> Option<Arc<ClickHouse>> {
        self.flush_req.notify_one();
        self.ch()
    }

    fn config(&self) -> UsageConfig {
        self.config.read().expect("usage config lock").clone()
    }

    // ── Recording ──────────────────────────────────────────────────────────

    /// Queue one event for buffered insertion (fire-and-forget; dropped if the
    /// engine is disabled).
    pub fn record(&self, ev: UsageEvent) {
        self.last_record.store(unix_now(), Ordering::Relaxed);
        let tx = self.inner.read().expect("usage inner lock").tx.clone();
        if let Some(tx) = tx {
            let _ = tx.send(ev);
        }
    }

    /// Insert events synchronously (used by the writer and by tests). No-op
    /// when disabled.
    pub async fn insert_events(&self, events: &[UsageEvent]) -> Result<()> {
        let Some(ch) = self.ch() else { return Ok(()) };
        ch.insert_ndjson("usage_events", &ndjson(events)).await?;
        if !events.is_empty() {
            self.usage_gen.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Changes whenever usage events were written (writer flush or
    /// [`Self::insert_events`]). Spend cannot move while it stays the same,
    /// so the budget sampler skips its scan (perf3 N1) — which would
    /// otherwise wake an idle-stopped server every metrics tick.
    pub fn usage_generation(&self) -> u64 {
        self.usage_gen.load(Ordering::SeqCst)
    }

    /// Delete the transcript-tailer's claude `completion` rows on or after
    /// `min_event_date` (`YYYY-MM-DD`), synchronously (`mutations_sync = 2`).
    ///
    /// Used by the daemon's one-time dedup rebuild: the pre-dedup tailer
    /// over-counted multi-line responses, so its rows are purged and re-derived
    /// from the transcripts. Tailer rows are exactly the claude completions
    /// with *no* work-graph dims (every other writer of token rows sets at
    /// least one); dim-carrying rows and other providers are untouched. Errors
    /// on a malformed date rather than interpolating it into SQL.
    pub async fn purge_claude_tailer_rows(&self, min_event_date: &str) -> Result<()> {
        let valid = min_event_date.len() == 10
            && min_event_date
                .chars()
                .zip("0000-00-00".chars())
                .all(|(c, m)| {
                    if m == '-' {
                        c == '-'
                    } else {
                        c.is_ascii_digit()
                    }
                });
        if !valid {
            return Err(otto_core::Error::Internal(format!(
                "purge: bad min_event_date {min_event_date:?}"
            )));
        }
        let Some(ch) = self.ch() else { return Ok(()) };
        ch.exec(&format!(
            "ALTER TABLE usage_events DELETE WHERE provider = 'claude' \
             AND kind = 'completion' \
             AND repo_id = '' AND branch = '' AND pr_number = '' AND story_id = '' \
             AND swarm_task_id = '' AND workflow_id = '' AND channel = '' \
             AND review_id = '' AND origin = '' \
             AND event_date >= '{min_event_date}' \
             SETTINGS mutations_sync = 2"
        ))
        .await?;
        // Spend moved: budgets and the totals memo must not reuse old sums.
        self.usage_gen.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Record one metrics sample. Buffered in memory and inserted with the
    /// others every METRICS_FLUSH_EVERY (and on shutdown) — `metrics()` reads
    /// the unflushed tail from memory. No-op when disabled.
    pub async fn store_metric(&self, m: &Metric) -> Result<()> {
        if self.ch().is_none() {
            return Ok(());
        }
        let due = {
            let mut b = self.metrics_buf.lock().expect("metrics buf lock");
            b.rows.push((chrono::Utc::now(), m.clone()));
            if b.rows.len() > METRICS_BUF_MAX {
                let n = b.rows.len() - METRICS_BUF_MAX;
                b.rows.drain(..n);
            }
            let since = *b.since.get_or_insert_with(std::time::Instant::now);
            since.elapsed() >= METRICS_FLUSH_EVERY
        };
        if due {
            self.flush_metrics().await?;
        }
        Ok(())
    }

    /// Insert every buffered metrics sample as ONE batch. A BACKGROUND write:
    /// while the server is idle-stopped it sends nothing and keeps the rows
    /// (returns `Ok(0)`) — the wake flusher writes them after the next
    /// foreground request restarts the server — and it never resets the idle
    /// clock. A failed insert keeps them (bounded) for the next attempt.
    /// Returns the rows written.
    pub async fn flush_metrics(&self) -> Result<usize> {
        self.flush_metrics_with(false).await
    }

    /// [`Self::flush_metrics`]; `wake` also restarts a parked server first
    /// (shutdown only, so nothing buffered is lost).
    async fn flush_metrics_with(&self, wake: bool) -> Result<usize> {
        let Some(ch) = self.ch() else { return Ok(0) };
        if !wake && ch.is_parked() {
            return Ok(0);
        }
        let rows = {
            let mut b = self.metrics_buf.lock().expect("metrics buf lock");
            b.since = None;
            std::mem::take(&mut b.rows)
        };
        if rows.is_empty() {
            return Ok(0);
        }
        let host = MetricsSampler::host();
        let mut payload = String::new();
        for (ts, m) in &rows {
            let row = serde_json::json!({
                "ts": ts.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "host": host,
                "cpu_pct": m.cpu_pct,
                "mem_used_mb": m.mem_used_mb,
                "mem_total_mb": m.mem_total_mb,
                "mem_pct": m.mem_pct,
                "load_avg_1": m.load_avg_1,
                "process_rss_mb": m.process_rss_mb,
                "process_cpu_pct": m.process_cpu_pct,
                "active_sessions": m.active_sessions,
            });
            payload.push_str(&row.to_string());
            payload.push('\n');
        }
        let res = if wake {
            ch.insert_ndjson("system_metrics", &payload)
                .await
                .map(|()| true)
        } else {
            ch.insert_ndjson_background("system_metrics", &payload)
                .await
        };
        match res {
            Ok(true) => Ok(rows.len()),
            // Parked between the check and the insert: hold them for the wake.
            Ok(false) => {
                self.requeue_metrics(rows);
                Ok(0)
            }
            Err(e) => {
                if !ch.server_alive() {
                    self.heal.store(true, Ordering::SeqCst);
                }
                self.requeue_metrics(rows);
                Err(e)
            }
        }
    }

    /// Put unwritten samples back IN FRONT of any that arrived meanwhile
    /// (oldest dropped past METRICS_BUF_MAX).
    fn requeue_metrics(&self, rows: Vec<(chrono::DateTime<chrono::Utc>, Metric)>) {
        let mut b = self.metrics_buf.lock().expect("metrics buf lock");
        let newer = std::mem::replace(&mut b.rows, rows);
        b.rows.extend(newer);
        if b.rows.len() > METRICS_BUF_MAX {
            let n = b.rows.len() - METRICS_BUF_MAX;
            b.rows.drain(..n);
        }
        b.since.get_or_insert_with(std::time::Instant::now);
    }

    /// Whether the metrics sampler (and its `UsageMetricsTick`, which also
    /// drives budget checks) should run this tick: something is live, a UI
    /// read `/usage/metrics` recently, or usage was recorded recently. An idle
    /// daemon nobody is watching samples nothing and wakes no WS client.
    pub fn sampler_wanted(&self, live_sessions: usize) -> bool {
        let fresh = |stamp: &AtomicU64| {
            unix_now().saturating_sub(stamp.load(Ordering::Relaxed)) < SAMPLER_WANTED_FOR.as_secs()
        };
        live_sessions > 0 || fresh(&self.last_metrics_read) || fresh(&self.last_record)
    }

    /// Override the ClickHouse idle-stop window (`None` = never stop). The
    /// running stopper task reads it each check.
    pub fn set_idle_stop(&self, after: Option<Duration>) {
        self.idle_stop_secs
            .store(after.map_or(0, |d| d.as_secs().max(1)), Ordering::Relaxed);
        self.idle_stop_changed.notify_waiters();
    }

    /// The live ClickHouse handle (perf probes / tests: pid, park state).
    pub fn clickhouse(&self) -> Option<Arc<ClickHouse>> {
        self.ch()
    }

    /// Resolve the shared engine for a local telemetry exporter. A dead child
    /// raises the existing recovery signal; the caller retries with the new
    /// handle and endpoint after the usage engine has recovered.
    pub fn telemetry_clickhouse(&self) -> Result<Arc<ClickHouse>> {
        let ch = self
            .ch()
            .ok_or_else(|| otto_core::Error::Upstream("usage engine offline".into()))?;
        if !ch.server_alive() {
            self.heal.store(true, Ordering::SeqCst);
            return Err(otto_core::Error::Upstream("usage engine recovering".into()));
        }
        Ok(ch)
    }

    // ── Raw passthroughs (sibling stores: k8s monitor) ──────────────────────

    /// Raw DDL/DML for sibling stores. Errors (instead of no-op) when the
    /// engine is degraded so callers can surface "usage engine offline".
    pub async fn exec_sql(&self, sql: &str) -> Result<()> {
        let Some(ch) = self.ch() else {
            return Err(otto_core::Error::Upstream("usage engine offline".into()));
        };
        ch.exec(sql).await
    }

    /// Raw NDJSON insert into `table` (self-heal flagged like `store_metric`).
    pub async fn insert_ndjson(&self, table: &str, ndjson: &str) -> Result<()> {
        let Some(ch) = self.ch() else {
            return Err(otto_core::Error::Upstream("usage engine offline".into()));
        };
        let res = ch.insert_ndjson(table, ndjson).await;
        if res.is_err() && !ch.server_alive() {
            self.heal.store(true, Ordering::SeqCst);
        }
        res
    }

    /// Raw `SELECT … FORMAT JSONEachRow` passthrough.
    pub async fn query_rows(&self, sql: &str) -> Result<Vec<serde_json::Value>> {
        let Some(ch) = self.ch() else {
            return Err(otto_core::Error::Upstream("usage engine offline".into()));
        };
        ch.query_rows(sql).await
    }

    // ── Config ───────────────────────────────────────────────────────────────

    /// Apply a new retention window live (updates in-memory config + alters the
    /// table TTLs). Persisting to `settings` is the caller's job.
    pub async fn set_retention(&self, retention_days: u32) -> Result<()> {
        {
            let mut c = self.config.write().expect("usage config lock");
            c.retention_days = retention_days.max(1);
        }
        if let Some(ch) = self.ch() {
            ensure_ttl(&ch, retention_days).await?;
        }
        Ok(())
    }

    /// Install (or update) ClickHouse via the official one-liner
    /// (`curl https://clickhouse.com/ | sh`), dropping the binary into Otto's
    /// own `bin/` directory, symlinking it onto `PATH` (`~/.local/bin`) so it's
    /// runnable from anywhere, then re-initializing the engine against it. The
    /// download is large (~hundreds of MB) so this can take a while. Returns the
    /// absolute path to the installed binary.
    pub async fn install_clickhouse(&self) -> Result<PathBuf> {
        use otto_core::Error;

        let bin_dir = self.data_dir.join("bin");
        std::fs::create_dir_all(&bin_dir)
            .map_err(|e| Error::Internal(format!("create bin dir: {e}")))?;

        // The official installer drops a `clickhouse` binary in the cwd.
        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("curl -fsSL https://clickhouse.com/ | sh")
            .current_dir(&bin_dir)
            .output()
            .await
            .map_err(|e| Error::Internal(format!("run clickhouse installer: {e}")))?;
        let bin = bin_dir.join("clickhouse");
        if !out.status.success() && !bin.is_file() {
            return Err(Error::Internal(format!(
                "clickhouse install failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        if !bin.is_file() {
            return Err(Error::Internal(
                "clickhouse installer did not produce a binary".into(),
            ));
        }

        // Make it runnable from anywhere by symlinking onto PATH. ottod augments
        // PATH with ~/.local/bin, so a link there is picked up daemon-wide.
        #[cfg(unix)]
        if let Some(home) = dirs::home_dir() {
            let local = home.join(".local/bin");
            if std::fs::create_dir_all(&local).is_ok() {
                let link = local.join("clickhouse");
                let _ = std::fs::remove_file(&link);
                if let Err(e) = std::os::unix::fs::symlink(&bin, &link) {
                    tracing::warn!("usage: could not symlink clickhouse onto PATH: {e}");
                }
            }
        }

        // Adopt the freshly installed binary and bring the engine up against it.
        let mut cfg = self.config();
        cfg.clickhouse_path = Some(bin.display().to_string());
        cfg.enabled = true;
        self.reinit(cfg).await;
        tracing::info!("usage: clickhouse installed at {}", bin.display());
        Ok(bin)
    }

    /// Update the metrics sampling interval in the in-memory config. The
    /// daemon's sampler loop reads it via [`Self::metrics_interval`].
    pub fn set_metrics_interval(&self, secs: u64) {
        let mut c = self.config.write().expect("usage config lock");
        c.metrics_interval_secs = secs.max(5);
    }

    pub fn metrics_interval(&self) -> Duration {
        Duration::from_secs(self.config().metrics_interval_secs.max(5))
    }

    // ── Queries ──────────────────────────────────────────────────────────────

    /// Per-provider rollup over the last `days` (inclusive of today).
    pub async fn provider_usage(&self, days: u32, otto_only: bool) -> Result<Vec<ProviderUsage>> {
        self.rows(&format!(
            "SELECT provider,
                    count() AS events,
                    sum(input_tokens) AS input_tokens,
                    sum(output_tokens) AS output_tokens,
                    sum(cache_read_tokens) AS cache_read_tokens,
                    sum(cache_write_tokens) AS cache_write_tokens,
                    sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                    round(sum(cost_usd), 6) AS cost_usd
             FROM usage_events
             WHERE event_date >= today() - {since} {ws}
             GROUP BY provider
             ORDER BY total_tokens DESC, events DESC",
            since = since(days),
            ws = ws_filter(otto_only)
        ))
        .await
    }

    /// Per-day rollup over the last `days`.
    pub async fn daily_usage(&self, days: u32, otto_only: bool) -> Result<Vec<DailyUsage>> {
        self.rows(&format!(
            "SELECT toString(event_date) AS day,
                    count() AS events,
                    sum(input_tokens) AS input_tokens,
                    sum(output_tokens) AS output_tokens,
                    sum(cache_read_tokens) AS cache_read_tokens,
                    sum(cache_write_tokens) AS cache_write_tokens,
                    sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                    round(sum(cost_usd), 6) AS cost_usd
             FROM usage_events
             WHERE event_date >= today() - {since} {ws}
             GROUP BY event_date
             ORDER BY event_date",
            since = since(days),
            ws = ws_filter(otto_only)
        ))
        .await
    }

    /// Top sessions by token volume over the last `days`.
    pub async fn session_usage(
        &self,
        days: u32,
        limit: u32,
        otto_only: bool,
    ) -> Result<Vec<SessionUsage>> {
        self.rows(&format!(
            "SELECT session_id,
                    any(workspace_id) AS workspace_id,
                    any(provider) AS provider,
                    any(model) AS model,
                    count() AS events,
                    sum(input_tokens) AS input_tokens,
                    sum(output_tokens) AS output_tokens,
                    sum(cache_read_tokens) AS cache_read_tokens,
                    sum(cache_write_tokens) AS cache_write_tokens,
                    sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                    round(sum(cost_usd), 6) AS cost_usd,
                    toString(max(ts)) AS last_active
             FROM usage_events
             WHERE event_date >= today() - {since} {ws}
             GROUP BY session_id
             ORDER BY total_tokens DESC, events DESC
             LIMIT {limit}",
            since = since(days),
            ws = ws_filter(otto_only),
            limit = limit.max(1)
        ))
        .await
    }

    /// Per-session raw sums over the last `days` — every session (no top-N cap),
    /// unenriched. The server classifies these into per-feature buckets for the
    /// by-kind rollup (see [`Self::feature_usage`]).
    pub async fn session_totals(&self, days: u32, otto_only: bool) -> Result<Vec<SessionTotals>> {
        Ok(self.session_totals_shared(days, otto_only).await?.to_vec())
    }

    /// [`Self::session_totals`] without the copy: memoised for
    /// SESSION_TOTALS_TTL and single-flight — the lock is held across the
    /// uncached scan, so concurrent callers share ONE query.
    pub async fn session_totals_shared(
        &self,
        days: u32,
        otto_only: bool,
    ) -> Result<Arc<Vec<SessionTotals>>> {
        let mut memo = self.totals_cache.lock().await;
        // Read BEFORE the scan: a flush landing mid-scan leaves the memo on
        // the older generation, so the next caller re-scans.
        let gen = self.usage_generation();
        if let Some((key, at_gen, at, rows)) = &*memo {
            if *key == (days, otto_only) && *at_gen == gen && at.elapsed() < SESSION_TOTALS_TTL {
                return Ok(Arc::clone(rows));
            }
        }
        let rows = Arc::new(self.session_totals_uncached(days, otto_only).await?);
        *memo = Some((
            (days, otto_only),
            gen,
            std::time::Instant::now(),
            Arc::clone(&rows),
        ));
        Ok(rows)
    }

    async fn session_totals_uncached(
        &self,
        days: u32,
        otto_only: bool,
    ) -> Result<Vec<SessionTotals>> {
        self.rows(&format!(
            "SELECT session_id,
                    any(workspace_id) AS workspace_id,
                    any(provider) AS provider,
                    count() AS events,
                    sum(input_tokens) AS input_tokens,
                    sum(output_tokens) AS output_tokens,
                    sum(cache_read_tokens) AS cache_read_tokens,
                    sum(cache_write_tokens) AS cache_write_tokens,
                    sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                    round(sum(cost_usd), 6) AS cost_usd,
                    topK(1)(model)[1] AS model,
                    toString(max(ts)) AS last_active
             FROM usage_events
             WHERE event_date >= today() - {since} {ws}
             GROUP BY session_id",
            since = since(days),
            ws = ws_filter(otto_only)
        ))
        .await
    }

    /// Fold per-session sums into per-feature buckets using a caller-supplied
    /// `session_id → feature label` classifier. The engine has no view of the
    /// SQLite session metadata that defines a "feature", so the server passes a
    /// closure (mirroring its session-row enrichment). Buckets are returned
    /// sorted by total tokens, then cost. Pricing is untouched — each session's
    /// `cost_usd` is already the per-row sum.
    pub async fn feature_usage(
        &self,
        days: u32,
        otto_only: bool,
        classify: impl Fn(&SessionTotals) -> String,
    ) -> Result<Vec<FeatureUsage>> {
        let totals = self.session_totals_shared(days, otto_only).await?;
        let mut buckets: std::collections::HashMap<String, FeatureUsage> =
            std::collections::HashMap::new();
        for t in totals.iter() {
            let feature = classify(t);
            let b = buckets
                .entry(feature.clone())
                .or_insert_with(|| FeatureUsage {
                    feature,
                    ..Default::default()
                });
            b.events += t.events;
            b.input_tokens += t.input_tokens;
            b.output_tokens += t.output_tokens;
            b.cache_read_tokens += t.cache_read_tokens;
            b.cache_write_tokens += t.cache_write_tokens;
            b.total_tokens += t.total_tokens;
            b.cost_usd += t.cost_usd;
            b.sessions += 1;
        }
        let mut out: Vec<FeatureUsage> = buckets.into_values().collect();
        // Round cost to the same 6 dp the SQL rollups use (sums of rounded
        // per-session values can drift a few ulps).
        for b in &mut out {
            b.cost_usd = (b.cost_usd * 1_000_000.0).round() / 1_000_000.0;
        }
        out.sort_by(|a, b| {
            b.total_tokens
                .cmp(&a.total_tokens)
                .then(b.cost_usd.total_cmp(&a.cost_usd))
        });
        Ok(out)
    }

    // ── Work-graph attribution + forecast (B1) ───────────────────────────────

    /// `GET /usage/attribution?by=<dim>` — GROUP BY one work-graph dimension over
    /// the last `days`. Returns rows sorted by cost descending, filtering out the
    /// empty-string "not set" key so callers only see attributed work. Returns an
    /// empty vec when the engine is disabled or the column doesn't exist yet
    /// (pre-migration installs that haven't restarted will get empty rows for the
    /// new columns until the migration runs — that's correct and safe).
    pub async fn attribution(
        &self,
        dim: &AttributionDimension,
        days: u32,
    ) -> otto_core::Result<Vec<AttributionRow>> {
        let col = dim.column();
        // Guard: the column must be a safe identifier (no injection via the
        // dimension enum — all arms are string literals above).
        let since = since(days);
        // Note: alias `cost_usd` would shadow the column name and trigger
        // NOT_AN_AGGREGATE in ClickHouse's strict alias-resolution mode.
        // Use a distinct alias `cost` and rename client-side via the
        // `AttributionRow.cost_usd` field mapping.
        self.rows(&format!(
            "SELECT {col} AS key,
                    round(sum(cost_usd), 6) AS cost,
                    sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS tokens,
                    uniqExact(session_id) AS sessions
             FROM usage_events
             WHERE event_date >= today() - {since}
               AND {col} != ''
             GROUP BY {col}
             ORDER BY cost DESC, tokens DESC"
        ))
        .await
    }

    /// The model with the most events for `provider` over the last 30 days —
    /// `None` when the engine is down or has no history for it.
    async fn dominant_model(&self, provider: &str) -> Option<String> {
        let ch = self.ch_read()?;
        let provider = ch_string(provider);
        let rows = ch
            .query_rows(&format!(
                "SELECT model, count() AS n
                 FROM usage_events
                 WHERE event_date >= today() - 29
                   AND provider = '{provider}'
                   AND model != ''
                 GROUP BY model
                 ORDER BY n DESC
                 LIMIT 1"
            ))
            .await
            .ok()?;
        rows.first()
            .and_then(|r| r["model"].as_str())
            .filter(|m| !m.is_empty())
            .map(str::to_string)
    }

    /// `POST /usage/forecast` — estimate the cost of a future run from recent
    /// per-run averages. `req.feature` is the usage-kind label ("review",
    /// "product", "agent", …). When `req.est_tokens` is given the projected cost
    /// is priced directly; otherwise it is derived from the average tokens per
    /// session over the last 30 days for the feature + provider pair.
    ///
    /// Returns a [`ForecastResp`] that is always `Ok` — if the engine is down or
    /// has no history the resp carries a "$0 (no data)" basis string.
    pub async fn forecast(&self, req: &ForecastReq) -> ForecastResp {
        // Price a caller-supplied token estimate directly (most actionable path).
        if let Some(est) = req.est_tokens.filter(|&t| t > 0) {
            // Split evenly between input / output for pricing (conservative).
            let half = est / 2;
            // Price by the provider's dominant recent MODEL: the rate card is
            // keyed by model id, and a bare provider name ("claude") matched
            // nothing, so every forecast used the flagship fallback rate.
            let model = self
                .dominant_model(&req.provider)
                .await
                .unwrap_or_else(|| req.provider.clone());
            let cost = crate::pricing::estimate_cost(&model, half, half, 0, 0);
            return ForecastResp {
                projected_cost_usd: cost,
                basis: format!(
                    "priced {est} estimated tokens ({half} in / {half} out) at {model} rates"
                ),
            };
        }

        // No explicit token count — derive from recent-run averages for this
        // feature + provider pair over the last 30 days. We query the per-session
        // totals and compute the average cost per session for matching sessions.
        let Some(ch) = self.ch_read() else {
            return ForecastResp {
                projected_cost_usd: 0.0,
                basis: "usage engine not available".into(),
            };
        };

        // We need to join with the SQLite session metadata to know a session's
        // feature label — but the engine has no SQLite access. Instead we use
        // the `origin` dimension (stamped by the runner) as a proxy. "review" →
        // origin="review"; "product" → "product"; plain sessions → "manual".
        // This is best-effort: un-attributed sessions return the "no data" path.
        let feature = ch_string(&req.feature);
        let provider = ch_string(&req.provider);

        // Per-session totals subquery — one row per session_id, all within the
        // last 30 days, matching origin + provider. The outer query averages them.
        // Uses explicit column aliases that don't collide with column names.
        let inner_sql = format!(
            "SELECT session_id,
                    round(sum(cost_usd), 6) AS sess_cost,
                    sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS sess_tokens
             FROM usage_events
             WHERE event_date >= today() - 29
               AND origin = '{feature}'
               AND provider = '{provider}'
             GROUP BY session_id
             HAVING count() > 0"
        );
        let agg_sql = format!(
            "SELECT count() AS n,
                    avg(sess_cost) AS mean_cost,
                    avg(sess_tokens) AS mean_tokens
             FROM ({inner_sql}) AS s"
        );

        let rows: Vec<serde_json::Value> = match ch.query_rows(&agg_sql).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("usage: forecast query failed: {e}");
                return ForecastResp {
                    projected_cost_usd: 0.0,
                    basis: "forecast query failed".into(),
                };
            }
        };

        let row = rows.first();
        let n = row.and_then(|r| r["n"].as_u64()).unwrap_or(0);
        let mean_cost = row.and_then(|r| r["mean_cost"].as_f64()).unwrap_or(0.0);
        let mean_tokens = row.and_then(|r| r["mean_tokens"].as_f64()).unwrap_or(0.0);

        if n == 0 || mean_cost == 0.0 {
            return ForecastResp {
                projected_cost_usd: 0.0,
                basis: format!(
                    "no recent {feature}/{provider} runs in the last 30 days to base a forecast on"
                ),
            };
        }

        let cost = (mean_cost * 1_000_000.0).round() / 1_000_000.0;
        ForecastResp {
            projected_cost_usd: cost,
            basis: format!(
                "average of {n} {feature}/{provider} sessions over last 30d \
                 (mean {:.0} tokens, mean ${:.4} each)",
                mean_tokens, mean_cost
            ),
        }
    }

    /// Token/cost totals for a single `session_id`, optionally bounded to events
    /// at or after `since`. Returns `None` when usage tracking is off or the
    /// session has no recorded events in the window yet, so callers can leave the
    /// fields null without crashing. Used to backfill per-run token columns once
    /// an agent turn finishes (see swarm runs).
    ///
    /// `since` matters because swarm sessions are REUSED across turns: without a
    /// lower bound this sums the session's LIFETIME usage, so a later run's
    /// backfill would (wrongly) be run1+run2+…. Passing the current turn's start
    /// bounds it to just this turn (one turn per agent at a time + sequential
    /// reuse means `ts >= since` cleanly isolates the turn).
    pub async fn session_totals_for(
        &self,
        session_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Option<SessionTotals> {
        // session ids are ULIDs, but escape defensively for the embedded query.
        let sid = ch_string(session_id);
        // `ts` is a DateTime64(3); a `'YYYY-MM-DD HH:MM:SS.mmm'` literal compares
        // correctly against it. The timestamp is our own (no escaping needed).
        let since_clause = since
            .map(|t| format!(" AND ts >= '{}'", t.format("%Y-%m-%d %H:%M:%S%.3f")))
            .unwrap_or_default();
        let rows: Vec<SessionTotals> = self
            .rows(&format!(
                "SELECT '{sid}' AS session_id,
                        any(workspace_id) AS workspace_id,
                        any(provider) AS provider,
                        count() AS events,
                        sum(input_tokens) AS input_tokens,
                        sum(output_tokens) AS output_tokens,
                        sum(cache_read_tokens) AS cache_read_tokens,
                        sum(cache_write_tokens) AS cache_write_tokens,
                        sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                        round(sum(cost_usd), 6) AS cost_usd
                 FROM usage_events
                 WHERE session_id = '{sid}'{since_clause}"
            ))
            .await
            .ok()?;
        // The aggregate always yields exactly one row; treat zero events as
        // "no usage yet" so the caller writes null rather than a misleading 0.
        rows.into_iter().find(|t| t.events > 0)
    }

    /// Lifetime totals for a caller-scoped set of sessions. Missing IDs stay
    /// absent (rather than manufacturing a zero), and no dashboard date/row
    /// limit changes the meaning of a work item's accumulated cost.
    pub async fn session_totals_for_ids(
        &self,
        session_ids: &[String],
    ) -> Option<Vec<SessionTotals>> {
        if session_ids.is_empty() {
            return Some(Vec::new());
        }
        if session_ids.len() > 500 {
            return None;
        }
        let ids = session_ids
            .iter()
            .map(|id| format!("'{}'", ch_string(id)))
            .collect::<Vec<_>>()
            .join(",");
        self.rows(&format!(
            "SELECT session_id, any(workspace_id) AS workspace_id, any(provider) AS provider,
                count() AS events, sum(input_tokens) AS input_tokens,
                sum(output_tokens) AS output_tokens, sum(cache_read_tokens) AS cache_read_tokens,
                sum(cache_write_tokens) AS cache_write_tokens,
                sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                round(sum(cost_usd), 6) AS cost_usd
             FROM usage_events WHERE session_id IN ({ids}) GROUP BY session_id"
        )).await.ok()
    }

    /// Full dashboard payload for the window. `otto_only` excludes externally
    /// recorded (non-Otto) sessions.
    ///
    /// The five rollups (provider, daily, session, model, day×model) run as one
    /// all-or-nothing [`ClickHouse::query_batch`] against the persistent
    /// server; any failure is returned as an error.
    pub async fn summary(&self, days: u32, otto_only: bool) -> Result<UsageSummary> {
        self.summary_scoped(days, otto_only, UsageScope::All).await
    }

    /// [`Self::summary`] narrowed to `scope`. A ClickHouse failure is an
    /// `Err` (the route turns it into an error with Retry) — only a missing
    /// engine ("not installed") yields the empty summary.
    pub async fn summary_scoped(
        &self,
        days: u32,
        otto_only: bool,
        scope: UsageScope<'_>,
    ) -> Result<UsageSummary> {
        let Some(ch) = self.ch_read() else {
            return Ok(UsageSummary {
                days,
                by_kind: Vec::new(),
                scope: scope.label().to_string(),
                ..Default::default()
            });
        };

        let ws = format!("{} {}", ws_filter(otto_only), scope.filter());
        let since = since(days);
        let limit = SESSION_LIMIT.max(1);

        let q_sessions = format!(
            "SELECT session_id,
                    any(workspace_id) AS workspace_id,
                    any(provider) AS provider,
                    any(model) AS model,
                    count() AS events,
                    sum(input_tokens) AS input_tokens,
                    sum(output_tokens) AS output_tokens,
                    sum(cache_read_tokens) AS cache_read_tokens,
                    sum(cache_write_tokens) AS cache_write_tokens,
                    sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                    round(sum(cost_usd), 6) AS cost_usd,
                    toString(max(ts)) AS last_active
             FROM usage_events
             WHERE event_date >= today() - {since} {ws}
             GROUP BY session_id
             ORDER BY total_tokens DESC, events DESC
             LIMIT {limit}"
        );

        // ONE grouped scan (day × provider × model, with event counts) feeds
        // the provider, daily, model and day×model rollups — they are pure
        // re-aggregations of it, done in Rust (`summary_rollups`). With the
        // top-sessions query that is 2 scans per summary instead of 5.
        let q_daily_models = grouped_sql(&format!("event_date >= today() - {since} {ws}"));

        // All-or-nothing: a failed read is an error, never zero totals (U1).
        // Root scope (R4): the top sessions are the head of the memoised,
        // single-flight `session_totals` rollup — the same scan the by-kind
        // breakdown and budgets reuse — so a summary is 2 scans in total.
        let mut batches = if matches!(scope, UsageScope::All) {
            let (totals, grouped) = tokio::join!(
                self.session_totals_shared(days, otto_only),
                ch.query_rows(&q_daily_models)
            );
            totals.and_then(|t| {
                let top = serde_json::to_value(top_sessions(&t, limit as usize))
                    .ok()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                Ok(vec![top, grouped?])
            })
        } else {
            ch.query_batch(&[q_sessions, q_daily_models]).await
        }
        .map_err(|e| {
            tracing::warn!("usage: summary batch failed: {e}");
            e
        })?;

        // Deserialize each result set into its typed Vec (same decoding as `rows()`).
        fn decode<T: serde::de::DeserializeOwned>(raw: Vec<serde_json::Value>) -> Vec<T> {
            raw.into_iter()
                .filter_map(|v| match serde_json::from_value(v) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        tracing::warn!("usage: decode summary row: {e}");
                        None
                    }
                })
                .collect()
        }

        let grouped: Vec<DayModelRow> = decode(batches.pop().unwrap_or_default());
        let sessions: Vec<SessionUsage> = decode(batches.pop().unwrap_or_default());
        let (providers, daily, models, daily_models) = summary_rollups(grouped);

        let total_events: u64 = providers.iter().map(|p| p.events).sum();
        let total_input_tokens: u64 = providers.iter().map(|p| p.input_tokens).sum();
        let total_output_tokens: u64 = providers.iter().map(|p| p.output_tokens).sum();
        let total_cache_read_tokens: u64 = providers.iter().map(|p| p.cache_read_tokens).sum();
        let total_cache_write_tokens: u64 = providers.iter().map(|p| p.cache_write_tokens).sum();
        let total_tokens: u64 = providers.iter().map(|p| p.total_tokens).sum();
        let total_cost_usd: f64 = providers.iter().map(|p| p.cost_usd).sum();

        Ok(UsageSummary {
            days,
            total_events,
            total_input_tokens,
            total_output_tokens,
            total_cache_read_tokens,
            total_cache_write_tokens,
            total_tokens,
            total_cost_usd,
            providers,
            daily,
            sessions,
            // Per-feature rollup needs SQLite session metadata to classify, so
            // the server fills this in (via `feature_usage`) after `summary`.
            by_kind: Vec::new(),
            models,
            daily_models,
            scope: scope.label().to_string(),
        })
    }

    /// The ccusage-style report over the window: daily, monthly, per-model,
    /// per-(day, model) and per-session tables. Errors propagate (no zeros).
    /// Sessions are raw; the server enriches titles/kinds like the summary's.
    /// Full shape (the export's): every table, up to REPORT_SESSION_LIMIT
    /// sessions — see [`Self::report_with`] for the page's slimmer read.
    pub async fn report(
        &self,
        days: u32,
        otto_only: bool,
        scope: UsageScope<'_>,
    ) -> Result<UsageReport> {
        self.report_with(days, otto_only, scope, ReportOptions::default())
            .await
    }

    /// [`Self::report`] with the session cap / `daily_models` opt-in (R3).
    /// TWO scans: the summary's single `(day, provider, model)` grouped scan
    /// (daily, monthly, models and day×model are re-aggregations of it, done
    /// in Rust) plus the capped per-session leaderboard.
    pub async fn report_with(
        &self,
        days: u32,
        otto_only: bool,
        scope: UsageScope<'_>,
        opts: ReportOptions,
    ) -> Result<UsageReport> {
        let generated_at = chrono::Utc::now().to_rfc3339();
        let mut report = UsageReport {
            days,
            generated_at,
            priced_as_of: crate::PRICED_AS_OF.to_string(),
            scope: scope.label().to_string(),
            otto_only,
            ..Default::default()
        };
        let Some(ch) = self.ch_read() else {
            return Ok(report);
        };
        let since = since(days);
        let cond = format!(
            "event_date >= today() - {since} {} {}",
            ws_filter(otto_only),
            scope.filter()
        );
        let limit = opts.sessions_limit.clamp(1, crate::REPORT_SESSION_LIMIT);
        let q_sessions = format!(
            "SELECT session_id,
                    any(workspace_id) AS workspace_id,
                    any(provider) AS provider,
                    topK(1)(model)[1] AS model,
                    count() AS events,
                    sum(input_tokens) AS input_tokens,
                    sum(output_tokens) AS output_tokens,
                    sum(cache_read_tokens) AS cache_read_tokens,
                    sum(cache_write_tokens) AS cache_write_tokens,
                    sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                    round(sum(cost_usd), 6) AS cost_usd,
                    toString(max(ts)) AS last_active
             FROM usage_events WHERE {cond}
             GROUP BY session_id
             ORDER BY total_tokens DESC, events DESC
             LIMIT {limit}"
        );
        let batches = ch.query_batch(&[grouped_sql(&cond), q_sessions]).await?;
        let mut it = batches.into_iter();
        let grouped: Vec<DayModelRow> = decode_rows(it.next().unwrap_or_default())?;
        report.sessions = decode_rows(it.next().unwrap_or_default())?;
        report.monthly = monthly_rollup(&grouped);
        let (_, daily, models, daily_models) = summary_rollups(grouped);
        report.daily = daily;
        report.models = models;
        if opts.daily_models {
            report.daily_models = daily_models;
        }
        for d in &report.daily {
            report.totals.add(&TokenTotals {
                input_tokens: d.input_tokens,
                output_tokens: d.output_tokens,
                cache_read_tokens: d.cache_read_tokens,
                cache_write_tokens: d.cache_write_tokens,
                total_tokens: d.total_tokens,
                cost_usd: d.cost_usd,
            });
        }
        report.totals.cost_usd = round6(report.totals.cost_usd);
        Ok(report)
    }

    /// `POST /usage/ccusage-check`: run ccusage over the last `days` (local
    /// dates, today inclusive) and compare it with our rows for the same days.
    /// Never fails — problems come back as `ran=false` + `error`.
    pub async fn ccusage_check(&self, days: u32) -> crate::types::CcusageCheck {
        use crate::ccusage;
        let days = days.clamp(1, 90);
        let today = chrono::Local::now().date_naive();
        let since = (today - chrono::Duration::days(i64::from(days) - 1)).to_string();
        let until = today.to_string();
        let args = ccusage::args(&since, &until);
        let mut out = crate::types::CcusageCheck {
            command: format!("npx {}", args.join(" ")),
            since: since.clone(),
            until: until.clone(),
            ..Default::default()
        };
        let ours = match self.daily_model_range(&since, &until).await {
            Ok(rows) => rows,
            Err(e) => {
                out.error = Some(format!("could not read Otto's usage: {e}"));
                return out;
            }
        };
        let Some(npx) = ccusage::find_npx() else {
            out.error = Some("npx was not found (install Node.js to run ccusage)".into());
            return out;
        };
        let started = std::time::Instant::now();
        let result = ccusage::run(&npx, &args).await;
        out.duration_ms = started.elapsed().as_millis() as u64;
        let theirs = match result {
            Ok(v) => ccusage::parse_daily(&v),
            Err(e) => {
                out.error = Some(e);
                return out;
            }
        };
        let d = ccusage::diff(&ours, &theirs);
        out.ran = true;
        out.rows = d.rows;
        out.daily = d.daily;
        out.totals_ours = d.totals_ours;
        out.totals_theirs = d.totals_theirs;
        out
    }

    /// Token totals per (day, provider, model) between two inclusive
    /// `YYYY-MM-DD` dates, every session included (external too) — the
    /// "ours" side of the ccusage cross-check.
    pub async fn daily_model_range(
        &self,
        since: &str,
        until: &str,
    ) -> Result<Vec<DailyModelUsage>> {
        let Some(ch) = self.ch_read() else {
            return Err(otto_core::Error::Upstream("usage engine offline".into()));
        };
        let cond = format!(
            "event_date BETWEEN toDate('{}') AND toDate('{}')",
            ch_string(since),
            ch_string(until)
        );
        decode_rows(ch.query_rows(&daily_models_sql(&cond)).await?)
    }

    /// System-metrics time series for the last `minutes`: the stored rows
    /// plus the not-yet-flushed tail from memory (same `ts` text shape as
    /// ClickHouse's `toString(DateTime64(3))` in local time). Reading also
    /// keeps the sampler awake (see [`Self::sampler_wanted`]).
    pub async fn metrics(&self, minutes: u32) -> Result<Vec<MetricPoint>> {
        self.last_metrics_read.store(unix_now(), Ordering::Relaxed);
        let minutes = minutes.max(1);
        let mut out: Vec<MetricPoint> = self
            .rows(&format!(
                "SELECT toString(ts) AS ts, cpu_pct, mem_used_mb, mem_total_mb, mem_pct,
                    load_avg_1, process_rss_mb, process_cpu_pct, active_sessions
             FROM system_metrics
             WHERE ts >= now() - INTERVAL {minutes} MINUTE
             ORDER BY ts"
            ))
            .await?;
        let cutoff = chrono::Utc::now() - chrono::Duration::minutes(i64::from(minutes));
        let b = self.metrics_buf.lock().expect("metrics buf lock");
        out.extend(
            b.rows
                .iter()
                .filter(|(ts, _)| *ts >= cutoff)
                .map(|(ts, m)| MetricPoint {
                    ts: ts
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M:%S%.3f")
                        .to_string(),
                    cpu_pct: m.cpu_pct,
                    mem_used_mb: m.mem_used_mb,
                    mem_total_mb: m.mem_total_mb,
                    mem_pct: m.mem_pct,
                    load_avg_1: m.load_avg_1,
                    process_rss_mb: m.process_rss_mb,
                    process_cpu_pct: m.process_cpu_pct,
                    active_sessions: m.active_sessions,
                }),
        );
        Ok(out)
    }

    /// Engine + ClickHouse status for the settings/wizard panel.
    pub async fn status(&self) -> UsageStatus {
        let cfg = self.config();
        let (ch, bin_path) = {
            let inner = self.inner.read().expect("usage inner lock");
            (inner.ch.clone(), inner.bin_path.clone())
        };
        let binary = bin_path.map(|p| p.display().to_string());
        let data_dir = self.data_dir.join("clickhouse").display().to_string();

        if let Some(ch) = ch {
            let version = ch.version().await.ok().filter(|s| !s.is_empty());
            // One round-trip for both counts (was two).
            let counts = ch
                .query_rows(
                    "SELECT (SELECT count() FROM usage_events) AS u, \
                            (SELECT count() FROM system_metrics) AS m",
                )
                .await
                .ok()
                .and_then(|r| r.into_iter().next());
            let count_of = |k: &str| {
                counts
                    .as_ref()
                    .and_then(|v| v.get(k))
                    .and_then(json_u64)
                    .unwrap_or(0)
            };
            let usage_rows = count_of("u");
            let buffered = self
                .metrics_buf
                .lock()
                .expect("metrics buf lock")
                .rows
                .len() as u64;
            let metric_rows = count_of("m") + buffered;
            let disk_bytes = self.disk_bytes(&ch).await;
            UsageStatus {
                available: true,
                enabled: cfg.enabled,
                binary,
                version,
                data_dir,
                retention_days: cfg.retention_days,
                metrics_interval_secs: cfg.metrics_interval_secs,
                usage_rows,
                metric_rows,
                disk_bytes,
                priced_as_of: crate::PRICED_AS_OF.to_string(),
            }
        } else {
            UsageStatus {
                available: false,
                enabled: cfg.enabled,
                binary,
                version: None,
                data_dir,
                retention_days: cfg.retention_days,
                metrics_interval_secs: cfg.metrics_interval_secs,
                usage_rows: 0,
                metric_rows: 0,
                disk_bytes: 0,
                priced_as_of: crate::PRICED_AS_OF.to_string(),
            }
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// On-disk size of the store, cached for [`DISK_SIZE_TTL`] (U2). Read from
    /// `system.parts` (active + outdated parts); the recursive directory walk
    /// is only the fallback and runs on a blocking thread, never on a tokio
    /// worker (the dir once held tens of thousands of parts).
    async fn disk_bytes(&self, ch: &ClickHouse) -> u64 {
        if let Some((at, bytes)) = *self.disk_cache.lock().expect("disk cache lock") {
            if at.elapsed() < DISK_SIZE_TTL {
                return bytes;
            }
        }
        let from_parts = ch
            .query_rows("SELECT sum(bytes_on_disk) AS n FROM system.parts")
            .await
            .ok()
            .and_then(|r| r.into_iter().next())
            .and_then(|v| v.get("n").and_then(json_u64))
            .filter(|n| *n > 0);
        let bytes = match from_parts {
            Some(n) => n,
            None => {
                let dir = ch.data_dir().to_path_buf();
                tokio::task::spawn_blocking(move || dir_size(&dir))
                    .await
                    .unwrap_or(0)
            }
        };
        *self.disk_cache.lock().expect("disk cache lock") =
            Some((std::time::Instant::now(), bytes));
        bytes
    }

    /// Run a query and deserialize each row into `T`.
    async fn rows<T: serde::de::DeserializeOwned>(&self, sql: &str) -> Result<Vec<T>> {
        let Some(ch) = self.ch_read() else {
            return Ok(Vec::new());
        };
        let raw = ch.query_rows(sql).await?;
        let mut out = Vec::with_capacity(raw.len());
        for v in raw {
            out.push(
                serde_json::from_value(v)
                    .map_err(|e| otto_core::Error::Internal(format!("decode usage row: {e}")))?,
            );
        }
        Ok(out)
    }
}

/// Background task: drains the event channel, batching inserts on a timer or
/// when the buffer fills. Exits when the channel closes (engine reinit/drop).
/// `heal` is raised when a flush fails against a dead server child, so the
/// engine's watcher restarts it. Failed flushes RETAIN their events (bounded)
/// for the next tick; what's still unflushed when the reinit ends this writer
/// is lost — a bounded, logged loss instead of the old drop-every-batch.
/// Repartition any usage table whose live definition has no `PARTITION BY`
/// (schema drift: `CREATE TABLE IF NOT EXISTS` never upgrades a table). Copy
/// → verify row counts → atomic `EXCHANGE` → drop the old data. Idempotent
/// (detected from `system.tables`, so it runs once) and non-fatal: on any
/// error the original table stays as it was and the next start retries.
async fn ensure_partitioned(ch: &ClickHouse, retention_days: u32) {
    for (table, date_col, order_by) in schema::PARTITIONED_TABLES {
        let key = match ch.query_rows(&schema::partition_key_sql(table)).await {
            Ok(rows) => rows
                .first()
                .and_then(|r| r.get("partition_key"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            Err(e) => {
                tracing::warn!("usage: partition check of {table} failed (non-fatal): {e}");
                continue;
            }
        };
        if key.as_deref().is_none_or(|k| !k.is_empty()) {
            continue; // partitioned already (or no such table)
        }
        tracing::info!("usage: repartitioning {table} by month (one-time)");
        let copy = schema::repartition_sql(table, date_col, order_by, retention_days);
        if let Err(e) = ch.exec(&copy).await {
            tracing::warn!("usage: repartition copy of {table} failed (non-fatal): {e}");
            let _ = ch
                .exec(&format!("DROP TABLE IF EXISTS {table}_repart SYNC"))
                .await;
            continue;
        }
        let count = |t: String| async move {
            ch.query_rows(&format!("SELECT count() AS n FROM {t}"))
                .await
                .ok()
                .and_then(|r| r.first().and_then(|v| v.get("n")).map(|n| n.to_string()))
        };
        let (old_n, new_n) = (
            count(table.to_string()).await,
            count(format!("{table}_repart")).await,
        );
        if old_n.is_none() || old_n != new_n {
            tracing::warn!(
                "usage: repartition of {table} aborted — row counts differ ({old_n:?} vs {new_n:?})"
            );
            let _ = ch
                .exec(&format!("DROP TABLE IF EXISTS {table}_repart SYNC"))
                .await;
            continue;
        }
        match ch.exec(&schema::swap_sql(table)).await {
            Ok(()) => tracing::info!(
                "usage: {table} repartitioned ({} rows)",
                old_n.unwrap_or_default()
            ),
            Err(e) => tracing::warn!("usage: repartition swap of {table} failed (non-fatal): {e}"),
        }
    }
}

/// Idle-stop loop (R1d): every IDLE_CHECK_EVERY, park the server if it served
/// nothing for the configured window. Weak, so a replaced/dropped handle ends
/// the task; the handle restarts itself on the next request.
fn spawn_idle_stopper(
    ch: Weak<ClickHouse>,
    idle_secs: Arc<AtomicU64>,
    changed: Arc<tokio::sync::Notify>,
) {
    tokio::spawn(async move {
        loop {
            let secs = idle_secs.load(Ordering::Relaxed);
            let every = if secs == 0 {
                IDLE_CHECK_EVERY
            } else {
                IDLE_CHECK_EVERY.min(Duration::from_secs(secs))
            };
            tokio::select! {
                _ = tokio::time::sleep(every) => {}
                // A new window applies now, not after the old sleep.
                _ = changed.notified() => continue,
            }
            let Some(ch) = ch.upgrade() else { break };
            let secs = idle_secs.load(Ordering::Relaxed);
            if secs > 0 {
                ch.maybe_park(Duration::from_secs(secs)).await;
            }
        }
    });
}

fn spawn_writer(
    ch: Arc<ClickHouse>,
    mut rx: mpsc::UnboundedReceiver<UsageEvent>,
    heal: Arc<HealSignal>,
    usage_gen: Arc<AtomicU64>,
    flush_req: Arc<tokio::sync::Notify>,
) {
    tokio::spawn(async move {
        let mut w = WriterBuf::default();
        // The flush timer is armed only while events are buffered (from the
        // first one): an idle daemon has no periodic wake-up at all.
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            let tick = async {
                match deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(ev) => {
                        w.push(ev);
                        if w.buf.len() >= FLUSH_BATCH {
                            flush(&ch, &mut w, &heal, &usage_gen, false).await;
                        }
                    }
                    None => {
                        // Final flush (reinit / shutdown): may wake a parked
                        // server so nothing buffered is lost.
                        flush(&ch, &mut w, &heal, &usage_gen, true).await;
                        break;
                    }
                },
                // A dashboard read (or a lazy restart) wants fresh rows now.
                _ = flush_req.notified() => flush(&ch, &mut w, &heal, &usage_gen, false).await,
                _ = tick => flush(&ch, &mut w, &heal, &usage_gen, false).await,
            }
            deadline = match (w.buf.is_empty(), deadline) {
                (true, _) => None,
                // A failed / deferred flush keeps its events: retry a full
                // interval later.
                (false, Some(d)) if d > tokio::time::Instant::now() => Some(d),
                (false, _) => Some(tokio::time::Instant::now() + FLUSH_INTERVAL),
            };
        }
    });
}

/// The writer's buffered events plus when the oldest arrived (the MAX_DEFER
/// clock while the server is parked).
#[derive(Default)]
struct WriterBuf {
    buf: Vec<UsageEvent>,
    since: Option<std::time::Instant>,
}

impl WriterBuf {
    fn push(&mut self, ev: UsageEvent) {
        self.since.get_or_insert_with(std::time::Instant::now);
        self.buf.push(ev);
    }

    /// Whether a parked server must be woken for these events: the oldest has
    /// waited MAX_DEFER, or the retained buffer is full.
    fn overdue(&self, max_defer: Duration) -> bool {
        self.buf.len() >= RETAIN_MAX || self.since.is_some_and(|t| t.elapsed() >= max_defer)
    }
}

/// What one writer flush should do with a parked server.
#[derive(Debug, PartialEq, Eq)]
enum ParkedAction {
    /// Keep the events buffered for the next foreground wake.
    Defer,
    /// Wake the server (background: idle clock untouched) and insert.
    Wake,
}

fn parked_action(w: &WriterBuf, final_flush: bool, max_defer: Duration) -> ParkedAction {
    if final_flush || w.overdue(max_defer) {
        ParkedAction::Wake
    } else {
        ParkedAction::Defer
    }
}

/// Write the buffered events as ONE background insert (S9-02). A parked
/// server is left parked unless [`parked_action`] says the events have waited
/// long enough (or this is the final flush). Failures keep the events.
async fn flush(
    ch: &Arc<ClickHouse>,
    w: &mut WriterBuf,
    heal: &HealSignal,
    usage_gen: &AtomicU64,
    final_flush: bool,
) {
    if w.buf.is_empty() {
        return;
    }
    let payload = ndjson(&w.buf);
    let res = match ch.insert_ndjson_background("usage_events", &payload).await {
        Ok(true) => Ok(()),
        Ok(false) => match parked_action(w, final_flush, MAX_DEFER) {
            ParkedAction::Defer => return,
            ParkedAction::Wake => match ch.wake_background().await {
                // The lease holds parking off for the insert; dropping it
                // does not reset the idle clock.
                Ok(_lease) => ch
                    .insert_ndjson_background("usage_events", &payload)
                    .await
                    .and_then(|sent| {
                        if sent {
                            Ok(())
                        } else {
                            Err(otto_core::Error::Upstream(
                                "usage: clickhouse parked again mid-flush".into(),
                            ))
                        }
                    }),
                Err(e) => Err(e),
            },
        },
        Err(e) => Err(e),
    };
    if let Err(e) = res {
        tracing::warn!("usage: flush failed: {e}");
        // Keep the events for the next attempt (bounded) instead of dropping
        // them — a transient failure or a self-heal restart then loses nothing.
        if w.buf.len() > RETAIN_MAX {
            let drop_n = w.buf.len() - RETAIN_MAX;
            w.buf.drain(..drop_n);
            tracing::warn!("usage: retained buffer full — dropped {drop_n} oldest events");
        }
        if !ch.server_alive() {
            heal.store(true, Ordering::SeqCst);
        }
        return;
    }
    w.buf.clear();
    w.since = None;
    usage_gen.fetch_add(1, Ordering::SeqCst);
}

/// Serialize events to newline-delimited JSON for `JSONEachRow` insertion.
fn ndjson(events: &[UsageEvent]) -> String {
    let mut s = String::new();
    for ev in events {
        if let Ok(line) = serde_json::to_string(ev) {
            s.push_str(&line);
            s.push('\n');
        }
    }
    s
}

/// ClickHouse's JSON output quotes 64-bit integers by default
/// (`output_format_json_quote_64bit_integers`); accept both shapes.
fn json_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// Decode a row set; a malformed row is an error (same as [`UsageEngine::rows`]).
fn decode_rows<T: serde::de::DeserializeOwned>(raw: Vec<serde_json::Value>) -> Result<Vec<T>> {
    raw.into_iter()
        .map(|v| {
            serde_json::from_value(v)
                .map_err(|e| otto_core::Error::Internal(format!("decode usage row: {e}")))
        })
        .collect()
}

/// The single `(day, provider, model)` grouped scan behind the summary AND the
/// report (unrounded cost: the Rust rollups round once, like the old SQL).
fn grouped_sql(cond: &str) -> String {
    format!(
        "SELECT toString(event_date) AS day, provider, model,
                count() AS events,
                sum(input_tokens) AS input_tokens,
                sum(output_tokens) AS output_tokens,
                sum(cache_read_tokens) AS cache_read_tokens,
                sum(cache_write_tokens) AS cache_write_tokens,
                sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                sum(cost_usd) AS cost_usd
         FROM usage_events
         WHERE {cond}
         GROUP BY event_date, provider, model
         ORDER BY event_date, total_tokens DESC"
    )
}

/// What a report includes beyond the always-present tables (R3).
#[derive(Debug, Clone, Copy)]
pub struct ReportOptions {
    /// Session leaderboard cap (clamped to REPORT_SESSION_LIMIT).
    pub sessions_limit: u32,
    /// Ship the per-(day, model) table — only the export renders it.
    pub daily_models: bool,
}

impl Default for ReportOptions {
    /// The full export shape (back-compatible with the old report).
    fn default() -> Self {
        Self {
            sessions_limit: crate::REPORT_SESSION_LIMIT,
            daily_models: true,
        }
    }
}

/// Per-month rollup of the grouped scan, ordered by month (`YYYY-MM`).
fn monthly_rollup(rows: &[DayModelRow]) -> Vec<MonthlyUsage> {
    let mut months: std::collections::BTreeMap<&str, MonthlyUsage> = Default::default();
    for r in rows {
        let key = r.day.get(..7).unwrap_or(&r.day);
        let m = months.entry(key).or_insert_with(|| MonthlyUsage {
            month: key.to_string(),
            ..Default::default()
        });
        m.events += r.events;
        m.input_tokens += r.input_tokens;
        m.output_tokens += r.output_tokens;
        m.cache_read_tokens += r.cache_read_tokens;
        m.cache_write_tokens += r.cache_write_tokens;
        m.total_tokens += r.total_tokens;
        m.cost_usd += r.cost_usd;
    }
    let mut out: Vec<MonthlyUsage> = months.into_values().collect();
    for m in &mut out {
        m.cost_usd = round6(m.cost_usd);
    }
    out
}

/// The summary's top-`limit` session rows from the full per-session rollup,
/// ordered like the old `ORDER BY total_tokens DESC, events DESC LIMIT n`.
fn top_sessions(totals: &[SessionTotals], limit: usize) -> Vec<SessionUsage> {
    let mut idx: Vec<&SessionTotals> = totals.iter().collect();
    idx.sort_by(|a, b| {
        b.total_tokens
            .cmp(&a.total_tokens)
            .then(b.events.cmp(&a.events))
    });
    idx.into_iter()
        .take(limit)
        .map(|t| SessionUsage {
            session_id: t.session_id.clone(),
            workspace_id: t.workspace_id.clone(),
            provider: t.provider.clone(),
            model: t.model.clone(),
            events: t.events,
            input_tokens: t.input_tokens,
            output_tokens: t.output_tokens,
            cache_read_tokens: t.cache_read_tokens,
            cache_write_tokens: t.cache_write_tokens,
            total_tokens: t.total_tokens,
            cost_usd: t.cost_usd,
            last_active: t.last_active.clone(),
            ..Default::default()
        })
        .collect()
}

/// One `(day, provider, model)` group of the summary's single scan.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct DayModelRow {
    day: String,
    provider: String,
    model: String,
    events: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    total_tokens: u64,
    cost_usd: f64,
}

/// Round a summed cost like the SQL `round(sum(cost_usd), 6)` did.
fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// Derive the summary's provider / daily / model / day×model rollups from the
/// `(day, provider, model)` groups, with the orderings the old per-rollup
/// queries had (providers & models: total tokens desc, events desc; daily by
/// day; day×model as scanned).
fn summary_rollups(
    rows: Vec<DayModelRow>,
) -> (
    Vec<ProviderUsage>,
    Vec<DailyUsage>,
    Vec<ModelUsage>,
    Vec<DailyModelUsage>,
) {
    use std::collections::BTreeMap;
    let mut providers: BTreeMap<String, ProviderUsage> = BTreeMap::new();
    let mut daily: BTreeMap<String, DailyUsage> = BTreeMap::new();
    let mut models: BTreeMap<(String, String), ModelUsage> = BTreeMap::new();
    let mut daily_models = Vec::with_capacity(rows.len());
    for r in rows {
        let p = providers
            .entry(r.provider.clone())
            .or_insert_with(|| ProviderUsage {
                provider: r.provider.clone(),
                ..Default::default()
            });
        p.events += r.events;
        p.input_tokens += r.input_tokens;
        p.output_tokens += r.output_tokens;
        p.cache_read_tokens += r.cache_read_tokens;
        p.cache_write_tokens += r.cache_write_tokens;
        p.total_tokens += r.total_tokens;
        p.cost_usd += r.cost_usd;
        let d = daily.entry(r.day.clone()).or_insert_with(|| DailyUsage {
            day: r.day.clone(),
            ..Default::default()
        });
        d.events += r.events;
        d.input_tokens += r.input_tokens;
        d.output_tokens += r.output_tokens;
        d.cache_read_tokens += r.cache_read_tokens;
        d.cache_write_tokens += r.cache_write_tokens;
        d.total_tokens += r.total_tokens;
        d.cost_usd += r.cost_usd;
        let m = models
            .entry((r.provider.clone(), r.model.clone()))
            .or_insert_with(|| ModelUsage {
                provider: r.provider.clone(),
                model: r.model.clone(),
                ..Default::default()
            });
        m.events += r.events;
        m.input_tokens += r.input_tokens;
        m.output_tokens += r.output_tokens;
        m.cache_read_tokens += r.cache_read_tokens;
        m.cache_write_tokens += r.cache_write_tokens;
        m.total_tokens += r.total_tokens;
        m.cost_usd += r.cost_usd;
        daily_models.push(DailyModelUsage {
            day: r.day,
            provider: r.provider,
            model: r.model,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cache_read_tokens: r.cache_read_tokens,
            cache_write_tokens: r.cache_write_tokens,
            total_tokens: r.total_tokens,
            cost_usd: round6(r.cost_usd),
        });
    }
    let by_total = |a: (u64, u64), b: (u64, u64)| b.0.cmp(&a.0).then(b.1.cmp(&a.1));
    let mut providers: Vec<ProviderUsage> = providers.into_values().collect();
    for p in &mut providers {
        p.cost_usd = round6(p.cost_usd);
    }
    providers.sort_by(|a, b| by_total((a.total_tokens, a.events), (b.total_tokens, b.events)));
    let mut daily: Vec<DailyUsage> = daily.into_values().collect();
    for d in &mut daily {
        d.cost_usd = round6(d.cost_usd);
    }
    let mut models: Vec<ModelUsage> = models.into_values().collect();
    for m in &mut models {
        m.cost_usd = round6(m.cost_usd);
    }
    models.sort_by(|a, b| by_total((a.total_tokens, a.events), (b.total_tokens, b.events)));
    (providers, daily, models, daily_models)
}

/// Per-(day, provider, model) token rollup under `cond` (a WHERE body).
fn daily_models_sql(cond: &str) -> String {
    format!(
        "SELECT toString(event_date) AS day, provider, model,
                sum(input_tokens) AS input_tokens,
                sum(output_tokens) AS output_tokens,
                sum(cache_read_tokens) AS cache_read_tokens,
                sum(cache_write_tokens) AS cache_write_tokens,
                sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS total_tokens,
                round(sum(cost_usd), 6) AS cost_usd
         FROM usage_events
         WHERE {cond}
         GROUP BY event_date, provider, model
         ORDER BY event_date, total_tokens DESC"
    )
}

/// Apply the retention window to both tables, but only where it differs from
/// the TTL the table already has (P5): an unchanged window issues no ALTER.
async fn ensure_ttl(ch: &ClickHouse, retention_days: u32) -> Result<()> {
    let want = retention_days.max(1);
    let current = ch
        .query_rows(schema::TABLE_TTLS_SQL)
        .await
        .unwrap_or_default();
    let all_match = current.len() == 2
        && current.iter().all(|row| {
            row.get("engine_full")
                .and_then(serde_json::Value::as_str)
                .and_then(schema::parse_ttl_days)
                == Some(want)
        });
    if all_match {
        return Ok(());
    }
    ch.exec(&schema::alter_ttl_sql(want)).await
}

/// Convert "last N days" into the ClickHouse `today() - X` offset (inclusive of
/// today): N days → offset N-1.
fn since(days: u32) -> u32 {
    days.max(1) - 1
}

/// SQL fragment that, when `otto_only`, excludes externally-recorded (non-Otto)
/// sessions from a usage aggregation. Empty otherwise (count everything).
fn ws_filter(otto_only: bool) -> String {
    if otto_only {
        format!("AND workspace_id != '{}'", crate::EXTERNAL_WORKSPACE)
    } else {
        String::new()
    }
}

/// Recursive on-disk size of `dir` in bytes (best-effort).
fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            total += dir_size(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

/// One-time compaction + retention enforcement of a pre-existing dataset. The old
/// per-query model left `usage_events` with tens of thousands of never-merged tiny
/// parts (1.4 GB of metadata for ~12 MB of data). With a persistent server,
/// `OPTIMIZE … FINAL` collapses them and the row-TTL drops expired rows during the
/// merge — reclaiming the disk and removing irrelevant (out-of-window) data.
///
/// Marker-guarded (`<dir>/.usage_compacted_v2`) so it runs at most once; spawned
/// (never blocks); bounded by a 30-minute timeout (ordinary background merges
/// converge regardless if it can't finish). The marker is written only on success
/// so a failure retries on the next boot.
async fn maybe_compact(ch: Arc<ClickHouse>, ch_dir: PathBuf, retention_days: u32) {
    let marker = ch_dir.join(".usage_compacted_v2");
    if marker.exists() {
        return;
    }
    let before_rows = scalar_count(&ch).await;
    let before_mb = dir_size(&ch_dir) / 1_048_576;
    tracing::info!("usage: one-time compaction starting (rows={before_rows}, dir={before_mb} MB)");

    // Ensure the retention window is applied so expired rows get dropped in the merge.
    let _ = ch.exec(&schema::alter_ttl_sql(retention_days)).await;

    let opt = tokio::time::timeout(
        Duration::from_secs(30 * 60),
        ch.exec("OPTIMIZE TABLE usage_events FINAL"),
    )
    .await;
    match opt {
        Ok(Ok(())) => {
            // Also compact the metrics table (cheap).
            let _ = ch.exec("OPTIMIZE TABLE system_metrics FINAL").await;
            let after_rows = scalar_count(&ch).await;
            let after_mb = dir_size(&ch_dir) / 1_048_576;
            tracing::info!(
                "usage: compaction done (rows {before_rows}->{after_rows}, {before_mb} MB -> {after_mb} MB)"
            );
            if let Err(e) = std::fs::write(&marker, "ok") {
                tracing::warn!("usage: could not write compaction marker: {e}");
            }
        }
        Ok(Err(e)) => tracing::warn!(
            "usage: compaction OPTIMIZE failed (non-fatal; background merges continue): {e}"
        ),
        Err(_) => tracing::warn!(
            "usage: compaction OPTIMIZE timed out after 30m (background merges continue)"
        ),
    }
}

/// `SELECT count() FROM usage_events`, 0 on any error.
async fn scalar_count(ch: &ClickHouse) -> u64 {
    ch.query_rows("SELECT count() AS n FROM usage_events")
        .await
        .ok()
        .and_then(|r| r.into_iter().next())
        .and_then(|v| v.get("n").and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

/// Escape `s` for a single-quoted ClickHouse string literal. ClickHouse treats
/// a backslash as an escape character inside literals, so doubling only `'`
/// let a trailing `\` swallow the closing quote and break out of the string.
fn ch_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod ch_string_tests {
    use super::ch_string;

    #[test]
    fn escapes_backslash_and_quote() {
        assert_eq!(ch_string("review"), "review");
        assert_eq!(ch_string("o'brien"), "o\\'brien");
        // A trailing backslash can no longer escape the closing quote.
        assert_eq!(ch_string("x\\"), "x\\\\");
        assert_eq!(ch_string("a\\' OR 1=1 --"), "a\\\\\\' OR 1=1 --");
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn scope_filter_narrows_to_listed_sessions() {
        assert_eq!(UsageScope::All.filter(), "");
        assert_eq!(
            UsageScope::Sessions(&[]).filter(),
            "AND 0",
            "no sessions → no rows"
        );
        let ids = vec!["s1".to_string(), "o'x".to_string()];
        assert_eq!(
            UsageScope::Sessions(&ids).filter(),
            "AND session_id IN ('s1','o\\'x')"
        );
        assert_eq!(UsageScope::All.label(), "all");
        assert_eq!(UsageScope::Sessions(&ids).label(), "own");
    }

    #[test]
    fn model_rollups_group_by_model() {
        let sql = grouped_sql("event_date >= today() - 6");
        assert!(sql.contains("GROUP BY event_date, provider, model"));
        assert!(sql.contains("count() AS events"));
        let sql = daily_models_sql("1");
        assert!(sql.contains("GROUP BY event_date, provider, model"));
    }

    fn dm(day: &str, model: &str, tokens: u64, cost: f64) -> DayModelRow {
        DayModelRow {
            day: day.into(),
            provider: "claude".into(),
            model: model.into(),
            events: 1,
            input_tokens: tokens,
            total_tokens: tokens,
            cost_usd: cost,
            ..Default::default()
        }
    }

    /// R3: the report's monthly table is a re-aggregation of the grouped
    /// scan (what the old `GROUP BY month` query returned), months ordered.
    #[test]
    fn monthly_rollup_groups_days_by_month() {
        let rows = vec![
            dm("2026-09-30", "a", 5, 0.1000004),
            dm("2026-10-01", "a", 7, 0.2),
            dm("2026-10-01", "b", 3, 0.3),
            dm("2026-09-02", "b", 1, 0.0000004),
        ];
        let m = monthly_rollup(&rows);
        assert_eq!(m.len(), 2);
        assert_eq!(
            (m[0].month.as_str(), m[0].total_tokens, m[0].events),
            ("2026-09", 6, 2)
        );
        assert_eq!(
            (m[1].month.as_str(), m[1].total_tokens, m[1].events),
            ("2026-10", 10, 2)
        );
        assert_eq!(m[0].cost_usd, 0.100001, "rounded once, after summing");
        assert_eq!(m[1].cost_usd, 0.5);
    }

    /// R4: the summary's top sessions come from the per-session totals,
    /// ordered like `ORDER BY total_tokens DESC, events DESC LIMIT n`.
    #[test]
    fn top_sessions_orders_and_caps_the_totals() {
        let t = |id: &str, tokens: u64, events: u64| SessionTotals {
            session_id: id.into(),
            total_tokens: tokens,
            events,
            model: format!("m-{id}"),
            last_active: "2026-10-03 10:00:00.000".into(),
            ..Default::default()
        };
        let rows = vec![t("a", 10, 1), t("b", 30, 1), t("c", 10, 5), t("d", 1, 1)];
        let top = top_sessions(&rows, 3);
        let ids: Vec<&str> = top.iter().map(|s| s.session_id.as_str()).collect();
        assert_eq!(ids, ["b", "c", "a"]);
        assert_eq!(top[0].model, "m-b");
        assert_eq!(top[0].last_active, "2026-10-03 10:00:00.000");
    }

    #[test]
    fn json_u64_accepts_quoted_integers() {
        assert_eq!(json_u64(&serde_json::json!(7)), Some(7));
        assert_eq!(json_u64(&serde_json::json!("12")), Some(12));
        assert_eq!(json_u64(&serde_json::json!(null)), None);
    }

    #[tokio::test]
    async fn summary_without_engine_is_empty_not_error() {
        // No ClickHouse ("not installed") is the one case that may return an
        // empty summary; a failing ClickHouse propagates (see summary_scoped).
        let dir = tempfile::tempdir().unwrap();
        let cfg = UsageConfig {
            enabled: false,
            ..Default::default()
        };
        let engine = UsageEngine::start(cfg, dir.path().to_path_buf()).await;
        let s = engine.summary(7, true).await.unwrap();
        assert_eq!(s.total_tokens, 0);
        assert_eq!(s.scope, "all");
        let ids = vec!["x".to_string()];
        let r = engine
            .report(7, true, UsageScope::Sessions(&ids))
            .await
            .unwrap();
        assert_eq!(r.scope, "own");
        assert!(engine
            .daily_model_range("2026-01-01", "2026-01-02")
            .await
            .is_err());
    }

    /// perf2/03 N5: the heal watcher has no poll — a raise is acted on at
    /// once (it used to wait for the next 5 s tick), and a raise made while
    /// nobody waits is not lost.
    #[tokio::test]
    async fn heal_watcher_reacts_to_a_raise_without_polling() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = UsageConfig {
            enabled: false,
            ..Default::default()
        };
        let engine = UsageEngine::start(cfg, dir.path().to_path_buf()).await;
        for _ in 0..2 {
            engine.raise_heal();
            let t = std::time::Instant::now();
            while engine.heal.flag.load(Ordering::SeqCst) {
                assert!(
                    t.elapsed() < Duration::from_secs(1),
                    "watcher did not drain the raise"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(t.elapsed() < Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod rollup_tests {
    use super::*;

    #[test]
    fn summary_rollups_reaggregate_the_single_scan() {
        let row = |day: &str, p: &str, m: &str, ev: u64, tot: u64, cost: f64| DayModelRow {
            day: day.into(),
            provider: p.into(),
            model: m.into(),
            events: ev,
            input_tokens: tot,
            total_tokens: tot,
            cost_usd: cost,
            ..Default::default()
        };
        let (providers, daily, models, dm) = summary_rollups(vec![
            row("2026-10-01", "claude", "opus", 2, 100, 0.1),
            row("2026-10-01", "codex", "gpt", 1, 500, 0.0000004),
            row("2026-10-02", "claude", "opus", 3, 50, 0.2),
            row("2026-10-02", "claude", "sonnet", 1, 50, 0.05),
        ]);
        let p: Vec<_> = providers
            .iter()
            .map(|p| (p.provider.as_str(), p.events, p.total_tokens))
            .collect();
        assert_eq!(p, vec![("codex", 1, 500), ("claude", 6, 200)]);
        assert!((providers[1].cost_usd - 0.35).abs() < 1e-9);
        assert_eq!(providers[0].cost_usd, 0.0, "rounded to 6 places like SQL");
        let d: Vec<_> = daily
            .iter()
            .map(|d| (d.day.as_str(), d.events, d.total_tokens))
            .collect();
        assert_eq!(d, vec![("2026-10-01", 3, 600), ("2026-10-02", 4, 100)]);
        let m: Vec<_> = models
            .iter()
            .map(|m| (m.model.as_str(), m.events, m.total_tokens))
            .collect();
        assert_eq!(
            m,
            vec![("gpt", 1, 500), ("opus", 5, 150), ("sonnet", 1, 50)]
        );
        assert_eq!(dm.len(), 4);
    }
}

#[cfg(test)]
mod writer_policy_tests {
    use super::*;

    /// S9-02: a parked server is NOT woken for fresh usage events (they wait
    /// for the next foreground wake) — only once the oldest has waited
    /// MAX_DEFER, the retained buffer is full, or this is the final flush.
    #[test]
    fn parked_server_defers_until_overdue_or_final() {
        let mut w = WriterBuf::default();
        w.push(UsageEvent::default());
        assert_eq!(
            parked_action(&w, false, MAX_DEFER),
            ParkedAction::Defer,
            "fresh events must not wake an idle-stopped server"
        );
        assert_eq!(parked_action(&w, true, MAX_DEFER), ParkedAction::Wake);
        // Oldest buffered event older than the defer window → wake.
        w.since = Some(std::time::Instant::now() - Duration::from_millis(20));
        assert_eq!(
            parked_action(&w, false, Duration::from_millis(10)),
            ParkedAction::Wake
        );
        // A full retained buffer wakes regardless of age.
        let mut full = WriterBuf::default();
        for _ in 0..RETAIN_MAX {
            full.push(UsageEvent::default());
        }
        assert_eq!(parked_action(&full, false, MAX_DEFER), ParkedAction::Wake);
    }

    /// S9-01: the writer flushes at most every 90 s (or per 2k rows), not every
    /// 15 s / 200 rows — each flush is a part that gets merged into the month.
    #[test]
    fn flush_cadence_is_coarse() {
        assert!(FLUSH_INTERVAL >= Duration::from_secs(60));
        assert!(FLUSH_BATCH >= 1_000);
        assert!(MAX_DEFER <= Duration::from_secs(30 * 60));
    }
}
