//! Redis driver (also AWS ElastiCache — same protocol; TLS via `rediss://`).
//!
//! Connects with the `redis` crate (tokio + rustls): AUTH (user+password /
//! password-only), SELECT db, lazy `SCAN` (never `KEYS *`) for the tree,
//! type/TTL/length for object detail, a raw-command console for `run`, and a
//! command-table + live-key-prefix completion source.
//!
//! Connections are cached per [`ResolvedConfig::cache_key`] as a
//! [`redis::aio::ConnectionManager`], which is cheaply cloneable and
//! auto-reconnects — so the AUTH/handshake is paid once and reused across calls
//! instead of re-dialing every time.

use std::collections::HashMap;
use std::time::Instant;

use async_trait::async_trait;
use otto_core::Result;
use redis::aio::ConnectionManager;
use redis::{
    Client, ConnectionAddr, ConnectionInfo, IntoConnectionInfo, RedisConnectionInfo,
    TlsCertificates, Value as RedisValue,
};
use serde_json::{json, Value as JsonValue};
use tokio::sync::Mutex;

use crate::driver::Driver;
use crate::types::{
    self, Capabilities, Column, CompletionContext, CompletionItem, CompletionKind,
    CompletionResponse, Engine, NodeKind, NodePath, ObjectDetail, QueryRequest, QueryResult,
    QueryStats, ResolvedConfig, SchemaNode, TestResult,
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

/// Redis driver. Caches one [`ConnectionManager`] per [`ResolvedConfig::cache_key`].
/// `Mutex<HashMap>` is `Default`-constructible, so `#[derive(Default)]` (used
/// by the registry) still works.
#[derive(Default)]
pub struct RedisDriver {
    clients: Mutex<HashMap<String, ConnectionManager>>,
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
            default_port: 6379,
            schema_levels: vec!["Database".into(), "Namespace".into(), "Key".into()],
            query_language: "redis".into(),
        }
    }

    async fn test(&self, cfg: &ResolvedConfig) -> Result<TestResult> {
        let started = Instant::now();
        // Connect + PING; report ok:false (don't Err) on any failure so the UI
        // can show the reason inline.
        let mut conn = match self.connect(cfg).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(TestResult {
                    ok: false,
                    latency_ms: Some(started.elapsed().as_millis() as u64),
                    message: e.to_string(),
                    server_version: None,
                })
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
        let mut conn = self.connect(cfg).await?;
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
                SchemaNode::new(format!("kdb:{index}"), format!("db{index}"), NodeKind::Keyspace)
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
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| types::invalid("redis: expected a keyspace node (kdb:<n>)"))?;

        let mut conn = self.connect(cfg).await?;
        select_db(&mut conn, db).await?;

        match (parent.get("ns"), filter) {
            // Children of a namespace: keys under `<prefix>:`, optionally narrowed
            // further by the prefix filter. Flat, capped, type looked up in bulk.
            (Some(ns), f) => {
                let pattern = match f {
                    Some(f) => format!("{}:{}*", glob_escape(ns), glob_escape(f)),
                    None => format!("{}:*", glob_escape(ns)),
                };
                let scan = scan_keys(&mut conn, Some(&pattern), KEY_LIST_CAP).await?;
                Ok(build_key_nodes(&mut conn, db, scan).await)
            }
            // Keyspace WITH a prefix filter: flat list of matching keys
            // (`SCAN MATCH <filter>*`) — no namespace grouping, capped + bulk type.
            (None, Some(f)) => {
                let pattern = format!("{}*", glob_escape(f));
                let scan = scan_keys(&mut conn, Some(&pattern), KEY_LIST_CAP).await?;
                Ok(build_key_nodes(&mut conn, db, scan).await)
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
                    let detail = if scan.more { format!("{count}+") } else { count.to_string() };
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
                let bare_scan = ScanOutcome { more: scan.more, keys: bare };
                nodes.extend(build_key_nodes(&mut conn, db, bare_scan).await);
                if scan.more {
                    nodes.push(truncation_hint(db, "Showing a sample — type a prefix to filter"));
                }
                Ok(nodes)
            }
        }
    }

    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        let db = path
            .get("kdb")
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| types::invalid("redis: object detail requires a keyspace (kdb:<n>)"))?;
        let key = path
            .get("key")
            .ok_or_else(|| types::invalid("redis: object detail requires a key (key:<k>)"))?
            .to_string();

        let mut conn = self.connect(cfg).await?;
        select_db(&mut conn, db).await?;

        let ty = type_of(&mut conn, &key).await;
        let ttl: i64 = redis::cmd("TTL")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .unwrap_or(-2);
        let encoding: Option<String> = redis::cmd("OBJECT")
            .arg("ENCODING")
            .arg(&key)
            .query_async(&mut conn)
            .await
            .ok();

        let length = length_of(&mut conn, &key, &ty).await;
        let preview = preview_of(&mut conn, &key, &ty).await;

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
        // One command per line; run them all and return the last as the grid.
        let commands: Vec<Vec<String>> = req
            .statement
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(split_args)
            .filter(|parts| !parts.is_empty())
            .collect();

        if commands.is_empty() {
            return Err(types::invalid("redis: no command provided"));
        }

        let mut conn = self.connect(cfg).await?;
        // Honour an explicitly selected keyspace from the request node context.
        if let Some(node) = req.node.as_deref() {
            if let Some(db) = NodePath::parse(node).get("kdb").and_then(|s| s.parse::<i64>().ok()) {
                select_db(&mut conn, db).await?;
            }
        }

        let started = Instant::now();
        let mut last_reply = RedisValue::Nil;
        for parts in &commands {
            let mut cmd = redis::cmd(&parts[0]);
            for arg in &parts[1..] {
                cmd.arg(arg);
            }
            last_reply = cmd
                .query_async::<RedisValue>(&mut conn)
                .await
                .map_err(types::upstream)?;
        }
        let duration_ms = started.elapsed().as_millis() as u64;

        let mut result = reply_to_result(last_reply);
        result.stats.duration_ms = duration_ms;
        result.stats.row_count = result.rows.len();
        Ok(result)
    }

    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        _ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        let mut items: Vec<CompletionItem> = REDIS_COMMANDS
            .iter()
            .map(|(name, summary)| CompletionItem::detailed(*name, CompletionKind::Command, *summary))
            .collect();

        // Live key prefixes from a quick (best-effort) SCAN of the current db.
        if let Ok(mut conn) = self.connect(cfg).await {
            if let Ok(scan) = scan_keys(&mut conn, None, SCAN_KEY_CAP).await {
                use std::collections::BTreeSet;
                let mut prefixes: BTreeSet<String> = BTreeSet::new();
                for key in scan.keys {
                    if let Some((prefix, _)) = key.split_once(':') {
                        prefixes.insert(prefix.to_string());
                    }
                }
                for prefix in prefixes {
                    // No dedicated "Key" completion kind exists; a live key
                    // namespace maps best onto `Field`.
                    items.push(CompletionItem::detailed(
                        format!("{prefix}:"),
                        CompletionKind::Field,
                        "key namespace",
                    ));
                }
            }
        }

        Ok(CompletionResponse { items })
    }
}

// --- Connection -------------------------------------------------------------

/// Build a `redis::Client` for `cfg`, honouring AUTH (username/password),
/// SELECT db, and TLS (`rediss://`).
fn build_client(cfg: &ResolvedConfig) -> Result<Client> {
    let db = cfg
        .database
        .as_deref()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);

    let tls = cfg.tls.enabled();
    let addr = if tls {
        ConnectionAddr::TcpTls {
            host: cfg.host.clone(),
            port: cfg.port,
            insecure: !cfg.tls.verify,
            tls_params: None,
        }
    } else {
        ConnectionAddr::Tcp(cfg.host.clone(), cfg.port)
    };

    let mut redis_info: RedisConnectionInfo = RedisConnectionInfo::default().set_db(db);
    if let Some(user) = cfg.user.as_deref().filter(|s| !s.is_empty()) {
        redis_info = redis_info.set_username(user);
    }
    if let Some(password) = cfg.password.as_deref().filter(|s| !s.is_empty()) {
        redis_info = redis_info.set_password(password);
    }

    let conn_info: ConnectionInfo = addr
        .into_connection_info()
        .map_err(types::upstream)?
        .set_redis_settings(redis_info);

    // Inline CA / client cert → build_with_tls; otherwise the default client
    // (system roots for verified TLS, or plain TCP).
    let needs_custom_tls = tls
        && (cfg.tls.ca_cert.as_deref().is_some_and(|s| !s.is_empty())
            || (cfg.tls.client_cert.as_deref().is_some_and(|s| !s.is_empty())
                && cfg.tls.client_key.as_deref().is_some_and(|s| !s.is_empty())));

    if needs_custom_tls {
        let client_tls = match (
            cfg.tls.client_cert.as_deref().filter(|s| !s.is_empty()),
            cfg.tls.client_key.as_deref().filter(|s| !s.is_empty()),
        ) {
            (Some(cert), Some(key)) => Some(redis::ClientTlsConfig {
                client_cert: cert.as_bytes().to_vec(),
                client_key: key.as_bytes().to_vec(),
            }),
            _ => None,
        };
        let root_cert = cfg
            .tls
            .ca_cert
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|ca| ca.as_bytes().to_vec());
        Client::build_with_tls(
            conn_info,
            TlsCertificates {
                client_tls,
                root_cert,
            },
        )
        .map_err(types::upstream)
    } else {
        Client::open(conn_info).map_err(types::upstream)
    }
}

impl RedisDriver {
    /// Get (or lazily build + cache) the `ConnectionManager` for `cfg`, keyed by
    /// [`ResolvedConfig::cache_key`]. A `ConnectionManager` clone shares the
    /// underlying multiplexed connection and auto-reconnects, so reusing it
    /// across calls avoids re-dialing + re-AUTHing. Holding the tokio mutex
    /// across the connect await only briefly serializes concurrent *first*
    /// connects to the same key; cache hits return immediately.
    async fn connect(&self, cfg: &ResolvedConfig) -> Result<ConnectionManager> {
        let cache_key = cfg.cache_key();
        let mut cache = self.clients.lock().await;
        if let Some(conn) = cache.get(&cache_key) {
            return Ok(conn.clone());
        }
        let client = build_client(cfg)?;
        let manager = ConnectionManager::new(client)
            .await
            .map_err(types::upstream)?;
        cache.insert(cache_key, manager.clone());
        Ok(manager)
    }
}

async fn select_db(conn: &mut ConnectionManager, db: i64) -> Result<()> {
    if db == 0 {
        // The connection already starts on db0 via the handshake.
        return Ok(());
    }
    redis::cmd("SELECT")
        .arg(db)
        .query_async::<RedisValue>(conn)
        .await
        .map_err(types::upstream)?;
    Ok(())
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
    conn: &mut ConnectionManager,
    pattern: Option<&str>,
    key_cap: usize,
) -> Result<ScanOutcome> {
    let count = if pattern.is_some() { SCAN_COUNT_FILTER } else { SCAN_COUNT };
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
    conn: &mut ConnectionManager,
    db: i64,
    mut scan: ScanOutcome,
) -> Vec<SchemaNode> {
    scan.keys.sort();
    let types = types_of(conn, &scan.keys).await;
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
    nodes
}

/// A passive (non-clickable, non-expandable) hint row appended to a truncated
/// key listing. Rendered as a `Folder` so the tree treats it as a label only.
fn truncation_hint(db: i64, msg: &str) -> SchemaNode {
    SchemaNode::new(format!("kdb:{db}/hint:{msg}"), format!("⋯ {msg}"), NodeKind::Folder)
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
async fn types_of(conn: &mut ConnectionManager, keys: &[String]) -> Vec<String> {
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

/// `TYPE key` → "string"/"hash"/... ("none" when missing).
async fn type_of(conn: &mut ConnectionManager, key: &str) -> String {
    redis::cmd("TYPE")
        .arg(key)
        .query_async::<String>(conn)
        .await
        .unwrap_or_else(|_| "unknown".into())
}

/// Length of the value sized appropriately for its type.
async fn length_of(conn: &mut ConnectionManager, key: &str, ty: &str) -> Option<i64> {
    let cmd = match ty {
        "string" => "STRLEN",
        "hash" => "HLEN",
        "list" => "LLEN",
        "set" => "SCARD",
        "zset" => "ZCARD",
        _ => return None,
    };
    redis::cmd(cmd).arg(key).query_async::<i64>(conn).await.ok()
}

/// A bounded value preview for the structure panel, shaped per type.
async fn preview_of(conn: &mut ConnectionManager, key: &str, ty: &str) -> JsonValue {
    match ty {
        "string" => redis::cmd("GET")
            .arg(key)
            .query_async::<RedisValue>(conn)
            .await
            .map(value_to_json)
            .unwrap_or(JsonValue::Null),
        "hash" => redis::cmd("HGETALL")
            .arg(key)
            .query_async::<RedisValue>(conn)
            .await
            .map(value_to_json)
            .unwrap_or(JsonValue::Null),
        "list" => redis::cmd("LRANGE")
            .arg(key)
            .arg(0)
            .arg(PREVIEW_LIMIT)
            .query_async::<RedisValue>(conn)
            .await
            .map(value_to_json)
            .unwrap_or(JsonValue::Null),
        "set" => redis::cmd("SMEMBERS")
            .arg(key)
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
        _ => JsonValue::Null,
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

/// Convert a raw `redis::Value` reply into a [`QueryResult`] grid.
fn reply_to_result(reply: RedisValue) -> QueryResult {
    match reply {
        RedisValue::Nil => QueryResult::message("(nil)"),
        RedisValue::Okay => QueryResult::message("OK"),
        RedisValue::Int(n) => single_value(JsonValue::from(n)),
        RedisValue::Double(d) => single_value(json!(d)),
        RedisValue::Boolean(b) => single_value(JsonValue::Bool(b)),
        RedisValue::SimpleString(s) => single_value(JsonValue::String(s)),
        RedisValue::BulkString(bytes) => single_value(bytes_to_json(bytes)),
        RedisValue::VerbatimString { text, .. } => single_value(JsonValue::String(text)),
        RedisValue::BigNumber(n) => single_value(bytes_to_json(n)),
        // Array / set: one row per element (nested arrays flattened to JSON).
        RedisValue::Array(items) | RedisValue::Set(items) => array_to_result(items),
        // Map (RESP3): two columns key/value.
        RedisValue::Map(pairs) => map_to_result(pairs),
        RedisValue::ServerError(e) => QueryResult::message(format!("(error) {e:?}")),
        RedisValue::Push { kind, data } => {
            let mut items = vec![value_to_json(RedisValue::SimpleString(kind.to_string()))];
            items.extend(data.into_iter().map(value_to_json));
            single_value(JsonValue::Array(items))
        }
        // `redis::Value` is #[non_exhaustive]; render anything new as text.
        other => single_value(JsonValue::String(format!("{other:?}"))),
    }
}

fn single_value(value: JsonValue) -> QueryResult {
    QueryResult {
        columns: vec![Column::new("value")],
        rows: vec![vec![value]],
        rows_affected: None,
        stats: QueryStats::default(),
        message: None,
        truncated: false,
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
        rows_affected: None,
        stats: QueryStats::default(),
        message: None,
        truncated: false,
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
        rows_affected: None,
        stats: QueryStats::default(),
        message: None,
        truncated: false,
    }
}

/// Recursively render a `redis::Value` as a JSON value (used for cells and the
/// object-detail preview). Bulk strings become UTF-8 strings when valid.
fn value_to_json(value: RedisValue) -> JsonValue {
    match value {
        RedisValue::Nil => JsonValue::Null,
        RedisValue::Okay => JsonValue::String("OK".into()),
        RedisValue::Int(n) => JsonValue::from(n),
        RedisValue::Double(d) => json!(d),
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
        RedisValue::ServerError(e) => JsonValue::String(format!("(error) {e:?}")),
        RedisValue::Push { kind, data } => {
            let mut items = vec![JsonValue::String(kind.to_string())];
            items.extend(data.into_iter().map(value_to_json));
            JsonValue::Array(items)
        }
        // `redis::Value` is #[non_exhaustive]; render anything new as text.
        other => JsonValue::String(format!("{other:?}")),
    }
}

/// Bulk-string bytes → JSON string (UTF-8) or, when binary, a base64-ish escape.
fn bytes_to_json(bytes: Vec<u8>) -> JsonValue {
    match String::from_utf8(bytes) {
        Ok(s) => JsonValue::String(s),
        Err(e) => JsonValue::String(format!("<{} bytes>", e.into_bytes().len())),
    }
}

// --- Argument splitter ------------------------------------------------------

/// Split a single command line into args, honouring double-quoted runs so
/// values with spaces survive (`SET k "a b"` → ["SET","k","a b"]).
fn split_args(line: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if in_quotes => {
                // Allow escaped quotes/backslashes inside a quoted run.
                if let Some(&next) = chars.peek() {
                    if next == '"' || next == '\\' {
                        current.push(next);
                        chars.next();
                        continue;
                    }
                }
                current.push('\\');
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

// --- Command table ----------------------------------------------------------

/// ~80 common Redis commands with one-line summaries for autocomplete.
const REDIS_COMMANDS: &[(&str, &str)] = &[
    ("GET", "GET key — get the value of a key"),
    ("SET", "SET key value — set a key to a string value"),
    ("SETEX", "SETEX key seconds value — set with expiry"),
    ("SETNX", "SETNX key value — set only if key does not exist"),
    ("GETSET", "GETSET key value — set and return old value"),
    ("GETDEL", "GETDEL key — get value and delete the key"),
    ("APPEND", "APPEND key value — append to a string value"),
    ("STRLEN", "STRLEN key — length of a string value"),
    ("DEL", "DEL key [key ...] — delete keys"),
    ("UNLINK", "UNLINK key [key ...] — async delete keys"),
    ("EXISTS", "EXISTS key [key ...] — count existing keys"),
    ("EXPIRE", "EXPIRE key seconds — set a key's TTL"),
    ("PEXPIRE", "PEXPIRE key ms — set a key's TTL in milliseconds"),
    ("EXPIREAT", "EXPIREAT key unix-time — expire at a timestamp"),
    ("TTL", "TTL key — remaining TTL in seconds"),
    ("PTTL", "PTTL key — remaining TTL in milliseconds"),
    ("PERSIST", "PERSIST key — remove a key's TTL"),
    ("TYPE", "TYPE key — data type of a key"),
    ("RENAME", "RENAME key newkey — rename a key"),
    ("RENAMENX", "RENAMENX key newkey — rename if newkey absent"),
    ("RANDOMKEY", "RANDOMKEY — return a random key"),
    ("KEYS", "KEYS pattern — keys matching a pattern (avoid in prod)"),
    ("SCAN", "SCAN cursor [MATCH pat] [COUNT n] — incrementally iterate keys"),
    ("DUMP", "DUMP key — serialized value of a key"),
    ("RESTORE", "RESTORE key ttl value — create a key from a dump"),
    ("OBJECT", "OBJECT ENCODING|REFCOUNT|IDLETIME key — introspect a key"),
    ("HGET", "HGET key field — get a hash field"),
    ("HSET", "HSET key field value [field value ...] — set hash fields"),
    ("HSETNX", "HSETNX key field value — set field if absent"),
    ("HMGET", "HMGET key field [field ...] — get multiple hash fields"),
    ("HGETALL", "HGETALL key — all fields and values of a hash"),
    ("HDEL", "HDEL key field [field ...] — delete hash fields"),
    ("HEXISTS", "HEXISTS key field — does a hash field exist"),
    ("HKEYS", "HKEYS key — all field names of a hash"),
    ("HVALS", "HVALS key — all values of a hash"),
    ("HLEN", "HLEN key — number of fields in a hash"),
    ("HINCRBY", "HINCRBY key field n — increment a hash field"),
    ("HSCAN", "HSCAN key cursor — incrementally iterate a hash"),
    ("LPUSH", "LPUSH key value [value ...] — prepend to a list"),
    ("RPUSH", "RPUSH key value [value ...] — append to a list"),
    ("LPUSHX", "LPUSHX key value — prepend only if list exists"),
    ("RPUSHX", "RPUSHX key value — append only if list exists"),
    ("LPOP", "LPOP key [count] — remove and return head element(s)"),
    ("RPOP", "RPOP key [count] — remove and return tail element(s)"),
    ("LRANGE", "LRANGE key start stop — a range of list elements"),
    ("LLEN", "LLEN key — length of a list"),
    ("LINDEX", "LINDEX key index — element at an index"),
    ("LSET", "LSET key index value — set element at an index"),
    ("LREM", "LREM key count value — remove elements from a list"),
    ("LTRIM", "LTRIM key start stop — trim a list to a range"),
    ("SADD", "SADD key member [member ...] — add to a set"),
    ("SREM", "SREM key member [member ...] — remove from a set"),
    ("SMEMBERS", "SMEMBERS key — all members of a set"),
    ("SCARD", "SCARD key — number of members in a set"),
    ("SISMEMBER", "SISMEMBER key member — is member of a set"),
    ("SPOP", "SPOP key [count] — remove and return random member(s)"),
    ("SRANDMEMBER", "SRANDMEMBER key [count] — random member(s)"),
    ("SINTER", "SINTER key [key ...] — intersect sets"),
    ("SUNION", "SUNION key [key ...] — union sets"),
    ("SDIFF", "SDIFF key [key ...] — difference of sets"),
    ("SSCAN", "SSCAN key cursor — incrementally iterate a set"),
    ("ZADD", "ZADD key score member [score member ...] — add to a sorted set"),
    ("ZREM", "ZREM key member [member ...] — remove from a sorted set"),
    ("ZRANGE", "ZRANGE key start stop [WITHSCORES] — range by rank"),
    ("ZREVRANGE", "ZREVRANGE key start stop — range by reverse rank"),
    ("ZRANGEBYSCORE", "ZRANGEBYSCORE key min max — range by score"),
    ("ZSCORE", "ZSCORE key member — score of a member"),
    ("ZRANK", "ZRANK key member — rank of a member"),
    ("ZCARD", "ZCARD key — number of members in a sorted set"),
    ("ZINCRBY", "ZINCRBY key n member — increment a member's score"),
    ("ZCOUNT", "ZCOUNT key min max — count members in a score range"),
    ("ZSCAN", "ZSCAN key cursor — incrementally iterate a sorted set"),
    ("INCR", "INCR key — increment an integer value by 1"),
    ("DECR", "DECR key — decrement an integer value by 1"),
    ("INCRBY", "INCRBY key n — increment an integer value by n"),
    ("DECRBY", "DECRBY key n — decrement an integer value by n"),
    ("INCRBYFLOAT", "INCRBYFLOAT key n — increment a float value"),
    ("MGET", "MGET key [key ...] — get multiple string values"),
    ("MSET", "MSET key value [key value ...] — set multiple keys"),
    ("MSETNX", "MSETNX key value [key value ...] — set only if none exist"),
    ("SETRANGE", "SETRANGE key offset value — overwrite part of a string"),
    ("GETRANGE", "GETRANGE key start end — substring of a string value"),
    ("INFO", "INFO [section] — server statistics and configuration"),
    ("DBSIZE", "DBSIZE — number of keys in the current database"),
    ("SELECT", "SELECT index — switch to a logical database"),
    ("FLUSHDB", "FLUSHDB — remove all keys from the current database"),
    ("FLUSHALL", "FLUSHALL — remove all keys from all databases"),
    ("PING", "PING [message] — health-check the server"),
    ("ECHO", "ECHO message — echo a message back"),
    ("TIME", "TIME — current server time"),
    ("CONFIG", "CONFIG GET|SET parameter — read or set server config"),
    ("CLIENT", "CLIENT subcommand — inspect/manage client connections"),
    ("COMMAND", "COMMAND [COUNT|INFO|DOCS] — command metadata"),
    ("MEMORY", "MEMORY USAGE key — estimate memory used by a key"),
    ("WAIT", "WAIT numreplicas timeout — wait for replication"),
    ("MULTI", "MULTI — start a transaction"),
    ("EXEC", "EXEC — execute a queued transaction"),
    ("DISCARD", "DISCARD — discard a queued transaction"),
    ("SUBSCRIBE", "SUBSCRIBE channel [channel ...] — subscribe to channels"),
    ("PUBLISH", "PUBLISH channel message — publish to a channel"),
];

// --- Unit tests (pure; no network) ------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
