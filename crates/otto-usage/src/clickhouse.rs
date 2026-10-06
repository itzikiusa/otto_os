//! Managed **persistent** ClickHouse server + a thin HTTP query client.
//!
//! Earlier this shelled out to a fresh `clickhouse local --path` process per
//! query. Each spawn had to re-attach the whole on-disk dataset, and — fatally —
//! an ephemeral process runs no durable background-merge scheduler, so the usage
//! table accumulated tens of thousands of tiny never-merged parts (1.4 GB of
//! metadata for ~12 MB of data) and a ~30 s cold load.
//!
//! Now we run ONE long-lived `clickhouse server` per daemon, bound to a loopback
//! HTTP port, and query it over HTTP. The data dir is attached ONCE at start,
//! caches stay warm, background merges + TTL run continuously (parts stay low,
//! expired rows get dropped), so queries are ~tens of ms and disk stays small.
//!
//! Tested against ClickHouse 26.3/26.6. A `clickhouse local` data dir is adopted
//! in place by synthesizing the `metadata/default.sql` that `local` mode omits.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::process::{Child, Command};

/// HTTP settings appended to every query — match the old CLI flags exactly:
/// emit 64-bit ints as numbers (not strings), and resolve bare identifiers to
/// columns rather than same-named SELECT aliases (avoids ILLEGAL_AGGREGATION on
/// `sum(a+b) AS total` next to `sum(a) AS a`). Verified equivalent over HTTP.
const QUERY_SETTINGS: &str =
    "output_format_json_quote_64bit_integers=0&prefer_column_name_to_alias=1";

/// Handle to a persistent ClickHouse server backing an on-disk `--path` dataset.
///
/// **Idle-stop (R1d).** The server is ~70 threads / ~135 MB RSS even when
/// nothing reads or writes a few MB of usage data. [`Self::maybe_park`] stops
/// it (SIGTERM → clean flush) once no request has run for an idle window and
/// none is in flight; the next query / insert / DDL transparently restarts it
/// ([`Self::ensure_running`], ~0.5 s on a small dir) on a fresh port. Callers
/// never see the difference: the usage writer and the metrics batch keep
/// buffering in memory while the request that woke the server waits.
///
/// Only FOREGROUND requests (queries, usage-event inserts, DDL) count as
/// activity. Background writes ([`Self::insert_ndjson_background`] — the
/// system-metrics batch) neither wake a parked server nor reset the idle
/// clock, or a live session's 5-minute metrics flush would keep the server
/// up forever (perf3 N1/G2).
pub struct ClickHouse {
    bin: PathBuf,
    data_dir: PathBuf,
    http: reqwest::Client,
    /// The CURRENT server process — the port changes across idle restarts.
    proc: Mutex<Proc>,
    /// Serializes park / unpark so two waking requests start one server.
    life: tokio::sync::Mutex<()>,
    /// How many idle-stops / lazy restarts happened (status + perf tests).
    parks: std::sync::atomic::AtomicU64,
    restarts: std::sync::atomic::AtomicU64,
    /// Statements sent + rows they read (from `X-ClickHouse-Summary`) — the
    /// per-summary / per-report query budget guards (R6) read these.
    queries: std::sync::atomic::AtomicU64,
    rows_read: std::sync::atomic::AtomicU64,
    /// Signalled after every successful lazy restart, so the engine can
    /// write the background rows it held back while the server was parked.
    wake_hook: std::sync::OnceLock<std::sync::Arc<tokio::sync::Notify>>,
}

/// Process state behind [`ClickHouse::proc`]. `inflight` lives here (not in
/// an atomic) so "check idle + park" is atomic with "a request begins".
struct Proc {
    /// The server child process. Killed on `shutdown()`/`Drop`.
    child: Option<Child>,
    base_url: String,
    /// Deliberately stopped for idleness — restarts on the next request, and
    /// counts as alive for the self-heal check (it did not crash).
    parked: bool,
    inflight: usize,
    last_use: std::time::Instant,
}

/// RAII marker for one in-flight request: blocks parking while it runs and,
/// for a foreground request (`stamp`), resets the idle clock when it ends.
struct Busy<'a> {
    ch: &'a ClickHouse,
    stamp: bool,
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        if let Ok(mut p) = self.ch.proc.lock() {
            p.inflight = p.inflight.saturating_sub(1);
            if self.stamp {
                p.last_use = std::time::Instant::now();
            }
        }
    }
}

/// An explicit lifetime lease for a sibling service using the HTTP endpoint.
/// Shares the request counter with parking, so acquisition and parking are atomic.
///
/// A BACKGROUND lease ([`ClickHouse::try_keep_awake_background`]) only blocks
/// parking while held: it never wakes a parked server and does not reset the
/// idle clock on release, so a periodic exporter cannot keep the server up.
pub struct ClickHouseLease {
    ch: std::sync::Arc<ClickHouse>,
    pub endpoint: String,
    stamp: bool,
}
impl Drop for ClickHouseLease {
    fn drop(&mut self) {
        if let Ok(mut p) = self.ch.proc.lock() {
            p.inflight = p.inflight.saturating_sub(1);
            if self.stamp {
                p.last_use = std::time::Instant::now();
            }
        }
    }
}

impl ClickHouse {
    /// Keep the current engine awake while an external local exporter uses it.
    /// Reacquire on engine replacement; the port is intentionally not permanent.
    pub async fn keep_awake(self: &std::sync::Arc<Self>) -> Result<ClickHouseLease> {
        self.proc.lock().unwrap().inflight += 1;
        let mut lease = ClickHouseLease {
            ch: self.clone(),
            endpoint: String::new(),
            stamp: true,
        };
        let (_busy, endpoint) = self.begin().await?;
        lease.endpoint = endpoint;
        Ok(lease)
    }

    /// Background WAKE: restarts a parked server for one bounded export but,
    /// unlike [`Self::keep_awake`], never resets the idle clock — once the lease
    /// drops, the next idle check may park the server again right away.
    pub async fn wake_background(self: &std::sync::Arc<Self>) -> Result<ClickHouseLease> {
        let parked = {
            let mut p = self.proc.lock().unwrap();
            p.inflight += 1;
            p.parked
        };
        let mut lease = ClickHouseLease {
            ch: self.clone(),
            endpoint: String::new(),
            stamp: false,
        };
        if parked {
            self.unpark().await?;
        }
        lease.endpoint = self.proc.lock().unwrap().base_url.clone();
        Ok(lease)
    }

    /// Background lease: `None` while the server is parked (or has no child),
    /// otherwise holds parking off until dropped WITHOUT resetting the idle
    /// clock. The in-flight mark is taken under the same lock `maybe_park`
    /// decides under, so the server cannot stop under a held lease.
    pub fn try_keep_awake_background(self: &std::sync::Arc<Self>) -> Option<ClickHouseLease> {
        let mut p = self.proc.lock().unwrap();
        if p.parked || p.child.is_none() {
            return None;
        }
        p.inflight += 1;
        Some(ClickHouseLease {
            ch: self.clone(),
            endpoint: p.base_url.clone(),
            stamp: false,
        })
    }

    /// Resolve the `clickhouse` binary in priority order: an explicit configured
    /// path, then `PATH`, then well-known install locations. Returns an absolute
    /// path so the daemon can run it regardless of the working directory.
    pub fn locate(configured: Option<&str>) -> Option<PathBuf> {
        if let Some(p) = configured.map(str::trim).filter(|s| !s.is_empty()) {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                return Some(pb);
            }
        }
        if let Ok(out) = std::process::Command::new("which")
            .arg("clickhouse")
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() && Path::new(&s).is_file() {
                    return Some(PathBuf::from(s));
                }
            }
        }
        let mut candidates = vec![
            PathBuf::from("/usr/local/bin/clickhouse"),
            PathBuf::from("/opt/homebrew/bin/clickhouse"),
        ];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("clickhouse"));
            candidates.push(home.join(".local/bin/clickhouse"));
            candidates.push(home.join("Library/Application Support/Otto/bin/clickhouse"));
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    pub fn binary(&self) -> &Path {
        &self.bin
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Start (or re-claim then start) a persistent server over `data_dir` and
    /// return a client once it answers `/ping`. Adopts an existing
    /// `clickhouse local` dir in place. Retries on a transient port race.
    pub async fn start(bin: PathBuf, data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| Error::Internal(format!("create clickhouse dir: {e}")))?;
        // Reclaim the dir from any orphaned server (crash / unclean shutdown).
        reclaim_dir(&data_dir).await;
        // `clickhouse local` omits metadata/default.sql; `server` requires it.
        ensure_default_metadata(&data_dir)?;

        // No global timeout — each request sets its own (queries are fast; DDL /
        // OPTIMIZE FINAL can take many minutes), so a slow merge isn't killed by a
        // short client cap.
        //
        // No idle keep-alive pool: on SIGTERM the server waits for every open
        // client connection ("Waiting for 1 outstanding connections") before
        // exiting, so one pooled idle socket turned each shutdown/restart into
        // a ~5-10 s stall (measured 10.1 s vs 0.1 s). A fresh loopback connect
        // per request costs well under a millisecond.
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .build()
            .map_err(|e| Error::Internal(format!("http client: {e}")))?;

        let (child, base_url) = spawn_server(&bin, &data_dir, &http).await?;
        Ok(Self {
            bin,
            data_dir,
            http,
            proc: Mutex::new(Proc {
                child: Some(child),
                base_url,
                parked: false,
                inflight: 0,
                last_use: std::time::Instant::now(),
            }),
            life: tokio::sync::Mutex::new(()),
            parks: Default::default(),
            restarts: Default::default(),
            queries: Default::default(),
            rows_read: Default::default(),
            wake_hook: std::sync::OnceLock::new(),
        })
    }

    /// Register the `Notify` signalled after each lazy restart (first call
    /// wins). A permit is kept when nobody waits, so a wake is never missed.
    pub fn set_wake_hook(&self, hook: std::sync::Arc<tokio::sync::Notify>) {
        let _ = self.wake_hook.set(hook);
    }

    /// Whether the spawned `clickhouse server` child is still running. `false`
    /// once it exited (crashed, or killed — e.g. by another process reclaiming
    /// the data dir). This is the engine's self-heal signal: a dead child with
    /// failing inserts means "restart the server", not "keep warning forever".
    /// An idle-PARKED server counts as alive: it stopped on purpose and the
    /// next request restarts it (a failed restart clears `parked`, so a server
    /// that cannot come back still trips the heal).
    pub fn server_alive(&self) -> bool {
        let mut p = self.proc.lock().unwrap();
        if p.parked {
            return true;
        }
        match p.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Whether the server is currently idle-stopped (no process running).
    pub fn is_parked(&self) -> bool {
        self.proc.lock().unwrap().parked
    }

    /// `(idle-stops, lazy restarts)` since this handle was created.
    pub fn park_stats(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.parks.load(Relaxed), self.restarts.load(Relaxed))
    }

    /// `(statements sent, rows read)` since this handle was created. Rows come
    /// from each response's `X-ClickHouse-Summary` header.
    pub fn query_stats(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.queries.load(Relaxed), self.rows_read.load(Relaxed))
    }

    /// The server's OS pid, when one is running (perf probes).
    pub fn server_pid(&self) -> Option<u32> {
        self.proc
            .lock()
            .unwrap()
            .child
            .as_ref()
            .and_then(|c| c.id())
    }

    /// Mark one request in flight and return the server URL to send it to,
    /// restarting a parked server first. The in-flight mark is taken BEFORE
    /// the parked check (under the same lock `maybe_park` decides under), so a
    /// request can never race onto a server that is being stopped.
    async fn begin(&self) -> Result<(Busy<'_>, String)> {
        let parked = {
            let mut p = self.proc.lock().unwrap();
            p.inflight += 1;
            p.parked
        };
        let busy = Busy {
            ch: self,
            stamp: true,
        };
        if parked {
            self.unpark().await?;
        }
        let url = self.proc.lock().unwrap().base_url.clone();
        Ok((busy, url))
    }

    /// Restart a parked server (serialized; a no-op if another request
    /// already did). A failed restart leaves `parked = false` with no child,
    /// so [`Self::server_alive`] reports dead and the engine's heal reinits.
    async fn unpark(&self) -> Result<()> {
        let _g = self.life.lock().await;
        if !self.proc.lock().unwrap().parked {
            return Ok(());
        }
        let started = std::time::Instant::now();
        let res = spawn_server(&self.bin, &self.data_dir, &self.http).await;
        let mut p = self.proc.lock().unwrap();
        p.parked = false;
        match res {
            Ok((child, base_url)) => {
                p.child = Some(child);
                p.base_url = base_url;
                self.restarts
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::info!(
                    "usage: clickhouse woke from idle-stop in {} ms",
                    started.elapsed().as_millis()
                );
                if let Some(hook) = self.wake_hook.get() {
                    hook.notify_one();
                }
                Ok(())
            }
            Err(e) => {
                p.child = None;
                Err(e)
            }
        }
    }

    /// Idle-stop the server when nothing is in flight and the last request
    /// ended at least `idle_after` ago. Returns whether it parked. The SIGTERM
    /// path flushes cleanly; the data dir lock is released, so the lazy
    /// restart re-attaches the same dataset.
    pub async fn maybe_park(&self, idle_after: Duration) -> bool {
        let _g = self.life.lock().await;
        let child = {
            let mut p = self.proc.lock().unwrap();
            if p.parked || p.child.is_none() || p.inflight > 0 {
                return false;
            }
            if p.last_use.elapsed() < idle_after {
                return false;
            }
            p.parked = true;
            p.child.take()
        };
        if let Some(child) = child {
            stop_child(child).await;
        }
        self.parks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::info!(
            "usage: clickhouse idle for {}s — stopped until the next query",
            idle_after.as_secs()
        );
        true
    }

    /// POST `sql` (with `settings`) and return the raw response body, erroring on
    /// a non-2xx (ClickHouse returns a readable message in the body). `timeout`
    /// bounds the request — short for queries, long for DDL/OPTIMIZE.
    async fn post(&self, sql: String, settings: &str, timeout: Duration) -> Result<String> {
        let (busy, base_url) = self.begin().await?;
        self.post_on(busy, &base_url, sql, settings, timeout).await
    }

    /// [`Self::post`] on an already-marked in-flight request; `_busy` is held
    /// until the reply is in.
    async fn post_on(
        &self,
        _busy: Busy<'_>,
        base_url: &str,
        sql: String,
        settings: &str,
        timeout: Duration,
    ) -> Result<String> {
        let url = if settings.is_empty() {
            format!("{base_url}/")
        } else {
            format!("{base_url}/?{settings}")
        };
        let resp = self
            .http
            .post(&url)
            .timeout(timeout)
            .body(sql)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse http: {e}")))?;
        self.queries
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(n) = resp
            .headers()
            .get("x-clickhouse-summary")
            .and_then(|v| v.to_str().ok())
            .and_then(summary_read_rows)
        {
            self.rows_read
                .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        }
        let ok = resp.status().is_success();
        let body = resp
            .text()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse response body: {e}")))?;
        if ok {
            Ok(body)
        } else {
            Err(Error::Internal(format!(
                "clickhouse query failed: {}",
                body.trim()
            )))
        }
    }

    /// `clickhouse server`'s version via `SELECT version()`, falling back to the
    /// binary's `--version` if the server isn't reachable.
    pub async fn version(&self) -> Result<String> {
        if let Ok(rows) = self.query_rows("SELECT version() AS v").await {
            if let Some(v) = rows
                .first()
                .and_then(|r| r.get("v"))
                .and_then(|v| v.as_str())
            {
                return Ok(format!("ClickHouse server version {v}"));
            }
        }
        let out = Command::new(&self.bin)
            .arg("local")
            .arg("--version")
            .output()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse --version: {e}")))?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string())
    }

    /// Run one or more `;`-separated statements that produce no rows (DDL,
    /// `ALTER`, …). Each statement is sent as its own HTTP request (our DDL never
    /// embeds `;` inside string literals, so a top-level split is safe).
    pub async fn exec(&self, sql: &str) -> Result<()> {
        // DDL/ALTER are fast, but OPTIMIZE … FINAL can take many minutes — allow up
        // to 30m per statement (the engine also wraps OPTIMIZE in its own timeout).
        for stmt in split_statements(sql) {
            // Buffer the (normally empty) result until execution finishes. A
            // streamed HTTP 200 can otherwise precede a server-side failure.
            self.post(stmt, "wait_end_of_query=1", Duration::from_secs(1800))
                .await?;
        }
        Ok(())
    }

    /// Run a `SELECT` and return each row as a JSON object.
    pub async fn query_rows(&self, sql: &str) -> Result<Vec<serde_json::Value>> {
        let body = self
            .post(
                format!("{sql}\nFORMAT JSONEachRow"),
                QUERY_SETTINGS,
                Duration::from_secs(120),
            )
            .await?;
        parse_rows(&body)
    }

    /// [`Self::query_rows`] for BACKGROUND readers (S9-303: the budget
    /// sampler, budget gates, swarm per-turn totals): never resets the idle
    /// clock, so a periodic reader cannot keep the server up. A parked server
    /// is left parked (`Ok(None)`) unless `wake`, which restarts it for this
    /// one query without stamping — the next idle check may park it again.
    pub async fn query_rows_background(
        &self,
        sql: &str,
        wake: bool,
    ) -> Result<Option<Vec<serde_json::Value>>> {
        let (parked, base_url) = {
            let mut p = self.proc.lock().unwrap();
            if p.parked && !wake {
                return Ok(None);
            }
            p.inflight += 1;
            (p.parked, p.base_url.clone())
        };
        let busy = Busy {
            ch: self,
            stamp: false,
        };
        let base_url = if parked {
            self.unpark().await?;
            self.proc.lock().unwrap().base_url.clone()
        } else {
            base_url
        };
        let body = self
            .post_on(
                busy,
                &base_url,
                format!("{sql}\nFORMAT JSONEachRow"),
                QUERY_SETTINGS,
                Duration::from_secs(120),
            )
            .await?;
        parse_rows(&body).map(Some)
    }

    /// Run several queries and return their row sets in order — CONCURRENT
    /// `query_rows` against the persistent server (a summary's five rollups
    /// cost one round trip of latency, not five). ALL-OR-NOTHING: any failure
    /// errors the whole batch, and callers propagate it (a failed read must
    /// surface as an error, never as an all-zero dashboard).
    pub async fn query_batch(&self, queries: &[String]) -> Result<Vec<Vec<serde_json::Value>>> {
        futures_util::future::try_join_all(queries.iter().map(|q| self.query_rows(q))).await
    }

    /// Bulk insert into `table` from newline-delimited JSON (one object per line).
    /// A blank payload is a no-op. `date_time_input_format=best_effort` lets
    /// rows carry RFC3339 timestamps (e.g. `UsageEvent.ts`) in `DateTime64`
    /// columns; rows that omit them still get the column DEFAULTs.
    ///
    /// `async_insert=1&wait_for_async_insert=1` lets the server coalesce
    /// inserts from several writers (the usage flusher, the metrics sampler,
    /// the rebuild) into fewer parts, while the call still returns only once
    /// the rows are durable — so failure handling is unchanged.
    pub async fn insert_ndjson(&self, table: &str, ndjson: &str) -> Result<()> {
        if ndjson.trim().is_empty() {
            return Ok(());
        }
        let (busy, base_url) = self.begin().await?;
        self.send_insert(busy, &base_url, table, ndjson).await
    }

    /// [`Self::insert_ndjson`] for BACKGROUND rows (system metrics): never
    /// wakes a parked server and never resets the idle clock. Returns
    /// `Ok(false)` without sending when the server is parked — the caller
    /// keeps the rows and writes them after the next foreground wake. The
    /// parked check and the in-flight mark share the lock `maybe_park`
    /// decides under, so the server cannot stop mid-insert.
    pub async fn insert_ndjson_background(&self, table: &str, ndjson: &str) -> Result<bool> {
        if ndjson.trim().is_empty() {
            return Ok(true);
        }
        let base_url = {
            let mut p = self.proc.lock().unwrap();
            if p.parked {
                return Ok(false);
            }
            p.inflight += 1;
            p.base_url.clone()
        };
        let busy = Busy {
            ch: self,
            stamp: false,
        };
        self.send_insert(busy, &base_url, table, ndjson)
            .await
            .map(|()| true)
    }

    /// POST one `JSONEachRow` insert; `_busy` is held until the reply is in.
    async fn send_insert(
        &self,
        _busy: Busy<'_>,
        base_url: &str,
        table: &str,
        ndjson: &str,
    ) -> Result<()> {
        let q = urlencode(&format!("INSERT INTO {table} FORMAT JSONEachRow"));
        let url =
            format!("{base_url}/?query={q}&date_time_input_format=best_effort{INSERT_SETTINGS}");
        let resp = self
            .http
            .post(&url)
            .timeout(Duration::from_secs(120))
            .body(ndjson.to_string())
            .send()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse insert http: {e}")))?;
        let ok = resp.status().is_success();
        let body = resp.text().await.unwrap_or_default();
        if ok {
            Ok(())
        } else {
            Err(Error::Internal(format!(
                "clickhouse insert failed: {}",
                body.trim()
            )))
        }
    }

    /// A handle with no server child, pointed at `base_url` — writer tests.
    /// `parked` makes it look idle-stopped; its (empty) binary path makes a
    /// wake fail fast instead of spawning anything.
    #[cfg(test)]
    pub(crate) fn for_tests(base_url: &str, parked: bool) -> Self {
        Self {
            bin: PathBuf::new(),
            data_dir: PathBuf::new(),
            http: reqwest::Client::new(),
            proc: Mutex::new(Proc {
                child: None,
                base_url: base_url.to_string(),
                parked,
                inflight: 0,
                last_use: std::time::Instant::now(),
            }),
            life: tokio::sync::Mutex::new(()),
            parks: Default::default(),
            restarts: Default::default(),
            queries: Default::default(),
            rows_read: Default::default(),
            wake_hook: std::sync::OnceLock::new(),
        }
    }

    /// Stop the server: SIGTERM (clean flush), bounded wait, SIGKILL fallback.
    /// Final — a shut-down handle is not restarted by later requests.
    pub async fn shutdown(&self) {
        let child = self.proc.lock().ok().and_then(|mut g| {
            g.parked = false;
            g.child.take()
        });
        if let Some(child) = child {
            stop_child(child).await;
        }
    }
}

/// SIGTERM (clean flush of in-flight inserts + merges), bounded wait, SIGKILL.
async fn stop_child(mut child: Child) {
    if let Some(pid) = child.id() {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .output()
            .await;
    }
    match tokio::time::timeout(Duration::from_secs(6), child.wait()).await {
        Ok(_) => {}
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }
}

/// Spawn `clickhouse server` over `data_dir` on a fresh loopback port and wait
/// for `/ping`; retries a transient port race. Returns the child + its URL.
async fn spawn_server(
    bin: &Path,
    data_dir: &Path,
    http: &reqwest::Client,
) -> Result<(Child, String)> {
    let mut last_err = String::new();
    for attempt in 0..3 {
        let port = free_loopback_port().map_err(|e| Error::Internal(format!("pick port: {e}")))?;
        let cfg = write_server_config(data_dir, port)?;
        let base_url = format!("http://127.0.0.1:{port}");
        tracing::info!(
            "usage: starting clickhouse server (binary {}, data {}, port {})",
            bin.display(),
            data_dir.display(),
            port
        );
        let mut child = Command::new(bin)
            .arg("server")
            .arg(format!("--config-file={}", cfg.display()))
            // No watchdog parent (a second 32 MB process whose only job is
            // restarting a crashed server): the engine's self-heal already
            // restarts a dead child, and the child we hold IS the server.
            .env("CLICKHOUSE_WATCHDOG_ENABLE", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::Internal(format!("spawn clickhouse server: {e}")))?;

        if wait_ping(http, &base_url, &mut child, Duration::from_secs(45)).await {
            return Ok((child, base_url));
        }
        last_err = format!(
            "server did not answer /ping on port {port} (attempt {})",
            attempt + 1
        );
        tracing::warn!("usage: {last_err} — retrying");
        stop_child(child).await; // kill the failed child before retrying
    }
    Err(Error::Internal(format!(
        "clickhouse server failed to start: {last_err}"
    )))
}

/// `read_rows` from an `X-ClickHouse-Summary` header (`{"read_rows":"12",…}`).
fn summary_read_rows(h: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(h).ok()?;
    let r = v.get("read_rows")?;
    r.as_u64().or_else(|| r.as_str()?.parse().ok())
}

/// Poll `/ping` until "Ok." or `timeout`; gives up early if the child exits.
async fn wait_ping(
    http: &reqwest::Client,
    base_url: &str,
    child: &mut Child,
    timeout: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    let url = format!("{base_url}/ping");
    // Fast first polls: a small dir answers in ~0.3-0.6 s, and a lazy
    // restart (idle-stop wake) has a request waiting on it.
    let mut delay = Duration::from_millis(50);
    while std::time::Instant::now() < deadline {
        if let Ok(resp) = http.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(body) = resp.text().await {
                    if body.trim() == "Ok." {
                        return true;
                    }
                }
            }
        }
        if !matches!(child.try_wait(), Ok(None)) {
            return false;
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_millis(300));
    }
    false
}

impl Drop for ClickHouse {
    fn drop(&mut self) {
        // Safety net if `shutdown()` wasn't called (e.g. a panic). `kill_on_drop`
        // already arms SIGKILL; this makes it explicit while the runtime is alive.
        if let Ok(mut g) = self.proc.lock() {
            if let Some(mut child) = g.child.take() {
                let _ = child.start_kill();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers (unit-tested where pure)
// ---------------------------------------------------------------------------

/// Split a `;`-separated SQL script into trimmed, non-empty statements. Strips
/// `--` line comments FIRST so a `;` inside a comment (our schema has one) doesn't
/// split a statement mid-way. Safe for our controlled DDL (no `--` or `;` inside
/// string literals).
/// URL settings appended to every insert (see [`ClickHouse::insert_ndjson`]).
const INSERT_SETTINGS: &str =
    "&async_insert=1&wait_for_async_insert=1&async_insert_busy_timeout_ms=10000";

fn split_statements(sql: &str) -> Vec<String> {
    let no_comments: String = sql
        .lines()
        .map(|l| match l.find("--") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    no_comments
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Minimal percent-encoding for the few chars that appear in our INSERT query
/// strings (space + the reserved ones). Avoids a urlencoding dep.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Escape XML special chars for safe inclusion in the generated config.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Grab a free loopback TCP port by binding to `:0` and reading it back.
fn free_loopback_port() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = l.local_addr()?.port();
    drop(l);
    Ok(port)
}

/// Parse a `JSONEachRow` body into one JSON object per non-blank line.
fn parse_rows(body: &str) -> Result<Vec<serde_json::Value>> {
    let mut rows = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line)
            .map_err(|e| Error::Internal(format!("parse clickhouse row: {e}")))?;
        rows.push(v);
    }
    Ok(rows)
}

/// Generate the minimal server config and return its path. Loopback-only, empty
/// default user (machine-local trust boundary, same as the old `local` mode),
/// memory-capped to stay desktop-light.
///
/// Footprint (SE-15: 808 threads / ~450 MB RSS idle for ~19 MB of data): the
/// global pool no longer parks up to 1000 idle threads, the schedule pools
/// (engines Otto doesn't use: Kafka, Distributed, Buffer) are shrunk, the mark
/// cache is 64 MB, and async metrics refresh every 60 s instead of 1 s.
/// `background_pool_size` is only lowered TOGETHER with MergeTree's
/// `number_of_free_entries_in_pool_*` settings (see R1c below) — it validates
/// them against the pool and refuses to attach tables when the pool is
/// smaller. Deliberately NOT touched: `max_thread_pool_size` (a hard cap makes
/// queries fail with "no free thread" instead of waiting).
///
/// Round 3 (r3-08-04 / r3-12-03; measured on a scratch data dir with the
/// bundled 26.7 build: attach, lightweight DELETE, mutation, OPTIMIZE FINAL and
/// a restart re-attach all pass):
/// - pools Otto has no use for are cut to the minimum: the schedule pool
///   16 → 4, `Common` 8 → 2, `Move` (multi-disk TTL moves) 8 → 1, `Fetch`
///   (replication) 16 → 1, and at most 4 idle global-pool threads parked;
/// - the jemalloc `MemoryWorker` refreshes once a second instead of the
///   build's default cadence, and the expression-JIT cache is 16 MB;
/// - `max_server_memory_usage` is a hard 1 GiB (the 0.3 ratio alone allowed
///   ~15 GB on this machine for ~20 MB of data);
/// - `memory_worker_use_cgroup` is OFF (S12-304): on Linux the MemoryWorker
///   otherwise corrects its tracker from the enclosing cgroup's usage — on a
///   CI runner that is the whole job (5.2 GiB), so every query tripped the
///   1 GiB cap while the server itself used ~150 MB. A no-op on macOS. The
///   setting exists in both the local 26.6 and CI's pinned 26.8 builds (and
///   an unknown server-config element would be ignored, not fatal);
/// - NO system log section (`query_log`, `trace_log`, `metric_log`,
///   `asynchronous_metric_log`, `part_log`, `text_log`, …) is declared, and with
///   a standalone config the server creates none (verified: `system.tables`
///   has no `*_log`). Never add one, not even `<x remove="1"/>`: without a base
///   config to merge into, an empty section would ENABLE that log.
///
/// Net (idle, scratch dir): threads 125 + a 3-thread watchdog → 80, the 32 MB
/// watchdog process gone (see `CLICKHOUSE_WATCHDOG_ENABLE` at spawn), CPU
/// 0.7–0.8 % → 0.5–0.6 %.
///
/// Perf wave (U7a): the `MemoryWorker` wakes every 10 s instead of 1 s (it
/// only refreshes the jemalloc RSS figure the 1 GiB cap reads), the IO /
/// parts-loading / parts-cleaning / table-loader pools are capped at a couple
/// of threads (a few MB of data never needs more), and the merge selector
/// sleeps 30 s backing off to 5 min instead of polling every few seconds —
/// with ~1 insert per 15 s there is never a merge to pick sooner.
///
/// Perf wave 2 (R1c): `background_pool_size` 16 → 4 (merges + mutations;
/// × the concurrency ratio 2 = 8 slots), PAIRED with the MergeTree
/// `number_of_free_entries_in_pool_*` thresholds lowered to 2 — the defaults
/// (8 / 20 / 25) exceed a 4-thread pool and MergeTree would refuse to attach
/// the tables. Measured on a scratch dir with the bundled build (attach,
/// `ALTER … DELETE` mutation, `OPTIMIZE FINAL`, restart re-attach all pass):
/// idle threads 70 → 58, idle CPU 0.47 % → 0.40 %, restart to `/ping` ~0.5 s.
/// Idle-stop ([`ClickHouse::maybe_park`]) takes it to 0 when nothing reads.
fn write_server_config(data_dir: &Path, port: u16) -> Result<PathBuf> {
    let server_dir = data_dir.join("server");
    std::fs::create_dir_all(&server_dir)
        .map_err(|e| Error::Internal(format!("create server dir: {e}")))?;
    std::fs::create_dir_all(data_dir.join("tmp"))
        .map_err(|e| Error::Internal(format!("create tmp dir: {e}")))?;
    let path = xml_escape(&format!("{}/", data_dir.to_string_lossy()));
    let tmp = xml_escape(&format!("{}/tmp/", data_dir.to_string_lossy()));
    // Console-only logging (stdout/stderr are /dev/null at spawn): Poco's
    // FileChannel never recovers from ENOSPC — once the disk filled, every log
    // call threw from RotateBySizeStrategy::mustRotate and the two AsyncLogger
    // threads spun at ~2 cores until restart, even after space was freed.
    // Server failures still surface to the daemon as query errors.
    //
    // `disable_internal_dns_cache`: a loopback-only single node never resolves a
    // peer, yet with the cache on the server resolves its OWN hostname at boot
    // and again from a 15 s updater task that shutdown waits on. When the Mac's
    // `<name>.local` doesn't resolve (mDNS off/blocked, sandboxes) each lookup
    // sits out a 5 s timeout: measured 5.5 s to the first /ping and 4.9 s to
    // exit on SIGTERM, vs 0.6 s / 0.2 s with the cache off.
    let xml = format!(
        "<clickhouse>\n\
         <logger><level>error</level><console>1</console></logger>\n\
         <http_port>{port}</http_port>\n\
         <listen_host>127.0.0.1</listen_host>\n\
         <disable_internal_dns_cache>1</disable_internal_dns_cache>\n\
         <path>{path}</path>\n\
         <tmp_path>{tmp}</tmp_path>\n\
         <mark_cache_size>67108864</mark_cache_size>\n\
         <uncompressed_cache_size>0</uncompressed_cache_size>\n\
         <max_server_memory_usage_to_ram_ratio>0.3</max_server_memory_usage_to_ram_ratio>\n\
         <max_server_memory_usage>1073741824</max_server_memory_usage>\n\
         <max_thread_pool_free_size>4</max_thread_pool_free_size>\n\
         <background_schedule_pool_size>4</background_schedule_pool_size>\n\
         <background_common_pool_size>2</background_common_pool_size>\n\
         <background_move_pool_size>1</background_move_pool_size>\n\
         <background_fetches_pool_size>1</background_fetches_pool_size>\n\
         <memory_worker_period_ms>10000</memory_worker_period_ms>\n\
         <memory_worker_use_cgroup>0</memory_worker_use_cgroup>\n\
         <max_io_thread_pool_size>8</max_io_thread_pool_size>\n\
         <max_io_thread_pool_free_size>0</max_io_thread_pool_free_size>\n\
         <max_active_parts_loading_thread_pool_size>2</max_active_parts_loading_thread_pool_size>\n\
         <max_outdated_parts_loading_thread_pool_size>2</max_outdated_parts_loading_thread_pool_size>\n\
         <max_parts_cleaning_thread_pool_size>2</max_parts_cleaning_thread_pool_size>\n\
         <tables_loader_foreground_pool_size>2</tables_loader_foreground_pool_size>\n\
         <tables_loader_background_pool_size>2</tables_loader_background_pool_size>\n\
         <background_pool_size>4</background_pool_size>\n\
         <background_merges_mutations_concurrency_ratio>2</background_merges_mutations_concurrency_ratio>\n\
         <merge_tree><merge_selecting_sleep_ms>30000</merge_selecting_sleep_ms>\
<max_merge_selecting_sleep_ms>300000</max_merge_selecting_sleep_ms>\
<number_of_free_entries_in_pool_to_execute_mutation>2</number_of_free_entries_in_pool_to_execute_mutation>\
<number_of_free_entries_in_pool_to_lower_max_size_of_merge>2</number_of_free_entries_in_pool_to_lower_max_size_of_merge>\
<number_of_free_entries_in_pool_to_execute_optimize_entire_partition>2</number_of_free_entries_in_pool_to_execute_optimize_entire_partition>\
</merge_tree>\n\
         <compiled_expression_cache_size>16777216</compiled_expression_cache_size>\n\
         <background_message_broker_schedule_pool_size>2</background_message_broker_schedule_pool_size>\n\
         <background_distributed_schedule_pool_size>2</background_distributed_schedule_pool_size>\n\
         <background_buffer_flush_schedule_pool_size>2</background_buffer_flush_schedule_pool_size>\n\
         <asynchronous_metrics_update_period_s>60</asynchronous_metrics_update_period_s>\n\
         <users><default><password/><networks><ip>127.0.0.1</ip></networks>\
         <profile>default</profile><quota>default</quota></default></users>\n\
         <profiles><default/></profiles><quotas><default/></quotas>\n\
         </clickhouse>\n"
    );
    let cfg = server_dir.join("config.xml");
    std::fs::write(&cfg, xml).map_err(|e| Error::Internal(format!("write config: {e}")))?;
    Ok(cfg)
}

/// `clickhouse local` creates the default database as a symlink under
/// `metadata/default` but writes no `metadata/default.sql`; `clickhouse server`
/// requires that ATTACH file (else "Data directory for default database exists,
/// but metadata file does not", Code 48). Synthesize it from the symlink's UUID.
/// No-op for a fresh dir (server creates the default DB itself) or an
/// already-adopted dir.
fn ensure_default_metadata(data_dir: &Path) -> Result<()> {
    let meta = data_dir.join("metadata");
    let link = meta.join("default");
    let sql = meta.join("default.sql");
    if sql.exists() {
        return Ok(());
    }
    // Only act when there's a symlinked default DB to adopt.
    let Ok(target) = std::fs::read_link(&link) else {
        return Ok(());
    };
    let uuid = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    if uuid.len() != 36 {
        // Not a UUID-named store dir — leave it to ClickHouse.
        tracing::warn!(
            "usage: default db link target not a uuid ({uuid:?}); skipping metadata synth"
        );
        return Ok(());
    }
    let body = format!("ATTACH DATABASE _ UUID '{uuid}'\nENGINE = Atomic\n");
    std::fs::write(&sql, body).map_err(|e| Error::Internal(format!("write default.sql: {e}")))?;
    tracing::info!(
        "usage: synthesized metadata/default.sql for in-place server adoption (uuid {uuid})"
    );
    Ok(())
}

/// If a stale ClickHouse server still holds `data_dir` (unclean shutdown), stop
/// it so we can start ours. Reads the `status` file's `PID:` line and verifies
/// the process is actually a clickhouse pointed at this dir (PID-reuse-safe)
/// before signalling.
async fn reclaim_dir(data_dir: &Path) {
    let status = data_dir.join("status");
    let Ok(text) = std::fs::read_to_string(&status) else {
        return;
    };
    let Some(pid) = parse_status_pid(&text) else {
        return;
    };
    // Verify the PID is a clickhouse process referencing OUR data dir.
    let out = match Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("command=")
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return,
    };
    let cmd = String::from_utf8_lossy(&out.stdout);
    let dir_s = data_dir.to_string_lossy();
    if !(cmd.contains("clickhouse") && cmd.contains(dir_s.as_ref())) {
        return; // stale status file / PID reused by an unrelated process
    }
    tracing::warn!("usage: reclaiming clickhouse data dir from stale server pid {pid}");
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .output()
        .await;
    // Wait up to 5s for it to exit, then SIGKILL.
    for _ in 0..50 {
        let alive = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !alive {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .output()
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
}

/// Parse the PID from a ClickHouse `status` file (first `PID: <n>` line).
fn parse_status_pid(text: &str) -> Option<u32> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("PID:") {
            if let Ok(pid) = rest.trim().parse::<u32>() {
                return Some(pid);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exec_rejects_truncated_success_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let n = socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort",
                )
                .await
                .unwrap();
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        let ch = ClickHouse {
            bin: PathBuf::new(),
            data_dir: PathBuf::new(),
            http: reqwest::Client::new(),
            proc: Mutex::new(Proc {
                child: None,
                base_url: format!("http://{addr}"),
                parked: false,
                inflight: 0,
                last_use: std::time::Instant::now(),
            }),
            life: tokio::sync::Mutex::new(()),
            parks: Default::default(),
            restarts: Default::default(),
            queries: Default::default(),
            rows_read: Default::default(),
            wake_hook: std::sync::OnceLock::new(),
        };
        let result = ch
            .exec("CREATE TABLE example (n UInt64) ENGINE=Memory")
            .await;
        let request = server.await.unwrap();
        assert!(
            result.is_err(),
            "a lost response body must not acknowledge a migration step"
        );
        assert!(
            request.contains("wait_end_of_query=1"),
            "exec must wait for query completion before HTTP success"
        );
    }

    #[test]
    fn server_config_logs_to_console_not_files() {
        // A file log wedges the server after ENOSPC (Poco rotation throws per
        // log call and the logger threads spin), so no <log>/<errorlog> file.
        let dir = tempfile::tempdir().unwrap();
        let cfg = write_server_config(dir.path(), 18123).unwrap();
        let xml = std::fs::read_to_string(cfg).unwrap();
        assert!(xml.contains("<console>1</console>"), "{xml}");
        assert!(
            !xml.contains("<log>") && !xml.contains("<errorlog>"),
            "{xml}"
        );
    }

    #[test]
    fn split_statements_basic() {
        let s = split_statements("CREATE TABLE a (x Int);\n  ALTER TABLE a ADD COLUMN y Int; \n");
        assert_eq!(s.len(), 2);
        assert!(s[0].starts_with("CREATE TABLE a"));
        assert!(s[1].starts_with("ALTER TABLE a"));
        assert!(split_statements("  ;; \n ;").is_empty());
    }

    #[test]
    fn split_statements_ignores_semicolon_in_line_comment() {
        // The real schema has `-- … (B1); nullable …` — the `;` is INSIDE a comment
        // and must NOT split the CREATE TABLE.
        let sql = "CREATE TABLE t (\n  a Int,\n  -- note (B1); still one statement\n  b Int\n) ENGINE=MergeTree ORDER BY a;\nALTER TABLE t ADD COLUMN c Int;";
        let s = split_statements(sql);
        assert_eq!(s.len(), 2, "comment semicolon must not split: {s:?}");
        assert!(s[0].contains("CREATE TABLE t") && s[0].contains("b Int"));
        assert!(s[1].starts_with("ALTER TABLE t"));
    }

    #[test]
    fn urlencode_reserved() {
        assert_eq!(
            urlencode("INSERT INTO t FORMAT JSONEachRow"),
            "INSERT%20INTO%20t%20FORMAT%20JSONEachRow"
        );
        assert_eq!(urlencode("abc-_.~"), "abc-_.~");
    }

    #[test]
    fn xml_escape_specials() {
        assert_eq!(
            xml_escape("/a&b/<c>/\"d\"/'e'"),
            "/a&amp;b/&lt;c&gt;/&quot;d&quot;/&apos;e&apos;"
        );
        assert_eq!(
            xml_escape("/Users/me/Application Support/Otto"),
            "/Users/me/Application Support/Otto"
        );
    }

    #[test]
    fn parse_status_pid_works() {
        let s = "PID: 96987\nStarted at: 2026-06-26 23:42:34\nRevision: 54508\n";
        assert_eq!(parse_status_pid(s), Some(96987));
        assert_eq!(parse_status_pid("Started at: x\nRevision: 1"), None);
        assert_eq!(parse_status_pid("PID: notanumber"), None);
    }

    #[test]
    fn summary_header_read_rows() {
        assert_eq!(
            summary_read_rows(r#"{"read_rows":"244","read_bytes":"9"}"#),
            Some(244)
        );
        assert_eq!(summary_read_rows(r#"{"read_rows":7}"#), Some(7));
        assert_eq!(summary_read_rows("nope"), None);
    }

    #[test]
    fn config_xml_well_formed_and_escaped() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write_server_config(dir.path(), 18999).unwrap();
        let xml = std::fs::read_to_string(&cfg).unwrap();
        assert!(xml.contains("<http_port>18999</http_port>"));
        assert!(xml.contains("<listen_host>127.0.0.1</listen_host>"));
        // No 5 s self-hostname DNS stall at boot / shutdown.
        assert!(xml.contains("<disable_internal_dns_cache>1</disable_internal_dns_cache>"));
        assert!(xml.contains("max_server_memory_usage_to_ram_ratio"));
        assert!(xml.contains("<max_server_memory_usage>1073741824</max_server_memory_usage>"));
        // S12-304: the 1 GiB cap tracks the server's OWN RSS, not its cgroup's.
        assert!(xml.contains("<memory_worker_use_cgroup>0</memory_worker_use_cgroup>"));
        assert!(xml.contains("<background_schedule_pool_size>4</background_schedule_pool_size>"));
        // The merge pool is lowered ONLY with every MergeTree free-entry
        // threshold below it (else tables refuse to attach).
        assert!(xml.contains("<background_pool_size>4</background_pool_size>"));
        for t in [
            "number_of_free_entries_in_pool_to_execute_mutation",
            "number_of_free_entries_in_pool_to_lower_max_size_of_merge",
            "number_of_free_entries_in_pool_to_execute_optimize_entire_partition",
        ] {
            assert!(
                xml.contains(&format!("<{t}>2</{t}>")),
                "{t} must pair the pool"
            );
        }
        // No system log section at all: in a standalone config even an empty
        // (or `remove="1"`) one would enable that log.
        for tag in xml
            .split('<')
            .filter_map(|t| t.split(['>', ' ', '/']).next())
        {
            assert!(
                !tag.ends_with("_log"),
                "system log section <{tag}> must not be declared"
            );
        }
        // path is present + the server/tmp dirs were created
        assert!(dir.path().join("server").is_dir());
        assert!(dir.path().join("tmp").is_dir());
    }

    #[test]
    fn ensure_default_metadata_synthesizes_from_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("metadata");
        let store = dir
            .path()
            .join("store/125/12529a29-611a-4836-91b1-4d7b8c8863fd");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir_all(&meta).unwrap();
        std::os::unix::fs::symlink(&store, meta.join("default")).unwrap();
        ensure_default_metadata(dir.path()).unwrap();
        let sql = std::fs::read_to_string(meta.join("default.sql")).unwrap();
        assert!(sql.contains("ATTACH DATABASE _ UUID '12529a29-611a-4836-91b1-4d7b8c8863fd'"));
        assert!(sql.contains("ENGINE = Atomic"));
    }

    #[test]
    fn ensure_default_metadata_noop_for_fresh_dir() {
        let dir = tempfile::tempdir().unwrap();
        // no metadata/default symlink → no-op, no file written
        ensure_default_metadata(dir.path()).unwrap();
        assert!(!dir.path().join("metadata/default.sql").exists());
    }

    #[test]
    fn free_port_is_loopback() {
        let p = free_loopback_port().unwrap();
        assert!(p > 0);
    }
}
