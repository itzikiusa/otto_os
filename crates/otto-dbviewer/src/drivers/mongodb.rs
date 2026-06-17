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
use crate::drivers::mongo_sql;
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
        _filter: Option<&str>,
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
        // Collection stats (best-effort): doc count, data/storage size, index sizes.
        if let Ok(stats) = db
            .run_command(doc! { "collStats": coll_name, "scale": 1 })
            .await
        {
            let mut s = Map::new();
            for k in [
                "count",
                "size",
                "storageSize",
                "avgObjSize",
                "nindexes",
                "totalIndexSize",
                "totalSize",
            ] {
                if let Some(v) = stats.get(k) {
                    s.insert(k.to_string(), bson_to_json(v));
                }
            }
            match stats.get("count") {
                Some(Bson::Int64(c)) => detail.row_count = Some(*c),
                Some(Bson::Int32(c)) => detail.row_count = Some(*c as i64),
                Some(Bson::Double(c)) => detail.row_count = Some(*c as i64),
                _ => {}
            }
            if !s.is_empty() {
                extra.insert("stats".into(), Value::Object(s));
            }
        }
        detail.extra = Value::Object(extra);

        Ok(detail)
    }

    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        // A `SELECT …` is translated to Mongo shorthand and run as such; the
        // generated command is surfaced back to the user. Anything else is the
        // native `db.coll.…` shorthand or a JSON command.
        let translated = if mongo_sql::looks_like_sql(&req.statement) {
            Some(mongo_sql::translate(&req.statement)?)
        } else {
            None
        };
        let parsed = parse_command(translated.as_deref().unwrap_or(&req.statement))?;
        // The active database arrives in `req.node` as a plain name (the UI's
        // active-DB selector, e.g. "promotions"), matching how SQL engines treat
        // `node`. Tolerate a structured NodePath (`db:<name>/…`) too. Fall back to
        // the connection's configured default database.
        let db_name = cfg
            .database
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                req.node
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|n| {
                        NodePath::parse(n)
                            .get("db")
                            .map(str::to_string)
                            .unwrap_or_else(|| n.to_string())
                    })
            })
            .ok_or_else(|| types::invalid("no database selected for this connection"))?;

        let client = self.connect(cfg).await?;
        let db = client.database(&db_name);
        let coll: Collection<Document> = db.collection(&parsed.collection);
        let max_rows = req.max_rows.unwrap_or(DEFAULT_MAX_ROWS);
        let started = Instant::now();

        // `.explain()` (or the request's explain flag) → return the query plan.
        if parsed.explain || req.explain {
            return explain_plan(&db, &parsed, started).await;
        }

        let mut result = match parsed.op {
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
                // Cap collection at the effective limit (not just max_rows) so an
                // explicit `.limit(n)` is honored; the extra fetched row flags truncation.
                collect_docs(cursor, limit as usize, started).await
            }
            MongoOp::Aggregate => {
                let pipeline = parsed.pipeline.unwrap_or_default();
                let cursor = coll.aggregate(pipeline).await.map_err(types::upstream)?;
                collect_docs(cursor, max_rows, started).await
            }
            MongoOp::UpdateOne | MongoOp::UpdateMany => {
                let filter = parsed.filter.unwrap_or_default();
                let update = parsed
                    .update
                    .ok_or_else(|| types::invalid("update requires an update document"))?;
                let res = if matches!(parsed.op, MongoOp::UpdateOne) {
                    coll.update_one(filter, update).await
                } else {
                    coll.update_many(filter, update).await
                }
                .map_err(types::upstream)?;
                Ok(write_result(
                    res.modified_count,
                    format!(
                        "matched {}, modified {}",
                        res.matched_count, res.modified_count
                    ),
                    started,
                ))
            }
            MongoOp::InsertOne | MongoOp::InsertMany => {
                let docs = parsed.documents.unwrap_or_default();
                if docs.is_empty() {
                    return Err(types::invalid("insert requires at least one document"));
                }
                let n = if matches!(parsed.op, MongoOp::InsertOne) {
                    coll.insert_one(docs.into_iter().next().unwrap())
                        .await
                        .map_err(types::upstream)?;
                    1
                } else {
                    coll.insert_many(&docs).await.map_err(types::upstream)?;
                    docs.len() as u64
                };
                Ok(write_result(n, format!("inserted {n}"), started))
            }
            MongoOp::DeleteOne | MongoOp::DeleteMany => {
                let filter = parsed.filter.unwrap_or_default();
                let res = if matches!(parsed.op, MongoOp::DeleteOne) {
                    coll.delete_one(filter).await
                } else {
                    coll.delete_many(filter).await
                }
                .map_err(types::upstream)?;
                Ok(write_result(
                    res.deleted_count,
                    format!("deleted {}", res.deleted_count),
                    started,
                ))
            }
            MongoOp::CreateIndex => {
                let keys = parsed
                    .index_keys
                    .ok_or_else(|| types::invalid("createIndex requires a key spec"))?;
                let name = index_name_for(&keys);
                let mut spec = doc! { "key": keys, "name": &name };
                if let Some(opts) = parsed.index_options {
                    for (k, v) in opts {
                        spec.insert(k, v);
                    }
                }
                db.run_command(doc! { "createIndexes": &parsed.collection, "indexes": [spec] })
                    .await
                    .map_err(types::upstream)?;
                Ok(write_result(1, format!("created index {name}"), started))
            }
            MongoOp::DropIndex => {
                let name = parsed
                    .index_name
                    .ok_or_else(|| types::invalid("dropIndex requires an index name"))?;
                db.run_command(doc! { "dropIndexes": &parsed.collection, "index": &name })
                    .await
                    .map_err(types::upstream)?;
                Ok(write_result(1, format!("dropped index {name}"), started))
            }
        }?;

        // Show the user the Mongo command we ran on their behalf.
        if let Some(t) = translated {
            result.message = Some(format!("Translated from SQL → {t}"));
        }
        Ok(result)
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

        // Collection operations (the `db.<coll>.<op>(…)` verbs) so typing after a
        // collection offers find/aggregate/count/distinct.
        for (label, detail) in MONGO_METHODS {
            items.push(CompletionItem::detailed(
                *label,
                CompletionKind::Command,
                *detail,
            ));
        }

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
    UpdateOne,
    UpdateMany,
    InsertOne,
    InsertMany,
    DeleteOne,
    DeleteMany,
    CreateIndex,
    DropIndex,
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
    /// Update modifications for update{One,Many} (e.g. `{ "$set": {…} }`).
    update: Option<Document>,
    /// Documents to insert for insert{One,Many}.
    documents: Option<Vec<Document>>,
    /// Index key spec for createIndex (e.g. `{ "field": 1 }`).
    index_keys: Option<Document>,
    /// Index options for createIndex (e.g. `{ "unique": true }`).
    index_options: Option<Document>,
    /// Index name (or key spec as a string) for dropIndex.
    index_name: Option<String>,
    /// True when `.explain()` was chained — return the query plan, don't execute.
    explain: bool,
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
    update: Option<Document>,
    documents: Option<Vec<Document>>,
    index_keys: Option<Document>,
    index_options: Option<Document>,
    index_name: Option<String>,
    explain: bool,
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
        update: parsed.update,
        documents: parsed.documents,
        index_keys: parsed.index_keys,
        index_options: parsed.index_options,
        index_name: parsed.index_name,
        explain: parsed.explain,
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

    let documents = match obj.get("documents") {
        Some(Value::Array(arr)) => {
            let mut docs = Vec::with_capacity(arr.len());
            for d in arr {
                docs.push(json_to_document(d)?);
            }
            Some(docs)
        }
        _ => match obj.get("document") {
            Some(v) if !v.is_null() => Some(vec![json_to_document(v)?]),
            _ => None,
        },
    };

    Ok(ParsedCommand {
        collection,
        op_kind,
        filter: to_doc(obj.get("filter"))?,
        projection: to_doc(obj.get("projection"))?,
        sort: to_doc(obj.get("sort"))?,
        limit: obj.get("limit").and_then(Value::as_i64),
        pipeline,
        update: to_doc(obj.get("update"))?,
        documents,
        index_keys: to_doc(obj.get("index_keys"))?,
        index_options: to_doc(obj.get("index_options"))?,
        index_name: obj.get("index_name").and_then(Value::as_str).map(str::to_string),
        explain: obj.get("explain").and_then(Value::as_bool).unwrap_or(false),
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
                // `find(filter)` or `find(filter, projection)` (mongosh 2-arg form).
                let parts = split_top_level_args(&arg);
                if let Some(f) = parts.first() {
                    if !f.trim().is_empty() {
                        cmd.filter = Some(parse_doc_arg(f)?);
                    }
                }
                if let Some(p) = parts.get(1) {
                    if !p.trim().is_empty() {
                        cmd.projection = Some(parse_doc_arg(p)?);
                    }
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
            "updateOne" | "updateMany" => {
                cmd.op_kind = Some(if method == "updateOne" {
                    MongoOp::UpdateOne
                } else {
                    MongoOp::UpdateMany
                });
                let parts = split_top_level_args(&arg);
                let filter = parts
                    .first()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("update requires a filter"))?;
                let update = parts
                    .get(1)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("update requires an update document"))?;
                cmd.filter = Some(parse_doc_arg(filter)?);
                cmd.update = Some(parse_doc_arg(update)?);
            }
            "insertOne" => {
                cmd.op_kind = Some(MongoOp::InsertOne);
                cmd.documents = Some(vec![parse_doc_arg(&arg)?]);
            }
            "insertMany" => {
                cmd.op_kind = Some(MongoOp::InsertMany);
                cmd.documents = Some(parse_pipeline_arg(&arg)?);
            }
            "deleteOne" | "deleteMany" => {
                cmd.op_kind = Some(if method == "deleteOne" {
                    MongoOp::DeleteOne
                } else {
                    MongoOp::DeleteMany
                });
                if !arg.trim().is_empty() {
                    cmd.filter = Some(parse_doc_arg(&arg)?);
                }
            }
            "createIndex" => {
                cmd.op_kind = Some(MongoOp::CreateIndex);
                let parts = split_top_level_args(&arg);
                let keys = parts
                    .first()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("createIndex requires a key spec"))?;
                cmd.index_keys = Some(parse_doc_arg(keys)?);
                if let Some(opts) = parts.get(1).filter(|s| !s.trim().is_empty()) {
                    cmd.index_options = Some(parse_doc_arg(opts)?);
                }
            }
            "dropIndex" => {
                cmd.op_kind = Some(MongoOp::DropIndex);
                cmd.index_name = Some(arg.trim().trim_matches('"').trim_matches('\'').to_string());
            }
            "explain" => {
                // A trailing `.explain()` modifies the preceding find/aggregate.
                cmd.explain = true;
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

/// Split a call's argument string on top-level commas (ignoring commas inside
/// `{}`/`[]`/`()` or string literals), e.g. `find(filter, projection)`.
fn split_top_level_args(arg: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_str: Option<u8> = None;
    let mut start = 0usize;
    let bytes = arg.as_bytes();
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
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    parts.push(arg[start..i].to_string());
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    parts.push(arg[start..].to_string());
    parts
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
        "updateOne" => Ok(MongoOp::UpdateOne),
        "updateMany" => Ok(MongoOp::UpdateMany),
        "insertOne" => Ok(MongoOp::InsertOne),
        "insertMany" => Ok(MongoOp::InsertMany),
        "deleteOne" => Ok(MongoOp::DeleteOne),
        "deleteMany" => Ok(MongoOp::DeleteMany),
        "createIndex" => Ok(MongoOp::CreateIndex),
        "dropIndex" => Ok(MongoOp::DropIndex),
        other => Err(types::invalid(format!("unsupported op '{other}'"))),
    }
}

// --- bson / json helpers ----------------------------------------------------

/// Convert a JSON value into a BSON `Document` (the value must be an object).
fn json_to_document(v: &Value) -> Result<Document> {
    match json_to_bson(v)? {
        Bson::Document(d) => Ok(d),
        _ => Err(types::invalid("expected a JSON object")),
    }
}

/// JSON → BSON, honoring the `{"$oid": "<hex>"}` extended-JSON form so edits can
/// target an `_id` (rendered to the grid as a hex string) by its real ObjectId.
fn json_to_bson(v: &Value) -> Result<Bson> {
    match v {
        Value::Object(map) => {
            if map.len() == 1 {
                if let Some(Value::String(hex)) = map.get("$oid") {
                    return mongodb::bson::oid::ObjectId::parse_str(hex)
                        .map(Bson::ObjectId)
                        .map_err(|e| types::invalid(format!("invalid $oid: {e}")));
                }
            }
            let mut doc = Document::new();
            for (k, val) in map {
                doc.insert(k.clone(), json_to_bson(val)?);
            }
            Ok(Bson::Document(doc))
        }
        Value::Array(arr) => {
            let items: Result<Vec<Bson>> = arr.iter().map(json_to_bson).collect();
            Ok(Bson::Array(items?))
        }
        _ => mongodb::bson::to_bson(v).map_err(types::upstream),
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
/// Mongo's default index name for a key spec, e.g. `{a:1,b:-1}` → `a_1_b_-1`.
fn index_name_for(keys: &Document) -> String {
    keys.iter()
        .map(|(k, v)| {
            let dir = v
                .as_i32()
                .map(|i| i.to_string())
                .or_else(|| v.as_i64().map(|i| i.to_string()))
                .or_else(|| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "1".to_string());
            format!("{k}_{dir}")
        })
        .collect::<Vec<_>>()
        .join("_")
}

/// Run the server `explain` command (queryPlanner verbosity) for a find/aggregate
/// and return the plan document as a single JSON cell.
async fn explain_plan(
    db: &mongodb::Database,
    parsed: &Parsed,
    started: Instant,
) -> Result<QueryResult> {
    let inner = match parsed.op {
        MongoOp::Find => {
            let mut d = doc! { "find": &parsed.collection };
            if let Some(f) = &parsed.filter {
                d.insert("filter", f.clone());
            }
            if let Some(p) = &parsed.projection {
                d.insert("projection", p.clone());
            }
            if let Some(s) = &parsed.sort {
                d.insert("sort", s.clone());
            }
            if let Some(l) = parsed.limit {
                d.insert("limit", l);
            }
            d
        }
        MongoOp::Aggregate => {
            let stages: Vec<Bson> = parsed
                .pipeline
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(Bson::Document)
                .collect();
            doc! { "aggregate": &parsed.collection, "pipeline": Bson::Array(stages), "cursor": {} }
        }
        _ => return Err(types::invalid("explain supports find and aggregate")),
    };
    let plan = db
        .run_command(doc! { "explain": inner, "verbosity": "queryPlanner" })
        .await
        .map_err(types::upstream)?;
    let mut result = QueryResult::empty();
    result.columns = vec![Column::typed("queryPlan", "json")];
    result.rows = vec![vec![bson_to_json(&Bson::Document(plan))]];
    result.stats = QueryStats {
        duration_ms: started.elapsed().as_millis() as u64,
        row_count: 1,
        bytes_read: None,
    };
    result.message = Some("Query plan (explain · queryPlanner)".into());
    Ok(result)
}

/// Build a `QueryResult` for a write op (no rows, just affected count + note).
fn write_result(affected: u64, message: String, started: Instant) -> QueryResult {
    let mut result = QueryResult::message(message);
    result.rows_affected = Some(affected);
    result.stats = QueryStats {
        duration_ms: started.elapsed().as_millis() as u64,
        row_count: 0,
        bytes_read: None,
    };
    result
}

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

/// Collection operations supported by the runner's `db.<coll>.<op>(…)` shorthand.
const MONGO_METHODS: &[(&str, &str)] = &[
    ("find", "read documents matching a filter"),
    ("aggregate", "run an aggregation pipeline"),
    ("countDocuments", "count documents matching a filter"),
];

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

/// End-to-end SQL → Mongo over a real MongoDB Docker container. Ignored by
/// default (needs Docker). Run with:
///   cargo test -p otto-dbviewer --lib -- --ignored --nocapture sql_to_mongo_e2e
#[cfg(test)]
mod sql_e2e {
    use super::*;
    use crate::driver::Driver;
    use crate::types::{Engine, QueryRequest, ResolvedConfig, TlsConfig};
    use mongodb::bson::{doc, Document};
    use mongodb::Client;
    use std::process::Command;
    use std::time::Duration;

    const PORT: u16 = 47019;
    const CONTAINER: &str = "otto-mongo-e2e";
    const IMAGE: &str = "mongo:8.2";

    /// Removes the container even if an assertion panics.
    struct Cleanup;
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = Command::new("docker").args(["rm", "-f", CONTAINER]).output();
        }
    }

    fn cfg() -> ResolvedConfig {
        ResolvedConfig {
            engine: Engine::Mongodb,
            host: "127.0.0.1".into(),
            port: PORT,
            user: None,
            password: None,
            database: Some("shop".into()),
            tls: TlsConfig::default(),
            params: serde_json::json!({}),
        }
    }

    async fn run_sql(d: &MongoDriver, sql: &str) -> QueryResult {
        d.run(
            &cfg(),
            &QueryRequest {
                statement: sql.into(),
                max_rows: Some(1000),
                ..Default::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("run failed for `{sql}`: {e:?}"))
    }

    fn cell<'a>(r: &'a QueryResult, row: usize, col: &str) -> &'a Value {
        let idx = r
            .columns
            .iter()
            .position(|c| c.name == col)
            .unwrap_or_else(|| panic!("missing column `{col}`"));
        &r.rows[row][idx]
    }

    async fn wait_for_mongo(uri: &str) -> Client {
        for _ in 0..60 {
            if let Ok(c) = Client::with_uri_str(uri).await {
                if c.database("admin").run_command(doc! {"ping": 1}).await.is_ok() {
                    return c;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        panic!("mongo container never became ready");
    }

    async fn seed(client: &Client) {
        let db = client.database("shop");
        db.collection::<Document>("players").drop().await.ok();
        db.collection::<Document>("accounts").drop().await.ok();
        db.collection::<Document>("players")
            .insert_many(vec![
                doc! {"id": 1, "name": "alice", "age": 35, "country": "US"},
                doc! {"id": 2, "name": "bob", "age": 42, "country": "US"},
                doc! {"id": 3, "name": "carol", "age": 28, "country": "CA"},
                doc! {"id": 4, "name": "dave", "age": 51, "country": "UK"},
                doc! {"id": 5, "name": "amy", "age": 25, "country": "US"},
            ])
            .await
            .unwrap();
        db.collection::<Document>("accounts")
            .insert_many(vec![
                doc! {"player_id": 1, "balance": 500},
                doc! {"player_id": 2, "balance": 50},
                doc! {"player_id": 3, "balance": 150},
            ])
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires docker"]
    async fn sql_to_mongo_e2e() {
        let _ = Command::new("docker").args(["rm", "-f", CONTAINER]).output();
        let out = Command::new("docker")
            .args([
                "run",
                "-d",
                "--name",
                CONTAINER,
                "-p",
                &format!("{PORT}:27017"),
                IMAGE,
            ])
            .output()
            .expect("docker run");
        assert!(
            out.status.success(),
            "docker run failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _cleanup = Cleanup;

        let client = wait_for_mongo(&format!("mongodb://127.0.0.1:{PORT}")).await;
        seed(&client).await;
        let d = MongoDriver::default();

        // 1. plain find — all rows.
        assert_eq!(run_sql(&d, "SELECT * FROM players").await.rows.len(), 5);

        // 2. projection + `>` + ORDER BY DESC + LIMIT (and the SQL banner).
        let r = run_sql(
            &d,
            "SELECT name, age FROM players WHERE age > 30 ORDER BY age DESC LIMIT 2",
        )
        .await;
        assert_eq!(r.rows.len(), 2);
        assert_eq!(cell(&r, 0, "name"), &json!("dave")); // 51 first
        assert_eq!(cell(&r, 1, "name"), &json!("bob")); // then 42
        assert!(r
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Translated from SQL"));

        // 3. COUNT(*) + `=`.
        assert_eq!(
            run_sql(&d, "SELECT COUNT(*) FROM players WHERE country = 'US'").await.rows[0][0],
            json!(3)
        );

        // 4–8. IN / NOT IN / LIKE / NOT(...) / BETWEEN.
        assert_eq!(run_sql(&d, "SELECT * FROM players WHERE country IN ('CA','UK')").await.rows.len(), 2);
        assert_eq!(run_sql(&d, "SELECT * FROM players WHERE country NOT IN ('US')").await.rows.len(), 2);
        assert_eq!(run_sql(&d, "SELECT * FROM players WHERE name LIKE 'a%'").await.rows.len(), 2);
        assert_eq!(run_sql(&d, "SELECT * FROM players WHERE NOT (country = 'US')").await.rows.len(), 2);
        assert_eq!(run_sql(&d, "SELECT * FROM players WHERE age BETWEEN 26 AND 36").await.rows.len(), 2);

        // 9. GROUP BY aggregate → 3 country groups.
        assert_eq!(
            run_sql(&d, "SELECT country, COUNT(*) AS n FROM players GROUP BY country").await.rows.len(),
            3
        );

        // 10. global aggregate (no GROUP BY).
        assert_eq!(run_sql(&d, "SELECT AVG(age) AS avg_age FROM players").await.rows.len(), 1);

        // 11. INNER JOIN — only accounts with balance > 100 (alice 500, carol 150).
        assert_eq!(
            run_sql(
                &d,
                "SELECT p.name, a.balance FROM players p JOIN accounts a ON p.id = a.player_id WHERE a.balance > 100",
            )
            .await
            .rows
            .len(),
            2
        );

        // 12. LEFT JOIN — every player kept, even those without an account.
        assert_eq!(
            run_sql(
                &d,
                "SELECT * FROM players p LEFT JOIN accounts a ON p.id = a.player_id",
            )
            .await
            .rows
            .len(),
            5
        );

        // 13. updateOne by field — value actually changes.
        run_sql(&d, r#"db.players.updateOne({"id": 1}, {"$set": {"age": 99}})"#).await;
        assert_eq!(
            run_sql(&d, "SELECT age FROM players WHERE id = 1").await.rows[0]
                [run_sql(&d, "SELECT age FROM players WHERE id = 1")
                    .await
                    .columns
                    .iter()
                    .position(|c| c.name == "age")
                    .unwrap()],
            json!(99)
        );

        // 14. insertOne then deleteOne — row count round-trips.
        run_sql(
            &d,
            r#"db.players.insertOne({"id": 6, "name": "zoe", "age": 30, "country": "US"})"#,
        )
        .await;
        assert_eq!(run_sql(&d, "SELECT * FROM players").await.rows.len(), 6);
        run_sql(&d, r#"db.players.deleteOne({"id": 6})"#).await;
        assert_eq!(run_sql(&d, "SELECT * FROM players").await.rows.len(), 5);

        // 15. updateOne targeting `_id` via `{$oid}` — the cell-edit path.
        let all = run_sql(&d, "SELECT * FROM players").await;
        let id_idx = all.columns.iter().position(|c| c.name == "_id").unwrap();
        let oid = all.rows[0][id_idx].as_str().unwrap().to_string();
        run_sql(
            &d,
            &format!(
                r#"db.players.updateOne({{"_id": {{"$oid": "{oid}"}}}}, {{"$set": {{"country": "ZZ"}}}})"#
            ),
        )
        .await;
        assert_eq!(
            run_sql(&d, "SELECT * FROM players WHERE country = 'ZZ'").await.rows.len(),
            1
        );

        // 16. createIndex / dropIndex.
        let r = run_sql(&d, r#"db.players.createIndex({"country": 1})"#).await;
        assert!(r.message.as_deref().unwrap_or("").contains("created index"));
        let r = run_sql(&d, r#"db.players.dropIndex("country_1")"#).await;
        assert!(r.message.as_deref().unwrap_or("").contains("dropped index"));

        // 17. explain returns a single query-plan row.
        let r = run_sql(&d, r#"db.players.find({"country": "US"}).explain()"#).await;
        assert_eq!(r.rows.len(), 1);
        assert!(r.columns.iter().any(|c| c.name == "queryPlan"));
    }
}
