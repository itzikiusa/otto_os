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
use mongodb::bson::{
    doc, spec::BinarySubtype, Binary as BsonBinary, Bson, DateTime as BsonDateTime, Decimal128,
    Document, Regex as BsonRegex, Timestamp as BsonTimestamp, Uuid as BsonUuid,
};
use mongodb::options::{
    ClientOptions, Compressor, Credential, ServerAddress, Socks5Proxy, Tls, TlsOptions,
};
use mongodb::{Client, Collection};
use otto_core::Result;
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;

use crate::driver::Driver;
use crate::drivers::{mongo_parse, mongo_sql};
use crate::export::{ExportCounts, ExportFormat, ExportSink};
use crate::tls::TlsFiles;
use crate::types::{
    self, compact_count, CancelToken, Capabilities, Column, CompletionContext, CompletionResponse,
    DbQueryPlan, Engine, IndexDef, NodeKind, NodePath, ObjectDetail, ObjectHit, ObjectSearchReq,
    ObjectSearchResult, QueryHandle, QueryRequest, QueryResult, QueryStats, ResolvedConfig,
    SchemaNode, TestResult,
};

/// How many documents to sample when inferring fields/types.
const SAMPLE_SIZE: i64 = 100;
/// Default row cap when a request doesn't set `max_rows`.
const DEFAULT_MAX_ROWS: usize = 50;
/// System databases sorted to the bottom of the tree.
const SYSTEM_DBS: &[&str] = &["admin", "local", "config"];
/// Largest magnitude a JSON number carries exactly through a JS `number`
/// (2^53). An `Int64` beyond it is emitted as the `{"$numberLong": "…"}`
/// sentinel so the digits survive the webview; below it a plain number is
/// both exact and far more readable.
const JSON_SAFE_INT: u64 = 1 << 53;

/// MongoDB driver. Caches one `mongodb::Client` per [`ResolvedConfig::cache_key`].
/// A `mongodb::Client` is internally connection-pooled and self-healing, and
/// cheap to clone (it's an `Arc` internally), so reusing it across calls avoids
/// re-establishing connections. `Mutex<HashMap>` is `Default`-constructible, so
/// `#[derive(Default)]` (used by the registry) still works.
#[derive(Default)]
pub struct MongoDriver {
    clients: Mutex<HashMap<String, Client>>,
    /// Per-connection completion cache: the collection list (cheap) plus each
    /// collection's sampled+indexed field paths (lazy, sampled only in context).
    completions: crate::complete::CompletionCache,
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
            // No session pinning across pooled ops — multi-document transactions
            // aren't wired, so we don't advertise them (was over-promised `true`).
            transactions: false,
            // `run_many` executes a `;`-separated script sequentially (already
            // supported — the flag now tells the truth).
            multi_statement: true,
            // Server-side cancel: every tracked find/aggregate/count is stamped
            // with `comment: "otto:<query_id>"`, and `cancel` resolves it through
            // `$currentOp` → `killOp` on a separate connection. A server that
            // denies `inprog`/`killop` degrades to a logged no-op (the client's
            // HTTP wait still ends), so advertising `true` never over-promises
            // more than "best effort" — the same contract as ClickHouse.
            cancel: true,
            // `.explain()` / the explain flag returns a query plan.
            explain: true,
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
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let client = self.connect(cfg).await?;
        let db_name = parent
            .get("db")
            .ok_or_else(|| types::invalid("expected a db:<name> node"))?;
        let db = client.database(db_name);

        match parent.get("coll") {
            // Collection node → sampled top-level fields (filtered by name when set).
            Some(coll_name) => {
                let coll: Collection<Document> = db.collection(coll_name);
                let size = adaptive_sample_size(&db, coll_name).await;
                let fields = sample_field_types(&coll, size).await?;
                let filter_lower = filter.map(|f| f.to_lowercase());
                Ok(fields
                    .into_iter()
                    .filter(|(name, _)| {
                        filter_lower
                            .as_deref()
                            .is_none_or(|f| name.to_lowercase().contains(f))
                    })
                    .map(|(name, ty)| {
                        let id = parent.child("field", &name).to_id();
                        SchemaNode::new(id, name, NodeKind::Field).with_detail(ty)
                    })
                    .collect())
            }
            // Database node → collections, filtered by name (case-insensitive) when set.
            None => {
                let mut names = db.list_collection_names().await.map_err(types::upstream)?;
                names.sort();
                let filter_lower = filter.map(|f| f.to_lowercase());
                let mut nodes = Vec::with_capacity(names.len());
                for name in names {
                    if let Some(f) = &filter_lower {
                        if !name.to_lowercase().contains(f.as_str()) {
                            continue;
                        }
                    }
                    let id = parent.child("coll", &name).to_id();
                    nodes.push(SchemaNode::new(id, name, NodeKind::Collection).expandable());
                }
                Ok(nodes)
            }
        }
    }

    async fn schema_children_with_counts(
        &self,
        cfg: &ResolvedConfig,
        parent: &NodePath,
        filter: Option<&str>,
        counts: bool,
    ) -> Result<Vec<SchemaNode>> {
        let nodes = self.schema_children(cfg, parent, filter).await?;
        // Only collection listings carry a count, and only when asked: this is
        // one `estimatedDocumentCount` round trip PER collection, which is why
        // it is a toggle and not the default.
        if !counts || parent.get("coll").is_some() {
            return Ok(nodes);
        }
        let Some(db_name) = parent.get("db") else {
            return Ok(nodes);
        };
        let client = self.connect(cfg).await?;
        let db = client.database(db_name);
        let mut out = Vec::with_capacity(nodes.len());
        for node in nodes {
            let coll: Collection<Document> = db.collection(&node.label);
            // Metadata estimate, never a full scan; a failure just omits it.
            match coll.estimated_document_count().await {
                Ok(n) => out.push(node.with_detail(compact_count(n as i64))),
                Err(_) => out.push(node),
            }
        }
        Ok(out)
    }

    async fn search_objects(
        &self,
        cfg: &ResolvedConfig,
        req: &ObjectSearchReq,
    ) -> Result<ObjectSearchResult> {
        if !req.wants("collection") {
            return Ok(ObjectSearchResult {
                supported: true,
                ..Default::default()
            });
        }
        let client = self.connect(cfg).await?;
        let limit = req.capped();
        // MongoDB has no cross-database catalog: an all-scope search is one
        // `listCollections` round trip per database. `scanned` reports the real
        // cost back to the caller so the UI can be honest about it.
        let dbs: Vec<String> = if req.all_schemas() {
            let mut names = client
                .list_database_names()
                .await
                .map_err(types::upstream)?;
            names.sort();
            names
        } else {
            req.schema.clone().into_iter().collect()
        };
        let needle = req.q.to_lowercase();
        let mut hits = Vec::new();
        let mut truncated = false;
        let mut scanned = 0usize;
        for db_name in dbs {
            if truncated {
                break;
            }
            scanned += 1;
            let names = match client.database(&db_name).list_collection_names().await {
                Ok(mut n) => {
                    n.sort();
                    n
                }
                // A database we cannot list (permissions) must not sink the
                // whole search — skip it and keep going.
                Err(_) => continue,
            };
            for name in names {
                if !name.to_lowercase().contains(&needle) {
                    continue;
                }
                if hits.len() == limit {
                    truncated = true;
                    break;
                }
                hits.push(ObjectHit {
                    schema: db_name.clone(),
                    name: name.clone(),
                    kind: NodeKind::Collection,
                    path: format!("db:{db_name}/coll:{name}"),
                });
            }
        }
        Ok(ObjectSearchResult {
            hits,
            truncated,
            scanned,
            supported: true,
        })
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
        // row_count is filled below from collStats (an exact, cheap count in
        // Mongo — unlike SQL's opt-in estimates).

        // Indexes: raw `listIndexes` documents so the full server definition
        // (partialFilterExpression, collation, expireAfterSeconds…) survives —
        // the typed IndexModel round-trip drops anything it doesn't model.
        if let Ok(reply) = db
            .run_command(doc! { "listIndexes": coll_name, "cursor": { "batchSize": 1000 } })
            .await
        {
            let batch = reply
                .get_document("cursor")
                .ok()
                .and_then(|c| c.get_array("firstBatch").ok())
                .cloned()
                .unwrap_or_default();
            let mut indexes = Vec::new();
            for item in &batch {
                let Some(spec) = item.as_document() else {
                    continue;
                };
                let keys = spec.get_document("key").cloned().unwrap_or_default();
                let name = spec
                    .get_str("name")
                    .map(str::to_string)
                    .unwrap_or_else(|_| join_index_keys(&keys));
                indexes.push(IndexDef {
                    name,
                    columns: keys.keys().map(|k| k.to_string()).collect(),
                    unique: spec.get_bool("unique").unwrap_or(false),
                    method: None,
                    definition: Some(bson_to_json(&Bson::Document(spec.clone()))),
                });
            }
            detail.indexes = indexes;
        }

        let mut extra = Map::new();

        // Collection stats FIRST: `avgObjSize` is what sizes the structure sample
        // below, so it has to be known before we sample.
        let mut avg_obj_size: Option<i64> = None;
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
            avg_obj_size = match stats.get("avgObjSize") {
                Some(Bson::Int64(n)) => Some(*n),
                Some(Bson::Int32(n)) => Some(*n as i64),
                Some(Bson::Double(n)) => Some(*n as i64),
                _ => None,
            };
            // Surface the exact collStats document count as the row count.
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

        // ONE byte-bounded sample serves all three things we used to fetch
        // separately (top-level types, nested paths, the sample document).
        let (paths, sample) = sample_structure(&coll, structure_sample_size(avg_obj_size))
            .await
            .unwrap_or_default();
        // Top-level fields are exactly the dot-free paths — no second pass needed.
        let field_types: Vec<(String, String)> = paths
            .iter()
            .filter(|(p, _)| !p.contains('.'))
            .cloned()
            .collect();
        extra.insert(
            "sampled_fields".into(),
            Value::Object(
                field_types
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect(),
            ),
        );
        // Nested dotted field PATHS (e.g. `players.playerId`,
        // `templatesAwardedCollections.succeededAwarded.templateId`) so the index
        // builder can index embedded fields, not just the top-level ones.
        if !paths.is_empty() {
            // Types alongside the paths: a collection has no `columns`, so the
            // structure tab builds its Fields table from these. A path is far more
            // useful as `lobbyMetaData.brand_id → int32` than as a bare string —
            // that's what tells you the field is worth an index.
            extra.insert(
                "sampled_path_types".into(),
                Value::Object(
                    paths
                        .iter()
                        .map(|(p, t)| (p.clone(), Value::String(t.clone())))
                        .collect(),
                ),
            );
            extra.insert(
                "sampled_paths".into(),
                Value::Array(paths.into_iter().map(|(p, _)| Value::String(p)).collect()),
            );
        }
        if let Some(s) = sample {
            extra.insert("sample".into(), bson_to_json(&Bson::Document(s)));
        }
        // Validator (best-effort) from listCollections options.
        if let Some(validator) = collection_validator(&db, coll_name).await {
            extra.insert("validator".into(), bson_to_json(&validator));
        }
        // (collStats was already fetched above — `avgObjSize` had to be known
        // before sampling — and filled `extra.stats` + `row_count` there.)
        detail.extra = Value::Object(extra);

        Ok(detail)
    }

    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        // Untracked run (widgets, export fallback, agents): same path with a
        // throwaway token — the MySQL/ClickHouse idiom.
        self.run_tracked(cfg, req, &CancelToken::new()).await
    }

    /// Tracked run: when the request carries a `query_id`, stamp every read
    /// (find/aggregate/count) with `comment: "otto:<query_id>"` and publish that
    /// tag through `token`, so a concurrent [`Driver::cancel`] can find the op in
    /// `$currentOp` and `killOp` it. Writes, index ops and `mongosh` scripts run
    /// untagged: the write option types carry no `comment`, a script is a
    /// separate process (its `timeout_ms` bound is client-side), and none of
    /// them is a long-running read the Stop button targets.
    async fn run_tracked(
        &self,
        cfg: &ResolvedConfig,
        req: &QueryRequest,
        token: &CancelToken,
    ) -> Result<QueryResult> {
        let tag = req
            .query_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(mongo_comment_tag);
        if let Some(t) = &tag {
            token.set(QueryHandle::MongoComment(t.clone()));
        }
        let comment = tag.as_deref();
        // A mongosh SCRIPT (variables, functions, control flow, getSiblingDB —
        // e.g. a seed/bootstrap file) is beyond the command parser: run it
        // through the real `mongosh` CLI instead of failing line one. The
        // service classified it as a WRITE before we got here (see
        // `types::looks_like_mongosh_script`), so guarded connections already
        // demanded their confirmation.
        if crate::types::looks_like_mongosh_script(&req.statement) {
            return self.run_script(cfg, req).await;
        }
        // A pasted script may hold several statements (e.g. a `deleteOne(...)`
        // followed by an `insertOne(...)`). Split on top-level `;` and run each
        // in order; a single statement takes the fast path unchanged.
        let statements = split_statements(&req.statement);
        if statements.len() <= 1 {
            let stmt = statements.into_iter().next().unwrap_or_default();
            let single = QueryRequest {
                statement: stmt,
                ..req.clone()
            };
            return self.run_one(cfg, &single, comment).await;
        }
        self.run_many(cfg, req, statements, comment).await
    }

    /// Server-side cancel for a tracked run: look the tagged op up in
    /// `$currentOp` (matched on `command.comment`, which a cursor's `getMore`s
    /// inherit) and `killOp` each opid, on a separate pooled connection. All
    /// best-effort: an op that already finished matches nothing (a successful
    /// no-op); a server that denies `inprog`/`killop` logs a warning and
    /// returns `Ok` so the client's Stop still resolves — only a genuine
    /// transport/server error propagates. `$currentOp` is first asked for
    /// `allUsers: true` (finds the op even when the pool authenticated it
    /// differently); when THAT is refused it is retried scoped to the current
    /// user, which needs no privilege and is what ran the query here.
    async fn cancel(&self, cfg: &ResolvedConfig, handle: &QueryHandle) -> Result<()> {
        let QueryHandle::MongoComment(tag) = handle else {
            return Ok(());
        };
        let client = self.connect(cfg).await?;
        let admin = client.database("admin");
        let ops = match current_ops(&admin, current_op_pipeline(tag)).await {
            Ok(docs) => docs,
            Err(e) if is_unauthorized(&e) => {
                match current_ops(&admin, current_op_pipeline_scoped(tag, false)).await {
                    Ok(docs) => docs,
                    Err(e) if is_unauthorized(&e) => {
                        tracing::warn!(tag, error = %e, "mongodb cancel: $currentOp denied — no-op");
                        return Ok(());
                    }
                    Err(e) => return Err(types::upstream(e)),
                }
            }
            Err(e) => return Err(types::upstream(e)),
        };
        for opid in opids_from_current_op(&ops) {
            match admin
                .run_command(doc! { "killOp": 1, "op": opid.clone() })
                .await
            {
                Ok(_) => tracing::debug!(tag, ?opid, "mongodb cancel: killOp issued"),
                Err(e) if is_unauthorized(&e) => {
                    tracing::warn!(tag, ?opid, error = %e, "mongodb cancel: killOp denied — no-op");
                    return Ok(());
                }
                Err(e) => return Err(types::upstream(e)),
            }
        }
        Ok(())
    }

    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        self.completion_impl(cfg, ctx).await
    }

    async fn invalidate_completion_cache(&self, cfg: &ResolvedConfig) {
        self.completions.invalidate(&cfg.cache_key());
    }

    /// Evict + shut down the cached `Client` for `cache_key` (connection close,
    /// or a config change superseded it). `Client::shutdown` closes the pool and
    /// stops the topology-monitor tasks; also drops the completion snapshot.
    async fn close(&self, cache_key: &str) {
        let client = self.clients.lock().await.remove(cache_key);
        if let Some(client) = client {
            client.shutdown().await;
        }
        self.completions.invalidate(cache_key);
    }

    /// Structured query plan via the server `explain` command (queryPlanner
    /// verbosity) for a find/aggregate. Never runs the query itself.
    async fn query_plan(
        &self,
        cfg: &ResolvedConfig,
        statement: &str,
        node: Option<&str>,
    ) -> Result<DbQueryPlan> {
        let translated = if mongo_sql::looks_like_sql(statement) {
            Some(mongo_sql::translate(statement)?)
        } else {
            None
        };
        let parsed = parse_command(translated.as_deref().unwrap_or(statement))?;
        if !matches!(parsed.op, MongoOp::Find | MongoOp::Aggregate) {
            return Err(types::invalid("query plan supports find / aggregate only"));
        }
        let db_name = resolve_db(cfg, node)?;
        let client = self.connect(cfg).await?;
        let db = client.database(&db_name);
        let raw = mongo_explain_value(&db, &parsed).await?;
        let root = crate::plan::from_mongo_queryplanner(&raw);
        Ok(DbQueryPlan {
            engine: "mongodb".into(),
            root,
            raw,
        })
    }

    /// Import parsed rows into a collection as batched `insertMany`. Values are
    /// mapped to BSON (numbers/bools/null preserved; the service coerces CSV
    /// string cells first). `batch_size` is clamped 1..=5000.
    async fn import_rows(
        &self,
        cfg: &ResolvedConfig,
        target: &str,
        columns: &[String],
        rows: &[Vec<Value>],
        batch_size: usize,
        node: Option<&str>,
    ) -> Result<(u64, u64)> {
        let db_name = resolve_db(cfg, node)?;
        let client = self.connect(cfg).await?;
        let coll: Collection<Document> = client.database(&db_name).collection(target);
        let batch = batch_size.clamp(1, 5000);
        let mut inserted = 0u64;
        let mut batches = 0u64;
        for chunk in rows.chunks(batch) {
            let docs: Vec<Document> = chunk.iter().map(|row| row_to_doc(columns, row)).collect();
            if docs.is_empty() {
                continue;
            }
            let res = coll.insert_many(&docs).await.map_err(types::upstream)?;
            inserted += res.inserted_ids.len() as u64;
            batches += 1;
        }
        Ok((inserted, batches))
    }

    /// Streaming export: iterate the `Cursor` (a `Stream`) document-by-document
    /// and write each straight to the file — the cursor is NEVER collected into a
    /// `Vec`, so daemon memory stays bounded for an arbitrarily large result.
    ///
    /// Only the row-returning ops (`find` / `aggregate`) are exportable. For the
    /// delimited formats (CSV/TSV) the column set is fixed from the FIRST document
    /// (Mongo is schemaless and we can't pre-scan the whole result without
    /// buffering it); later documents are projected onto those columns and any
    /// extra fields are dropped. JSON / NDJSON carry each document's full shape.
    async fn export_to_writer(
        &self,
        cfg: &ResolvedConfig,
        statement: &str,
        node: Option<&str>,
        format: ExportFormat,
        max_rows: Option<usize>,
        w: Box<dyn std::io::Write + Send>,
    ) -> Result<ExportCounts> {
        let parsed = parse_command(statement.trim())?;
        if !matches!(parsed.op, MongoOp::Find | MongoOp::Aggregate) {
            return Err(types::invalid("export supports find / aggregate only"));
        }
        let db_name = cfg
            .database
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                node.map(str::trim).filter(|s| !s.is_empty()).map(|n| {
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

        // Open the cursor (a Stream). We do NOT collect it.
        let mut cursor: mongodb::Cursor<Document> = match parsed.op {
            MongoOp::Find => {
                let mut action = coll.find(parsed.filter.unwrap_or_default());
                if let Some(p) = parsed.projection {
                    action = action.projection(p);
                }
                if let Some(s) = parsed.sort {
                    action = action.sort(s);
                }
                // Honour an explicit `.limit(n)`; otherwise unbounded (capped by
                // max_rows below, streaming).
                if let Some(l) = parsed.limit {
                    action = action.limit(l);
                }
                action.await.map_err(types::upstream)?
            }
            MongoOp::Aggregate => coll
                .aggregate(parsed.pipeline.unwrap_or_default())
                .await
                .map_err(types::upstream)?
                .with_type::<Document>(),
            _ => unreachable!("guarded above"),
        };

        let mut sink = ExportSink::new(w, format);

        let mut columns: Vec<String> = Vec::new();
        let mut header_written = false;
        let mut n: usize = 0;
        while let Some(next) = cursor.next().await {
            if let Some(cap) = max_rows {
                if n >= cap {
                    break;
                }
            }
            let doc = next.map_err(types::upstream)?;
            if !header_written {
                // Fix the column order from the first document (`_id` first).
                columns = first_doc_columns(&doc);
                let cols: Vec<Column> = columns.iter().cloned().map(Column::new).collect();
                sink.write_header(&cols)
                    .map_err(|e| otto_core::Error::Internal(format!("write export header: {e}")))?;
                header_written = true;
            }
            // Project onto the fixed columns for delimited formats; pass the full
            // document shape (keyed by the fixed columns, with extras dropped) for
            // JSON/NDJSON too so the row aligns with the header.
            let row: Vec<Value> = columns
                .iter()
                .map(|c| doc.get(c).map(bson_to_json).unwrap_or(Value::Null))
                .collect();
            sink.write_row(&row)
                .map_err(|e| otto_core::Error::Internal(format!("write export row: {e}")))?;
            n += 1;
        }
        if !header_written {
            sink.write_header(&[])
                .map_err(|e| otto_core::Error::Internal(format!("write export header: {e}")))?;
        }
        sink.finish()
            .map_err(|e| otto_core::Error::Internal(format!("finish export file: {e}")))
    }
}

/// Column order for the streaming Mongo export, taken from the first document:
/// `_id` first (when present), then the remaining keys in document order.
fn first_doc_columns(doc: &Document) -> Vec<String> {
    let mut cols: Vec<String> = Vec::with_capacity(doc.len());
    if doc.contains_key("_id") {
        cols.push("_id".to_string());
    }
    for key in doc.keys() {
        if key != "_id" {
            cols.push(key.clone());
        }
    }
    cols
}

// --- statement execution -----------------------------------------------------

impl MongoDriver {
    /// Run every statement from a multi-statement paste in order and fold the
    /// results into the shared batch shape: the **FIRST** statement's result is
    /// the top-level one, the rest go into `more_results` (each labelled with its
    /// statement preview). This aligns Mongo with the SQL drivers and the UI's
    /// result-set switcher — previously it returned the *last* result only, which
    /// hid all earlier results (intentional behavior change, noted in api.md). On
    /// the first failing statement execution stops with an `errored` entry and the
    /// completed results are returned (§2.2).
    async fn run_many(
        &self,
        cfg: &ResolvedConfig,
        req: &QueryRequest,
        statements: Vec<String>,
        comment: Option<&str>,
    ) -> Result<QueryResult> {
        let mut results: Vec<QueryResult> = Vec::with_capacity(statements.len());
        for stmt in statements {
            let preview = types::statement_preview(&stmt);
            let single = QueryRequest {
                statement: stmt,
                ..req.clone()
            };
            match self.run_one(cfg, &single, comment).await {
                Ok(mut r) => {
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

    /// Run one already-split statement: optional SQL→Mongo translation, command
    /// parse, then execute against the active database. `comment` is the cancel
    /// tag a tracked run stamps on its reads (`None` for untracked runs).
    async fn run_one(
        &self,
        cfg: &ResolvedConfig,
        req: &QueryRequest,
        comment: Option<&str>,
    ) -> Result<QueryResult> {
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
        // Per-statement timeout: MongoDB accepts `maxTimeMS` on cursors/commands.
        let max_time_ms = req
            .timeout_ms
            .filter(|&t| t > 0)
            .map(|t| i64::try_from(t).unwrap_or(i64::MAX));
        let started = Instant::now();

        // `.explain()` (or the request's explain flag) → return the query plan.
        if parsed.explain || req.explain {
            return explain_plan(&db, &parsed, started).await;
        }

        // The cancel tag rides on every read as the command `comment` (server
        // 4.4+; older servers ignore it), which is what `$currentOp` matches on.
        let comment_bson = comment.map(|c| Bson::String(c.to_string()));

        let mut result = match parsed.op {
            MongoOp::Count => {
                let filter = parsed.filter.unwrap_or_default();
                let mut count_opts = mongodb::options::CountOptions::default();
                if let Some(ms) = max_time_ms {
                    count_opts.max_time = Some(std::time::Duration::from_millis(ms as u64));
                }
                count_opts.comment = comment_bson;
                let n = coll
                    .count_documents(filter)
                    .with_options(count_opts)
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
                // Keyset pagination: an unconstrained find walking in `_id` order
                // pages with `{_id: {$gt: <last _id>}}` instead of `skip` (which
                // re-scans every skipped document). Eligibility is decided once
                // per page from the statement alone, and `{_id: 1}` is forced on
                // EVERY page of an eligible walk — page 1 included — so an
                // offset-paged "Prev" and a cursor-paged "Next" walk the same
                // order. Not eligible ⇒ the offset/`skip` path, cursor ignored.
                let keyset = keyset_filter(
                    parsed.filter.as_ref(),
                    parsed.sort.as_ref(),
                    parsed.limit.is_some(),
                    req.cursor.as_ref(),
                )?;
                let keyset_eligible = keyset.is_some();
                let cursor_applied = keyset_eligible && req.cursor.is_some();
                let (filter, sort) = match keyset {
                    Some((f, s)) => (f, Some(s)),
                    None => (parsed.filter.unwrap_or_default(), parsed.sort),
                };
                let mut action = coll.find(filter);
                if let Some(p) = parsed.projection {
                    action = action.projection(p);
                }
                if let Some(s) = sort {
                    action = action.sort(s);
                }
                let limit = parsed.limit.unwrap_or(max_rows as i64).min(max_rows as i64);
                action = action.limit(limit + 1);
                // Server-side pagination: the pager's `offset` maps to Mongo `skip`
                // (SQL engines map it to `OFFSET`). Applied only when there's no
                // explicit user `.limit(n)` — same rule as the SQL auto-limiter, so
                // the pager and the server never disagree — and not when the
                // keyset cursor already positioned the page.
                if let Some(off) = req.offset.filter(|&o| o > 0) {
                    if parsed.limit.is_none() && !cursor_applied {
                        action = action.skip(off);
                    }
                }
                if let Some(ms) = max_time_ms {
                    // maxTimeMS on the cursor tells the server to abort the query
                    // when the time budget is exceeded.
                    action = action.max_time(std::time::Duration::from_millis(ms as u64));
                }
                if let Some(c) = comment_bson {
                    action = action.comment(c);
                }
                let cursor = action.await.map_err(types::upstream)?;
                // Cap collection at the effective limit (not just max_rows) so an
                // explicit `.limit(n)` is honored; the extra fetched row flags truncation.
                let mut r = collect_docs(cursor, limit as usize, started).await?;
                // Flag the auto-limit ⇒ the UI shows its pager — but only when WE
                // capped it (no explicit user `.limit(n)`), mirroring the SQL path.
                r.auto_limited = parsed.limit.is_none().then_some(max_rows as u64);
                // Hand the client the keyset cursor for the next page: the last
                // `_id` of this one, only when more rows exist (the +1 probe was
                // fetched) — a full page walked to its end offers nothing.
                if keyset_eligible && r.truncated {
                    r.next_cursor = last_row_id(&r);
                }
                Ok(r)
            }
            MongoOp::Aggregate => {
                let pipeline = parsed.pipeline.unwrap_or_default();
                let mut agg_opts = mongodb::options::AggregateOptions::default();
                if let Some(ms) = max_time_ms {
                    agg_opts.max_time = Some(std::time::Duration::from_millis(ms as u64));
                }
                agg_opts.comment = comment_bson;
                let cursor = coll
                    .aggregate(pipeline)
                    .with_options(agg_opts)
                    .await
                    .map_err(types::upstream)?;
                collect_docs(cursor, max_rows, started).await
            }
            MongoOp::UpdateOne | MongoOp::UpdateMany => {
                let filter = parsed.filter.unwrap_or_default();
                let update = parsed
                    .update
                    .ok_or_else(|| types::invalid("update requires an update document"))?;
                let res = write_with_timeout(max_time_ms, async {
                    if matches!(parsed.op, MongoOp::UpdateOne) {
                        coll.update_one(filter, update).await
                    } else {
                        coll.update_many(filter, update).await
                    }
                })
                .await?;
                Ok(write_result(
                    res.modified_count,
                    format!(
                        "matched {}, modified {}",
                        res.matched_count, res.modified_count
                    ),
                    started,
                ))
            }
            MongoOp::ReplaceOne => {
                let filter = parsed.filter.unwrap_or_default();
                let replacement = parsed
                    .update
                    .ok_or_else(|| types::invalid("replaceOne requires a replacement document"))?;
                let res = write_with_timeout(max_time_ms, async {
                    coll.replace_one(filter, replacement).await
                })
                .await?;
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
                    write_with_timeout(max_time_ms, async {
                        coll.insert_one(docs.into_iter().next().unwrap()).await
                    })
                    .await?;
                    1
                } else {
                    write_with_timeout(max_time_ms, async { coll.insert_many(&docs).await })
                        .await?;
                    docs.len() as u64
                };
                Ok(write_result(n, format!("inserted {n}"), started))
            }
            MongoOp::DeleteOne | MongoOp::DeleteMany => {
                let filter = parsed.filter.unwrap_or_default();
                let res = write_with_timeout(max_time_ms, async {
                    if matches!(parsed.op, MongoOp::DeleteOne) {
                        coll.delete_one(filter).await
                    } else {
                        coll.delete_many(filter).await
                    }
                })
                .await?;
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
                write_with_timeout(max_time_ms, async {
                    db.run_command(doc! { "createIndexes": &parsed.collection, "indexes": [spec] })
                        .await
                })
                .await?;
                Ok(write_result(1, format!("created index {name}"), started))
            }
            MongoOp::DropIndex => {
                let name = parsed
                    .index_name
                    .ok_or_else(|| types::invalid("dropIndex requires an index name"))?;
                write_with_timeout(max_time_ms, async {
                    db.run_command(doc! { "dropIndexes": &parsed.collection, "index": &name })
                        .await
                })
                .await?;
                Ok(write_result(1, format!("dropped index {name}"), started))
            }
            MongoOp::GetIndexes => {
                // Raw `listIndexes` specs, one row per index — the same source the
                // structure tab reads, so `key`/`unique`/`partialFilterExpression`
                // survive verbatim instead of being flattened into a summary.
                let reply = db
                    .run_command(
                        doc! { "listIndexes": &parsed.collection, "cursor": { "batchSize": 1000 } },
                    )
                    .await
                    .map_err(types::upstream)?;
                let batch = reply
                    .get_document("cursor")
                    .ok()
                    .and_then(|c| c.get_array("firstBatch").ok())
                    .cloned()
                    .unwrap_or_default();
                let docs: Vec<Document> = batch
                    .iter()
                    .filter_map(|b| b.as_document().cloned())
                    .collect();
                Ok(docs_to_result(docs, false, started))
            }
        }?;

        // Show the user the Mongo command we ran on their behalf.
        if let Some(t) = translated {
            result.message = Some(format!("Translated from SQL → {t}"));
        }
        Ok(result)
    }

    async fn completion_impl(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        use crate::complete::mongo::{analyze, assemble, MongoExpect};

        let db_name = self.completion_db(cfg, ctx).await;

        // SQL-dialect completion. The runner accepts `SELECT … FROM <coll> WHERE …`
        // (translated to find/aggregate by `mongo_sql`); when the statement under
        // the cursor looks like SQL, complete it the SQL way — collections in a
        // table slot, the in-scope collection's fields (index-first) in a column
        // slot — so a SQL user gets the same smart completion as MySQL/ClickHouse.
        if mongo_sql::looks_like_sql(crate::complete::sql::current_statement(&ctx.prefix)) {
            return self.completion_sql(cfg, ctx, &db_name).await;
        }

        let mctx = analyze(&ctx.prefix);

        // Collection list (cheap, cached per (connection, db)).
        let collections = self.completion_collections(cfg, &db_name).await;

        // Field paths only when the cursor is at a query key — sample lazily and
        // cache per collection. The collection comes from the parsed `db.<coll>`
        // or, failing that, the tree node the user has selected.
        let fields = if matches!(mctx.expect, MongoExpect::Field { .. }) {
            let coll = mctx.collection.clone().or_else(|| {
                ctx.node
                    .as_deref()
                    .and_then(|n| NodePath::parse(n).get("coll").map(str::to_string))
            });
            match coll {
                Some(c) if !db_name.is_empty() => {
                    Some(self.completion_fields(cfg, &db_name, &c).await)
                }
                _ => None,
            }
        } else {
            None
        };

        let items = assemble(
            &mctx,
            &collections,
            fields.as_deref().map(|v| v.as_slice()),
            MONGO_OPERATORS,
            MONGO_METHODS,
        );
        Ok(CompletionResponse { items })
    }
}

impl MongoDriver {
    /// The database completions run against: the editor's active db, the
    /// connection default, or the selected tree node's db — and, when none of
    /// those is set, the first user (non-system) database on the server. Without
    /// this last fallback `db.` lists nothing until the user first picks a
    /// database (collections live under a db), which reads as "no completion".
    async fn completion_db(&self, cfg: &ResolvedConfig, ctx: &CompletionContext) -> String {
        if let Some(db) = ctx
            .database
            .clone()
            .or_else(|| cfg.database.clone())
            .or_else(|| {
                ctx.node
                    .as_deref()
                    .and_then(|n| NodePath::parse(n).get("db").map(str::to_string))
            })
            .filter(|s| !s.trim().is_empty())
        {
            return db;
        }
        self.completion_first_db(cfg).await.unwrap_or_default()
    }

    /// The first user (non-system) database, used to seed collection completion
    /// when the user hasn't selected a database yet. Best-effort: an unreachable
    /// server yields `None` (completion degrades to empty, never errors).
    async fn completion_first_db(&self, cfg: &ResolvedConfig) -> Option<String> {
        let client = self.connect(cfg).await.ok()?;
        let mut names = client.list_database_names().await.ok()?;
        // User databases first, system ones (admin/local/config) last — same order
        // as the schema tree, so the seed matches what the user would have picked.
        names.sort_by(|a, b| {
            let rank = |n: &str| SYSTEM_DBS.iter().position(|s| *s == n).map_or(0, |i| i + 1);
            rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
        });
        names
            .into_iter()
            .find(|n| !SYSTEM_DBS.contains(&n.as_str()))
    }

    /// SQL-dialect completion (`SELECT … FROM <coll> WHERE …`): delegate context
    /// analysis to the shared SQL analyzer, then assemble Mongo-flavored results
    /// — collections in a table slot, the in-scope collection's fields
    /// (index-first) in a column slot. Fields are sampled only when the cursor is
    /// at a column position (the common, cheap case).
    async fn completion_sql(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
        db_name: &str,
    ) -> Result<CompletionResponse> {
        use crate::complete::sql::{self, SqlExpect};

        let sctx = sql::analyze(&ctx.prefix, &ctx.suffix);
        let collections = self.completion_collections(cfg, db_name).await;

        let fields = if matches!(sctx.expect, SqlExpect::Column { .. }) {
            let node_coll = ctx
                .node
                .as_deref()
                .and_then(|n| NodePath::parse(n).get("coll").map(str::to_string));
            match resolve_sql_collection(&sctx, node_coll.as_deref()) {
                Some(c) if !db_name.is_empty() => {
                    Some(self.completion_fields(cfg, db_name, &c).await)
                }
                _ => None,
            }
        } else {
            None
        };

        let items = crate::complete::mongo::assemble_sql(
            &sctx,
            &collections,
            fields.as_deref().map(|v| v.as_slice()),
            MONGO_SQL_KEYWORDS,
            MONGO_SQL_FUNCTIONS,
        );
        Ok(CompletionResponse { items })
    }

    /// The (cached) list of collection names for a database, backing collection
    /// completion. Cached as a snapshot's `objects` until refresh; an unavailable
    /// connection yields an empty list (not cached).
    async fn completion_collections(&self, cfg: &ResolvedConfig, db: &str) -> Vec<String> {
        if db.is_empty() {
            return Vec::new();
        }
        let cache_key = cfg.cache_key();
        if let Some(s) = self.completions.get_snapshot(&cache_key, db) {
            return s.objects.iter().map(|o| o.name.clone()).collect();
        }
        let Ok(client) = self.connect(cfg).await else {
            return Vec::new();
        };
        let mongo_db = client.database(db);
        let Ok(mut names) = mongo_db.list_collection_names().await else {
            return Vec::new();
        };
        names.sort();
        let databases = client.list_database_names().await.unwrap_or_default();
        let objects = names
            .iter()
            .map(|n| crate::complete::ObjectSnap {
                name: n.clone(),
                kind: crate::complete::ObjKind::Collection,
                fields: Vec::new(),
                fields_ready: false,
            })
            .collect();
        self.completions.put_snapshot(
            &cache_key,
            db,
            crate::complete::SchemaSnapshot {
                databases,
                objects,
                ..Default::default()
            },
        );
        names
    }

    /// A collection's (cached) field paths for field completion — its index key
    /// paths (index-first, incl. embedded `x.a` and their parent `x`) followed by
    /// sampled nested field paths. Sampled only when the collection is in context.
    async fn completion_fields(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
        coll: &str,
    ) -> std::sync::Arc<Vec<crate::complete::FieldSnap>> {
        use crate::complete::{rank_strength, FieldSnap, Rank};

        let cache_key = cfg.cache_key();
        if let Some(f) = self.completions.get_fields(&cache_key, db, coll) {
            return f;
        }
        let Ok(client) = self.connect(cfg).await else {
            return std::sync::Arc::new(Vec::new());
        };
        let collection: Collection<Document> = client.database(db).collection(coll);

        // Insertion-ordered path list with the strongest rank seen for each path.
        let mut order: Vec<String> = Vec::new();
        let mut rank: std::collections::HashMap<String, Rank> = std::collections::HashMap::new();
        let mut add = |path: String, r: Rank| match rank.get_mut(&path) {
            Some(existing) => {
                if rank_strength(r) > rank_strength(*existing) {
                    *existing = r;
                }
            }
            None => {
                order.push(path.clone());
                rank.insert(path, r);
            }
        };

        // Indexes first: each key path (+ its dotted ancestors) ranked by index kind.
        if let Ok(mut cursor) = collection.list_indexes().await {
            while let Some(Ok(model)) = cursor.next().await {
                let name = model.options.as_ref().and_then(|o| o.name.clone());
                let unique = model
                    .options
                    .as_ref()
                    .and_then(|o| o.unique)
                    .unwrap_or(false);
                let is_id = name.as_deref() == Some("_id_");
                let r = if is_id {
                    Rank::Pk
                } else if unique {
                    Rank::Unique
                } else {
                    Rank::Index
                };
                for key in model.keys.keys() {
                    // Add every ancestor prefix (`addr` before `addr.city`) so the
                    // parent is offered first and refines after a `.`.
                    let parts: Vec<&str> = key.split('.').collect();
                    for i in 1..parts.len() {
                        add(parts[..i].join("."), Rank::Index);
                    }
                    add(key.to_string(), r);
                }
            }
        }

        // Then sampled field paths (nested, depth-bounded); plain unless indexed.
        let mut types: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let sample_size = adaptive_sample_size(&client.database(db), coll).await;
        if let Ok(sampled) = sample_field_paths(&collection, sample_size).await {
            for (path, ty) in sampled {
                types.entry(path.clone()).or_insert(ty);
                add(path, Rank::Plain);
            }
        }

        let fields: Vec<FieldSnap> = order
            .into_iter()
            .map(|path| {
                let r = rank.get(&path).copied().unwrap_or(Rank::Plain);
                let ty = types.get(&path).cloned();
                FieldSnap::new(path, ty, r)
            })
            .collect();

        self.completions.put_fields(&cache_key, db, coll, fields)
    }

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
    /// When tunnelled, every server connection is routed through the SSH SOCKS5
    /// proxy (see below).
    async fn client_options(&self, cfg: &ResolvedConfig) -> Result<ClientOptions> {
        let mut opts = match cfg.param_str("conn_string") {
            // A full connection string (e.g. `mongodb+srv://…` for Atlas) wins.
            // SRV discovery + replica-set topology happen inside the driver, so
            // host/port and TLS come from the URI — we only layer the SOCKS proxy
            // on below.
            Some(conn_string) => {
                let uri = match cfg.password.as_deref() {
                    Some(secret) => conn_string.replace("{secret}", secret),
                    None => conn_string,
                };
                ClientOptions::parse(&uri).await.map_err(types::upstream)?
            }
            None => self.host_port_options(cfg)?,
        };

        // Tunnelled (service::resolve set `__socks_port`): dial every server
        // through the SSH dynamic SOCKS5 proxy on 127.0.0.1. This is what makes a
        // `mongodb+srv` Atlas cluster reachable via a bastion — the driver
        // resolves the real shard hosts (public SRV) and connects to each through
        // the proxy, preserving the real SNI so Atlas's load balancer routes the
        // TLS handshake. It also fixes plain replica sets, whose SDAM would
        // otherwise dial member hostnames directly and bypass a local forward.
        if let Some(port) = cfg.params.get("__socks_port").and_then(Value::as_u64) {
            opts.socks5_proxy = Some(
                Socks5Proxy::builder()
                    .host("127.0.0.1")
                    .port(Some(port as u16))
                    .build(),
            );
        }

        // Negotiate WIRE COMPRESSION unless the URI already stated a preference.
        //
        // This is the difference between usable and unusable on a fat collection
        // reached through a bastion: such a link is bandwidth-bound (measured
        // ~68 KB/s steady-state on a real SSH SOCKS tunnel — TCP-over-TCP with a
        // high RTT), and document payloads of this shape (thousands of short,
        // repeated field names and ids) compress ~39x with zstd. A 3.7MB read of
        // 10 documents took 54s uncompressed; the same bytes compressed are ~95KB.
        //
        // Order is the client's PREFERENCE — the server picks the first it also
        // supports, and if it supports none the connection simply stays
        // uncompressed, so this can't break a server that lacks them. Small
        // messages are left alone by the driver, so the CPU cost is confined to
        // the payloads that actually benefit.
        if opts.compressors.is_none() {
            opts.compressors = Some(vec![
                Compressor::Zstd { level: None },
                Compressor::Snappy,
                Compressor::Zlib { level: None },
            ]);
        }

        Ok(opts)
    }

    /// Build `ClientOptions` from discrete host/port + credential + replica_set
    /// + TLS (the non-`conn_string` path).
    fn host_port_options(&self, cfg: &ResolvedConfig) -> Result<ClientOptions> {
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

    /// Execute a mongosh SCRIPT (real JavaScript — see
    /// `types::looks_like_mongosh_script`) through an actual
    /// `mongosh --quiet --file` run against the SAME resolved endpoint the
    /// native driver uses (tunnel, TLS, credentials). The explorer's parser
    /// covers single `db.coll.op(...)` statements; operational scripts need
    /// genuine shell semantics, and a faked JS subset would silently diverge
    /// from what the same file does under mongosh — so run the real thing.
    /// Output is the shell's stdout, one line per row.
    async fn run_script(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        /// A bootstrap script legitimately builds indexes; an interactive query
        /// console still needs SOME bound. Generous on purpose.
        const SCRIPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
        /// Stdout lines kept in the result grid before truncation.
        const SCRIPT_MAX_LINES: usize = 10_000;

        let started = Instant::now();
        let uri = mongosh_invocation(cfg, req.node.as_deref())?;
        let mut file = tempfile::Builder::new()
            .prefix("otto-mongosh-")
            .suffix(".js")
            .tempfile()
            .map_err(|e| types::invalid(format!("script temp file: {e}")))?;
        // SECURITY: the URI carries the DB credential, so it must NEVER appear
        // on mongosh's argv (visible to every local process via `ps` for the
        // script's lifetime). Instead the shell starts with `--nodb` and the
        // 0600 temp script itself connects via a `db = connect(<uri>)` prelude.
        std::io::Write::write_all(&mut file, mongosh_script_prelude(&uri).as_bytes())
            .map_err(|e| types::invalid(format!("script temp file: {e}")))?;
        std::io::Write::write_all(&mut file, req.statement.as_bytes())
            .map_err(|e| types::invalid(format!("script temp file: {e}")))?;

        let mut cmd = tokio::process::Command::new("mongosh");
        cmd.arg("--nodb")
            .arg("--quiet")
            .arg("--file")
            .arg(file.path())
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        let out = match tokio::time::timeout(SCRIPT_TIMEOUT, cmd.output()).await {
            Err(_) => {
                return Err(types::upstream(format!(
                    "mongosh script timed out after {} minutes",
                    SCRIPT_TIMEOUT.as_secs() / 60
                )))
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(types::invalid(
                    "this input is a mongosh SCRIPT (variables/functions/control flow), which \
                     Otto runs through the real `mongosh` CLI — but `mongosh` was not found on \
                     the daemon's PATH. Install it (`brew install mongosh`), or open a terminal \
                     session on this connection (Connections → this MongoDB), which runs \
                     mongosh directly.",
                ));
            }
            Ok(Err(e)) => return Err(types::upstream(format!("spawn mongosh: {e}"))),
            Ok(Ok(out)) => out,
        };
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if !out.status.success() {
            // The script's own prints ARE the diagnostic (e.g. a "[FAIL] idx…"
            // line before an assertion throws) — surface the tail of both
            // streams, not just stderr.
            let clip = |s: &str| -> String {
                let t = s.trim();
                let start = t
                    .char_indices()
                    .rev()
                    .nth(3999)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                t[start..].to_string()
            };
            let code = out
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "killed by signal".into());
            return Err(types::upstream(format!(
                "mongosh exited {code}\n{}\n{}",
                clip(&stderr),
                clip(&stdout)
            )));
        }
        let mut rows: Vec<Vec<Value>> = stdout
            .lines()
            .map(|l| vec![Value::String(l.to_string())])
            .collect();
        let truncated = rows.len() > SCRIPT_MAX_LINES;
        rows.truncate(SCRIPT_MAX_LINES);
        let row_count = rows.len();
        Ok(QueryResult {
            columns: vec![Column::new("output")],
            rows,
            message: Some("mongosh script finished".into()),
            stats: QueryStats {
                duration_ms: started.elapsed().as_millis() as u64,
                row_count,
                bytes_read: None,
            },
            truncated,
            ..QueryResult::empty()
        })
    }
}

/// Percent-encode a URI userinfo component (RFC 3986: unreserved bytes kept).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the connection URI a mongosh SCRIPT run connects with (mirroring
/// `client_options` — a full `conn_string` wins, else host/port + credential +
/// replica_set, with the SSH SOCKS5 tunnel layered on as the Node driver's
/// `proxyHost`/`proxyPort` URI options, which mongosh honors). TLS for the
/// host/port path rides as URI options (`tls`, `tlsCAFile`,
/// `tlsCertificateKeyFile`, `tlsAllowInvalidCertificates`) rather than argv
/// flags — the URI never touches argv (see `run_script`), so neither may the
/// TLS setup that must accompany it. Pure so tests can assert the shapes
/// without spawning a shell.
fn mongosh_invocation(cfg: &ResolvedConfig, node: Option<&str>) -> Result<String> {
    let mut uri = match cfg.param_str("conn_string") {
        Some(conn_string) => match cfg.password.as_deref() {
            Some(secret) => conn_string.replace("{secret}", secret),
            None => conn_string,
        },
        None => {
            let auth = match (cfg.user.as_deref(), cfg.password.as_deref()) {
                (Some(u), Some(p)) => format!("{}:{}@", pct(u), pct(p)),
                (Some(u), None) => format!("{}@", pct(u)),
                _ => String::new(),
            };
            // The selected database lands in the URI path so bare `db` refers
            // to it, exactly like the native run path; a script that targets
            // others calls db.getSiblingDB itself.
            let db = resolve_db(cfg, node).unwrap_or_default();
            let mut query: Vec<String> = Vec::new();
            if cfg.user.is_some() {
                query.push(format!(
                    "authSource={}",
                    cfg.param_str("auth_source")
                        .unwrap_or_else(|| "admin".into())
                ));
            }
            if let Some(rs) = cfg.param_str("replica_set") {
                query.push(format!("replicaSet={rs}"));
            }
            if cfg.tls.enabled() {
                let files = TlsFiles::materialize(&cfg.tls)?;
                query.push("tls=true".to_string());
                if !cfg.tls.verify {
                    query.push("tlsAllowInvalidCertificates=true".to_string());
                }
                if let Some(ca) = files.ca {
                    query.push(format!("tlsCAFile={}", pct(&ca.to_string_lossy())));
                }
                if let Some(pair) = files.client_pair.or(files.client_cert) {
                    query.push(format!(
                        "tlsCertificateKeyFile={}",
                        pct(&pair.to_string_lossy())
                    ));
                }
            }
            let query = if query.is_empty() {
                String::new()
            } else {
                format!("?{}", query.join("&"))
            };
            format!(
                "mongodb://{auth}{host}:{port}/{db}{query}",
                host = cfg.host,
                port = cfg.port
            )
        }
    };
    // Tunnelled (service::resolve set `__socks_port`): dial through the SSH
    // dynamic SOCKS5 proxy exactly like the native client does.
    if let Some(port) = cfg.params.get("__socks_port").and_then(Value::as_u64) {
        let sep = if uri.contains('?') { '&' } else { '?' };
        uri.push_str(&format!("{sep}proxyHost=127.0.0.1&proxyPort={port}"));
    }
    Ok(uri)
}

/// The one-line prelude prepended to a mongosh script so the 0600 temp FILE —
/// not argv — carries the credential-bearing URI: the shell starts `--nodb` and
/// the script itself connects. JSON-encoding the URI yields a valid JS string
/// literal (quotes/backslashes escaped).
fn mongosh_script_prelude(uri: &str) -> String {
    format!("db = connect({});\n", Value::String(uri.to_string()))
}

// --- command parsing --------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MongoOp {
    Find,
    Aggregate,
    Count,
    UpdateOne,
    UpdateMany,
    ReplaceOne,
    InsertOne,
    InsertMany,
    DeleteOne,
    DeleteMany,
    CreateIndex,
    DropIndex,
    /// `db.coll.getIndexes()` — READ-only; returns the raw `listIndexes` specs.
    GetIndexes,
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

/// Split a pasted script into individual statements on top-level `;`,
/// respecting string literals (`'`, `"`, `` ` ``), bracket depth, and `//` /
/// `/* */` comments. Each piece is cleaned of surrounding trivia; blank /
/// comment-only pieces are dropped. A paste with no top-level `;` yields one
/// statement (the whole input).
///
/// `pub(crate)` because the write-gate classifier (`types::mongo_is_write`)
/// must split with EXACTLY the rules the executor uses — a classifier that
/// judges the whole paste while the driver runs each piece is a guard bypass.
pub(crate) fn split_statements(input: &str) -> Vec<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut depth: i32 = 0;
    let mut in_str: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        match in_str {
            Some(q) => {
                if b == b'\\' {
                    i += 2; // skip escaped char (incl. an escaped quote)
                    continue;
                }
                if b == q {
                    in_str = None;
                }
                i += 1;
            }
            None => {
                if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
                    i += 2;
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    i += 2;
                    while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/'))
                    {
                        i += 1;
                    }
                    i += 2; // consume the closing */
                } else {
                    match b {
                        b'"' | b'\'' | b'`' => in_str = Some(b),
                        b'{' | b'[' | b'(' => depth += 1,
                        b'}' | b']' | b')' => depth -= 1,
                        b';' if depth <= 0 => {
                            if let Some(s) = clean_statement(&input[start..i]) {
                                out.push(s);
                            }
                            start = i + 1;
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
        }
    }
    if let Some(s) = clean_statement(&input[start..]) {
        out.push(s);
    }
    out
}

/// Trim a raw statement slice: strip leading whitespace + `//` / `/* */`
/// comments (so the shorthand parser starts at `db.`/`{`) and surrounding
/// whitespace. `None` when nothing meaningful remains.
fn clean_statement(slice: &str) -> Option<String> {
    let mut s = slice.trim();
    loop {
        if let Some(rest) = s.strip_prefix("//") {
            s = rest
                .split_once('\n')
                .map(|x| x.1)
                .unwrap_or("")
                .trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest
                .split_once("*/")
                .map(|x| x.1)
                .unwrap_or("")
                .trim_start();
        } else {
            break;
        }
    }
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
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

/// Classify ONE already-split Mongo statement as read-only by PARSING it with
/// the same parser the executor uses (`parse_command`) and anchoring the
/// decision to the resolved [`MongoOp`] — never a substring scan, which a
/// `.find(` inside a string literal can spoof. Parse failure ⇒ NOT read-only
/// (the conservative default: the guard refuses what it can't vet; an
/// unparseable statement errors at execution anyway).
pub(crate) fn statement_is_read_only(stmt: &str) -> bool {
    let Ok(parsed) = parse_command(stmt.trim()) else {
        return false;
    };
    match parsed.op {
        MongoOp::Find | MongoOp::Count | MongoOp::GetIndexes => true,
        // Aggregation reads — unless a top-level stage writes ($out/$merge must
        // be top-level pipeline stages, so checking stage keys is exact).
        MongoOp::Aggregate => !parsed
            .pipeline
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|stage| stage.keys().any(|k| k == "$out" || k == "$merge")),
        _ => false,
    }
}

fn parse_json_command(raw: &str) -> Result<ParsedCommand> {
    let value = mongo_parse::parse_value(raw)?;
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
        index_name: obj
            .get("index_name")
            .and_then(Value::as_str)
            .map(str::to_string),
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
            "replaceOne" => {
                // Whole-document replacement (the grid's JSON-view editor emits
                // this); the replacement rides the `update` slot like updateOne.
                cmd.op_kind = Some(MongoOp::ReplaceOne);
                let parts = split_top_level_args(&arg);
                let filter = parts
                    .first()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("replaceOne requires a filter"))?;
                let replacement = parts
                    .get(1)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| types::invalid("replaceOne requires a replacement document"))?;
                cmd.filter = Some(parse_doc_arg(filter)?);
                cmd.update = Some(parse_doc_arg(replacement)?);
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
            // Takes no arguments; anything passed is ignored, as in mongosh.
            "getIndexes" | "getIndices" => {
                cmd.op_kind = Some(MongoOp::GetIndexes);
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
    // A `\"` inside a string must not close it (nor may the escaped char be
    // read as a quote/bracket) — without this, `find({k:"a\"b)"})` miscounted
    // depth and spuriously rejected valid input.
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate() {
        match in_str {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == q {
                    in_str = None;
                }
            }
            None => match b {
                // Backtick included so a `query: \`…SQL…\`` template's embedded
                // parens/quotes don't corrupt the depth count.
                b'"' | b'\'' | b'`' => in_str = Some(b),
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
    // Honour `\"` inside strings — same rule as `extract_balanced`.
    let mut escaped = false;
    let mut start = 0usize;
    let bytes = arg.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match in_str {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == q {
                    in_str = None;
                }
            }
            None => match b {
                b'"' | b'\'' | b'`' => in_str = Some(b),
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
    let value = mongo_parse::parse_value(arg.trim())?;
    json_to_document(&value)
}

fn parse_pipeline_arg(arg: &str) -> Result<Vec<Document>> {
    let value = mongo_parse::parse_value(arg.trim())?;
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
        "replaceOne" => Ok(MongoOp::ReplaceOne),
        "insertOne" => Ok(MongoOp::InsertOne),
        "insertMany" => Ok(MongoOp::InsertMany),
        "deleteOne" => Ok(MongoOp::DeleteOne),
        "deleteMany" => Ok(MongoOp::DeleteMany),
        "createIndex" => Ok(MongoOp::CreateIndex),
        "dropIndex" => Ok(MongoOp::DropIndex),
        "getIndexes" | "getIndices" | "listIndexes" => Ok(MongoOp::GetIndexes),
        other => Err(types::invalid(format!("unsupported op '{other}'"))),
    }
}

// --- server-side cancel (comment tag → $currentOp → killOp) ------------------

/// The `comment` a tracked run stamps on its reads, keyed by the client's
/// `query_id`: `otto:<query_id>`. Namespaced so a `$currentOp` match can never
/// pick up a comment some other tool set to the same bare id.
fn mongo_comment_tag(query_id: &str) -> String {
    format!("otto:{query_id}")
}

/// `$currentOp` pipeline that resolves a comment tag to opids:
/// `[{$currentOp: {allUsers: true, localOps: true}}, {$match: {"command.comment":
/// tag}}, {$project: {opid: 1}}]`. `allUsers` needs the `inprog` privilege;
/// `localOps` reports the ops on the node we're connected to (a mongos's own
/// ops rather than the shards'), where `killOp` must also be issued. Runs on
/// the `admin` database.
fn current_op_pipeline(tag: &str) -> Vec<Document> {
    current_op_pipeline_scoped(tag, true)
}

/// [`current_op_pipeline`] with `allUsers` as given — `false` lists only the
/// current user's ops, which needs no privilege and still finds a run this
/// same client issued.
fn current_op_pipeline_scoped(tag: &str, all_users: bool) -> Vec<Document> {
    vec![
        doc! { "$currentOp": { "allUsers": all_users, "localOps": true } },
        doc! { "$match": { "command.comment": tag } },
        doc! { "$project": { "opid": 1 } },
    ]
}

/// The `opid` of every `$currentOp` row, kept as raw BSON: a mongod reports a
/// number, a mongos a `"shard:<n>"` string, and `killOp` wants it back verbatim.
fn opids_from_current_op(docs: &[Document]) -> Vec<Bson> {
    docs.iter().filter_map(|d| d.get("opid").cloned()).collect()
}

/// Run a `$currentOp` pipeline on `admin` and collect its rows.
async fn current_ops(
    admin: &mongodb::Database,
    pipeline: Vec<Document>,
) -> std::result::Result<Vec<Document>, mongodb::error::Error> {
    let mut cursor = admin.aggregate(pipeline).await?;
    let mut docs = Vec::new();
    while let Some(next) = cursor.next().await {
        docs.push(next?);
    }
    Ok(docs)
}

/// A privilege refusal (`Unauthorized` / `not authorized on admin to execute
/// command …`) — the one cancel failure that is a server policy rather than a
/// fault, so it degrades to a logged no-op instead of an error.
fn is_unauthorized(e: &impl std::fmt::Display) -> bool {
    let msg = e.to_string();
    msg.contains("Unauthorized") || msg.contains("not authorized")
}

// --- keyset pagination -------------------------------------------------------

/// Decide whether a `find` can page by keyset, and if so build the page's
/// `(filter, sort)`. Eligible ⇔ no explicit `.limit(n)` (the auto-limiter's own
/// rule — an explicit limit is never paged), the sort is absent or exactly
/// `{_id: 1}`, and the filter carries no top-level `_id` (a user already pinning
/// `_id` gets the natural order they asked for). When eligible the sort is
/// `{_id: 1}` ALWAYS — page 1 included — so every page of the walk shares one
/// order; with a `cursor` (the previous page's last `_id`, Extended JSON) the
/// filter becomes `{$and: [<filter or {}>, {_id: {$gt: cursor}}]}` — `$and`
/// rather than a merged key, so a filter using `$or`/`$and` at the top stays
/// intact. `Ok(None)` ⇒ not eligible, use offset/`skip`. An undecodable cursor
/// is the caller's error (a 400), never a silent fall-back to `skip`.
fn keyset_filter(
    filter: Option<&Document>,
    sort: Option<&Document>,
    explicit_limit: bool,
    cursor: Option<&Value>,
) -> Result<Option<(Document, Document)>> {
    if explicit_limit || !sort.is_none_or(sort_is_id_asc) {
        return Ok(None);
    }
    if filter.is_some_and(|f| f.contains_key("_id")) {
        return Ok(None);
    }
    let base = filter.cloned().unwrap_or_default();
    let filter = match cursor {
        Some(c) => {
            let last = json_to_bson(c)
                .map_err(|e| types::invalid(format!("invalid keyset cursor: {e}")))?;
            doc! { "$and": [base, { "_id": { "$gt": last } }] }
        }
        None => base,
    };
    Ok(Some((filter, doc! { "_id": 1 })))
}

/// `{_id: 1}` exactly — one key, ascending — with the direction compared
/// numerically (the parser yields `Int64(1)`, a pasted EJSON sort may say
/// `Int32`/`Double`).
fn sort_is_id_asc(sort: &Document) -> bool {
    sort.len() == 1
        && sort.get("_id").is_some_and(|dir| match dir {
            Bson::Int32(n) => *n == 1,
            Bson::Int64(n) => *n == 1,
            Bson::Double(f) => *f == 1.0,
            _ => false,
        })
}

/// The `_id` cell of the LAST row (already the typed Extended-JSON projection
/// the wire carries, so it round-trips through [`json_to_bson`] unchanged) —
/// the keyset cursor for the next page. `None` when the result has no `_id`
/// column (a projection dropped it) or no rows, in which case the client keeps
/// paging by offset.
fn last_row_id(r: &QueryResult) -> Option<Value> {
    let idx = r.columns.iter().position(|c| c.name == "_id")?;
    r.rows.last()?.get(idx).filter(|v| !v.is_null()).cloned()
}

// --- bson / json helpers ----------------------------------------------------

/// Convert a JSON value into a BSON `Document` (the value must be an object).
fn json_to_document(v: &Value) -> Result<Document> {
    match json_to_bson(v)? {
        Bson::Document(d) => Ok(d),
        _ => Err(types::invalid("expected a JSON object")),
    }
}

/// JSON → BSON, decoding the MongoDB Extended JSON sentinels that
/// [`mongo_parse`] emits for mongosh constructors (`{"$oid": …}`, `{"$date":
/// …}`, `{"$numberLong": …}`, …) into their real BSON types.
fn json_to_bson(v: &Value) -> Result<Bson> {
    match v {
        Value::Object(map) => {
            if map.len() == 1 {
                if let Some(decoded) = decode_ejson(map) {
                    return decoded;
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

/// Decode a single-key MongoDB Extended JSON sentinel into its BSON type.
/// Returns `None` when the key isn't a recognized sentinel, so the object is
/// treated as a normal document — that's also why update operators like
/// `{"$set": …}` pass through untouched (`$set` isn't a sentinel).
fn decode_ejson(map: &Map<String, Value>) -> Option<Result<Bson>> {
    let (key, val) = map.iter().next()?;
    let decoded = match key.as_str() {
        "$oid" => ejson_str(val, "$oid").and_then(|s| {
            mongodb::bson::oid::ObjectId::parse_str(&s)
                .map(Bson::ObjectId)
                .map_err(|e| types::invalid(format!("invalid $oid: {e}")))
        }),
        "$date" => decode_date(val),
        "$numberLong" => ejson_i64(val, "$numberLong").map(Bson::Int64),
        "$numberInt" => ejson_i64(val, "$numberInt").and_then(|n| {
            i32::try_from(n)
                .map(Bson::Int32)
                .map_err(|_| types::invalid("$numberInt out of range"))
        }),
        "$numberDecimal" => ejson_str(val, "$numberDecimal").and_then(|s| {
            s.parse::<Decimal128>()
                .map(Bson::Decimal128)
                .map_err(|e| types::invalid(format!("invalid $numberDecimal: {e:?}")))
        }),
        "$uuid" => ejson_str(val, "$uuid").and_then(|s| {
            BsonUuid::parse_str(&s)
                .map(Bson::from)
                .map_err(|e| types::invalid(format!("invalid $uuid: {e}")))
        }),
        "$regularExpression" => decode_regex(val),
        "$binary" => decode_binary(val),
        "$timestamp" => decode_timestamp(val),
        _ => return None,
    };
    Some(decoded)
}

/// `{"$binary": {"base64": "…", "subType": "<2 hex>"}}` (canonical Extended
/// JSON) → `Bson::Binary`; the subtype defaults to generic (`00`) when absent.
fn decode_binary(v: &Value) -> Result<Bson> {
    let obj = v
        .as_object()
        .ok_or_else(|| types::invalid("$binary expects an object"))?;
    let b64 = obj
        .get("base64")
        .and_then(Value::as_str)
        .ok_or_else(|| types::invalid("$binary expects a base64 string"))?;
    let subtype = match obj.get("subType").and_then(Value::as_str) {
        None => BinarySubtype::Generic,
        Some(hex) => u8::from_str_radix(hex, 16)
            .map(BinarySubtype::from)
            .map_err(|_| types::invalid(format!("invalid $binary subType '{hex}'")))?,
    };
    BsonBinary::from_base64(b64, subtype)
        .map(Bson::Binary)
        .map_err(|e| types::invalid(format!("invalid $binary: {e}")))
}

/// `{"$timestamp": {"t": <u32>, "i": <u32>}}` → `Bson::Timestamp`.
fn decode_timestamp(v: &Value) -> Result<Bson> {
    let obj = v
        .as_object()
        .ok_or_else(|| types::invalid("$timestamp expects an object"))?;
    let field = |k: &str| -> Result<u32> {
        obj.get(k)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| types::invalid(format!("$timestamp.{k} must be a u32")))
    };
    Ok(Bson::Timestamp(BsonTimestamp {
        time: field("t")?,
        increment: field("i")?,
    }))
}

fn ejson_str(v: &Value, what: &str) -> Result<String> {
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| types::invalid(format!("{what} expects a string")))
}

fn ejson_i64(v: &Value, what: &str) -> Result<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| types::invalid(format!("{what} is not an integer"))),
        Value::String(s) => s
            .parse::<i64>()
            .map_err(|_| types::invalid(format!("{what} is not an integer"))),
        _ => Err(types::invalid(format!(
            "{what} expects a number or numeric string"
        ))),
    }
}

/// `$date` accepts an RFC-3339 string, a date-only `YYYY-MM-DD`, epoch millis as
/// a number, or the canonical `{"$numberLong": "<ms>"}` wrapper.
fn decode_date(v: &Value) -> Result<Bson> {
    match v {
        Value::String(s) => {
            if let Ok(dt) = BsonDateTime::parse_rfc3339_str(s) {
                return Ok(Bson::DateTime(dt));
            }
            // Tolerate a date-only `YYYY-MM-DD` by assuming midnight UTC.
            if let Ok(dt) = BsonDateTime::parse_rfc3339_str(format!("{s}T00:00:00Z")) {
                return Ok(Bson::DateTime(dt));
            }
            Err(types::invalid(format!("invalid $date string '{s}'")))
        }
        Value::Number(n) => {
            let ms = n
                .as_i64()
                .ok_or_else(|| types::invalid("$date millis must be an integer"))?;
            Ok(Bson::DateTime(BsonDateTime::from_millis(ms)))
        }
        Value::Object(inner) => {
            let ms = ejson_i64(inner.get("$numberLong").unwrap_or(&Value::Null), "$date")?;
            Ok(Bson::DateTime(BsonDateTime::from_millis(ms)))
        }
        _ => Err(types::invalid("invalid $date")),
    }
}

fn decode_regex(v: &Value) -> Result<Bson> {
    let obj = v
        .as_object()
        .ok_or_else(|| types::invalid("$regularExpression expects an object"))?;
    let pattern = obj
        .get("pattern")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let options = obj
        .get("options")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(Bson::RegularExpression(BsonRegex { pattern, options }))
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
        Bson::DateTime(dt) => Value::String(
            dt.try_to_rfc3339_string()
                .unwrap_or_else(|_| dt.to_string()),
        ),
        Bson::Timestamp(ts) => json!({ "t": ts.time, "i": ts.increment }),
        Bson::Decimal128(d) => Value::String(d.to_string()),
        Bson::Symbol(s) => Value::String(s.clone()),
        Bson::RegularExpression(re) => Value::String(format!("/{}/{}", re.pattern, re.options)),
        Bson::JavaScriptCode(code) => Value::String(code.clone()),
        Bson::JavaScriptCodeWithScope(c) => Value::String(c.code.clone()),
        Bson::Binary(bin) => Value::String(binary_to_string(bin)),
        Bson::MaxKey => Value::String("$maxKey".into()),
        Bson::MinKey => Value::String("$minKey".into()),
        Bson::DbPointer(_) => Value::String("$dbPointer".into()),
    }
}

/// Like [`bson_to_json`] but PRESERVES the type of values whose plain JSON
/// projection is lossy or indistinguishable — ObjectId, DateTime, Decimal128,
/// a Long beyond 2^53, Binary (UUID or raw), Timestamp — by emitting their
/// MongoDB Extended JSON sentinel (`{"$oid": …}`, `{"$date": …}`,
/// `{"$numberDecimal": …}`, `{"$numberLong": "…"}`, `{"$uuid": …}`, `{"$binary":
/// {base64, subType}}`, `{"$timestamp": {t, i}}`). The query-RESULTS path uses
/// this so the UI can render `ObjectId("…")` / `ISODate("…")` (a real type hint
/// the user can act on) and a cell "query by value" round-trips correctly — the
/// runner's own parser (`mongo_parse` / `decode_ejson`) decodes every one of
/// these sentinels back to its BSON type. An `Int64` within ±2^53 stays a plain
/// number: it is exact in a JS `number` and far more readable (on the way back
/// in a plain integer parses as `Int64` anyway). Recurses through
/// documents/arrays so nested values are typed too.
fn bson_to_json_typed(b: &Bson) -> Value {
    match b {
        Bson::ObjectId(oid) => json!({ "$oid": oid.to_hex() }),
        Bson::DateTime(dt) => {
            json!({ "$date": dt.try_to_rfc3339_string().unwrap_or_else(|_| dt.to_string()) })
        }
        Bson::Decimal128(d) => json!({ "$numberDecimal": d.to_string() }),
        Bson::Int64(n) if n.unsigned_abs() > JSON_SAFE_INT => {
            json!({ "$numberLong": n.to_string() })
        }
        Bson::Binary(bin) => binary_to_json_typed(bin),
        Bson::Timestamp(ts) => json!({ "$timestamp": { "t": ts.time, "i": ts.increment } }),
        Bson::Array(arr) => Value::Array(arr.iter().map(bson_to_json_typed).collect()),
        Bson::Document(doc) => Value::Object(
            doc.iter()
                .map(|(k, v)| (k.clone(), bson_to_json_typed(v)))
                .collect(),
        ),
        // Everything else already has an unambiguous JSON projection.
        _ => bson_to_json(b),
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Typed projection of a BSON binary: UUID subtypes (4 = standard, 3 = legacy)
/// as `{"$uuid": "<hyphenated>"}` — the id the user can read and query — and
/// every other subtype as the canonical `{"$binary": {"base64": "…", "subType":
/// "<2 hex, lowercase>"}}`, which [`decode_binary`] takes back verbatim.
fn binary_to_json_typed(bin: &BsonBinary) -> Value {
    if matches!(bin.subtype, BinarySubtype::Uuid | BinarySubtype::UuidOld) {
        if let Ok(bytes) = <[u8; 16]>::try_from(bin.bytes.as_slice()) {
            return json!({ "$uuid": BsonUuid::from_bytes(bytes).to_string() });
        }
    }
    json!({
        "$binary": {
            "base64": base64_encode(&bin.bytes),
            "subType": format!("{:02x}", u8::from(bin.subtype)),
        }
    })
}

/// Render a BSON binary value: UUID subtypes (4 = standard, 3 = legacy) show as
/// the hyphenated UUID the way Compass/DataGrip do — base64 gibberish for what
/// is semantically an id is useless to the user. Everything else stays base64.
fn binary_to_string(bin: &mongodb::bson::Binary) -> String {
    use mongodb::bson::spec::BinarySubtype;
    if matches!(bin.subtype, BinarySubtype::Uuid | BinarySubtype::UuidOld) {
        if let Ok(bytes) = <[u8; 16]>::try_from(bin.bytes.as_slice()) {
            return BsonUuid::from_bytes(bytes).to_string();
        }
    }
    base64_encode(&bin.bytes)
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
    keys.keys()
        .map(|k| k.to_string())
        .collect::<Vec<_>>()
        .join("_")
}

// --- sampling & result shaping ----------------------------------------------

/// Byte budget for ONE structure-inference sample.
///
/// `$sample` is document-COUNT based, which is the wrong unit here: a flat size
/// of 100 is nothing on 1KB documents and ~37MB on the 370KB documents of a
/// `lobby_format_history`-shaped collection. The structure tab used to pay that
/// TWICE (top-level types + nested paths) plus a whole `first_document` — ~74MB
/// over the wire to produce a field list, which took 136s through a bastion.
/// Bound the BYTES and let the document size decide the count.
const STRUCTURE_SAMPLE_BYTES: i64 = 2 * 1024 * 1024;
/// Never sample fewer than this — a couple of documents still has to be able to
/// show a heterogeneous collection's shape.
const STRUCTURE_SAMPLE_MIN: i64 = 5;

/// How many documents to sample given the collection's average object size
/// (`collStats.avgObjSize`). Unknown size ⇒ the old flat [`SAMPLE_SIZE`].
fn structure_sample_size(avg_obj_size: Option<i64>) -> i64 {
    match avg_obj_size.filter(|n| *n > 0) {
        Some(avg) => (STRUCTURE_SAMPLE_BYTES / avg).clamp(STRUCTURE_SAMPLE_MIN, SAMPLE_SIZE),
        None => SAMPLE_SIZE,
    }
}

/// Ask the server for `avgObjSize` so a standalone caller can size its sample the
/// same way [`Driver::object_detail`] does. Cheap (one `collStats`); falling back
/// to the flat size on error only costs us the old behaviour.
async fn adaptive_sample_size(db: &mongodb::Database, coll_name: &str) -> i64 {
    let avg = db
        .run_command(doc! { "collStats": coll_name, "scale": 1 })
        .await
        .ok()
        .and_then(|stats| match stats.get("avgObjSize") {
            Some(Bson::Int64(n)) => Some(*n),
            Some(Bson::Int32(n)) => Some(*n as i64),
            Some(Bson::Double(n)) => Some(*n as i64),
            _ => None,
        });
    structure_sample_size(avg)
}

/// Sample up to `sample_size` docs and infer a type per top-level key. The
/// first observed type wins; `_id` is always reported first.
async fn sample_field_types(
    coll: &Collection<Document>,
    sample_size: i64,
) -> Result<Vec<(String, String)>> {
    let pipeline = vec![doc! { "$sample": { "size": sample_size } }];
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

/// ONE sample pass yielding everything the structure tab needs: the dotted field
/// paths (top-level ones are simply the dot-free entries) and a representative
/// document. Replaces the old three separate fetches of the same collection.
async fn sample_structure(
    coll: &Collection<Document>,
    sample_size: i64,
) -> Result<(Vec<(String, String)>, Option<Document>)> {
    let pipeline = vec![doc! { "$sample": { "size": sample_size } }];
    let mut cursor = coll.aggregate(pipeline).await.map_err(types::upstream)?;
    let mut order: Vec<String> = Vec::new();
    let mut types: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut first: Option<Document> = None;
    while let Some(next) = cursor.next().await {
        let doc = match next {
            Ok(d) => d,
            Err(_) => continue,
        };
        walk_field_paths(&doc, "", 0, &mut order, &mut types);
        if first.is_none() {
            first = Some(doc);
        }
        if order.len() >= MAX_FIELD_PATHS {
            break;
        }
    }
    order.sort_by_key(|k| if k == "_id" { 0 } else { 1 });
    let paths = order
        .into_iter()
        .map(|k| {
            let ty = types.get(&k).cloned().unwrap_or_default();
            (k, ty)
        })
        .collect();
    Ok((paths, first))
}

/// Max nesting depth for sampled embedded field paths (`a.b.c` is depth 3).
const MAX_FIELD_DEPTH: usize = 3;
/// Cap on total sampled paths so a wide/deep collection can't bloat completion.
const MAX_FIELD_PATHS: usize = 400;

/// Sample docs and infer dotted field PATHS (incl. embedded `addr.city` and the
/// first element of document-arrays), depth- and count-bounded. Backs Mongo
/// field completion's "indexes first, then sampled fields" with embedded support.
async fn sample_field_paths(
    coll: &Collection<Document>,
    sample_size: i64,
) -> Result<Vec<(String, String)>> {
    let pipeline = vec![doc! { "$sample": { "size": sample_size } }];
    let mut cursor = coll.aggregate(pipeline).await.map_err(types::upstream)?;
    let mut order: Vec<String> = Vec::new();
    let mut types: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    while let Some(next) = cursor.next().await {
        let doc = match next {
            Ok(d) => d,
            Err(_) => continue,
        };
        walk_field_paths(&doc, "", 0, &mut order, &mut types);
        if order.len() >= MAX_FIELD_PATHS {
            break;
        }
    }
    order.sort_by_key(|k| if k == "_id" { 0 } else { 1 });
    Ok(order
        .into_iter()
        .map(|k| {
            let ty = types.get(&k).cloned().unwrap_or_default();
            (k, ty)
        })
        .collect())
}

/// Recursively record `prefix.key` paths and their BSON type names.
fn walk_field_paths(
    doc: &Document,
    prefix: &str,
    depth: usize,
    order: &mut Vec<String>,
    types: &mut std::collections::HashMap<String, String>,
) {
    for (key, value) in doc.iter() {
        if order.len() >= MAX_FIELD_PATHS {
            return;
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if !types.contains_key(&path) {
            order.push(path.clone());
            types.insert(path.clone(), bson_type_name(value).to_string());
        }
        if depth + 1 < MAX_FIELD_DEPTH {
            match value {
                Bson::Document(sub) => walk_field_paths(sub, &path, depth + 1, order, types),
                Bson::Array(arr) => {
                    if let Some(Bson::Document(sub)) = arr.first() {
                        walk_field_paths(sub, &path, depth + 1, order, types);
                    }
                }
                _ => {}
            }
        }
    }
}

// NOTE: the former `first_document` (a separate `find().limit(1)` purely to
// populate `extra.sample`) is gone — `sample_structure` keeps the first document
// of the sample it already pulled, so the structure tab makes one fewer
// round trip and, on a fat collection, transfers one fewer ~370KB document.

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
    let plan = mongo_explain_value(db, parsed).await?;
    let mut result = QueryResult::empty();
    result.columns = vec![Column::typed("queryPlan", "json")];
    result.rows = vec![vec![plan]];
    result.stats = QueryStats {
        duration_ms: started.elapsed().as_millis() as u64,
        row_count: 1,
        bytes_read: None,
    };
    result.message = Some("Query plan (explain · queryPlanner)".into());
    Ok(result)
}

/// The `explain` inner command doc for a find/aggregate.
fn explain_inner(parsed: &Parsed) -> Result<Document> {
    match parsed.op {
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
            Ok(d)
        }
        MongoOp::Aggregate => {
            let stages: Vec<Bson> = parsed
                .pipeline
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(Bson::Document)
                .collect();
            Ok(
                doc! { "aggregate": &parsed.collection, "pipeline": Bson::Array(stages), "cursor": {} },
            )
        }
        _ => Err(types::invalid("explain supports find and aggregate")),
    }
}

/// Run the server `explain` command (queryPlanner verbosity) and return the plan
/// document as JSON — shared by the interactive explain and the query-plan endpoint.
async fn mongo_explain_value(db: &mongodb::Database, parsed: &Parsed) -> Result<Value> {
    let inner = explain_inner(parsed)?;
    let plan = db
        .run_command(doc! { "explain": inner, "verbosity": "queryPlanner" })
        .await
        .map_err(types::upstream)?;
    Ok(bson_to_json(&Bson::Document(plan)))
}

/// The database a Mongo op runs against: the connection's configured database,
/// else the active-db `node` (a plain name or a `db:<name>` path).
fn resolve_db(cfg: &ResolvedConfig, node: Option<&str>) -> Result<String> {
    cfg.database
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            node.map(str::trim).filter(|s| !s.is_empty()).map(|n| {
                NodePath::parse(n)
                    .get("db")
                    .map(str::to_string)
                    .unwrap_or_else(|| n.to_string())
            })
        })
        .ok_or_else(|| types::invalid("no database selected for this connection"))
}

/// Build a BSON document from a parsed import row: each column → its coerced
/// value mapped to BSON (numbers/bools/null preserved). A missing/short cell
/// becomes null.
fn row_to_doc(columns: &[String], row: &[Value]) -> Document {
    let mut doc = Document::new();
    for (i, col) in columns.iter().enumerate() {
        let v = row.get(i).cloned().unwrap_or(Value::Null);
        let bson = json_to_bson(&v).unwrap_or(Bson::Null);
        doc.insert(col.clone(), bson);
    }
    doc
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

/// Client-side wall clock for Mongo WRITE ops (`QueryRequest::timeout_ms`): the
/// driver's update/insert/delete/index option types expose no `maxTimeMS`, so
/// the bound is enforced on the request future — a stuck `updateMany` no longer
/// runs forever with no cancel path. `None` runs unbounded, as before.
async fn write_with_timeout<T, F>(max_time_ms: Option<i64>, fut: F) -> Result<T>
where
    F: std::future::Future<Output = std::result::Result<T, mongodb::error::Error>>,
{
    match max_time_ms {
        Some(ms) => tokio::time::timeout(std::time::Duration::from_millis(ms as u64), fut)
            .await
            .map_err(|_| {
                types::upstream(format!(
                    "mongodb: write timed out after {ms}ms (client-side — the server may \
                     still complete it)"
                ))
            })?
            .map_err(types::upstream),
        None => fut.await.map_err(types::upstream),
    }
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
    Ok(docs_to_result(docs, truncated, started))
}

/// Shape already-materialized documents into a [`QueryResult`]. Split out of
/// [`collect_docs`] so command replies that arrive as a `firstBatch` array
/// (`listIndexes`) get the identical column-union treatment as a cursor.
fn docs_to_result(docs: Vec<Document>, truncated: bool, started: Instant) -> QueryResult {
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
                .map(|col| {
                    // Cap oversized cells like every other engine — a single
                    // huge embedded document/string must not freeze the grid.
                    doc.get(col)
                        .map(|b| types::cap_cell(bson_to_json_typed(b)))
                        .unwrap_or(Value::Null)
                })
                .collect()
        })
        .collect();

    let row_count = rows.len();
    QueryResult {
        columns: columns.into_iter().map(Column::new).collect(),
        rows,
        stats: QueryStats {
            duration_ms: started.elapsed().as_millis() as u64,
            row_count,
            bytes_read: None,
        },
        truncated,
        ..QueryResult::empty()
    }
}

// --- aggregation operator/stage catalog for completion ----------------------

/// Resolve which collection a SQL column slot refers to, for field completion.
/// Resolution order, mirroring how `mongo_sql` resolves field paths:
///   1. an explicit `qualifier.` that names a FROM table or alias → that collection;
///   2. otherwise the base (first) FROM collection — this also handles an embedded
///      dotted path like `WHERE address.city`, where `address` is NOT a table but
///      a field path on the base collection (the field list already carries the
///      dotted paths, which the editor filters against the typed prefix);
///   3. failing any FROM table, the collection selected in the schema tree.
fn resolve_sql_collection(
    sctx: &crate::complete::sql::SqlCtx,
    node_coll: Option<&str>,
) -> Option<String> {
    use crate::complete::sql::SqlExpect;
    if let SqlExpect::Column { qualifier: Some(q) } = &sctx.expect {
        if let Some(t) = sctx.tables.iter().find(|t| {
            t.alias
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(q))
                || t.name.eq_ignore_ascii_case(q)
        }) {
            return Some(t.name.clone());
        }
        // Unknown qualifier → fall through to the base collection (embedded path).
    }
    sctx.tables
        .first()
        .map(|t| t.name.clone())
        .or_else(|| node_coll.map(str::to_string))
}

/// SQL keywords offered when completing the SQL dialect Mongo accepts — kept to
/// the subset `mongo_sql` actually translates (single-word tokens so they match
/// the editor's `[\w$.]` completion boundary).
const MONGO_SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IN", "IS", "NULL", "LIKE", "BETWEEN", "GROUP",
    "ORDER", "BY", "LIMIT", "ASC", "DESC", "AS", "JOIN", "INNER", "LEFT", "ON", "COUNT",
    "DISTINCT",
];

/// SQL aggregate functions `mongo_sql` maps to `$group` accumulators / countDocuments.
const MONGO_SQL_FUNCTIONS: &[(&str, &str)] = &[
    ("COUNT", "COUNT(*) — count documents"),
    ("SUM", "SUM(field) — sum a numeric field"),
    ("AVG", "AVG(field) — average a numeric field"),
    ("MIN", "MIN(field) — minimum of a field"),
    ("MAX", "MAX(field) — maximum of a field"),
];

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

    fn script_cfg(params: Value) -> ResolvedConfig {
        ResolvedConfig {
            engine: crate::types::Engine::Mongodb,
            host: "127.0.0.1".into(),
            port: 27017,
            user: Some("root".into()),
            password: Some("p@ss/w".into()),
            database: None,
            tls: Default::default(),
            params,
        }
    }

    /// The script runner must dial EXACTLY what the native client dials:
    /// host/port + percent-encoded credentials + authSource/replicaSet, the
    /// selected database in the URI path, and — when the service opened an SSH
    /// tunnel — the SOCKS5 proxy as Node-driver URI options.
    #[test]
    fn mongosh_invocation_builds_the_native_equivalent_uri() {
        let uri = mongosh_invocation(&script_cfg(json!({})), Some("db:promotions")).unwrap();
        assert_eq!(
            uri,
            "mongodb://root:p%40ss%2Fw@127.0.0.1:27017/promotions?authSource=admin"
        );

        // Tunnelled → SOCKS options appended to the existing query string.
        let uri = mongosh_invocation(
            &script_cfg(json!({ "__socks_port": 1080, "replica_set": "rs0" })),
            Some("db:promotions"),
        )
        .unwrap();
        assert!(uri.contains("replicaSet=rs0"), "uri: {uri}");
        assert!(
            uri.ends_with("&proxyHost=127.0.0.1&proxyPort=1080"),
            "uri: {uri}"
        );
    }

    /// A full `conn_string` wins verbatim (with `{secret}` substituted), and
    /// the SOCKS options join with the right separator.
    #[test]
    fn mongosh_invocation_conn_string_wins_with_secret_substitution() {
        let mut cfg = script_cfg(json!({
            "conn_string": "mongodb+srv://u:{secret}@cluster.example.net/app?retryWrites=true",
            "__socks_port": 1081,
        }));
        cfg.password = Some("s3c".into());
        let uri = mongosh_invocation(&cfg, None).unwrap();
        assert_eq!(
            uri,
            "mongodb+srv://u:s3c@cluster.example.net/app?retryWrites=true&proxyHost=127.0.0.1&proxyPort=1081"
        );
    }

    /// The credential-bearing URI reaches mongosh through the 0600 script file
    /// (`db = connect("…")`), never argv — the prelude must be a valid JS string
    /// literal even when the URI holds quotes/backslashes.
    #[test]
    fn mongosh_prelude_json_escapes_the_uri() {
        assert_eq!(
            mongosh_script_prelude("mongodb://u:p@h:1/db"),
            "db = connect(\"mongodb://u:p@h:1/db\");\n"
        );
        assert_eq!(
            mongosh_script_prelude("mongodb://u:p\"x\\@h:1/db"),
            "db = connect(\"mongodb://u:p\\\"x\\\\@h:1/db\");\n"
        );
    }

    #[test]
    fn binary_uuid_subtype_renders_uuid_string() {
        use mongodb::bson::spec::BinarySubtype;
        let uuid = BsonUuid::new();
        let bin = mongodb::bson::Binary {
            subtype: BinarySubtype::Uuid,
            bytes: uuid.bytes().to_vec(),
        };
        assert_eq!(binary_to_string(&bin), uuid.to_string());
        // Non-UUID subtypes stay base64.
        let raw = mongodb::bson::Binary {
            subtype: BinarySubtype::Generic,
            bytes: vec![1, 2, 3],
        };
        assert_eq!(binary_to_string(&raw), base64_encode(&[1, 2, 3]));
        // A malformed 15-byte "uuid" must not panic — falls back to base64.
        let bad = mongodb::bson::Binary {
            subtype: BinarySubtype::Uuid,
            bytes: vec![0; 15],
        };
        assert_eq!(binary_to_string(&bad), base64_encode(&[0; 15]));
    }

    // --- SQL-dialect completion routing + collection resolution ----------------

    #[test]
    fn sql_routing_predicate() {
        use crate::complete::sql::current_statement;
        // SQL statements route to the SQL completion path…
        assert!(mongo_sql::looks_like_sql(current_statement(
            "SELECT * FROM customers WHERE "
        )));
        assert!(mongo_sql::looks_like_sql(current_statement(
            "db.x.find({}); SELECT * FROM c WHERE "
        )));
        // …native Mongo shorthand / JSON commands do NOT.
        assert!(!mongo_sql::looks_like_sql(current_statement(
            "db.customers.find({ "
        )));
        assert!(!mongo_sql::looks_like_sql(current_statement(
            "db.customers.aggregate([{ $match: { "
        )));
    }

    #[test]
    fn resolve_collection_unqualified_uses_base() {
        let sctx = crate::complete::sql::analyze("SELECT * FROM customers WHERE ", "");
        assert_eq!(
            resolve_sql_collection(&sctx, None).as_deref(),
            Some("customers")
        );
    }

    #[test]
    fn resolve_collection_alias_qualifier() {
        let sctx = crate::complete::sql::analyze("SELECT * FROM orders o WHERE o.", "");
        assert_eq!(
            resolve_sql_collection(&sctx, None).as_deref(),
            Some("orders")
        );
    }

    #[test]
    fn resolve_collection_embedded_path_falls_back_to_base() {
        // `WHERE address.` — `address` is NOT a table; it's an embedded field path
        // on the base collection, so completion targets the base collection.
        let sctx = crate::complete::sql::analyze("SELECT * FROM profiles WHERE address.", "");
        assert_eq!(
            resolve_sql_collection(&sctx, None).as_deref(),
            Some("profiles")
        );
    }

    #[test]
    fn resolve_collection_no_from_uses_tree_node() {
        let sctx = crate::complete::sql::analyze("WHERE ", "");
        assert_eq!(
            resolve_sql_collection(&sctx, Some("customers")).as_deref(),
            Some("customers")
        );
    }

    #[test]
    fn shorthand_find_basic() {
        let p = parse_command("db.customers.find({})").unwrap();
        assert_eq!(p.collection, "customers");
        assert_eq!(p.op, MongoOp::Find);
    }

    #[test]
    fn shorthand_find_with_filter_limit_sort() {
        let p = parse_command(r#"db.orders.find({"status":"paid"}).sort({"total":-1}).limit(5)"#)
            .unwrap();
        assert_eq!(p.collection, "orders");
        assert_eq!(p.op, MongoOp::Find);
        assert_eq!(p.limit, Some(5));
        assert_eq!(p.filter.unwrap().get_str("status").unwrap(), "paid");
        // serde_json integers become BSON Int64.
        assert_eq!(p.sort.unwrap().get_i64("total").unwrap(), -1);
    }

    #[test]
    fn shorthand_aggregate() {
        let p =
            parse_command(r#"db.events.aggregate([{"$match":{"k":1}},{"$count":"n"}])"#).unwrap();
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
    fn shorthand_replace_one() {
        // The grid's JSON-view document editor emits replaceOne by `_id`.
        let p = parse_command(
            r#"db.orders.replaceOne({"_id": {"$oid": "6725e2532c39b0477e55d679"}}, {"status": "paid", "items": [{"qty": 5}]})"#,
        )
        .unwrap();
        assert_eq!(p.collection, "orders");
        assert_eq!(p.op, MongoOp::ReplaceOne);
        assert!(p.filter.unwrap().contains_key("_id"));
        let replacement = p.update.unwrap();
        assert_eq!(replacement.get_str("status").unwrap(), "paid");
        assert!(replacement.contains_key("items"));
    }

    /// `\"` inside a string literal must not miscount bracket depth or split
    /// args — `find({k:"a\"b)"})` is valid input.
    #[test]
    fn shorthand_honours_escaped_quotes_in_strings() {
        let p = parse_command(r#"db.c.find({k:"a\"b)"})"#).unwrap();
        assert_eq!(p.op, MongoOp::Find);
        assert_eq!(p.filter.unwrap().get_str("k").unwrap(), "a\"b)");
        // And in a two-arg call the top-level comma split ignores an escaped
        // quote followed by a comma inside the string.
        let p = parse_command(r#"db.c.find({k:"a\",b"}, {k:1})"#).unwrap();
        assert_eq!(p.filter.unwrap().get_str("k").unwrap(), "a\",b");
        assert!(p.projection.is_some());
    }

    #[test]
    fn splits_multi_statement_paste_with_comments() {
        let src = r#"
            // header comment
            db.dashboards.deleteOne({ dashboardId: "x" });
            db.dashboards.insertOne({ dashboardId: "x", note: "a; b inside a string" });
        "#;
        let stmts = split_statements(src);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("db.dashboards.deleteOne"));
        assert!(stmts[1].starts_with("db.dashboards.insertOne"));
    }

    #[test]
    fn single_statement_without_semicolon_is_one() {
        assert_eq!(split_statements("db.players.find({})").len(), 1);
        // Comment-only / blank input yields no statements.
        assert!(split_statements("// just a comment\n").is_empty());
    }

    #[test]
    fn parses_mongosh_insert_with_date_and_backtick() {
        // The exact troublesome shape: unquoted keys, new Date(), a backtick SQL
        // template with parens + single quotes, nested array, trailing commas.
        let p = parse_command(
            "db.dashboards.insertOne({ dashboardId: \"player-activities\", createdAt: new Date(), \
             widgets: [{ id: \"w1\", query: `SELECT a FROM t WHERE x IN ('A','B')` },], })",
        )
        .unwrap();
        assert_eq!(p.collection, "dashboards");
        assert_eq!(p.op, MongoOp::InsertOne);
        let docs = p.documents.unwrap();
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.get_str("dashboardId").unwrap(), "player-activities");
        // new Date() decoded to a real BSON DateTime, not a string/object.
        assert!(matches!(doc.get("createdAt"), Some(Bson::DateTime(_))));
        // Backtick SQL preserved verbatim inside the nested widget.
        let w0 = doc.get_array("widgets").unwrap()[0].as_document().unwrap();
        assert!(w0.get_str("query").unwrap().contains("IN ('A','B')"));
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

    #[test]
    fn structure_sample_is_bounded_by_bytes_not_document_count() {
        // Small documents: the flat sample is already cheap, keep it.
        assert_eq!(structure_sample_size(Some(1_024)), SAMPLE_SIZE);
        assert_eq!(structure_sample_size(Some(20_480)), SAMPLE_SIZE);
        // Fat documents (the `lobby_format_history` case, ~370KB): a flat 100
        // would pull ~37MB per pass. Bound it by the byte budget instead.
        let n = structure_sample_size(Some(370_000));
        assert!(n < SAMPLE_SIZE, "expected a reduced sample, got {n}");
        assert!(
            n * 370_000 <= STRUCTURE_SAMPLE_BYTES,
            "over budget: {n} docs"
        );
        // Never collapse to nothing — a heterogeneous collection still needs a
        // few documents to show its shape, even when each one is enormous.
        assert_eq!(
            structure_sample_size(Some(50_000_000)),
            STRUCTURE_SAMPLE_MIN
        );
        // Unknown / nonsense stats fall back to the old behaviour.
        assert_eq!(structure_sample_size(None), SAMPLE_SIZE);
        assert_eq!(structure_sample_size(Some(0)), SAMPLE_SIZE);
    }

    #[test]
    fn bson_to_json_typed_preserves_oid_and_date() {
        let oid = mongodb::bson::oid::ObjectId::new();
        let dt = mongodb::bson::DateTime::from_millis(1_565_191_869_123);
        let doc = doc! {
            "_id": oid,
            "createdDate": dt,
            "name": "x",
            "nested": { "innerDate": dt },
        };
        let v = bson_to_json_typed(&Bson::Document(doc));
        let obj = v.as_object().unwrap();
        // ObjectId → {"$oid": hex}; the UI renders this as ObjectId("…").
        assert_eq!(obj.get("_id").unwrap(), &json!({ "$oid": oid.to_hex() }));
        // DateTime → {"$date": iso}; nested dates are typed too (recursive).
        assert!(obj.get("createdDate").unwrap().get("$date").is_some());
        assert!(obj
            .get("nested")
            .unwrap()
            .get("innerDate")
            .unwrap()
            .get("$date")
            .is_some());
        // Plain scalars are unchanged.
        assert_eq!(obj.get("name").unwrap(), "x");
    }

    // ---- server-side cancel helpers -----------------------------------------

    #[test]
    fn mongo_comment_tag_is_namespaced_by_query_id() {
        assert_eq!(mongo_comment_tag("q-42"), "otto:q-42");
        assert_eq!(mongo_comment_tag(""), "otto:");
    }

    #[test]
    fn current_op_pipeline_matches_comment_and_projects_opid() {
        let p = current_op_pipeline("otto:q-42");
        assert_eq!(
            p,
            vec![
                doc! { "$currentOp": { "allUsers": true, "localOps": true } },
                doc! { "$match": { "command.comment": "otto:q-42" } },
                doc! { "$project": { "opid": 1 } },
            ]
        );
        // The unprivileged retry differs ONLY in `allUsers`.
        let scoped = current_op_pipeline_scoped("otto:q-42", false);
        assert_eq!(
            scoped[0],
            doc! { "$currentOp": { "allUsers": false, "localOps": true } }
        );
        assert_eq!(scoped[1..], p[1..]);
    }

    #[test]
    fn opids_from_current_op_keeps_raw_bson_and_skips_rows_without_one() {
        // mongod reports a number, mongos a "shard:n" string — both go back to
        // killOp verbatim; a row without `opid` is ignored.
        let docs = vec![
            doc! { "opid": 1234 },
            doc! { "opid": "shard01:987" },
            doc! { "desc": "conn12" },
        ];
        assert_eq!(
            opids_from_current_op(&docs),
            vec![Bson::Int32(1234), Bson::String("shard01:987".into())]
        );
        assert!(opids_from_current_op(&[]).is_empty());
    }

    #[test]
    fn is_unauthorized_matches_privilege_refusals_only() {
        assert!(is_unauthorized(&"Command failed: Unauthorized"));
        assert!(is_unauthorized(
            &"not authorized on admin to execute command { aggregate: 1, pipeline: [ { $currentOp: … } ] }"
        ));
        assert!(!is_unauthorized(&"connection reset by peer"));
    }

    // ---- keyset pagination --------------------------------------------------

    #[test]
    fn keyset_filter_rejects_explicit_limit_other_sort_and_id_filter() {
        // An explicit `.limit(n)` is never paged (neither by offset nor keyset).
        assert!(keyset_filter(None, None, true, None).unwrap().is_none());
        // A sort on another field (or descending `_id`) needs the offset path.
        assert!(keyset_filter(None, Some(&doc! { "age": -1 }), false, None)
            .unwrap()
            .is_none());
        assert!(keyset_filter(None, Some(&doc! { "_id": -1 }), false, None)
            .unwrap()
            .is_none());
        assert!(
            keyset_filter(None, Some(&doc! { "_id": 1, "age": 1 }), false, None)
                .unwrap()
                .is_none()
        );
        // A filter already pinning `_id` keeps the order the user asked for.
        let f = doc! { "_id": { "$in": [1, 2] } };
        assert!(keyset_filter(Some(&f), None, false, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn keyset_filter_forces_id_sort_on_page_one() {
        // Eligible without a cursor: the filter is untouched, the sort becomes
        // `{_id: 1}` — page 1 included, so every page shares one order.
        let f = doc! { "country": "US" };
        let (filter, sort) = keyset_filter(Some(&f), None, false, None).unwrap().unwrap();
        assert_eq!(filter, f);
        assert_eq!(sort, doc! { "_id": 1 });
        // No filter at all ⇒ an empty filter document.
        let (filter, sort) = keyset_filter(None, None, false, None).unwrap().unwrap();
        assert_eq!(filter, doc! {});
        assert_eq!(sort, doc! { "_id": 1 });
        // An explicit `{_id: 1}` sort is eligible too (any numeric 1).
        assert!(
            keyset_filter(None, Some(&doc! { "_id": 1_i64 }), false, None)
                .unwrap()
                .is_some()
        );
        assert!(keyset_filter(None, Some(&doc! { "_id": 1.0 }), false, None)
            .unwrap()
            .is_some());
    }

    #[test]
    fn keyset_filter_with_cursor_ands_a_typed_gt_on_id() {
        let oid = mongodb::bson::oid::ObjectId::new();
        let f = doc! { "$or": [{ "a": 1 }, { "b": 2 }] };
        let cursor = json!({ "$oid": oid.to_hex() });
        let (filter, sort) = keyset_filter(Some(&f), None, false, Some(&cursor))
            .unwrap()
            .unwrap();
        // `$and` keeps a top-level `$or` intact, and the cursor is decoded to a
        // real ObjectId (not compared as a string / sub-document).
        assert_eq!(filter, doc! { "$and": [f, { "_id": { "$gt": oid } }] });
        assert_eq!(sort, doc! { "_id": 1 });
        // No filter + cursor ⇒ `$and: [{}, …]` (harmless, and keeps one shape).
        let cursor = json!(41);
        let (filter, _) = keyset_filter(None, None, false, Some(&cursor))
            .unwrap()
            .unwrap();
        assert_eq!(filter, doc! { "$and": [{}, { "_id": { "$gt": 41_i64 } }] });
        // A cursor that cannot be decoded is the caller's error, not a silent skip.
        let bad = json!({ "$oid": "nope" });
        assert!(keyset_filter(None, None, false, Some(&bad)).is_err());
    }

    #[test]
    fn last_row_id_reads_the_id_column_of_the_last_row() {
        let started = Instant::now();
        let a = mongodb::bson::oid::ObjectId::new();
        let b = mongodb::bson::oid::ObjectId::new();
        let r = docs_to_result(
            vec![doc! { "_id": a, "n": 1 }, doc! { "n": 2, "_id": b }],
            true,
            started,
        );
        // `_id` is pinned to column 0 whatever the document order was.
        assert_eq!(last_row_id(&r), Some(json!({ "$oid": b.to_hex() })));
        // No `_id` column (projected away) / no rows ⇒ no cursor.
        let r = docs_to_result(vec![doc! { "n": 1 }], true, started);
        assert!(last_row_id(&r).is_none());
        assert!(last_row_id(&QueryResult::empty()).is_none());
    }

    // ---- type fidelity ------------------------------------------------------

    #[test]
    fn bson_to_json_typed_gates_int64_at_2_pow_53() {
        // Within ±2^53 a Long is exact in a JS number — keep it plain.
        assert_eq!(bson_to_json_typed(&Bson::Int64(42)), json!(42));
        assert_eq!(
            bson_to_json_typed(&Bson::Int64(1 << 53)),
            json!(9007199254740992_i64)
        );
        assert_eq!(
            bson_to_json_typed(&Bson::Int64(-(1 << 53))),
            json!(-9007199254740992_i64)
        );
        // Beyond it the digits would round in the webview — sentinel, both signs.
        assert_eq!(
            bson_to_json_typed(&Bson::Int64((1 << 53) + 1)),
            json!({ "$numberLong": "9007199254740993" })
        );
        assert_eq!(
            bson_to_json_typed(&Bson::Int64(i64::MIN)),
            json!({ "$numberLong": "-9223372036854775808" })
        );
        // …and it decodes back to the same Int64.
        let back = json_to_bson(&json!({ "$numberLong": "9007199254740993" })).unwrap();
        assert_eq!(back, Bson::Int64(9007199254740993));
        // Int32 is never a sentinel.
        assert_eq!(bson_to_json_typed(&Bson::Int32(7)), json!(7));
    }

    #[test]
    fn bson_to_json_typed_binary_uuid_vs_generic() {
        let uuid = BsonUuid::new();
        let v = bson_to_json_typed(&Bson::Binary(BsonBinary::from(uuid)));
        assert_eq!(v, json!({ "$uuid": uuid.to_string() }));
        // Legacy UUID subtype (3) renders the same way.
        let legacy = BsonBinary {
            subtype: BinarySubtype::UuidOld,
            bytes: uuid.bytes().to_vec(),
        };
        assert_eq!(
            bson_to_json_typed(&Bson::Binary(legacy)),
            json!({ "$uuid": uuid.to_string() })
        );
        // Generic bytes → canonical `$binary` with a 2-hex lowercase subtype.
        let raw = BsonBinary {
            subtype: BinarySubtype::Generic,
            bytes: b"hello".to_vec(),
        };
        assert_eq!(
            bson_to_json_typed(&Bson::Binary(raw)),
            json!({ "$binary": { "base64": "aGVsbG8=", "subType": "00" } })
        );
        let user = BsonBinary {
            subtype: BinarySubtype::UserDefined(0x80),
            bytes: vec![1, 2],
        };
        assert_eq!(
            bson_to_json_typed(&Bson::Binary(user))["$binary"]["subType"],
            json!("80")
        );
    }

    #[test]
    fn binary_and_uuid_round_trip_through_typed_json() {
        let raw = Bson::Binary(BsonBinary {
            subtype: BinarySubtype::Md5,
            bytes: vec![0, 255, 16],
        });
        assert_eq!(json_to_bson(&bson_to_json_typed(&raw)).unwrap(), raw);
        let uuid = Bson::Binary(BsonBinary::from(BsonUuid::new()));
        assert_eq!(json_to_bson(&bson_to_json_typed(&uuid)).unwrap(), uuid);
        // A missing subType defaults to generic; a bad one is an error.
        assert_eq!(
            json_to_bson(&json!({ "$binary": { "base64": "AQI=" } })).unwrap(),
            Bson::Binary(BsonBinary {
                subtype: BinarySubtype::Generic,
                bytes: vec![1, 2]
            })
        );
        assert!(
            json_to_bson(&json!({ "$binary": { "base64": "AQI=", "subType": "zz" } })).is_err()
        );
        assert!(json_to_bson(&json!({ "$binary": { "base64": "not base64!" } })).is_err());
    }

    #[test]
    fn timestamp_round_trips_through_typed_json() {
        let ts = Bson::Timestamp(BsonTimestamp {
            time: 1_700_000_000,
            increment: 7,
        });
        let v = bson_to_json_typed(&ts);
        assert_eq!(
            v,
            json!({ "$timestamp": { "t": 1_700_000_000_u32, "i": 7 } })
        );
        assert_eq!(json_to_bson(&v).unwrap(), ts);
        // Nested inside a document/array it is typed and decoded the same way.
        let doc = doc! { "ops": [ts.clone()] };
        let back = json_to_bson(&bson_to_json_typed(&Bson::Document(doc.clone()))).unwrap();
        assert_eq!(back, Bson::Document(doc));
        // Out-of-range / missing fields are errors.
        assert!(json_to_bson(&json!({ "$timestamp": { "t": 1 } })).is_err());
        assert!(json_to_bson(&json!({ "$timestamp": { "t": 1, "i": 4294967296_u64 } })).is_err());
    }

    #[test]
    fn parses_update_one_with_set_unset_rename_and_typed_sentinels() {
        let p = parse_command(
            r#"db.c.updateOne({_id:{"$oid":"5f1d7f3e2c4b1a0001234567"}}, {"$set":{"a.b":1,"big":{"$numberLong":"9007199254740993"},"at":{"$date":"2024-01-02T03:04:05Z"}},"$unset":{"x":""},"$rename":{"o":"n"}})"#,
        )
        .unwrap();
        assert_eq!(p.collection, "c");
        assert_eq!(p.op, MongoOp::UpdateOne);
        // The `_id` filter decodes to a real ObjectId.
        let filter = p.filter.unwrap();
        assert!(matches!(filter.get("_id"), Some(Bson::ObjectId(_))));
        // All three operators survive as separate top-level keys…
        let update = p.update.unwrap();
        let set = update.get_document("$set").unwrap();
        assert_eq!(set.get("a.b"), Some(&Bson::Int64(1)));
        assert_eq!(set.get("big"), Some(&Bson::Int64(9007199254740993)));
        assert!(matches!(set.get("at"), Some(Bson::DateTime(_))));
        assert_eq!(
            update.get_document("$unset").unwrap().get_str("x").unwrap(),
            ""
        );
        assert_eq!(
            update
                .get_document("$rename")
                .unwrap()
                .get_str("o")
                .unwrap(),
            "n"
        );
    }
}

/// End-to-end SQL → Mongo (plus server-side cancel and keyset paging) over a
/// real MongoDB Docker container. Ignored by default (needs Docker). Run with:
///   cargo test -p otto-dbviewer --lib -- --ignored --nocapture sql_to_mongo_e2e
///   cargo test -p otto-dbviewer --lib -- --ignored --nocapture mongo_cancel_kills_tagged_find_e2e
///   cargo test -p otto-dbviewer --lib -- --ignored --nocapture mongo_keyset_walk_e2e
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
            let _ = Command::new("docker")
                .args(["rm", "-f", CONTAINER])
                .output();
        }
    }

    fn cfg() -> ResolvedConfig {
        cfg_on(PORT)
    }

    fn cfg_on(port: u16) -> ResolvedConfig {
        ResolvedConfig {
            engine: Engine::Mongodb,
            host: "127.0.0.1".into(),
            port,
            user: None,
            password: None,
            database: Some("shop".into()),
            tls: TlsConfig::default(),
            params: serde_json::json!({}),
        }
    }

    /// Start a throwaway `mongo` container named `name` on `port` and wait for
    /// it. The returned guard removes it on drop (the tests below each use
    /// their own so `--ignored` can run them in parallel).
    struct Container(&'static str);
    impl Drop for Container {
        fn drop(&mut self) {
            let _ = Command::new("docker").args(["rm", "-f", self.0]).output();
        }
    }
    async fn start_container(name: &'static str, port: u16) -> (Container, Client) {
        let _ = Command::new("docker").args(["rm", "-f", name]).output();
        let out = Command::new("docker")
            .args([
                "run",
                "-d",
                "--name",
                name,
                "-p",
                &format!("{port}:27017"),
                IMAGE,
            ])
            .output()
            .expect("docker run");
        assert!(
            out.status.success(),
            "docker run failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let guard = Container(name);
        let client = wait_for_mongo(&format!("mongodb://127.0.0.1:{port}")).await;
        (guard, client)
    }

    fn id_cells(r: &QueryResult) -> Vec<Value> {
        let idx = r
            .columns
            .iter()
            .position(|c| c.name == "_id")
            .expect("_id column");
        r.rows.iter().map(|row| row[idx].clone()).collect()
    }

    /// A tracked `find` blocked in a server-side `$where: sleep(5000)` (5 docs
    /// ⇒ ≥25 s untouched) is stamped `comment: "otto:<query_id>"`, shows up in
    /// `$currentOp` under that tag, and `cancel` kills it: the run ends with an
    /// error well inside 5 s.
    #[tokio::test]
    #[ignore = "requires docker"]
    async fn mongo_cancel_kills_tagged_find_e2e() {
        const CANCEL_PORT: u16 = 47020;
        let (_c, client) = start_container("otto-mongo-cancel-e2e", CANCEL_PORT).await;
        seed(&client).await;
        let d = std::sync::Arc::new(MongoDriver::default());
        let token = CancelToken::new();
        let req = QueryRequest {
            statement: r#"db.players.find({"$where": "sleep(5000) || true"})"#.into(),
            query_id: Some("e2e-cancel".into()),
            max_rows: Some(10),
            ..Default::default()
        };
        let started = Instant::now();
        let run = {
            let (d, token) = (std::sync::Arc::clone(&d), token.clone());
            tokio::spawn(async move { d.run_tracked(&cfg_on(CANCEL_PORT), &req, &token).await })
        };
        // The tag is published before the find is issued…
        let handle = loop {
            if let Some(h) = token.handle() {
                break h;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let QueryHandle::MongoComment(tag) = &handle else {
            panic!("expected a MongoComment handle, got {handle:?}")
        };
        assert_eq!(tag, "otto:e2e-cancel");
        // …and the op is visible in `$currentOp` under it once the server
        // starts evaluating the `$where`.
        let admin = client.database("admin");
        let mut seen = Vec::new();
        for _ in 0..100 {
            seen = current_ops(&admin, current_op_pipeline(tag)).await.unwrap();
            if !seen.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!seen.is_empty(), "tagged op never appeared in $currentOp");

        d.cancel(&cfg_on(CANCEL_PORT), &handle).await.unwrap();
        let outcome = run.await.unwrap();
        assert!(
            outcome.is_err(),
            "a killed find must surface the interruption, got {outcome:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancel took {:?} — the sleep ran to completion",
            started.elapsed()
        );
        // Cancelling an already-finished op is a successful no-op.
        d.cancel(&cfg_on(CANCEL_PORT), &handle).await.unwrap();
    }

    /// Keyset walk over 2,500 documents at 1,000 rows/page: `next_cursor` on the
    /// truncated pages only, no duplicate `_id` across the walk, "Prev" by offset
    /// lands on exactly the page "Next" by cursor produced (same forced order),
    /// and a non-eligible find ignores the cursor.
    #[tokio::test]
    #[ignore = "requires docker"]
    async fn mongo_keyset_walk_e2e() {
        const KEYSET_PORT: u16 = 47021;
        let (_c, client) = start_container("otto-mongo-keyset-e2e", KEYSET_PORT).await;
        let coll = client.database("shop").collection::<Document>("keyset");
        let docs: Vec<Document> = (0..2500)
            .map(|n| doc! { "n": n, "even": n % 2 == 0 })
            .collect();
        coll.insert_many(docs).await.unwrap();
        let d = MongoDriver::default();
        let cfg = cfg_on(KEYSET_PORT);
        let page = |cursor: Option<Value>, offset: Option<u64>, stmt: &str| QueryRequest {
            statement: stmt.into(),
            max_rows: Some(1000),
            offset,
            cursor,
            ..Default::default()
        };

        let p1 = d
            .run(&cfg, &page(None, None, "db.keyset.find({})"))
            .await
            .unwrap();
        assert_eq!(p1.rows.len(), 1000);
        assert!(p1.truncated);
        assert_eq!(p1.auto_limited, Some(1000));
        let c1 = p1.next_cursor.clone().expect("page 1 offers a cursor");
        assert_eq!(&c1, id_cells(&p1).last().unwrap());

        let p2 = d
            .run(&cfg, &page(Some(c1), Some(1000), "db.keyset.find({})"))
            .await
            .unwrap();
        assert_eq!(p2.rows.len(), 1000);
        let c2 = p2.next_cursor.clone().expect("page 2 offers a cursor");

        let p3 = d
            .run(&cfg, &page(Some(c2), Some(2000), "db.keyset.find({})"))
            .await
            .unwrap();
        assert_eq!(p3.rows.len(), 500);
        assert!(!p3.truncated);
        assert!(p3.next_cursor.is_none(), "the last page offers no cursor");

        // No duplicates, and the walk is in ascending `n` (= insertion = `_id`) order.
        let mut all = id_cells(&p1);
        all.extend(id_cells(&p2));
        all.extend(id_cells(&p3));
        let unique: std::collections::BTreeSet<String> =
            all.iter().map(|v| v.to_string()).collect();
        assert_eq!(unique.len(), 2500);
        let n_idx = p2.columns.iter().position(|c| c.name == "n").unwrap();
        assert_eq!(p2.rows[0][n_idx], json!(1000));
        assert_eq!(p3.rows[499][n_idx], json!(2499));

        // "Prev" from page 3 is offset-based (no cursor) and must reproduce page 2
        // exactly — the forced `{_id: 1}` makes both paths walk one order.
        let prev = d
            .run(&cfg, &page(None, Some(1000), "db.keyset.find({})"))
            .await
            .unwrap();
        assert_eq!(id_cells(&prev), id_cells(&p2));
        assert_eq!(prev.next_cursor, p2.next_cursor);

        // A filter + cursor keeps the filter (`$and`) — the even half only.
        let e1 = d
            .run(&cfg, &page(None, None, "db.keyset.find({even: true})"))
            .await
            .unwrap();
        let e2 = d
            .run(
                &cfg,
                &page(
                    e1.next_cursor.clone(),
                    Some(1000),
                    "db.keyset.find({even: true})",
                ),
            )
            .await
            .unwrap();
        assert_eq!(e2.rows.len(), 250);
        assert!(e2.next_cursor.is_none());
        assert!(e2.rows.iter().all(|r| r[n_idx].as_i64().unwrap() % 2 == 0));

        // Not eligible (sort on another field): the cursor is ignored, `skip`
        // pages, and no cursor is offered.
        let s1 = d
            .run(&cfg, &page(None, None, "db.keyset.find({}).sort({n: -1})"))
            .await
            .unwrap();
        assert!(s1.next_cursor.is_none());
        let bogus = id_cells(&s1)[0].clone();
        let s2 = d
            .run(
                &cfg,
                &page(Some(bogus), Some(2000), "db.keyset.find({}).sort({n: -1})"),
            )
            .await
            .unwrap();
        assert_eq!(s2.rows.len(), 500);
        assert_eq!(s2.rows[0][n_idx], json!(499));
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
                if c.database("admin")
                    .run_command(doc! {"ping": 1})
                    .await
                    .is_ok()
                {
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
        let _ = Command::new("docker")
            .args(["rm", "-f", CONTAINER])
            .output();
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
            run_sql(&d, "SELECT COUNT(*) FROM players WHERE country = 'US'")
                .await
                .rows[0][0],
            json!(3)
        );

        // 4–8. IN / NOT IN / LIKE / NOT(...) / BETWEEN.
        assert_eq!(
            run_sql(&d, "SELECT * FROM players WHERE country IN ('CA','UK')")
                .await
                .rows
                .len(),
            2
        );
        assert_eq!(
            run_sql(&d, "SELECT * FROM players WHERE country NOT IN ('US')")
                .await
                .rows
                .len(),
            2
        );
        assert_eq!(
            run_sql(&d, "SELECT * FROM players WHERE name LIKE 'a%'")
                .await
                .rows
                .len(),
            2
        );
        assert_eq!(
            run_sql(&d, "SELECT * FROM players WHERE NOT (country = 'US')")
                .await
                .rows
                .len(),
            2
        );
        assert_eq!(
            run_sql(&d, "SELECT * FROM players WHERE age BETWEEN 26 AND 36")
                .await
                .rows
                .len(),
            2
        );

        // 9. GROUP BY aggregate → 3 country groups.
        assert_eq!(
            run_sql(
                &d,
                "SELECT country, COUNT(*) AS n FROM players GROUP BY country"
            )
            .await
            .rows
            .len(),
            3
        );

        // 10. global aggregate (no GROUP BY).
        assert_eq!(
            run_sql(&d, "SELECT AVG(age) AS avg_age FROM players")
                .await
                .rows
                .len(),
            1
        );

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
        run_sql(
            &d,
            r#"db.players.updateOne({"id": 1}, {"$set": {"age": 99}})"#,
        )
        .await;
        assert_eq!(
            run_sql(&d, "SELECT age FROM players WHERE id = 1")
                .await
                .rows[0][run_sql(&d, "SELECT age FROM players WHERE id = 1")
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
        // `_id` renders as the `{"$oid": …}` sentinel (or a bare hex string in
        // older shapes) — accept both.
        let id_cell = &all.rows[0][id_idx];
        let oid = id_cell
            .as_str()
            .map(str::to_string)
            .or_else(|| {
                id_cell
                    .get("$oid")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .unwrap();
        run_sql(
            &d,
            &format!(
                r#"db.players.updateOne({{"_id": {{"$oid": "{oid}"}}}}, {{"$set": {{"country": "ZZ"}}}})"#
            ),
        )
        .await;
        assert_eq!(
            run_sql(&d, "SELECT * FROM players WHERE country = 'ZZ'")
                .await
                .rows
                .len(),
            1
        );

        // 16. createIndex / getIndexes / dropIndex.
        let r = run_sql(&d, r#"db.players.createIndex({"country": 1})"#).await;
        assert!(r.message.as_deref().unwrap_or("").contains("created index"));
        // getIndexes returns ROWS (one per index) with the raw listIndexes spec —
        // `_id_` plus the one just created — not a write summary.
        let r = run_sql(&d, "db.players.getIndexes()").await;
        assert!(r.columns.iter().any(|c| c.name == "name"));
        assert!(r.columns.iter().any(|c| c.name == "key"));
        let names: Vec<String> = r
            .rows
            .iter()
            .filter_map(|row| {
                let i = r.columns.iter().position(|c| c.name == "name")?;
                row.get(i)?.as_str().map(str::to_string)
            })
            .collect();
        assert!(names.iter().any(|n| n == "_id_"), "got {names:?}");
        assert!(names.iter().any(|n| n == "country_1"), "got {names:?}");
        let r = run_sql(&d, r#"db.players.dropIndex("country_1")"#).await;
        assert!(r.message.as_deref().unwrap_or("").contains("dropped index"));
        // …and the dropped index is gone from the listing.
        let r = run_sql(&d, "db.players.getIndexes()").await;
        assert_eq!(r.rows.len(), 1, "only _id_ should remain");

        // 17. explain returns a single query-plan row.
        let r = run_sql(&d, r#"db.players.find({"country": "US"}).explain()"#).await;
        assert_eq!(r.rows.len(), 1);
        assert!(r.columns.iter().any(|c| c.name == "queryPlan"));
    }
}
