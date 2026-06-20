//! [`UsageEngine`] — the façade the daemon talks to. Owns the ClickHouse
//! handle, a background batch-writer for usage events, the live config, and all
//! the aggregate queries the dashboard reads.
//!
//! The ClickHouse handle + writer live behind an `RwLock` ([`Inner`]) so the
//! engine can be (re)initialized at runtime — e.g. right after the wizard
//! installs or updates the `clickhouse` binary — without a daemon restart.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use otto_core::Result;
use tokio::sync::mpsc;

use crate::clickhouse::ClickHouse;
use crate::metrics::{Metric, MetricsSampler};
use crate::schema;
use crate::types::{
    DailyUsage, FeatureUsage, MetricPoint, ProviderUsage, SessionTotals, SessionUsage, UsageConfig,
    UsageEvent, UsageStatus, UsageSummary,
};

/// Flush the usage buffer at least this often.
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);
/// …or sooner once this many events are buffered.
const FLUSH_BATCH: usize = 200;
/// Default cap on the session leaderboard.
const SESSION_LIMIT: u32 = 50;

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
}

impl UsageEngine {
    /// Build the engine: locate the binary, create the schema with the
    /// configured retention, and spawn the batch writer. Never fails — on any
    /// problem it returns a degraded (no-op) engine that still reports status
    /// and can be revived later via [`Self::reinit`].
    pub async fn start(config: UsageConfig, data_dir: PathBuf) -> Arc<Self> {
        let engine = Arc::new(Self {
            inner: RwLock::new(Inner::default()),
            config: RwLock::new(config.clone()),
            data_dir,
        });
        engine.reinit(config).await;
        engine
    }

    /// (Re)initialize the ClickHouse handle from `config`. Tears down any
    /// previous writer (dropping the old channel ends its task) and swaps in a
    /// fresh handle. Safe to call repeatedly.
    pub async fn reinit(&self, config: UsageConfig) {
        let ch_dir = self.data_dir.join("clickhouse");
        let bin_path = ClickHouse::locate(config.clickhouse_path.as_deref());

        let (ch, tx) = if config.enabled {
            match &bin_path {
                Some(bin) => {
                    let ch = Arc::new(ClickHouse::new(bin.clone(), ch_dir));
                    match ch.exec(&schema::schema_sql(config.retention_days)).await {
                        Ok(()) => {
                            let (tx, rx) = mpsc::unbounded_channel();
                            spawn_writer(Arc::clone(&ch), rx);
                            tracing::info!(
                                "usage: clickhouse ready at {} (binary {})",
                                ch.data_dir().display(),
                                bin.display()
                            );
                            (Some(ch), Some(tx))
                        }
                        Err(e) => {
                            tracing::warn!("usage: clickhouse schema init failed: {e}");
                            (None, None)
                        }
                    }
                }
                None => {
                    tracing::info!("usage: clickhouse binary not found — usage tracking disabled");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        *self.config.write().expect("usage config lock") = config;
        *self.inner.write().expect("usage inner lock") = Inner { ch, tx, bin_path };
    }

    /// True when usage tracking is live (binary found + schema created).
    pub fn available(&self) -> bool {
        self.inner.read().expect("usage inner lock").ch.is_some()
    }

    fn ch(&self) -> Option<Arc<ClickHouse>> {
        self.inner.read().expect("usage inner lock").ch.clone()
    }

    fn config(&self) -> UsageConfig {
        self.config.read().expect("usage config lock").clone()
    }

    // ── Recording ──────────────────────────────────────────────────────────

    /// Queue one event for buffered insertion (fire-and-forget; dropped if the
    /// engine is disabled).
    pub fn record(&self, ev: UsageEvent) {
        let tx = self.inner.read().expect("usage inner lock").tx.clone();
        if let Some(tx) = tx {
            let _ = tx.send(ev);
        }
    }

    /// Insert events synchronously (used by the writer and by tests). No-op
    /// when disabled.
    pub async fn insert_events(&self, events: &[UsageEvent]) -> Result<()> {
        let Some(ch) = self.ch() else { return Ok(()) };
        ch.insert_ndjson("usage_events", &ndjson(events)).await
    }

    /// Persist one metrics sample. No-op when disabled.
    pub async fn store_metric(&self, m: &Metric) -> Result<()> {
        let Some(ch) = self.ch() else { return Ok(()) };
        let row = serde_json::json!({
            "host": MetricsSampler::host(),
            "cpu_pct": m.cpu_pct,
            "mem_used_mb": m.mem_used_mb,
            "mem_total_mb": m.mem_total_mb,
            "mem_pct": m.mem_pct,
            "load_avg_1": m.load_avg_1,
            "process_rss_mb": m.process_rss_mb,
            "process_cpu_pct": m.process_cpu_pct,
            "active_sessions": m.active_sessions,
        });
        ch.insert_ndjson("system_metrics", &format!("{row}\n")).await
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
            ch.exec(&schema::alter_ttl_sql(retention_days)).await?;
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
    pub async fn session_usage(&self, days: u32, limit: u32, otto_only: bool) -> Result<Vec<SessionUsage>> {
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
                    round(sum(cost_usd), 6) AS cost_usd
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
        let totals = self.session_totals(days, otto_only).await?;
        let mut buckets: std::collections::HashMap<String, FeatureUsage> =
            std::collections::HashMap::new();
        for t in &totals {
            let feature = classify(t);
            let b = buckets.entry(feature.clone()).or_insert_with(|| FeatureUsage {
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
        let sid = session_id.replace('\'', "''");
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

    /// Full dashboard payload for the window. `otto_only` excludes externally
    /// recorded (non-Otto) sessions.
    pub async fn summary(&self, days: u32, otto_only: bool) -> Result<UsageSummary> {
        let providers = self.provider_usage(days, otto_only).await.unwrap_or_default();
        let daily = self.daily_usage(days, otto_only).await.unwrap_or_default();
        let sessions = self
            .session_usage(days, SESSION_LIMIT, otto_only)
            .await
            .unwrap_or_default();

        let total_events = providers.iter().map(|p| p.events).sum();
        let total_input_tokens = providers.iter().map(|p| p.input_tokens).sum();
        let total_output_tokens = providers.iter().map(|p| p.output_tokens).sum();
        let total_cache_read_tokens = providers.iter().map(|p| p.cache_read_tokens).sum();
        let total_cache_write_tokens = providers.iter().map(|p| p.cache_write_tokens).sum();
        let total_tokens = providers.iter().map(|p| p.total_tokens).sum();
        let total_cost_usd = providers.iter().map(|p| p.cost_usd).sum();

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
        })
    }

    /// System-metrics time series for the last `minutes`.
    pub async fn metrics(&self, minutes: u32) -> Result<Vec<MetricPoint>> {
        self.rows(&format!(
            "SELECT toString(ts) AS ts, cpu_pct, mem_used_mb, mem_total_mb, mem_pct,
                    load_avg_1, process_rss_mb, process_cpu_pct, active_sessions
             FROM system_metrics
             WHERE ts >= now() - INTERVAL {minutes} MINUTE
             ORDER BY ts",
            minutes = minutes.max(1)
        ))
        .await
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
            let usage_rows = self.scalar("SELECT count() FROM usage_events").await;
            let metric_rows = self.scalar("SELECT count() FROM system_metrics").await;
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
                disk_bytes: dir_size(ch.data_dir()),
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

    /// Run a query and deserialize each row into `T`.
    async fn rows<T: serde::de::DeserializeOwned>(&self, sql: &str) -> Result<Vec<T>> {
        let Some(ch) = self.ch() else { return Ok(Vec::new()) };
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

    /// Run a single-`count()` query, returning 0 on any error.
    async fn scalar(&self, sql: &str) -> u64 {
        let Some(ch) = self.ch() else { return 0 };
        let rows = match ch.query_rows(sql).await {
            Ok(r) => r,
            Err(_) => return 0,
        };
        rows.first()
            .and_then(|r| r.as_object())
            .and_then(|o| o.values().next())
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    }
}

/// Background task: drains the event channel, batching inserts on a timer or
/// when the buffer fills. Exits when the channel closes (engine reinit/drop).
fn spawn_writer(ch: Arc<ClickHouse>, mut rx: mpsc::UnboundedReceiver<UsageEvent>) {
    tokio::spawn(async move {
        let mut buf: Vec<UsageEvent> = Vec::new();
        let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
        loop {
            tokio::select! {
                maybe = rx.recv() => match maybe {
                    Some(ev) => {
                        buf.push(ev);
                        if buf.len() >= FLUSH_BATCH {
                            flush(&ch, &mut buf).await;
                        }
                    }
                    None => {
                        flush(&ch, &mut buf).await;
                        break;
                    }
                },
                _ = ticker.tick() => flush(&ch, &mut buf).await,
            }
        }
    });
}

async fn flush(ch: &ClickHouse, buf: &mut Vec<UsageEvent>) {
    if buf.is_empty() {
        return;
    }
    let payload = ndjson(buf);
    buf.clear();
    if let Err(e) = ch.insert_ndjson("usage_events", &payload).await {
        tracing::warn!("usage: flush failed: {e}");
    }
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
