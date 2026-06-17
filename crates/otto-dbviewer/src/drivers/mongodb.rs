//! MongoDB driver (also Atlas via `mongodb+srv://`, replica sets).
//!
//! Builds `ClientOptions` from a [`ResolvedConfig`]: a full `conn_string`
//! (with `{secret}` substitution) wins, otherwise a `mongodb://` URI is
//! assembled from host/port + credential + `replica_set`/`auth_source`/TLS.
//! The tree is lazy — databases, then collections (with an estimated count),
//! then sampled top-level fields via `$sample`. `run` accepts both a JSON
//! command object and a tolerant `db.coll.find(...)` shorthand.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use async_trait::async_trait;
use futures_util::StreamExt;
use mongodb::bson::{doc, Bson, Document};
use mongodb::options::{ClientOptions, Credential, ServerAddress, Tls, TlsOptions};
use mongodb::{Client, Collection};
use otto_core::Result;
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;

use crate::driver::Driver;
use crate::tls::TlsFiles;
use crate::types::{
    self, Capabilities, Column, CompletionContext, CompletionItem, CompletionKind,
    CompletionResponse, Engine, IndexDef, NodePath, NodeKind, ObjectDetail, QueryRequest,
    QueryResult, QueryStats, ResolvedConfig, SchemaNode, TestResult,
};

/// How many documents to sample when inferring fields/types.
const SAMPLE_SIZE: i64 = 100;
/// Default row cap when a request doesn't set `max_rows`.
const DEFAULT_MAX_ROWS: usize = 50;
/// System databases sorted to the bottom of the tree.
const SYSTEM_DBS: &[&str] = &["admin", "local", "config"];

/// MongoDB driver. Caches one `mongodb::Client` per [`ResolvedConfig::cache_key`].
/// A `mongodb::Client` is internally connection-pooled and self-healing, and
/// cheap to clone (it's an `Arc` internally), so reusing it across calls avoids
/// re-establishing connections. `Mutex<HashMap>` is `Default`-constructible, so
/// `#[derive(Default)]` (used by the registry) still works.
#[derive(Default)]
pub struct MongoDriver {
    clients: Mutex<HashMap<String, Client>>,
}

#[async_trait]
impl Driver for MongoDriver {
    fn engine(&self) -> Engine {
        Engine::Mongodb
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            engine: Engine::Mongodb,
            sql: false,
            joins: false,
            transactions: true,
            multi_statement: false,
            default_port: 27017,
            schema_levels: vec!["Database".into(), "Collection".into(), "Field".into()],
            query_language: "mongo".into(),
        }
    }

    async fn test(&self, cfg: &ResolvedConfig) -> Result<TestResult> {
        let started = Instant::now();
        let client = match self.connect(cfg).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(TestResult {
                    ok: false,
                    latency_ms: None,
                    message: e.to_string(),
                    server_version: None,
                })
            }
        };

        match client.list_database_names().await {
            Ok(_) => {
                let latency = started.elapsed().as_millis() as u64;
                let version = client
                    .database("admin")
                    .run_command(doc! { "buildInfo": 1 })
                    .await
                    .ok()
                    .and_then(|d| d.get_str("version").ok().map(str::to_string));
                Ok(TestResult {
                    ok: true,
                    latency_ms: Some(latency),
                    message: "connected".into(),
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
        let client = self.connect(cfg).await?;
        let mut names = client
            .list_database_names()
            .await
            .map_err(types::upstream)?;
        // User databases first, system databases (admin/local/config) last.
        names.sort_by(|a, b| {
            let rank = |n: &str| SYSTEM_DBS.iter().position(|s| *s == n).map_or(0, |i| i + 1);
            rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
        });
        Ok(names
            .into_iter()
            .map(|name| {
                SchemaNode::new(format!("db:{name}"), name, NodeKind::Database).expandable()
            })
            .collect())
    }

    async fn schema_children(
        &self,
        cfg: &ResolvedConfig,
        parent: &NodePath,
    ) -> Result<Vec<SchemaNode>> {
        let client = self.connect(cfg).await?;
        let db_name = parent
            .get("db")
            .ok_or_else(|| types::invalid("expected a db:<name> node"))?;
        let db = client.database(db_name);

        match parent.get("coll") {
            // Collection node → sampled top-level fields.
            Some(coll_name) => {
                let coll: Collection<Document> = db.collection(coll_name);
                let fields = sample_field_types(&coll).await?;
                Ok(fields
                    .into_iter()
                    .map(|(name, ty)| {
                        let id = parent.child("field", &name).to_id();
                        SchemaNode::new(id, name, NodeKind::Field).with_detail(ty)
                    })
                    .collect())
            }
            // Database node → collections (name only; no estimated counts —
            // estimated_document_count is an estimate the user doesn't want).
            None => {
                let mut names = db
                    .list_collection_names()
                    .await
                    .map_err(types::upstream)?;
                names.sort();
                let mut nodes = Vec::with_capacity(names.len());
                for name in names {
                    let id = parent.child("coll", &name).to_id();
                    nodes.push(SchemaNode::new(id, name, NodeKind::Collection).expandable());
                }
                Ok(nodes)
            }
        }
    }

    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        let client = self.connect(cfg).await?;
        let db_name = path
            .get("db")
            .ok_or_else(|| types::invalid("expected a db:<name> node"))?;
        let coll_name = path
            .get("coll")
            .ok_or_else(|| types::invalid("object_detail expects a collection node"))?;
        let db = client.database(db_name);
        let coll: Collection<Document> = db.collection(coll_name);

        let mut detail = ObjectDetail::new(coll_name.to_string(), NodeKind::Collection);
        // row_count stays None: the only cheap source is estimated_document_count,
        // which is an estimate the user doesn't want surfaced.

        // Indexes (name + keys → IndexDef; unique from options).
        if let Ok(mut cursor) = coll.list_indexes().await {
            let mut indexes = Vec::new();
            while let Some(next) = cursor.next().await {
                if let Ok(model) = next {
                    let columns: Vec<String> =
                        model.keys.keys().map(|k| k.to_string()).collect();
                    let unique = model
                        .options
                        .as_ref()
                        .and_then(|o| o.unique)
                        .unwrap_or(false);
                    let name = model
                        .options
                        .as_ref()
                        .and_then(|o| o.name.clone())
                        .unwrap_or_else(|| join_index_keys(&model.keys));
                    indexes.push(IndexDef {
                        name,
                        columns,
                        unique,
                        method: None,
                    });
                }
            }
            detail.indexes = indexes;
        }

        // Sampled field→type map and one sample document.
        let field_types = sample_field_types(&coll).await.unwrap_or_default();
        let sample = first_document(&coll).await;

        let mut extra = Map::new();
        extra.insert(
            "sampled_fields".into(),
            Value::Object(
                field_types
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect(),
            ),
        );
        if let Some(s) = sample {
            extra.insert("sample".into(), bson_to_json(&Bson::Document(s)));
        }
        // Validator (best-effort) from listCollections options.
        if let Some(validator) = collection_validator(&db, coll_name).await {
            extra.insert("validator".into(), bson_to_json(&validator));
        }
        detail.extra = Value::Object(extra);

        Ok(detail)
    }

    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        let parsed = parse_command(&req.statement)?;
        let db_name = cfg
            .database
            .clone()
            .or_else(|| req.node.as_deref().and_then(|n| {
                NodePath::parse(n).get("db").map(str::to_string)
            }))
            .ok_or_else(|| types::invalid("no database selected for this connection"))?;

        let client = self.connect(cfg).await?;
        let db = client.database(&db_name);
        let coll: Collection<Document> = db.collection(&parsed.collection);
        let max_rows = req.max_rows.unwrap_or(DEFAULT_MAX_ROWS);
        let started = Instant::now();

        match parsed.op {
            MongoOp::Count => {
                let filter = parsed.filter.unwrap_or_default();
                let n = coll
                    .count_documents(filter)
                    .await
                    .map_err(types::upstream)?;
                let mut result = QueryResult::message(n.to_string());
                result.columns = vec![Column::typed("count", "int")];
                result.rows = vec![vec![json!(n)]];
                result.stats = QueryStats {
                    duration_ms: started.elapsed().as_millis() as u64,
                    row_count: 1,
                    bytes_read: None,
                };
                Ok(result)
            }
            MongoOp::Find => {
                let mut action = coll.find(parsed.filter.unwrap_or_default());
                if let Some(p) = parsed.projection {
                    action = action.projection(p);
                }
                if let Some(s) = parsed.sort {
                    action = action.sort(s);
                }
                let limit = parsed.limit.unwrap_or(max_rows as i64).min(max_rows as i64);
                action = action.limit(limit + 1);
                let cursor = action.await.map_err(types::upstream)?;
                collect_docs(cursor, max_rows, started).await
            }
            MongoOp::Aggregate => {
                let pipeline = parsed.pipeline.unwrap_or_default();
                let cursor = coll.aggregate(pipeline).await.map_err(types::upstream)?;
                collect_docs(cursor, max_rows, started).await
            }
        }
    }

    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        let mut items: Vec<CompletionItem> = MONGO_OPERATORS
            .iter()
            .map(|(label, detail)| {
                CompletionItem::detailed(*label, CompletionKind::Operator, *detail)
            })
            .collect();

        // Live collection + sampled-field identifiers, best-effort (never fail
        // completion if the connection is momentarily unavailable).
        let db_name = ctx
            .database
            .clone()
            .or_else(|| cfg.database.clone())
            .or_else(|| {
                ctx.node
                    .as_deref()
                    .and_then(|n| NodePath::parse(n).get("db").map(str::to_string))
            });
        if let Some(db_name) = db_name {
            if let Ok(client) = self.connect(cfg).await {
                let db = client.database(&db_name);
                if let Ok(mut names) = db.list_collection_names().await {
                    names.sort();
                    for name in &names {
                        items.push(CompletionItem::new(
                            name.clone(),
                            CompletionKind::Collection,
                        ));
                    }
                    // Sampled fields from the contextually-selected collection.
                    if let Some(coll_name) = ctx
                        .node
                        .as_deref()
                        .and_then(|n| NodePath::parse(n).get("coll").map(str::to_string))
                    {
                        let coll: Collection<Document> = db.collection(&coll_name);
                        if let Ok(fields) = sample_field_types(&coll).await {
                            for (name, ty) in fields {
                                items.push(CompletionItem::detailed(
                                    name,
                                    CompletionKind::Field,
                                    ty,
                                ));
                            }
                        }
                    }
                }
            }
        }

        Ok(CompletionResponse { items })
    }
}

impl MongoDriver {
    /// Get (or lazily build + cache) the `Client` for `cfg`, keyed by
    /// [`ResolvedConfig::cache_key`]. The client is internally pooled +
    /// self-healing and cheap to clone, so reusing it across calls amortizes
    /// connection setup. Holding the tokio mutex across the build await only
    /// briefly serializes concurrent *first* builds for the same key.
    async fn connect(&self, cfg: &ResolvedConfig) -> Result<Client> {
        let cache_key = cfg.cache_key();
        let mut cache = self.clients.lock().await;
        if let Some(client) = cache.get(&cache_key) {
            return Ok(client.clone());
        }
        let opts = self.client_options(cfg).await?;
        let client = Client::with_options(opts).map_err(types::upstream)?;
        cache.insert(cache_key, client.clone());
        Ok(client)
    }

    /// Assemble `ClientOptions`: a full `conn_string` wins (with `{secret}`
    /// substitution); otherwise host/port + credential + replica_set + TLS.
    async fn client_options(&self, cfg: &ResolvedConfig) -> Result<ClientOptions> {
        if let Some(conn_string) = cfg.param_str("conn_string") {
            let uri = match cfg.password.as_deref() {
                Some(secret) => conn_string.replace("{secret}", secret),
                None => conn_string,
            };
            return ClientOptions::parse(&uri).await.map_err(types::upstream);
        }

        let mut opts = ClientOptions::default();
        opts.hosts = vec![ServerAddress::Tcp {
            host: cfg.host.clone(),
            port: Some(cfg.port),
        }];
        opts.app_name = Some("otto-dbviewer".into());

        // Credential: root creds authenticate against `admin` by default.
        if let Some(user) = cfg.user.clone() {
            let source = cfg
                .param_str("auth_source")
                .unwrap_or_else(|| "admin".into());
            let credential = Credential::builder()
                .username(user)
                .password(cfg.password.clone())
                .source(source)
                .build();
            opts.credential = Some(credential);
        }

        if let Some(replica_set) = cfg.param_str("replica_set") {
            opts.repl_set_name = Some(replica_set);
        }

        if cfg.tls.enabled() {
            let files = TlsFiles::materialize(&cfg.tls)?;
            let mut tls = TlsOptions::default();
            tls.allow_invalid_certificates = Some(!cfg.tls.verify);
            tls.ca_file_path = files.ca;
            tls.cert_key_file_path = files.client_pair.or(files.client_cert);
            opts.tls = Some(Tls::Enabled(tls));
        }

        if let Some(db) = cfg.database.clone() {
            opts.default_database = Some(db);
        }

        Ok(opts)
    }
}

// --- command parsing --------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MongoOp {
    Find,
    Aggregate,
    Count,
}

#[derive(Debug, Default)]
struct ParsedCommand {
    collection: String,
    op_kind: Option<MongoOp>,
    filter: Option<Document>,
    projection: Option<Document>,
    sort: Option<Document>,
    limit: Option<i64>,
    pipeline: Option<Vec<Document>>,
}

/// A fully-resolved command (op is known). [`ParsedCommand`] is the mutable
/// intermediate the JSON / shorthand parsers fill; this is the finalized form.
struct Parsed {
    collection: String,
    op: MongoOp,
    filter: Option<Document>,
    projection: Option<Document>,
    sort: Option<Document>,
    limit: Option<i64>,
    pipeline: Option<Vec<Document>>,
}

/// Parse a statement into a normalized command. Supports a JSON command object
/// (`{collection, op, filter, projection, sort, limit, pipeline}`) and a
/// `db.<coll>.find({...})` / `.aggregate([...])` / `.countDocuments({...})`
/// shorthand with optional `.limit(n)` / `.sort({...})`.
fn parse_command(statement: &str) -> Result<Parsed> {
    let trimmed = statement.trim();
    if trimmed.is_empty() {
        return Err(types::invalid("empty statement"));
    }

    let parsed = if trimmed.starts_with('{') {
        parse_json_command(trimmed)?
    } else {
        parse_shorthand(trimmed)?
    };

    let op = parsed
        .op_kind
        .ok_or_else(|| types::invalid("could not determine operation (find/aggregate/count)"))?;
    if parsed.collection.is_empty() {
        return Err(types::invalid("no collection specified"));
    }
    Ok(Parsed {
        collection: parsed.collection,
        op,
        filter: parsed.filter,
        projection: parsed.projection,
        sort: parsed.sort,
        limit: parsed.limit,
        pipeline: parsed.pipeline,
    })
}

fn parse_json_command(raw: &str) -> Result<ParsedCommand> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| types::invalid(format!("invalid JSON command: {e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| types::invalid("command must be a JSON object"))?;

    let collection = obj
        .get("collection")
        .and_then(Value::as_str)
        .ok_or_else(|| types::invalid("command missing \"collection\""))?
        .to_string();

    let op_str = obj
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or(if obj.contains_key("pipeline") {
            "aggregate"
        } else {
            "find"
        });
    let op_kind = Some(op_from_str(op_str)?);

    let to_doc = |v: Option<&Value>| -> Result<Option<Document>> {
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(v) => Ok(Some(json_to_document(v)?)),
        }
    };

    let pipeline = match obj.get("pipeline") {
        Some(Value::Array(arr)) => {
            let mut stages = Vec::with_capacity(arr.len());
            for stage in arr {
                stages.push(json_to_document(stage)?);
            }
            Some(stages)
        }
        _ => None,
    };

    Ok(ParsedCommand {
        collection,
        op_kind,
        filter: to_doc(obj.get("filter"))?,
        projection: to_doc(obj.get("projection"))?,
        sort: to_doc(obj.get("sort"))?,
        limit: obj.get("limit").and_then(Value::as_i64),
        pipeline,
    })
}

/// Tolerant `db.<coll>.<op>(<arg>)` parser with optional `.limit(n)`/`.sort({})`.
fn parse_shorthand(raw: &str) -> Result<ParsedCommand> {
    let s = raw.strip_prefix("db.").unwrap_or(raw);
    // collection name = up to the first '.'
    let dot = s
        .find('.')
        .ok_or_else(|| types::invalid("expected db.<collection>.<op>(...)"))?;
    let collection = s[..dot].trim().to_string();
    let rest = &s[dot + 1..];

    let mut cmd = ParsedCommand {
        collection,
        ..Default::default()
    };

    // Walk method calls: name(args) ['.' name(args)]*
    let mut cursor = rest;
    while !cursor.trim().is_empty() {
        let cursor_t = cursor.trim_start();
        let paren = cursor_t
            .find('(')
            .ok_or_else(|| types::invalid("expected method call like find(...)"))?;
        let method = cursor_t[..paren].trim().to_string();
        let (arg, after) = extract_balanced(&cursor_t[paren..])?;

        match method.as_str() {
            "find" => {
                cmd.op_kind = Some(MongoOp::Find);
                if !arg.trim().is_empty() {
                    cmd.filter = Some(parse_doc_arg(&arg)?);
                }
            }
            "aggregate" => {
                cmd.op_kind = Some(MongoOp::Aggregate);
                cmd.pipeline = Some(parse_pipeline_arg(&arg)?);
            }
            "countDocuments" | "count" => {
                cmd.op_kind = Some(MongoOp::Count);
                if !arg.trim().is_empty() {
                    cmd.filter = Some(parse_doc_arg(&arg)?);
                }
            }
            "limit" => {
                cmd.limit = arg.trim().parse::<i64>().ok();
            }
            "sort" => {
                cmd.sort = Some(parse_doc_arg(&arg)?);
            }
            "projection" => {
                cmd.projection = Some(parse_doc_arg(&arg)?);
            }
            other => {
                return Err(types::invalid(format!("unsupported method '{other}'")));
            }
        }

        // Advance past the closing paren, then any leading '.'.
        cursor = after.trim_start();
        cursor = cursor.strip_prefix('.').unwrap_or(cursor);
    }

    Ok(cmd)
}

/// Given a string starting with '(', return (inside, remainder-after-close).
fn extract_balanced(s: &str) -> Result<(String, &str)> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'(') {
        return Err(types::invalid("expected '('"));
    }
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match in_str {
            Some(q) => {
                if b == q {
                    in_str = None;
                }
            }
            None => match b {
                b'"' | b'\'' => in_str = Some(b),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        let inside = &s[1..i];
                        return Ok((inside.to_string(), &s[i + 1..]));
                    }
                }
                _ => {}
            },
        }
    }
    Err(types::invalid("unbalanced parentheses in statement"))
}

fn parse_doc_arg(arg: &str) -> Result<Document> {
    let value: Value = serde_json::from_str(arg.trim())
        .map_err(|e| types::invalid(format!("invalid JSON argument: {e}")))?;
    json_to_document(&value)
}

fn parse_pipeline_arg(arg: &str) -> Result<Vec<Document>> {
    let value: Value = serde_json::from_str(arg.trim())
        .map_err(|e| types::invalid(format!("invalid pipeline JSON: {e}")))?;
    let arr = value
        .as_array()
        .ok_or_else(|| types::invalid("aggregate expects an array pipeline"))?;
    arr.iter().map(json_to_document).collect()
}

fn op_from_str(op: &str) -> Result<MongoOp> {
    match op {
        "find" => Ok(MongoOp::Find),
        "aggregate" => Ok(MongoOp::Aggregate),
        "count" | "countDocuments" => Ok(MongoOp::Count),
        other => Err(types::invalid(format!("unsupported op '{other}'"))),
    }
}

// --- bson / json helpers ----------------------------------------------------

/// Convert a JSON value into a BSON `Document` (the value must be an object).
fn json_to_document(v: &Value) -> Result<Document> {
    match mongodb::bson::to_bson(v).map_err(types::upstream)? {
        Bson::Document(d) => Ok(d),
        _ => Err(types::invalid("expected a JSON object")),
    }
}

/// Convert a `bson::Bson` to a clean `serde_json::Value`. ObjectId renders as
/// its hex string; dates/timestamps/decimals/binary collapse to readable
/// scalars rather than MongoDB extended-JSON wrappers.
fn bson_to_json(b: &Bson) -> Value {
    match b {
        Bson::Double(f) => json!(f),
        Bson::String(s) => Value::String(s.clone()),
        Bson::Boolean(b) => Value::Bool(*b),
        Bson::Null | Bson::Undefined => Value::Null,
        Bson::Int32(i) => json!(i),
        Bson::Int64(i) => json!(i),
        Bson::ObjectId(oid) => Value::String(oid.to_hex()),
        Bson::Array(arr) => Value::Array(arr.iter().map(bson_to_json).collect()),
        Bson::Document(doc) => Value::Object(
            doc.iter()
                .map(|(k, v)| (k.clone(), bson_to_json(v)))
                .collect(),
        ),
        Bson::DateTime(dt) => Value::String(dt.try_to_rfc3339_string().unwrap_or_else(|_| dt.to_string())),
        Bson::Timestamp(ts) => json!({ "t": ts.time, "i": ts.increment }),
        Bson::Decimal128(d) => Value::String(d.to_string()),
        Bson::Symbol(s) => Value::String(s.clone()),
        Bson::RegularExpression(re) => {
            Value::String(format!("/{}/{}", re.pattern, re.options))
        }
        Bson::JavaScriptCode(code) => Value::String(code.clone()),
        Bson::JavaScriptCodeWithScope(c) => Value::String(c.code.clone()),
        Bson::Binary(bin) => Value::String(base64_encode(&bin.bytes)),
        Bson::MaxKey => Value::String("$maxKey".into()),
        Bson::MinKey => Value::String("$minKey".into()),
        Bson::DbPointer(_) => Value::String("$dbPointer".into()),
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A short human label for a BSON type (used for field detail / completion).
fn bson_type_name(b: &Bson) -> &'static str {
    match b {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Boolean(_) => "bool",
        Bson::Null => "null",
        Bson::Undefined => "undefined",
        Bson::Int32(_) => "int32",
        Bson::Int64(_) => "int64",
        Bson::ObjectId(_) => "objectId",
        Bson::Array(_) => "array",
        Bson::Document(_) => "object",
        Bson::DateTime(_) => "date",
        Bson::Timestamp(_) => "timestamp",
        Bson::Decimal128(_) => "decimal",
        Bson::Symbol(_) => "symbol",
        Bson::RegularExpression(_) => "regex",
        Bson::JavaScriptCode(_) | Bson::JavaScriptCodeWithScope(_) => "javascript",
        Bson::Binary(_) => "binary",
        Bson::MaxKey => "maxKey",
        Bson::MinKey => "minKey",
        Bson::DbPointer(_) => "dbPointer",
    }
}

fn join_index_keys(keys: &Document) -> String {
    keys.keys().map(|k| k.to_string()).collect::<Vec<_>>().join("_")
}

// --- sampling & result shaping ----------------------------------------------

/// Sample up to [`SAMPLE_SIZE`] docs and infer a type per top-level key. The
/// first observed type wins; `_id` is always reported first.
async fn sample_field_types(coll: &Collection<Document>) -> Result<Vec<(String, String)>> {
    let pipeline = vec![doc! { "$sample": { "size": SAMPLE_SIZE } }];
    let mut cursor = coll.aggregate(pipeline).await.map_err(types::upstream)?;
    let mut order: Vec<String> = Vec::new();
    let mut types: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    while let Some(next) = cursor.next().await {
        let doc = match next {
            Ok(d) => d,
            Err(_) => continue,
        };
        for (key, value) in doc.iter() {
            if !types.contains_key(key) {
                order.push(key.clone());
                types.insert(key.clone(), bson_type_name(value).to_string());
            }
        }
    }
    // `_id` first, then insertion order.
    order.sort_by_key(|k| if k == "_id" { 0 } else { 1 });
    Ok(order
        .into_iter()
        .map(|k| {
            let ty = types.get(&k).cloned().unwrap_or_default();
            (k, ty)
        })
        .collect())
}

/// Fetch a single document for the structure-tab sample (None if empty).
async fn first_document(coll: &Collection<Document>) -> Option<Document> {
    let mut cursor = coll.find(doc! {}).limit(1).await.ok()?;
    match cursor.next().await {
        Some(Ok(doc)) => Some(doc),
        _ => None,
    }
}

/// Best-effort collection validator from `listCollections`.
async fn collection_validator(db: &mongodb::Database, coll_name: &str) -> Option<Bson> {
    let cmd = doc! {
        "listCollections": 1,
        "filter": { "name": coll_name },
    };
    let reply = db.run_command(cmd).await.ok()?;
    let batch = reply
        .get_document("cursor")
        .ok()?
        .get_array("firstBatch")
        .ok()?;
    let first = batch.first()?.as_document()?;
    first
        .get_document("options")
        .ok()?
        .get("validator")
        .cloned()
}

/// Drain a document cursor into a tabular [`QueryResult`]. Columns are the union
/// of top-level keys (stable order, `_id` first). Sets `truncated` when capped.
async fn collect_docs(
    mut cursor: mongodb::Cursor<Document>,
    max_rows: usize,
    started: Instant,
) -> Result<QueryResult> {
    let mut docs: Vec<Document> = Vec::new();
    let mut truncated = false;
    while let Some(next) = cursor.next().await {
        let doc = next.map_err(types::upstream)?;
        if docs.len() >= max_rows {
            truncated = true;
            break;
        }
        docs.push(doc);
    }

    // Union of top-level keys in stable first-seen order, `_id` pinned first.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut columns: Vec<String> = Vec::new();
    if docs.iter().any(|d| d.contains_key("_id")) {
        columns.push("_id".into());
        seen.insert("_id".into());
    }
    for doc in &docs {
        for key in doc.keys() {
            if seen.insert(key.clone()) {
                columns.push(key.clone());
            }
        }
    }

    let rows: Vec<Vec<Value>> = docs
        .iter()
        .map(|doc| {
            columns
                .iter()
                .map(|col| doc.get(col).map(bson_to_json).unwrap_or(Value::Null))
                .collect()
        })
        .collect();

    let row_count = rows.len();
    Ok(QueryResult {
        columns: columns.into_iter().map(Column::new).collect(),
        rows,
        rows_affected: None,
        stats: QueryStats {
            duration_ms: started.elapsed().as_millis() as u64,
            row_count,
            bytes_read: None,
        },
        message: None,
        truncated,
    })
}

// --- aggregation operator/stage catalog for completion ----------------------

const MONGO_OPERATORS: &[(&str, &str)] = &[
    // pipeline stages
    ("$match", "stage: filter documents"),
    ("$group", "stage: group by _id and accumulate"),
    ("$project", "stage: reshape documents"),
    ("$sort", "stage: order documents"),
    ("$limit", "stage: cap document count"),
    ("$skip", "stage: skip N documents"),
    ("$lookup", "stage: left outer join another collection"),
    ("$unwind", "stage: deconstruct an array field"),
    ("$count", "stage: count documents into a field"),
    ("$addFields", "stage: add computed fields"),
    ("$set", "stage: add/replace fields"),
    ("$unset", "stage: remove fields"),
    ("$replaceRoot", "stage: promote a document to root"),
    ("$facet", "stage: multiple sub-pipelines"),
    ("$bucket", "stage: group into buckets"),
    ("$sample", "stage: random sample of documents"),
    ("$out", "stage: write results to a collection"),
    ("$merge", "stage: merge results into a collection"),
    // query / comparison operators
    ("$eq", "match values equal to"),
    ("$ne", "match values not equal to"),
    ("$gt", "match values greater than"),
    ("$gte", "match values greater than or equal"),
    ("$lt", "match values less than"),
    ("$lte", "match values less than or equal"),
    ("$in", "match any value in an array"),
    ("$nin", "match no value in an array"),
    ("$exists", "match documents with the field"),
    ("$type", "match by BSON type"),
    ("$regex", "match by regular expression"),
    // logical
    ("$and", "logical AND"),
    ("$or", "logical OR"),
    ("$not", "logical NOT"),
    ("$nor", "logical NOR"),
    // accumulators / expressions
    ("$sum", "accumulate a sum"),
    ("$avg", "accumulate an average"),
    ("$min", "accumulate the minimum"),
    ("$max", "accumulate the maximum"),
    ("$first", "first value in a group"),
    ("$last", "last value in a group"),
    ("$push", "append values to an array"),
    ("$addToSet", "append unique values to an array"),
    ("$concat", "concatenate strings"),
    ("$cond", "conditional expression"),
    ("$ifNull", "fallback when null"),
    ("$size", "array length"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_find_basic() {
        let p = parse_command("db.customers.find({})").unwrap();
        assert_eq!(p.collection, "customers");
        assert_eq!(p.op, MongoOp::Find);
    }

    #[test]
    fn shorthand_find_with_filter_limit_sort() {
        let p = parse_command(
            r#"db.orders.find({"status":"paid"}).sort({"total":-1}).limit(5)"#,
        )
        .unwrap();
        assert_eq!(p.collection, "orders");
        assert_eq!(p.op, MongoOp::Find);
        assert_eq!(p.limit, Some(5));
        assert_eq!(
            p.filter.unwrap().get_str("status").unwrap(),
            "paid"
        );
        // serde_json integers become BSON Int64.
        assert_eq!(p.sort.unwrap().get_i64("total").unwrap(), -1);
    }

    #[test]
    fn shorthand_aggregate() {
        let p = parse_command(r#"db.events.aggregate([{"$match":{"k":1}},{"$count":"n"}])"#)
            .unwrap();
        assert_eq!(p.collection, "events");
        assert_eq!(p.op, MongoOp::Aggregate);
        assert_eq!(p.pipeline.unwrap().len(), 2);
    }

    #[test]
    fn shorthand_count() {
        let p = parse_command(r#"db.users.countDocuments({"active":true})"#).unwrap();
        assert_eq!(p.collection, "users");
        assert_eq!(p.op, MongoOp::Count);
    }

    #[test]
    fn json_command_form() {
        let p = parse_command(
            r#"{"collection":"products","op":"find","filter":{"price":{"$gt":10}},"limit":3}"#,
        )
        .unwrap();
        assert_eq!(p.collection, "products");
        assert_eq!(p.op, MongoOp::Find);
        assert_eq!(p.limit, Some(3));
    }

    #[test]
    fn json_command_pipeline_defaults_to_aggregate() {
        let p = parse_command(r#"{"collection":"e","pipeline":[{"$count":"n"}]}"#).unwrap();
        assert_eq!(p.op, MongoOp::Aggregate);
        assert_eq!(p.pipeline.unwrap().len(), 1);
    }

    #[test]
    fn bson_to_json_simple_doc() {
        let oid = mongodb::bson::oid::ObjectId::new();
        let doc = doc! {
            "_id": oid,
            "email": "a@b.com",
            "n": 42i32,
            "active": true,
            "tags": ["x", "y"],
        };
        let v = bson_to_json(&Bson::Document(doc));
        let obj = v.as_object().unwrap();
        assert_eq!(obj.get("_id").unwrap(), &Value::String(oid.to_hex()));
        assert_eq!(obj.get("email").unwrap(), "a@b.com");
        assert_eq!(obj.get("n").unwrap(), &json!(42));
        assert_eq!(obj.get("active").unwrap(), &Value::Bool(true));
        assert_eq!(obj.get("tags").unwrap(), &json!(["x", "y"]));
    }
}
