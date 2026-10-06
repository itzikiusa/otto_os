//! PostgreSQL driver (also Amazon RDS/Aurora Postgres, CockroachDB-compatible).
//!
//! Mirrors the MySQL driver's structure ([`crate::drivers::mysql`]): connects via
//! `sqlx`'s `PgConnectOptions` (host/port/user/password/database + TLS),
//! introspects the catalog (`pg_catalog` + `information_schema`), decodes rows to
//! `serde_json::Value`, and populates `foreign_keys` for the visual JOIN builder.
//! Connection *pools* are cached per [`ResolvedConfig::cache_key`] and reused.
//!
//! Postgres scopes unqualified names by **schema** within a connection's single
//! database, so the tree's top level is the database's schemas (public first,
//! `pg_*`/`information_schema` hidden) and the "active database" selector maps to
//! `SET search_path` — the `db:<schema>` node segment carries the schema name,
//! keeping every downstream path (`db:<schema>/table:<t>`) identical in shape to
//! MySQL's.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::resource_cache::ResourceCache;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use otto_core::Result;
use serde_json::Value;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow, PgSslMode};
use sqlx::{Column as _, Connection as _, Executor as _, Row, TypeInfo};

use crate::driver::Driver;
use crate::export::{ExportCounts, ExportFormat, ExportSink};
use crate::split::{split_statements, SqlDialect, StatementSpan};
use crate::tls::TlsFiles;
use crate::types::{
    self, CancelToken, Capabilities, Column, ColumnDef, CompletionContext, CompletionResponse,
    DbQueryPlan, Engine, ForeignKey, IndexDef, NodeKind, NodePath, ObjectDetail, ObjectHit,
    ObjectSearchReq, ObjectSearchResult, QueryHandle, QueryRequest, QueryResult, ResolvedConfig,
    SchemaNode, TestResult,
};

const DEFAULT_MAX_ROWS: usize = 1000;
const POOL_MAX_CONNECTIONS: u32 = 4;
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a request waits for a pooled session before failing with a clear
/// "connection busy" error instead of sqlx's silent 30 s default.
const POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
/// Upper bound on the out-of-band cancel (its own short-lived connection).
const CANCEL_TIMEOUT: Duration = Duration::from_secs(10);
/// Rows per decode task handed to the blocking pool.
const DECODE_CHUNK: usize = 1024;
/// A tail of at most this many cells is decoded inline (a spawn costs more).
const INLINE_DECODE_CELLS: usize = 16 * 1024;

/// PostgreSQL driver. Holds a per-`cache_key` pool cache + a completion snapshot
/// cache, exactly like [`crate::drivers::mysql::MysqlDriver`].
#[derive(Default)]
pub struct PostgresDriver {
    pools: ResourceCache<sqlx::PgPool>,
    completions: crate::complete::CompletionCache,
}

#[async_trait]
impl Driver for PostgresDriver {
    fn engine(&self) -> Engine {
        Engine::Postgres
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            engine: Engine::Postgres,
            sql: true,
            joins: true,
            // Pooled connections: every `run` acquires an independent session, so
            // there's no place to hold a BEGIN…COMMIT across calls (same as MySQL).
            transactions: false,
            multi_statement: true,
            // `pg_cancel_backend(pid)` on a separate pooled connection.
            cancel: true,
            // `EXPLAIN (FORMAT JSON)`.
            explain: true,
            default_port: 5432,
            // The top browse level is a schema within the connection's database;
            // shape mirrors MySQL's Database→Table→Column.
            schema_levels: vec!["Schema".into(), "Table".into(), "Column".into()],
            query_language: "sql".into(),
        }
    }

    async fn test(&self, cfg: &ResolvedConfig) -> Result<TestResult> {
        let started = Instant::now();
        let pool = match self.pool(cfg).await {
            Ok(pool) => pool,
            Err(e) => {
                return Ok(TestResult {
                    ok: false,
                    latency_ms: None,
                    message: e.to_string(),
                    server_version: None,
                });
            }
        };
        let version: String = match sqlx::query_scalar("SELECT version()")
            .fetch_one(&pool)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return Ok(TestResult {
                    ok: false,
                    latency_ms: None,
                    message: e.to_string(),
                    server_version: None,
                });
            }
        };
        Ok(TestResult {
            ok: true,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            message: "ok".into(),
            server_version: Some(version),
        })
    }

    async fn native_grants(
        &self,
        cfg: &ResolvedConfig,
    ) -> Result<Vec<crate::native_access::NativeGrant>> {
        use crate::native_access::{setup_error, NativeGrant};
        let pool = self.pool(cfg).await?;
        let elevated: bool = sqlx::query_scalar("SELECT rolsuper OR rolcreaterole OR rolcreatedb OR rolreplication OR rolbypassrls FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&pool).await.map_err(types::upstream)?;
        let memberships: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_auth_members WHERE member = (SELECT oid FROM pg_roles WHERE rolname = current_user)")
            .fetch_one(&pool).await.map_err(types::upstream)?;
        let database_authority: bool = sqlx::query_scalar("SELECT has_database_privilege(current_database(), 'CREATE') OR has_database_privilege(current_database(), 'TEMP') OR datdba = (SELECT oid FROM pg_roles WHERE rolname = current_user) FROM pg_database WHERE datname = current_database()")
            .fetch_one(&pool).await.map_err(types::upstream)?;
        if elevated || memberships != 0 || database_authority {
            return Err(setup_error(
                "native role has administrative, inherited-role, database-owner, CREATE, or TEMP authority",
            ));
        }
        let unsafe_routines: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') AND has_function_privilege(p.oid, 'EXECUTE')")
            .fetch_one(&pool).await.map_err(types::upstream)?;
        let unsafe_objects: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND c.relkind IN ('v','m','f') AND has_table_privilege(c.oid, 'SELECT,INSERT,UPDATE,DELETE')")
            .fetch_one(&pool).await.map_err(types::upstream)?;
        let triggers: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_trigger t WHERE NOT t.tgisinternal AND has_table_privilege(t.tgrelid, 'INSERT,UPDATE,DELETE')")
            .fetch_one(&pool).await.map_err(types::upstream)?;
        let cascades: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_constraint c WHERE c.contype='f' AND (c.confdeltype IN ('c','n','d') OR c.confupdtype IN ('c','n','d')) AND has_table_privilege(c.confrelid,'DELETE,UPDATE')")
            .fetch_one(&pool).await.map_err(types::upstream)?;
        if unsafe_routines != 0 || unsafe_objects != 0 || triggers != 0 || cascades != 0 {
            return Err(setup_error(
                "native routine, view, foreign-table, or trigger authority cannot prove the requested scope",
            ));
        }
        let schemas: Vec<(String, bool, bool)> = sqlx::query_as("SELECT nspname, has_schema_privilege(oid,'CREATE') OR nspowner = (SELECT oid FROM pg_roles WHERE rolname = current_user), has_schema_privilege(oid,'USAGE') FROM pg_namespace WHERE nspname !~ '^pg_' AND nspname <> 'information_schema'")
            .fetch_all(&pool).await.map_err(types::upstream)?;
        let mut grants = Vec::new();
        for (child, create, usage) in schemas {
            // Caller SQL cannot address PUBLIC catalogs or metadata functions:
            // the shared AST gate directs those reads through filtered metadata.
            if usage {
                grants.push(NativeGrant {
                    child: child.clone(),
                    operation: "db_browse",
                });
            }
            if create {
                grants.push(NativeGrant {
                    child: child.clone(),
                    operation: "db_schema",
                });
            }
            if !usage {
                continue;
            }
            let rights: (bool, bool, bool) = sqlx::query_as("SELECT COALESCE(bool_or(has_table_privilege(c.oid,'SELECT') OR has_any_column_privilege(c.oid,'SELECT')),false), COALESCE(bool_or(has_table_privilege(c.oid,'INSERT,UPDATE,DELETE,TRUNCATE') OR has_any_column_privilege(c.oid,'INSERT,UPDATE')),false), COALESCE(bool_or(c.relowner = (SELECT oid FROM pg_roles WHERE rolname = current_user)),false) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind IN ('r','p','v','m','f')")
                .bind(&child).fetch_one(&pool).await.map_err(types::upstream)?;
            if rights.0 {
                grants.push(NativeGrant {
                    child: child.clone(),
                    operation: "db_query",
                });
            }
            if rights.1 {
                grants.push(NativeGrant {
                    child: child.clone(),
                    operation: "db_data",
                });
            }
            if rights.2 {
                grants.push(NativeGrant {
                    child: child.clone(),
                    operation: "db_schema",
                });
            }
            let sequences: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND CASE WHEN c.relkind='S' THEN has_sequence_privilege(c.oid,'USAGE,UPDATE') ELSE false END)")
                .bind(&child).fetch_one(&pool).await.map_err(types::upstream)?;
            if sequences {
                grants.push(NativeGrant {
                    child,
                    operation: "db_data",
                });
            }
        }
        Ok(grants)
    }

    async fn schema_root(&self, cfg: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
        let pool = self.pool(cfg).await?;
        // The connection's database, browsed by schema: skip the system schemas
        // (`pg_*`, `information_schema`) and show `public` first.
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT nspname FROM pg_catalog.pg_namespace \
             WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' \
             ORDER BY (nspname <> 'public'), nspname",
        )
        .fetch_all(&pool)
        .await
        .map_err(types::upstream)?;
        Ok(rows
            .into_iter()
            .map(|(name,)| {
                SchemaNode::new(format!("db:{name}"), name, NodeKind::Schema).expandable()
            })
            .collect())
    }

    async fn search_objects(
        &self,
        cfg: &ResolvedConfig,
        req: &ObjectSearchReq,
    ) -> Result<ObjectSearchResult> {
        // One catalog query covers every schema IN THE CONNECTED DATABASE — a
        // Postgres connection cannot cross databases, so "all schemas" means
        // exactly that, and the UI says so rather than implying a server sweep.
        let pool = self.pool(cfg).await?;
        let limit = req.capped();
        let mut sql = String::from(
            "SELECT n.nspname, c.relname, c.relkind::text FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r','p','v','m','f') AND c.relname ILIKE $1 \
             AND n.nspname NOT IN ('pg_catalog','information_schema') \
             AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%'",
        );
        if !req.all_schemas() {
            sql.push_str(" AND n.nspname = $3");
        }
        sql.push_str(" ORDER BY n.nspname, c.relname LIMIT $2");
        let pattern = format!("%{}%", req.q);
        let q = sqlx::query_as::<_, (String, String, String)>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(&pattern)
            .bind((limit + 1) as i64);
        let q = if req.all_schemas() {
            q
        } else {
            q.bind(req.schema.clone().unwrap_or_default())
        };
        let rows = q.fetch_all(&pool).await.map_err(types::upstream)?;

        let truncated = rows.len() > limit;
        let mut schemas: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut hits = Vec::new();
        for (schema, name, relkind) in rows.into_iter().take(limit) {
            // r/p = table & partitioned table, f = foreign table; v/m = views.
            let (kind, seg, label) = match relkind.as_str() {
                "v" | "m" => (NodeKind::View, "view", "view"),
                _ => (NodeKind::Table, "table", "table"),
            };
            if !req.wants(label) {
                continue;
            }
            let path = NodePath::parse(&format!("db:{schema}"))
                .child(seg, &name)
                .to_id();
            schemas.insert(schema.clone());
            hits.push(ObjectHit {
                schema,
                name,
                kind,
                path,
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
        let schema = parent
            .get("db")
            .ok_or_else(|| types::invalid("schema_children: parent has no schema segment"))?
            .to_string();

        // db:<schema>/<obj>:<name> for a table/view/matview → its columns.
        if parent.get("table").is_some()
            || parent.get("view").is_some()
            || parent.get("matview").is_some()
        {
            let obj = parent
                .get("table")
                .or_else(|| parent.get("view"))
                .or_else(|| parent.get("matview"))
                .unwrap()
                .to_string();
            return self.columns_of(cfg, &schema, &obj, parent, filter).await;
        }

        // db:<schema>/folder:<f> → the objects in that folder.
        if let Some(folder) = parent.get("folder") {
            return self.objects_in_folder(cfg, &schema, folder, filter).await;
        }

        // db:<schema> → the object folders. Tables & Views always; Materialized
        // Views & Functions only when the schema actually has some (dimmed count).
        let mut folders = vec![
            SchemaNode::new(
                parent.child("folder", "tables").to_id(),
                "Tables",
                NodeKind::Folder,
            )
            .expandable(),
            SchemaNode::new(
                parent.child("folder", "views").to_id(),
                "Views",
                NodeKind::Folder,
            )
            .expandable(),
        ];
        if let Ok(pool) = self.pool(cfg).await {
            let matviews: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_catalog.pg_matviews WHERE schemaname = $1",
            )
            .bind(&schema)
            .fetch_one(&pool)
            .await
            .unwrap_or(0);
            if matviews > 0 {
                folders.push(
                    SchemaNode::new(
                        parent.child("folder", "matviews").to_id(),
                        "Materialized Views",
                        NodeKind::Folder,
                    )
                    .with_detail(matviews.to_string())
                    .expandable(),
                );
            }
            let functions: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname = $1",
            )
            .bind(&schema)
            .fetch_one(&pool)
            .await
            .unwrap_or(0);
            if functions > 0 {
                folders.push(
                    SchemaNode::new(
                        parent.child("folder", "functions").to_id(),
                        "Functions",
                        NodeKind::Folder,
                    )
                    .with_detail(functions.to_string())
                    .expandable(),
                );
            }
        }
        Ok(folders)
    }

    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        if path.get("function").is_some() {
            return self.function_detail(cfg, path).await;
        }
        let schema = path
            .get("db")
            .ok_or_else(|| types::invalid("object_detail: path has no schema segment"))?
            .to_string();
        let (name, kind, is_view) = if let Some(v) = path.get("view") {
            (v.to_string(), NodeKind::View, true)
        } else if let Some(m) = path.get("matview") {
            (m.to_string(), NodeKind::View, true)
        } else {
            let t = path
                .get("table")
                .ok_or_else(|| types::invalid("object_detail: path has no table/view segment"))?;
            (t.to_string(), NodeKind::Table, false)
        };

        let pool = self.pool(cfg).await?;

        // The independent catalog reads run concurrently (pool of 4): over an
        // SSH tunnel they used to cost one RTT each, back to back. The PK flag
        // is folded into the columns after the join; only the synthesized
        // table DDL depends on the columns, so it runs last.
        let cols_q = sqlx::query_as::<_, PgColumnRow>(
            "SELECT a.attname AS name, \
                    pg_catalog.format_type(a.atttypid, a.atttypmod) AS data_type, \
                    (NOT a.attnotnull) AS nullable, \
                    pg_get_expr(ad.adbin, ad.adrelid) AS col_default \
             FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_catalog.pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
             WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
        )
        .bind(&schema)
        .bind(&name)
        .fetch_all(&pool);
        // Primary-key columns (in key order).
        let pk_q = sqlx::query_scalar::<_, String>(
            "SELECT a.attname FROM pg_catalog.pg_constraint con \
             JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_catalog.pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = ANY(con.conkey) \
             WHERE con.contype = 'p' AND n.nspname = $1 AND c.relname = $2 \
             ORDER BY array_position(con.conkey, a.attnum)",
        )
        .bind(&schema)
        .bind(&name)
        .fetch_all(&pool);
        let view_ddl = async {
            if is_view {
                self.view_ddl(&pool, &schema, &name, path.get("matview").is_some())
                    .await
                    .ok()
            } else {
                None
            }
        };
        let (col_rows, primary_key, indexes, foreign_keys, view_ddl) = tokio::join!(
            cols_q,
            pk_q,
            self.indexes_of(&pool, &schema, &name),
            self.foreign_keys_of(&pool, &schema, &name),
            view_ddl,
        );
        let col_rows = col_rows.map_err(types::upstream)?;
        let primary_key = primary_key.unwrap_or_default();
        let indexes = indexes.unwrap_or_default();
        let foreign_keys = foreign_keys.unwrap_or_default();

        let columns: Vec<ColumnDef> = col_rows
            .into_iter()
            .map(|c| {
                let is_pk = primary_key.iter().any(|pk| pk == &c.name);
                ColumnDef {
                    name: c.name,
                    data_type: c.data_type,
                    nullable: c.nullable,
                    default: c.col_default,
                    key: is_pk.then(|| "PRI".to_string()),
                    extra: None,
                    comment: None,
                }
            })
            .collect();
        let ddl = if is_view {
            view_ddl
        } else {
            self.table_ddl(&pool, &schema, &name, &columns).await.ok()
        };

        let mut detail = ObjectDetail::new(name, kind);
        detail.columns = columns;
        detail.primary_key = primary_key;
        detail.indexes = indexes;
        detail.foreign_keys = foreign_keys;
        detail.ddl = ddl;
        Ok(detail)
    }

    /// Opt-in `reltuples` estimate (the planner's cheap row-count guess; -1 when
    /// the table was never analyzed). Only for base tables.
    async fn object_detail_with_opts(
        &self,
        cfg: &ResolvedConfig,
        path: &NodePath,
        approx_row_count: bool,
    ) -> Result<ObjectDetail> {
        let mut detail = self.object_detail(cfg, path).await?;
        if !approx_row_count || detail.kind != NodeKind::Table {
            return Ok(detail);
        }
        let (schema, table) = match (path.get("db"), path.get("table")) {
            (Some(s), Some(t)) => (s.to_string(), t.to_string()),
            _ => return Ok(detail),
        };
        let pool = self.pool(cfg).await?;
        let est: Option<f64> = sqlx::query_scalar(
            "SELECT c.reltuples FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relname = $2",
        )
        .bind(&schema)
        .bind(&table)
        .fetch_optional(&pool)
        .await
        .unwrap_or(None);
        if let Some(n) = est.filter(|&n| n >= 0.0) {
            detail.row_count = Some(n as i64);
        }
        Ok(detail)
    }

    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        self.run_tracked(cfg, req, &CancelToken::new()).await
    }

    /// Evict + close the cached pool for `cache_key` (connection close, or a
    /// config change superseded it); also drops the matching completion snapshot.
    fn detach(&self, cache_key: &str) -> Option<std::sync::Arc<dyn Driver>> {
        let captured = Self::default();
        for (key, value) in self.pools.take_where(|key| key == cache_key) {
            captured.pools.insert_ready(key, value);
        }
        self.completions.invalidate(cache_key);
        Some(std::sync::Arc::new(captured))
    }

    async fn close(&self, cache_key: &str) {
        let pool = self.pools.remove(cache_key);
        if let Some(pool) = pool {
            pool.close().await;
        }
        self.completions.invalidate(cache_key);
    }

    /// Drop cached pools nobody acquired for `idle`. Only the cache's handle is
    /// dropped (no explicit `close`): a query running longer than the window
    /// holds its own clone, so it finishes and the pool goes with the last
    /// clone; idle sessions are then closed by sqlx.
    async fn evict_idle(&self, idle: Duration) -> usize {
        // Same 5-minute tick: expired completion snapshots go too (DB2-07).
        self.completions.sweep();
        self.pools.take_idle(idle).len()
    }

    /// The whole schema's diagram in three concurrent catalog queries
    /// (relations, columns, PK/FK constraints) instead of one `object_detail`
    /// (2–6 sequential queries) per table.
    async fn schema_graph_bulk(
        &self,
        cfg: &ResolvedConfig,
        schema: &str,
        max_tables: usize,
    ) -> Result<Option<types::SchemaGraph>> {
        let pool = self.pool(cfg).await?;
        // Same objects and order as the lazy tree's folders: tables
        // (incl. partitioned), then views, then materialized views; by name.
        let rels_q = sqlx::query_as::<_, (String, String)>(
            "SELECT c.relname, c.relkind::text FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ('r','p','v','m') \
             ORDER BY CASE WHEN c.relkind IN ('r','p') THEN 0 WHEN c.relkind = 'v' THEN 1 ELSE 2 END, \
                      c.relname",
        )
        .bind(schema)
        .fetch_all(&pool);
        let cols_q = sqlx::query_as::<_, (String, String, String, bool)>(
            "SELECT c.relname, a.attname, pg_catalog.format_type(a.atttypid, a.atttypmod), \
                    (NOT a.attnotnull) \
             FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ('r','p','v','m') \
               AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY c.relname, a.attnum",
        )
        .bind(schema)
        .fetch_all(&pool);
        #[allow(clippy::type_complexity)]
        let cons_q = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(
            "SELECT c.relname, con.conname, con.contype::text, att.attname, \
                    fn.nspname, fc.relname, fatt.attname \
             FROM pg_catalog.pg_constraint con \
             JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) ON true \
             JOIN pg_catalog.pg_attribute att ON att.attrelid = con.conrelid AND att.attnum = k.attnum \
             LEFT JOIN pg_catalog.pg_class fc ON fc.oid = con.confrelid \
             LEFT JOIN pg_catalog.pg_namespace fn ON fn.oid = fc.relnamespace \
             LEFT JOIN LATERAL unnest(con.confkey) WITH ORDINALITY AS fk(attnum, ord) ON fk.ord = k.ord \
             LEFT JOIN pg_catalog.pg_attribute fatt ON fatt.attrelid = con.confrelid AND fatt.attnum = fk.attnum \
             WHERE con.contype IN ('p','f') AND n.nspname = $1 \
             ORDER BY c.relname, con.conname, k.ord",
        )
        .bind(schema)
        .fetch_all(&pool);
        let (rels, cols, cons) =
            tokio::try_join!(rels_q, cols_q, cons_q).map_err(types::upstream)?;

        let db = NodePath::parse(&format!("db:{schema}"));
        let rels = rels
            .into_iter()
            .map(|(name, relkind)| {
                let (seg, kind) = match relkind.as_str() {
                    "v" => ("view", NodeKind::View),
                    "m" => ("matview", NodeKind::View),
                    _ => ("table", NodeKind::Table),
                };
                BulkRel {
                    id: db.child(seg, &name).to_id(),
                    name,
                    kind,
                }
            })
            .collect();
        let cols = cols
            .into_iter()
            .map(|(table, name, data_type, nullable)| BulkCol {
                table,
                name,
                data_type,
                nullable,
                primary_key: false,
            })
            .collect();
        let mut pks = Vec::new();
        let mut fks = Vec::new();
        for (table, name, contype, column, ref_schema, ref_table, ref_column) in cons {
            if contype == "p" {
                pks.push((table, column));
            } else if let (Some(ref_table), Some(ref_column)) = (ref_table, ref_column) {
                fks.push(BulkFk {
                    table,
                    name,
                    column,
                    ref_schema,
                    ref_table,
                    ref_column,
                });
            }
        }
        Ok(Some(assemble_bulk_graph(
            schema, rels, cols, &pks, fks, max_tables,
        )))
    }

    async fn run_tracked(
        &self,
        cfg: &ResolvedConfig,
        req: &QueryRequest,
        token: &CancelToken,
    ) -> Result<QueryResult> {
        if cfg
            .params
            .get("__read_only_execution")
            .and_then(Value::as_bool)
            == Some(true)
        {
            return governed_read(&self.pool(cfg).await?, req, token).await;
        }
        let text = req.statement.trim();
        if text.is_empty() {
            return Err(types::invalid("empty statement"));
        }
        let max_rows = req.max_rows.unwrap_or(DEFAULT_MAX_ROWS);
        let pool = self.pool(cfg).await?;
        // Active-db node = a schema → `SET search_path`.
        let scope_schema = req.scope_database();
        let active_schema = scope_schema.as_deref();
        let timeout_ms = req.timeout_ms.filter(|&t| t > 0);

        let spans = split_statements(text, SqlDialect::Postgres);
        if spans.len() > 1 {
            return run_batch(&pool, &spans, max_rows, active_schema, timeout_ms, token).await;
        }
        let statement = spans.first().map(|s| s.text.as_str()).unwrap_or(text);
        let started = Instant::now();

        let (result, auto_limited) = if is_read_statement(statement) {
            let ri = types::inject_row_limit(statement, max_rows.saturating_add(1), req.offset);
            (
                run_read(
                    &pool,
                    &ri.sql,
                    max_rows,
                    ri.limited,
                    active_schema,
                    timeout_ms,
                    token,
                )
                .await,
                ri.limited.then_some(max_rows as u64),
            )
        } else {
            (
                run_write(&pool, statement, active_schema, token).await,
                None,
            )
        };
        let duration_ms = started.elapsed().as_millis() as u64;

        let mut result = result?;
        result.stats.duration_ms = duration_ms;
        result.stats.row_count = result.rows.len();
        result.auto_limited = auto_limited;
        Ok(result)
    }

    /// Cancel the running query on its backend: `pg_cancel_backend(pid)` on a
    /// separate pooled connection. A finished/unknown pid returns `false` (a
    /// successful no-op), never an error.
    async fn cancel(&self, cfg: &ResolvedConfig, handle: &QueryHandle) -> Result<()> {
        let QueryHandle::PostgresBackendPid(pid) = handle else {
            return Ok(());
        };
        let Some(pool) = self.pools.get_ready(&cfg.cache_key()) else {
            return Ok(());
        };
        let pid = *pid;
        // Never wait in the pool's queue: Stop matters most exactly when every
        // pooled session is busy. Use an idle one if there is one right now,
        // else a one-off connection built from the pool's own options.
        let cancel = async {
            if let Some(mut conn) = pool.try_acquire() {
                let _ = sqlx::query("SELECT pg_cancel_backend($1)")
                    .bind(pid)
                    .execute(&mut *conn)
                    .await;
                return;
            }
            let opts = pool.connect_options();
            if let Ok(mut conn) = sqlx::PgConnection::connect_with(&opts).await {
                let _ = sqlx::query("SELECT pg_cancel_backend($1)")
                    .bind(pid)
                    .execute(&mut conn)
                    .await;
                let _ = conn.close().await;
            }
        };
        let _ = tokio::time::timeout(CANCEL_TIMEOUT, cancel).await;
        Ok(())
    }

    /// Structured query plan via `EXPLAIN (FORMAT JSON)`. Postgres returns the
    /// plan as a single `json` cell (a one-element array). The statement is
    /// EXPLAIN-wrapped — never executed raw (no `ANALYZE`, so no side effects).
    async fn query_plan(
        &self,
        cfg: &ResolvedConfig,
        statement: &str,
        node: Option<&str>,
    ) -> Result<DbQueryPlan> {
        let stmt = statement.trim().trim_end_matches(';');
        if stmt.is_empty() {
            return Err(types::invalid("empty statement"));
        }
        let pool = self.pool(cfg).await?;
        let mut conn = acquire(&pool).await?;
        let mut conn = conn
            .begin_with("BEGIN READ ONLY")
            .await
            .map_err(types::upstream)?;
        // Always explicit (inside the transaction, so it reverts on rollback):
        // without a schema, the session default — not a pooled leftover.
        let schema = node.map(str::trim).filter(|s| !s.is_empty());
        (&mut *conn)
            .execute(sqlx::raw_sql(sqlx::AssertSqlSafe(search_path_sql(schema))))
            .await
            .map_err(types::upstream)?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN (FORMAT JSON) {stmt}")))
            .fetch_one(&mut *conn)
            .await
            .map_err(types::upstream)?;
        // The plan column decodes straight to a serde_json::Value (json type).
        let raw: Value = row.try_get(0).map_err(types::upstream)?;
        let root = crate::plan::from_pg_json(&raw);
        Ok(DbQueryPlan {
            engine: "postgres".into(),
            root,
            raw,
        })
    }

    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        // For Postgres the completion scope is the active *schema* (public by
        // default), not a database.
        let scope = ctx
            .database
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "public".to_string());
        let snap = self.completion_snapshot(cfg, &scope).await;
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

    /// Streaming export via sqlx's row cursor (`.fetch`) — one row at a time,
    /// bounded daemon memory. Only row-returning statements are exportable.
    async fn export_to_writer(
        &self,
        cfg: &ResolvedConfig,
        statement: &str,
        node: Option<&str>,
        format: ExportFormat,
        max_rows: Option<usize>,
        w: Box<dyn std::io::Write + Send>,
    ) -> Result<ExportCounts> {
        use futures_util::TryStreamExt as _;

        let statement = statement.trim();
        if statement.is_empty() {
            return Err(types::invalid("empty statement"));
        }
        if !is_read_statement(statement) {
            return Err(types::invalid(
                "export supports row-returning statements only",
            ));
        }

        let pool = self.pool(cfg).await?;
        let mut conn = acquire(&pool).await?;
        let mut conn = conn
            .begin_with("BEGIN READ ONLY")
            .await
            .map_err(types::upstream)?;
        // Always explicit (inside the transaction, so it reverts on rollback):
        // without a schema, the session default — not a pooled leftover.
        let schema = node.map(str::trim).filter(|s| !s.is_empty());
        (&mut *conn)
            .execute(sqlx::raw_sql(sqlx::AssertSqlSafe(search_path_sql(schema))))
            .await
            .map_err(types::upstream)?;

        let mut rows = sqlx::query(sqlx::AssertSqlSafe(statement)).fetch(&mut *conn);
        let mut sink = ExportSink::new(w, format);
        let mut header_written = false;
        let mut decoders: Vec<PgCell> = Vec::new();
        let mut n: usize = 0;
        while let Some(row) = rows.try_next().await.map_err(types::upstream)? {
            if let Some(cap) = max_rows {
                if n >= cap {
                    break;
                }
            }
            if !header_written {
                let columns: Vec<Column> = row
                    .columns()
                    .iter()
                    .map(|c| Column::typed(c.name(), c.type_info().name()))
                    .collect();
                sink.write_header(&columns)
                    .map_err(|e| otto_core::Error::Internal(format!("write export header: {e}")))?;
                header_written = true;
            }
            if decoders.is_empty() {
                decoders = row
                    .columns()
                    .iter()
                    .map(|c| pg_cell_decoder(c.type_info().name()))
                    .collect();
            }
            let cells: Vec<Value> = decoders
                .iter()
                .enumerate()
                .map(|(i, dec)| pg_cell(&row, i, *dec))
                .collect();
            sink.write_row(&cells)
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

impl PostgresDriver {
    /// Columns of a table/view (lazy expansion), with an optional case-insensitive
    /// substring filter (`ILIKE`).
    async fn columns_of(
        &self,
        cfg: &ResolvedConfig,
        schema: &str,
        table: &str,
        parent: &NodePath,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let pool = self.pool(cfg).await?;
        let rows: Vec<(String, String)> = match filter {
            Some(f) if !f.is_empty() => {
                sqlx::query_as(
                    "SELECT column_name, data_type FROM information_schema.columns \
                 WHERE table_schema = $1 AND table_name = $2 AND column_name ILIKE $3 \
                 ORDER BY ordinal_position",
                )
                .bind(schema)
                .bind(table)
                .bind(format!("%{f}%"))
                .fetch_all(&pool)
                .await
            }
            _ => {
                sqlx::query_as(
                    "SELECT column_name, data_type FROM information_schema.columns \
                 WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
                )
                .bind(schema)
                .bind(table)
                .fetch_all(&pool)
                .await
            }
        }
        .map_err(types::upstream)?;
        Ok(rows
            .into_iter()
            .map(|(name, ty)| {
                SchemaNode::new(
                    parent.child("column", &name).to_id(),
                    name,
                    NodeKind::Column,
                )
                .with_detail(ty)
            })
            .collect())
    }

    /// Objects in a schema folder (tables / views / matviews / functions), with an
    /// optional case-insensitive substring filter.
    async fn objects_in_folder(
        &self,
        cfg: &ResolvedConfig,
        schema: &str,
        folder: &str,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let pool = self.pool(cfg).await?;
        let schema_path = NodePath::parse(&format!("db:{schema}"));
        let pat = filter.filter(|f| !f.is_empty()).map(|f| format!("%{f}%"));

        // Functions live in pg_proc; the leaves aren't expandable (no children).
        if folder == "functions" {
            let names: Vec<(String,)> = match &pat {
                Some(p) => {
                    sqlx::query_as(
                        "SELECT p.proname FROM pg_catalog.pg_proc p \
                     JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname = $1 AND p.proname ILIKE $2 ORDER BY p.proname",
                    )
                    .bind(schema)
                    .bind(p)
                    .fetch_all(&pool)
                    .await
                }
                None => {
                    sqlx::query_as(
                        "SELECT p.proname FROM pg_catalog.pg_proc p \
                     JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname = $1 ORDER BY p.proname",
                    )
                    .bind(schema)
                    .fetch_all(&pool)
                    .await
                }
            }
            .map_err(types::upstream)?;
            return Ok(names
                .into_iter()
                .map(|(name,)| {
                    SchemaNode::new(
                        schema_path.child("function", &name).to_id(),
                        name,
                        NodeKind::Function,
                    )
                })
                .collect());
        }

        // Tables / views / matviews from pg_class by relkind.
        let (relkinds, kind, seg): (&[&str], NodeKind, &str) = match folder {
            "tables" => (&["r", "p"], NodeKind::Table, "table"),
            "views" => (&["v"], NodeKind::View, "view"),
            "matviews" => (&["m"], NodeKind::View, "matview"),
            other => return Err(types::invalid(format!("unknown folder: {other}"))),
        };
        // relkind is a single-char catalog value; build the IN list from constants.
        let in_list = relkinds
            .iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(",");
        let base = format!(
            "SELECT c.relname FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ({in_list})"
        );
        let rows: Vec<(String,)> = match &pat {
            Some(p) => {
                sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "{base} AND c.relname ILIKE $2 ORDER BY c.relname"
                )))
                .bind(schema)
                .bind(p)
                .fetch_all(&pool)
                .await
            }
            None => {
                sqlx::query_as(sqlx::AssertSqlSafe(format!("{base} ORDER BY c.relname")))
                    .bind(schema)
                    .fetch_all(&pool)
                    .await
            }
        }
        .map_err(types::upstream)?;
        Ok(rows
            .into_iter()
            .map(|(name,)| {
                SchemaNode::new(schema_path.child(seg, &name).to_id(), name, kind).expandable()
            })
            .collect())
    }

    async fn indexes_of(
        &self,
        pool: &sqlx::PgPool,
        schema: &str,
        table: &str,
    ) -> Result<Vec<IndexDef>> {
        let rows: Vec<PgIndexRow> = sqlx::query_as(
            "SELECT ic.relname AS name, ix.indisunique AS is_unique, am.amname AS method, \
                    pg_catalog.pg_get_indexdef(ix.indexrelid) AS def, \
                    a.attname AS col, k.ord::int AS ord \
             FROM pg_catalog.pg_index ix \
             JOIN pg_catalog.pg_class ic ON ic.oid = ix.indexrelid \
             JOIN pg_catalog.pg_class tc ON tc.oid = ix.indrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace \
             JOIN pg_catalog.pg_am am ON am.oid = ic.relam \
             LEFT JOIN LATERAL unnest(string_to_array(ix.indkey::text, ' ')::int[]) \
                       WITH ORDINALITY AS k(attnum, ord) ON true \
             LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid = tc.oid AND a.attnum = k.attnum \
             WHERE n.nspname = $1 AND tc.relname = $2 \
             ORDER BY ic.relname, k.ord",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await
        .map_err(types::upstream)?;

        let mut order: Vec<String> = Vec::new();
        let mut by_name: HashMap<String, IndexDef> = HashMap::new();
        for r in rows {
            let entry = by_name.entry(r.name.clone()).or_insert_with(|| {
                order.push(r.name.clone());
                IndexDef {
                    name: r.name.clone(),
                    columns: Vec::new(),
                    unique: r.is_unique,
                    method: Some(r.method.clone()),
                    definition: r.def.clone().map(Value::String),
                }
            });
            // Expression-index members have no attname (attnum 0) → skipped.
            if let Some(col) = r.col {
                entry.columns.push(col);
            }
        }
        Ok(order
            .into_iter()
            .filter_map(|n| by_name.remove(&n))
            .collect())
    }

    async fn foreign_keys_of(
        &self,
        pool: &sqlx::PgPool,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        let rows: Vec<PgFkRow> = sqlx::query_as(
            "SELECT con.conname AS name, att.attname AS col, \
                    fn.nspname AS ref_schema, fc.relname AS ref_table, fatt.attname AS ref_col, \
                    k.ord::int AS ord \
             FROM pg_catalog.pg_constraint con \
             JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_catalog.pg_class fc ON fc.oid = con.confrelid \
             JOIN pg_catalog.pg_namespace fn ON fn.oid = fc.relnamespace \
             JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) ON true \
             JOIN pg_catalog.pg_attribute att ON att.attrelid = con.conrelid AND att.attnum = k.attnum \
             JOIN LATERAL unnest(con.confkey) WITH ORDINALITY AS fk(attnum, ord) ON fk.ord = k.ord \
             JOIN pg_catalog.pg_attribute fatt ON fatt.attrelid = con.confrelid AND fatt.attnum = fk.attnum \
             WHERE con.contype = 'f' AND n.nspname = $1 AND c.relname = $2 \
             ORDER BY con.conname, k.ord",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await
        .map_err(types::upstream)?;

        let mut order: Vec<String> = Vec::new();
        let mut by_name: HashMap<String, ForeignKey> = HashMap::new();
        for r in rows {
            let entry = by_name.entry(r.name.clone()).or_insert_with(|| {
                order.push(r.name.clone());
                ForeignKey {
                    name: r.name.clone(),
                    columns: Vec::new(),
                    ref_table: r.ref_table.clone(),
                    ref_columns: Vec::new(),
                    ref_schema: Some(r.ref_schema.clone()),
                }
            });
            entry.columns.push(r.col);
            entry.ref_columns.push(r.ref_col);
        }
        Ok(order
            .into_iter()
            .filter_map(|n| by_name.remove(&n))
            .collect())
    }

    /// Synthesize a `CREATE TABLE` from the catalog: columns + every constraint
    /// (`pg_get_constraintdef`) + secondary index defs (`pg_get_indexdef`).
    async fn table_ddl(
        &self,
        pool: &sqlx::PgPool,
        schema: &str,
        table: &str,
        columns: &[ColumnDef],
    ) -> Result<String> {
        let mut out = format!(
            "CREATE TABLE {}.{} (\n",
            quote_ident(schema),
            quote_ident(table)
        );
        let col_lines: Vec<String> = columns
            .iter()
            .map(|c| {
                let mut line = format!("    {} {}", quote_ident(&c.name), c.data_type);
                if !c.nullable {
                    line.push_str(" NOT NULL");
                }
                if let Some(d) = &c.default {
                    line.push_str(&format!(" DEFAULT {d}"));
                }
                line
            })
            .collect();
        out.push_str(&col_lines.join(",\n"));

        // Table constraints (PK/FK/unique/check), verbatim from the catalog.
        let cons: Vec<(String, String)> = sqlx::query_as(
            "SELECT conname, pg_get_constraintdef(oid) \
             FROM pg_catalog.pg_constraint \
             WHERE conrelid = (quote_ident($1) || '.' || quote_ident($2))::regclass \
             ORDER BY contype DESC, conname",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for (name, def) in &cons {
            out.push_str(&format!(",\n    CONSTRAINT {} {}", quote_ident(name), def));
        }
        out.push_str("\n);");

        // Secondary indexes not backing a constraint.
        let idx: Vec<(String,)> = sqlx::query_as(
            "SELECT pg_get_indexdef(ix.indexrelid) \
             FROM pg_catalog.pg_index ix \
             JOIN pg_catalog.pg_class tc ON tc.oid = ix.indrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace \
             WHERE n.nspname = $1 AND tc.relname = $2 \
               AND NOT ix.indisprimary \
               AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_constraint con \
                               WHERE con.conindid = ix.indexrelid)",
        )
        .bind(schema)
        .bind(table)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for (def,) in idx {
            out.push_str(&format!("\n{def};"));
        }
        Ok(out)
    }

    /// `CREATE [MATERIALIZED] VIEW … AS <pg_get_viewdef>`.
    async fn view_ddl(
        &self,
        pool: &sqlx::PgPool,
        schema: &str,
        name: &str,
        materialized: bool,
    ) -> Result<String> {
        let def: String = sqlx::query_scalar(
            "SELECT pg_get_viewdef((quote_ident($1) || '.' || quote_ident($2))::regclass, true)",
        )
        .bind(schema)
        .bind(name)
        .fetch_one(pool)
        .await
        .map_err(types::upstream)?;
        let kw = if materialized {
            "MATERIALIZED VIEW"
        } else {
            "VIEW"
        };
        Ok(format!(
            "CREATE {kw} {}.{} AS\n{}",
            quote_ident(schema),
            quote_ident(name),
            def
        ))
    }

    /// Structure of a function: its arguments (rendered as the object's "columns",
    /// plus a return-type row) and the full `pg_get_functiondef` DDL.
    async fn function_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        let schema = path
            .get("db")
            .ok_or_else(|| types::invalid("function_detail: path has no schema segment"))?
            .to_string();
        let name = path
            .get("function")
            .ok_or_else(|| types::invalid("function_detail: path has no function segment"))?
            .to_string();
        let pool = self.pool(cfg).await?;

        // First overload by oid: arguments + result type (always safe).
        let meta: Option<(String, String)> = sqlx::query_as(
            "SELECT pg_get_function_arguments(p.oid), pg_get_function_result(p.oid) \
             FROM pg_catalog.pg_proc p \
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname = $1 AND p.proname = $2 ORDER BY p.oid LIMIT 1",
        )
        .bind(&schema)
        .bind(&name)
        .fetch_optional(&pool)
        .await
        .map_err(types::upstream)?;

        let mut columns: Vec<ColumnDef> = Vec::new();
        if let Some((args, result)) = meta {
            for arg in args.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                // "name type" | "mode name type" | just "type".
                let (label, ty) = arg.split_once(char::is_whitespace).unwrap_or(("", arg));
                columns.push(ColumnDef {
                    name: if label.is_empty() {
                        "(arg)".into()
                    } else {
                        label.to_string()
                    },
                    data_type: ty.trim().to_string(),
                    nullable: true,
                    default: None,
                    key: None,
                    extra: Some("IN".into()),
                    comment: None,
                });
            }
            columns.push(ColumnDef {
                name: "(returns)".into(),
                data_type: result,
                nullable: true,
                default: None,
                key: None,
                extra: Some("RETURNS".into()),
                comment: None,
            });
        }

        // pg_get_functiondef errors for aggregate/window funcs — best-effort.
        let ddl: Option<String> = sqlx::query_scalar(
            "SELECT pg_get_functiondef(p.oid) FROM pg_catalog.pg_proc p \
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname = $1 AND p.proname = $2 ORDER BY p.oid LIMIT 1",
        )
        .bind(&schema)
        .bind(&name)
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();

        let mut detail = ObjectDetail::new(name, NodeKind::Function);
        detail.columns = columns;
        detail.ddl = ddl;
        Ok(detail)
    }

    async fn completion_snapshot(
        &self,
        cfg: &ResolvedConfig,
        schema: &str,
    ) -> std::sync::Arc<crate::complete::SchemaSnapshot> {
        // Single-flight + negatively cached: a failed build is remembered
        // briefly instead of re-introspecting on every completion request.
        self.completions
            .snapshot_or_build(&cfg.cache_key(), schema, || {
                self.build_completion_snapshot(cfg, schema)
            })
            .await
    }

    /// Introspect the catalog into a [`SchemaSnapshot`]: the database's schemas
    /// (the "databases" list), the scoped schema's tables/views + index-ranked
    /// columns, and its functions.
    async fn build_completion_snapshot(
        &self,
        cfg: &ResolvedConfig,
        schema: &str,
    ) -> Option<crate::complete::SchemaSnapshot> {
        use crate::complete::{FieldSnap, ObjKind, ObjectSnap, Rank, RoutineSnap, SchemaSnapshot};

        let pool = self.pool(cfg).await.ok()?;
        let databases: Vec<String> = sqlx::query_as::<_, (String,)>(
            "SELECT nspname FROM pg_catalog.pg_namespace \
             WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' ORDER BY nspname",
        )
        .fetch_all(&pool)
        .await
        .ok()?
        .into_iter()
        .map(|(d,)| d)
        .collect();

        if schema.is_empty() {
            return Some(SchemaSnapshot {
                databases,
                objects: Vec::new(),
                routines: Vec::new(),
            });
        }

        // The four catalog reads are independent: one concurrent wave instead
        // of four back-to-back round trips. Straight off `pg_class` /
        // `pg_attribute` (like the bulk schema graph), NOT the
        // `information_schema` views: those evaluate per-row privilege checks
        // over the whole catalog and cost seconds on 10k+ relations (DB2-07).
        // Matviews now complete too (the view skipped them); types come from
        // `format_type` (`character varying(255)`, `text[]`).
        let tables_q = sqlx::query_as::<_, (String, String)>(
            "SELECT c.relname, \
                    CASE WHEN c.relkind IN ('v','m') THEN 'VIEW' ELSE 'BASE TABLE' END \
             FROM pg_catalog.pg_class c \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ('r','p','f','v','m') \
             ORDER BY c.relname",
        )
        .bind(schema)
        .fetch_all(&pool);
        let cols_q = sqlx::query_as::<_, (String, String, String)>(
            "SELECT c.relname, a.attname, pg_catalog.format_type(a.atttypid, a.atttypmod) \
             FROM pg_catalog.pg_attribute a \
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ('r','p','f','v','m') \
               AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY c.relname, a.attnum",
        )
        .bind(schema)
        .fetch_all(&pool);
        let routines_q = sqlx::query_as::<_, (String, String)>(
            "SELECT p.proname, p.prokind::text FROM pg_catalog.pg_proc p \
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
             WHERE n.nspname = $1 ORDER BY p.proname",
        )
        .bind(schema)
        .fetch_all(&pool);
        // (table, column) → strongest index rank.
        let stats_q = sqlx::query_as::<_, (String, String, bool, bool)>(
            "SELECT tc.relname, a.attname, ix.indisprimary, ix.indisunique \
             FROM pg_catalog.pg_index ix \
             JOIN pg_catalog.pg_class tc ON tc.oid = ix.indrelid \
             JOIN pg_catalog.pg_namespace n ON n.oid = tc.relnamespace \
             JOIN pg_catalog.pg_attribute a ON a.attrelid = tc.oid AND a.attnum = ANY(ix.indkey) \
             WHERE n.nspname = $1",
        )
        .bind(schema)
        .fetch_all(&pool);
        let (tables, cols, routines, stats) = tokio::join!(tables_q, cols_q, routines_q, stats_q);
        let tables = tables.unwrap_or_default();
        let cols = cols.unwrap_or_default();
        let stats = stats.unwrap_or_default();
        let routines: Vec<RoutineSnap> = routines
            .unwrap_or_default()
            .into_iter()
            .map(|(name, kind)| RoutineSnap {
                name,
                // 'p' = procedure; everything else (f/a/w) is called like a function.
                is_function: kind != "p",
            })
            .collect();

        let mut rank: HashMap<(String, String), Rank> = HashMap::new();
        for (t, c, is_primary, is_unique) in stats {
            let r = if is_primary {
                Rank::Pk
            } else if is_unique {
                Rank::Unique
            } else {
                Rank::Index
            };
            let key = (t.to_ascii_lowercase(), c.to_ascii_lowercase());
            let entry = rank.entry(key).or_insert(r);
            if crate::complete::rank_strength(r) > crate::complete::rank_strength(*entry) {
                *entry = r;
            }
        }

        let mut by_table: HashMap<String, Vec<FieldSnap>> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for (t, c, ty) in cols {
            let key = (t.to_ascii_lowercase(), c.to_ascii_lowercase());
            let r = rank.get(&key).copied().unwrap_or(Rank::Plain);
            by_table.entry(t.clone()).or_insert_with(|| {
                order.push(t.clone());
                Vec::new()
            });
            by_table
                .get_mut(&t)
                .unwrap()
                .push(FieldSnap::new(c, Some(ty), r));
        }

        let mut objects: Vec<ObjectSnap> = Vec::new();
        for (name, ttype) in tables {
            let kind = if ttype.eq_ignore_ascii_case("VIEW") {
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
        for name in order {
            if let Some(fields) = by_table.remove(&name) {
                objects.push(ObjectSnap {
                    name,
                    kind: ObjKind::Table,
                    fields,
                    fields_ready: true,
                });
            }
        }

        Some(SchemaSnapshot {
            databases,
            objects,
            routines,
        })
    }

    async fn pool(&self, cfg: &ResolvedConfig) -> Result<sqlx::PgPool> {
        self.pools
            .get_or_try_init(
                cfg.cache_key(),
                cfg.lifecycle.as_ref(),
                |_| true,
                build_pool(cfg),
            )
            .await
    }
}

/// Build a fresh `PgPool` from a resolved config. Never called directly by the
/// driver methods — they go through [`PostgresDriver::pool`] for caching.
async fn build_pool(cfg: &ResolvedConfig) -> Result<sqlx::PgPool> {
    let mut opts = PgConnectOptions::new().host(&cfg.host).port(cfg.port);
    if let Some(user) = cfg.user.as_deref().filter(|s| !s.is_empty()) {
        opts = opts.username(user);
    }
    if let Some(password) = cfg.password.as_deref() {
        opts = opts.password(password);
    }
    if let Some(db) = cfg.database.as_deref().filter(|s| !s.is_empty()) {
        opts = opts.database(db);
    }

    // TLS: map TlsMode → PgSslMode + inline CA/client cert via temp files.
    // When verification is on, verify the HOSTNAME too (VerifyFull) — VerifyCa
    // alone accepts any CA-signed cert for a different host (MITM). Exception:
    // through an SSH tunnel the TCP endpoint is 127.0.0.1 (the cert's name can
    // never match), and an explicit `tls.server_name` override has no sqlx
    // surface to honour — both keep chain-only verification (VerifyCa).
    let hostname_checkable = cfg.param_str("__tunnel_host").is_none()
        && cfg
            .tls
            .server_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .is_none();
    let ssl_mode = match cfg.tls.mode {
        types::TlsMode::Disabled => PgSslMode::Disable,
        types::TlsMode::Preferred => PgSslMode::Prefer,
        types::TlsMode::Required => {
            if cfg.tls.verify && hostname_checkable {
                PgSslMode::VerifyFull
            } else if cfg.tls.verify {
                PgSslMode::VerifyCa
            } else {
                PgSslMode::Require
            }
        }
    };
    opts = opts.ssl_mode(ssl_mode);
    if cfg.tls.enabled() {
        let files = TlsFiles::materialize(&cfg.tls)?;
        if let Some(ca) = files.ca {
            opts = opts.ssl_root_cert(ca);
        }
        if let Some(cert) = files.client_cert {
            opts = opts.ssl_client_cert(cert);
        }
        if let Some(key) = files.client_key {
            opts = opts.ssl_client_key(key);
        }
    }

    // Session timezone (default: leave the server's) applied on each new connection.
    let tz = cfg.param_str("timezone").filter(|s| !s.is_empty());
    PgPoolOptions::new()
        .max_connections(POOL_MAX_CONNECTIONS)
        .idle_timeout(POOL_IDLE_TIMEOUT)
        .acquire_timeout(POOL_ACQUIRE_TIMEOUT)
        .after_connect(move |conn, _meta| {
            let tz = tz.clone();
            Box::pin(async move {
                if let Some(tz) = tz {
                    let stmt = format!("SET TIME ZONE '{}'", tz.replace('\'', "''"));
                    let _ = (&mut *conn)
                        .execute(sqlx::raw_sql(sqlx::AssertSqlSafe(stmt.as_str())))
                        .await;
                }
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .map_err(types::upstream)
}

// --- Bulk schema graph (shared by the SQL drivers) ---------------------------

/// One diagrammable relation from a bulk catalog read, in tree order.
pub(crate) struct BulkRel {
    /// The lazy tree's node id for this object (`db:<s>/table:<t>` …).
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
}

/// One column row from a bulk catalog read (`primary_key` may be pre-set by
/// engines that report it per column, e.g. MySQL's `column_key = 'PRI'`).
pub(crate) struct BulkCol {
    pub table: String,
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
}

/// One FK column pair from a bulk catalog read, rows ordered by
/// (table, constraint, key position).
pub(crate) struct BulkFk {
    pub table: String,
    pub name: String,
    pub column: String,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_column: String,
}

/// Assemble a [`types::SchemaGraph`] from set-based catalog rows — the same
/// shape the per-object walk builds: the first `max_tables` relations (in the
/// given order) with PK/FK column flags, the FK edges out of them (a missing
/// ref schema defaults to this schema), `truncated` when relations were cut.
pub(crate) fn assemble_bulk_graph(
    schema: &str,
    rels: Vec<BulkRel>,
    cols: Vec<BulkCol>,
    pks: &[(String, String)],
    fks: Vec<BulkFk>,
    max_tables: usize,
) -> types::SchemaGraph {
    use std::collections::HashSet;
    let truncated = rels.len() > max_tables;
    let kept: Vec<BulkRel> = rels.into_iter().take(max_tables).collect();
    let kept_names: HashSet<&str> = kept.iter().map(|r| r.name.as_str()).collect();
    let pk: HashSet<(&str, &str)> = pks.iter().map(|(t, c)| (t.as_str(), c.as_str())).collect();
    let fk_cols: HashSet<(&str, &str)> = fks
        .iter()
        .map(|f| (f.table.as_str(), f.column.as_str()))
        .collect();
    let mut by_table: HashMap<&str, Vec<types::GraphColumn>> = HashMap::new();
    for c in &cols {
        if !kept_names.contains(c.table.as_str()) {
            continue;
        }
        by_table
            .entry(c.table.as_str())
            .or_default()
            .push(types::GraphColumn {
                name: c.name.clone(),
                data_type: c.data_type.clone(),
                nullable: c.nullable,
                primary_key: c.primary_key || pk.contains(&(c.table.as_str(), c.name.as_str())),
                foreign_key: fk_cols.contains(&(c.table.as_str(), c.name.as_str())),
            });
    }
    // Group FK rows into edges (rows arrive ordered by table, constraint, pos).
    let mut edges: Vec<types::GraphEdge> = Vec::new();
    for f in &fks {
        if !kept_names.contains(f.table.as_str()) {
            continue;
        }
        match edges.last_mut() {
            Some(e) if e.from_table == f.table && e.name == f.name => {
                e.from_columns.push(f.column.clone());
                e.to_columns.push(f.ref_column.clone());
            }
            _ => edges.push(types::GraphEdge {
                name: f.name.clone(),
                from_table: f.table.clone(),
                from_columns: vec![f.column.clone()],
                to_schema: f.ref_schema.clone().unwrap_or_else(|| schema.to_string()),
                to_table: f.ref_table.clone(),
                to_columns: vec![f.ref_column.clone()],
            }),
        }
    }
    let tables = kept
        .into_iter()
        .map(|r| types::GraphTable {
            columns: by_table.remove(r.name.as_str()).unwrap_or_default(),
            id: r.id,
            schema: schema.to_string(),
            name: r.name,
            kind: r.kind,
        })
        .collect();
    types::SchemaGraph {
        schema: schema.to_string(),
        tables,
        edges,
        relationships: true,
        truncated,
    }
}

// --- Query execution --------------------------------------------------------

/// First-keyword detection of row-returning statements.
fn is_read_statement(statement: &str) -> bool {
    matches!(
        first_keyword(statement).as_str(),
        "SELECT" | "WITH" | "SHOW" | "EXPLAIN" | "TABLE" | "VALUES"
    )
}

/// First SQL keyword (uppercased), skipping whitespace and `--` / `/* */` comments.
fn first_keyword(statement: &str) -> String {
    let mut s = statement.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
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
    s.chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Double-quote a Postgres identifier (doubling embedded quotes). Only used for
/// schema-tree-sourced identifiers (DDL synthesis / `SET search_path`).
fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// `SET search_path TO "<schema>"` — the Postgres analogue of MySQL's `USE`.
fn set_search_path_sql(schema: &str) -> String {
    format!("SET search_path TO {}", quote_ident(schema))
}

/// Capture the backend PID (`pg_backend_pid()`) so a concurrent cancel can
/// `pg_cancel_backend(pid)` this exact connection. Best-effort. Only the
/// governed (read-only transaction) path still pays this as its own round
/// trip; ad-hoc runs fold it into [`session_setup_sql`].
async fn capture_backend_pid(conn: &mut sqlx::PgConnection, token: &CancelToken) {
    if let Ok(pid) = sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
    {
        token.set(QueryHandle::PostgresBackendPid(pid));
    }
}

/// Acquire a pooled session, mapping sqlx's pool timeout to a clear message.
async fn acquire(pool: &sqlx::PgPool) -> Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    pool.acquire().await.map_err(acquire_error)
}

/// `PoolTimedOut` means every pooled session stayed busy for
/// [`POOL_ACQUIRE_TIMEOUT`]: say so instead of sqlx's generic text.
fn acquire_error(e: sqlx::Error) -> otto_core::Error {
    match e {
        sqlx::Error::PoolTimedOut => types::upstream(format!(
            "connection busy: all {POOL_MAX_CONNECTIONS} sessions to this server are in use \
             (waited {}s); stop a running query or retry",
            POOL_ACQUIRE_TIMEOUT.as_secs()
        )),
        other => types::upstream(other),
    }
}

/// The `search_path` statement for a request: the selected schema, else the
/// session DEFAULT (`RESET` — the role/database setting), never whatever an
/// earlier request on the same pooled session left behind.
fn search_path_sql(schema: Option<&str>) -> String {
    schema
        .map(set_search_path_sql)
        .unwrap_or_else(|| "RESET search_path".to_string())
}

/// ONE simple-protocol round trip that sets a request's whole session context
/// — its `search_path` (see [`search_path_sql`]) and statement timeout (`RESET`
/// to the role/database default when the request sets none, so neither `0`
/// nor a timeout left by a cancelled run applies) — AND returns the backend
/// pid for cancel. It used to be two trips (setup, then `pg_backend_pid()`)
/// plus a trailing `RESET` the response waited on.
fn session_setup_sql(schema: Option<&str>, timeout_ms: Option<u64>) -> String {
    let timeout = timeout_ms
        .map(|ms| format!("SET statement_timeout = {ms}"))
        .unwrap_or_else(|| "RESET statement_timeout".to_string());
    format!(
        "{}; {timeout}; SELECT pg_backend_pid()",
        search_path_sql(schema)
    )
}

/// The `BEGIN` of a governed read: opens the read-only transaction and sets
/// its scope in the same simple-protocol round trip (both revert with the
/// transaction), instead of three separate trips.
fn governed_begin_sql(schema: Option<&str>, timeout_ms: Option<u64>) -> String {
    let mut sql = format!("BEGIN READ ONLY; {}", search_path_sql(schema));
    if let Some(ms) = timeout_ms.filter(|ms| *ms > 0) {
        sql.push_str(&format!("; SET LOCAL statement_timeout = {ms}"));
    }
    sql
}

/// Acquire a pooled session with this request's context set EXPLICITLY (see
/// [`session_setup_sql`]) and its backend pid recorded in `token`. Pooled
/// sessions keep what an earlier request set, so a request without a selected
/// schema used to run on whichever `search_path` its connection had last been
/// given.
async fn acquire_session(
    pool: &sqlx::PgPool,
    schema: Option<&str>,
    timeout_ms: Option<u64>,
    token: &CancelToken,
) -> Result<sqlx::pool::PoolConnection<sqlx::Postgres>> {
    let mut conn = acquire(pool).await?;
    let rows = sqlx::raw_sql(sqlx::AssertSqlSafe(session_setup_sql(schema, timeout_ms)))
        .fetch_all(&mut *conn)
        .await
        .map_err(types::upstream)?;
    if let Some(pid) = rows.last().and_then(|r| r.try_get::<i32, _>(0).ok()) {
        token.set(QueryHandle::PostgresBackendPid(pid));
    }
    Ok(conn)
}

/// Don't leave a per-statement cap on the pooled session for the tree /
/// completion queries that share it — but don't make the RESPONSE wait for
/// that round trip either: the session finishes its reset in the background
/// and only then returns to the pool.
fn reset_timeout_in_background(mut conn: sqlx::pool::PoolConnection<sqlx::Postgres>) {
    tokio::spawn(async move {
        let _ = sqlx::raw_sql("RESET statement_timeout")
            .execute(&mut *conn)
            .await;
    });
}

async fn run_read(
    pool: &sqlx::PgPool,
    statement: &str,
    max_rows: usize,
    server_bounded: bool,
    active_schema: Option<&str>,
    timeout_ms: Option<u64>,
    token: &CancelToken,
) -> Result<QueryResult> {
    let mut conn = acquire_session(pool, active_schema, timeout_ms, token).await?;
    // Reads leave no session state, except the explicit lock/config functions.
    if types::sql_leaves_session_state(statement) {
        conn.close_on_drop();
    }
    let out = exec_read_conn(
        &mut conn,
        statement,
        max_rows,
        server_bounded,
        &mut types::ByteBudget::default(),
    )
    .await?;
    if out.unread {
        // Rows left on the wire: discard the session (no RESET — it would
        // drain them first; a closed session takes its timeout with it). A
        // server-bounded read at the row cap drained its end-of-result and
        // keeps the session (DB2-01).
        conn.close_on_drop();
        return Ok(out.result);
    }
    if timeout_ms.is_some() {
        reset_timeout_in_background(conn);
    }
    Ok(out.result)
}

async fn run_write(
    pool: &sqlx::PgPool,
    statement: &str,
    active_schema: Option<&str>,
    token: &CancelToken,
) -> Result<QueryResult> {
    let mut conn = acquire_session(pool, active_schema, None, token).await?;
    // A `SET`/`BEGIN`/`SET ROLE`/… would outlive this request on a pooled
    // session: close it afterwards instead of returning it to the pool.
    if types::sql_leaves_session_state(statement) {
        conn.close_on_drop();
    }
    exec_write_conn(&mut conn, statement).await
}

/// Execute a true multi-statement batch (>1 statement) on ONE shared session, in
/// order — first result on top, the rest in `more_results`; stop + `errored`
/// entry on the first failure (§2.2). Eligible SELECTs are server-bounded.
/// Non-rewritable reads retain SQL/session semantics (SQLx drains a clipped
/// result before the next statement); the pager remains single-statement only.
async fn run_batch(
    pool: &sqlx::PgPool,
    spans: &[StatementSpan],
    max_rows: usize,
    active_schema: Option<&str>,
    timeout_ms: Option<u64>,
    token: &CancelToken,
) -> Result<QueryResult> {
    let mut conn = acquire_session(pool, active_schema, timeout_ms, token).await?;
    if spans
        .iter()
        .any(|span| types::sql_leaves_session_state(&span.text))
    {
        conn.close_on_drop();
    }
    let mut results: Vec<QueryResult> = Vec::with_capacity(spans.len());
    // One budget for the whole response, shared by every statement's rows.
    let mut budget = types::ByteBudget::default();
    let mut unread = false;
    for span in spans {
        let stmt = span.text.as_str();
        let started = Instant::now();
        let outcome = if is_read_statement(stmt) {
            let limited = types::inject_row_limit(stmt, max_rows.saturating_add(1), None);
            exec_read_conn(
                &mut conn,
                &limited.sql,
                max_rows,
                limited.limited,
                &mut budget,
            )
            .await
            .map(|out| {
                // A later statement drains them anyway (same session); the
                // last one's are skipped by closing it.
                unread = out.unread;
                out.result
            })
        } else {
            exec_write_conn(&mut conn, stmt).await
        };
        match outcome {
            Ok(mut r) => {
                r.stats.duration_ms = started.elapsed().as_millis() as u64;
                r.stats.row_count = r.rows.len();
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
    if unread {
        conn.close_on_drop();
    } else if timeout_ms.is_some() {
        reset_timeout_in_background(conn);
    }
    Ok(types::fold_batch_results(results))
}

/// A shaped read plus whether rows were left UNREAD on the wire (see the
/// MySQL driver's `ReadOut`): the caller then discards the session instead of
/// letting its next use drain them.
struct ReadOut {
    result: QueryResult,
    unread: bool,
}

/// Run a row-returning statement and shape it into a `QueryResult`, capped at
/// `max_rows` rows and at the response `budget`.
///
/// - Stops pulling at the cap instead of draining the rest of a non-LIMIT-able
///   read (a batch statement, `TABLE t`, VALUES…). A `server_bounded` read
///   (injected `LIMIT max_rows+1`) at the cap instead finishes the result —
///   the probe row was the last, only CommandComplete/ReadyForQuery remain — so
///   the pooled session survives the default "open table" view and every page
///   (DB2-01; it used to be closed, and the next Run reconnected).
/// - Ad-hoc SQL runs as an unnamed statement (`persistent(false)`): it no
///   longer fills the per-session statement cache and evicts the tree /
///   completion statements that ARE reused (DB2-02).
/// - Each column's decoder is picked once from its type ([`PgCell`]).
/// - The budget is charged per row from the RAW wire sizes
///   ([`raw_row_json_len`]), before decoding.
/// - Decoding runs in [`DECODE_CHUNK`]-row chunks on the blocking pool while
///   the next rows stream in (the MySQL pipeline): a wide JSONB/numeric page
///   no longer blocks a tokio worker for tens to hundreds of ms.
async fn exec_read_conn(
    conn: &mut sqlx::PgConnection,
    statement: &str,
    max_rows: usize,
    server_bounded: bool,
    budget: &mut types::ByteBudget,
) -> Result<ReadOut> {
    use futures_util::TryStreamExt as _;

    let mut stream = sqlx::query(sqlx::AssertSqlSafe(statement))
        .persistent(false)
        .fetch(&mut *conn);
    let mut columns: Vec<Column> = Vec::new();
    let mut decoders: std::sync::Arc<[PgCell]> = std::sync::Arc::from(Vec::new());
    let mut chunk: Vec<PgRow> = Vec::new();
    let mut decoding: Vec<tokio::task::JoinHandle<Vec<Vec<Value>>>> = Vec::new();
    let mut kept = 0usize;
    let mut truncated = false;
    let mut truncated_reason = None;
    let mut unread = false;
    while let Some(row) = stream.try_next().await.map_err(types::upstream)? {
        if columns.is_empty() {
            let mut decs = Vec::with_capacity(row.columns().len());
            for col in row.columns() {
                columns.push(Column::typed(col.name(), col.type_info().name()));
                decs.push(pg_cell_decoder(col.type_info().name()));
            }
            decoders = decs.into();
        }
        if kept >= max_rows {
            truncated = true;
            unread = !(server_bounded
                && types::drain_leftover(&mut stream, types::LEFTOVER_DRAIN_ROWS).await);
            break;
        }
        // Always keep at least one row so a single huge row still shows.
        if !budget.charge(raw_row_json_len(&row, &decoders)) && kept > 0 {
            truncated = true;
            truncated_reason = Some(types::TruncatedReason::Bytes);
            unread = true;
            break;
        }
        kept += 1;
        chunk.push(row);
        if chunk.len() >= DECODE_CHUNK {
            let rows = std::mem::take(&mut chunk);
            let dec = decoders.clone();
            decoding.push(tokio::task::spawn_blocking(move || {
                decode_pg_rows(&rows, &dec)
            }));
        }
    }
    drop(stream);

    let mut out_rows: Vec<Vec<Value>> = Vec::with_capacity(kept);
    for task in decoding {
        out_rows.extend(
            task.await
                .map_err(|e| otto_core::Error::Internal(format!("decode task failed: {e}")))?,
        );
    }
    if !chunk.is_empty() {
        if chunk.len() * decoders.len() <= INLINE_DECODE_CELLS {
            out_rows.extend(decode_pg_rows(&chunk, &decoders));
        } else {
            let dec = decoders.clone();
            out_rows.extend(
                tokio::task::spawn_blocking(move || decode_pg_rows(&chunk, &dec))
                    .await
                    .map_err(|e| otto_core::Error::Internal(format!("decode task failed: {e}")))?,
            );
        }
    }

    Ok(ReadOut {
        result: QueryResult {
            columns,
            rows: out_rows,
            truncated,
            truncated_reason,
            ..QueryResult::empty()
        },
        unread,
    })
}

/// Decode fetched rows with their per-column decoders, capping oversized
/// cells (text/bytea/JSON) like ClickHouse does — an uncapped multi-MB cell
/// freezes the grid and bloats the WS frame.
fn decode_pg_rows(rows: &[PgRow], decoders: &[PgCell]) -> Vec<Vec<Value>> {
    rows.iter()
        .map(|row| {
            decoders
                .iter()
                .enumerate()
                .map(|(i, dec)| types::cap_cell(pg_cell(row, i, *dec)))
                .collect()
        })
        .collect()
}

/// Estimated JSON size of a row from its raw wire bytes (no decoding).
fn raw_row_json_len(row: &PgRow, decoders: &[PgCell]) -> usize {
    let mut n = 2;
    for (i, dec) in decoders.iter().enumerate() {
        let raw = row
            .try_get_raw(i)
            .ok()
            .filter(|v| !sqlx::ValueRef::is_null(v))
            .and_then(|v| v.as_bytes().ok().map(<[u8]>::len));
        n += 3 + raw_cell_json_len(raw, *dec);
    }
    n
}

/// Rendered-size estimate of one cell from its raw byte length: text/JSON
/// are ~1:1 (capped at the cell cap), the base64-rendered fallback grows by
/// 4/3, and fixed-width binary values (ints, floats, timestamps, uuids,
/// numerics) are counted at a typical rendered width.
fn raw_cell_json_len(raw: Option<usize>, dec: PgCell) -> usize {
    match raw.map(|len| len.min(types::MAX_CELL_CHARS)) {
        None => 4,
        Some(len) => match dec {
            PgCell::Text | PgCell::Json => len,
            PgCell::Cascade => len.div_ceil(3) * 4,
            _ => len.max(24),
        },
    }
}

/// How one Postgres result column decodes, chosen ONCE from its type name
/// (sqlx `PgTypeInfo::name()`). The common types take exactly one `try_get`;
/// everything else runs the full [`pg_value_to_json`] cascade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PgCell {
    Int2,
    Int4,
    Int8,
    Bool,
    Float4,
    Float8,
    Json,
    Numeric,
    Text,
    TimestampTz,
    Timestamp,
    Date,
    Time,
    Uuid,
    Cascade,
}

fn pg_cell_decoder(type_name: &str) -> PgCell {
    match type_name {
        "INT2" => PgCell::Int2,
        "INT4" => PgCell::Int4,
        "INT8" => PgCell::Int8,
        "BOOL" => PgCell::Bool,
        "FLOAT4" => PgCell::Float4,
        "FLOAT8" => PgCell::Float8,
        "JSON" | "JSONB" => PgCell::Json,
        "NUMERIC" => PgCell::Numeric,
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" => PgCell::Text,
        "TIMESTAMPTZ" => PgCell::TimestampTz,
        "TIMESTAMP" => PgCell::Timestamp,
        "DATE" => PgCell::Date,
        "TIME" => PgCell::Time,
        "UUID" => PgCell::Uuid,
        _ => PgCell::Cascade,
    }
}

/// Decode one cell with its column's decoder — the same shaping the cascade
/// applies to that type; a rejected value falls back to the cascade.
fn pg_cell(row: &PgRow, idx: usize, dec: PgCell) -> Value {
    use sqlx::types::chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
    use sqlx::types::{BigDecimal, Uuid};
    fn text<T: ToString>(v: Option<T>) -> Value {
        v.map(|t| Value::String(t.to_string()))
            .unwrap_or(Value::Null)
    }
    let hit = match dec {
        PgCell::Int2 => row
            .try_get::<Option<i16>, _>(idx)
            .map(|v| v.map(|n| Value::from(n as i64)).unwrap_or(Value::Null))
            .ok(),
        PgCell::Int4 => row
            .try_get::<Option<i32>, _>(idx)
            .map(|v| v.map(Value::from).unwrap_or(Value::Null))
            .ok(),
        PgCell::Int8 => row
            .try_get::<Option<i64>, _>(idx)
            .map(|v| v.map(types::i64_to_json).unwrap_or(Value::Null))
            .ok(),
        PgCell::Bool => row
            .try_get::<Option<bool>, _>(idx)
            .map(|v| v.map(Value::Bool).unwrap_or(Value::Null))
            .ok(),
        PgCell::Float4 => row
            .try_get::<Option<f32>, _>(idx)
            .map(|v| float_to_json(v.map(|n| n as f64)))
            .ok(),
        PgCell::Float8 => row.try_get::<Option<f64>, _>(idx).map(float_to_json).ok(),
        PgCell::Json => row
            .try_get::<Option<Value>, _>(idx)
            .map(|v| v.unwrap_or(Value::Null))
            .ok(),
        PgCell::Numeric => row.try_get::<Option<BigDecimal>, _>(idx).map(text).ok(),
        PgCell::Text => row
            .try_get::<Option<String>, _>(idx)
            .map(|v| v.map(Value::String).unwrap_or(Value::Null))
            .ok(),
        PgCell::TimestampTz => row
            .try_get::<Option<DateTime<Utc>>, _>(idx)
            .map(|v| {
                v.map(|t| Value::String(t.to_rfc3339()))
                    .unwrap_or(Value::Null)
            })
            .ok(),
        PgCell::Timestamp => row.try_get::<Option<NaiveDateTime>, _>(idx).map(text).ok(),
        PgCell::Date => row.try_get::<Option<NaiveDate>, _>(idx).map(text).ok(),
        PgCell::Time => row.try_get::<Option<NaiveTime>, _>(idx).map(text).ok(),
        PgCell::Uuid => row.try_get::<Option<Uuid>, _>(idx).map(text).ok(),
        PgCell::Cascade => None,
    };
    hit.unwrap_or_else(|| pg_value_to_json(row, idx))
}

async fn exec_write_conn(conn: &mut sqlx::PgConnection, statement: &str) -> Result<QueryResult> {
    let res = sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(&mut *conn)
        .await
        .map_err(types::upstream)?;
    let affected = res.rows_affected();
    let mut result = QueryResult::message(format!("{affected} row(s) affected"));
    result.rows_affected = Some(affected);
    Ok(result)
}

/// Decode a single cell of a Postgres row to a `serde_json::Value`. Postgres is
/// strongly typed, so we try each supported type in turn (sqlx rejects an
/// incompatible `try_get`), formatting temporal/decimal/uuid types as strings and
/// falling back to base64 bytes then Null.
fn pg_value_to_json(row: &PgRow, idx: usize) -> Value {
    use sqlx::types::chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
    use sqlx::types::{BigDecimal, Uuid};

    if let Ok(v) = row.try_get::<Option<i32>, _>(idx) {
        return v.map(Value::from).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(idx) {
        // INT8: exact digits beyond 2^53 (see `types::i64_to_json`).
        return v.map(types::i64_to_json).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<i16>, _>(idx) {
        return v.map(|n| Value::from(n as i64)).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<bool>, _>(idx) {
        return v.map(Value::Bool).unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(idx) {
        return float_to_json(v);
    }
    if let Ok(v) = row.try_get::<Option<f32>, _>(idx) {
        return float_to_json(v.map(|n| n as f64));
    }
    // JSON / JSONB decode straight to a Value.
    if let Ok(Some(val)) = row.try_get::<Option<Value>, _>(idx) {
        return val;
    }
    // NUMERIC / DECIMAL → exact string (never lossy f64).
    if let Ok(v) = row.try_get::<Option<BigDecimal>, _>(idx) {
        return v
            .map(|n| Value::String(n.to_string()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(idx) {
        return v.map(Value::String).unwrap_or(Value::Null);
    }
    // Temporal types → ISO-ish strings (chrono formats without extra features).
    if let Ok(v) = row.try_get::<Option<DateTime<Utc>>, _>(idx) {
        return v
            .map(|t| Value::String(t.to_rfc3339()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<NaiveDateTime>, _>(idx) {
        return v
            .map(|t| Value::String(t.to_string()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<NaiveDate>, _>(idx) {
        return v
            .map(|t| Value::String(t.to_string()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<NaiveTime>, _>(idx) {
        return v
            .map(|t| Value::String(t.to_string()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Uuid>, _>(idx) {
        return v
            .map(|u| Value::String(u.to_string()))
            .unwrap_or(Value::Null);
    }
    // Arrays — Postgres uses these everywhere (int[]/text[]/uuid[]/…). Without
    // explicit branches they fell through to Null like the DECIMAL/DATETIME bugs.
    if let Ok(v) = row.try_get::<Option<Vec<String>>, _>(idx) {
        return v
            .map(|a| Value::Array(a.into_iter().map(Value::String).collect()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<i64>>, _>(idx) {
        return v
            .map(|a| Value::Array(a.into_iter().map(Value::from).collect()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<i32>>, _>(idx) {
        return v
            .map(|a| Value::Array(a.into_iter().map(Value::from).collect()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<f64>>, _>(idx) {
        return v
            .map(|a| Value::Array(a.into_iter().map(|n| float_to_json(Some(n))).collect()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<bool>>, _>(idx) {
        return v
            .map(|a| Value::Array(a.into_iter().map(Value::Bool).collect()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<Uuid>>, _>(idx) {
        return v
            .map(|a| {
                Value::Array(
                    a.into_iter()
                        .map(|u| Value::String(u.to_string()))
                        .collect(),
                )
            })
            .unwrap_or(Value::Null);
    }
    // INTERVAL / MONEY — structured binary encodings with no String decode.
    if let Ok(v) = row.try_get::<Option<sqlx::postgres::types::PgInterval>, _>(idx) {
        return v
            .map(|i| Value::String(interval_to_string(&i)))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<sqlx::postgres::types::PgMoney>, _>(idx) {
        return v
            .map(|m| Value::String(money_to_string(m.0)))
            .unwrap_or(Value::Null);
    }
    // INET / CIDR / MACADDR — network types (need the ipnetwork/mac_address features).
    if let Ok(v) = row.try_get::<Option<sqlx::types::ipnetwork::IpNetwork>, _>(idx) {
        return v
            .map(|n| Value::String(n.to_string()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<sqlx::types::mac_address::MacAddress>, _>(idx) {
        return v
            .map(|m| Value::String(m.to_string()))
            .unwrap_or(Value::Null);
    }
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(idx) {
        return match v {
            Some(bytes) => Value::String(B64.encode(bytes)),
            None => Value::Null,
        };
    }
    // Last resort — NEVER silently render a non-NULL cell as Null. User-defined
    // enums (binary encoding = the label text), citext, and future types render
    // as text when UTF-8, else base64. Only a true SQL NULL stays Null.
    raw_cell_fallback(row.try_get_raw(idx))
}

/// Decode-of-last-resort for a raw Postgres value: SQL NULL → Null, UTF-8
/// payload → its text, binary payload → base64. Uses `try_decode_unchecked` —
/// the checked decode would re-reject through the very `compatible()` gates
/// that routed us here. Errors (no such column) → Null.
fn raw_cell_fallback(
    raw: std::result::Result<sqlx::postgres::PgValueRef<'_>, sqlx::Error>,
) -> Value {
    use sqlx::{Value as _, ValueRef as _};
    let Ok(raw) = raw else { return Value::Null };
    if raw.is_null() {
        return Value::Null;
    }
    let owned = sqlx::ValueRef::to_owned(&raw);
    match owned.try_decode_unchecked::<String>() {
        Ok(s) => Value::String(s),
        Err(_) => match owned.try_decode_unchecked::<Vec<u8>>() {
            Ok(b) => Value::String(B64.encode(b)),
            Err(_) => Value::Null,
        },
    }
}

/// Render a `PgInterval` as a compact ISO-8601-ish duration (`P1M2DT3.5S`
/// shape; `PT0S` when zero). Months/days stay separate — Postgres does not
/// normalize them into each other.
fn interval_to_string(i: &sqlx::postgres::types::PgInterval) -> String {
    let mut out = String::from("P");
    if i.months != 0 {
        out.push_str(&format!("{}M", i.months));
    }
    if i.days != 0 {
        out.push_str(&format!("{}D", i.days));
    }
    if i.microseconds != 0 || (i.months == 0 && i.days == 0) {
        let secs = i.microseconds as f64 / 1_000_000.0;
        // Trim trailing zeros for whole-second values (PT90S not PT90.000000S).
        if (secs - secs.trunc()).abs() < f64::EPSILON {
            out.push_str(&format!("T{}S", secs as i64));
        } else {
            out.push_str(&format!("T{secs}S"));
        }
    }
    out
}

/// Render a `PgMoney` (int64 cents at the default locale scale of 2) as a
/// plain decimal string — `1234` → `"12.34"`, `-5` → `"-0.05"`.
fn money_to_string(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

/// Pure shaping of an optional f64 (Null if absent or non-finite).
fn float_to_json(v: Option<f64>) -> Value {
    match v {
        Some(n) => serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        None => Value::Null,
    }
}

// --- Introspection row structs ----------------------------------------------

#[derive(sqlx::FromRow)]
struct PgColumnRow {
    name: String,
    data_type: String,
    nullable: bool,
    col_default: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PgIndexRow {
    name: String,
    is_unique: bool,
    method: String,
    def: Option<String>,
    col: Option<String>,
    #[allow(dead_code)]
    ord: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct PgFkRow {
    name: String,
    col: String,
    ref_schema: String,
    ref_table: String,
    ref_col: String,
    #[allow(dead_code)]
    ord: Option<i32>,
}

// --- Completion data --------------------------------------------------------

const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "ALTER",
    "DROP",
    "TABLE",
    "VIEW",
    "MATERIALIZED",
    "INDEX",
    "SCHEMA",
    "SEQUENCE",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "FULL",
    "OUTER",
    "CROSS",
    "LATERAL",
    "ON",
    "USING",
    "GROUP",
    "BY",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "FETCH",
    "AS",
    "DISTINCT",
    "ON CONFLICT",
    "RETURNING",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS",
    "IN",
    "LIKE",
    "ILIKE",
    "SIMILAR",
    "BETWEEN",
    "EXISTS",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "ALL",
    "ANY",
    "ASC",
    "DESC",
    "NULLS",
    "FIRST",
    "LAST",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "UNIQUE",
    "CHECK",
    "DEFAULT",
    "CONSTRAINT",
    "WITH",
    "RECURSIVE",
    "TRUNCATE",
    "GRANT",
    "REVOKE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "ANALYZE",
    "EXPLAIN",
    "VACUUM",
    "COPY",
    "GENERATED",
    "IDENTITY",
    "SERIAL",
    "TEXT",
    "INTEGER",
    "BIGINT",
    "BOOLEAN",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "JSONB",
    "UUID",
    "NUMERIC",
    "ARRAY",
];

const FUNCTIONS: &[(&str, &str)] = &[
    ("count", "count(expr) — number of rows"),
    ("sum", "sum(expr) — total"),
    ("avg", "avg(expr) — average"),
    ("min", "min(expr) — minimum"),
    ("max", "max(expr) — maximum"),
    ("coalesce", "coalesce(a, b, …) — first non-null"),
    ("nullif", "nullif(a, b) — null if equal"),
    ("greatest", "greatest(a, …) — largest"),
    ("least", "least(a, …) — smallest"),
    ("now", "now() — current timestamptz"),
    ("current_date", "current_date — today"),
    ("current_timestamp", "current_timestamp — now"),
    ("date_trunc", "date_trunc(field, ts) — truncate timestamp"),
    ("date_part", "date_part(field, ts) — extract field"),
    ("extract", "extract(field FROM ts) — extract field"),
    ("to_char", "to_char(value, fmt) — format to text"),
    ("to_timestamp", "to_timestamp(text, fmt) — parse timestamp"),
    ("age", "age(ts) — interval since"),
    ("length", "length(str) — character length"),
    ("char_length", "char_length(str) — character length"),
    ("lower", "lower(str) — lowercase"),
    ("upper", "upper(str) — uppercase"),
    ("trim", "trim(str) — strip whitespace"),
    ("substring", "substring(str FROM a FOR b) — substring"),
    ("substr", "substr(str, a, b) — substring"),
    ("replace", "replace(str, from, to) — replace substring"),
    ("split_part", "split_part(str, delim, n) — nth field"),
    ("concat", "concat(a, b, …) — concatenate"),
    ("concat_ws", "concat_ws(sep, …) — concat with separator"),
    (
        "string_agg",
        "string_agg(expr, delim) — aggregate to string",
    ),
    ("array_agg", "array_agg(expr) — aggregate to array"),
    ("jsonb_agg", "jsonb_agg(expr) — aggregate to JSON array"),
    (
        "jsonb_build_object",
        "jsonb_build_object(k, v, …) — build JSON object",
    ),
    (
        "jsonb_extract_path",
        "jsonb_extract_path(json, path…) — extract",
    ),
    ("round", "round(num, decimals) — round"),
    ("floor", "floor(num) — round down"),
    ("ceil", "ceil(num) — round up"),
    ("abs", "abs(num) — absolute value"),
    ("mod", "mod(a, b) — modulo"),
    ("power", "power(base, exp) — power"),
    ("sqrt", "sqrt(num) — square root"),
    ("random", "random() — random 0..1"),
    ("gen_random_uuid", "gen_random_uuid() — random UUID"),
    ("cast", "cast(expr AS type) — type cast"),
    ("coalesce", "coalesce(a, b, …) — first non-null"),
    ("row_number", "row_number() OVER (…) — window row number"),
    ("rank", "rank() OVER (…) — window rank"),
    ("lag", "lag(expr) OVER (…) — previous row value"),
    ("lead", "lead(expr) OVER (…) — next row value"),
];

// --- Unit tests -------------------------------------------------------------

/// Native read-only transaction is the authority for effects hidden behind
/// overload resolution, operators, casts, views and routines, including root.
/// This path is selected only by a server-derived resolved-config flag.
async fn governed_read(
    pool: &sqlx::PgPool,
    req: &QueryRequest,
    token: &CancelToken,
) -> Result<QueryResult> {
    let mut conn = acquire(pool).await?;
    // Always explicit (reverted with the transaction): the selected schema,
    // else the session default rather than a pooled leftover — set in the
    // same round trip as the BEGIN.
    let schema = req.scope_database();
    let mut tx = conn
        .begin_with(sqlx::AssertSqlSafe(governed_begin_sql(
            schema.as_deref(),
            req.timeout_ms,
        )))
        .await
        .map_err(types::upstream)?;
    capture_backend_pid(&mut tx, token).await;
    let spans = split_statements(req.statement.trim(), SqlDialect::Postgres);
    if spans.is_empty() {
        return Err(types::invalid("empty statement"));
    }
    let max_rows = req.max_rows.unwrap_or(DEFAULT_MAX_ROWS);
    let single = spans.len() == 1;
    let mut results = Vec::new();
    let mut budget = types::ByteBudget::default();
    let mut unread = false;
    for span in spans {
        let started = Instant::now();
        let sql = types::inject_row_limit(
            &span.text,
            max_rows.saturating_add(1),
            if single { req.offset } else { None },
        );
        let out = exec_read_conn(&mut tx, &sql.sql, max_rows, sql.limited, &mut budget).await?;
        unread |= out.unread;
        let mut result = out.result;
        result.stats.duration_ms = started.elapsed().as_millis() as u64;
        result.stats.row_count = result.rows.len();
        if single {
            result.auto_limited = sql.limited.then_some(max_rows as u64)
        } else {
            result.statement = Some(types::statement_preview(&span.text));
        }
        results.push(result);
    }
    if unread {
        // A ROLLBACK would first drain the unread rows: drop the transaction
        // and discard the session (the server rolls back on disconnect).
        drop(tx);
        conn.close_on_drop();
    } else {
        tx.rollback().await.map_err(types::upstream)?;
    }
    Ok(types::fold_batch_results(results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_cell_decoder_is_chosen_by_column_type() {
        for (name, want) in [
            ("INT2", PgCell::Int2),
            ("INT4", PgCell::Int4),
            ("INT8", PgCell::Int8),
            ("BOOL", PgCell::Bool),
            ("FLOAT4", PgCell::Float4),
            ("FLOAT8", PgCell::Float8),
            ("JSON", PgCell::Json),
            ("JSONB", PgCell::Json),
            ("NUMERIC", PgCell::Numeric),
            ("TEXT", PgCell::Text),
            ("VARCHAR", PgCell::Text),
            ("BPCHAR", PgCell::Text),
            ("NAME", PgCell::Text),
            ("TIMESTAMPTZ", PgCell::TimestampTz),
            ("TIMESTAMP", PgCell::Timestamp),
            ("DATE", PgCell::Date),
            ("TIME", PgCell::Time),
            ("UUID", PgCell::Uuid),
            // Arrays, intervals, money, inet, bytea, enums… keep the cascade.
            ("INT4[]", PgCell::Cascade),
            ("INTERVAL", PgCell::Cascade),
            ("BYTEA", PgCell::Cascade),
            ("mood", PgCell::Cascade),
        ] {
            assert_eq!(pg_cell_decoder(name), want, "{name}");
        }
    }

    #[test]
    fn interval_renders_iso_ish() {
        use sqlx::postgres::types::PgInterval;
        let i = PgInterval {
            months: 1,
            days: 2,
            microseconds: 3_500_000,
        };
        assert_eq!(interval_to_string(&i), "P1M2DT3.5S");
        let whole = PgInterval {
            months: 0,
            days: 0,
            microseconds: 90_000_000,
        };
        assert_eq!(interval_to_string(&whole), "PT90S");
        let zero = PgInterval {
            months: 0,
            days: 0,
            microseconds: 0,
        };
        assert_eq!(interval_to_string(&zero), "PT0S");
    }

    #[test]
    fn money_renders_two_decimals() {
        assert_eq!(money_to_string(1234), "12.34");
        assert_eq!(money_to_string(-5), "-0.05");
        assert_eq!(money_to_string(0), "0.00");
        assert_eq!(money_to_string(100), "1.00");
    }

    #[test]
    fn capabilities_are_honest() {
        let c = PostgresDriver::default().capabilities();
        assert_eq!(c.engine, Engine::Postgres);
        assert!(c.sql && c.joins && c.multi_statement && c.cancel && c.explain);
        assert!(!c.transactions);
        assert_eq!(c.default_port, 5432);
    }

    #[test]
    fn detects_read_statements() {
        assert!(is_read_statement("SELECT 1"));
        assert!(is_read_statement("  select * from t"));
        assert!(is_read_statement("WITH c AS (SELECT 1) SELECT * FROM c"));
        assert!(is_read_statement("EXPLAIN SELECT 1"));
        assert!(is_read_statement("TABLE customers"));
        assert!(is_read_statement("VALUES (1),(2)"));
        assert!(is_read_statement("-- c\nSELECT 1"));
    }

    #[test]
    fn detects_write_statements() {
        assert!(!is_read_statement("INSERT INTO t VALUES (1)"));
        assert!(!is_read_statement("UPDATE t SET a = 1"));
        assert!(!is_read_statement("DELETE FROM t"));
        assert!(!is_read_statement("CREATE TABLE t (id INT)"));
        assert!(!is_read_statement("DROP TABLE t"));
        assert!(!is_read_statement("TRUNCATE t"));
    }

    #[test]
    fn quote_ident_doubles_quotes() {
        assert_eq!(quote_ident("plain"), "\"plain\"");
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(
            set_search_path_sql("public"),
            "SET search_path TO \"public\""
        );
    }

    /// Every run sets its whole session context explicitly, so nothing an
    /// earlier request left on the pooled session (a schema, a timeout from a
    /// run cancelled before its reset) applies to it.
    #[test]
    fn session_setup_is_always_explicit() {
        assert_eq!(
            session_setup_sql(Some("shop"), Some(5000)),
            "SET search_path TO \"shop\"; SET statement_timeout = 5000; SELECT pg_backend_pid()"
        );
        assert_eq!(
            session_setup_sql(None, None),
            "RESET search_path; RESET statement_timeout; SELECT pg_backend_pid()"
        );
    }

    /// DB-02 round-trip guard: the whole per-run setup (scope, timeout AND
    /// the cancel pid) is ONE simple-protocol script, so a warm pooled
    /// session pays exactly one round trip before the statement; the governed
    /// path folds its scope into the BEGIN.
    #[test]
    fn per_run_setup_is_one_round_trip() {
        for (schema, timeout) in [(Some("s"), Some(10)), (None, None), (Some("a"), None)] {
            let sql = session_setup_sql(schema, timeout);
            // One script; the pid is its only row-returning statement, last.
            assert!(sql.ends_with("; SELECT pg_backend_pid()"), "{sql}");
            assert_eq!(sql.matches("SELECT").count(), 1, "{sql}");
        }
        assert_eq!(
            governed_begin_sql(Some("shop"), Some(250)),
            "BEGIN READ ONLY; SET search_path TO \"shop\"; SET LOCAL statement_timeout = 250"
        );
        assert_eq!(
            governed_begin_sql(None, Some(0)),
            "BEGIN READ ONLY; RESET search_path"
        );
    }

    #[test]
    fn raw_cell_estimates_track_rendered_size() {
        assert_eq!(raw_cell_json_len(None, PgCell::Text), 4);
        assert_eq!(raw_cell_json_len(Some(100), PgCell::Text), 100);
        assert_eq!(raw_cell_json_len(Some(3), PgCell::Cascade), 4);
        assert_eq!(raw_cell_json_len(Some(8), PgCell::Int8), 24);
        assert_eq!(
            raw_cell_json_len(Some(types::MAX_CELL_CHARS * 4), PgCell::Json),
            types::MAX_CELL_CHARS
        );
    }

    /// The bulk diagram matches the per-object walk's shape: tree order,
    /// PK/FK flags, composite FK edges grouped, `truncated` past the cap,
    /// FK edges only out of kept tables.
    #[test]
    fn bulk_graph_assembles_flags_edges_and_truncation() {
        let rel = |n: &str, kind| BulkRel {
            id: format!("db:s/table:{n}"),
            name: n.into(),
            kind,
        };
        let col = |t: &str, c: &str| BulkCol {
            table: t.into(),
            name: c.into(),
            data_type: "int".into(),
            nullable: false,
            primary_key: false,
        };
        let fk = |t: &str, name: &str, c: &str, rt: &str, rc: &str| BulkFk {
            table: t.into(),
            name: name.into(),
            column: c.into(),
            ref_schema: None,
            ref_table: rt.into(),
            ref_column: rc.into(),
        };
        let g = assemble_bulk_graph(
            "s",
            vec![
                rel("a", NodeKind::Table),
                rel("b", NodeKind::Table),
                rel("v", NodeKind::View),
            ],
            vec![
                col("a", "id"),
                col("b", "id"),
                col("b", "a1"),
                col("b", "a2"),
                col("v", "x"),
            ],
            &[("a".into(), "id".into()), ("b".into(), "id".into())],
            vec![
                fk("b", "fk_ab", "a1", "a", "id"),
                fk("b", "fk_ab", "a2", "a", "id2"),
                fk("v", "fk_v", "x", "a", "id"),
            ],
            2,
        );
        assert!(g.truncated && g.relationships);
        assert_eq!(g.tables.len(), 2);
        assert_eq!(g.tables[1].id, "db:s/table:b");
        let b = &g.tables[1].columns;
        assert!(b[0].primary_key && !b[0].foreign_key);
        assert!(b[1].foreign_key && b[2].foreign_key && !b[1].primary_key);
        assert_eq!(g.edges.len(), 1, "edges out of the cut table are dropped");
        assert_eq!(g.edges[0].from_columns, vec!["a1", "a2"]);
        assert_eq!(g.edges[0].to_columns, vec!["id", "id2"]);
        assert_eq!(g.edges[0].to_schema, "s");
    }

    #[test]
    fn session_changing_statements_retire_the_pooled_session() {
        for sql in [
            "SET search_path TO other",
            "SET ROLE admin",
            "BEGIN",
            "START TRANSACTION",
            "LOCK TABLE t",
            "PREPARE p AS SELECT 1",
            "CREATE TEMP TABLE t (id int)",
            "SELECT set_config('search_path', 'x', false)",
            "SELECT pg_advisory_lock(1)",
            "SELECT pg_try_advisory_lock(1)",
            "SELECT pg_try_advisory_lock_shared(1, 2)",
            "SELECT * INTO TEMP t2 FROM t",
            "select id into temporary table t3 from t",
            "SELECT * INTO UNLOGGED t4 FROM t",
            "WITH x AS (SELECT 1) SELECT * INTO TEMP t5 FROM x",
        ] {
            assert!(types::sql_leaves_session_state(sql), "{sql}");
        }
        for sql in [
            "SELECT * FROM t",
            "UPDATE t SET a = 1",
            "CREATE TABLE t (id int)",
            "SELECT into_x, \"intox\" FROM t",
        ] {
            assert!(!types::sql_leaves_session_state(sql), "{sql}");
        }
    }

    #[test]
    fn float_shaping() {
        assert_eq!(float_to_json(Some(1.5)), serde_json::json!(1.5));
        assert_eq!(float_to_json(None), Value::Null);
        assert_eq!(float_to_json(Some(f64::NAN)), Value::Null);
    }
}
