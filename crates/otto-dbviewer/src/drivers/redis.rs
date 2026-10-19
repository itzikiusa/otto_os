//! Redis driver (also AWS ElastiCache — same protocol; TLS via `rediss://`).
//!
//! Connects with the `redis` crate (tokio + rustls): AUTH (user+password /
//! password-only), SELECT db, lazy `SCAN` (never `KEYS *`) for the tree,
//! type/TTL/length for object detail, a raw-command console for `run`, and a
//! command-table + live-key-prefix completion source.
//!
//! Connections are cached per [`ResolvedConfig::cache_key`] as a
//! [`transport::Connection`], which is cheaply cloneable and
//! reconnects on the next acquisition after retirement. AUTH is reused across
//! healthy calls. A shared operation gate bounds entire pipeline replies. No
//! failed command is automatically replayed.

use std::time::{Duration, Instant};

use crate::resource_cache::ResourceCache;
use async_trait::async_trait;
use otto_core::Result;
mod framing;
mod tls;
mod transport;
use redis::{RedisConnectionInfo, Value as RedisValue};
use serde_json::{json, Value as JsonValue};
use transport::Connection as BoundedConnection;

use crate::driver::Driver;
use crate::types::{
    self, Capabilities, Column, CompletionContext, CompletionItem, CompletionKind,
    CompletionResponse, Engine, NodeKind, NodePath, ObjectDetail, QueryRequest, QueryResult,
    ResolvedConfig, SchemaNode, TestResult,
};

/// Cap on keys sampled during a keyspace SCAN used to derive namespace groups,
/// so the tree stays responsive on huge databases.
const SCAN_KEY_CAP: usize = 3000;
/// Cap on individual key nodes rendered in one listing (filtered or namespace
/// expansion). Beyond this the listing is truncated with a hint to refine.
const KEY_LIST_CAP: usize = 500;
/// Hard cap on SCAN round-trips so a sparse filter never traverses an entire
/// multi-hundred-thousand-key database before giving up.
const SCAN_MAX_ROUNDS: usize = 80;
/// COUNT hint for each SCAN round-trip.
const SCAN_COUNT: usize = 200;
/// Larger COUNT for filtered scans (fewer round-trips to gather matches).
const SCAN_COUNT_FILTER: usize = 500;
/// Cap on collection elements shown in an object-detail value preview.
const PREVIEW_LIMIT: isize = 50;
/// Cap on rows a console `run` returns when the request doesn't set `max_rows`
/// (matches the SQL drivers' default page).
const DEFAULT_MAX_ROWS: usize = 1000;
/// Byte cap for a string-value preview (`GETRANGE`), matching the grid's
/// per-cell cap so a multi-hundred-MB string never crosses the wire whole.
const PREVIEW_STRING_BYTES: isize = 1_048_575;

/// Redis driver. Caches one [`BoundedConnection`] per [`ResolvedConfig::cache_key`].
/// `Mutex<HashMap>` is `Default`-constructible, so `#[derive(Default)]` (used
/// by the registry) still works.
#[derive(Default)]
pub struct RedisDriver {
    clients: ResourceCache<BoundedConnection>,
}

#[async_trait]
impl Driver for RedisDriver {
    fn engine(&self) -> Engine {
        Engine::Redis
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            engine: Engine::Redis,
            sql: false,
            joins: false,
            transactions: false,
            multi_statement: true,
            // No server-side per-command cancel; Stop is client-side only.
            cancel: false,
            // Redis has no query-plan surface — the Explain button is hidden.
            explain: false,
            default_port: 6379,
            schema_levels: vec!["Database".into(), "Namespace".into(), "Key".into()],
            query_language: "redis".into(),
        }
    }

    async fn test(&self, cfg: &ResolvedConfig) -> Result<TestResult> {
        let started = Instant::now();
        // Connect + PING; report ok:false (don't Err) on any failure so the UI
        // can show the reason inline.
        let mut conn = match self.connect(cfg, default_db(cfg)).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(TestResult {
                    ok: false,
                    latency_ms: Some(started.elapsed().as_millis() as u64),
                    message: e.to_string(),
                    server_version: None,
                });
            }
        };

        let pong: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
        if let Err(e) = pong {
            return Ok(TestResult {
                ok: false,
                latency_ms: Some(started.elapsed().as_millis() as u64),
                message: e.to_string(),
                server_version: None,
            });
        }

        // INFO server → redis_version.
        let server_version = match redis::cmd("INFO")
            .arg("server")
            .query_async::<String>(&mut conn)
            .await
        {
            Ok(info) => info_field(&info, "redis_version"),
            Err(_) => None,
        };

        Ok(TestResult {
            ok: true,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            message: "Connected".into(),
            server_version,
        })
    }

    async fn schema_root(&self, cfg: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
        let mut conn = self.connect(cfg, default_db(cfg)).await?;
        let info: String = redis::cmd("INFO")
            .arg("keyspace")
            .query_async(&mut conn)
            .await
            .map_err(types::upstream)?;

        // Lines look like `db0:keys=11,expires=0,avg_ttl=0`.
        let mut nodes = Vec::new();
        let mut saw_db0 = false;
        for line in info.lines() {
            let line = line.trim();
            if !line.starts_with("db") {
                continue;
            }
            let Some((db_part, stats)) = line.split_once(':') else {
                continue;
            };
            let Ok(index) = db_part.trim_start_matches("db").parse::<u32>() else {
                continue;
            };
            let keys = keyspace_keys(stats).unwrap_or(0);
            if index == 0 {
                saw_db0 = true;
            }
            nodes.push(
                SchemaNode::new(
                    format!("kdb:{index}"),
                    format!("db{index}"),
                    NodeKind::Keyspace,
                )
                .with_detail(format!("{keys} keys"))
                .expandable(),
            );
        }

        // Always surface db0 even when it has no keys (it's the default target).
        if !saw_db0 {
            nodes.insert(
                0,
                SchemaNode::new("kdb:0", "db0", NodeKind::Keyspace)
                    .with_detail("0 keys")
                    .expandable(),
            );
        }

        Ok(nodes)
    }

    async fn schema_children(
        &self,
        cfg: &ResolvedConfig,
        parent: &NodePath,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let db = parent
            .get("kdb")
            .and_then(parse_db_index)
            .ok_or_else(|| types::invalid("redis: expected a keyspace node (kdb:<n>)"))?;

        // A connection pinned to this keyspace at handshake — no SELECT races.
        let mut conn = self.connect(cfg, db).await?;

        match (parent.get("ns"), filter) {
            // Children of a namespace: keys under `<prefix>:`, optionally narrowed
            // further by the prefix filter. Flat, capped, type looked up in bulk.
            (Some(ns), f) => {
                let pattern = match f {
                    Some(f) => format!("{}:{}*", glob_escape(ns), glob_escape(f)),
                    None => format!("{}:*", glob_escape(ns)),
                };
                let scan = scan_keys(&mut conn, Some(&pattern), KEY_LIST_CAP).await?;
                build_key_nodes(&mut conn, db, scan).await
            }
            // Keyspace WITH a prefix filter: flat list of matching keys
            // (`SCAN MATCH <filter>*`) — no namespace grouping, capped + bulk type.
            (None, Some(f)) => {
                let pattern = format!("{}*", glob_escape(f));
                let scan = scan_keys(&mut conn, Some(&pattern), KEY_LIST_CAP).await?;
                build_key_nodes(&mut conn, db, scan).await
            }
            // Keyspace overview (no filter): a sampled SCAN grouped into namespaces
            // (by the substring before the first `:`) plus any bare keys.
            (None, None) => {
                let scan = scan_keys(&mut conn, None, SCAN_KEY_CAP).await?;

                use std::collections::BTreeMap;
                let mut namespaces: BTreeMap<String, usize> = BTreeMap::new();
                let mut bare: Vec<String> = Vec::new();
                for key in &scan.keys {
                    match key.split_once(':') {
                        Some((prefix, _)) => {
                            *namespaces.entry(prefix.to_string()).or_insert(0) += 1;
                        }
                        None => bare.push(key.clone()),
                    }
                }

                let mut nodes = Vec::with_capacity(namespaces.len() + bare.len() + 1);
                for (prefix, count) in namespaces {
                    // Count is from the sample; mark it approximate when truncated.
                    let detail = if scan.more {
                        format!("{count}+")
                    } else {
                        count.to_string()
                    };
                    nodes.push(
                        SchemaNode::new(
                            format!("kdb:{db}/ns:{prefix}"),
                            format!("{prefix}:*"),
                            NodeKind::KeyNamespace,
                        )
                        .with_detail(detail)
                        .expandable(),
                    );
                }
                bare.sort();
                bare.truncate(KEY_LIST_CAP);
                let bare_scan = ScanOutcome {
                    more: scan.more,
                    keys: bare,
                };
                nodes.extend(build_key_nodes(&mut conn, db, bare_scan).await?);
                if scan.more {
                    nodes.push(truncation_hint(
                        db,
                        "Showing a sample — type a prefix to filter",
                    ));
                }
                Ok(nodes)
            }
        }
    }

    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        let db = path
            .get("kdb")
            .and_then(parse_db_index)
            .ok_or_else(|| types::invalid("redis: object detail requires a keyspace (kdb:<n>)"))?;
        let key = path
            .get("key")
            .ok_or_else(|| types::invalid("redis: object detail requires a key (key:<k>)"))?
            .to_string();

        let mut conn = self.connect(cfg, db).await?;

        // Trip 1 (DB-08): TYPE + TTL + OBJECT ENCODING pipelined. Trip 2: the
        // type-dependent length + preview, also pipelined where possible.
        let (ty, ttl, encoding) = key_header(&mut conn, &key).await;
        conn.ensure_alive()?;
        let (length, preview) = length_and_preview(&mut conn, &key, &ty).await;
        conn.ensure_alive()?;

        let mut detail = ObjectDetail::new(key, NodeKind::Key);
        detail.extra = json!({
            "type": ty,
            "ttl": ttl,
            "encoding": encoding,
            "length": length,
            "preview": preview,
        });
        Ok(detail)
    }

    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        // Parse every command line upfront (honouring single/double quotes,
        // `\xHH` binary escapes, and inline `#` comments) and validate against
        // continuous-stream / framing guards before opening the connection.
        let mut commands: Vec<(String, Vec<Vec<u8>>)> = Vec::new();
        for raw_line in req.statement.lines() {
            let trimmed = raw_line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let parts = try_split_args(trimmed)?;
            if parts.is_empty() {
                continue;
            }
            refuse_connection_state_command_bytes(&parts)?;
            commands.push((trimmed.to_string(), parts));
        }

        if commands.is_empty() {
            return Err(types::invalid("redis: no command provided"));
        }

        // Per-statement timeout: applied as a per-command wall-clock deadline so
        // each command in a multi-line input gets its own budget (rather than
        // sharing a single total budget across the batch).
        let timeout = req.timeout_ms.filter(|&t| t > 0).map(Duration::from_millis);
        let max_rows = req.max_rows.unwrap_or(DEFAULT_MAX_ROWS);

        // Operation-isolated connection: each `run()` call executes its batch on
        // a dedicated connection initially pinned to `keyspace_for(cfg, req)` and
        // configured with `req.timeout_ms` (so `SELECT`, `MULTI`/`EXEC`/`DISCARD`/
        // `WATCH`/`UNWATCH`, and blocking commands like `BLPOP` or `XREAD BLOCK`
        // work across the batch and never leak state or block the cached metadata
        // connection; dropping `conn` on return or cancel closes the socket).
        let db = keyspace_for(cfg, req)?;
        let mut conn = self.connect_isolated(cfg, db, timeout).await?;

        // Single-command fast path: preserves the exact top-level wire shape
        // (`statement: None`, `more_results: []`, and HTTP error on failure).
        if commands.len() == 1 {
            let (_, parts) = commands.into_iter().next().unwrap();
            let started = Instant::now();
            let reply = exec_command(&mut conn, &parts, timeout).await?;
            let duration_ms = started.elapsed().as_millis() as u64;
            let mut result = shape_reply(reply, max_rows).await?;
            result.stats.duration_ms = duration_ms;
            result.stats.row_count = result.rows.len();
            return Ok(result);
        }

        // Multi-command batch: execute in order on the same isolated connection,
        // share one response byte budget across all result sets, label each
        // result with its statement preview, and stop with an `errored` entry on
        // the first failure (matching SQL and MongoDB batch semantics).
        let mut results: Vec<QueryResult> = Vec::with_capacity(commands.len());
        let mut budget = types::ByteBudget::default();
        for (raw_line, parts) in commands {
            let preview = types::statement_preview(&raw_line);
            let cmd_started = Instant::now();
            match exec_command(&mut conn, &parts, timeout).await {
                Ok(reply) => {
                    let duration_ms = cmd_started.elapsed().as_millis() as u64;
                    let (mut r, next_budget) =
                        shape_reply_with_budget(reply, max_rows, budget).await?;
                    budget = next_budget;
                    r.stats.duration_ms = duration_ms;
                    r.stats.row_count = r.rows.len();
                    r.statement = Some(preview);
                    results.push(r);
                }
                Err(e) => {
                    results.push(types::errored_batch_entry(preview, e.to_string()));
                    break;
                }
            }
        }
        Ok(types::fold_batch_results(results))
    }

    /// Evict the cached connection managers for `cache_key` (connection close,
    /// or a config change superseded them). Redis keys its cache per logical db
    /// (`<cache_key>|db=<n>`), so every db's manager for this config is dropped
    /// — dropping a `BoundedConnection` closes the multiplexed connection and
    /// stops its driver task.
    fn detach(&self, cache_key: &str) -> Option<std::sync::Arc<dyn Driver>> {
        let captured = Self::default();
        for (key, value) in self
            .clients
            .take_where(|key| key.starts_with(&format!("{cache_key}|")))
        {
            captured.clients.insert_ready(key, value);
        }
        Some(std::sync::Arc::new(captured))
    }

    async fn close(&self, cache_key: &str) {
        let prefix = format!("{cache_key}|");
        self.clients.remove_where(|k| k.starts_with(&prefix));
    }

    /// Drop connection managers unused for `idle` (DB-09). Dropping the
    /// cache's clone stops the driver task once any in-flight command
    /// holding its own clone finishes — nothing is closed underneath it.
    async fn evict_idle(&self, idle: Duration) -> usize {
        self.clients.take_idle(idle).len()
    }

    async fn completion(
        &self,
        _cfg: &ResolvedConfig,
        _ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        // Commands ONLY — a static list, returned instantly. We deliberately do
        // NOT scan the keyspace for live key prefixes here: that round-tripped on
        // every keystroke and stalled completion for seconds on large databases.
        let items: Vec<CompletionItem> = REDIS_COMMANDS
            .iter()
            .map(|(name, summary)| {
                CompletionItem::detailed(*name, CompletionKind::Command, *summary)
            })
            .collect();

        Ok(CompletionResponse {
            items,
            ..Default::default()
        })
    }
}

// --- Connection -------------------------------------------------------------

/// Parse a non-negative Redis logical database index (`0`, `1`, …). Negative
/// indices are rejected so `kdb:-1` never reaches `SELECT -1` or cache keys.
fn parse_db_index(raw: &str) -> Option<i64> {
    raw.trim().parse::<i64>().ok().filter(|&n| n >= 0)
}

/// The logical database a call defaults to when no keyspace node scopes it:
/// the profile's configured database index, else db0.
fn default_db(cfg: &ResolvedConfig) -> i64 {
    cfg.database
        .as_deref()
        .and_then(parse_db_index)
        .unwrap_or(0)
}

/// The keyspace a command runs in: the request's scope (`kdb:<n>`, or a bare
/// index from a caller that already stripped the tag), else the profile
/// default. The connection is PINNED to it at handshake (no shared-connection
/// `SELECT` — that cross-wired concurrent requests, including writes).
///
/// A scope that names no keyspace (or a negative index) is REFUSED rather than
/// defaulted: falling back to the default database is exactly how a `DEL`
/// issued with db3 selected used to delete db0's same-named key.
fn keyspace_for(cfg: &ResolvedConfig, req: &QueryRequest) -> Result<i64> {
    match req.scope() {
        None => {
            if let Some(raw) = cfg
                .database
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return parse_db_index(raw).ok_or_else(|| {
                    types::invalid(format!(
                        "redis: '{raw}' is not a valid non-negative database index"
                    ))
                });
            }
            Ok(0)
        }
        Some(scope) => scope.keyspace().filter(|&db| db >= 0).ok_or_else(|| {
            types::invalid(format!(
                "redis: '{}' is not a keyspace (expected kdb:<n>)",
                scope.child()
            ))
        }),
    }
}

/// Build the `RedisConnectionInfo` for `cfg` PINNED to logical database `db`
/// (selected during the connection handshake), honouring AUTH (username and/or
/// password). TCP and TLS sockets are opened directly by [`BoundedConnection`].
fn redis_connection_info(cfg: &ResolvedConfig, db: i64) -> RedisConnectionInfo {
    let mut redis_info = RedisConnectionInfo::default().set_db(db);
    if let Some(user) = cfg.user.as_deref().filter(|s| !s.is_empty()) {
        redis_info = redis_info.set_username(user);
    }
    if let Some(password) = cfg.password.as_deref().filter(|s| !s.is_empty()) {
        redis_info = redis_info.set_password(password);
    }
    redis_info
}

/// Execute one parsed Redis command (command name + raw byte arguments) with an
/// optional client-side wall-clock timeout.
async fn exec_command(
    conn: &mut BoundedConnection,
    parts: &[Vec<u8>],
    timeout: Option<Duration>,
) -> Result<RedisValue> {
    use redis::aio::ConnectionLike;
    let cmd_name = std::str::from_utf8(&parts[0])
        .map_err(|_| types::invalid("redis: command name must be valid UTF-8"))?;
    let mut cmd = redis::cmd(cmd_name);
    for arg in &parts[1..] {
        cmd.arg(arg.as_slice());
    }
    // query_async recursively extracts errors from arrays, discarding EXEC's
    // successful siblings when one transaction command fails at runtime.
    // Keep the bounded raw reply and reject only an outer command error.
    let fut = conn.req_packed_command(&cmd);
    let mut value = if let Some(dur) = timeout {
        tokio::time::timeout(dur, fut)
            .await
            .map_err(|_| {
                types::upstream(format!(
                    "redis: command timed out after {}ms",
                    dur.as_millis()
                ))
            })?
            .map_err(types::upstream)?
    } else {
        fut.await.map_err(types::upstream)?
    };
    loop {
        match value {
            RedisValue::Attribute { data, .. } => value = *data,
            RedisValue::ServerError(e) => {
                return Err(types::upstream(redis::RedisError::from(e)));
            }
            other => return Ok(other),
        }
    }
}

impl RedisDriver {
    /// Get (or lazily build + cache) the `BoundedConnection` for `(cfg, db)`,
    /// keyed by [`ResolvedConfig::cache_key`] PLUS the logical database index.
    ///
    /// Used by metadata inspection (`schema_root`, `schema_children`,
    /// `object_detail`). Each cached manager is PINNED to its db at handshake
    /// (`set_db`).
    async fn connect(&self, cfg: &ResolvedConfig, db: i64) -> Result<BoundedConnection> {
        if db < 0 {
            return Err(types::invalid("redis: database index must be non-negative"));
        }
        let cache_key = format!("{}|db={db}", cfg.cache_key());
        self.clients
            .get_or_try_init(
                cache_key,
                cfg.lifecycle.as_ref(),
                BoundedConnection::is_alive,
                async {
                    let info = redis_connection_info(cfg, db);
                    BoundedConnection::connect(cfg, &info).await
                },
            )
            .await
    }

    /// Open a fresh, operation-isolated `BoundedConnection` for a `run()` batch,
    /// initially pinned to `db` and configured with a transport response timeout
    /// that accommodates `timeout` (even when > 30s for blocking commands like
    /// `BLPOP jobs 45` with `timeout_ms: Some(60_000)`). Dropping the returned
    /// connection on batch completion or cancellation immediately aborts its
    /// driver task and closes the underlying TCP/TLS socket.
    async fn connect_isolated(
        &self,
        cfg: &ResolvedConfig,
        db: i64,
        timeout: Option<Duration>,
    ) -> Result<BoundedConnection> {
        if db < 0 {
            return Err(types::invalid("redis: database index must be non-negative"));
        }
        let info = redis_connection_info(cfg, db);
        let response_timeout = Some(
            timeout
                .map(|d| {
                    d.max(Duration::from_secs(30))
                        .saturating_add(Duration::from_secs(5))
                })
                .unwrap_or(Duration::from_secs(30)),
        );
        BoundedConnection::connect_with_timeout(cfg, &info, response_timeout).await
    }
}

// --- SCAN / type / length / preview ----------------------------------------

/// Outcome of a bounded SCAN: the collected keys and whether more may exist
/// (cap reached, or the round-trip budget was exhausted before a full sweep).
struct ScanOutcome {
    keys: Vec<String>,
    more: bool,
}

/// Cursor-loop SCAN collecting up to `key_cap` keys, optionally filtered by
/// `MATCH <pattern>`. Never uses `KEYS *`. Bounded on BOTH the key count and the
/// number of round-trips ([`SCAN_MAX_ROUNDS`]), so a sparse filter on a huge
/// keyspace can't stall the tree by traversing every key. `more` reports whether
/// the listing is partial.
async fn scan_keys(
    conn: &mut BoundedConnection,
    pattern: Option<&str>,
    key_cap: usize,
) -> Result<ScanOutcome> {
    let count = if pattern.is_some() {
        SCAN_COUNT_FILTER
    } else {
        SCAN_COUNT
    };
    let mut cursor: u64 = 0;
    let mut keys: Vec<String> = Vec::new();
    let mut rounds = 0usize;
    let mut more = false;
    loop {
        rounds += 1;
        let mut cmd = redis::cmd("SCAN");
        cmd.arg(cursor);
        if let Some(pat) = pattern {
            cmd.arg("MATCH").arg(pat);
        }
        cmd.arg("COUNT").arg(count);

        let (next, batch): (u64, Vec<String>) =
            cmd.query_async(conn).await.map_err(types::upstream)?;
        keys.extend(batch);
        cursor = next;

        if keys.len() >= key_cap {
            keys.truncate(key_cap);
            more = true;
            break;
        }
        if cursor == 0 {
            break; // full sweep complete
        }
        if rounds >= SCAN_MAX_ROUNDS {
            more = true; // gave up early — listing is partial
            break;
        }
    }
    Ok(ScanOutcome { keys, more })
}

/// Build `Key` nodes for a scanned key set, looking up every key's type in a
/// single pipelined batch (one round-trip instead of one TYPE per key). Appends
/// a passive truncation hint when the listing was capped/partial.
async fn build_key_nodes(
    conn: &mut BoundedConnection,
    db: i64,
    mut scan: ScanOutcome,
) -> Result<Vec<SchemaNode>> {
    scan.keys.sort();
    let types = types_of(conn, &scan.keys).await;
    conn.ensure_alive()?;
    let mut nodes = Vec::with_capacity(scan.keys.len() + 1);
    for (i, key) in scan.keys.iter().enumerate() {
        let kind = types.get(i).cloned().unwrap_or_else(|| "unknown".into());
        nodes.push(
            SchemaNode::new(format!("kdb:{db}/key:{key}"), key.clone(), NodeKind::Key)
                .with_detail(kind),
        );
    }
    if scan.more {
        nodes.push(truncation_hint(db, "More keys — refine the prefix"));
    }
    Ok(nodes)
}

/// A passive (non-clickable, non-expandable) hint row appended to a truncated
/// key listing. Rendered as a `Folder` so the tree treats it as a label only.
fn truncation_hint(db: i64, msg: &str) -> SchemaNode {
    SchemaNode::new(
        format!("kdb:{db}/hint:{msg}"),
        format!("⋯ {msg}"),
        NodeKind::Folder,
    )
}

/// Escape Redis glob metacharacters (`* ? [ ] \`) so a user-typed prefix is
/// matched literally before the trailing `*` is appended.
fn glob_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Pipelined `TYPE` for many keys: one round-trip per [`KEY_LIST_CAP`]-sized
/// chunk instead of one per key. Returns a vec aligned with `keys` (missing or
/// failed lookups become "unknown").
async fn types_of(conn: &mut BoundedConnection, keys: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(keys.len());
    for chunk in keys.chunks(KEY_LIST_CAP) {
        let mut pipe = redis::pipe();
        for key in chunk {
            pipe.cmd("TYPE").arg(key);
        }
        match pipe.query_async::<Vec<String>>(conn).await {
            Ok(mut types) if types.len() == chunk.len() => out.append(&mut types),
            _ => out.extend(chunk.iter().map(|_| "unknown".to_string())),
        }
    }
    out
}

/// `TYPE`, `TTL` and `OBJECT ENCODING` for one key in a single pipelined
/// round trip. Each reply is decoded independently, so one failing command
/// (e.g. `OBJECT` disabled by ACL) degrades only its own field.
async fn key_header(conn: &mut BoundedConnection, key: &str) -> (String, i64, Option<String>) {
    let mut pipe = redis::pipe();
    // Per-command errors come back in place (as `ServerError` values).
    pipe.ignore_errors();
    pipe.cmd("TYPE")
        .arg(key)
        .cmd("TTL")
        .arg(key)
        .cmd("OBJECT")
        .arg("ENCODING")
        .arg(key);
    match pipe.query_async::<Vec<RedisValue>>(conn).await {
        Ok(replies) => decode_key_header(replies),
        // Transport-level failure of the pipeline: fall back to the
        // sequential reads (each degrades on its own).
        Err(_) => {
            let ty = type_of(conn, key).await;
            let ttl: i64 = redis::cmd("TTL")
                .arg(key)
                .query_async(conn)
                .await
                .unwrap_or(-2);
            let encoding: Option<String> = redis::cmd("OBJECT")
                .arg("ENCODING")
                .arg(key)
                .query_async(conn)
                .await
                .ok();
            (ty, ttl, encoding)
        }
    }
}

/// Decode the `[TYPE, TTL, OBJECT ENCODING]` pipeline replies (pure).
fn decode_key_header(replies: Vec<RedisValue>) -> (String, i64, Option<String>) {
    let mut it = replies.into_iter();
    let ty = it
        .next()
        .and_then(|v| redis::from_redis_value::<String>(v).ok())
        .unwrap_or_else(|| "unknown".into());
    let ttl = it
        .next()
        .and_then(|v| redis::from_redis_value::<i64>(v).ok())
        .unwrap_or(-2);
    let encoding = it
        .next()
        .and_then(|v| redis::from_redis_value::<String>(v).ok());
    (ty, ttl, encoding)
}

/// Length + preview for the structure panel. For the range-previewed types
/// (string/list/zset/stream) both commands go in ONE pipelined trip; hash/set
/// previews are cursor scans, so they run after the length.
async fn length_and_preview(
    conn: &mut BoundedConnection,
    key: &str,
    ty: &str,
) -> (Option<i64>, JsonValue) {
    let len_cmd = match ty {
        "string" => "STRLEN",
        "list" => "LLEN",
        "zset" => "ZCARD",
        "stream" => "XLEN",
        _ => {
            let length = length_of(conn, key, ty).await;
            return (length, preview_of(conn, key, ty).await);
        }
    };
    let mut pipe = redis::pipe();
    pipe.ignore_errors();
    pipe.cmd(len_cmd).arg(key);
    match ty {
        "string" => pipe
            .cmd("GETRANGE")
            .arg(key)
            .arg(0)
            .arg(PREVIEW_STRING_BYTES),
        "list" => pipe.cmd("LRANGE").arg(key).arg(0).arg(PREVIEW_LIMIT),
        "stream" => pipe
            .cmd("XRANGE")
            .arg(key)
            .arg("-")
            .arg("+")
            .arg("COUNT")
            .arg(PREVIEW_LIMIT),
        _ => pipe
            .cmd("ZRANGE")
            .arg(key)
            .arg(0)
            .arg(PREVIEW_LIMIT)
            .arg("WITHSCORES"),
    };
    match pipe.query_async::<Vec<RedisValue>>(conn).await {
        Ok(replies) => {
            let mut it = replies.into_iter();
            let length = it
                .next()
                .and_then(|v| redis::from_redis_value::<i64>(v).ok());
            let preview = it.next().map(value_to_json).unwrap_or(JsonValue::Null);
            (length, preview)
        }
        Err(_) => {
            let length = length_of(conn, key, ty).await;
            (length, preview_of(conn, key, ty).await)
        }
    }
}

/// `TYPE key` → "string"/"hash"/... ("none" when missing).
async fn type_of(conn: &mut BoundedConnection, key: &str) -> String {
    redis::cmd("TYPE")
        .arg(key)
        .query_async::<String>(conn)
        .await
        .unwrap_or_else(|_| "unknown".into())
}

/// Length of the value sized appropriately for its type.
async fn length_of(conn: &mut BoundedConnection, key: &str, ty: &str) -> Option<i64> {
    let cmd = match ty {
        "string" => "STRLEN",
        "hash" => "HLEN",
        "list" => "LLEN",
        "set" => "SCARD",
        "zset" => "ZCARD",
        "stream" => "XLEN",
        _ => return None,
    };
    redis::cmd(cmd).arg(key).query_async::<i64>(conn).await.ok()
}

/// A bounded value preview for the structure panel, shaped per type. Every
/// branch is size-bounded server-side: `GETRANGE` instead of `GET` (a string
/// value can be hundreds of MB), `HSCAN`/`SSCAN` instead of `HGETALL`/`SMEMBERS`
/// (a hash/set can hold millions of members), and range commands with an
/// explicit stop for list/zset/stream.
async fn preview_of(conn: &mut BoundedConnection, key: &str, ty: &str) -> JsonValue {
    match ty {
        "string" => redis::cmd("GETRANGE")
            .arg(key)
            .arg(0)
            .arg(PREVIEW_STRING_BYTES)
            .query_async::<RedisValue>(conn)
            .await
            .map(value_to_json)
            .unwrap_or(JsonValue::Null),
        "hash" => scan_preview(conn, key, "HSCAN", true).await,
        "set" => scan_preview(conn, key, "SSCAN", false).await,
        "list" => redis::cmd("LRANGE")
            .arg(key)
            .arg(0)
            .arg(PREVIEW_LIMIT)
            .query_async::<RedisValue>(conn)
            .await
            .map(value_to_json)
            .unwrap_or(JsonValue::Null),
        "zset" => redis::cmd("ZRANGE")
            .arg(key)
            .arg(0)
            .arg(PREVIEW_LIMIT)
            .arg("WITHSCORES")
            .query_async::<RedisValue>(conn)
            .await
            .map(value_to_json)
            .unwrap_or(JsonValue::Null),
        "stream" => redis::cmd("XRANGE")
            .arg(key)
            .arg("-")
            .arg("+")
            .arg("COUNT")
            .arg(PREVIEW_LIMIT)
            .query_async::<RedisValue>(conn)
            .await
            .map(value_to_json)
            .unwrap_or(JsonValue::Null),
        "ReJSON-RL" => redis::cmd("JSON.GET")
            .arg(key)
            .query_async::<RedisValue>(conn)
            .await
            .map(|v| types::cap_cell(value_to_json(v)))
            .unwrap_or(JsonValue::Null),
        "TSDB-TYPE" => redis::cmd("TS.GET")
            .arg(key)
            .query_async::<RedisValue>(conn)
            .await
            .map(value_to_json)
            .unwrap_or(JsonValue::Null),
        _ => JsonValue::Null,
    }
}

/// Cursor-scan a bounded preview of a hash (`HSCAN`, field/value pairs → an
/// object) or set (`SSCAN`, members → an array), stopping at [`PREVIEW_LIMIT`]
/// entries or [`SCAN_MAX_ROUNDS`] round-trips — never materialising the whole
/// collection the way `HGETALL`/`SMEMBERS` did.
async fn scan_preview(
    conn: &mut BoundedConnection,
    key: &str,
    cmd: &str,
    pairs: bool,
) -> JsonValue {
    let cap = PREVIEW_LIMIT as usize;
    let mut cursor: u64 = 0;
    let mut items: Vec<RedisValue> = Vec::new();
    for _ in 0..SCAN_MAX_ROUNDS {
        let reply: redis::RedisResult<(u64, Vec<RedisValue>)> = redis::cmd(cmd)
            .arg(key)
            .arg(cursor)
            .arg("COUNT")
            .arg(SCAN_COUNT)
            .query_async(conn)
            .await;
        let Ok((next, batch)) = reply else {
            return JsonValue::Null;
        };
        items.extend(batch);
        cursor = next;
        // For a hash, each entry is a field+value PAIR of reply elements.
        let want = if pairs { cap * 2 } else { cap };
        if cursor == 0 || items.len() >= want {
            break;
        }
    }
    if pairs {
        let mut obj = serde_json::Map::new();
        for chunk in items.chunks(2).take(cap) {
            let [f, v] = chunk else { break };
            let field = match value_to_json(f.clone()) {
                JsonValue::String(s) => s,
                other => other.to_string(),
            };
            obj.insert(field, value_to_json(v.clone()));
        }
        JsonValue::Object(obj)
    } else {
        items.truncate(cap);
        JsonValue::Array(items.into_iter().map(value_to_json).collect())
    }
}

// --- INFO parsing -----------------------------------------------------------

/// Pull a `field:value` line out of an INFO blob.
fn info_field(info: &str, field: &str) -> Option<String> {
    for line in info.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(field) {
            if let Some(value) = rest.strip_prefix(':') {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// Parse `keys=N` out of a keyspace stats segment (`keys=11,expires=0,...`).
fn keyspace_keys(stats: &str) -> Option<i64> {
    for part in stats.split(',') {
        if let Some(value) = part.trim().strip_prefix("keys=") {
            return value.parse().ok();
        }
    }
    None
}

// --- Reply conversion -------------------------------------------------------

/// Convert a RESP3 double to JSON: finite values serialize as numbers, while
/// `NaN`, `+inf` and `-inf` (valid RESP3 float replies, e.g. from `ZSCORE` or
/// `INCRBYFLOAT`) serialize as strings (`"nan"`, `"inf"`, `"-inf"`) instead of
/// collapsing to `null`.
fn double_to_json(d: f64) -> JsonValue {
    if d.is_finite() {
        json!(d)
    } else if d.is_nan() {
        JsonValue::String("nan".into())
    } else if d.is_sign_positive() {
        JsonValue::String("inf".into())
    } else {
        JsonValue::String("-inf".into())
    }
}

/// Convert a raw `redis::Value` reply into a [`QueryResult`] grid.
fn reply_to_result(reply: RedisValue) -> QueryResult {
    match reply {
        RedisValue::Nil => QueryResult::message("(nil)"),
        RedisValue::Okay => QueryResult::message("OK"),
        RedisValue::Int(n) => single_value(types::i64_to_json(n)),
        RedisValue::Double(d) => single_value(double_to_json(d)),
        RedisValue::Boolean(b) => single_value(JsonValue::Bool(b)),
        RedisValue::SimpleString(s) => single_value(JsonValue::String(s)),
        RedisValue::BulkString(bytes) => single_value(bytes_to_json(bytes)),
        RedisValue::VerbatimString { text, .. } => single_value(JsonValue::String(text)),
        RedisValue::BigNumber(n) => single_value(bytes_to_json(n)),
        // Array / set: one row per element (nested arrays flattened to JSON).
        RedisValue::Array(items) | RedisValue::Set(items) => array_to_result(items),
        // Map (RESP3): two columns key/value.
        RedisValue::Map(pairs) => map_to_result(pairs),
        RedisValue::Attribute { data, .. } => reply_to_result(*data),
        RedisValue::ServerError(e) => QueryResult::message(format!("(error) {e}")),
        RedisValue::Push { kind, data } => {
            let mut items = vec![value_to_json(RedisValue::SimpleString(kind.to_string()))];
            items.extend(data.into_iter().map(value_to_json));
            single_value(JsonValue::Array(items))
        }
        // `redis::Value` is #[non_exhaustive]; render anything new as text.
        other => single_value(JsonValue::String(format!("{other:?}"))),
    }
}

/// Replies with more top-level elements than this convert on the blocking pool.
const OFF_RUNTIME_ELEMENTS: usize = 16 * 1024;

/// Top-level element count of a reply (rows it would produce before capping).
fn reply_len(reply: &RedisValue) -> usize {
    match reply {
        RedisValue::Array(items) | RedisValue::Set(items) => items.len(),
        RedisValue::Map(pairs) => pairs.len(),
        RedisValue::Attribute { data, .. } => reply_len(data),
        _ => 1,
    }
}

/// Shape one reply with its own fresh [`types::ByteBudget`], offloading huge
/// collections onto the blocking pool.
async fn shape_reply(reply: RedisValue, max_rows: usize) -> Result<QueryResult> {
    let (result, _) =
        shape_reply_with_budget(reply, max_rows, types::ByteBudget::default()).await?;
    Ok(result)
}

/// Shape one reply against a shared [`types::ByteBudget`] (used across a
/// multi-command batch), offloading huge collections onto the blocking pool.
async fn shape_reply_with_budget(
    reply: RedisValue,
    max_rows: usize,
    budget: types::ByteBudget,
) -> Result<(QueryResult, types::ByteBudget)> {
    if reply_len(&reply) > OFF_RUNTIME_ELEMENTS {
        tokio::task::spawn_blocking(move || {
            bounded_reply_to_result_with_budget(reply, max_rows, budget)
        })
        .await
        .map_err(|e| types::upstream(format!("redis: decode task failed: {e}")))
    } else {
        Ok(bounded_reply_to_result_with_budget(reply, max_rows, budget))
    }
}

/// [`reply_to_result`] bounded for the run path: an Array/Set/Map reply is
/// sliced to `max_rows` BEFORE any JSON conversion (the dropped tail is never
/// converted), every cell is size-capped, and rows stop once the response byte
/// budget is spent (at least one row is always kept).
#[cfg(test)]
fn bounded_reply_to_result(reply: RedisValue, max_rows: usize) -> QueryResult {
    bounded_reply_to_result_with_budget(reply, max_rows, types::ByteBudget::default()).0
}

fn bounded_reply_to_result_with_budget(
    mut reply: RedisValue,
    max_rows: usize,
    mut budget: types::ByteBudget,
) -> (QueryResult, types::ByteBudget) {
    while let RedisValue::Attribute { data, .. } = reply {
        reply = *data;
    }
    let mut truncated = false;
    let mut cells_truncated = false;
    let mut truncated_reason = None;
    let mut push_row = |rows: &mut Vec<Vec<JsonValue>>, row: Vec<JsonValue>| -> bool {
        let row: Vec<JsonValue> = row
            .into_iter()
            .map(|v| types::cap_cell_tracked(v, &mut cells_truncated))
            .collect();
        let size: usize = row.iter().map(types::approx_json_len).sum();
        if !budget.charge(size) && !rows.is_empty() {
            truncated_reason = Some(types::TruncatedReason::Bytes);
            return false;
        }
        rows.push(row);
        true
    };
    let (columns, rows) = match reply {
        RedisValue::Array(mut items) | RedisValue::Set(mut items) => {
            if items.len() > max_rows {
                items.truncate(max_rows);
                truncated = true;
            }
            let mut rows = Vec::with_capacity(items.len());
            for item in items {
                if !push_row(&mut rows, vec![value_to_json(item)]) {
                    truncated = true;
                    break;
                }
            }
            (vec![Column::new("value")], rows)
        }
        RedisValue::Map(mut pairs) => {
            if pairs.len() > max_rows {
                pairs.truncate(max_rows);
                truncated = true;
            }
            let mut rows = Vec::with_capacity(pairs.len());
            for (k, v) in pairs {
                if !push_row(&mut rows, vec![value_to_json(k), value_to_json(v)]) {
                    truncated = true;
                    break;
                }
            }
            (vec![Column::new("key"), Column::new("value")], rows)
        }
        other => {
            let mut result = reply_to_result(other);
            for row in &mut result.rows {
                for cell in row.iter_mut() {
                    *cell =
                        types::cap_cell_tracked(std::mem::take(cell), &mut result.cells_truncated);
                }
                let size: usize = row.iter().map(types::approx_json_len).sum();
                let _ = budget.charge(size);
            }
            return (result, budget);
        }
    };
    (
        QueryResult {
            cells_truncated,
            columns,
            rows,
            truncated,
            truncated_reason,
            ..QueryResult::empty()
        },
        budget,
    )
}

fn single_value(value: JsonValue) -> QueryResult {
    QueryResult {
        columns: vec![Column::new("value")],
        rows: vec![vec![value]],
        ..QueryResult::empty()
    }
}

fn array_to_result(items: Vec<RedisValue>) -> QueryResult {
    let rows: Vec<Vec<JsonValue>> = items
        .into_iter()
        .map(|item| vec![value_to_json(item)])
        .collect();
    QueryResult {
        columns: vec![Column::new("value")],
        rows,
        ..QueryResult::empty()
    }
}

fn map_to_result(pairs: Vec<(RedisValue, RedisValue)>) -> QueryResult {
    let rows: Vec<Vec<JsonValue>> = pairs
        .into_iter()
        .map(|(k, v)| vec![value_to_json(k), value_to_json(v)])
        .collect();
    QueryResult {
        columns: vec![Column::new("key"), Column::new("value")],
        rows,
        ..QueryResult::empty()
    }
}

/// Recursively render a `redis::Value` as a JSON value (used for cells and the
/// object-detail preview). Bulk strings become UTF-8 strings when valid.
fn value_to_json(value: RedisValue) -> JsonValue {
    match value {
        RedisValue::Nil => JsonValue::Null,
        RedisValue::Okay => JsonValue::String("OK".into()),
        RedisValue::Int(n) => types::i64_to_json(n),
        RedisValue::Double(d) => double_to_json(d),
        RedisValue::Boolean(b) => JsonValue::Bool(b),
        RedisValue::SimpleString(s) => JsonValue::String(s),
        RedisValue::BulkString(bytes) => bytes_to_json(bytes),
        RedisValue::VerbatimString { text, .. } => JsonValue::String(text),
        RedisValue::BigNumber(n) => bytes_to_json(n),
        RedisValue::Array(items) | RedisValue::Set(items) => {
            JsonValue::Array(items.into_iter().map(value_to_json).collect())
        }
        RedisValue::Map(pairs) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in pairs {
                let key = match value_to_json(k) {
                    JsonValue::String(s) => s,
                    other => other.to_string(),
                };
                obj.insert(key, value_to_json(v));
            }
            JsonValue::Object(obj)
        }
        RedisValue::Attribute { data, .. } => value_to_json(*data),
        RedisValue::ServerError(e) => JsonValue::String(format!("(error) {e}")),
        RedisValue::Push { kind, data } => {
            let mut items = vec![JsonValue::String(kind.to_string())];
            items.extend(data.into_iter().map(value_to_json));
            JsonValue::Array(items)
        }
        // `redis::Value` is #[non_exhaustive]; render anything new as text.
        other => JsonValue::String(format!("{other:?}")),
    }
}

/// Bulk-string bytes → JSON string (UTF-8) or, when binary, base64 of the
/// payload (matching the MySQL/Postgres blob rendering) — the old `<N bytes>`
/// placeholder discarded the value entirely, making binary-serialized cache
/// entries impossible to inspect.
fn bytes_to_json(bytes: Vec<u8>) -> JsonValue {
    match String::from_utf8(bytes) {
        Ok(s) => JsonValue::String(s),
        Err(e) => {
            use base64::Engine as _;
            let raw = e.into_bytes();
            JsonValue::String(base64::engine::general_purpose::STANDARD.encode(raw))
        }
    }
}

// --- Argument splitter ------------------------------------------------------

/// Split a single Redis command line into raw byte arguments, matching Redis
/// `sdssplitargs` (`src/sds.c`) quoting rules:
/// - Double-quoted `"..."` strings decode `\n`, `\r`, `\t`, `\b`, `\a`, and
///   lowercase `\xHH` raw hex byte escapes; any other `\c` drops the backslash
///   and emits `c` (`\"` -> `"`, `\\` -> `\`, `\q` -> `q`, `\X41` -> `X41`).
/// - Single-quoted `'...'` strings preserve all characters literally except
///   `\'` (so `'a\\b'` retains both backslashes and `'{"a": "b c"}'` stays intact).
/// - A closing quote (`"` or `'`) must be followed by whitespace or end-of-line.
/// - Unquoted `#` at a token boundary starts an inline comment to end-of-line.
/// - Unclosed single or double quotes return an error instead of executing a
///   mangled command.
fn try_split_args(line: &str) -> Result<Vec<Vec<u8>>> {
    let mut args: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut in_double = false;
    let mut in_single = false;
    let mut has_token = false;
    let mut chars = line.chars().peekable();
    let mut utf8_buf = [0u8; 4];

    while let Some(c) = chars.next() {
        if in_double {
            match c {
                '"' => {
                    if chars.peek().is_some_and(|next| !next.is_whitespace()) {
                        return Err(types::invalid(
                            "redis: closing quote must be followed by whitespace",
                        ));
                    }
                    in_double = false;
                }
                '\\' => match chars.next() {
                    None => {
                        return Err(types::invalid("redis: unclosed quote in command"));
                    }
                    Some('n') => current.push(b'\n'),
                    Some('r') => current.push(b'\r'),
                    Some('t') => current.push(b'\t'),
                    Some('b') => current.push(0x08),
                    Some('a') => current.push(0x07),
                    Some('x') => {
                        let mut lookahead = chars.clone();
                        if let (Some(h1), Some(h2)) = (lookahead.next(), lookahead.next()) {
                            if let (Some(d1), Some(d2)) = (h1.to_digit(16), h2.to_digit(16)) {
                                chars.next();
                                chars.next();
                                current.push(((d1 as u8) << 4) | (d2 as u8));
                                continue;
                            }
                        }
                        current.push(b'x');
                    }
                    Some(other) => {
                        current.extend_from_slice(other.encode_utf8(&mut utf8_buf).as_bytes());
                    }
                },
                other => {
                    current.extend_from_slice(other.encode_utf8(&mut utf8_buf).as_bytes());
                }
            }
        } else if in_single {
            match c {
                '\'' => {
                    if chars.peek().is_some_and(|next| !next.is_whitespace()) {
                        return Err(types::invalid(
                            "redis: closing quote must be followed by whitespace",
                        ));
                    }
                    in_single = false;
                }
                '\\' => {
                    if chars.peek().copied() == Some('\'') {
                        chars.next();
                        current.push(b'\'');
                    } else {
                        current.push(b'\\');
                    }
                }
                other => {
                    current.extend_from_slice(other.encode_utf8(&mut utf8_buf).as_bytes());
                }
            }
        } else {
            match c {
                '"' => {
                    in_double = true;
                    has_token = true;
                }
                '\'' => {
                    in_single = true;
                    has_token = true;
                }
                '#' if !has_token => {
                    // Inline `#` comment after whitespace (or at start of line).
                    break;
                }
                c if c.is_whitespace() => {
                    if has_token {
                        args.push(std::mem::take(&mut current));
                        has_token = false;
                    }
                }
                other => {
                    current.extend_from_slice(other.encode_utf8(&mut utf8_buf).as_bytes());
                    has_token = true;
                }
            }
        }
    }

    if in_double || in_single {
        return Err(types::invalid("redis: unclosed quote in command"));
    }
    if has_token {
        args.push(current);
    }
    Ok(args)
}

/// Convenience UTF-8 wrapper around [`try_split_args`].
#[cfg(test)]
fn split_args(line: &str) -> Vec<String> {
    try_split_args(line)
        .unwrap_or_default()
        .into_iter()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .collect()
}

fn refuse_connection_state_command_bytes(parts: &[Vec<u8>]) -> Result<()> {
    let strings: Vec<String> = parts
        .iter()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .collect();
    refuse_connection_state_command(&strings)
}

// --- Command table ----------------------------------------------------------

/// Refuse commands that switch the connection out of request/response framing
/// into an unbounded push stream (`SUBSCRIBE`, `MONITOR`, `SYNC`/`PSYNC`) or
/// desynchronize/re-authenticate the transport (`CLIENT REPLY OFF|SKIP`,
/// `CLIENT TRACKING ON`, `AUTH`, `HELLO`, `QUIT`, `RESET`).
///
/// Stateful batch commands (`SELECT`, `MULTI`/`EXEC`/`DISCARD`/`WATCH`/`UNWATCH`,
/// per-connection `CLIENT` modes, and bounded blocking commands like `BLPOP` or
/// `XREAD BLOCK`) ARE supported because [`RedisDriver::run`] executes every
/// batch on its own operation-isolated connection that closes when the batch
/// finishes or is cancelled.
fn refuse_connection_state_command(parts: &[String]) -> Result<()> {
    let Some(head) = parts.first() else {
        return Ok(());
    };
    let name = head.to_ascii_uppercase();
    let sub = parts.get(1).map(|s| s.to_ascii_uppercase());
    let arg2 = parts.get(2).map(|s| s.to_ascii_uppercase());
    let why = match name.as_str() {
        "SUBSCRIBE" | "PSUBSCRIBE" | "SSUBSCRIBE" | "UNSUBSCRIBE" | "PUNSUBSCRIBE"
        | "SUNSUBSCRIBE" | "MONITOR" | "SYNC" | "PSYNC" => {
            "continuous pub/sub, replication, and MONITOR streams are not supported in the request/response console — use a terminal redis-cli session"
        }
        "RESET" | "QUIT" | "HELLO" | "AUTH" => {
            "connection credentials and protocol are managed by the connection settings"
        }
        "CLIENT"
            if sub.as_deref() == Some("REPLY")
                && matches!(arg2.as_deref(), Some("OFF" | "SKIP")) =>
        {
            "CLIENT REPLY OFF/SKIP suppresses server replies and desynchronizes request/response framing"
        }
        "CLIENT"
            if sub.as_deref() == Some("TRACKING") && arg2.as_deref() == Some("ON") =>
        {
            "CLIENT TRACKING ON injects unsolicited invalidation push frames"
        }
        _ => return Ok(()),
    };
    Err(types::invalid(format!(
        "redis: {name}{} is not supported in the console — {why}",
        match (name.as_str(), sub.as_deref()) {
            ("CLIENT", Some(sub)) => format!(" {sub}"),
            _ => String::new(),
        }
    )))
}

/// True when every command in `statement` parses cleanly and is a known
/// read-only Redis command. Unclosed quotes or unknown/mutating commands fail
/// closed (`false` → treated as a write).
pub fn statement_is_read_only(statement: &str) -> bool {
    for raw_line in statement.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Ok(parts) = try_split_args(trimmed) else {
            return false;
        };
        if parts.is_empty() {
            continue;
        }
        if !command_is_read_only(&parts) {
            return false;
        }
    }
    true
}

/// True when any command line in `statement` invokes `KEYS` (even if the command
/// verb itself is quoted, e.g. `"KEYS" *` or `'keys' *`).
pub fn statement_uses_keys(statement: &str) -> bool {
    for raw_line in statement.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        match try_split_args(trimmed) {
            Ok(parts) => {
                if parts
                    .first()
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .is_some_and(|cmd| cmd.eq_ignore_ascii_case("KEYS"))
                {
                    return true;
                }
            }
            Err(_) => {
                let head = trimmed
                    .trim_matches(|c| c == '"' || c == '\'')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                if head.eq_ignore_ascii_case("KEYS") {
                    return true;
                }
            }
        }
    }
    false
}

fn command_is_read_only(parts: &[Vec<u8>]) -> bool {
    let Some(cmd) = parts.first().and_then(|b| std::str::from_utf8(b).ok()) else {
        return false;
    };
    let upper = cmd.to_ascii_uppercase();
    let sub = parts
        .get(1)
        .and_then(|b| std::str::from_utf8(b).ok())
        .map(str::to_ascii_uppercase);

    match upper.as_str() {
        "CONFIG" => matches!(sub.as_deref(), Some("GET" | "HELP")),
        "CLIENT" => matches!(
            sub.as_deref(),
            Some("ID" | "INFO" | "LIST" | "GETNAME" | "GETREDIR" | "TRACKINGINFO" | "HELP")
        ),
        "MEMORY" => matches!(
            sub.as_deref(),
            Some("USAGE" | "STATS" | "DOCTOR" | "MALLOC-STATS" | "HELP")
        ),
        "ACL" => match sub.as_deref() {
            Some("WHOAMI" | "LIST" | "USERS" | "GETUSER" | "CAT" | "DRYRUN" | "HELP") => true,
            Some("LOG") => match parts.get(2).and_then(|b| std::str::from_utf8(b).ok()) {
                None => true,
                Some(arg) => parts.len() == 3 && arg.parse::<u64>().is_ok(),
            },
            _ => false,
        },
        "SLOWLOG" => matches!(sub.as_deref(), Some("GET" | "LEN" | "HELP")),
        "LATENCY" => matches!(
            sub.as_deref(),
            Some("LATEST" | "HISTORY" | "GRAPH" | "DOCTOR" | "HISTOGRAM" | "HELP")
        ),
        "FUNCTION" => matches!(sub.as_deref(), Some("LIST" | "STATS" | "DUMP" | "HELP")),
        "SCRIPT" => matches!(sub.as_deref(), Some("EXISTS" | "SHOW" | "HELP")),
        "CLUSTER" => matches!(
            sub.as_deref(),
            Some(
                "INFO"
                    | "NODES"
                    | "SLOTS"
                    | "SHARDS"
                    | "KEYSLOT"
                    | "MYID"
                    | "MYSHARDID"
                    | "LINKS"
                    | "REPLICAS"
                    | "SLAVES"
                    | "COUNTKEYSINSLOT"
                    | "GETKEYSINSLOT"
                    | "HELP"
            )
        ),
        "MODULE" => matches!(sub.as_deref(), Some("LIST" | "HELP")),
        "OBJECT" => matches!(
            sub.as_deref(),
            Some("ENCODING" | "FREQ" | "IDLETIME" | "REFCOUNT" | "HELP")
        ),
        "XINFO" => matches!(
            sub.as_deref(),
            Some("CONSUMERS" | "GROUPS" | "STREAM" | "HELP")
        ),
        "PUBSUB" => matches!(
            sub.as_deref(),
            Some("CHANNELS" | "NUMSUB" | "NUMPAT" | "SHARDCHANNELS" | "SHARDNUMSUB" | "HELP")
        ),
        "COMMAND" => matches!(
            sub.as_deref(),
            None | Some(
                "COUNT" | "DOCS" | "GETKEYS" | "GETKEYSANDFLAGS" | "INFO" | "LIST" | "HELP"
            )
        ),
        other => REDIS_READ_ONLY_VERBS.contains(&other),
    }
}

/// Pure single-word read-only Redis verbs (native Redis through 8.0 + Redis Stack modules).
/// Two-word families (`CONFIG`, `CLIENT`, `MEMORY`, `ACL`, `SLOWLOG`, `LATENCY`,
/// `FUNCTION`, `SCRIPT`, `CLUSTER`, `MODULE`, `OBJECT`, `XINFO`, `PUBSUB`, `COMMAND`)
/// are matched by positive subcommand allowlists in [`command_is_read_only`].
/// Batch framing verbs (`SELECT`, `MULTI`, `EXEC`, `DISCARD`, `WATCH`, `UNWATCH`)
/// run on an operation-isolated connection and only pass `statement_is_read_only`
/// when every queued command in the same batch is also read-only.
#[rustfmt::skip]
const REDIS_READ_ONLY_VERBS: &[&str] = &[
    // Strings & generic keys
    "GET", "MGET", "STRLEN", "GETRANGE", "SUBSTR", "LCS", "EXISTS", "TYPE", "TTL", "PTTL",
    "EXPIRETIME", "PEXPIRETIME", "KEYS", "SCAN", "RANDOMKEY", "DUMP", "SORT_RO",
    // Hashes (including Redis 7.4 field TTL inspection)
    "HGET", "HMGET", "HGETALL", "HKEYS", "HVALS", "HLEN", "HEXISTS", "HSCAN", "HSTRLEN",
    "HRANDFIELD", "HTTL", "HPTTL", "HEXPIRETIME", "HPEXPIRETIME",
    // Lists
    "LRANGE", "LINDEX", "LLEN", "LPOS",
    // Sets
    "SMEMBERS", "SISMEMBER", "SMISMEMBER", "SCARD", "SSCAN", "SRANDMEMBER",
    "SINTER", "SINTERCARD", "SUNION", "SDIFF",
    // Sorted sets
    "ZRANGE", "ZREVRANGE", "ZRANGEBYSCORE", "ZREVRANGEBYSCORE", "ZRANGEBYLEX", "ZREVRANGEBYLEX",
    "ZSCORE", "ZMSCORE", "ZRANK", "ZREVRANK", "ZCARD", "ZCOUNT", "ZLEXCOUNT", "ZSCAN",
    "ZRANDMEMBER", "ZINTER", "ZINTERCARD", "ZUNION", "ZDIFF",
    // Streams
    "XLEN", "XRANGE", "XREVRANGE", "XPENDING", "XREAD",
    // Bitmaps & HyperLogLog & Geo
    "GETBIT", "BITCOUNT", "BITPOS", "BITFIELD_RO", "PFCOUNT",
    "GEOPOS", "GEODIST", "GEOSEARCH", "GEOHASH", "GEORADIUS_RO", "GEORADIUSBYMEMBER_RO",
    // Read-only scripting, isolated batch control & server introspection
    "EVAL_RO", "EVALSHA_RO", "FCALL_RO", "DBSIZE", "PING", "ECHO", "INFO", "TIME",
    "ROLE", "LASTSAVE", "LOLWUT", "SELECT", "MULTI", "EXEC", "DISCARD", "WATCH", "UNWATCH",
    // Redis Stack modules (RedisJSON, RediSearch, RedisTimeSeries, Probabilistic)
    "JSON.GET", "JSON.MGET", "JSON.TYPE", "JSON.STRLEN", "JSON.ARRLEN", "JSON.OBJLEN",
    "JSON.OBJKEYS", "JSON.RESP",
    "FT.SEARCH", "FT.AGGREGATE", "FT.INFO", "FT._LIST", "FT.EXPLAIN", "FT.EXPLAINCLI",
    "TS.GET", "TS.MGET", "TS.RANGE", "TS.REVRANGE", "TS.MRANGE", "TS.MREVRANGE", "TS.INFO",
    "TS.QUERYINDEX",
    "BF.EXISTS", "BF.MEXISTS", "BF.INFO", "BF.SCANDUMP",
    "CF.EXISTS", "CF.MEXISTS", "CF.COUNT", "CF.INFO", "CF.SCANDUMP",
    "CMS.QUERY", "CMS.INFO",
    "TOPK.QUERY", "TOPK.COUNT", "TOPK.LIST", "TOPK.INFO",
    "TDIGEST.QUANTILE", "TDIGEST.CDF", "TDIGEST.RANK", "TDIGEST.REVRANK", "TDIGEST.BYRANK",
    "TDIGEST.BYREVRANK", "TDIGEST.MIN", "TDIGEST.MAX", "TDIGEST.TRIMMED_MEAN", "TDIGEST.INFO",
];

const REDIS_COMMANDS: &[(&str, &str)] = &[
    // Strings
    ("GET", "GET key — get the value of a key"),
    ("SET", "SET key value [EX s|PX ms] [NX|XX] [GET] — set a string value"),
    ("SETEX", "SETEX key seconds value — set with expiry"),
    ("PSETEX", "PSETEX key ms value — set with millisecond expiry"),
    ("SETNX", "SETNX key value — set only if key does not exist"),
    ("GETSET", "GETSET key value — set and return old value"),
    ("GETDEL", "GETDEL key — get value and delete the key"),
    ("GETEX", "GETEX key [EX s|PX ms|PERSIST] — get value and update expiry"),
    ("APPEND", "APPEND key value — append to a string value"),
    ("STRLEN", "STRLEN key — byte length of a string value"),
    ("SUBSTR", "SUBSTR key start end — substring of a string value"),
    ("LCS", "LCS key1 key2 [LEN] [IDX] — longest common subsequence"),
    ("MGET", "MGET key [key ...] — get multiple string values"),
    ("MSET", "MSET key value [key value ...] — set multiple keys"),
    ("MSETNX", "MSETNX key value [key value ...] — set only if none exist"),
    ("SETRANGE", "SETRANGE key offset value — overwrite part of a string"),
    ("GETRANGE", "GETRANGE key start end — substring of a string value"),
    ("INCR", "INCR key — increment an integer value by 1"),
    ("DECR", "DECR key — decrement an integer value by 1"),
    ("INCRBY", "INCRBY key n — increment an integer value by n"),
    ("DECRBY", "DECRBY key n — decrement an integer value by n"),
    ("INCRBYFLOAT", "INCRBYFLOAT key n — increment a float value"),
    // Generic / Keys
    ("DEL", "DEL key [key ...] — delete keys synchronously"),
    ("UNLINK", "UNLINK key [key ...] — delete keys asynchronously"),
    ("EXISTS", "EXISTS key [key ...] — count existing keys"),
    ("TOUCH", "TOUCH key [key ...] — update last access time of keys"),
    ("COPY", "COPY source destination [DB db] [REPLACE] — copy a key"),
    ("EXPIRE", "EXPIRE key seconds [NX|XX|GT|LT] — set a key's TTL"),
    ("PEXPIRE", "PEXPIRE key ms [NX|XX|GT|LT] — set a key's TTL in ms"),
    ("EXPIREAT", "EXPIREAT key unix-time-seconds — expire at a timestamp"),
    ("PEXPIREAT", "PEXPIREAT key unix-time-ms — expire at a ms timestamp"),
    ("EXPIRETIME", "EXPIRETIME key — absolute Unix expiry timestamp in seconds"),
    ("PEXPIRETIME", "PEXPIRETIME key — absolute Unix expiry timestamp in ms"),
    ("TTL", "TTL key — remaining TTL in seconds"),
    ("PTTL", "PTTL key — remaining TTL in milliseconds"),
    ("PERSIST", "PERSIST key — remove a key's TTL"),
    ("TYPE", "TYPE key — data type of a key"),
    ("RENAME", "RENAME key newkey — rename a key"),
    ("RENAMENX", "RENAMENX key newkey — rename if newkey absent"),
    ("RANDOMKEY", "RANDOMKEY — return a random key"),
    ("KEYS", "KEYS pattern — keys matching a pattern (avoid in prod)"),
    ("SCAN", "SCAN cursor [MATCH pat] [COUNT n] [TYPE t] — iterate keys"),
    ("DUMP", "DUMP key — serialized value of a key"),
    ("RESTORE", "RESTORE key ttl value [REPLACE] — create a key from a dump"),
    ("OBJECT", "OBJECT ENCODING|REFCOUNT|IDLETIME|FREQ key — introspect a key"),
    ("SORT", "SORT key [BY pat] [LIMIT offset count] [ASC|DESC] — sort elements"),
    ("SORT_RO", "SORT_RO key [BY pat] [LIMIT offset count] — read-only sort"),
    // Hashes (including Redis 7.4 field expiration)
    ("HGET", "HGET key field — get a hash field"),
    ("HSET", "HSET key field value [field value ...] — set hash fields"),
    ("HSETNX", "HSETNX key field value — set field if absent"),
    ("HMSET", "HMSET key field value [field value ...] — set hash fields"),
    ("HMGET", "HMGET key field [field ...] — get multiple hash fields"),
    ("HGETALL", "HGETALL key — all fields and values of a hash"),
    ("HDEL", "HDEL key field [field ...] — delete hash fields"),
    ("HEXISTS", "HEXISTS key field — does a hash field exist"),
    ("HKEYS", "HKEYS key — all field names of a hash"),
    ("HVALS", "HVALS key — all values of a hash"),
    ("HLEN", "HLEN key — number of fields in a hash"),
    ("HSTRLEN", "HSTRLEN key field — string length of a hash field"),
    ("HINCRBY", "HINCRBY key field n — increment a hash field"),
    ("HINCRBYFLOAT", "HINCRBYFLOAT key field n — increment a float hash field"),
    ("HRANDFIELD", "HRANDFIELD key [count [WITHVALUES]] — random hash field(s)"),
    ("HSCAN", "HSCAN key cursor [MATCH pat] [COUNT n] — iterate a hash"),
    ("HEXPIRE", "HEXPIRE key seconds FIELDS numfields field [field ...] — set hash field TTL"),
    ("HPEXPIRE", "HPEXPIRE key ms FIELDS numfields field [field ...] — set hash field TTL in ms"),
    ("HEXPIREAT", "HEXPIREAT key unix-time FIELDS numfields field [field ...] — expire hash fields at timestamp"),
    ("HPEXPIREAT", "HPEXPIREAT key unix-time-ms FIELDS numfields field [field ...] — expire hash fields at ms timestamp"),
    ("HEXPIRETIME", "HEXPIRETIME key FIELDS numfields field [field ...] — Unix expiry of hash fields"),
    ("HPEXPIRETIME", "HPEXPIRETIME key FIELDS numfields field [field ...] — ms Unix expiry of hash fields"),
    ("HTTL", "HTTL key FIELDS numfields field [field ...] — remaining TTL of hash fields"),
    ("HPTTL", "HPTTL key FIELDS numfields field [field ...] — remaining ms TTL of hash fields"),
    ("HPERSIST", "HPERSIST key FIELDS numfields field [field ...] — remove TTL from hash fields"),
    // Lists
    ("LPUSH", "LPUSH key value [value ...] — prepend to a list"),
    ("RPUSH", "RPUSH key value [value ...] — append to a list"),
    ("LPUSHX", "LPUSHX key value [value ...] — prepend only if list exists"),
    ("RPUSHX", "RPUSHX key value [value ...] — append only if list exists"),
    ("LPOP", "LPOP key [count] — remove and return head element(s)"),
    ("RPOP", "RPOP key [count] — remove and return tail element(s)"),
    ("LMPOP", "LMPOP numkeys key [key ...] LEFT|RIGHT [COUNT n] — pop from first non-empty list"),
    ("LRANGE", "LRANGE key start stop — a range of list elements"),
    ("LLEN", "LLEN key — length of a list"),
    ("LINDEX", "LINDEX key index — element at an index"),
    ("LINSERT", "LINSERT key BEFORE|AFTER pivot element — insert relative to pivot"),
    ("LPOS", "LPOS key element [RANK r] [COUNT n] — index of matching element"),
    ("LSET", "LSET key index value — set element at an index"),
    ("LREM", "LREM key count value — remove elements from a list"),
    ("LTRIM", "LTRIM key start stop — trim a list to a range"),
    ("LMOVE", "LMOVE source destination LEFT|RIGHT LEFT|RIGHT — atomically move element"),
    ("RPOPLPUSH", "RPOPLPUSH source destination — pop tail and push to head"),
    // Sets
    ("SADD", "SADD key member [member ...] — add to a set"),
    ("SREM", "SREM key member [member ...] — remove from a set"),
    ("SMEMBERS", "SMEMBERS key — all members of a set"),
    ("SCARD", "SCARD key — number of members in a set"),
    ("SISMEMBER", "SISMEMBER key member — is member of a set"),
    ("SMISMEMBER", "SMISMEMBER key member [member ...] — check multiple set members"),
    ("SMOVE", "SMOVE source destination member — move member between sets"),
    ("SPOP", "SPOP key [count] — remove and return random member(s)"),
    ("SRANDMEMBER", "SRANDMEMBER key [count] — random member(s)"),
    ("SINTER", "SINTER key [key ...] — intersect sets"),
    ("SINTERSTORE", "SINTERSTORE destination key [key ...] — store set intersection"),
    ("SINTERCARD", "SINTERCARD numkeys key [key ...] [LIMIT n] — cardinality of set intersection"),
    ("SUNION", "SUNION key [key ...] — union sets"),
    ("SUNIONSTORE", "SUNIONSTORE destination key [key ...] — store set union"),
    ("SDIFF", "SDIFF key [key ...] — difference of sets"),
    ("SDIFFSTORE", "SDIFFSTORE destination key [key ...] — store set difference"),
    ("SSCAN", "SSCAN key cursor [MATCH pat] [COUNT n] — iterate a set"),
    // Sorted sets
    ("ZADD", "ZADD key [NX|XX] [GT|LT] score member [score member ...] — add to sorted set"),
    ("ZREM", "ZREM key member [member ...] — remove from a sorted set"),
    ("ZRANGE", "ZRANGE key start stop [BYSCORE|BYLEX] [REV] [WITHSCORES] — range of members"),
    ("ZREVRANGE", "ZREVRANGE key start stop [WITHSCORES] — range by reverse rank"),
    ("ZRANGEBYSCORE", "ZRANGEBYSCORE key min max [WITHSCORES] — range by score"),
    ("ZREVRANGEBYSCORE", "ZREVRANGEBYSCORE key max min [WITHSCORES] — reverse range by score"),
    ("ZRANGEBYLEX", "ZRANGEBYLEX key min max [LIMIT offset count] — range by lex order"),
    ("ZREVRANGEBYLEX", "ZREVRANGEBYLEX key max min [LIMIT offset count] — reverse lex range"),
    ("ZRANGESTORE", "ZRANGESTORE dst src min max [BYSCORE|BYLEX] [REV] — store range"),
    ("ZSCORE", "ZSCORE key member — score of a member"),
    ("ZMSCORE", "ZMSCORE key member [member ...] — scores of multiple members"),
    ("ZRANK", "ZRANK key member — ascending rank of a member"),
    ("ZREVRANK", "ZREVRANK key member — descending rank of a member"),
    ("ZCARD", "ZCARD key — number of members in a sorted set"),
    ("ZINCRBY", "ZINCRBY key n member — increment a member's score"),
    ("ZCOUNT", "ZCOUNT key min max — count members in a score range"),
    ("ZLEXCOUNT", "ZLEXCOUNT key min max — count members in a lexical range"),
    ("ZPOPMIN", "ZPOPMIN key [count] — remove and return lowest-scoring member(s)"),
    ("ZPOPMAX", "ZPOPMAX key [count] — remove and return highest-scoring member(s)"),
    ("ZMPOP", "ZMPOP numkeys key [key ...] MIN|MAX [COUNT n] — pop from first non-empty zset"),
    ("ZRANDMEMBER", "ZRANDMEMBER key [count [WITHSCORES]] — random member(s)"),
    ("ZINTER", "ZINTER numkeys key [key ...] [WITHSCORES] — intersect sorted sets"),
    ("ZINTERSTORE", "ZINTERSTORE destination numkeys key [key ...] — store zset intersection"),
    ("ZINTERCARD", "ZINTERCARD numkeys key [key ...] [LIMIT n] — cardinality of zset intersection"),
    ("ZUNION", "ZUNION numkeys key [key ...] [WITHSCORES] — union sorted sets"),
    ("ZUNIONSTORE", "ZUNIONSTORE destination numkeys key [key ...] — store zset union"),
    ("ZDIFF", "ZDIFF numkeys key [key ...] [WITHSCORES] — difference of sorted sets"),
    ("ZDIFFSTORE", "ZDIFFSTORE destination numkeys key [key ...] — store zset difference"),
    ("ZREMRANGEBYRANK", "ZREMRANGEBYRANK key start stop — remove by rank range"),
    ("ZREMRANGEBYSCORE", "ZREMRANGEBYSCORE key min max — remove by score range"),
    ("ZREMRANGEBYLEX", "ZREMRANGEBYLEX key min max — remove by lex range"),
    ("ZSCAN", "ZSCAN key cursor [MATCH pat] [COUNT n] — iterate a sorted set"),
    // Streams
    ("XADD", "XADD key [NOMKSTREAM] [MAXLEN|MINID ~ n] *|id field value [field value ...] — append stream entry"),
    ("XLEN", "XLEN key — number of entries in a stream"),
    ("XRANGE", "XRANGE key start end [COUNT n] — stream entries in ID range"),
    ("XREVRANGE", "XREVRANGE key end start [COUNT n] — stream entries in reverse ID range"),
    ("XREAD", "XREAD [COUNT n] STREAMS key [key ...] id [id ...] — read entries from streams"),
    ("XREADGROUP", "XREADGROUP GROUP group consumer [COUNT n] STREAMS key [key ...] id — read via consumer group"),
    ("XACK", "XACK key group id [id ...] — acknowledge stream messages"),
    ("XGROUP", "XGROUP CREATE|DESTROY|DELCONSUMER|SETID key group ... — manage consumer groups"),
    ("XINFO", "XINFO STREAM|GROUPS|CONSUMERS key — inspect stream metadata"),
    ("XDEL", "XDEL key id [id ...] — delete entries from a stream"),
    ("XTRIM", "XTRIM key MAXLEN|MINID [~] threshold — trim a stream"),
    ("XPENDING", "XPENDING key group [[IDLE ms] start end count [consumer]] — inspect pending entries"),
    ("XCLAIM", "XCLAIM key group consumer min-idle-time id [id ...] — claim pending entries"),
    ("XAUTOCLAIM", "XAUTOCLAIM key group consumer min-idle-time start [COUNT n] — auto-claim pending entries"),
    // Bitmaps & Bitfields
    ("SETBIT", "SETBIT key offset 0|1 — set or clear the bit at offset"),
    ("GETBIT", "GETBIT key offset — get the bit value at offset"),
    ("BITCOUNT", "BITCOUNT key [start end [BYTE|BIT]] — count set bits"),
    ("BITPOS", "BITPOS key 0|1 [start [end [BYTE|BIT]]] — first bit set to 0 or 1"),
    ("BITOP", "BITOP AND|OR|XOR|NOT destkey key [key ...] — bitwise operation"),
    ("BITFIELD", "BITFIELD key [GET|SET|INCRBY ...] — arbitrary bitfield operations"),
    ("BITFIELD_RO", "BITFIELD_RO key [GET type offset ...] — read-only bitfield operations"),
    // HyperLogLog
    ("PFADD", "PFADD key element [element ...] — add elements to a HyperLogLog"),
    ("PFCOUNT", "PFCOUNT key [key ...] — approximated cardinality of HyperLogLog(s)"),
    ("PFMERGE", "PFMERGE destkey sourcekey [sourcekey ...] — merge HyperLogLogs"),
    // Geospatial
    ("GEOADD", "GEOADD key [NX|XX] longitude latitude member [...] — add geo items"),
    ("GEOPOS", "GEOPOS key member [member ...] — longitude/latitude of members"),
    ("GEODIST", "GEODIST key member1 member2 [M|KM|FT|MI] — distance between members"),
    ("GEOSEARCH", "GEOSEARCH key FROMMEMBER|FROMLONLAT ... BYRADIUS|BYBOX ... — query geo area"),
    ("GEOSEARCHSTORE", "GEOSEARCHSTORE dst src FROMMEMBER|FROMLONLAT ... — store geo search"),
    ("GEOHASH", "GEOHASH key member [member ...] — Geohash strings of members"),
    // Transactions, Blocking & Session Control
    ("MULTI", "MULTI — start a transaction block on the batch connection"),
    ("EXEC", "EXEC — execute all commands queued after MULTI"),
    ("DISCARD", "DISCARD — discard all commands queued after MULTI"),
    ("WATCH", "WATCH key [key ...] — watch keys for conditional MULTI/EXEC"),
    ("UNWATCH", "UNWATCH — forget all watched keys"),
    ("SELECT", "SELECT index — switch logical database for subsequent commands in batch"),
    ("BLPOP", "BLPOP key [key ...] timeout — blocking pop from head of list"),
    ("BRPOP", "BRPOP key [key ...] timeout — blocking pop from tail of list"),
    ("BLMOVE", "BLMOVE source destination LEFT|RIGHT LEFT|RIGHT timeout — blocking atomic list move"),
    ("BLMPOP", "BLMPOP timeout numkeys key [key ...] LEFT|RIGHT [COUNT n] — blocking pop from first non-empty list"),
    ("BZPOPMIN", "BZPOPMIN key [key ...] timeout — blocking pop of lowest-scoring zset member"),
    ("BZPOPMAX", "BZPOPMAX key [key ...] timeout — blocking pop of highest-scoring zset member"),
    ("BZMPOP", "BZMPOP timeout numkeys key [key ...] MIN|MAX [COUNT n] — blocking pop from first non-empty zset"),
    ("WAIT", "WAIT numreplicas timeout — wait for replication of prior writes"),
    ("WAITAOF", "WAITAOF numlocal numreplicas timeout — wait for AOF fsync of prior writes"),
    // Scripting & Functions
    ("EVAL", "EVAL script numkeys [key ...] [arg ...] — run a Lua script"),
    ("EVAL_RO", "EVAL_RO script numkeys [key ...] [arg ...] — run a read-only Lua script"),
    ("EVALSHA", "EVALSHA sha1 numkeys [key ...] [arg ...] — run cached Lua script"),
    ("EVALSHA_RO", "EVALSHA_RO sha1 numkeys [key ...] [arg ...] — run cached read-only Lua script"),
    ("FCALL", "FCALL function numkeys [key ...] [arg ...] — invoke a Redis function"),
    ("FCALL_RO", "FCALL_RO function numkeys [key ...] [arg ...] — invoke a read-only Redis function"),
    ("FUNCTION", "FUNCTION LIST|LOAD|DELETE|STATS — manage Redis functions"),
    ("SCRIPT", "SCRIPT EXISTS|LOAD|FLUSH|KILL — manage Lua script cache"),
    // Server / Admin / PubSub
    ("INFO", "INFO [section] — server statistics and configuration"),
    ("DBSIZE", "DBSIZE — number of keys in the current database"),
    ("FLUSHDB", "FLUSHDB [ASYNC|SYNC] — remove all keys from the current database"),
    ("FLUSHALL", "FLUSHALL [ASYNC|SYNC] — remove all keys from all databases"),
    ("PING", "PING [message] — health-check the server"),
    ("ECHO", "ECHO message — echo a message back"),
    ("TIME", "TIME — current server time"),
    ("ROLE", "ROLE — replication role of the instance"),
    ("LASTSAVE", "LASTSAVE — Unix timestamp of last successful RDB save"),
    ("LOLWUT", "LOLWUT [VERSION v] — display Redis version and generative art"),
    ("CONFIG", "CONFIG GET|SET|REWRITE|RESETSTAT — read or set server config"),
    ("CLIENT", "CLIENT LIST|INFO|ID|KILL — inspect/manage client connections"),
    ("COMMAND", "COMMAND [COUNT|INFO|DOCS|LIST] — command metadata"),
    ("MEMORY", "MEMORY USAGE|STATS|DOCTOR key — inspect memory usage"),
    ("ACL", "ACL WHOAMI|LIST|GETUSER|SETUSER|DELUSER|CAT|LOG — access control lists"),
    ("SLOWLOG", "SLOWLOG GET|LEN|RESET — inspect slow queries log"),
    ("LATENCY", "LATENCY LATEST|HISTORY|DOCTOR|GRAPH — latency diagnostics"),
    ("CLUSTER", "CLUSTER INFO|NODES|SLOTS|SHARDS|KEYSLOT — cluster topology"),
    ("MODULE", "MODULE LIST|LOAD|UNLOAD — manage loaded Redis modules"),
    ("BGSAVE", "BGSAVE [SCHEDULE] — asynchronously save dataset to disk"),
    ("BGREWRITEAOF", "BGREWRITEAOF — asynchronously rewrite append-only file"),
    ("PUBLISH", "PUBLISH channel message — publish to a channel"),
    ("SPUBLISH", "SPUBLISH shardchannel message — publish to a shard channel"),
    ("PUBSUB", "PUBSUB CHANNELS|NUMSUB|NUMPAT|SHARDCHANNELS — inspect pub/sub state"),
    // Redis Stack modules (RedisJSON, RediSearch, RedisTimeSeries, Probabilistic)
    ("JSON.GET", "JSON.GET key [path ...] — get JSON value at path(s)"),
    ("JSON.SET", "JSON.SET key path json [NX|XX] — set JSON value at path"),
    ("JSON.MGET", "JSON.MGET key [key ...] path — get JSON path from multiple keys"),
    ("JSON.DEL", "JSON.DEL key [path] — delete JSON value at path"),
    ("JSON.TYPE", "JSON.TYPE key [path] — type of JSON value at path"),
    ("JSON.NUMINCRBY", "JSON.NUMINCRBY key path number — increment JSON number"),
    ("JSON.STRAPPEND", "JSON.STRAPPEND key [path] json-string — append to JSON string"),
    ("JSON.STRLEN", "JSON.STRLEN key [path] — length of JSON string"),
    ("JSON.ARRAPPEND", "JSON.ARRAPPEND key path value [value ...] — append to JSON array"),
    ("JSON.ARRLEN", "JSON.ARRLEN key [path] — length of JSON array"),
    ("JSON.OBJKEYS", "JSON.OBJKEYS key [path] — keys of JSON object"),
    ("JSON.OBJLEN", "JSON.OBJLEN key [path] — number of keys in JSON object"),
    ("FT.SEARCH", "FT.SEARCH index query [LIMIT offset num] — search a RediSearch index"),
    ("FT.AGGREGATE", "FT.AGGREGATE index query [LOAD ...] [GROUPBY ...] — aggregate index results"),
    ("FT.INFO", "FT.INFO index — metadata and stats for a RediSearch index"),
    ("FT._LIST", "FT._LIST — list all RediSearch indexes"),
    ("FT.CREATE", "FT.CREATE index [ON HASH|JSON] SCHEMA ... — create a RediSearch index"),
    ("FT.EXPLAIN", "FT.EXPLAIN index query — execution plan for a search query"),
    ("TS.ADD", "TS.ADD key timestamp value [LABELS ...] — append sample to time series"),
    ("TS.GET", "TS.GET key [LATEST] — get latest sample from time series"),
    ("TS.MGET", "TS.MGET [LATEST] FILTER filterExpr... — latest samples matching filter"),
    ("TS.RANGE", "TS.RANGE key fromTimestamp toTimestamp [AGGREGATION ...] — query time range"),
    ("TS.REVRANGE", "TS.REVRANGE key fromTimestamp toTimestamp [AGGREGATION ...] — reverse time range"),
    ("TS.MRANGE", "TS.MRANGE fromTimestamp toTimestamp FILTER filterExpr... — multi-series range"),
    ("TS.INFO", "TS.INFO key [DEBUG] — metadata and stats for a time series"),
    ("BF.ADD", "BF.ADD key item — add an item to a Bloom filter"),
    ("BF.EXISTS", "BF.EXISTS key item — check if item exists in a Bloom filter"),
    ("BF.INFO", "BF.INFO key — metadata for a Bloom filter"),
    ("CF.ADD", "CF.ADD key item — add an item to a Cuckoo filter"),
    ("CF.EXISTS", "CF.EXISTS key item — check if item exists in a Cuckoo filter"),
    ("CF.INFO", "CF.INFO key — metadata for a Cuckoo filter"),
    ("CMS.QUERY", "CMS.QUERY key item [item ...] — estimated counts from Count-Min Sketch"),
    ("CMS.INFO", "CMS.INFO key — metadata for a Count-Min Sketch"),
    ("TOPK.LIST", "TOPK.LIST key [WITHCOUNT] — full list of items in Top-K list"),
    ("TOPK.QUERY", "TOPK.QUERY key item [item ...] — check if items are in Top-K list"),
    ("TDIGEST.QUANTILE", "TDIGEST.QUANTILE key quantile [quantile ...] — estimate quantiles from t-digest"),
    ("TDIGEST.INFO", "TDIGEST.INFO key — metadata for a t-digest sketch"),
];

// --- Unit tests (pure; no network) ------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_scalar_and_nested_redis_replies_report_lossy_cells() {
        let scalar = RedisValue::BulkString(vec![b'x'; types::MAX_CELL_CHARS + 1]);
        assert!(bounded_reply_to_result(scalar.clone(), 10).cells_truncated);
        assert!(bounded_reply_to_result(RedisValue::Array(vec![scalar]), 10).cells_truncated);
        assert!(
            !bounded_reply_to_result(
                RedisValue::BulkString(b"ordinary...[truncated 1 chars]".to_vec()),
                10
            )
            .cells_truncated
        );
    }

    #[test]
    fn connection_state_commands_are_refused() {
        let p = |line: &str| split_args(line);
        // Continuous server-push streams and framing/credential desynchronizers stay refused.
        for line in [
            "SUBSCRIBE ch",
            "psubscribe news.*",
            "SSUBSCRIBE s",
            "UNSUBSCRIBE",
            "PUNSUBSCRIBE",
            "SUNSUBSCRIBE",
            "MONITOR",
            "SYNC",
            "PSYNC ? -1",
            "CLIENT REPLY OFF",
            "client reply skip",
            "CLIENT TRACKING ON",
            "RESET",
            "QUIT",
            "HELLO 3",
            "AUTH user pass",
        ] {
            let e = refuse_connection_state_command(&p(line)).unwrap_err();
            assert!(
                e.to_string().contains("not supported in the console"),
                "{line}: {e}"
            );
        }
        // Operation-isolated batch commands (SELECT, MULTI/EXEC/DISCARD/WATCH/UNWATCH,
        // blocking commands, and per-connection client modes) are allowed in `run()`.
        for ok in [
            "GET k",
            "CLIENT LIST",
            "CLIENT REPLY ON",
            "CLIENT TRACKING OFF",
            "CLIENT SETNAME tab-1",
            "CLIENT CACHING yes",
            "CLIENT NO-EVICT on",
            "CLIENT NO-TOUCH on",
            "CLIENT SETINFO LIB-NAME x",
            "READONLY",
            "READWRITE",
            "SELECT 3",
            "MULTI",
            "EXEC",
            "DISCARD",
            "WATCH k",
            "UNWATCH",
            "BLPOP jobs 1",
            "BRPOP jobs 5",
            "BLMOVE a b LEFT RIGHT 1",
            "BLMPOP 1 1 a LEFT",
            "BZPOPMIN z 1",
            "BZPOPMAX z 1",
            "BZMPOP 1 1 z MIN",
            "WAIT 1 100",
            "WAITAOF 1 1 100",
            "XREAD BLOCK 100 STREAMS s $",
            "XREADGROUP GROUP g c BLOCK 100 STREAMS s >",
            "LPOP jobs",
            "XREAD COUNT 10 STREAMS s 0",
            "ZPOPMIN z",
            "CLIENT ID",
            "XREAD STREAMS block 0",
            "XREADGROUP GROUP g c COUNT 1 STREAMS BLOCK >",
            "CLIENT KILL TYPE normal",
            "CLIENT KILL ID 7",
        ] {
            assert!(refuse_connection_state_command(&p(ok)).is_ok(), "{ok}");
        }
        // Continuous stream verbs are never offered in completions, while
        // transaction, SELECT, and blocking verbs ARE offered.
        for gone in ["SUBSCRIBE", "PSUBSCRIBE", "MONITOR", "SYNC"] {
            assert!(REDIS_COMMANDS.iter().all(|(c, _)| *c != gone), "{gone}");
        }
        for present in [
            "SELECT", "MULTI", "EXEC", "DISCARD", "WATCH", "BLPOP", "BZPOPMIN", "WAIT",
        ] {
            assert!(
                REDIS_COMMANDS.iter().any(|(c, _)| *c == present),
                "{present}"
            );
        }
    }

    #[test]
    fn binary_bulk_string_renders_base64_not_placeholder() {
        // Non-UTF-8 payloads must stay inspectable (base64), never "<N bytes>".
        let v = bytes_to_json(vec![0xff, 0xfe, 0x00, 0x01]);
        assert_eq!(v, JsonValue::String("//4AAQ==".to_string()));
        // UTF-8 stays plain text.
        assert_eq!(
            bytes_to_json(b"hello".to_vec()),
            JsonValue::String("hello".into())
        );
    }

    fn keyspace_cfg(database: Option<&str>) -> ResolvedConfig {
        ResolvedConfig {
            lifecycle: None,
            engine: Engine::Redis,
            host: "127.0.0.1".into(),
            port: 6379,
            user: None,
            password: None,
            database: database.map(str::to_string),
            tls: Default::default(),
            params: JsonValue::Null,
        }
    }

    fn scoped(node: Option<&str>) -> QueryRequest {
        QueryRequest {
            statement: "DEL session:42".into(),
            node: node.map(str::to_string),
            ..Default::default()
        }
    }

    /// Every Redis entry point (console, "Get value", grid edits/deletes, MCP)
    /// scopes with `kdb:<n>`; the service canonicalizes to the same form. The
    /// selected keyspace must reach the handshake — never the default db.
    #[test]
    fn keyspace_scope_reaches_the_driver() {
        let cfg = keyspace_cfg(None);
        // The UI / MCP / reviewed-change shapes.
        assert_eq!(keyspace_for(&cfg, &scoped(Some("kdb:3"))).unwrap(), 3);
        assert_eq!(
            keyspace_for(&cfg, &scoped(Some("kdb:3/key:session:42"))).unwrap(),
            3
        );
        // The service's canonical node for `kdb:3`.
        let canonical = crate::access::canonical_node(Some("kdb:3"));
        assert_eq!(canonical.as_deref(), Some("kdb:3"));
        assert_eq!(
            keyspace_for(&cfg, &scoped(canonical.as_deref())).unwrap(),
            3
        );
        // The pre-fix rewrite (`kdb:3` → `3`) and the MCP `database` field.
        assert_eq!(keyspace_for(&cfg, &scoped(Some("3"))).unwrap(), 3);
        assert_eq!(keyspace_for(&cfg, &scoped(Some("db:3"))).unwrap(), 3);
        assert_eq!(keyspace_for(&cfg, &scoped(Some("db3"))).unwrap(), 3);
        // No scope → the profile default, else db0.
        assert_eq!(keyspace_for(&cfg, &scoped(None)).unwrap(), 0);
        assert_eq!(
            keyspace_for(&keyspace_cfg(Some("5")), &scoped(None)).unwrap(),
            5
        );
        // A scope that names no keyspace (or a negative index) is refused,
        // never defaulted to db0.
        assert!(keyspace_for(&cfg, &scoped(Some("sessions"))).is_err());
        assert!(keyspace_for(&cfg, &scoped(Some("kdb:x"))).is_err());
        assert!(keyspace_for(&cfg, &scoped(Some("kdb:-1"))).is_err());
        assert!(keyspace_for(&cfg, &scoped(Some("db:-2"))).is_err());
        assert!(keyspace_for(&cfg, &scoped(Some("-1"))).is_err());
        assert!(keyspace_for(&keyspace_cfg(Some("-1")), &scoped(None)).is_err());
    }

    #[test]
    fn capabilities_are_honest() {
        let c = RedisDriver::default().capabilities();
        assert_eq!(c.engine, Engine::Redis);
        assert!(!c.sql && !c.joins && !c.transactions);
        // No server-side cancel and no query-plan surface.
        assert!(!c.cancel);
        assert!(!c.explain);
    }

    #[test]
    fn split_args_handles_quotes_and_spaces() {
        assert_eq!(split_args("GET app:name"), vec!["GET", "app:name"]);
        assert_eq!(
            split_args(r#"SET greeting "hello world""#),
            vec!["SET", "greeting", "hello world"]
        );
        assert_eq!(
            split_args(r#"HSET k f "a \"quoted\" b""#),
            vec!["HSET", "k", "f", r#"a "quoted" b"#]
        );
        assert!(split_args("   ").is_empty());
        // Collapses runs of whitespace between bare tokens.
        assert_eq!(split_args("MGET  a   b"), vec!["MGET", "a", "b"]);
    }

    #[test]
    fn split_args_supports_single_quotes_escapes_hex_bytes_comments_and_rejects_unclosed() {
        // Single-quoted JSON with inner double quotes and spaces stays intact.
        assert_eq!(
            split_args(r#"SET session:1 '{"user": "Ada Lovelace", "roles": ["admin", "ops"]}'"#),
            vec![
                "SET",
                "session:1",
                r#"{"user": "Ada Lovelace", "roles": ["admin", "ops"]}"#,
            ]
        );
        // Single-quoted strings only escape `\'`; `\\` retains BOTH backslashes (`sdssplitargs`).
        assert_eq!(
            split_args(r"SET note 'it\'s a\\b'"),
            vec!["SET", "note", r"it's a\\b"]
        );
        // Double-quoted unknown escapes drop the backslash (`"a\q"` -> `aq`, `"\X41"` -> `X41`).
        assert_eq!(
            split_args(r#"ECHO "a\q" "\X41""#),
            vec!["ECHO", "aq", "X41"]
        );
        // Standard C escapes and lowercase \xHH binary bytes inside double quotes.
        let raw = try_split_args(r#"SET bin "line1\nline2\t\r\x00\xff\xfe""#).unwrap();
        assert_eq!(raw[0], b"SET");
        assert_eq!(raw[1], b"bin");
        assert_eq!(raw[2], b"line1\nline2\t\r\x00\xff\xfe");
        // Empty quoted strings produce an empty argument (`SET k ""` or `SET k ''`).
        assert_eq!(split_args(r#"SET empty """#), vec!["SET", "empty", ""]);
        assert_eq!(split_args("SET empty ''"), vec!["SET", "empty", ""]);
        // Inline `#` comment after whitespace is stripped, while `#` inside a
        // token or inside quotes is preserved.
        assert_eq!(
            split_args("INCR counter:visits   # increment visit counter"),
            vec!["INCR", "counter:visits"]
        );
        assert_eq!(
            split_args(r##"HSET user#1 tag "#vip" # inline note"##),
            vec!["HSET", "user#1", "tag", "#vip"]
        );
        // Unclosed quotes or closing quotes not followed by whitespace fail with a validation error.
        assert!(try_split_args(r#"SET k "unclosed"#).is_err());
        assert!(try_split_args("SET k 'unclosed").is_err());
        assert!(try_split_args(r#"ECHO "foo"bar"#).is_err());
        assert!(try_split_args("ECHO 'foo'bar").is_err());
    }

    #[test]
    fn reply_to_result_bulk_string_is_single_cell() {
        let result = reply_to_result(RedisValue::BulkString(b"Otto Shop".to_vec()));
        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.columns[0].name, "value");
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], JsonValue::String("Otto Shop".into()));
    }

    #[test]
    fn reply_to_result_nil_is_message() {
        let result = reply_to_result(RedisValue::Nil);
        assert_eq!(result.message.as_deref(), Some("(nil)"));
    }

    #[test]
    fn reply_to_result_array_is_row_per_element() {
        let reply = RedisValue::Array(vec![
            RedisValue::BulkString(b"a".to_vec()),
            RedisValue::Int(2),
        ]);
        let result = reply_to_result(reply);
        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], JsonValue::String("a".into()));
        assert_eq!(result.rows[1][0], JsonValue::from(2));
    }

    #[test]
    fn reply_to_result_preserves_64bit_int_precision_nonfinite_floats_and_attributes() {
        // Safe JS integers stay numbers; integers past 2^53 - 1 serialize as exact strings.
        let safe = reply_to_result(RedisValue::Int(types::MAX_SAFE_JSON_INTEGER));
        assert_eq!(safe.rows[0][0], json!(types::MAX_SAFE_JSON_INTEGER));
        let unsafe_hi = reply_to_result(RedisValue::Int(types::MAX_SAFE_JSON_INTEGER + 1));
        assert_eq!(
            unsafe_hi.rows[0][0],
            JsonValue::String((types::MAX_SAFE_JSON_INTEGER + 1).to_string())
        );
        let unsafe_min = value_to_json(RedisValue::Int(i64::MIN));
        assert_eq!(unsafe_min, JsonValue::String(i64::MIN.to_string()));

        // RESP3 non-finite floats render as strings instead of collapsing to null.
        assert_eq!(
            reply_to_result(RedisValue::Double(f64::INFINITY)).rows[0][0],
            JsonValue::String("inf".into())
        );
        assert_eq!(
            reply_to_result(RedisValue::Double(f64::NEG_INFINITY)).rows[0][0],
            JsonValue::String("-inf".into())
        );
        assert_eq!(
            value_to_json(RedisValue::Double(f64::NAN)),
            JsonValue::String("nan".into())
        );
        assert_eq!(value_to_json(RedisValue::Double(3.5)), json!(3.5));

        // RESP3 Attribute wrapper unwraps cleanly for both scalar and collection replies.
        let attributed = RedisValue::Attribute {
            data: Box::new(RedisValue::Array(vec![
                RedisValue::BulkString(b"x".to_vec()),
                RedisValue::BulkString(b"y".to_vec()),
            ])),
            attributes: vec![(RedisValue::SimpleString("ttl".into()), RedisValue::Int(120))],
        };
        assert_eq!(reply_len(&attributed), 2);
        let bounded = bounded_reply_to_result(attributed, 1);
        assert_eq!(bounded.rows, vec![vec![JsonValue::String("x".into())]]);
        assert!(bounded.truncated);
    }

    #[test]
    fn statement_read_only_and_keys_classification_matches_executor_parsing() {
        assert!(statement_is_read_only(
            "GET k\n# comment\nXLEN stream:orders # inline"
        ));
        assert!(statement_is_read_only(
            "JSON.GET doc:1 $\nFT.SEARCH idx '@tag:{blue}'\nTS.GET metric:cpu\nMEMORY USAGE k"
        ));
        assert!(statement_is_read_only(
            "HTTL user:1 FIELDS 1 email\nLCS a b\nSINTERCARD 2 s1 s2\nEVAL_RO 'return 1' 0"
        ));
        assert!(statement_is_read_only(
            "CONFIG GET maxmemory\nCLIENT LIST\nCLIENT INFO\nACL WHOAMI\nACL LOG\nACL LOG 10\nSLOWLOG GET 5\nSCRIPT SHOW deadbeef"
        ));
        // Mutating commands, unknown subcommands, and unclosed/invalid quotes fail closed as writes.
        assert!(!statement_is_read_only("SET k v"));
        assert!(!statement_is_read_only("\"SET\" k v"));
        assert!(!statement_is_read_only("MEMORY PURGE"));
        assert!(!statement_is_read_only("MEMORY UNKNOWN"));
        assert!(!statement_is_read_only("CONFIG SET maxmemory 100"));
        assert!(!statement_is_read_only("CONFIG RESETSTAT"));
        assert!(!statement_is_read_only("CLIENT KILL ID 7"));
        assert!(!statement_is_read_only("ACL LOG RESET"));
        assert!(!statement_is_read_only("ACL SETUSER alice on"));
        assert!(!statement_is_read_only("SLOWLOG RESET"));
        assert!(!statement_is_read_only("JSON.SET doc:1 $ '{}'"));
        assert!(!statement_is_read_only("GET \"unclosed"));
        assert!(!statement_is_read_only("GET \"foo\"bar"));

        // KEYS detection catches bare and quoted forms while ignoring comments.
        assert!(statement_uses_keys("KEYS user:*"));
        assert!(statement_uses_keys("\"keys\" *"));
        assert!(statement_uses_keys("'KEYS' *"));
        assert!(!statement_uses_keys("# KEYS *\nGET KEYS"));
    }

    #[test]
    fn completion_catalog_covers_streams_redis7_and_modules() {
        for expected in [
            "XADD",
            "XLEN",
            "XRANGE",
            "XREADGROUP",
            "GETEX",
            "LCS",
            "EXPIRETIME",
            "HEXPIRE",
            "HTTL",
            "LMPOP",
            "ZMPOP",
            "SINTERCARD",
            "ZINTERCARD",
            "EVAL_RO",
            "FCALL_RO",
            "JSON.GET",
            "FT.SEARCH",
            "TS.RANGE",
            "BF.EXISTS",
            "TOPK.LIST",
        ] {
            assert!(
                REDIS_COMMANDS.iter().any(|(c, _)| *c == expected),
                "missing completion command: {expected}"
            );
        }
    }

    #[test]
    fn keyspace_keys_parses_stats() {
        assert_eq!(keyspace_keys("keys=11,expires=0,avg_ttl=0"), Some(11));
        assert_eq!(keyspace_keys("expires=0"), None);
    }

    #[test]
    fn info_field_extracts_version() {
        let info = "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\n";
        assert_eq!(info_field(info, "redis_version").as_deref(), Some("7.2.4"));
    }
}

#[cfg(test)]
mod perf_tests {
    use super::*;

    /// DB-07: a huge `KEYS`/`LRANGE 0 -1`-shaped reply is sliced to the row cap
    /// before conversion and charged against the response byte budget.
    #[test]
    fn big_array_reply_is_capped_before_conversion() {
        let items: Vec<RedisValue> = (0..200_000)
            .map(|i| RedisValue::BulkString(format!("key:{i}").into_bytes()))
            .collect();
        let reply = RedisValue::Array(items);
        assert_eq!(reply_len(&reply), 200_000);
        let r = bounded_reply_to_result(reply, 1_000);
        assert_eq!(r.rows.len(), 1_000);
        assert!(r.truncated);
        assert!(r.truncated_reason.is_none());
        assert_eq!(r.rows[999][0], JsonValue::String("key:999".into()));

        // Under the cap: untouched, not truncated.
        let r = bounded_reply_to_result(
            RedisValue::Map(vec![(
                RedisValue::SimpleString("a".into()),
                RedisValue::Int(1),
            )]),
            10,
        );
        assert_eq!(r.columns.len(), 2);
        assert_eq!(r.rows, vec![vec![json!("a"), json!(1)]]);
        assert!(!r.truncated);
    }

    #[test]
    fn byte_budget_stops_rows_but_keeps_at_least_one() {
        // ~1 MiB cells (each capped at the per-cell cap) blow the 32 MB budget
        // long before the 1000-row cap.
        let cell = vec![b'x'; 1 << 20];
        let items: Vec<RedisValue> = (0..200)
            .map(|_| RedisValue::BulkString(cell.clone()))
            .collect();
        let r = bounded_reply_to_result(RedisValue::Array(items), 1_000);
        assert!(r.truncated);
        assert_eq!(r.truncated_reason, Some(types::TruncatedReason::Bytes));
        assert!(!r.rows.is_empty() && r.rows.len() < 200);
    }

    #[test]
    fn scalar_replies_still_render() {
        // Non-collection replies go through the unbounded shaping unchanged.
        let ok = bounded_reply_to_result(RedisValue::Okay, 10);
        assert_eq!(ok.rows, reply_to_result(RedisValue::Okay).rows);
        assert!(!ok.truncated);
        let r = bounded_reply_to_result(RedisValue::Int(7), 10);
        assert_eq!(r.rows, vec![vec![json!(7)]]);
    }

    #[test]
    fn key_header_decodes_each_reply_independently() {
        let (ty, ttl, enc) = decode_key_header(vec![
            RedisValue::SimpleString("hash".into()),
            RedisValue::Int(-1),
            RedisValue::BulkString(b"listpack".to_vec()),
        ]);
        assert_eq!(
            (ty.as_str(), ttl, enc.as_deref()),
            ("hash", -1, Some("listpack"))
        );
        // OBJECT refused / missing → only the encoding degrades.
        let (ty, ttl, enc) = decode_key_header(vec![
            RedisValue::SimpleString("string".into()),
            RedisValue::Int(30),
            RedisValue::Nil,
        ]);
        assert_eq!((ty.as_str(), ttl, enc), ("string", 30, None));
        assert_eq!(decode_key_header(Vec::new()).0, "unknown");
    }
}
