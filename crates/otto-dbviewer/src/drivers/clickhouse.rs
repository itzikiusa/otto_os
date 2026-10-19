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
    map_arrays: ResourceCache<bool>,
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
    cells_truncated: bool,
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
    /// Unique to a submitted batch; preserves SET/USE without cross-tab state.
    session_id: Option<String>,
    map_arrays: bool,
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
    #[serde(default, deserialize_with = "deserialize_rows_preserving_maps")]
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

/// Older servers emit Map as JSON objects, including duplicate keys. Serde's
/// Value visitor overwrites those keys; retain duplicates as ordered pairs.
struct LosslessJson(Value);
impl<'de> Deserialize<'de> for LosslessJson {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = LosslessJson;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON value")
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(LosslessJson(Value::Null))
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                v: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(LosslessJson(Value::Bool(v)))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
                Ok(LosslessJson(Value::from(v)))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
                Ok(LosslessJson(Value::from(v)))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
                Ok(LosslessJson(Value::from(v)))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                v: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(LosslessJson(Value::String(v.into())))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(LosslessJson(Value::String(v)))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(LosslessJson(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(LosslessJson(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut pairs = Vec::new();
                let mut keys = std::collections::HashSet::new();
                let mut duplicate = false;
                while let Some((key, LosslessJson(value))) =
                    map.next_entry::<String, LosslessJson>()?
                {
                    duplicate |= !keys.insert(key.clone());
                    pairs.push((key, value));
                }
                Ok(LosslessJson(if duplicate {
                    Value::Array(
                        pairs
                            .into_iter()
                            .map(|(k, v)| Value::Array(vec![Value::String(k), v]))
                            .collect(),
                    )
                } else {
                    Value::Object(pairs.into_iter().collect())
                }))
            }
        }
        decoder.deserialize_any(Visitor)
    }
}
fn deserialize_rows_preserving_maps<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Vec<Vec<Value>>, D::Error> {
    Vec::<Vec<LosslessJson>>::deserialize(decoder).map(|rows| {
        rows.into_iter()
            .map(|row| row.into_iter().map(|v| v.0).collect())
            .collect()
    })
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
        conn.map_arrays = self.map_arrays.get_or_try_init(cfg.cache_key(), cfg.lifecycle.as_ref(), |_| true, async {
            // Negotiate before executing user SQL; never replay a user query
            // after an unsupported-setting error on an older server.
            let response = conn.client.post(&conn.base).headers(conn.headers.clone())
                .timeout(Duration::from_secs(5))
                .body("SELECT count() FROM system.settings WHERE name = 'output_format_json_map_as_array_of_tuples' FORMAT TSV")
                .send().await.map_err(req_err)?;
            if !response.status().is_success() { return Ok(false); }
            let bytes = response.bytes().await.map_err(req_err)?;
            Ok(bytes.as_ref() == b"1\n")
        }).await?;
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
        let base = format!("{scheme}://{}/", host_port(&url_host, cfg.port));

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
            timezone: if cfg
                .params
                .get("__ch_session_continued")
                .and_then(Value::as_bool)
                == Some(true)
            {
                None
            } else {
                timezone
            },
            database: if cfg
                .params
                .get("__ch_session_continued")
                .and_then(Value::as_bool)
                == Some(true)
            {
                None
            } else {
                active_db
                    .map(str::to_string)
                    .or_else(|| cfg.database.clone())
            },
            session_id: cfg.param_str("__ch_session"),
            map_arrays: false,
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

    /// Decode native blocks within interactive row/byte budgets.
    async fn native_query_scoped(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        row_cap: Option<usize>,
        active_db: Option<&str>,
    ) -> Result<RawRows> {
        use futures_util::StreamExt;

        // A selected database must not mutate the cached profile connection:
        // another tab may concurrently query a different database. Scoped
        // connections are operation-owned, so close/eviction cannot orphan them.
        let readonly = cfg
            .params
            .get("__read_only_execution")
            .and_then(Value::as_bool)
            == Some(true);
        // A shared native client serializes requests internally. An operation
        // cancelled while queued could otherwise execute after its caller exits.
        // Only an explicit sequential batch owns/reuses one session connection.
        let standalone = cfg.param_str("__ch_session").is_none();
        let client = if readonly
            || standalone
            || active_db.is_some_and(|db| Some(db) != cfg.database.as_deref())
        {
            let mut scoped = cfg.clone();
            scoped.database = active_db
                .map(str::to_string)
                .or_else(|| cfg.database.clone());
            Arc::new(native_connect(&scoped).await?)
        } else {
            self.native_client(cfg).await?
        };
        let mut stream = client.query_raw(sql).await.map_err(native_err)?;

        let mut meta: Vec<(String, String)> = Vec::new();
        let mut data: Vec<Vec<Value>> = Vec::new();
        let mut budget = types::ByteBudget::default();
        let mut cells_truncated = false;
        let mut truncated_bytes = false;
        let mut partial = false;
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
            if row_cap.is_some_and(|cap| data.len() >= cap) {
                partial = true;
                break;
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
            if row_cap.is_none() {
                data.extend(rows);
                continue;
            }
            for row in rows {
                let row: Vec<_> = row
                    .into_iter()
                    .map(|v| types::cap_cell_tracked(v, &mut cells_truncated))
                    .collect();
                let bytes = row.iter().map(|v| types::approx_json_len(v) + 1).sum();
                if !budget.charge(bytes) && !data.is_empty() {
                    truncated_bytes = true;
                    partial = true;
                    break;
                }
                data.push(row);
            }
            if partial {
                break;
            }
        }

        // klickhouse drops its row channel on a protocol/type decode failure
        // without forwarding that error. Never present that as an empty result.
        if !partial && client.is_closed() {
            return Err(native_closed_error());
        }
        Ok(RawRows {
            meta,
            data,
            prepared: row_cap.is_some(),
            cells_truncated,
            truncated_bytes,
            partial,
            ..RawRows::default()
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn native_query_controlled(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        row_cap: Option<usize>,
        active_db: Option<&str>,
        query_id: Option<String>,
        timeout_secs: Option<u64>,
    ) -> Result<RawRows> {
        let id = query_id.unwrap_or_else(new_query_id);
        let marked = format!("/* otto-query:{id} */\n{sql}");
        let seconds = timeout_secs.unwrap_or(DEFAULT_MAX_EXECUTION_SECS);
        match tokio::time::timeout(
            Duration::from_secs(seconds),
            self.native_query_scoped(cfg, &marked, row_cap, active_db),
        )
        .await
        {
            Ok(Ok(result)) => {
                if result.partial {
                    let _ = native_kill(cfg, &id).await;
                }
                Ok(result)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => {
                let _ = native_kill(cfg, &id).await;
                Err(types::upstream(format!(
                    "clickhouse: native query timed out after {seconds}s"
                )))
            }
        }
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
    /// params. Native uses an operation-owned connection when the selected
    /// database differs from the profile. Native timeout is client-side and
    /// cancellation identifies an unguessable query marker on a control connection.
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
    /// uses a client-side deadline followed by a server KILL on timeout.
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
                self.native_query_controlled(cfg, sql, row_cap, active_db, query_id, timeout_secs)
                    .await
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
    /// native database isolation).
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
                let raw = self
                    .native_query_controlled(cfg, sql, None, active_db, query_id, timeout_secs)
                    .await?;
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
        let mut cells_truncated = resp.cells_truncated;
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
                let row: Vec<Value> = row
                    .into_iter()
                    .map(|v| types::cap_cell_tracked(v, &mut cells_truncated))
                    .collect();
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
            cells_truncated,
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
        // HTTP uses query_id; native uses the same id in a query marker.
        let http = transport_for(cfg) == Transport::Http;
        let mut batch_cfg = cfg.clone();
        batch_cfg.database = active_db
            .map(str::to_string)
            .or_else(|| cfg.database.clone());
        batch_cfg.params["__ch_session"] = Value::String(new_query_id());
        batch_cfg.params["__ch_session_continued"] = Value::Bool(false);
        let batch_key = batch_cfg.cache_key();
        let mut results: Vec<QueryResult> = Vec::with_capacity(spans.len());
        for (index, span) in spans.iter().enumerate() {
            // Native already has a dedicated session connection. HTTP must not
            // reset USE/session_timezone on each subsequent request.
            if http {
                batch_cfg.params["__ch_session_continued"] = Value::Bool(index > 0);
            }
            let cfg = &batch_cfg;
            let stmt = span.text.as_str();
            let query_id = new_query_id();
            token.set(QueryHandle::ClickhouseQueryId(query_id.clone()));
            let outcome = if returns_rows(stmt) {
                self.exec_ch_read(
                    cfg,
                    stmt,
                    None,
                    Some(query_id),
                    timeout_secs,
                    max_rows,
                    false,
                )
                .await
            } else {
                self.exec_ch_write(cfg, stmt, None, Some(query_id), timeout_secs)
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
        drop(self.native.remove(&batch_key));
        drop(self.clients.remove(&batch_key));
        self.map_arrays.remove(&batch_key);
        self.map_arrays.remove(&batch_cfg.cache_key());
        // HTTP continued requests have a distinct client cache key.
        drop(self.clients.remove(&batch_cfg.cache_key()));
        Ok(types::fold_batch_results(results))
    }
}

/// Build a fresh `reqwest::Client` from the resolved config, materializing any
/// inline CA for custom-CA TLS. Never called directly by the driver methods —
/// they go through [`ClickhouseDriver::client`] for caching.
/// IPv6 literals need brackets in both HTTP URLs and native socket addresses.
fn host_port(host: &str, port: u16) -> String {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

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
        let body = output_format_sql(sql, "JSONCompact");
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
                // The refusal text can be forged by the statement itself
                // (`SELECT throwIf(1, 'Code: 164. … Cannot modify ''readonly''
                // setting')` is a read the classifier passes), so the message
                // alone never disables the native barrier: a FIXED probe must
                // confirm the server profile already enforces read-only before
                // the request is retried without `readonly=2` and memoised.
                if !self.profile_is_read_only().await {
                    return Err(e);
                }
                self.readonly_refused.store(true, Ordering::Relaxed);
                self.send(body, false).await
            }
            other => other,
        }
    }

    /// True when the server's own profile for this user is read-only
    /// (`getSetting('readonly') >= 1`) — probed with a fixed statement, never
    /// a user one. Any failure counts as "not read-only".
    async fn profile_is_read_only(&self) -> bool {
        let Ok(resp) = self.send(READONLY_PROBE_SQL.to_string(), false).await else {
            return false;
        };
        let Ok(text) = self.read_capped(resp).await else {
            return false;
        };
        text.trim().parse::<u8>().is_ok_and(|level| level >= 1)
    }

    /// One POST with every request setting attached; returns the response
    /// once its status is a success (an error status reads the small body
    /// and maps it to the server's message).
    async fn send(&self, body: String, readonly: bool) -> Result<reqwest::Response> {
        let mut req = compressed(self.client.post(&self.base).headers(self.headers.clone()));
        if self.map_arrays {
            req = req.query(&[
                ("output_format_json_map_as_array_of_tuples", "1"),
                ("output_format_json_named_tuples_as_objects", "0"),
            ]);
        }
        // Do not depend on server/profile defaults for browser-safe JSON numbers.
        req = req.query(&[
            ("output_format_json_quote_64bit_integers", "1"),
            ("output_format_json_quote_decimals", "1"),
        ]);
        if readonly {
            req = req.query(&[("readonly", "2")]);
        }
        if let Some(session) = &self.session_id {
            req = req.query(&[("session_id", session.as_str()), ("session_timeout", "60")]);
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
            .send_checked(output_format_sql(sql, "JSONCompact"))
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
    async fn post_stream(&self, body: String, json_array: bool) -> Result<reqwest::Response> {
        let mut req = compressed(self.client.post(&self.base).headers(self.headers.clone()));
        if self.map_arrays {
            req = req.query(&[
                ("output_format_json_map_as_array_of_tuples", "1"),
                ("output_format_json_named_tuples_as_objects", "0"),
            ]);
        }
        req = req.query(&[
            ("output_format_json_quote_64bit_integers", "1"),
            ("output_format_json_quote_decimals", "1"),
            (
                "output_format_json_array_of_rows",
                if json_array { "1" } else { "0" },
            ),
        ]);
        if self.readonly {
            req = req.query(&[("readonly", "2")]);
        }
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
    let endpoint = host_port(&cfg.host, cfg.port);

    if !use_tls {
        let client = klickhouse::Client::connect(&endpoint, options)
            .await
            .map_err(native_err)?;
        configure_native(&client, cfg).await?;
        return Ok(client);
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

    let client = klickhouse::Client::connect_tls(&endpoint, options, server_name, &connector)
        .await
        .map_err(native_err)?;
    configure_native(&client, cfg).await?;
    Ok(client)
}

async fn configure_native(client: &klickhouse::Client, cfg: &ResolvedConfig) -> Result<()> {
    let timezone = cfg
        .param_str("timezone")
        .filter(|tz| !tz.is_empty())
        .unwrap_or_else(|| "UTC".into());
    if let Err(error) = client
        .execute(format!("SET session_timezone = '{}'", esc(&timezone)))
        .await
    {
        if !matches!(
            &error,
            klickhouse::KlickhouseError::ServerException { code: 164, .. }
        ) {
            return Err(native_err(error));
        }
        // readonly=1 may reject the timezone SET. Verify its policy and
        // effective timezone on this connection before accepting its
        // stronger policy; never downgrade it to readonly=2 or ignore a mismatch.
        let mut row = client
            .query_one::<klickhouse::RawRow>(
                "SELECT toUInt8(getSetting('readonly')) AS mode, timezone() AS timezone",
            )
            .await
            .map_err(native_err)?;
        let mode: u8 = row.try_get("mode").map_err(native_err)?;
        let current_timezone: String = row.try_get("timezone").map_err(native_err)?;
        if mode == 1 && current_timezone == timezone {
            return Ok(());
        }
        return Err(native_err(error));
    }
    if cfg
        .params
        .get("__read_only_execution")
        .and_then(Value::as_bool)
        == Some(true)
    {
        // Some servers allow SET to its existing value under readonly=1, so
        // the timezone SET above may succeed. Still preserve that stronger
        // policy rather than trying to replace it with Otto's readonly=2.
        let mut row = client
            .query_one::<klickhouse::RawRow>("SELECT toUInt8(getSetting('readonly')) AS mode")
            .await
            .map_err(native_err)?;
        let mode: u8 = row.try_get("mode").map_err(native_err)?;
        if mode != 1 {
            client.execute("SET readonly=2").await.map_err(native_err)?;
        }
    }
    Ok(())
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
    cells_truncated: bool,
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
            cells_truncated: false,
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
                match serde_json::from_str::<Vec<LosslessJson>>(cells) {
                    Ok(row) => self.accept_row(row.into_iter().map(|v| v.0).collect()),
                    // A mid-stream failure with in-band exceptions off: the
                    // server appended its raw `Code: N. DB::Exception: …` text.
                    Err(e) => {
                        return Err(match crate::errors::ch_trailing_exception(line) {
                            Some(ex) => {
                                crate::errors::ch_midstream_message(&ex, Some(self.rows.len()))
                            }
                            None => format!("clickhouse: undecodable result row: {e}"),
                        });
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
        let row: Vec<Value> = row
            .into_iter()
            .map(|v| types::cap_cell_tracked(v, &mut self.cells_truncated))
            .collect();
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
                cells_truncated: self.cells_truncated,
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
                    cells_truncated: self.cells_truncated,
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
    let cols: Vec<_> = block
        .column_types
        .iter()
        .map(|(name, ty)| (block.column_data.get(name), ty))
        .collect();
    let n = (block.rows as usize).min(take);
    let mut out = Vec::with_capacity(n);
    for r in 0..n {
        out.push(
            cols.iter()
                .map(|(col, ty)| {
                    col.and_then(|c| c.get(r))
                        .map(|value| typed_value_to_json(value, ty))
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
fn native_closed_error() -> otto_core::Error {
    types::upstream("ClickHouse native connection closed before result completion (possibly an unsupported result type). Use the HTTP transport, or explicitly select toJSONString(column) for the affected column. The query was not retried; a write may already have executed.")
}

fn native_err(e: klickhouse::KlickhouseError) -> otto_core::Error {
    let mut message = crate::errors::ch_native_message(&e);
    if matches!(e, klickhouse::KlickhouseError::TypeParseError(_)) {
        message.push_str(". This native client cannot decode this ClickHouse type. Use the HTTP transport, or explicitly select toJSONString(column) for the affected column. The query was not retried.");
    }
    otto_core::Error::Upstream(message)
}

/// klickhouse generates its wire query_id internally. An unguessable leading
/// comment identifies our request without matching another tab/user's query.
/// A fresh control connection prevents cancellation queueing behind the query.
async fn native_kill(cfg: &ResolvedConfig, id: &str) -> Result<()> {
    let prefix = format!("/* otto-query:{id} */");
    let sql = format!(
        "KILL QUERY WHERE startsWith(query, '{}') AND user = currentUser() ASYNC",
        esc(&prefix)
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut control = cfg.clone();
        if let Some(params) = control.params.as_object_mut() {
            params.remove("__read_only_execution");
        }
        let client = native_connect(&control).await?;
        client.execute(sql).await.map_err(native_err)
    })
    .await
    .map_err(|_| types::upstream("clickhouse: native cancellation timed out"))?
}

// --- Native value decoding --------------------------------------------------

/// Convert a decoded native `klickhouse::Value` into a clean JSON value for the
/// grid. The mapping mirrors the HTTP `JSONCompact` shapes: ints/floats →
/// number, String/FixedString → string, Date/DateTime → ISO string, Nullable →
/// null/inner, Array/Tuple/Map → nested JSON. Exotic types fall back to the
/// value's own string form (`Display`), so nothing ever breaks the grid.
fn typed_value_to_json(value: &klickhouse::Value, ty: &klickhouse::Type) -> Value {
    use klickhouse::{Type as T, Value as V};
    match (value, ty) {
        (V::Null, _) => Value::Null,
        (_, T::Nullable(inner) | T::LowCardinality(inner)) => typed_value_to_json(value, inner),
        (V::Enum8(n), T::Enum8(entries)) => entries
            .iter()
            .find(|(_, v)| v == n)
            .map(|(label, _)| Value::String(label.clone()))
            .unwrap_or_else(|| value_to_json(value)),
        (V::Enum16(n), T::Enum16(entries)) => entries
            .iter()
            .find(|(_, v)| v == n)
            .map(|(label, _)| Value::String(label.clone()))
            .unwrap_or_else(|| value_to_json(value)),
        (V::Array(values), T::Array(inner)) => Value::Array(
            values
                .iter()
                .map(|v| typed_value_to_json(v, inner))
                .collect(),
        ),
        (V::Tuple(values), T::Tuple(types)) => Value::Array(
            values
                .iter()
                .zip(types)
                .map(|(v, t)| typed_value_to_json(v, t))
                .collect(),
        ),
        (V::Map(keys, values), T::Map(key_type, value_type)) => Value::Array(
            keys.iter()
                .zip(values)
                .map(|(key, value)| {
                    Value::Array(vec![
                        typed_value_to_json(key, key_type),
                        typed_value_to_json(value, value_type),
                    ])
                })
                .collect(),
        ),
        (V::DateTime(dt), _) => chrono::DateTime::<klickhouse::Tz>::try_from(*dt)
            .map(|t| Value::String(t.format("%Y-%m-%d %H:%M:%S").to_string()))
            .unwrap_or_else(|_| value_to_json(value)),
        (V::DateTime64(dt), T::DateTime64(precision, tz)) => {
            chrono::DateTime::<chrono::Utc>::try_from(*dt)
                .map(|t| {
                    let t = t.with_timezone(tz);
                    let mut text = t.format("%Y-%m-%d %H:%M:%S").to_string();
                    if *precision > 0 {
                        let fraction = format!("{:09}", t.timestamp_subsec_nanos());
                        text.push('.');
                        text.push_str(&fraction[..(*precision).min(9)]);
                    }
                    Value::String(text)
                })
                .unwrap_or_else(|_| value_to_json(value))
        }
        _ => value_to_json(value),
    }
}

fn value_to_json(v: &klickhouse::Value) -> Value {
    use klickhouse::Value as V;
    match v {
        V::Null => Value::Null,

        // Signed ints: keep small ones as JSON numbers; 128/256-bit can exceed
        // f64/i64 precision, so render them as strings to avoid silent loss.
        V::Int8(x) => json_num_i(*x as i64),
        V::Int16(x) => json_num_i(*x as i64),
        V::Int32(x) => json_num_i(*x as i64),
        V::Int64(x) => Value::String(x.to_string()),
        V::Int128(x) => Value::String(x.to_string()),
        V::Int256(_) => Value::String(v.to_string()),

        // Unsigned ints.
        V::UInt8(x) => json_num_u(*x as u64),
        V::UInt16(x) => json_num_u(*x as u64),
        V::UInt32(x) => json_num_u(*x as u64),
        V::UInt64(x) => Value::String(x.to_string()),
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
///
/// Matched on the READONLY exception code (164) together with its message —
/// but a statement can still forge both, so the caller confirms with
/// [`READONLY_PROBE_SQL`] before trusting it.
fn is_readonly_setting_refused(message: &str) -> bool {
    message.contains("Code: 164.") && message.contains("Cannot modify 'readonly' setting")
}

/// The fixed statement that confirms a server profile is already read-only.
const READONLY_PROBE_SQL: &str = "SELECT getSetting('readonly') FORMAT TSV";

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

/// Replace an explicit output FORMAT while preserving the SQL and SETTINGS.
/// Input formats on INSERT never enter this helper. Token locations avoid
/// mistaking a literal/comment/quoted identifier for an output clause.
fn output_format_sql(sql: &str, format: &str) -> String {
    use sqlparser::{
        dialect::ClickHouseDialect,
        tokenizer::{Token, Tokenizer},
    };
    let sql = sql.trim_end().trim_end_matches(';');
    if let Ok(tokens) = Tokenizer::new(&ClickHouseDialect {}, sql).tokenize_with_location() {
        let significant: Vec<_> = tokens
            .iter()
            .filter(|t| !matches!(t.token, Token::Whitespace(_)))
            .collect();
        let mut depth = 0usize;
        for (i, token) in significant.iter().enumerate() {
            match &token.token {
                Token::LParen => depth += 1,
                Token::RParen => depth = depth.saturating_sub(1),
                Token::Word(w)
                    if depth == 0
                        && w.quote_style.is_none()
                        && w.value.eq_ignore_ascii_case("FORMAT") =>
                {
                    let Some(name) = significant.get(i + 1) else {
                        continue;
                    };
                    if !matches!(&name.token, Token::Word(w) if w.quote_style.is_none()) {
                        continue;
                    }
                    // FORMAT follows a complete expression/clause, never an
                    // operator introducing a column named `format`.
                    if i == 0
                        || !matches!(
                            &significant[i - 1].token,
                            Token::Word(_)
                                | Token::RParen
                                | Token::RBracket
                                | Token::Number(..)
                                | Token::SingleQuotedString(_)
                        )
                    {
                        continue;
                    }
                    // FORMAT is a suffix clause; reject expression/alias uses.
                    if i > 0
                        && matches!(
                            &significant[i - 1].token,
                            Token::Period
                                | Token::Comma
                                | Token::Word(sqlparser::tokenizer::Word {
                                    keyword: sqlparser::keywords::Keyword::AS
                                        | sqlparser::keywords::Keyword::SELECT
                                        | sqlparser::keywords::Keyword::FROM
                                        | sqlparser::keywords::Keyword::JOIN,
                                    ..
                                })
                        )
                    {
                        continue;
                    }
                    if significant.get(i + 2).is_some_and(|next| {
                        !matches!(
                        &next.token, Token::Word(w) if w.value.eq_ignore_ascii_case("SETTINGS"))
                    }) {
                        continue;
                    }
                    let mut line = 1u64;
                    let mut column = 1u64;
                    for (offset, ch) in sql.char_indices() {
                        if line == name.span.start.line && column == name.span.start.column {
                            let end = offset + name.token.to_string().len();
                            return format!("{}{format}{}", &sql[..offset], &sql[end..]);
                        }
                        if ch == '\n' {
                            line += 1;
                            column = 1;
                        } else {
                            column += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    format!("{sql}\nFORMAT {format}")
}

/// Does this statement return a rowset? (first keyword decides).
fn returns_rows(sql: &str) -> bool {
    use sqlparser::{
        dialect::ClickHouseDialect,
        tokenizer::{Token, Tokenizer},
    };
    let Ok(tokens) = Tokenizer::new(&ClickHouseDialect {}, sql).tokenize() else {
        return false;
    };
    let first = tokens
        .iter()
        .find(|t| !matches!(t, Token::Whitespace(_) | Token::LParen));
    matches!(first, Some(Token::Word(w)) if w.quote_style.is_none() && matches!(
        w.value.to_ascii_uppercase().as_str(),
        "SELECT" | "SHOW" | "DESC" | "DESCRIBE" | "EXPLAIN" | "WITH" |
        "EXISTS" | "CHECK" | "KILL" | "BACKUP" | "RESTORE"
    ))
}

/// System databases that exist on every server; keep them but sort last.
fn is_system_db(name: &str) -> bool {
    matches!(name, "system" | "INFORMATION_SCHEMA" | "information_schema")
}

// Per-cell size capping (≈1 MiB, truncation marker) now lives in
// [`crate::types::cap_cell`], shared by the MySQL and Postgres drivers too so
// oversized cells behave identically across engines.

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
            // HTTP query ids and native query markers support server-side KILL
            // through independent control requests on both transports.
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
                collation: None,
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
        self.map_arrays.remove(cache_key);
        self.completions.invalidate(cache_key);
        Some(std::sync::Arc::new(captured))
    }

    async fn close(&self, cache_key: &str) {
        self.clients.remove(cache_key);
        self.map_arrays.remove(cache_key);
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
        // Same 5-minute tick: expired completion snapshots go too (DB2-07).
        self.completions.sweep();
        let http = self.clients.take_idle(idle);
        let native = self.native.take_idle(idle);
        let mut memo = self
            .readonly_refused
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for (key, _) in &http {
            self.map_arrays.remove(key);
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
        // HTTP uses query_id; native uses the same id in a query marker.

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
        // native transport carries it in an unguessable leading comment.
        let query_id = new_query_id();
        token.set(QueryHandle::ClickhouseQueryId(query_id.clone()));

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
        if transport_for(cfg) == Transport::Native {
            return native_kill(cfg, qid).await;
        }
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
    /// server-side `INTO OUTFILE` that would write to the tunnel host). Capped
    /// exports decode streaming rows to enforce the cap even with an existing
    /// SQL LIMIT; native exports likewise stream and format result blocks.
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

        // JSONEachRow can stream a single array directly with
        // output_format_json_array_of_rows: no whole-result decode or byte cap.
        if transport_for(cfg) == Transport::Http {
            let ch_format = format.clickhouse_format().unwrap_or("JSONEachRow");
            return self
                .export_http_format(cfg, sql, active_db, ch_format, max_rows, format, w)
                .await;
        }

        self.export_native(cfg, sql, active_db, format, max_rows, w)
            .await
    }
}

impl ClickhouseDriver {
    async fn export_native(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        format: ExportFormat,
        max_rows: Option<usize>,
        w: Box<dyn std::io::Write + Send>,
    ) -> Result<ExportCounts> {
        use futures_util::StreamExt;
        let mut scoped = cfg.clone();
        scoped.database = active_db
            .map(str::to_string)
            .or_else(|| cfg.database.clone());
        let client = native_connect(&scoped).await?;
        let id = new_query_id();
        let query = format!("/* otto-query:{id} */\n{}", capped_sql(sql, max_rows));
        let mut stream = client.query_raw(query).await.map_err(native_err)?;
        let mut sink = ExportSink::new(w, format);
        let mut header = false;
        let mut count = 0usize;
        while let Some(block) = stream.next().await {
            let block = block.map_err(native_err)?;
            let remaining = max_rows.unwrap_or(usize::MAX).saturating_sub(count);
            let had_header = header;
            let (next_sink, wrote_header, rows) =
                tokio::task::spawn_blocking(move || -> Result<_> {
                    let mut wrote_header = had_header;
                    if !had_header && !block.column_types.is_empty() {
                        let columns: Vec<_> = block
                            .column_types
                            .iter()
                            .map(|(name, ty)| Column::typed(name, ty.to_string()))
                            .collect();
                        sink.write_header(&columns).map_err(types::upstream)?;
                        wrote_header = true;
                    }
                    let rows = decode_native_block(&block, remaining);
                    for row in &rows {
                        sink.write_row(row).map_err(types::upstream)?;
                    }
                    Ok((sink, wrote_header, rows.len()))
                })
                .await
                .map_err(types::upstream)??;
            sink = next_sink;
            header = wrote_header;
            count += rows;
            if max_rows.is_some_and(|cap| count >= cap) {
                drop(stream);
                let _ = native_kill(cfg, &id).await;
                break;
            }
        }
        if !max_rows.is_some_and(|cap| count >= cap) && client.is_closed() {
            return Err(native_closed_error());
        }
        tokio::task::spawn_blocking(move || sink.finish().map_err(types::upstream))
            .await
            .map_err(types::upstream)?
    }

    /// Decode complete JSON row lines, retaining at most one incomplete line.
    /// This client-side cap does not alter nested queries or explicit SQL LIMITs.
    async fn export_http_capped(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
        format: ExportFormat,
        cap: usize,
        w: Box<dyn std::io::Write + Send>,
    ) -> Result<ExportCounts> {
        use futures_util::StreamExt as _;
        let id = new_query_id();
        let conn = self.connect_id(cfg, active_db, Some(id)).await?;
        let response = conn
            .post_stream(
                output_format_sql(sql, "JSONCompactEachRowWithNamesAndTypes"),
                false,
            )
            .await?;
        let mut decoder = BodyDecoder::for_response(&response)?;
        let mut stream = response.bytes_stream();
        let mut pending = Vec::new();
        let mut names: Option<Vec<String>> = None;
        let mut header = false;
        let mut rows = 0usize;
        let mut sink = ExportSink::new(w, format);
        let result: Result<()> = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(req_err)?;
                let keep_reading = decoder.decode_to(&chunk, |chunk| {
                    pending.extend_from_slice(chunk);
                    let mut consumed = 0;
                    while let Some(end) = pending[consumed..].iter().position(|byte| *byte == b'\n')
                    {
                        let end = consumed + end;
                        let line = &pending[consumed..end];
                        consumed = end + 1;
                        if line.is_empty() {
                            continue;
                        }
                        match names.as_ref() {
                            None => {
                                names =
                                    Some(serde_json::from_slice(line).map_err(types::upstream)?);
                            }
                            Some(names) if !header => {
                                let types: Vec<String> =
                                    serde_json::from_slice(line).map_err(types::upstream)?;
                                let columns: Vec<_> = names
                                    .iter()
                                    .zip(types)
                                    .map(|(name, ty)| Column::typed(name, ty))
                                    .collect();
                                sink.write_header(&columns).map_err(types::upstream)?;
                                header = true;
                            }
                            Some(_) => {
                                let row: Vec<LosslessJson> =
                                    serde_json::from_slice(line).map_err(types::upstream)?;
                                sink.write_row(&row.into_iter().map(|v| v.0).collect::<Vec<_>>())
                                    .map_err(types::upstream)?;
                                rows += 1;
                            }
                        }
                        if header && rows >= cap {
                            return Ok(false);
                        }
                    }
                    pending.drain(..consumed);
                    if pending.len() > HTTP_RESPONSE_BYTE_CAP {
                        return Err(types::upstream(
                            "ClickHouse export row exceeds response byte limit",
                        ));
                    }
                    Ok(true)
                })?;
                if !keep_reading {
                    return Ok(());
                }
            }
            if !pending.iter().all(u8::is_ascii_whitespace) {
                return Err(types::upstream(
                    "ClickHouse export ended with an incomplete row",
                ));
            }
            if !header {
                return Err(types::upstream(
                    "ClickHouse export did not return column metadata",
                ));
            }
            Ok(())
        }
        .await;
        drop(stream);
        if rows >= cap || result.is_err() {
            conn.spawn_kill();
        }
        result?;
        sink.finish().map_err(types::upstream)
    }

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

        if let Some(cap) = max_rows {
            return self
                .export_http_capped(cfg, sql, active_db, format, cap, w)
                .await;
        }
        let conn = self.connect_id(cfg, active_db, None).await?;
        let body = output_format_sql(sql, ch_format);
        let resp = conn.post_stream(body, format == ExportFormat::Json).await?;

        let mut sink = ExportSink::new(w, ExportFormat::Ndjson);
        let mut body = BodyDecoder::for_response(&resp)?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(req_err)?;
            body.decode_to(&chunk, |bytes| {
                sink.write_raw(bytes).map_err(types::upstream)?;
                Ok(true)
            })?;
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

    /// Deliver bounded decompressed windows; exports never allocate the full
    /// expansion of a highly compressible network chunk.
    fn decode_to(
        &mut self,
        chunk: &[u8],
        mut consume: impl FnMut(&[u8]) -> Result<bool>,
    ) -> Result<bool> {
        use zstd::stream::raw::{InBuffer, Operation as _, OutBuffer};
        let (dec, window) = match self {
            Self::Identity => return consume(chunk),
            Self::Zstd { dec, window } => (dec, window),
        };
        let mut input = InBuffer::around(chunk);
        loop {
            let mut output = OutBuffer::around(&mut window[..]);
            dec.run(&mut input, &mut output).map_err(|e| {
                otto_core::Error::Upstream(format!("clickhouse: corrupt zstd response: {e}"))
            })?;
            let n = output.pos();
            if n > 0 && !consume(&window[..n])? {
                return Ok(false);
            }
            if input.pos() == chunk.len() && n < window.len() {
                break;
            }
        }
        Ok(true)
    }

    /// Interactive replies retain a strict per-chunk allocation bound.
    fn decode<'a>(&mut self, chunk: &'a [u8]) -> Result<std::borrow::Cow<'a, [u8]>> {
        if matches!(self, Self::Identity) {
            return Ok(std::borrow::Cow::Borrowed(chunk));
        }
        let mut out = Vec::with_capacity(chunk.len().saturating_mul(4));
        self.decode_to(chunk, |bytes| {
            if out.len().saturating_add(bytes.len()) > ZSTD_CHUNK_DECODE_CAP {
                return Err(response_cap_error());
            }
            out.extend_from_slice(bytes);
            Ok(true)
        })?;
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
mod tests;

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
            "bench CH JSONCompact decode 100k×30 ({} MB): {:?}, rows {} (byte budget hit: {})",
            body.len() / (1024 * 1024),
            t.elapsed(),
            raw.data.len(),
            raw.truncated_bytes
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
