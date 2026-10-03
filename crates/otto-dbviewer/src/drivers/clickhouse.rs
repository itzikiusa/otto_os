//! ClickHouse driver — two transports behind one driver.
//!
//! - **HTTP interface** (8123/8443) via `reqwest` + `FORMAT JSONCompact`: the
//!   default, used for the managed/cloud `https://…:8443` case. Raw HTTP fits
//!   arbitrary SQL better than the `clickhouse` crate's compile-time Row API.
//! - **Native TCP protocol** (9000 plain / 9440 TLS) via `klickhouse`: the
//!   wire protocol the official `clickhouse-client` and most managed setups use.
//!   The HTTP interface's TLS handshake times out for some Yandex Managed CH +
//!   SSH-tunnel setups; the native protocol is what their working tools speak,
//!   so we support it directly.
//!
//! Transport is chosen by port (9000/9440 → native; 9440 → native TLS) or an
//! explicit `transport: "native"` param. Both transports normalize to the same
//! [`RawRows`] (column name+CH-type + JSON rows), so the introspection SQL
//! (`system.databases`/`tables`/`columns`, `SHOW CREATE`), row shaping, cell
//! truncation, and completion logic are shared between them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::resource_cache::ResourceCache;
use async_trait::async_trait;
use otto_core::Result;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;

use crate::driver::Driver;
use crate::export::{ExportCounts, ExportFormat, ExportSink};
use crate::split::{split_statements, SqlDialect, StatementSpan};
use crate::tls::TlsFiles;
use crate::types::{
    self, CancelToken, Capabilities, Column, ColumnDef, CompletionContext, CompletionResponse,
    DbQueryPlan, Engine, GraphColumn, GraphTable, NodeKind, NodePath, ObjectDetail, ObjectHit,
    ObjectSearchReq, ObjectSearchResult, QueryHandle, QueryRequest, QueryResult, QueryStats,
    ResolvedConfig, SchemaGraph, SchemaNode, TestResult,
};

/// ClickHouse driver. Caches one transport handle per [`ResolvedConfig::cache_key`]:
///
/// - HTTP: a `reqwest::Client` (cheap to clone; carries its own keep-alive
///   connection pool, so reuse avoids re-establishing TCP/TLS each call).
/// - Native: an `Arc<klickhouse::Client>` (a long-lived multiplexed connection;
///   reusing it across calls avoids re-handshaking the native protocol + TLS).
///
/// Both caches use per-key initialization slots and are Default-constructible, keeping the
/// `#[derive(Default)]` the registry relies on.
#[derive(Default)]
pub struct ClickhouseDriver {
    clients: ResourceCache<reqwest::Client>,
    native: ResourceCache<Arc<klickhouse::Client>>,
    /// Per-connection schema snapshot cache backing smart completion.
    completions: crate::complete::CompletionCache,
    /// Per-cache-key "this server refuses `readonly=2`" memo (DB-02). A user
    /// whose profile is already read-only gets a refusal for touching the
    /// setting; the first refusal flips the flag so every later read-only
    /// request skips the param instead of paying a failed round trip first.
    readonly_refused: std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>,
}

// --- Transport selection ----------------------------------------------------

/// The two wire transports. Selected from the resolved config: native when the
/// port is the native protocol port (9000 plain / 9440 TLS) or the profile
/// asks for it via `params.transport == "native"`; HTTP otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Http,
    Native,
}

fn transport_for(cfg: &ResolvedConfig) -> Transport {
    match cfg.param_str("transport") {
        Some(t) if t.eq_ignore_ascii_case("native") => return Transport::Native,
        Some(t) if t.eq_ignore_ascii_case("http") => return Transport::Http,
        _ => {}
    }
    // Decide by the ORIGINAL port: when tunneled, the service rewrites cfg.port
    // to the ephemeral local forward port and stashes the real port in
    // `__tunnel_port` (a JSON number), so a 9440 connection over a tunnel still
    // selects native.
    let port = cfg
        .params
        .get("__tunnel_port")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as u16)
        .unwrap_or(cfg.port);
    if is_native_port(port) {
        Transport::Native
    } else {
        Transport::Http
    }
}

/// Is this a ClickHouse native-protocol port? The canonical ports are 9000
/// (plain) and 9440 (TLS). We also recognize a forwarded native port — an SSH
/// tunnel's local forward is often the upstream port with a leading digit
/// (e.g. `19000` → `9000`, `19440` → `9440`) — by matching the trailing
/// `9000`/`9440` while excluding the HTTP ports. (An explicit
/// `transport: "native"` param always wins, for any other forward port.)
fn is_native_port(port: u16) -> bool {
    matches!(port, 9000 | 9440) || matches!(port % 10000, 9000 | 9440)
}

/// Hard cap on an interactive HTTP response body (the `run`/introspection path;
/// exports stream uncapped via `post_stream`). Bounds daemon RAM for statements
/// the auto-LIMIT injector can't rewrite; exceeding it is a clear 502, not an OOM.
const HTTP_RESPONSE_BYTE_CAP: usize = 128 * 1024 * 1024;

/// Replies at least this big are JSON-decoded on the blocking pool.
const OFF_RUNTIME_PARSE_BYTES: usize = 1024 * 1024;

/// The streaming run reader ([`Conn::query_json_capped`]) hands complete lines
/// to the row decoder in batches of about this many bytes; a batch this big
/// is decoded on the blocking pool so no runtime worker parses megabytes.
const STREAM_PARSE_CHUNK_BYTES: usize = 256 * 1024;

/// Native transport: once the row cap is reached, drain at most this many
/// further blocks (the injected LIMIT normally ends the stream right away).
/// Past that the stream is dropped and the cached client evicted, so the
/// server-side query dies with its connection instead of being drained whole.
const NATIVE_DRAIN_BLOCKS: usize = 8;

/// Native blocks with at least this many cells are decoded on the blocking pool.
const NATIVE_OFF_RUNTIME_CELLS: usize = 16 * 1024;

/// Server-side `max_execution_time` (seconds) for an HTTP request whose tab
/// sets no timeout — just under the client's default 60s wall clock, so the
/// server stops the query itself instead of running on after the client left.
const DEFAULT_MAX_EXECUTION_SECS: u64 = 55;

/// A transport-agnostic rowset: column (name, CH type) pairs and JSON rows. Both
/// the HTTP `JSONCompact` reply and a decoded native `Block` normalize to this,
/// so all higher-level logic (introspection, run, completion) is shared.
#[derive(Debug, Default)]
struct RawRows {
    meta: Vec<(String, String)>,
    data: Vec<Vec<Value>>,
    bytes_read: u64,
    /// The rows are already cell-capped and charged against the response
    /// [`types::ByteBudget`] (the streaming HTTP reader does this as it goes).
    prepared: bool,
    /// The reader stopped because the byte budget ran out (more rows existed).
    truncated_bytes: bool,
    /// The reader stopped before the end of the reply, so the trailing
    /// statistics (`bytes_read`) are unknown.
    partial: bool,
}

impl RawRows {
    /// Column names only — for code paths that read positional cells.
    fn first_col_strs(&self) -> impl Iterator<Item = &str> {
        self.data
            .iter()
            .filter_map(|row| row.first().and_then(Value::as_str))
    }
}

// --- HTTP plumbing ----------------------------------------------------------

/// Everything needed to issue a request: the built client, the base URL
/// (`http(s)://host:port/`), and the auth headers.
struct Conn {
    client: reqwest::Client,
    base: String,
    headers: HeaderMap,
    /// Session time zone (default UTC), sent as the `session_timezone` setting
    /// so DateTime values render in the user's configured zone.
    timezone: Option<String>,
    /// Active database (if the user selected one), sent as the `database`
    /// request param so unqualified table names resolve against it.
    database: Option<String>,
    /// Server-side `query_id` to tag this request with, so a concurrent
    /// `KILL QUERY WHERE query_id = '<id>'` can target it. `None` for
    /// introspection/completion (no need to cancel those).
    query_id: Option<String>,
    /// Per-statement wall-clock timeout (seconds), sent as the ClickHouse
    /// `max_execution_time` HTTP request setting. `None` = no limit.
    timeout_secs: Option<u64>,
    /// Run in the server's read-only mode (`readonly=2`: reads and setting
    /// changes only). Set for the MCP read-only path via the server-derived
    /// `__read_only_execution` flag — a native barrier behind the statement
    /// classifier. `2`, not `1`, because `1` also forbids the per-request
    /// settings sent here (`session_timezone`, `max_execution_time`).
    readonly: bool,
    /// Shared per-cache-key memo: the server refused `readonly=2` once, so
    /// skip the param (see [`ClickhouseDriver::readonly_refused`]).
    readonly_refused: Arc<AtomicBool>,
    /// Server-side row cap for the run path: sent as `max_result_rows` +
    /// `result_overflow_mode=break` so the server stops producing rows. Only
    /// set for single-SELECT statements the LIMIT injector couldn't rewrite.
    result_row_cap: Option<usize>,
}

/// The shape of a `FORMAT JSONCompact` reply.
#[derive(Debug, Deserialize)]
struct JsonResponse {
    #[serde(default)]
    meta: Vec<MetaCol>,
    #[serde(default)]
    data: Vec<Vec<Value>>,
    #[serde(default)]
    statistics: Statistics,
    /// Set when the query failed AFTER the 200 status went out (a mid-stream
    /// failure) and `http_write_exception_in_output_format` is on: the rows
    /// above are partial, so the reply is an error, never a success.
    #[serde(default)]
    exception: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MetaCol {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(Debug, Default, Deserialize)]
struct Statistics {
    #[serde(default)]
    bytes_read: u64,
}

impl JsonResponse {
    /// Normalize an HTTP `JSONCompact` reply into a transport-agnostic [`RawRows`].
    fn into_raw(self) -> RawRows {
        RawRows {
            meta: self.meta.into_iter().map(|m| (m.name, m.ty)).collect(),
            data: self.data,
            bytes_read: self.statistics.bytes_read,
            ..RawRows::default()
        }
    }
}

impl ClickhouseDriver {
    /// Get (or lazily build) the cached completion snapshot for `(connection, db)`.
    async fn completion_snapshot(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
    ) -> std::sync::Arc<crate::complete::SchemaSnapshot> {
        // Single-flight + negatively cached: a failed build is remembered
        // briefly instead of re-introspecting on every completion request.
        self.completions
            .snapshot_or_build(&cfg.cache_key(), db, || {
                self.build_completion_snapshot(cfg, db)
            })
            .await
    }

    /// Introspect `system.*` into a snapshot: databases, the scoped db's
    /// tables/views, and columns ranked by index membership — a column in the
    /// primary/sorting key (`is_in_primary_key`) ranks `Pk`, one covered by a
    /// data-skipping index ranks `Index` (ClickHouse has no UNIQUE).
    async fn build_completion_snapshot(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
    ) -> Option<crate::complete::SchemaSnapshot> {
        use crate::complete::{FieldSnap, ObjKind, ObjectSnap, Rank, SchemaSnapshot};

        const DB_SQL: &str = "SELECT name FROM system.databases ORDER BY name";
        if db.is_empty() {
            let databases: Vec<String> = self
                .query_rows(cfg, DB_SQL)
                .await
                .ok()?
                .first_col_strs()
                .map(str::to_string)
                .collect();
            return Some(SchemaSnapshot {
                databases,
                objects: Vec::new(),
                ..Default::default()
            });
        }

        let tbl_sql = format!(
            "SELECT name, engine FROM system.tables WHERE database = '{}' ORDER BY name",
            esc(db)
        );
        let col_sql = format!(
            "SELECT table, name, type, is_in_primary_key FROM system.columns \
             WHERE database = '{}' ORDER BY table, position",
            esc(db)
        );
        // Data-skipping indexes: mark any column whose name appears in an index
        // expression as `Index` (best-effort token match; CH index exprs can be
        // arbitrary, but the common case is a bare column).
        let idx_sql = format!(
            "SELECT table, expr FROM system.data_skipping_indices WHERE database = '{}'",
            esc(db)
        );
        // The four catalog reads are independent: one round-trip wave (DB-08).
        let (dbs, tables, cols, skip) = tokio::join!(
            self.query_rows(cfg, DB_SQL),
            self.query_rows(cfg, &tbl_sql),
            self.query_rows(cfg, &col_sql),
            self.query_rows(cfg, &idx_sql),
        );
        let databases: Vec<String> = dbs.ok()?.first_col_strs().map(str::to_string).collect();
        let tables = tables.ok()?;
        let cols = cols.ok()?;
        let skip = skip.unwrap_or_default();
        // (table_lc) → set of indexed column names referenced by skip-index exprs.
        let mut skip_exprs: HashMap<String, Vec<String>> = HashMap::new();
        for row in &skip.data {
            let t = row
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let expr = row.get(1).and_then(Value::as_str).unwrap_or("").to_string();
            skip_exprs.entry(t).or_default().push(expr);
        }

        // Group columns by table, assigning ranks.
        let mut by_table: HashMap<String, Vec<FieldSnap>> = HashMap::new();
        for row in &cols.data {
            let table = row
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = row.get(1).and_then(Value::as_str).unwrap_or("").to_string();
            let ty = row.get(2).and_then(Value::as_str).unwrap_or("").to_string();
            let in_pk = row.get(3).map(cell_truthy).unwrap_or(false);
            let rank = if in_pk {
                Rank::Pk
            } else if skip_exprs
                .get(&table.to_ascii_lowercase())
                .map(|exprs| exprs.iter().any(|e| expr_mentions(e, &name)))
                .unwrap_or(false)
            {
                Rank::Index
            } else {
                Rank::Plain
            };
            by_table
                .entry(table)
                .or_default()
                .push(FieldSnap::new(name, Some(ty), rank));
        }

        let mut objects: Vec<ObjectSnap> = Vec::new();
        for row in &tables.data {
            let name = row
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let engine = row.get(1).and_then(Value::as_str).unwrap_or("");
            let kind = if engine.contains("View") {
                ObjKind::View
            } else {
                ObjKind::Table
            };
            let fields = by_table.remove(&name).unwrap_or_default();
            objects.push(ObjectSnap {
                name,
                kind,
                fields,
                fields_ready: true,
            });
        }

        Some(SchemaSnapshot {
            databases,
            objects,
            ..Default::default()
        })
    }

    /// Get (or lazily build + cache) the `reqwest::Client` for `cfg`, keyed by
    /// [`ResolvedConfig::cache_key`]. Holding the tokio mutex across the
    /// (synchronous) build is fine — it only briefly serializes concurrent
    /// *first* builds for the same key; cache hits return immediately. A
    /// `reqwest::Client` clone is cheap (shares the inner connection pool).
    async fn client(&self, cfg: &ResolvedConfig) -> Result<reqwest::Client> {
        self.clients
            .get_or_try_init(cfg.cache_key(), cfg.lifecycle.as_ref(), |_| true, async {
                build_client(cfg)
            })
            .await
    }

    /// Build a [`Conn`] for one operation: the cached `reqwest::Client` plus the
    /// base URL, auth headers, session timezone, optional active database, an
    /// optional server-side `query_id` (set so `run` can later cancel via
    /// `KILL QUERY WHERE query_id = '<id>'`; `None` for introspection/completion),
    /// and an optional per-statement timeout in seconds (`max_execution_time`).
    /// All derived cheaply from `cfg`.
    #[allow(dead_code)]
    async fn connect_id(
        &self,
        cfg: &ResolvedConfig,
        active_db: Option<&str>,
        query_id: Option<String>,
    ) -> Result<Conn> {
        self.connect_id_timeout(cfg, active_db, query_id, None)
            .await
    }

    async fn connect_id_timeout(
        &self,
        cfg: &ResolvedConfig,
        active_db: Option<&str>,
        query_id: Option<String>,
        timeout_secs: Option<u64>,
    ) -> Result<Conn> {
        let client = self.client(cfg).await?;
        let mut conn =
            Self::connection_from_client(cfg, active_db, query_id, timeout_secs, client)?;
        if conn.readonly {
            conn.readonly_refused = self.readonly_memo(&cfg.cache_key());
        }
        Ok(conn)
    }

    /// The shared "server refuses `readonly`" flag for one cache key.
    fn readonly_memo(&self, cache_key: &str) -> Arc<AtomicBool> {
        let mut map = self
            .readonly_refused
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Arc::clone(map.entry(cache_key.to_string()).or_default())
    }

    fn connection_from_client(
        cfg: &ResolvedConfig,
        active_db: Option<&str>,
        query_id: Option<String>,
        timeout_secs: Option<u64>,
        client: reqwest::Client,
    ) -> Result<Conn> {
        let scheme = if cfg.tls.enabled() { "https" } else { "http" };
        // Through an SSH tunnel the service rewrites host→127.0.0.1; use the
        // ORIGINAL hostname in the URL so the TLS SNI + Host header are the real
        // host (managed ClickHouse routes by SNI). `build_client` maps that host
        // back to the local tunnel port via reqwest `.resolve`.
        let url_host = cfg
            .param_str("__tunnel_host")
            .unwrap_or_else(|| cfg.host.clone());
        let base = format!("{scheme}://{url_host}:{}/", cfg.port);

        let user = cfg.user.clone().unwrap_or_else(|| "default".to_string());
        let key = cfg.password.clone().unwrap_or_default();
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-ClickHouse-User",
            HeaderValue::from_str(&user).map_err(types::upstream)?,
        );
        headers.insert(
            "X-ClickHouse-Key",
            HeaderValue::from_str(&key).map_err(types::upstream)?,
        );

        // Default to UTC when unset (user request: "by default, UTC").
        let timezone = Some(
            cfg.param_str("timezone")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "UTC".to_string()),
        );

        Ok(Conn {
            client,
            base,
            headers,
            timezone,
            database: active_db.map(str::to_string),
            query_id,
            timeout_secs,
            readonly: cfg
                .params
                .get("__read_only_execution")
                .and_then(serde_json::Value::as_bool)
                == Some(true),
            readonly_refused: Arc::new(AtomicBool::new(false)),
            result_row_cap: None,
        })
    }

    // --- Native (klickhouse) plumbing ---------------------------------------

    /// Get (or lazily open + cache) the native `klickhouse::Client` for `cfg`,
    /// keyed by [`ResolvedConfig::cache_key`]. A `klickhouse::Client` is a
    /// long-lived multiplexed connection, so we keep it behind an `Arc` and
    /// reuse it. A closed (dropped/broken) connection is transparently
    /// reopened: if the cached client reports closed, we discard and rebuild.
    async fn native_client(&self, cfg: &ResolvedConfig) -> Result<Arc<klickhouse::Client>> {
        self.native
            .get_or_try_init(
                cfg.cache_key(),
                cfg.lifecycle.as_ref(),
                |client| !client.is_closed(),
                async { Ok(Arc::new(native_connect(cfg).await?)) },
            )
            .await
    }

    /// Run one SQL statement over the native protocol and normalize the decoded
    /// blocks into a transport-agnostic [`RawRows`]. This is the native twin of
    /// [`Conn::query_json`]: connect (cached, TLS per cfg), stream result
    /// `Block`s, and decode each column's `Value`s to JSON dynamically.
    /// Uncapped — introspection/export callers; the interactive path goes
    /// through [`Self::native_query_capped`].
    async fn native_query(&self, cfg: &ResolvedConfig, sql: &str) -> Result<RawRows> {
        self.native_query_capped(cfg, sql, None).await
    }

    /// Like [`Self::native_query`] but stops collecting once `row_cap` rows are
    /// held (further blocks are drained undecoded), bounding daemon RAM for
    /// interactive statements the auto-LIMIT injector couldn't rewrite. Callers
    /// pass their page size + 1 so truncation detection still works.
    async fn native_query_capped(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        row_cap: Option<usize>,
    ) -> Result<RawRows> {
        use futures_util::StreamExt;

        let client = self.native_client(cfg).await?;
        let mut stream = client.query_raw(sql).await.map_err(native_err)?;

        let mut meta: Vec<(String, String)> = Vec::new();
        let mut data: Vec<Vec<Value>> = Vec::new();
        let mut drained = 0usize;
        while let Some(block) = stream.next().await {
            let block = block.map_err(native_err)?;
            // The first non-empty block establishes the column order/types; the
            // header block (rows == 0) still carries column_types, so we capture
            // metadata from whichever block first exposes columns.
            if meta.is_empty() && !block.column_types.is_empty() {
                meta = block
                    .column_types
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.to_string()))
                    .collect();
            }
            if block.rows == 0 {
                continue;
            }
            // Past the cap: drain a few blocks undecoded (the injected LIMIT
            // normally ends the stream here, keeping the cached connection
            // reusable). A statement still streaming after that is abandoned:
            // drop the stream (klickhouse discards the rest) and evict the
            // cached client, so later queries don't queue behind it and the
            // server query dies with the connection once in-flight users finish.
            if row_cap.is_some_and(|cap| data.len() >= cap) {
                drained += 1;
                if drained > NATIVE_DRAIN_BLOCKS {
                    drop(stream);
                    drop(self.native.remove(&cfg.cache_key()));
                    break;
                }
                continue;
            }
            let take = row_cap.map_or(usize::MAX, |cap| cap - data.len());
            let cells = (block.rows as usize).saturating_mul(block.column_types.len());
            let rows = if cells >= NATIVE_OFF_RUNTIME_CELLS {
                tokio::task::spawn_blocking(move || decode_native_block(&block, take))
                    .await
                    .map_err(|e| types::upstream(format!("clickhouse: decode task failed: {e}")))?
            } else {
                decode_native_block(&block, take)
            };
            data.extend(rows);
        }

        Ok(RawRows {
            meta,
            data,
            ..RawRows::default()
        })
    }

    /// Run a row-returning statement over whichever transport `cfg` selects.
    /// Introspection/completion callers use this no-scope wrapper (they qualify
    /// their own table names); `run` uses [`Self::query_rows_db`] to scope to the
    /// active database.
    async fn query_rows(&self, cfg: &ResolvedConfig, sql: &str) -> Result<RawRows> {
        self.query_rows_db(cfg, sql, None, None).await
    }

    /// Like [`Self::query_rows`] but scopes unqualified table names to
    /// `active_db` (when `Some`) and, when `query_id` is set, tags the HTTP
    /// request so a concurrent cancel can `KILL QUERY` it. On the HTTP transport
    /// this sets the `database` / `query_id` / `max_execution_time` request
    /// params. NATIVE TRANSPORT TODO: the native client's default database is
    /// fixed at connect time (cached per config), so active-db scoping is not
    /// applied there yet — unqualified names resolve against the profile's
    /// database. The native transport also doesn't expose a per-query `query_id`
    /// or per-statement timeout here, so native queries aren't server-cancellable
    /// or timeout-bounded (cancel / timeout become no-ops there). Most connections
    /// use HTTP.
    async fn query_rows_db(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        query_id: Option<String>,
    ) -> Result<RawRows> {
        self.query_rows_db_timeout(cfg, sql, active_db, query_id, None)
            .await
    }

    async fn query_rows_db_timeout(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        query_id: Option<String>,
        timeout_secs: Option<u64>,
    ) -> Result<RawRows> {
        self.query_rows_db_capped(cfg, sql, active_db, query_id, timeout_secs, None, false)
            .await
    }

    /// Full-fat variant: `row_cap` bounds how many rows either transport
    /// materialises. Over HTTP a capped read streams the reply row by row and
    /// stops at the cap or the response byte budget (killing the server query);
    /// `server_cap` additionally asks the server to stop producing rows
    /// (`max_result_rows` + `result_overflow_mode=break`). The native transport
    /// has no server-side `max_execution_time` or cancellable `query_id`, so
    /// `timeout_secs` is enforced there as a client-side wall clock.
    #[allow(clippy::too_many_arguments)]
    async fn query_rows_db_capped(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        query_id: Option<String>,
        timeout_secs: Option<u64>,
        row_cap: Option<usize>,
        server_cap: bool,
    ) -> Result<RawRows> {
        match transport_for(cfg) {
            Transport::Http => {
                let mut conn = self
                    .connect_id_timeout(cfg, active_db, query_id, timeout_secs)
                    .await?;
                match row_cap {
                    Some(cap) => {
                        if server_cap {
                            conn.result_row_cap = Some(cap);
                        }
                        conn.query_json_capped(sql, cap).await
                    }
                    None => Ok(conn.query_json(sql).await?.into_raw()),
                }
            }
            Transport::Native => {
                native_with_timeout(timeout_secs, self.native_query_capped(cfg, sql, row_cap)).await
            }
        }
    }

    /// Run a statement whose reply we want as raw text (DDL / `SHOW CREATE`).
    /// Over native there is no "raw text" format — we run it as a normal query
    /// and join the single string column the server returns. No-scope wrapper;
    /// `run` uses [`Self::query_text_db`].
    async fn query_text(&self, cfg: &ResolvedConfig, sql: &str) -> Result<String> {
        self.query_text_db(cfg, sql, None, None).await
    }

    /// Like [`Self::query_text`] but scopes to `active_db` and tags the HTTP
    /// request with `query_id` (see [`Self::query_rows_db`] for the
    /// native-transport TODO).
    async fn query_text_db(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        query_id: Option<String>,
    ) -> Result<String> {
        self.query_text_db_timeout(cfg, sql, active_db, query_id, None)
            .await
    }

    async fn query_text_db_timeout(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        query_id: Option<String>,
        timeout_secs: Option<u64>,
    ) -> Result<String> {
        match transport_for(cfg) {
            Transport::Http => {
                let conn = self
                    .connect_id_timeout(cfg, active_db, query_id, timeout_secs)
                    .await?;
                conn.query_raw(sql).await
            }
            Transport::Native => {
                let raw = native_with_timeout(timeout_secs, self.native_query(cfg, sql)).await?;
                // SHOW CREATE / single-value replies come back as one row, one
                // string cell; stringify whatever the first cell is.
                let text = raw
                    .data
                    .first()
                    .and_then(|row| row.first())
                    .map(json_cell_to_text)
                    .unwrap_or_default();
                Ok(text)
            }
        }
    }

    /// Run one row-returning statement and shape it into a `QueryResult`,
    /// capping at `max_rows` (flagging `truncated`) and capping oversized cells.
    /// Shared by the single-statement fast path and the batch loop.
    #[allow(clippy::too_many_arguments)]
    async fn exec_ch_read(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        query_id: Option<String>,
        timeout_secs: Option<u64>,
        max_rows: usize,
        limited: bool,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        // Materialise at most max_rows+1 rows (the +1 flags truncation) on
        // both transports. When the LIMIT injector couldn't bound the statement
        // (FORMAT/SETTINGS/…), also ask the server to stop at the cap — only
        // for a single SELECT: `max_result_rows` is checked for subqueries
        // too, and `break` would silently clip an `IN (SELECT …)` / UNION arm.
        let server_cap = !limited && single_select(sql);
        let resp = self
            .query_rows_db_capped(
                cfg,
                sql,
                active_db,
                query_id,
                timeout_secs,
                Some(max_rows.saturating_add(1)),
                server_cap,
            )
            .await?;
        let duration_ms = started.elapsed().as_millis() as u64;

        let columns: Vec<Column> = resp
            .meta
            .iter()
            .map(|(name, ty)| Column::typed(name, ty))
            .collect();

        let total = resp.data.len();
        let mut truncated = total > max_rows;
        // Cap oversized cells (e.g. AggregateFunction/*State blobs) so a giant
        // value can't break the grid, and stop at the response byte budget like
        // the MySQL/Postgres readers (at least one row is always kept).
        let mut budget = types::ByteBudget::default();
        let mut truncated_reason = None;
        let bytes_read = (!resp.partial).then_some(resp.bytes_read);
        let rows: Vec<Vec<Value>> = if resp.prepared {
            // The streaming reader already capped cells and charged the budget.
            if resp.truncated_bytes {
                truncated = true;
                truncated_reason = Some(types::TruncatedReason::Bytes);
            }
            let mut data = resp.data;
            data.truncate(max_rows);
            data
        } else {
            let mut rows = Vec::with_capacity(total.min(max_rows));
            for row in resp.data.into_iter().take(max_rows) {
                let row: Vec<Value> = row.into_iter().map(cap_cell).collect();
                let size = row.iter().map(|v| types::approx_json_len(v) + 1).sum();
                if !budget.charge(size) && !rows.is_empty() {
                    truncated = true;
                    truncated_reason = Some(types::TruncatedReason::Bytes);
                    break;
                }
                rows.push(row);
            }
            rows
        };
        let row_count = rows.len();

        Ok(QueryResult {
            columns,
            rows,
            stats: QueryStats {
                duration_ms,
                row_count,
                bytes_read,
            },
            truncated,
            truncated_reason,
            ..QueryResult::empty()
        })
    }

    /// Run one write/DDL statement (no rowset) and return the "OK" acknowledgement.
    async fn exec_ch_write(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        query_id: Option<String>,
        timeout_secs: Option<u64>,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        self.query_text_db_timeout(cfg, sql, active_db, query_id, timeout_secs)
            .await?;
        let mut result = QueryResult::message("OK");
        result.rows_affected = None;
        result.stats = QueryStats {
            duration_ms: started.elapsed().as_millis() as u64,
            row_count: 0,
            bytes_read: None,
        };
        Ok(result)
    }

    /// Execute a true multi-statement batch (>1 statement) in order. Each
    /// statement gets its own fresh `query_id` (recorded in `token` on the HTTP
    /// transport) so a concurrent cancel targets the statement currently running.
    /// The first result is the top-level one, the rest go into `more_results`,
    /// each labelled with its statement preview; on the first failure execution
    /// stops with an `errored` entry and the completed results are returned (§2.2).
    /// Batches get no auto-LIMIT/OFFSET injection (the pager is single-statement
    /// only), but each read is still capped at `max_rows`.
    async fn run_ch_batch(
        &self,
        cfg: &ResolvedConfig,
        spans: &[StatementSpan],
        max_rows: usize,
        active_db: Option<&str>,
        timeout_secs: Option<u64>,
        token: &CancelToken,
    ) -> Result<QueryResult> {
        // Server-side cancel via query_id only exists on the HTTP transport.
        let http = transport_for(cfg) == Transport::Http;
        let mut results: Vec<QueryResult> = Vec::with_capacity(spans.len());
        for span in spans {
            let stmt = span.text.as_str();
            let query_id = new_query_id();
            if http {
                token.set(QueryHandle::ClickhouseQueryId(query_id.clone()));
            }
            let outcome = if returns_rows(stmt) {
                self.exec_ch_read(
                    cfg,
                    stmt,
                    active_db,
                    Some(query_id),
                    timeout_secs,
                    max_rows,
                    false,
                )
                .await
            } else {
                self.exec_ch_write(cfg, stmt, active_db, Some(query_id), timeout_secs)
                    .await
            };
            match outcome {
                Ok(mut r) => {
                    r.statement = Some(types::statement_preview(stmt));
                    results.push(r);
                }
                Err(e) => {
                    results.push(types::errored_batch_entry(
                        types::statement_preview(stmt),
                        e.to_string(),
                    ));
                    break;
                }
            }
        }
        Ok(types::fold_batch_results(results))
    }
}

/// Build a fresh `reqwest::Client` from the resolved config, materializing any
/// inline CA for custom-CA TLS. Never called directly by the driver methods —
/// they go through [`ClickhouseDriver::client`] for caching.
fn build_client(cfg: &ResolvedConfig) -> Result<reqwest::Client> {
    // Bound connection establishment (TCP+TLS) so a misconfigured endpoint (e.g.
    // HTTPS against a plain-HTTP port, or the native 9000/9440 port) fails fast
    // with a clear error instead of hanging forever. The per-request wall-clock
    // timeout is set on each `post`/`post_stream` call instead of a blanket
    // client-wide cap: the old 60s ceiling truncated long queries the user
    // explicitly allowed via `timeout_ms`, and killed >60s streaming exports (§2.4).
    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(10));
    // Tunnel case: the URL host is the real hostname (for SNI/cert), but the TCP
    // must go to the local forward — map it there. (reqwest uses the URL port
    // with the overridden IP, so the forward's local port is honoured.)
    if let Some(tunnel_host) = cfg.param_str("__tunnel_host") {
        let local = std::net::SocketAddr::from(([127, 0, 0, 1], cfg.port));
        builder = builder.resolve(&tunnel_host, local);
    }
    if cfg.tls.enabled() {
        let files = TlsFiles::materialize(&cfg.tls)?;
        if let Some(ca_path) = files.ca {
            let ca_bytes = std::fs::read(&ca_path).map_err(types::upstream)?;
            let cert = reqwest::Certificate::from_pem(&ca_bytes).map_err(types::upstream)?;
            builder = builder.add_root_certificate(cert);
        }
        if !cfg.tls.verify {
            builder = builder.danger_accept_invalid_certs(true);
        }
    }
    builder.build().map_err(types::upstream)
}

impl Conn {
    /// Run a statement that returns rows, asking for `FORMAT JSONCompact`, and
    /// parse the reply.
    async fn query_json(&self, sql: &str) -> Result<JsonResponse> {
        let body = format!("{sql}\nFORMAT JSONCompact");
        let text = self.post(body).await?;
        // A big reply (up to HTTP_RESPONSE_BYTE_CAP) is hundreds of ms of JSON
        // parsing — do that on the blocking pool, not a runtime worker every
        // other request shares. Small replies (introspection) stay inline.
        let parsed = if text.len() < OFF_RUNTIME_PARSE_BYTES {
            decode_json_reply(&text)
        } else {
            tokio::task::spawn_blocking(move || decode_json_reply(&text))
                .await
                .map_err(|e| types::upstream(format!("clickhouse: decode task failed: {e}")))?
        };
        parsed.map_err(otto_core::Error::Upstream)
    }

    /// Run a statement and return the raw response text (for DDL / SHOW CREATE
    /// / write statements with no JSON envelope).
    async fn query_raw(&self, sql: &str) -> Result<String> {
        self.post(sql.to_string()).await
    }

    /// POST a body to the HTTP interface and read the reply under the hard
    /// [`HTTP_RESPONSE_BYTE_CAP`]; non-2xx replies map to a 502 carrying the
    /// server's error text.
    async fn post(&self, body: String) -> Result<String> {
        let resp = self.send_checked(body).await?;
        self.read_capped(resp).await
    }

    /// Send a body, honouring the read-only mode. A read-only connection asks
    /// for `readonly=2`; a user whose server profile is ALREADY read-only may
    /// not touch that setting at all, and the server enforcing read-only is
    /// exactly the guarantee wanted — so that refusal is retried without it,
    /// and remembered per cache key (DB-02) so later requests skip the doomed
    /// first attempt (and the body copy it needs).
    async fn send_checked(&self, body: String) -> Result<reqwest::Response> {
        if !self.readonly || self.readonly_refused.load(Ordering::Relaxed) {
            return self.send(body, false).await;
        }
        match self.send(body.clone(), true).await {
            Err(e) if is_readonly_setting_refused(&e.to_string()) => {
                self.readonly_refused.store(true, Ordering::Relaxed);
                self.send(body, false).await
            }
            other => other,
        }
    }

    /// One POST with every request setting attached; returns the response
    /// once its status is a success (an error status reads the small body
    /// and maps it to the server's message).
    async fn send(&self, body: String, readonly: bool) -> Result<reqwest::Response> {
        let mut req = compressed(self.client.post(&self.base).headers(self.headers.clone()));
        if readonly {
            req = req.query(&[("readonly", "2")]);
        }
        if let Some(tz) = &self.timezone {
            // ClickHouse 23.4+ honours session_timezone as a request setting.
            req = req.query(&[("session_timezone", tz.as_str())]);
        }
        if let Some(db) = &self.database {
            // The `database` request param sets the default DB for unqualified
            // table names (the HTTP analogue of `USE <db>`).
            req = req.query(&[("database", db.as_str())]);
        }
        if let Some(qid) = &self.query_id {
            // Tag the server-side query so `KILL QUERY WHERE query_id = '<id>'`
            // can target it from another connection.
            req = req.query(&[("query_id", qid.as_str())]);
        }
        if let Some(cap) = self.result_row_cap {
            // Server-side row cap: the server stops producing rows at (about —
            // block granularity) `cap` instead of streaming the whole result.
            req = req.query(&[
                ("max_result_rows", cap.to_string().as_str()),
                ("result_overflow_mode", "break"),
            ]);
        }
        // `max_execution_time` is a ClickHouse per-query setting (seconds,
        // integer; 0 = unlimited — the guard in `run_tracked` ensures only
        // positive tab timeouts reach here). ALWAYS sent: without a tab
        // timeout the client still gives up at its default wall clock, and a
        // query with no server-side bound kept burning the cluster after the
        // UI had already reported the timeout (retries piled up more).
        let server_secs = self.timeout_secs.unwrap_or(DEFAULT_MAX_EXECUTION_SECS);
        req = req.query(&[("max_execution_time", server_secs.to_string().as_str())]);
        // Per-request client-side wall clock (replaces the old blanket 60s client
        // timeout, §2.4): the server bound plus a few seconds of grace so
        // ClickHouse's own `max_execution_time` fires first (cleaner server
        // error).
        req = req.timeout(Duration::from_secs(server_secs.saturating_add(5)));
        let resp = match req.body(body).send().await {
            Ok(resp) => resp,
            Err(e) => {
                self.kill_after_client_timeout(&e).await;
                return Err(req_err(e));
            }
        };
        if !resp.status().is_success() {
            let text = self.read_capped(resp).await?;
            return Err(otto_core::Error::Upstream(crate::errors::clean_ch_message(
                text.trim(),
            )));
        }
        Ok(resp)
    }

    /// Read a body with a hard byte cap instead of `text()`: statements the
    /// auto-LIMIT injector bails on (FORMAT/SETTINGS/UNION, batches) can
    /// return an unbounded body that would otherwise be buffered whole into
    /// daemon RAM. Exceeding the cap is a clear error, not an OOM.
    async fn read_capped(&self, resp: reqwest::Response) -> Result<String> {
        use futures_util::StreamExt as _;
        let mut buf: Vec<u8> = Vec::new();
        let mut body = BodyDecoder::for_response(&resp)?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => {
                    self.kill_after_client_timeout(&e).await;
                    return Err(req_err(e));
                }
            };
            let chunk = body.decode(&chunk)?;
            if buf.len() + chunk.len() > HTTP_RESPONSE_BYTE_CAP {
                return Err(response_cap_error());
            }
            buf.extend_from_slice(&chunk);
        }
        // Valid UTF-8 (the normal case) moves the buffer; only a broken body
        // pays the lossy copy.
        Ok(String::from_utf8(buf)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
    }

    /// The run path's reader (DB-01): `FORMAT JSONCompact` parsed row by row
    /// straight off the response stream. Cells are capped and charged against
    /// the response [`types::ByteBudget`] as they arrive; reading stops at
    /// `row_cap` rows or when the budget runs out, the response is dropped and
    /// the server query killed by `query_id` — so a statement the LIMIT
    /// injector couldn't bound costs `row_cap` rows of RAM, not 128 MiB.
    async fn query_json_capped(&self, sql: &str, row_cap: usize) -> Result<RawRows> {
        use futures_util::StreamExt as _;
        let resp = self
            .send_checked(format!("{sql}\nFORMAT JSONCompact"))
            .await?;
        let mut state = CompactStream::new(Some(row_cap));
        // `carry` = bytes after the last newline seen; `pending` = complete
        // lines not yet decoded.
        let mut carry: Vec<u8> = Vec::new();
        let mut pending: Vec<u8> = Vec::new();
        let mut body = BodyDecoder::for_response(&resp)?;
        let mut stream = resp.bytes_stream();
        let mut exhausted = true;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => {
                    self.kill_after_client_timeout(&e).await;
                    return Err(req_err(e));
                }
            };
            let chunk = match body.decode(&chunk) {
                Ok(chunk) => chunk,
                Err(e) => {
                    self.spawn_kill();
                    return Err(e);
                }
            };
            match chunk.iter().rposition(|&b| b == b'\n') {
                Some(i) => {
                    pending.append(&mut carry);
                    pending.extend_from_slice(&chunk[..=i]);
                    carry.extend_from_slice(&chunk[i + 1..]);
                }
                None => carry.extend_from_slice(&chunk),
            }
            if carry.len() > HTTP_RESPONSE_BYTE_CAP || state.buffered() > HTTP_RESPONSE_BYTE_CAP {
                self.spawn_kill();
                return Err(response_cap_error());
            }
            if pending.len() >= STREAM_PARSE_CHUNK_BYTES {
                state = feed_compact(state, std::mem::take(&mut pending)).await?;
                if state.stopped {
                    exhausted = false;
                    break;
                }
            }
        }
        drop(stream);
        if exhausted {
            if !carry.is_empty() {
                pending.append(&mut carry);
                pending.push(b'\n');
            }
            if !pending.is_empty() {
                state = feed_compact(state, pending).await?;
            }
        } else {
            // We hung up mid-reply; make sure the server stops too.
            self.spawn_kill();
        }
        state.finish().map_err(otto_core::Error::Upstream)
    }

    /// `KILL QUERY` for this request's `query_id`, as a ready-to-send request.
    fn kill_request(&self) -> Option<reqwest::RequestBuilder> {
        let qid = self.query_id.as_ref()?;
        let sql = format!("KILL QUERY WHERE query_id = '{}'", esc(qid));
        Some(
            self.client
                .post(&self.base)
                .headers(self.headers.clone())
                .timeout(Duration::from_secs(10))
                .body(sql),
        )
    }

    /// Fire-and-forget `KILL QUERY` (the result is already in hand; don't make
    /// the caller wait on the kill round trip).
    fn spawn_kill(&self) {
        if let Some(req) = self.kill_request() {
            tokio::spawn(async move {
                let _ = req.send().await;
            });
        }
    }

    /// The client gave up on a tagged query (its wall clock expired, while
    /// sending or while reading the reply) but the server may still be running
    /// it: kill it by `query_id`, best-effort, on a fresh request — the same
    /// statement an explicit Stop sends. Any other error, or an untagged
    /// (introspection) request, is left alone.
    async fn kill_after_client_timeout(&self, e: &reqwest::Error) {
        if !e.is_timeout() {
            return;
        }
        if let Some(req) = self.kill_request() {
            let _ = req.send().await;
        }
    }

    /// POST a body and return the streaming `reqwest::Response` (for
    /// `bytes_stream()`), WITHOUT buffering it — the large-batch export path. On a
    /// non-2xx the error body is small, so we read it fully to surface the
    /// server's message; on success the body is left unread for the caller to
    /// stream chunk-by-chunk to disk.
    async fn post_stream(&self, body: String) -> Result<reqwest::Response> {
        let mut req = compressed(self.client.post(&self.base).headers(self.headers.clone()));
        if let Some(tz) = &self.timezone {
            req = req.query(&[("session_timezone", tz.as_str())]);
        }
        if let Some(db) = &self.database {
            req = req.query(&[("database", db.as_str())]);
        }
        if let Some(qid) = &self.query_id {
            req = req.query(&[("query_id", qid.as_str())]);
        }
        // The export stream owns its own progress/cancel semantics; an effectively
        // unlimited (24h) per-request timeout keeps the old blanket 60s client cap
        // from truncating a long export mid-stream (§2.4).
        req = req.timeout(Duration::from_secs(24 * 60 * 60));
        let resp = req.body(body).send().await.map_err(req_err)?;
        if !resp.status().is_success() {
            // Capped AND decoded (the error body may be compressed too).
            let text = self.read_capped(resp).await?;
            return Err(otto_core::Error::Upstream(crate::errors::clean_ch_message(
                text.trim(),
            )));
        }
        Ok(resp)
    }
}

// --- Native connection / TLS ------------------------------------------------

/// Open a native-protocol connection to the resolved endpoint. Native TLS (port
/// 9440 or `cfg.tls` enabled) builds a rustls connector honouring `cfg.tls`
/// (custom inline CA, and `verify=false` → accept any cert); the SNI is set to
/// the real hostname (`__tunnel_host` when tunnelled, else `cfg.host`).
async fn native_connect(cfg: &ResolvedConfig) -> Result<klickhouse::Client> {
    let options = klickhouse::ClientOptions {
        username: cfg.user.clone().unwrap_or_else(|| "default".to_string()),
        password: cfg.password.clone().unwrap_or_default(),
        default_database: cfg.database.clone().unwrap_or_default(),
        tcp_nodelay: true,
    };
    // 9440 (or a forwarded *9440) is the conventional native-TLS port; also
    // honour an explicit TLS mode in the profile.
    let use_tls = cfg.tls.enabled() || cfg.port % 10000 == 9440;
    // The TCP endpoint is always the resolved (post-tunnel) host:port — for a
    // tunnel that's 127.0.0.1:<local-forward>; otherwise the real host.
    let endpoint = format!("{}:{}", cfg.host, cfg.port);

    if !use_tls {
        return klickhouse::Client::connect(&endpoint, options)
            .await
            .map_err(native_err);
    }

    // Native TLS: SNI = the real hostname. Through an SSH tunnel the TCP target
    // is 127.0.0.1:<local>, but the certificate is issued for the real host, so
    // we set the server name from `__tunnel_host` when present (managed CH also
    // routes by SNI). LIMITATION: if the tunnel host is unknown we fall back to
    // `cfg.host` (127.0.0.1 for a tunnel), which won't match a real cert —
    // such setups must either provide `__tunnel_host`, set `tls.server_name`,
    // or disable verification (`tls.verify = false`).
    let connector = build_native_tls(cfg)?;
    let sni = cfg
        .tls
        .server_name
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.param_str("__tunnel_host"))
        .unwrap_or_else(|| cfg.host.clone());
    let server_name = rustls_pki_types::ServerName::try_from(sni)
        .map_err(|e| types::invalid(format!("clickhouse: invalid TLS server name: {e}")))?;

    klickhouse::Client::connect_tls(&endpoint, options, server_name, &connector)
        .await
        .map_err(native_err)
}

/// Build a `tokio_rustls::TlsConnector` for the native transport from `cfg.tls`:
/// honour an inline CA (added to the root store), and when `verify=false`
/// install a no-op certificate verifier (signatures are still checked, the
/// chain/hostname are not) so self-signed / mismatched certs are accepted.
fn build_native_tls(cfg: &ResolvedConfig) -> Result<tokio_rustls::TlsConnector> {
    use rustls::pki_types::pem::PemObject;

    let mut roots = rustls::RootCertStore::empty();
    // Start from the bundled webpki trust anchors (system-equivalent roots).
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // Add any inline CA PEM (private CA / managed-CH bundle).
    if let Some(ca) = cfg.tls.ca_cert.as_deref().filter(|s| !s.is_empty()) {
        for cert in rustls::pki_types::CertificateDer::pem_slice_iter(ca.as_bytes()) {
            let cert = cert.map_err(|e| types::invalid(format!("clickhouse: bad CA PEM: {e}")))?;
            roots
                .add(cert)
                .map_err(|e| types::invalid(format!("clickhouse: bad CA cert: {e}")))?;
        }
    }

    let config = if cfg.tls.verify {
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth()
    } else {
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier::new()))
            .with_no_client_auth()
    };

    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

/// A `ServerCertVerifier` that accepts any server certificate (used only when
/// `cfg.tls.verify == false`, the native equivalent of reqwest's
/// `danger_accept_invalid_certs`). Signature verification is still delegated to
/// the crypto provider so the handshake stays well-formed; only chain validity
/// and hostname matching are skipped.
#[derive(Debug)]
struct NoVerifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl NoVerifier {
    fn new() -> Self {
        Self {
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Decode a `FORMAT JSONCompact` reply, turning a mid-stream failure into the
/// server's real error (E7): the in-band `"exception"` field (rows above it are
/// partial — never report them as success), or, when the body is broken JSON,
/// the raw `Code: N. DB::Exception: …` the server appended to it — instead of
/// serde's `trailing characters at line 1 column 98213`.
fn decode_json_reply(text: &str) -> std::result::Result<JsonResponse, String> {
    match serde_json::from_str::<JsonResponse>(text) {
        Ok(reply) => match &reply.exception {
            Some(ex) => Err(crate::errors::ch_midstream_message(
                ex,
                Some(reply.data.len()),
            )),
            None => Ok(reply),
        },
        Err(e) => match crate::errors::ch_trailing_exception(text) {
            Some(ex) => Err(crate::errors::ch_midstream_message(&ex, None)),
            None => Err(e.to_string()),
        },
    }
}

fn response_cap_error() -> otto_core::Error {
    types::upstream(format!(
        "clickhouse: response larger than the {} MiB interactive cap — \
         narrow the query, add a LIMIT, or use export",
        HTTP_RESPONSE_BYTE_CAP / (1024 * 1024)
    ))
}

/// Where the streaming `JSONCompact` reader is in the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamPhase {
    /// Before `"data":` — buffering the `meta` header.
    Head,
    /// Inside the `data` array: one row per line.
    Data,
    /// After the `data` array: `rows`, `statistics`, maybe `exception`.
    Tail,
    /// The reply isn't laid out one row per line (e.g. a server with JSON
    /// pretty-printing off): buffer it whole and decode it at the end.
    Whole,
}

/// Incremental decoder for a `FORMAT JSONCompact` reply (DB-01). ClickHouse
/// writes that format one data row per line, so rows can be decoded as they
/// arrive and reading can stop at the row cap / byte budget instead of
/// buffering the full body. Anything not laid out that way falls back to the
/// whole-body decoder ([`decode_json_reply`]).
struct CompactStream {
    phase: StreamPhase,
    /// Header text (Head), trailer text (Tail) or the whole body (Whole).
    text: String,
    meta: Vec<(String, String)>,
    rows: Vec<Vec<Value>>,
    row_cap: Option<usize>,
    budget: types::ByteBudget,
    truncated_bytes: bool,
    /// Enough rows are held: further data rows are skipped undecoded (the
    /// network loop hangs up at the next batch boundary; a reply already fully
    /// received is still walked to its trailer for `statistics`).
    stopped: bool,
}

impl CompactStream {
    fn new(row_cap: Option<usize>) -> Self {
        Self {
            phase: StreamPhase::Head,
            text: String::new(),
            meta: Vec::new(),
            rows: Vec::new(),
            row_cap,
            budget: types::ByteBudget::default(),
            truncated_bytes: false,
            stopped: false,
        }
    }

    /// Bytes of reply text held undecoded (bounded by the caller).
    fn buffered(&self) -> usize {
        self.text.len()
    }

    /// Feed one complete line (no trailing newline).
    fn push_line(&mut self, line: &str) -> std::result::Result<(), String> {
        match self.phase {
            StreamPhase::Head => {
                self.text.push_str(line);
                self.text.push('\n');
                if line.trim() == "\"data\":" {
                    // `{ "meta": [...], "data":` + `[]}` is a complete object.
                    let probe = format!("{}[]}}", self.text);
                    match serde_json::from_str::<JsonResponse>(&probe) {
                        Ok(head) => {
                            self.meta = head.meta.into_iter().map(|m| (m.name, m.ty)).collect();
                            self.text.clear();
                            self.phase = StreamPhase::Data;
                        }
                        Err(_) => self.phase = StreamPhase::Whole,
                    }
                } else if self.text.len() > OFF_RUNTIME_PARSE_BYTES {
                    // A metadata header is never this big: not the layout we know.
                    self.phase = StreamPhase::Whole;
                }
            }
            StreamPhase::Data => {
                let t = line.trim();
                if t.is_empty() || t == "[" {
                    return Ok(());
                }
                if t == "]" || t == "]," {
                    self.phase = StreamPhase::Tail;
                    return Ok(());
                }
                if self.stopped {
                    return Ok(());
                }
                let cells = t.strip_suffix(',').unwrap_or(t);
                match serde_json::from_str::<Vec<Value>>(cells) {
                    Ok(row) => self.accept_row(row),
                    // A mid-stream failure with in-band exceptions off: the
                    // server appended its raw `Code: N. DB::Exception: …` text.
                    Err(e) => {
                        return Err(match crate::errors::ch_trailing_exception(line) {
                            Some(ex) => {
                                crate::errors::ch_midstream_message(&ex, Some(self.rows.len()))
                            }
                            None => format!("clickhouse: undecodable result row: {e}"),
                        })
                    }
                }
            }
            StreamPhase::Tail | StreamPhase::Whole => {
                self.text.push_str(line);
                self.text.push('\n');
            }
        }
        Ok(())
    }

    fn accept_row(&mut self, row: Vec<Value>) {
        let row: Vec<Value> = row.into_iter().map(cap_cell).collect();
        let size = row.iter().map(|v| types::approx_json_len(v) + 1).sum();
        // At least one row is always kept, like the MySQL/Postgres readers.
        if !self.budget.charge(size) && !self.rows.is_empty() {
            self.truncated_bytes = true;
            self.stopped = true;
            return;
        }
        self.rows.push(row);
        if self.row_cap.is_some_and(|cap| self.rows.len() >= cap) {
            self.stopped = true;
        }
    }

    /// Feed a batch of complete lines.
    fn push_lines(&mut self, bytes: &[u8]) -> std::result::Result<(), String> {
        for line in bytes.split(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(line);
            self.push_line(line.strip_suffix('\r').unwrap_or(&line))?;
        }
        Ok(())
    }

    fn finish(self) -> std::result::Result<RawRows, String> {
        // Hung up (or the reply ended) inside the rows after the cap was hit:
        // the rows held are good, the trailing statistics are unknown.
        if self.stopped && self.phase != StreamPhase::Tail {
            return Ok(RawRows {
                meta: self.meta,
                data: self.rows,
                bytes_read: 0,
                prepared: true,
                truncated_bytes: self.truncated_bytes,
                partial: true,
            });
        }
        match self.phase {
            // Never saw a row-per-line layout: decode the body whole.
            StreamPhase::Head | StreamPhase::Whole => {
                decode_json_reply(&self.text).map(JsonResponse::into_raw)
            }
            StreamPhase::Data => Err(crate::errors::ch_trailing_exception(&self.text)
                .map(|ex| crate::errors::ch_midstream_message(&ex, Some(self.rows.len())))
                .unwrap_or_else(|| "clickhouse: reply ended inside the result rows".to_string())),
            StreamPhase::Tail => {
                // `"rows": n, "statistics": {…}[, "exception": "…"]` + `}`.
                let tail = format!("{{{}", self.text);
                let bytes_read = match serde_json::from_str::<JsonResponse>(&tail) {
                    Ok(reply) => {
                        if let Some(ex) = &reply.exception {
                            return Err(crate::errors::ch_midstream_message(
                                ex,
                                Some(self.rows.len()),
                            ));
                        }
                        reply.statistics.bytes_read
                    }
                    Err(_) => {
                        if let Some(ex) = crate::errors::ch_trailing_exception(&self.text) {
                            return Err(crate::errors::ch_midstream_message(
                                &ex,
                                Some(self.rows.len()),
                            ));
                        }
                        0
                    }
                };
                Ok(RawRows {
                    meta: self.meta,
                    data: self.rows,
                    bytes_read,
                    prepared: true,
                    truncated_bytes: self.truncated_bytes,
                    partial: false,
                })
            }
        }
    }
}

/// Decode a batch of lines into `state` — on the blocking pool when the batch
/// is big, inline otherwise.
async fn feed_compact(mut state: CompactStream, bytes: Vec<u8>) -> Result<CompactStream> {
    if bytes.len() < STREAM_PARSE_CHUNK_BYTES {
        state
            .push_lines(&bytes)
            .map_err(otto_core::Error::Upstream)?;
        return Ok(state);
    }
    tokio::task::spawn_blocking(move || state.push_lines(&bytes).map(|()| state))
        .await
        .map_err(|e| types::upstream(format!("clickhouse: decode task failed: {e}")))?
        .map_err(otto_core::Error::Upstream)
}

/// Decode up to `take` rows of a native column-major block into row-major
/// JSON. The column vectors are resolved once per block (not per cell).
fn decode_native_block(block: &klickhouse::block::Block, take: usize) -> Vec<Vec<Value>> {
    // Columns are ordered by `column_types` (an IndexMap preserves SELECT order).
    let cols: Vec<Option<&Vec<klickhouse::Value>>> = block
        .column_types
        .keys()
        .map(|name| block.column_data.get(name))
        .collect();
    let n = (block.rows as usize).min(take);
    let mut out = Vec::with_capacity(n);
    for r in 0..n {
        out.push(
            cols.iter()
                .map(|col| {
                    col.and_then(|c| c.get(r))
                        .map(value_to_json)
                        .unwrap_or(Value::Null)
                })
                .collect(),
        );
    }
    out
}

/// Shape the `system.tables` + `system.columns` rows of
/// [`ClickhouseDriver::schema_graph_bulk`] into a [`SchemaGraph`]. Node ids
/// match [`ClickhouseDriver::schema_children`] (`db:<db>/table:<name>`).
fn build_bulk_graph(
    schema: &str,
    tables: &RawRows,
    columns: &RawRows,
    max_tables: usize,
) -> SchemaGraph {
    let mut by_table: HashMap<&str, Vec<GraphColumn>> = HashMap::new();
    for row in &columns.data {
        let (Some(table), Some(name)) = (
            row.first().and_then(Value::as_str),
            row.get(1).and_then(Value::as_str),
        ) else {
            continue;
        };
        let data_type = row.get(2).and_then(Value::as_str).unwrap_or("").to_string();
        by_table.entry(table).or_default().push(GraphColumn {
            name: name.to_string(),
            nullable: data_type.starts_with("Nullable("),
            data_type,
            primary_key: row.get(3).is_some_and(cell_truthy),
            foreign_key: false,
        });
    }
    let truncated = tables.data.len() > max_tables;
    let tables = tables
        .data
        .iter()
        .take(max_tables)
        .filter_map(|row| {
            let name = row.first().and_then(Value::as_str)?;
            let engine = row.get(1).and_then(Value::as_str).unwrap_or("");
            Some(GraphTable {
                id: format!("db:{schema}/table:{name}"),
                schema: schema.to_string(),
                name: name.to_string(),
                kind: if engine.ends_with("View") {
                    NodeKind::View
                } else {
                    NodeKind::Table
                },
                columns: by_table.remove(name).unwrap_or_default(),
            })
        })
        .collect();
    SchemaGraph {
        schema: schema.to_string(),
        tables,
        edges: Vec::new(),
        // Same as the per-object walk (`capabilities().joins`), so the UI's
        // "no relationships" hint doesn't change with the code path.
        relationships: true,
        truncated,
    }
}

/// True when the statement contains exactly one `SELECT` keyword (no
/// subquery / UNION arm a server-side `max_result_rows` + `break` could clip).
/// Over-counting (a literal mentioning "select") only disables the cap.
fn single_select(sql: &str) -> bool {
    sql.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|tok| tok.eq_ignore_ascii_case("select"))
        .count()
        == 1
}

/// Map a `klickhouse` error to an Upstream error (a 502 — the database's fault).
/// A server exception keeps `Code: N. DB::Exception: …` (the HTTP shape) and
/// sends its stack trace to the debug log, never into the message.
fn native_err(e: klickhouse::KlickhouseError) -> otto_core::Error {
    otto_core::Error::Upstream(crate::errors::ch_native_message(&e))
}

/// Enforce a per-statement wall clock on the NATIVE transport (which has no
/// server-side `max_execution_time` request setting and no cancellable
/// `query_id` — without this a hung native query blocked forever while
/// `cancel()` was a silent no-op). `None` runs unbounded, as before.
async fn native_with_timeout<T>(
    timeout_secs: Option<u64>,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    match timeout_secs {
        Some(secs) => tokio::time::timeout(Duration::from_secs(secs), fut)
            .await
            .map_err(|_| {
                types::upstream(format!(
                    "clickhouse: native query timed out after {secs}s (client-side — the \
                     native transport has no server-side cancel)"
                ))
            })?,
        None => fut.await,
    }
}

// --- Native value decoding --------------------------------------------------

/// Convert a decoded native `klickhouse::Value` into a clean JSON value for the
/// grid. The mapping mirrors the HTTP `JSONCompact` shapes: ints/floats →
/// number, String/FixedString → string, Date/DateTime → ISO string, Nullable →
/// null/inner, Array/Tuple/Map → nested JSON. Exotic types fall back to the
/// value's own string form (`Display`), so nothing ever breaks the grid.
fn value_to_json(v: &klickhouse::Value) -> Value {
    use klickhouse::Value as V;
    match v {
        V::Null => Value::Null,

        // Signed ints: keep small ones as JSON numbers; 128/256-bit can exceed
        // f64/i64 precision, so render them as strings to avoid silent loss.
        V::Int8(x) => json_num_i(*x as i64),
        V::Int16(x) => json_num_i(*x as i64),
        V::Int32(x) => json_num_i(*x as i64),
        V::Int64(x) => json_num_i(*x),
        V::Int128(x) => Value::String(x.to_string()),
        V::Int256(_) => Value::String(v.to_string()),

        // Unsigned ints.
        V::UInt8(x) => json_num_u(*x as u64),
        V::UInt16(x) => json_num_u(*x as u64),
        V::UInt32(x) => json_num_u(*x as u64),
        V::UInt64(x) => json_num_u(*x),
        V::UInt128(x) => Value::String(x.to_string()),
        V::UInt256(_) => Value::String(v.to_string()),

        // Floats.
        V::Float32(x) => json_num_f(*x as f64),
        V::Float64(x) => json_num_f(*x),
        V::BFloat16(x) => json_num_f(f32::from(*x) as f64),

        // Decimals render with their fractional point via Display (no f64 loss).
        V::Decimal32(..) | V::Decimal64(..) | V::Decimal128(..) | V::Decimal256(..) => {
            Value::String(v.to_string())
        }

        // String/FixedString arrive as raw bytes; decode lossily to text.
        V::String(bytes) => Value::String(String::from_utf8_lossy(bytes).into_owned()),

        V::Uuid(u) => Value::String(u.to_string()),

        // Dates/times → ISO-8601 strings.
        V::Date(d) => {
            let date: chrono::NaiveDate = (*d).into();
            Value::String(date.to_string())
        }
        V::DateTime(dt) => match chrono::DateTime::<klickhouse::Tz>::try_from(*dt) {
            Ok(t) => Value::String(t.to_rfc3339()),
            Err(_) => Value::String(v.to_string()),
        },
        V::DateTime64(dt) => match chrono::DateTime::<chrono::Utc>::try_from(*dt) {
            Ok(t) => Value::String(t.to_rfc3339()),
            Err(_) => Value::String(v.to_string()),
        },

        // Enums serialize as their backing integer (matches CH JSON for raw enums).
        V::Enum8(x) => json_num_i(*x as i64),
        V::Enum16(x) => json_num_i(*x as i64),

        // Nested containers → nested JSON.
        V::Array(items) | V::Tuple(items) => {
            Value::Array(items.iter().map(value_to_json).collect())
        }
        V::Map(keys, vals) => {
            // Best-effort: a JSON object when keys stringify, else an array of
            // [key, value] pairs (covers non-string keys).
            let mut obj = serde_json::Map::new();
            let mut all_str = true;
            for (k, val) in keys.iter().zip(vals.iter()) {
                match map_key_to_string(k) {
                    Some(ks) => {
                        obj.insert(ks, value_to_json(val));
                    }
                    None => {
                        all_str = false;
                        break;
                    }
                }
            }
            if all_str {
                Value::Object(obj)
            } else {
                Value::Array(
                    keys.iter()
                        .zip(vals.iter())
                        .map(|(k, val)| Value::Array(vec![value_to_json(k), value_to_json(val)]))
                        .collect(),
                )
            }
        }

        // IPs and geo types: their string form is the natural representation.
        V::Ipv4(_) | V::Ipv6(_) => Value::String(strip_quotes(v.to_string())),
        V::Point(_) | V::Ring(_) | V::Polygon(_) | V::MultiPolygon(_) => {
            Value::String(v.to_string())
        }
    }
}

/// A JSON number from an i64.
fn json_num_i(x: i64) -> Value {
    Value::Number(x.into())
}

/// A JSON number from a u64.
fn json_num_u(x: u64) -> Value {
    Value::Number(x.into())
}

/// A JSON number from an f64; non-finite floats degrade to their string form
/// (JSON has no NaN/Infinity).
fn json_num_f(x: f64) -> Value {
    serde_json::Number::from_f64(x)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(x.to_string()))
}

/// Stringify a map key value for use as a JSON object key (strings & numbers).
fn map_key_to_string(v: &klickhouse::Value) -> Option<String> {
    use klickhouse::Value as V;
    match v {
        V::String(b) => Some(String::from_utf8_lossy(b).into_owned()),
        V::Int8(_)
        | V::Int16(_)
        | V::Int32(_)
        | V::Int64(_)
        | V::UInt8(_)
        | V::UInt16(_)
        | V::UInt32(_)
        | V::UInt64(_) => Some(v.to_string()),
        _ => None,
    }
}

/// The native `Value::Display` wraps IPs in single quotes (`'1.2.3.4'`); strip a
/// single layer of surrounding quotes for a clean cell.
fn strip_quotes(s: String) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        t[1..t.len() - 1].to_string()
    } else {
        s
    }
}

/// Render a JSON cell as plain text for raw-text replies (SHOW CREATE etc.):
/// unwrap a JSON string, leave other scalars as their JSON text.
fn json_cell_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

// --- Shared helpers ---------------------------------------------------------

/// Map a reqwest transport error into an Upstream error, unwinding its source
/// chain so the *real* cause (TLS handshake failure, connection refused, …) is
/// visible instead of the terse "error sending request for url (…)". Critical
/// for diagnosing TLS-through-an-SSH-tunnel issues (cert host mismatch, or a
/// plain-HTTP port reached over HTTPS).
/// True for the server's refusal to change the `readonly` setting — what a
/// user whose profile is already read-only gets when a request sets it.
fn is_readonly_setting_refused(message: &str) -> bool {
    message.contains("Cannot modify 'readonly' setting")
}

fn req_err(e: reqwest::Error) -> otto_core::Error {
    use std::error::Error as _;
    let mut msg = e.to_string();
    let mut src: Option<&dyn std::error::Error> = e.source();
    while let Some(s) = src {
        let part = s.to_string();
        if !msg.contains(&part) {
            msg.push_str(": ");
            msg.push_str(&part);
        }
        src = s.source();
    }
    otto_core::Error::Upstream(msg)
}

/// Escape a value for embedding inside a ClickHouse single-quoted string
/// literal. ClickHouse honours BACKSLASH escapes in string literals, so `\` must
/// be doubled BEFORE quotes are doubled — otherwise a value ending in `\`
/// swallows the closing quote and the following bytes parse as raw SQL (the
/// object-search box and node-path segments reach here with user input).
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "''")
}

/// Truthy interpretation of a `system.columns.is_in_primary_key` cell, which
/// may arrive as a JSON number (`1`), string (`"1"`), or bool depending on the
/// ClickHouse output format.
fn cell_truthy(v: &Value) -> bool {
    v.as_i64() == Some(1) || v.as_str() == Some("1") || v.as_bool() == Some(true)
}

/// Whether a (case-insensitive) column `name` appears as a bare identifier token
/// in a data-skipping-index expression `expr` (best-effort: splits on non-ident
/// chars so `lower(country)` matches `country`).
fn expr_mentions(expr: &str, name: &str) -> bool {
    expr.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$')
        .any(|tok| tok.eq_ignore_ascii_case(name))
}

/// A fresh, opaque ClickHouse `query_id` for one execution. A ULID — purely
/// `[0-9A-Z]`, so it's safe in the `query_id` request param and the
/// `KILL QUERY WHERE query_id = '…'` literal.
fn new_query_id() -> String {
    format!("otto-{}", otto_core::new_id())
}

/// Escape a backtick-quoted identifier (`db`.`tbl`). ClickHouse applies string
/// escaping rules inside backtick idents too, so `\` must be doubled as well —
/// a trailing `\` would otherwise swallow the closing backtick.
fn esc_ident(s: &str) -> String {
    s.replace('\\', "\\\\").replace('`', "``")
}

/// Does this statement return a rowset? (first keyword decides).
fn returns_rows(sql: &str) -> bool {
    let first = sql
        .trim_start()
        .split(|c: char| c.is_whitespace() || c == '(')
        .find(|w| !w.is_empty())
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        first.as_str(),
        "SELECT" | "SHOW" | "DESC" | "DESCRIBE" | "EXPLAIN" | "WITH"
    )
}

/// System databases that exist on every server; keep them but sort last.
fn is_system_db(name: &str) -> bool {
    matches!(name, "system" | "INFORMATION_SCHEMA" | "information_schema")
}

// Per-cell size capping (≈1 MiB, truncation marker) now lives in
// [`crate::types::cap_cell`], shared by the MySQL and Postgres drivers too so
// oversized cells behave identically across engines.
use crate::types::cap_cell;

#[async_trait]
impl Driver for ClickhouseDriver {
    fn engine(&self) -> Engine {
        Engine::Clickhouse
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            engine: Engine::Clickhouse,
            sql: true,
            joins: true,
            transactions: false,
            multi_statement: true,
            // Server-side cancel via `KILL QUERY WHERE query_id=…` — only tagged on
            // the HTTP transport, but advertised as capable; the native transport's
            // cancel is a harmless no-op.
            cancel: true,
            // `EXPLAIN` / `EXPLAIN json=1`.
            explain: true,
            default_port: 8123,
            schema_levels: vec!["Database".into(), "Table".into(), "Column".into()],
            query_language: "sql".into(),
        }
    }

    async fn test(&self, cfg: &ResolvedConfig) -> Result<TestResult> {
        let started = Instant::now();
        match self.query_rows(cfg, "SELECT version()").await {
            Ok(resp) => {
                let latency = started.elapsed().as_millis() as u64;
                let version = resp
                    .data
                    .first()
                    .and_then(|row| row.first())
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Ok(TestResult {
                    ok: true,
                    latency_ms: Some(latency),
                    message: "connected".to_string(),
                    server_version: version,
                })
            }
            Err(e) => Ok(TestResult {
                ok: false,
                latency_ms: None,
                message: e.to_string(),
                server_version: None,
            }),
        }
    }

    async fn schema_root(&self, cfg: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
        let resp = self
            .query_rows(cfg, "SELECT name FROM system.databases ORDER BY name")
            .await?;
        let mut nodes: Vec<(bool, SchemaNode)> = resp
            .first_col_strs()
            .map(|name| {
                let node =
                    SchemaNode::new(format!("db:{name}"), name, NodeKind::Database).expandable();
                (is_system_db(name), node)
            })
            .collect();
        // User databases first, system databases last; preserve name order within.
        nodes.sort_by_key(|a| a.0);
        Ok(nodes.into_iter().map(|(_, n)| n).collect())
    }

    async fn search_objects(
        &self,
        cfg: &ResolvedConfig,
        req: &ObjectSearchReq,
    ) -> Result<ObjectSearchResult> {
        // `system.tables` is a single cheap catalog read covering every database.
        let limit = req.capped();
        let mut sql = format!(
            "SELECT database, name, engine FROM system.tables \
             WHERE positionCaseInsensitive(name, '{}') > 0 \
             AND database NOT IN ('system','INFORMATION_SCHEMA','information_schema')",
            esc(&req.q)
        );
        if !req.all_schemas() {
            sql.push_str(&format!(
                " AND database = '{}'",
                esc(req.schema.as_deref().unwrap_or(""))
            ));
        }
        sql.push_str(&format!(" ORDER BY database, name LIMIT {}", limit + 1));
        let rows = self.query_rows(cfg, &sql).await?;

        let truncated = rows.data.len() > limit;
        let mut schemas: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut hits = Vec::new();
        for row in rows.data.iter().take(limit) {
            let db = row.first().and_then(Value::as_str).unwrap_or("");
            let name = row.get(1).and_then(Value::as_str).unwrap_or("");
            let engine = row.get(2).and_then(Value::as_str).unwrap_or("");
            if db.is_empty() || name.is_empty() {
                continue;
            }
            // ClickHouse has no TABLE_TYPE; the *View engines are the views.
            let (kind, seg, label) = if engine.ends_with("View") {
                (NodeKind::View, "table", "view")
            } else {
                (NodeKind::Table, "table", "table")
            };
            if !req.wants(label) {
                continue;
            }
            schemas.insert(db.to_string());
            hits.push(ObjectHit {
                schema: db.to_string(),
                name: name.to_string(),
                kind,
                path: format!("db:{db}/{seg}:{name}"),
            });
        }
        Ok(ObjectSearchResult {
            hits,
            truncated,
            scanned: schemas.len(),
            supported: true,
        })
    }

    async fn schema_children(
        &self,
        cfg: &ResolvedConfig,
        parent: &NodePath,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let db = parent
            .get("db")
            .ok_or_else(|| types::invalid("clickhouse: missing database in node path"))?;

        if let Some(table) = parent.get("table") {
            // Columns of a table/view — filter by name when a prefix/substring
            // is supplied (case-insensitive server-side via ILIKE).
            let sql = if let Some(f) = filter.filter(|s| !s.is_empty()) {
                format!(
                    "SELECT name, type FROM system.columns \
                     WHERE database = '{}' AND table = '{}' AND name ILIKE '%{}%' ORDER BY position",
                    esc(db),
                    esc(table),
                    esc(f),
                )
            } else {
                format!(
                    "SELECT name, type FROM system.columns \
                     WHERE database = '{}' AND table = '{}' ORDER BY position",
                    esc(db),
                    esc(table),
                )
            };
            let resp = self.query_rows(cfg, &sql).await?;
            let base = parent.to_id();
            Ok(resp
                .data
                .iter()
                .filter_map(|row| {
                    let name = row.first().and_then(Value::as_str)?;
                    let ty = row.get(1).and_then(Value::as_str).unwrap_or("");
                    Some(
                        SchemaNode::new(format!("{base}/column:{name}"), name, NodeKind::Column)
                            .with_detail(ty),
                    )
                })
                .collect())
        } else {
            // Tables + views of a database — filter by name when present.
            let sql = if let Some(f) = filter.filter(|s| !s.is_empty()) {
                format!(
                    "SELECT name, engine FROM system.tables \
                     WHERE database = '{}' AND name ILIKE '%{}%' ORDER BY name",
                    esc(db),
                    esc(f),
                )
            } else {
                format!(
                    "SELECT name, engine FROM system.tables WHERE database = '{}' ORDER BY name",
                    esc(db),
                )
            };
            let resp = self.query_rows(cfg, &sql).await?;
            Ok(resp
                .data
                .iter()
                .filter_map(|row| {
                    let name = row.first().and_then(Value::as_str)?;
                    let engine = row.get(1).and_then(Value::as_str).unwrap_or("");
                    let kind = if engine.ends_with("View") {
                        NodeKind::View
                    } else {
                        NodeKind::Table
                    };
                    // Engine is shown as a secondary, space-permitting detail
                    // (the tree layout gives the table name priority and lets
                    // the engine truncate first; full engine is on hover).
                    Some(
                        SchemaNode::new(format!("db:{db}/table:{name}"), name, kind)
                            .with_detail(engine)
                            .expandable(),
                    )
                })
                .collect())
        }
    }

    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        let db = path
            .get("db")
            .ok_or_else(|| types::invalid("clickhouse: missing database in node path"))?;
        let table = path
            .get("table")
            .ok_or_else(|| types::invalid("clickhouse: missing table in node path"))?;

        // Columns.
        let col_sql = format!(
            "SELECT name, type, default_kind, default_expression, comment, is_in_primary_key \
             FROM system.columns WHERE database = '{}' AND table = '{}' ORDER BY position",
            esc(db),
            esc(table)
        );
        // Table-level metadata.
        let tbl_sql = format!(
            "SELECT engine, partition_key, sorting_key, primary_key, total_rows \
             FROM system.tables WHERE database = '{}' AND name = '{}'",
            esc(db),
            esc(table)
        );
        // DDL via SHOW CREATE (raw text, tab-separated raw so we get it verbatim
        // over HTTP; over native it comes back as a single string cell).
        let ddl_sql = format!(
            "SHOW CREATE TABLE `{}`.`{}` FORMAT TabSeparatedRaw",
            esc_ident(db),
            esc_ident(table)
        );
        // The three catalog reads are independent: one round-trip wave, not
        // three (DB-08). A failed SHOW CREATE only drops the DDL.
        let (cols, tbl, ddl) = tokio::try_join!(
            self.query_rows(cfg, &col_sql),
            self.query_rows(cfg, &tbl_sql),
            async { Ok::<_, otto_core::Error>(self.query_text(cfg, &ddl_sql).await.ok()) },
        )?;
        let ddl = ddl
            .map(|s| s.trim_end().to_string())
            .filter(|s| !s.is_empty());
        let mut columns = Vec::new();
        let mut primary_key = Vec::new();
        for row in &cols.data {
            let name = row
                .first()
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let data_type = row.get(1).and_then(Value::as_str).unwrap_or("").to_string();
            let default_kind = row.get(2).and_then(Value::as_str).unwrap_or("");
            let default_expr = row.get(3).and_then(Value::as_str).unwrap_or("");
            let comment = row.get(4).and_then(Value::as_str).unwrap_or("");
            // is_in_primary_key arrives as a UInt8 number (0/1).
            let in_pk = row
                .get(5)
                .map(|v| v.as_u64() == Some(1) || v.as_str() == Some("1"))
                .unwrap_or(false);

            let default = if !default_expr.is_empty() {
                Some(if default_kind.is_empty() {
                    default_expr.to_string()
                } else {
                    format!("{default_kind} {default_expr}")
                })
            } else {
                None
            };

            if in_pk {
                primary_key.push(name.clone());
            }
            // ClickHouse has no SQL NULL semantics unless the type is Nullable(..).
            let nullable = data_type.starts_with("Nullable(");
            columns.push(ColumnDef {
                name,
                data_type,
                nullable,
                default,
                key: in_pk.then(|| "PRI".to_string()),
                extra: None,
                comment: (!comment.is_empty()).then(|| comment.to_string()),
            });
        }

        let row = tbl.data.first();
        let engine = row
            .and_then(|r| r.first())
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let partition_key = row
            .and_then(|r| r.get(1))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let sorting_key = row
            .and_then(|r| r.get(2))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let pk_expr = row
            .and_then(|r| r.get(3))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let row_count = row.and_then(|r| r.get(4)).and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        });

        let kind = if engine.ends_with("View") {
            NodeKind::View
        } else {
            NodeKind::Table
        };

        let mut detail = ObjectDetail::new(table, kind);
        detail.columns = columns;
        detail.primary_key = primary_key;
        detail.row_count = row_count;
        detail.ddl = ddl;
        detail.extra = serde_json::json!({
            "engine": engine,
            "partition_key": partition_key,
            "sorting_key": sorting_key,
            "primary_key": pk_expr,
        });
        Ok(detail)
    }

    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        // Run with a throwaway token (no cancel tracking for the bare `run`).
        self.run_tracked(cfg, req, &CancelToken::new()).await
    }

    /// Evict the cached transport handles for `cache_key` (connection close, or
    /// a config change superseded them). Dropping the `reqwest::Client` releases
    /// its keep-alive pool; dropping the last `Arc<klickhouse::Client>` closes
    /// the native connection. Also drops the matching completion snapshot.
    fn detach(&self, cache_key: &str) -> Option<std::sync::Arc<dyn Driver>> {
        let captured = Self::default();
        for (key, value) in self.clients.take_where(|key| key == cache_key) {
            captured.clients.insert_ready(key, value);
        }
        for (key, value) in self.native.take_where(|key| key == cache_key) {
            captured.native.insert_ready(key, value);
        }
        self.completions.invalidate(cache_key);
        Some(std::sync::Arc::new(captured))
    }

    async fn close(&self, cache_key: &str) {
        self.clients.remove(cache_key);
        self.native.remove(cache_key);
        self.completions.invalidate(cache_key);
        self.readonly_refused
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(cache_key);
    }

    /// Detach handles idle past `idle` and DROP the cache's reference — never
    /// close explicitly: a query running longer than the window holds its own
    /// clone, and the handle goes away with the last clone.
    async fn evict_idle(&self, idle: Duration) -> usize {
        let http = self.clients.take_idle(idle);
        let native = self.native.take_idle(idle);
        let mut memo = self
            .readonly_refused
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for (key, _) in &http {
            memo.remove(key);
        }
        http.len() + native.len()
    }

    /// Diagram / assistant schema in two set-based catalog reads (DB-03)
    /// instead of an `object_detail` (3 round trips) per table. ClickHouse has
    /// no foreign keys, so the graph has no edges.
    async fn schema_graph_bulk(
        &self,
        cfg: &ResolvedConfig,
        schema: &str,
        max_tables: usize,
    ) -> Result<Option<SchemaGraph>> {
        let db = esc(schema);
        // One extra row tells us whether the list was clipped.
        let tables_sql = format!(
            "SELECT name, engine FROM system.tables WHERE database = '{db}' \
             ORDER BY name LIMIT {}",
            max_tables.saturating_add(1)
        );
        let columns_sql = format!(
            "SELECT table, name, type, is_in_primary_key FROM system.columns \
             WHERE database = '{db}' AND table IN \
             (SELECT name FROM system.tables WHERE database = '{db}' ORDER BY name LIMIT {max_tables}) \
             ORDER BY table, position"
        );
        let (tables, columns) = tokio::try_join!(
            self.query_rows(cfg, &tables_sql),
            self.query_rows(cfg, &columns_sql),
        )?;
        Ok(Some(build_bulk_graph(
            schema, &tables, &columns, max_tables,
        )))
    }

    async fn run_tracked(
        &self,
        cfg: &ResolvedConfig,
        req: &QueryRequest,
        token: &CancelToken,
    ) -> Result<QueryResult> {
        let text = req.statement.trim();
        if text.is_empty() {
            return Err(types::invalid("clickhouse: empty statement"));
        }
        let max_rows = req.max_rows.unwrap_or(1000);

        // The active database (if the user selected one) scopes unqualified
        // table names — see query_rows_db for how it's applied per transport.
        let scope_db = req.scope_database();
        let active_db = scope_db.as_deref();

        // Convert ms → seconds (round up) for ClickHouse's `max_execution_time`.
        let timeout_secs = req.timeout_ms.filter(|&t| t > 0).map(|t| t.div_ceil(1000));
        // Server-side cancel via query_id only exists on the HTTP transport.
        let http = transport_for(cfg) == Transport::Http;

        // A true batch (>1 statement) runs each statement in order, tagging each
        // with its own query_id so a cancel hits the running one; the single path
        // keeps auto-LIMIT/OFFSET injection (§2.2).
        let spans = split_statements(text, SqlDialect::Clickhouse);
        if spans.len() > 1 {
            return self
                .run_ch_batch(cfg, &spans, max_rows, active_db, timeout_secs, token)
                .await;
        }
        // 0 spans ⇒ comment-only paste: run the original text (unchanged behavior).
        let stmt = spans.first().map(|s| s.text.as_str()).unwrap_or(text);

        // Tag this execution with a fresh server-side query_id (HTTP transport)
        // and record it in the token, so a concurrent cancel can KILL it. The
        // native transport ignores it (cancel becomes a no-op there).
        let query_id = new_query_id();
        if http {
            token.set(QueryHandle::ClickhouseQueryId(query_id.clone()));
        }

        if returns_rows(stmt) {
            let ri = types::inject_row_limit(stmt, max_rows.saturating_add(1), req.offset);
            let mut r = self
                .exec_ch_read(
                    cfg,
                    &ri.sql,
                    active_db,
                    Some(query_id),
                    timeout_secs,
                    max_rows,
                    ri.limited,
                )
                .await?;
            // Report the user-visible page size (max_rows), not the +1 probe.
            r.auto_limited = ri.limited.then_some(max_rows as u64);
            Ok(r)
        } else {
            self.exec_ch_write(cfg, stmt, active_db, Some(query_id), timeout_secs)
                .await
        }
    }

    /// Cancel the running query by its `query_id`: `KILL QUERY WHERE query_id =
    /// '<id>'`. Issued on a separate connection (always HTTP here — the captured
    /// handle only exists for HTTP-transport runs). `KILL QUERY ... SYNC` would
    /// block until the query actually stops; we use the default async form so the
    /// cancel returns promptly and ClickHouse stops the query out of band. An
    /// already-finished query simply matches no rows — a successful no-op.
    async fn cancel(&self, cfg: &ResolvedConfig, handle: &QueryHandle) -> Result<()> {
        let QueryHandle::ClickhouseQueryId(qid) = handle else {
            return Ok(());
        };
        // `query_id` is a server-generated UUID; still escape defensively for the
        // string literal.
        let sql = format!("KILL QUERY WHERE query_id = '{}'", esc(qid));
        // Best-effort: a no-match KILL is a successful no-op; don't surface a
        // transient error as a cancel failure.
        if let Some(client) = self.clients.get_ready(&cfg.cache_key()) {
            let conn = Self::connection_from_client(cfg, None, None, None, client)?;
            let _ = conn.query_raw(&sql).await;
        }
        Ok(())
    }

    /// Structured query plan via `EXPLAIN json = 1` (a single JSON cell), falling
    /// back to plain-text `EXPLAIN` (one node per line) when JSON isn't available.
    /// The statement is EXPLAIN-wrapped — never executed raw.
    async fn query_plan(
        &self,
        cfg: &ResolvedConfig,
        statement: &str,
        node: Option<&str>,
    ) -> Result<DbQueryPlan> {
        let stmt = statement.trim().trim_end_matches(';');
        if stmt.is_empty() {
            return Err(types::invalid("clickhouse: empty statement"));
        }
        let active_db = node.map(str::trim).filter(|s| !s.is_empty());

        // Preferred: EXPLAIN json = 1 → the plan JSON in the first cell.
        if let Ok(resp) = self
            .query_rows_db_timeout(
                cfg,
                &format!("EXPLAIN json = 1 {stmt}"),
                active_db,
                None,
                None,
            )
            .await
        {
            let raw: Option<serde_json::Value> = match resp.data.first().and_then(|r| r.first()) {
                Some(serde_json::Value::String(s)) => serde_json::from_str(s).ok(),
                Some(other) => Some(other.clone()),
                None => None,
            };
            if let Some(raw) = raw {
                let root = crate::plan::from_clickhouse_json(&raw);
                return Ok(DbQueryPlan {
                    engine: "clickhouse".into(),
                    root,
                    raw,
                });
            }
        }

        // Fallback: plain EXPLAIN → text lines as a single-node plan.
        let resp = self
            .query_rows_db_timeout(cfg, &format!("EXPLAIN {stmt}"), active_db, None, None)
            .await?;
        let lines: Vec<String> = resp
            .data
            .iter()
            .filter_map(|r| r.first())
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect();
        let raw = serde_json::Value::Array(
            lines
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        );
        let root = crate::plan::from_clickhouse_text(&lines);
        Ok(DbQueryPlan {
            engine: "clickhouse".into(),
            root,
            raw,
        })
    }

    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        let scope_db = ctx
            .database
            .clone()
            .or_else(|| cfg.database.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_default();

        let snap = self.completion_snapshot(cfg, &scope_db).await;
        let sql_ctx = crate::complete::sql::analyze(&ctx.prefix, &ctx.suffix);
        let items = crate::complete::sql::assemble(&sql_ctx, &snap, KEYWORDS, FUNCTIONS);
        Ok(CompletionResponse {
            items,
            ..Default::default()
        })
    }

    async fn invalidate_completion_cache(&self, cfg: &ResolvedConfig) {
        self.completions.invalidate(&cfg.cache_key());
    }

    fn cached_completion_snapshot(
        &self,
        cfg: &ResolvedConfig,
        scope: &str,
    ) -> Option<std::sync::Arc<crate::complete::SchemaSnapshot>> {
        self.completions.get_snapshot(&cfg.cache_key(), scope)
    }

    fn assemble_completion(
        &self,
        snap: &crate::complete::SchemaSnapshot,
        ctx: &CompletionContext,
    ) -> Vec<crate::types::CompletionItem> {
        let sql_ctx = crate::complete::sql::analyze(&ctx.prefix, &ctx.suffix);
        crate::complete::sql::assemble(&sql_ctx, snap, KEYWORDS, FUNCTIONS)
    }

    /// Streaming export. On the HTTP transport (the common case, incl. over an
    /// SSH tunnel) we ask ClickHouse for an explicit `FORMAT <fmt>` and stream the
    /// response body (`reqwest` `bytes_stream()`) straight to the file — constant
    /// memory, native formatting, and it lands on the user's LOCAL path (NOT a
    /// server-side `INTO OUTFILE` that would write to the tunnel host). The `json`
    /// array format has no single-pass CH FORMAT, so it falls back to the
    /// daemon-side row formatter (logged); on the native transport we stream
    /// result blocks and format rows ourselves.
    async fn export_to_writer(
        &self,
        cfg: &ResolvedConfig,
        statement: &str,
        node: Option<&str>,
        format: ExportFormat,
        max_rows: Option<usize>,
        w: Box<dyn std::io::Write + Send>,
    ) -> Result<ExportCounts> {
        let sql = statement.trim();
        if sql.is_empty() {
            return Err(types::invalid("clickhouse: empty statement"));
        }
        if !returns_rows(sql) {
            return Err(types::invalid(
                "export supports row-returning statements only",
            ));
        }
        let active_db = node.map(str::trim).filter(|s| !s.is_empty());

        // HTTP + a streamable FORMAT → native streaming, the preferred path.
        if transport_for(cfg) == Transport::Http {
            if let Some(ch_format) = format.clickhouse_format() {
                return self
                    .export_http_format(cfg, sql, active_db, ch_format, max_rows, format, w)
                    .await;
            }
            // JSON-array format: no single CH FORMAT — fall back to the buffered
            // daemon-side formatter (logged so it's never silent).
            tracing::warn!(
                "clickhouse export: 'json' array format has no streaming CH FORMAT — \
                 buffering the result to format it daemon-side"
            );
            let resp = self
                .query_rows_db(cfg, &capped_sql(sql, max_rows), active_db, None)
                .await?;
            return write_rawrows(w, format, &resp, max_rows);
        }

        // Native transport: stream result blocks (already done by native_query,
        // which decodes block-by-block) and format rows ourselves. The native
        // client doesn't expose active-db scoping here (see query_rows_db TODO),
        // so unqualified names resolve against the profile database.
        tracing::warn!(
            "clickhouse export: native transport materialises decoded blocks before \
             formatting (no FORMAT streaming); large exports may use more memory than HTTP"
        );
        let resp = self.native_query(cfg, &capped_sql(sql, max_rows)).await?;
        write_rawrows(w, format, &resp, max_rows)
    }
}

impl ClickhouseDriver {
    /// The HTTP `FORMAT`-streaming export: issue `<sql> FORMAT <ch_format>` and
    /// splice the response body through `w` chunk-by-chunk. Row count isn't known
    /// from the streamed bytes, so it's reported as 0 (bytes are exact).
    #[allow(clippy::too_many_arguments)]
    async fn export_http_format(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        ch_format: &str,
        max_rows: Option<usize>,
        format: ExportFormat,
        w: Box<dyn std::io::Write + Send>,
    ) -> Result<ExportCounts> {
        use futures_util::StreamExt as _;

        let conn = self.connect_id(cfg, active_db, None).await?;
        let body = format!("{}\nFORMAT {ch_format}", capped_sql(sql, max_rows));
        let resp = conn.post_stream(body).await?;

        let mut sink = ExportSink::new(w, format);
        let mut body = BodyDecoder::for_response(&resp)?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(req_err)?;
            let chunk = body.decode(&chunk)?;
            sink.write_raw(&chunk)
                .map_err(|e| otto_core::Error::Internal(format!("write export chunk: {e}")))?;
        }
        sink.finish()
            .map_err(|e| otto_core::Error::Internal(format!("finish export file: {e}")))
    }
}

// --- HTTP response compression (DB2-05) -------------------------------------

/// Ask ClickHouse to compress the response body: `enable_http_compression=1`
/// plus `Accept-Encoding: zstd`. `FORMAT JSONCompact` / TSV / CSV compress
/// 5–10× and a tunnel/WAN link is bandwidth-bound, so a page or an export
/// arrives several times faster. Decoded by [`BodyDecoder`] — NOT by
/// enabling reqwest's `zstd`/`gzip` features, which would switch on silent
/// auto-decompression for every other reqwest user in the daemon (the API
/// client must show the wire as-is). A server that ignores the request
/// answers uncompressed (no `Content-Encoding`) and that passes through.
fn compressed(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    req.header(reqwest::header::ACCEPT_ENCODING, "zstd")
        .query(&[("enable_http_compression", "1")])
}

/// Streaming decoder for a ClickHouse response body (see [`compressed`]):
/// chunk in, decompressed bytes out, so the line-by-line JSONCompact reader,
/// the capped reader and the export sink stay streaming.
enum BodyDecoder {
    Identity,
    Zstd {
        dec: Box<zstd::stream::raw::Decoder<'static>>,
        /// Reused output window.
        window: Vec<u8>,
    },
}

/// Output window per `Decoder::run` call.
const ZSTD_WINDOW: usize = 128 * 1024;
/// Most one network chunk may decode to. Totals stay bounded by each reader's
/// own cap (the run path's row/byte budget, [`HTTP_RESPONSE_BYTE_CAP`]); this
/// stops a single decompression bomb chunk before it reaches them.
const ZSTD_CHUNK_DECODE_CAP: usize = 64 * 1024 * 1024;

impl BodyDecoder {
    fn for_response(resp: &reqwest::Response) -> Result<Self> {
        Self::for_encoding(
            resp.headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok()),
        )
    }

    fn for_encoding(encoding: Option<&str>) -> Result<Self> {
        match encoding.map(|e| e.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("identity") => Ok(Self::Identity),
            Some("zstd") => Ok(Self::Zstd {
                dec: Box::new(zstd::stream::raw::Decoder::new().map_err(|e| {
                    otto_core::Error::Internal(format!("clickhouse: zstd decoder: {e}"))
                })?),
                window: vec![0u8; ZSTD_WINDOW],
            }),
            Some(other) => Err(otto_core::Error::Upstream(format!(
                "clickhouse: unsupported response encoding '{other}'"
            ))),
        }
    }

    /// Decode one body chunk (identity borrows it unchanged).
    fn decode<'a>(&mut self, chunk: &'a [u8]) -> Result<std::borrow::Cow<'a, [u8]>> {
        use zstd::stream::raw::{InBuffer, Operation as _, OutBuffer};
        let (dec, window) = match self {
            Self::Identity => return Ok(std::borrow::Cow::Borrowed(chunk)),
            Self::Zstd { dec, window } => (dec, window),
        };
        let mut out = Vec::with_capacity(chunk.len().saturating_mul(4));
        let mut input = InBuffer::around(chunk);
        loop {
            let mut output = OutBuffer::around(&mut window[..]);
            dec.run(&mut input, &mut output).map_err(|e| {
                otto_core::Error::Upstream(format!("clickhouse: corrupt zstd response: {e}"))
            })?;
            let n = output.pos();
            out.extend_from_slice(&window[..n]);
            if out.len() > ZSTD_CHUNK_DECODE_CAP {
                return Err(response_cap_error());
            }
            // Input consumed and the window not filled ⇒ everything decodable
            // so far has been flushed.
            if input.pos() == chunk.len() && n < window.len() {
                break;
            }
        }
        Ok(std::borrow::Cow::Owned(out))
    }
}

/// Append a `LIMIT n` to cap the export when `max_rows` is set (ClickHouse honours
/// a trailing LIMIT on the export query so the stream stops server-side). Reuses
/// the conservative limit injector shared with the interactive path.
fn capped_sql(sql: &str, max_rows: Option<usize>) -> String {
    match max_rows {
        Some(n) => types::inject_row_limit(sql, n, None).sql,
        None => sql.to_string(),
    }
}

/// Write a materialised [`RawRows`] (the JSON-array fallback / native path)
/// through the row formatter to `w`, honouring `max_rows`.
fn write_rawrows(
    w: Box<dyn std::io::Write + Send>,
    format: ExportFormat,
    resp: &RawRows,
    max_rows: Option<usize>,
) -> Result<ExportCounts> {
    let columns: Vec<Column> = resp
        .meta
        .iter()
        .map(|(name, ty)| Column::typed(name, ty))
        .collect();
    let mut sink = ExportSink::new(w, format);
    sink.write_header(&columns)
        .map_err(|e| otto_core::Error::Internal(format!("write export header: {e}")))?;
    let take = max_rows.unwrap_or(usize::MAX);
    for row in resp.data.iter().take(take) {
        sink.write_row(row)
            .map_err(|e| otto_core::Error::Internal(format!("write export row: {e}")))?;
    }
    sink.finish()
        .map_err(|e| otto_core::Error::Internal(format!("finish export file: {e}")))
}

// --- Static completion sources ----------------------------------------------

/// ClickHouse SQL keywords / clause heads.
const KEYWORDS: &[&str] = &[
    "SELECT",
    "DISTINCT",
    "FROM",
    "PREWHERE",
    "WHERE",
    "GROUP BY",
    "HAVING",
    "ORDER BY",
    "LIMIT",
    "OFFSET",
    "LIMIT BY",
    "WITH",
    "WITH TOTALS",
    "WITH ROLLUP",
    "WITH CUBE",
    "UNION ALL",
    "UNION DISTINCT",
    "INTERSECT",
    "EXCEPT",
    "JOIN",
    "INNER JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "FULL JOIN",
    "CROSS JOIN",
    "ANY JOIN",
    "ALL JOIN",
    "ASOF JOIN",
    "ARRAY JOIN",
    "LEFT ARRAY JOIN",
    "ON",
    "USING",
    "AS",
    "AND",
    "OR",
    "NOT",
    "IN",
    "GLOBAL IN",
    "BETWEEN",
    "LIKE",
    "ILIKE",
    "IS NULL",
    "IS NOT NULL",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "ASC",
    "DESC",
    "NULLS FIRST",
    "NULLS LAST",
    "SETTINGS",
    "FORMAT",
    "SAMPLE",
    "FINAL",
    "INSERT INTO",
    "VALUES",
    "SELECT *",
    "CREATE TABLE",
    "CREATE DATABASE",
    "CREATE VIEW",
    "CREATE MATERIALIZED VIEW",
    "ATTACH",
    "DETACH",
    "ALTER TABLE",
    "DROP TABLE",
    "DROP DATABASE",
    "RENAME TABLE",
    "TRUNCATE TABLE",
    "OPTIMIZE TABLE",
    "ENGINE",
    "PARTITION BY",
    "PRIMARY KEY",
    "ORDER BY",
    "TTL",
    "SHOW DATABASES",
    "SHOW TABLES",
    "SHOW CREATE TABLE",
    "DESCRIBE TABLE",
    "EXPLAIN",
    "SYSTEM",
    "USE",
];

/// ClickHouse builtin functions with a short signature/summary.
const FUNCTIONS: &[(&str, &str)] = &[
    ("count", "count() / count(expr) — number of rows"),
    ("countIf", "countIf(cond) — count where cond"),
    ("countDistinct", "countDistinct(expr) — alias for uniqExact"),
    ("sum", "sum(expr) — total"),
    ("sumIf", "sumIf(expr, cond) — conditional sum"),
    ("avg", "avg(expr) — mean"),
    ("avgIf", "avgIf(expr, cond) — conditional mean"),
    ("min", "min(expr)"),
    ("max", "max(expr)"),
    ("any", "any(expr) — first encountered value"),
    ("anyLast", "anyLast(expr) — last encountered value"),
    ("argMin", "argMin(arg, val) — arg of the minimum val"),
    ("argMax", "argMax(arg, val) — arg of the maximum val"),
    ("uniq", "uniq(expr) — approximate distinct count"),
    ("uniqExact", "uniqExact(expr) — exact distinct count"),
    ("uniqCombined", "uniqCombined(expr) — distinct count"),
    ("quantile", "quantile(level)(expr) — approximate quantile"),
    ("quantiles", "quantiles(l1, l2, ...)(expr)"),
    ("quantileExact", "quantileExact(level)(expr)"),
    ("median", "median(expr) — quantile(0.5)"),
    ("stddevPop", "stddevPop(expr)"),
    ("stddevSamp", "stddevSamp(expr)"),
    ("varPop", "varPop(expr)"),
    ("varSamp", "varSamp(expr)"),
    ("groupArray", "groupArray(expr) — values into an array"),
    (
        "groupUniqArray",
        "groupUniqArray(expr) — distinct values array",
    ),
    ("groupArrayInsertAt", "groupArrayInsertAt(x, pos)"),
    ("arrayJoin", "arrayJoin(arr) — unfold an array into rows"),
    ("arrayMap", "arrayMap(func, arr)"),
    ("arrayFilter", "arrayFilter(func, arr)"),
    ("arraySum", "arraySum(arr)"),
    ("arrayCount", "arrayCount(func, arr)"),
    ("arrayElement", "arrayElement(arr, n) — arr[n]"),
    ("has", "has(arr, elem) — array contains element"),
    ("hasAll", "hasAll(arr, subset)"),
    ("hasAny", "hasAny(arr, set)"),
    ("indexOf", "indexOf(arr, x)"),
    ("length", "length(x) — array/string length"),
    ("empty", "empty(x)"),
    ("notEmpty", "notEmpty(x)"),
    ("toDate", "toDate(x) — cast to Date"),
    ("toDateTime", "toDateTime(x) — cast to DateTime"),
    ("toDateTime64", "toDateTime64(x, precision)"),
    ("toStartOfDay", "toStartOfDay(dt)"),
    ("toStartOfHour", "toStartOfHour(dt)"),
    ("toStartOfMinute", "toStartOfMinute(dt)"),
    ("toStartOfMonth", "toStartOfMonth(d)"),
    ("toStartOfWeek", "toStartOfWeek(d)"),
    ("toStartOfYear", "toStartOfYear(d)"),
    ("toYYYYMM", "toYYYYMM(d)"),
    ("toYear", "toYear(d)"),
    ("toMonth", "toMonth(d)"),
    ("toDayOfMonth", "toDayOfMonth(d)"),
    ("toHour", "toHour(dt)"),
    ("toMinute", "toMinute(dt)"),
    ("dateDiff", "dateDiff(unit, start, end)"),
    ("dateAdd", "dateAdd(unit, n, date)"),
    ("dateSub", "dateSub(unit, n, date)"),
    ("now", "now() — current DateTime"),
    ("today", "today() — current Date"),
    ("yesterday", "yesterday() — previous Date"),
    ("formatDateTime", "formatDateTime(dt, format)"),
    ("toString", "toString(x) — cast to String"),
    ("toInt32", "toInt32(x)"),
    ("toInt64", "toInt64(x)"),
    ("toUInt64", "toUInt64(x)"),
    ("toFloat64", "toFloat64(x)"),
    ("toDecimal64", "toDecimal64(x, scale)"),
    ("cast", "cast(x AS Type) / CAST(x, 'Type')"),
    ("ifNull", "ifNull(x, alt) — alt when x is NULL"),
    ("nullIf", "nullIf(a, b) — NULL when a = b"),
    ("coalesce", "coalesce(x, ...) — first non-NULL"),
    ("if", "if(cond, then, else)"),
    ("multiIf", "multiIf(c1, v1, ..., else)"),
    ("lower", "lower(s)"),
    ("upper", "upper(s)"),
    ("concat", "concat(s1, s2, ...)"),
    ("substring", "substring(s, offset, length)"),
    ("splitByChar", "splitByChar(sep, s)"),
    ("replaceAll", "replaceAll(haystack, pattern, replacement)"),
    ("trim", "trim(s)"),
    ("position", "position(haystack, needle)"),
    ("match", "match(s, pattern) — regex test"),
    ("extractAll", "extractAll(s, pattern)"),
    ("round", "round(x, n)"),
    ("floor", "floor(x)"),
    ("ceil", "ceil(x)"),
    ("abs", "abs(x)"),
    ("greatest", "greatest(a, b, ...)"),
    ("least", "least(a, b, ...)"),
    ("rand", "rand() — random UInt32"),
    ("dictGet", "dictGet(dict, attr, key)"),
    ("bitmapCardinality", "bitmapCardinality(bitmap)"),
    ("runningDifference", "runningDifference(x)"),
    ("rowNumberInAllBlocks", "rowNumberInAllBlocks()"),
];

// --- Unit tests (no network) ------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mid_stream_exception_field_is_an_error_not_partial_rows() {
        let body = r#"{"meta":[{"name":"x","type":"UInt8"}],"data":[[1],[2],[3]],"rows":3,"exception":"Code: 241. DB::Exception: Memory limit (for query) exceeded. (MEMORY_LIMIT_EXCEEDED) (version 24.8.4.13 (official build))"}"#;
        let err = decode_json_reply(body).unwrap_err();
        assert_eq!(
            err,
            "Code: 241. DB::Exception: Memory limit (for query) exceeded. (MEMORY_LIMIT_EXCEEDED)\nSTREAMED_ROWS: 3"
        );
    }

    #[test]
    fn mid_stream_broken_json_surfaces_the_server_error() {
        let body = "{\"meta\":[{\"name\":\"x\",\"type\":\"UInt8\"}],\"data\":[[1],[2]\nCode: 159. DB::Exception: Timeout exceeded: elapsed 30.0001 seconds, maximum: 30. (TIMEOUT_EXCEEDED) (version 24.8.4.13 (official build))\n";
        let err = decode_json_reply(body).unwrap_err();
        assert!(
            err.starts_with("Code: 159. DB::Exception: Timeout exceeded"),
            "{err}"
        );
        assert!(
            err.ends_with("(TIMEOUT_EXCEEDED)\nSTREAMED_ROWS: unknown"),
            "{err}"
        );
        assert!(!err.contains("trailing characters"));
        // A plain broken body with no server error keeps serde's text.
        assert!(decode_json_reply("{\"data\":[[1]").is_err());
        // A clean reply still decodes.
        let ok = decode_json_reply(r#"{"meta":[],"data":[[1]]}"#).unwrap();
        assert_eq!(ok.data.len(), 1);
    }

    #[test]
    fn capabilities_are_honest() {
        let c = ClickhouseDriver::default().capabilities();
        assert_eq!(c.engine, Engine::Clickhouse);
        assert!(c.sql && c.joins && c.multi_statement);
        assert!(!c.transactions);
        assert!(c.cancel, "KILL QUERY WHERE query_id supported (HTTP)");
        assert!(c.explain);
    }

    fn base_cfg(port: u16) -> ResolvedConfig {
        ResolvedConfig {
            lifecycle: None,
            engine: Engine::Clickhouse,
            host: "127.0.0.1".into(),
            port,
            user: None,
            password: None,
            database: None,
            tls: types::TlsConfig::default(),
            params: json!({}),
        }
    }

    /// The MCP path's server-derived flag turns on the native read-only mode;
    /// ordinary runs never send it.
    #[test]
    fn read_only_execution_flag_selects_server_readonly_mode() {
        let plain = ClickhouseDriver::connection_from_client(
            &base_cfg(8123),
            None,
            None,
            None,
            reqwest::Client::new(),
        )
        .unwrap();
        assert!(!plain.readonly);
        let mut cfg = base_cfg(8123);
        cfg.params = json!({ "__read_only_execution": true });
        let guarded = ClickhouseDriver::connection_from_client(
            &cfg,
            None,
            None,
            None,
            reqwest::Client::new(),
        )
        .unwrap();
        assert!(guarded.readonly);
        assert!(is_readonly_setting_refused(
            "Code: 164. DB::Exception: Cannot modify 'readonly' setting in readonly mode. (READONLY)"
        ));
        assert!(!is_readonly_setting_refused(
            "Code: 164. DB::Exception: Cannot execute query in readonly mode. (READONLY)"
        ));
    }

    #[test]
    fn detects_statements_that_return_rows() {
        assert!(returns_rows("SELECT 1"));
        assert!(returns_rows("  select count() from t"));
        assert!(returns_rows("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(returns_rows("SHOW TABLES"));
        assert!(returns_rows("DESCRIBE TABLE t"));
        assert!(returns_rows("desc t"));
        assert!(returns_rows("EXPLAIN SELECT 1"));
        assert!(returns_rows("(SELECT 1)"));

        assert!(!returns_rows("INSERT INTO t VALUES (1)"));
        assert!(!returns_rows("CREATE TABLE t (a Int32) ENGINE = Memory"));
        assert!(!returns_rows("ALTER TABLE t ADD COLUMN b Int32"));
        assert!(!returns_rows("DROP TABLE t"));
        assert!(!returns_rows("  optimize table t final"));
        assert!(!returns_rows(""));
    }

    #[test]
    fn escapes_identifier_quotes() {
        assert_eq!(esc("plain"), "plain");
        assert_eq!(esc("o'brien"), "o''brien");
        assert_eq!(esc("a'b'c"), "a''b''c");
        assert_eq!(esc_ident("plain"), "plain");
        assert_eq!(esc_ident("we`ird"), "we``ird");
    }

    /// CH honours `\` escapes in string literals AND backtick idents — an
    /// unescaped trailing backslash was a quote break-out (injection).
    #[test]
    fn escapes_backslashes_in_literals_and_idents() {
        // `\'` alone would read as an ESCAPED quote; `\\''` stays inert.
        assert_eq!(esc("evil\\"), "evil\\\\");
        assert_eq!(esc("a\\'b"), "a\\\\''b");
        assert_eq!(esc_ident("t\\"), "t\\\\");
        assert_eq!(esc_ident("a\\`b"), "a\\\\``b");
    }

    #[test]
    fn transport_selected_by_port() {
        // HTTP ports → HTTP.
        assert_eq!(transport_for(&base_cfg(8123)), Transport::Http);
        assert_eq!(transport_for(&base_cfg(8443)), Transport::Http);
        assert_eq!(transport_for(&base_cfg(18123)), Transport::Http);
        // Native protocol ports → native.
        assert_eq!(transport_for(&base_cfg(9000)), Transport::Native);
        assert_eq!(transport_for(&base_cfg(9440)), Transport::Native);
        // Forwarded native ports (tunnel local forward keeps the trailing
        // 9000/9440) → still native.
        assert_eq!(transport_for(&base_cfg(19000)), Transport::Native);
        assert_eq!(transport_for(&base_cfg(19440)), Transport::Native);
        // An arbitrary local-forward port falls back to HTTP unless overridden.
        assert_eq!(transport_for(&base_cfg(34567)), Transport::Http);
    }

    #[test]
    fn native_port_recognition() {
        assert!(is_native_port(9000));
        assert!(is_native_port(9440));
        assert!(is_native_port(19000));
        assert!(is_native_port(19440));
        assert!(!is_native_port(8123));
        assert!(!is_native_port(8443));
        assert!(!is_native_port(18123));
        assert!(!is_native_port(3306));
    }

    #[test]
    fn transport_param_forces_native() {
        // An explicit transport=native param overrides the port heuristic — for
        // any forward port that doesn't carry a recognizable native suffix.
        let mut cfg = base_cfg(34567);
        cfg.params = json!({ "transport": "native" });
        assert_eq!(transport_for(&cfg), Transport::Native);

        // Case-insensitive.
        cfg.params = json!({ "transport": "Native" });
        assert_eq!(transport_for(&cfg), Transport::Native);
    }

    #[test]
    fn value_scalars_to_json() {
        use klickhouse::Value as V;
        assert_eq!(value_to_json(&V::Null), Value::Null);
        assert_eq!(value_to_json(&V::Int32(-7)), json!(-7));
        assert_eq!(value_to_json(&V::UInt64(5)), json!(5));
        assert_eq!(value_to_json(&V::Float64(1.5)), json!(1.5));
        assert_eq!(value_to_json(&V::String(b"hello".to_vec())), json!("hello"));
        // 128-bit ints render as strings (out of f64/i64 range safely).
        assert_eq!(
            value_to_json(&V::Int128(170141183460469231731687303715884105727i128)),
            json!("170141183460469231731687303715884105727")
        );
    }

    #[test]
    fn value_array_and_tuple_to_json() {
        use klickhouse::Value as V;
        let arr = V::Array(vec![V::Int32(1), V::Int32(2), V::Int32(3)]);
        assert_eq!(value_to_json(&arr), json!([1, 2, 3]));

        let tup = V::Tuple(vec![V::String(b"a".to_vec()), V::Int32(9)]);
        assert_eq!(value_to_json(&tup), json!(["a", 9]));
    }

    #[test]
    fn value_map_to_json_object() {
        use klickhouse::Value as V;
        let map = V::Map(
            vec![V::String(b"k1".to_vec()), V::String(b"k2".to_vec())],
            vec![V::Int32(1), V::Int32(2)],
        );
        assert_eq!(value_to_json(&map), json!({ "k1": 1, "k2": 2 }));
    }

    #[test]
    fn strips_surrounding_quotes() {
        assert_eq!(strip_quotes("'1.2.3.4'".to_string()), "1.2.3.4");
        assert_eq!(strip_quotes("plain".to_string()), "plain");
    }

    #[test]
    fn json_cell_to_text_unwraps_strings() {
        assert_eq!(
            json_cell_to_text(&json!("CREATE TABLE x")),
            "CREATE TABLE x"
        );
        assert_eq!(json_cell_to_text(&json!(42)), "42");
        assert_eq!(json_cell_to_text(&Value::Null), "");
    }

    // --- Streaming run reader / readonly memo / bulk graph (perf wave) ------

    /// A `FORMAT JSONCompact` reply laid out the way ClickHouse writes it.
    fn compact_reply(rows: &[&str], tail_extra: &str) -> String {
        let mut s = String::from(
            "{\n\t\"meta\":\n\t[\n\t\t{\n\t\t\t\"name\": \"n\",\n\t\t\t\"type\": \"UInt64\"\n\t\t},\n\t\t{\n\t\t\t\"name\": \"s\",\n\t\t\t\"type\": \"String\"\n\t\t}\n\t],\n\n\t\"data\":\n\t[\n",
        );
        s.push_str(
            &rows
                .iter()
                .map(|r| format!("\t\t{r}"))
                .collect::<Vec<_>>()
                .join(",\n"),
        );
        s.push_str("\n\t],\n\n\t\"rows\": 2,\n");
        s.push_str(tail_extra);
        s.push_str("\n\t\"statistics\":\n\t{\n\t\t\"elapsed\": 0.001,\n\t\t\"rows_read\": 2,\n\t\t\"bytes_read\": 16\n\t}\n}\n");
        s
    }

    fn feed_all(state: &mut CompactStream, body: &str) -> std::result::Result<(), String> {
        state.push_lines(body.as_bytes())
    }

    #[test]
    fn compact_stream_decodes_rows_meta_and_statistics() {
        let body = compact_reply(&[r#"[1, "a"]"#, r#"[2, "b,]"]"#], "");
        let mut st = CompactStream::new(Some(10));
        feed_all(&mut st, &body).unwrap();
        let raw = st.finish().unwrap();
        assert_eq!(
            raw.meta,
            vec![("n".into(), "UInt64".into()), ("s".into(), "String".into())]
        );
        assert_eq!(
            raw.data,
            vec![vec![json!(1), json!("a")], vec![json!(2), json!("b,]")]]
        );
        assert_eq!(raw.bytes_read, 16);
        assert!(raw.prepared && !raw.partial && !raw.truncated_bytes);
    }

    #[test]
    fn compact_stream_stops_at_the_row_cap() {
        let body = compact_reply(&["[1, \"a\"]", "[2, \"b\"]"], "");
        let mut st = CompactStream::new(Some(1));
        feed_all(&mut st, &body).unwrap();
        assert!(st.stopped);
        let raw = st.finish().unwrap();
        assert_eq!(raw.data.len(), 1);
        // The whole reply was in hand, so the trailer still yields statistics.
        assert!(!raw.partial && raw.prepared);
        assert_eq!(raw.bytes_read, 16);

        // Hanging up inside the rows: rows kept, statistics unknown.
        let cut = &body[..body.rfind("\n\t],").unwrap()];
        let mut st = CompactStream::new(Some(1));
        feed_all(&mut st, cut).unwrap();
        let raw = st.finish().unwrap();
        assert_eq!(raw.data.len(), 1);
        assert!(raw.partial);
    }

    #[test]
    fn compact_stream_stops_at_the_byte_budget_keeping_at_least_one_row() {
        let body = compact_reply(&["[1, \"aaaaaaaaaa\"]", "[2, \"bbbbbbbbbb\"]"], "");
        let mut st = CompactStream::new(Some(10));
        st.budget = types::ByteBudget::new(5);
        feed_all(&mut st, &body).unwrap();
        let raw = st.finish().unwrap();
        assert_eq!(raw.data.len(), 1);
        assert!(raw.truncated_bytes && !raw.partial);
    }

    #[test]
    fn compact_stream_in_band_exception_is_an_error() {
        let body = compact_reply(
            &["[1, \"a\"]"],
            "\t\"exception\": \"Code: 241. DB::Exception: Memory limit exceeded. (MEMORY_LIMIT_EXCEEDED)\",",
        );
        let mut st = CompactStream::new(Some(10));
        feed_all(&mut st, &body).unwrap();
        let err = st.finish().unwrap_err();
        assert!(err.contains("MEMORY_LIMIT_EXCEEDED"), "{err}");
        assert!(err.ends_with("STREAMED_ROWS: 1"), "{err}");
    }

    #[test]
    fn compact_stream_raw_trailing_exception_mid_rows_is_an_error() {
        let mut body = compact_reply(&["[1, \"a\"]"], "");
        body.truncate(body.rfind("\n\t],").unwrap());
        body.push_str(",\nCode: 241. DB::Exception: Memory limit exceeded. (MEMORY_LIMIT_EXCEEDED) (version 24.8)\n");
        let mut st = CompactStream::new(Some(10));
        let err = feed_all(&mut st, &body).unwrap_err();
        assert!(err.contains("MEMORY_LIMIT_EXCEEDED"), "{err}");
    }

    #[test]
    fn compact_stream_falls_back_to_whole_body_for_single_line_json() {
        let body = r#"{"meta":[{"name":"x","type":"UInt8"}],"data":[[1],[2]],"rows":2,"statistics":{"bytes_read":7}}"#;
        let mut st = CompactStream::new(Some(10));
        feed_all(&mut st, body).unwrap();
        let raw = st.finish().unwrap();
        assert_eq!(raw.data.len(), 2);
        assert_eq!(raw.bytes_read, 7);
        // The whole-body path leaves capping/budgeting to `exec_ch_read`.
        assert!(!raw.prepared);
    }

    #[test]
    fn single_select_counts_select_keywords() {
        assert!(single_select("SELECT * FROM t FORMAT JSON"));
        assert!(single_select("select 1 settings max_threads = 1"));
        assert!(!single_select("SELECT 1 UNION ALL SELECT 2"));
        assert!(!single_select(
            "SELECT * FROM t WHERE id IN (SELECT id FROM u)"
        ));
        assert!(!single_select("SHOW TABLES"));
        assert!(single_select("SELECT selected_at FROM t"));
    }

    #[test]
    fn bulk_graph_groups_columns_and_flags_truncation() {
        let tables = RawRows {
            data: vec![
                vec![json!("a"), json!("MergeTree")],
                vec![json!("b_mv"), json!("MaterializedView")],
                vec![json!("c"), json!("MergeTree")],
            ],
            ..RawRows::default()
        };
        let columns = RawRows {
            data: vec![
                vec![json!("a"), json!("id"), json!("UInt64"), json!(1)],
                vec![
                    json!("a"),
                    json!("note"),
                    json!("Nullable(String)"),
                    json!(0),
                ],
                vec![json!("b_mv"), json!("x"), json!("Int32"), json!("0")],
            ],
            ..RawRows::default()
        };
        let g = build_bulk_graph("shop", &tables, &columns, 2);
        assert!(g.truncated);
        assert_eq!(g.tables.len(), 2);
        assert_eq!(g.tables[0].id, "db:shop/table:a");
        assert_eq!(g.tables[0].columns.len(), 2);
        assert!(g.tables[0].columns[0].primary_key);
        assert!(g.tables[0].columns[1].nullable && !g.tables[0].columns[1].primary_key);
        assert_eq!(g.tables[1].kind, NodeKind::View);
        assert!(g.edges.is_empty());
        let g = build_bulk_graph("shop", &tables, &columns, 3);
        assert!(!g.truncated);
        assert!(g.tables[2].columns.is_empty());
    }

    /// One request seen by [`FakeCh`]: (request target, body).
    type Seen = Arc<std::sync::Mutex<Vec<(String, String)>>>;

    /// Minimal HTTP/1.1 server standing in for ClickHouse: records every
    /// request and hands the socket to `respond` to write the reply.
    async fn fake_ch<F, Fut>(respond: F) -> (std::net::SocketAddr, Seen)
    where
        F: Fn(String, String, tokio::net::TcpStream) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: Seen = Arc::default();
        let seen2 = Arc::clone(&seen);
        let respond = Arc::new(respond);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let seen = Arc::clone(&seen2);
                let respond = Arc::clone(&respond);
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    let head_end = loop {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let len = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    while buf.len() < head_end + len {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let target = head.split_whitespace().nth(1).unwrap_or("").to_string();
                    let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                    seen.lock().unwrap().push((target.clone(), body.clone()));
                    respond(target, body, sock).await;
                });
            }
        });
        (addr, seen)
    }

    async fn reply(mut sock: tokio::net::TcpStream, status: &str, body: &str) {
        use tokio::io::AsyncWriteExt;
        let msg = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(msg.as_bytes()).await;
        let _ = sock.shutdown().await;
    }

    fn test_conn(addr: std::net::SocketAddr) -> Conn {
        Conn {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            base: format!("http://{addr}/"),
            headers: HeaderMap::new(),
            timezone: None,
            database: None,
            query_id: Some("otto-q1".into()),
            timeout_secs: None,
            readonly: false,
            readonly_refused: Arc::new(AtomicBool::new(false)),
            result_row_cap: None,
        }
    }

    /// DB-01: a ~200 MB reply resolves to `row_cap` rows, the reader hangs up
    /// long before the server finishes writing, and the query is KILLed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn capped_run_reader_stops_early_on_a_huge_reply_and_kills_the_query() {
        use std::sync::atomic::AtomicUsize;
        use tokio::io::AsyncWriteExt;
        const TOTAL_ROWS: usize = 200_000;
        let written = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let (w, f) = (Arc::clone(&written), Arc::clone(&finished));
        let (addr, seen) = fake_ch(move |_target, body, mut sock| {
            let (written, finished) = (Arc::clone(&w), Arc::clone(&f));
            async move {
                if body.starts_with("KILL QUERY") {
                    return reply(sock, "200 OK", "").await;
                }
                let head = compact_reply(&[], "");
                let head = &head[..head.rfind("\t[\n").unwrap() + 3];
                let pad = "x".repeat(1000);
                let mut out = format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{head}");
                for i in 0..TOTAL_ROWS {
                    out.push_str(&format!("\t\t[{i}, \"{pad}\"],\n"));
                    if out.len() >= 64 * 1024 {
                        if sock.write_all(out.as_bytes()).await.is_err() {
                            break;
                        }
                        out.clear();
                    }
                    written.store(i + 1, Ordering::Relaxed);
                }
                finished.store(true, Ordering::Relaxed);
            }
        })
        .await;

        let conn = test_conn(addr);
        let raw = conn
            .query_json_capped("SELECT * FROM big", 101)
            .await
            .unwrap();
        assert_eq!(raw.data.len(), 101);
        assert!(raw.prepared && raw.partial && !raw.truncated_bytes);
        assert_eq!(raw.data[100][0], json!(100));

        // The server's writer sees the hang-up and stops well short of the end.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !finished.load(Ordering::Relaxed) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let rows_written = written.load(Ordering::Relaxed);
        assert!(
            rows_written < TOTAL_ROWS / 4,
            "reader consumed too much: server wrote {rows_written} rows"
        );
        // …and the server-side query is killed by its query_id.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let killed = seen
                .lock()
                .unwrap()
                .iter()
                .any(|(_, b)| b == "KILL QUERY WHERE query_id = 'otto-q1'");
            if killed {
                break;
            }
            assert!(Instant::now() < deadline, "no KILL QUERY was sent");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// DB-01 server-side cap: `max_result_rows` + `result_overflow_mode=break`
    /// ride on the request only when `result_row_cap` is set.
    #[tokio::test]
    async fn server_row_cap_is_sent_as_request_settings() {
        let body = compact_reply(&["[1, \"a\"]"], "");
        let (addr, seen) = fake_ch(move |_t, _b, sock| {
            let body = body.clone();
            async move { reply(sock, "200 OK", &body).await }
        })
        .await;
        let mut conn = test_conn(addr);
        conn.query_json_capped("SELECT 1", 5).await.unwrap();
        conn.result_row_cap = Some(5);
        let raw = conn.query_json_capped("SELECT 1", 5).await.unwrap();
        assert_eq!(raw.data.len(), 1);
        assert_eq!(raw.bytes_read, 16);
        let seen = seen.lock().unwrap();
        assert!(!seen[0].0.contains("max_result_rows"), "{}", seen[0].0);
        assert!(seen[1].0.contains("max_result_rows=5"), "{}", seen[1].0);
        assert!(
            seen[1].0.contains("result_overflow_mode=break"),
            "{}",
            seen[1].0
        );
    }

    /// DB2-05: a zstd body decodes chunk by chunk to exactly the original bytes
    /// however the network splits it; identity passes through; an encoding we
    /// never asked for is a clear error, not garbage rows.
    #[test]
    fn zstd_body_decodes_across_arbitrary_chunk_splits() {
        let rows: Vec<String> = (0..5_000).map(|i| format!("[{i}, \"row {i}\"]")).collect();
        let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
        let plain = compact_reply(&refs, "");
        let packed = zstd::stream::encode_all(plain.as_bytes(), 3).unwrap();
        assert!(packed.len() * 5 < plain.len(), "JSONCompact compresses ≥5×");
        for split in [1usize, 7, 4096, packed.len()] {
            let mut dec = BodyDecoder::for_encoding(Some("zstd")).unwrap();
            let mut out = Vec::new();
            for chunk in packed.chunks(split) {
                out.extend_from_slice(&dec.decode(chunk).unwrap());
            }
            assert_eq!(out, plain.as_bytes(), "split {split}");
        }
        let mut id = BodyDecoder::for_encoding(None).unwrap();
        assert!(matches!(
            id.decode(b"abc").unwrap(),
            std::borrow::Cow::Borrowed(b"abc")
        ));
        assert!(BodyDecoder::for_encoding(Some("br")).is_err());
        let mut dec = BodyDecoder::for_encoding(Some("zstd")).unwrap();
        assert!(dec.decode(b"definitely not zstd").is_err());
    }

    /// DB2-05 end to end: every request asks for compression, and a compressed
    /// JSONCompact reply streams into the same rows as a plain one.
    #[tokio::test]
    async fn compressed_reply_streams_into_rows() {
        let plain = compact_reply(&["[1, \"a\"]", "[2, \"b\"]"], "");
        let packed = zstd::stream::encode_all(plain.as_bytes(), 3).unwrap();
        let (addr, seen) = fake_ch(move |_t, _b, mut sock| {
            let packed = packed.clone();
            async move {
                use tokio::io::AsyncWriteExt;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Encoding: zstd\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    packed.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&packed).await;
                let _ = sock.shutdown().await;
            }
        })
        .await;
        let conn = test_conn(addr);
        let raw = conn.query_json_capped("SELECT 1", 10).await.unwrap();
        assert_eq!(raw.data.len(), 2);
        assert_eq!(raw.data[1][1], json!("b"));
        let seen = seen.lock().unwrap();
        assert!(
            seen[0].0.contains("enable_http_compression=1"),
            "{}",
            seen[0].0
        );
    }

    /// DB-02: the first `readonly` refusal is remembered — later requests skip
    /// the doomed attempt (2 round trips once, then 1 per request).
    #[tokio::test]
    async fn readonly_refusal_is_memoized_per_connection() {
        let (addr, seen) = fake_ch(|target, _b, sock| async move {
            if target.contains("readonly=2") {
                reply(
                    sock,
                    "500 Internal Server Error",
                    "Code: 164. DB::Exception: Cannot modify 'readonly' setting in readonly mode. (READONLY)",
                )
                .await
            } else {
                reply(sock, "200 OK", "1\n").await
            }
        })
        .await;
        let mut conn = test_conn(addr);
        conn.readonly = true;
        assert_eq!(conn.post("SELECT 1".into()).await.unwrap(), "1\n");
        assert!(conn.readonly_refused.load(Ordering::Relaxed));
        assert_eq!(conn.post("SELECT 1".into()).await.unwrap(), "1\n");
        assert_eq!(conn.post("SELECT 1".into()).await.unwrap(), "1\n");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert_eq!(
            seen.iter()
                .filter(|(t, _)| t.contains("readonly=2"))
                .count(),
            1
        );
    }

    #[test]
    fn readonly_memo_is_shared_per_cache_key() {
        let d = ClickhouseDriver::default();
        let a = d.readonly_memo("k1");
        a.store(true, Ordering::Relaxed);
        assert!(d.readonly_memo("k1").load(Ordering::Relaxed));
        assert!(!d.readonly_memo("k2").load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn evict_idle_drops_only_stale_handles() {
        let d = ClickhouseDriver::default();
        let cfg = base_cfg(8123);
        d.client(&cfg).await.unwrap();
        assert_eq!(d.evict_idle(Duration::from_secs(3600)).await, 0);
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert_eq!(d.evict_idle(Duration::from_millis(1)).await, 1);
        assert!(d.clients.get_ready(&cfg.cache_key()).is_none());
    }
}

/// Decode timing for the streamed `JSONCompact` reader (DB2-04).
#[cfg(test)]
mod perf_bench {
    use super::*;

    /// A `FORMAT JSONCompact` reply laid out the way ClickHouse writes it (one
    /// data row per line) with `rows × cols` mixed cells.
    fn compact_body(rows: usize, cols: usize) -> String {
        let mut s = String::from("{\n\t\"meta\":\n\t[\n");
        let meta: Vec<String> = (0..cols)
            .map(|c| {
                format!("\t\t{{\n\t\t\t\"name\": \"c{c}\",\n\t\t\t\"type\": \"String\"\n\t\t}}")
            })
            .collect();
        s.push_str(&meta.join(",\n"));
        s.push_str("\n\t],\n\n\t\"data\":\n\t[\n");
        for r in 0..rows {
            s.push_str("\t\t[");
            for c in 0..cols {
                if c > 0 {
                    s.push_str(", ");
                }
                match c % 3 {
                    0 => s.push_str(&(r * 31 + c).to_string()),
                    1 => s.push_str(&format!("\"customer-{r}-{c}@example.com\"")),
                    _ => s.push_str("\"2026-10-03 17:14:25\""),
                }
            }
            s.push(']');
            if r + 1 < rows {
                s.push(',');
            }
            s.push('\n');
        }
        s.push_str(&format!("\t],\n\n\t\"rows\": {rows},\n\n\t\"statistics\":\n\t{{\n\t\t\"elapsed\": 0.001,\n\t\t\"rows_read\": {rows},\n\t\t\"bytes_read\": 16\n\t}}\n}}\n"));
        s
    }

    /// Feed in 64 KiB batches of whole lines, as the network loop does.
    fn decode(body: &str, cap: Option<usize>) -> RawRows {
        let mut st = CompactStream::new(cap);
        let bytes = body.as_bytes();
        let mut start = 0;
        while start < bytes.len() {
            let mut end = (start + 64 * 1024).min(bytes.len());
            while end < bytes.len() && bytes[end - 1] != b'\n' {
                end += 1;
            }
            st.push_lines(&bytes[start..end.saturating_sub(1).max(start)])
                .unwrap();
            start = end;
        }
        st.finish().unwrap()
    }

    /// CI ceiling: 10k×30 streamed decode stays well under a few seconds in a
    /// debug build (catches an accidental quadratic re-scan of the buffer).
    #[test]
    fn compact_stream_10k_x_30_within_ceiling() {
        let body = compact_body(10_000, 30);
        let t = std::time::Instant::now();
        let raw = decode(&body, None);
        let took = t.elapsed();
        assert_eq!(raw.data.len(), 10_000);
        assert_eq!(raw.data[0].len(), 30);
        assert!(
            took < Duration::from_secs(5),
            "10k×30 JSONCompact decode took {took:?} (ceiling 5 s)"
        );
    }

    /// Bench (on demand, `-- --ignored --nocapture bench_`): 100k×30 decode,
    /// uncapped and capped at 1,001 rows (the default page + probe row).
    #[test]
    #[ignore]
    fn bench_compact_stream_100k_x_30() {
        let body = compact_body(100_000, 30);
        let t = std::time::Instant::now();
        let raw = decode(&body, None);
        eprintln!(
            "bench CH JSONCompact decode 100k×30 ({} MB): {:?}, rows {}",
            body.len() / (1024 * 1024),
            t.elapsed(),
            raw.data.len()
        );
        let t = std::time::Instant::now();
        let raw = decode(&body, Some(1_001));
        eprintln!(
            "bench CH JSONCompact decode 100k×30 capped at 1001: {:?}, rows {}",
            t.elapsed(),
            raw.data.len()
        );
    }
}
