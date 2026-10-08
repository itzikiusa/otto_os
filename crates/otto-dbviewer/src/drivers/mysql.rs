//! MySQL driver (also MariaDB, Percona, AWS RDS/Aurora — same wire protocol).
//!
//! Connects from a [`ResolvedConfig`] via `sqlx`'s `MySqlConnectOptions`
//! (host/port/user/password/database + TLS), introspects via
//! `information_schema`, decodes rows to `serde_json::Value`, and populates
//! `foreign_keys` for the visual JOIN builder. Connection *pools* are cached
//! per [`ResolvedConfig::cache_key`] and reused across calls — a `MySqlPool`
//! clone is just an `Arc` bump, so the expensive TCP+TLS+auth handshake is paid
//! once and amortized over every subsequent schema/object/query call.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::resource_cache::ResourceCache;
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use otto_core::Result;
use serde_json::Value;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlRow, MySqlSslMode};
use sqlx::{Column as _, Connection as _, Executor as _, Row, TypeInfo};

use crate::driver::Driver;
use crate::export::{ExportCounts, ExportFormat, ExportSink};
use crate::split::{split_statements, SqlDialect, StatementSpan};
use crate::tls::TlsFiles;
use crate::types::{
    self, compact_count, CancelToken, Capabilities, Column, ColumnDef, CompletionContext,
    CompletionResponse, DbQueryPlan, Engine, ForeignKey, IndexDef, NodeKind, NodePath,
    ObjectDetail, ObjectHit, ObjectSearchReq, ObjectSearchResult, QueryHandle, QueryRequest,
    QueryResult, ResolvedConfig, SchemaNode, TestResult,
};

const DEFAULT_MAX_ROWS: usize = 1000;
/// Max pooled connections per cached config. Small — the DB Explorer issues
/// short introspection/query calls per connection. Sized to let the completion
/// snapshot's five independent `information_schema` queries all run in a single
/// concurrent wave (`build_completion_snapshot` fires them via `tokio::join!`)
/// with a connection to spare for a user query racing the build.
const POOL_MAX_CONNECTIONS: u32 = 6;
/// Drop idle pooled connections after this long so a long-lived cached pool
/// doesn't pin server-side connections forever.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a request waits for a pooled session before failing with a clear
/// "connection busy" error instead of sqlx's silent 30 s default.
const POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
/// Upper bound on the out-of-band `KILL QUERY` (its own short-lived connection).
const CANCEL_TIMEOUT: Duration = Duration::from_secs(10);

/// MySQL driver. Holds a per-`cache_key` pool cache so connections are reused
/// across calls instead of re-handshaking every time. `Mutex<HashMap>` is
/// `Default`-constructible, so `#[derive(Default)]` (used by the registry)
/// still works.
#[derive(Default)]
pub struct MysqlDriver {
    pools: ResourceCache<sqlx::MySqlPool>,
    /// Per-connection schema snapshot cache backing smart completion.
    completions: crate::complete::CompletionCache,
}

#[async_trait]
impl Driver for MysqlDriver {
    fn engine(&self) -> Engine {
        Engine::Mysql
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            engine: Engine::Mysql,
            sql: true,
            joins: true,
            // Pooled connections: every `run` acquires an independent session, so
            // there's no place to hold a BEGIN…COMMIT across calls. Advertise the
            // honest `false` rather than a transaction affordance we can't back.
            transactions: false,
            multi_statement: true,
            // `KILL QUERY <conn_id>` on a separate pooled connection.
            cancel: true,
            // `EXPLAIN` / `EXPLAIN FORMAT=JSON`.
            explain: true,
            default_port: 3306,
            schema_levels: vec!["Database".into(), "Table".into(), "Column".into()],
            query_language: "sql".into(),
        }
    }

    async fn test(&self, cfg: &ResolvedConfig) -> Result<TestResult> {
        let started = Instant::now();
        let pool = match self.pool(cfg).await {
            Ok(pool) => pool,
            Err(e) => {
                // Surface connect failures as a non-ok TestResult (not Err), so
                // the UI shows the message rather than a generic 502.
                return Ok(TestResult {
                    ok: false,
                    latency_ms: None,
                    message: e.to_string(),
                    server_version: None,
                });
            }
        };
        let version: String = match sqlx::query_scalar("SELECT VERSION()")
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
        let latency = started.elapsed().as_millis() as u64;
        Ok(TestResult {
            ok: true,
            latency_ms: Some(latency),
            message: "ok".into(),
            server_version: Some(version),
        })
    }

    async fn native_grants(
        &self,
        cfg: &ResolvedConfig,
    ) -> Result<Vec<crate::native_access::NativeGrant>> {
        use crate::native_access::{mysql_grants, setup_error};
        let pool = self.pool(cfg).await?;
        let rows = sqlx::query("SHOW GRANTS")
            .fetch_all(&pool)
            .await
            .map_err(types::upstream)?;
        let rows: Vec<String> = rows
            .iter()
            .map(|r| r.try_get(0))
            .collect::<std::result::Result<_, _>>()
            .map_err(types::upstream)?;
        let mut grants = mysql_grants(&rows)?;
        for grant in &grants {
            if grant.operation == "db_query"
                && !rows.iter().any(|r| {
                    r.contains("SHOW VIEW") && r.contains(&format!(" ON `{}`.* TO ", grant.child))
                })
            {
                return Err(setup_error(
                    "query credentials need SHOW VIEW so definer-view privileges can be inspected",
                ));
            }
        }
        // A definer view can read another database using its owner's rights.
        // Routines/roles are already excluded by SHOW GRANTS above.
        let views: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.views WHERE security_type <> 'INVOKER'",
        )
        .fetch_one(&pool)
        .await
        .map_err(types::upstream)?;
        if views != 0 {
            return Err(setup_error(
                "definer views require a separately verified adapter",
            ));
        }
        // Trigger metadata is hidden without TRIGGER. Refuse write credentials
        // without that inspection privilege rather than treating an empty list
        // as proof. Even visible triggers are refused (definer side effects).
        for grant in &grants {
            if grant.operation == "db_data" {
                let trigger_grant = rows.iter().any(|r| {
                    r.contains("TRIGGER") && r.contains(&format!(" ON `{}`.* TO ", grant.child))
                });
                if !trigger_grant {
                    return Err(setup_error(
                        "data-write credentials need inspectable trigger privileges",
                    ));
                }
                let count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM information_schema.triggers WHERE trigger_schema = ?",
                )
                .bind(&grant.child)
                .fetch_one(&pool)
                .await
                .map_err(types::upstream)?;
                if count != 0 {
                    return Err(setup_error(
                        "native triggers may cross the permitted database scope",
                    ));
                }
            }
        }
        if grants.iter().any(|g| g.operation == "db_data") {
            // MySQL hides foreign keys on inaccessible child databases, yet a
            // DELETE/UPDATE may cascade into them. Without a separate complete
            // catalog inspector, such credentials require all-database data
            // permission. An empty child represents that explicit broad ceiling.
            grants.push(crate::native_access::NativeGrant {
                child: String::new(),
                operation: "db_data",
            });
        }
        Ok(grants)
    }

    async fn schema_root(&self, cfg: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
        let pool = self.pool(cfg).await?;
        // System schemas last but still included.
        let rows: Vec<(String,)> = sqlx::query_as(
            // information_schema text columns come back as VARBINARY in MySQL 8;
            // CAST to CHAR so sqlx decodes them as String.
            "SELECT CAST(schema_name AS CHAR) FROM information_schema.schemata \
             ORDER BY schema_name IN ('mysql','information_schema','performance_schema','sys'), \
                      schema_name",
        )
        .fetch_all(&pool)
        .await
        .map_err(types::upstream)?;
        Ok(rows
            .into_iter()
            .map(|(name,)| {
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
        self.schema_children_with_counts(cfg, parent, filter, false)
            .await
    }

    async fn search_objects(
        &self,
        cfg: &ResolvedConfig,
        req: &ObjectSearchReq,
    ) -> Result<ObjectSearchResult> {
        let pool = self.pool(cfg).await?;
        let limit = req.capped();
        // ONE catalog query covers every schema — the whole reason this is cheap
        // on MySQL. Ask for limit+1 so a full page tells us more exist.
        let mut sql = String::from(
            "SELECT CAST(table_schema AS CHAR), CAST(table_name AS CHAR), CAST(table_type AS CHAR) \
             FROM information_schema.tables \
             WHERE LOWER(CAST(table_name AS CHAR)) LIKE LOWER(?) \
             AND table_schema NOT IN ('information_schema','performance_schema','mysql','sys')",
        );
        if !req.all_schemas() {
            sql.push_str(" AND table_schema = ?");
        }
        sql.push_str(" ORDER BY table_schema, table_name LIMIT ?");
        let pattern = format!("%{}%", req.q);
        let q = sqlx::query_as::<_, (String, String, String)>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(&pattern);
        let q = if req.all_schemas() {
            q
        } else {
            q.bind(req.schema.clone().unwrap_or_default())
        };
        let rows = q
            .bind((limit + 1) as i64)
            .fetch_all(&pool)
            .await
            .map_err(types::upstream)?;

        let truncated = rows.len() > limit;
        let mut schemas: std::collections::HashSet<String> = std::collections::HashSet::new();
        let hits = rows
            .into_iter()
            .take(limit)
            .filter_map(|(schema, name, ttype)| {
                let (kind, seg, label) = if ttype.eq_ignore_ascii_case("VIEW") {
                    (NodeKind::View, "view", "view")
                } else {
                    (NodeKind::Table, "table", "table")
                };
                if !req.wants(label) {
                    return None;
                }
                schemas.insert(schema.clone());
                let path = NodePath::parse(&format!("db:{schema}"))
                    .child(seg, &name)
                    .to_id();
                Some(ObjectHit {
                    schema,
                    name,
                    kind,
                    path,
                })
            })
            .collect();
        Ok(ObjectSearchResult {
            hits,
            truncated,
            scanned: schemas.len(),
            supported: true,
        })
    }

    async fn schema_children_with_counts(
        &self,
        cfg: &ResolvedConfig,
        parent: &NodePath,
        filter: Option<&str>,
        counts: bool,
    ) -> Result<Vec<SchemaNode>> {
        let db = parent
            .get("db")
            .ok_or_else(|| types::invalid("schema_children: parent has no database segment"))?
            .to_string();

        // db:<n>/table:<t> -> columns of the table (filter by column name).
        if let Some(table) = parent.get("table") {
            return self.columns_of(cfg, &db, table, parent, filter).await;
        }

        // db:<n>/folder:tables | folder:views -> the objects in that folder (filter by name).
        if let Some(folder) = parent.get("folder") {
            return self
                .objects_in_folder(cfg, &db, folder, filter, counts)
                .await;
        }

        // db:<n> -> the object folders; no per-folder filter at this level.
        // Tables & Views are always shown (matching Workbench). Procedures &
        // Functions are shown only when the database actually has routines of that
        // kind — with their count as dimmed detail — so the many databases without
        // routines stay uncluttered.
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
        // Best-effort routine counts; on error (e.g. no privilege on
        // information_schema.routines) we simply omit the routine folders rather
        // than failing the whole database expand.
        if let Ok(pool) = self.pool(cfg).await {
            let counts: Vec<(String, i64)> = sqlx::query_as(
                "SELECT CAST(routine_type AS CHAR), COUNT(*) \
                 FROM information_schema.routines WHERE routine_schema = ? \
                 GROUP BY routine_type",
            )
            .bind(&db)
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
            let count_of = |t: &str| {
                counts
                    .iter()
                    .find(|(rt, _)| rt.eq_ignore_ascii_case(t))
                    .map(|(_, n)| *n)
            };
            if let Some(n) = count_of("PROCEDURE").filter(|&n| n > 0) {
                folders.push(
                    SchemaNode::new(
                        parent.child("folder", "procedures").to_id(),
                        "Procedures",
                        NodeKind::Folder,
                    )
                    .with_detail(n.to_string())
                    .expandable(),
                );
            }
            if let Some(n) = count_of("FUNCTION").filter(|&n| n > 0) {
                folders.push(
                    SchemaNode::new(
                        parent.child("folder", "functions").to_id(),
                        "Functions",
                        NodeKind::Folder,
                    )
                    .with_detail(n.to_string())
                    .expandable(),
                );
            }
            // Triggers live in information_schema.triggers (a different catalog
            // table); show the folder only when the database has any.
            let trig_count: Option<i64> = sqlx::query_scalar(
                "SELECT COUNT(*) FROM information_schema.triggers WHERE trigger_schema = ?",
            )
            .bind(&db)
            .fetch_one(&pool)
            .await
            .ok();
            if let Some(n) = trig_count.filter(|&n| n > 0) {
                folders.push(
                    SchemaNode::new(
                        parent.child("folder", "triggers").to_id(),
                        "Triggers",
                        NodeKind::Folder,
                    )
                    .with_detail(n.to_string())
                    .expandable(),
                );
            }
        }
        Ok(folders)
    }

    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        // Stored procedure / function: no columns/indexes/FKs — just parameters
        // (rendered as the object's "columns") and the CREATE DDL.
        if path.get("procedure").is_some() || path.get("function").is_some() {
            return self.routine_detail(cfg, path).await;
        }
        // Trigger: event/timing/table metadata + SHOW CREATE TRIGGER DDL.
        if path.get("trigger").is_some() {
            return self.trigger_detail(cfg, path).await;
        }
        let db = path
            .get("db")
            .ok_or_else(|| types::invalid("object_detail: path has no database segment"))?
            .to_string();
        let table = path
            .get("table")
            .or_else(|| path.get("view"))
            .ok_or_else(|| types::invalid("object_detail: path has no table/view segment"))?
            .to_string();
        let is_view = path.get("view").is_some();

        let pool = self.pool(cfg).await?;

        // The four catalog reads are independent: one concurrent wave (pool of
        // 6) instead of four back-to-back round trips.
        // Columns from information_schema.
        let cols_q = sqlx::query_as::<_, ColumnRow>(
            // CAST text columns to CHAR (MySQL 8 returns information_schema text
            // as VARBINARY); keep the original names so FromRow still matches.
            "SELECT CAST(column_name AS CHAR) AS column_name, \
                    CAST(data_type AS CHAR) AS data_type, \
                    CAST(column_type AS CHAR) AS column_type, \
                    CAST(is_nullable AS CHAR) AS is_nullable, \
                    CAST(column_default AS CHAR) AS column_default, \
                    CAST(column_key AS CHAR) AS column_key, \
                    CAST(extra AS CHAR) AS extra, \
                    CAST(column_comment AS CHAR) AS column_comment, \
                    CAST(collation_name AS CHAR) AS collation_name \
             FROM information_schema.columns \
             WHERE table_schema = ? AND table_name = ? \
             ORDER BY ordinal_position",
        )
        .bind(&db)
        .bind(&table)
        .fetch_all(&pool);

        let (col_rows, indexes, foreign_keys, ddl) = tokio::join!(
            cols_q,
            // Indexes via SHOW INDEX, grouped by Key_name (ordered by Seq_in_index).
            self.indexes_of(&pool, &db, &table),
            // Foreign keys (CRITICAL for the JOIN builder).
            self.foreign_keys_of(&pool, &db, &table),
            // DDL via SHOW CREATE TABLE / VIEW (2nd column).
            self.show_create(&pool, &db, &table, is_view),
        );
        let col_rows = col_rows.map_err(types::upstream)?;
        let indexes = indexes?;
        let foreign_keys = foreign_keys?;
        let ddl = ddl.ok();

        let mut columns = Vec::with_capacity(col_rows.len());
        let mut primary_key = Vec::new();
        for c in col_rows {
            if c.column_key.as_deref() == Some("PRI") {
                primary_key.push(c.column_name.clone());
            }
            let data_type = if c.column_type.is_empty() {
                c.data_type
            } else {
                c.column_type
            };
            columns.push(ColumnDef {
                name: c.column_name,
                data_type,
                nullable: c.is_nullable.eq_ignore_ascii_case("YES"),
                default: c.column_default,
                key: c.column_key.filter(|s| !s.is_empty()),
                extra: c.extra.filter(|s| !s.is_empty()),
                comment: c.column_comment.filter(|s| !s.is_empty()),
                collation: c.collation_name.filter(|s| !s.is_empty()),
            });
        }

        // Row count is intentionally left empty: the only cheap source here is
        // information_schema.tables.table_rows, which is an InnoDB *estimate*
        // (often wildly off) — the user doesn't want estimated counts shown.

        let mut detail = ObjectDetail::new(
            table,
            if is_view {
                NodeKind::View
            } else {
                NodeKind::Table
            },
        );
        detail.columns = columns;
        detail.primary_key = primary_key;
        detail.indexes = indexes;
        detail.foreign_keys = foreign_keys;
        // detail.row_count stays None — no estimated counts by default.
        // See `object_detail_with_opts` for the opt-in InnoDB estimate.
        detail.ddl = ddl;
        Ok(detail)
    }

    /// Opt-in: populate `row_count` from `information_schema.tables.table_rows`
    /// (an InnoDB page-statistics estimate — may be off by ±30% or more on large
    /// tables, but is zero-cost compared with `COUNT(*)`). Only fills the count for
    /// BASE TABLEs; views and non-InnoDB tables return None.
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
        let db = match path.get("db") {
            Some(d) => d.to_string(),
            None => return Ok(detail),
        };
        let table = match path.get("table") {
            Some(t) => t.to_string(),
            None => return Ok(detail),
        };
        let pool = self.pool(cfg).await?;
        // `table_rows` is i64 in InnoDB statistics; treat negative/NULL as absent.
        let est: Option<i64> = sqlx::query_scalar(
            "SELECT table_rows FROM information_schema.tables \
             WHERE table_schema = ? AND table_name = ? AND table_type = 'BASE TABLE'",
        )
        .bind(&db)
        .bind(&table)
        .fetch_optional(&pool)
        .await
        .unwrap_or(None);
        if let Some(n) = est.filter(|&n| n >= 0) {
            detail.row_count = Some(n);
        }
        Ok(detail)
    }

    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        // Run with a throwaway token (no cancel tracking for the bare `run`).
        self.run_tracked(cfg, req, &CancelToken::new()).await
    }

    /// Evict + close the cached pool for `cache_key` (connection close, or a
    /// config change superseded it). `Pool::close` drains the backend
    /// connections; in-flight queries on the pool error out. Also drops the
    /// completion snapshot keyed to the same config.
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

    /// The whole database's diagram in three concurrent `information_schema`
    /// queries (tables, columns with their `PRI` key flag, FK key columns)
    /// instead of one `object_detail` (4 sequential queries) per table.
    async fn schema_graph_bulk(
        &self,
        cfg: &ResolvedConfig,
        schema: &str,
        max_tables: usize,
    ) -> Result<Option<types::SchemaGraph>> {
        use crate::drivers::postgres::{assemble_bulk_graph, BulkCol, BulkFk, BulkRel};
        let pool = self.pool(cfg).await?;
        // Same objects and order as the lazy tree's Tables then Views folders.
        let rels_q = sqlx::query_as::<_, (String, String)>(
            "SELECT CAST(table_name AS CHAR), CAST(table_type AS CHAR) \
             FROM information_schema.tables \
             WHERE table_schema = ? AND table_type IN ('BASE TABLE', 'VIEW') \
             ORDER BY (table_type = 'VIEW'), table_name",
        )
        .bind(schema)
        .fetch_all(&pool);
        let cols_q = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
            "SELECT CAST(table_name AS CHAR), CAST(column_name AS CHAR), \
                    CAST(COALESCE(NULLIF(column_type, ''), data_type) AS CHAR), \
                    CAST(is_nullable AS CHAR), CAST(column_key AS CHAR) \
             FROM information_schema.columns \
             WHERE table_schema = ? \
             ORDER BY table_name, ordinal_position",
        )
        .bind(schema)
        .fetch_all(&pool);
        #[allow(clippy::type_complexity)]
        let fks_q = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(
            "SELECT CAST(kcu.table_name AS CHAR), CAST(kcu.constraint_name AS CHAR), \
                    CAST(kcu.column_name AS CHAR), \
                    CAST(kcu.referenced_table_schema AS CHAR), \
                    CAST(kcu.referenced_table_name AS CHAR), \
                    CAST(kcu.referenced_column_name AS CHAR) \
             FROM information_schema.key_column_usage kcu \
             JOIN information_schema.referential_constraints rc \
               ON rc.constraint_schema = kcu.table_schema \
              AND rc.constraint_name = kcu.constraint_name \
             WHERE kcu.table_schema = ? AND kcu.referenced_table_name IS NOT NULL \
             ORDER BY kcu.table_name, kcu.constraint_name, kcu.ordinal_position",
        )
        .bind(schema)
        .fetch_all(&pool);
        let (rels, cols, fks) = tokio::try_join!(rels_q, cols_q, fks_q).map_err(types::upstream)?;

        let db = NodePath::parse(&format!("db:{schema}"));
        let rels = rels
            .into_iter()
            .map(|(name, table_type)| {
                let (seg, kind) = if table_type.eq_ignore_ascii_case("VIEW") {
                    ("view", NodeKind::View)
                } else {
                    ("table", NodeKind::Table)
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
            .map(|(table, name, data_type, nullable, key)| BulkCol {
                table,
                name,
                data_type,
                nullable: nullable.eq_ignore_ascii_case("YES"),
                primary_key: key.as_deref() == Some("PRI"),
            })
            .collect();
        let fks = fks
            .into_iter()
            .filter_map(|(table, name, column, ref_schema, ref_table, ref_column)| {
                Some(BulkFk {
                    table,
                    name,
                    column,
                    ref_schema,
                    ref_table: ref_table?,
                    ref_column: ref_column?,
                })
            })
            .collect();
        Ok(Some(assemble_bulk_graph(
            schema,
            rels,
            cols,
            &[],
            fks,
            max_tables,
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
            return governed_read(&self.pool(cfg).await?, cfg, req, token).await;
        }
        let text = req.statement.trim();
        if text.is_empty() {
            return Err(types::invalid("empty statement"));
        }
        let max_rows = req.max_rows.unwrap_or(DEFAULT_MAX_ROWS);
        let pool = self.pool(cfg).await?;

        // The active database (if the user selected one, else the profile
        // default) scopes unqualified table names: we `USE` it on the
        // connection before running the query — ALWAYS explicitly, since a
        // pooled session may still be in another request's database.
        let scope_db = req.scope_database();
        let active_db = effective_db(scope_db.as_deref(), cfg);

        // Split with MySQL's lexical rules (backticks, `#` comments, backslash
        // escapes). A true batch (>1 statement) runs every statement in order on
        // one shared session; the single-statement fast path keeps its
        // auto-LIMIT/OFFSET injection and MAX_EXECUTION_TIME hint (§2.2).
        let spans = split_statements(text, SqlDialect::Mysql);
        if spans.len() > 1 {
            return run_batch(&pool, &spans, max_rows, active_db, token).await;
        }
        // 0 spans ⇒ comment-only paste: run the original text (unchanged behavior).
        let statement = spans.first().map(|s| s.text.as_str()).unwrap_or(text);
        let started = Instant::now();

        let (result, auto_limited) = if is_read_statement(statement) {
            let ri = types::inject_row_limit(statement, max_rows.saturating_add(1), req.offset);
            // Inject MySQL's MAX_EXECUTION_TIME(ms) optimizer hint when a per-statement
            // timeout is requested. The hint goes right after the SELECT keyword so it
            // is valid even after LIMIT injection. Non-SELECT reads (e.g. SHOW, EXPLAIN)
            // are passed through unchanged — MySQL only honours the hint on SELECTs.
            let sql = if let Some(ms) = req.timeout_ms.filter(|&t| t > 0) {
                if ri.sql.trim_start().to_uppercase().starts_with("SELECT") {
                    // "SELECT /*+ MAX_EXECUTION_TIME(N) */ ..."
                    ri.sql.replacen(
                        "SELECT",
                        &format!("SELECT /*+ MAX_EXECUTION_TIME({ms}) */"),
                        1,
                    )
                } else {
                    ri.sql.clone()
                }
            } else {
                ri.sql.clone()
            };
            (
                run_read(&pool, &sql, max_rows, ri.limited, active_db, token).await,
                // Report the user-visible page size (max_rows), not the +1 probe.
                ri.limited.then_some(max_rows as u64),
            )
        } else {
            (run_write(&pool, statement, active_db, token).await, None)
        };
        let duration_ms = started.elapsed().as_millis() as u64;

        let mut result = result?;
        result.stats.duration_ms = duration_ms;
        result.stats.row_count = result.rows.len();
        result.auto_limited = auto_limited;
        Ok(result)
    }

    /// Kill the running query on its backend connection: `KILL QUERY <connid>`.
    /// Runs on a separate pooled connection (you can't issue it on the blocked
    /// one). `KILL QUERY` cancels only the *current statement* on that connection,
    /// not the connection itself — the pooled session survives and is reused. A
    /// stale/finished connection id makes MySQL return an "Unknown thread id"
    /// error, which we swallow (the query is already gone — cancel succeeded).
    async fn cancel(&self, cfg: &ResolvedConfig, handle: &QueryHandle) -> Result<()> {
        let QueryHandle::MysqlConnId(conn_id) = handle else {
            return Ok(());
        };
        let Some(pool) = self.pools.get_ready(&cfg.cache_key()) else {
            return Ok(());
        };
        // `KILL QUERY <id>` takes a bare integer; the id came from CONNECTION_ID()
        // (a u64), so there's nothing to escape.
        let sql = format!("KILL QUERY {conn_id}");
        // Best-effort: an already-finished query yields "Unknown thread id 1234"
        // — that's a successful no-op cancel, not a failure to report.
        // Never wait in the pool's queue: Stop matters most exactly when every
        // pooled session is busy. Use an idle one if there is one right now,
        // else a one-off connection built from the pool's own options.
        let cancel = async {
            if let Some(mut conn) = pool.try_acquire() {
                let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str()))
                    .execute(&mut *conn)
                    .await;
                return;
            }
            let opts = pool.connect_options();
            if let Ok(mut conn) = sqlx::MySqlConnection::connect_with(&opts).await {
                let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_str()))
                    .execute(&mut conn)
                    .await;
                let _ = conn.close().await;
            }
        };
        let _ = tokio::time::timeout(CANCEL_TIMEOUT, cancel).await;
        Ok(())
    }

    /// Structured query plan via `EXPLAIN FORMAT=JSON`. The statement is
    /// EXPLAIN-wrapped (never executed raw), so this is read-only by construction.
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
        let node = node.map(str::trim).filter(|s| !s.is_empty());
        let mut conn = acquire_scoped(&pool, effective_db(node, cfg), None).await?;
        let mut conn = conn
            .begin_with("START TRANSACTION READ ONLY")
            .await
            .map_err(types::upstream)?;
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN FORMAT=JSON {stmt}")))
            .fetch_one(&mut *conn)
            .await
            .map_err(types::upstream)?;
        let json_str: String = row.try_get(0).map_err(types::upstream)?;
        let raw: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| types::invalid(format!("parse EXPLAIN JSON: {e}")))?;
        let root = crate::plan::from_mysql_json(&raw);
        Ok(DbQueryPlan {
            engine: "mysql".into(),
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

        // Cached, context-aware completion. The snapshot (databases + tables +
        // index-ranked columns) is built once per (connection, db) and reused
        // until refresh; only the cheap pure analysis runs per keystroke.
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

    /// Streaming export: use sqlx's row CURSOR (`.fetch(&mut conn)`) so the
    /// (potentially huge) result is pulled one row at a time and written straight
    /// through `w` — daemon memory stays bounded (NOT `fetch_all`). Only
    /// row-returning statements are exportable; a write/DDL is rejected (the
    /// service's write-gate already blocks guarded writes, but an export of a
    /// non-read makes no sense).
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
        let node = node.map(str::trim).filter(|s| !s.is_empty());
        let mut conn = acquire_scoped(&pool, effective_db(node, cfg), None).await?;
        let mut conn = conn
            .begin_with("START TRANSACTION READ ONLY")
            .await
            .map_err(types::upstream)?;

        // A real cursor over the wire: each `try_next().await` fetches the next
        // row; nothing buffers the whole result.
        let mut rows = sqlx::query(sqlx::AssertSqlSafe(statement)).fetch(&mut *conn);
        let mut sink = ExportSink::new(w, format);

        let mut header_written = false;
        let mut decoders: Vec<CellDecoder> = Vec::new();
        let mut n: usize = 0;
        while let Some(row) = rows.try_next().await.map_err(types::upstream)? {
            if let Some(cap) = max_rows {
                if n >= cap {
                    break;
                }
            }
            if !header_written {
                decoders = column_decoders(&row);
                let columns: Vec<Column> = row
                    .columns()
                    .iter()
                    .map(|c| Column::typed(c.name(), c.type_info().name()))
                    .collect();
                sink.write_header(&columns)
                    .map_err(|e| otto_core::Error::Internal(format!("write export header: {e}")))?;
                header_written = true;
            }
            let cells: Vec<Value> = decoders
                .iter()
                .enumerate()
                .map(|(i, dec)| mysql_cell(&row, i, *dec))
                .collect();
            sink.write_row(&cells)
                .map_err(|e| otto_core::Error::Internal(format!("write export row: {e}")))?;
            n += 1;
        }
        // An empty result still needs the header (with-names) / `[]` (json).
        if !header_written {
            sink.write_header(&[])
                .map_err(|e| otto_core::Error::Internal(format!("write export header: {e}")))?;
        }
        sink.finish()
            .map_err(|e| otto_core::Error::Internal(format!("finish export file: {e}")))
    }
}

impl MysqlDriver {
    /// Columns of a table (lazy expansion of a `table:` node). When `filter` is
    /// given, returns only columns whose name contains the substring
    /// (case-insensitive, via SQL LIKE).
    async fn columns_of(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
        table: &str,
        parent: &NodePath,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let pool = self.pool(cfg).await?;
        // Build the LIKE pattern when a filter is present; the `%` wrapping makes
        // it a substring match. `LOWER()` on both sides is the portable MySQL way
        // to do case-insensitive LIKE without relying on the collation.
        let (sql, name_filter): (&str, Option<String>) = match filter {
            Some(f) if !f.is_empty() => (
                "SELECT CAST(column_name AS CHAR), CAST(column_type AS CHAR) \
                 FROM information_schema.columns \
                 WHERE table_schema = ? AND table_name = ? \
                 AND LOWER(CAST(column_name AS CHAR)) LIKE LOWER(?) \
                 ORDER BY ordinal_position",
                Some(format!("%{f}%")),
            ),
            _ => (
                "SELECT CAST(column_name AS CHAR), CAST(column_type AS CHAR) \
                 FROM information_schema.columns \
                 WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
                None,
            ),
        };
        let rows: Vec<(String, String)> = if let Some(pat) = name_filter {
            sqlx::query_as(sql)
                .bind(db)
                .bind(table)
                .bind(pat)
                .fetch_all(&pool)
                .await
        } else {
            sqlx::query_as(sql)
                .bind(db)
                .bind(table)
                .fetch_all(&pool)
                .await
        }
        .map_err(types::upstream)?;
        Ok(rows
            .into_iter()
            .map(|(name, column_type)| {
                SchemaNode::new(
                    parent.child("column", &name).to_id(),
                    name,
                    NodeKind::Column,
                )
                .with_detail(column_type)
            })
            .collect())
    }

    /// Tables or views in a database folder. When `filter` is given, returns only
    /// objects whose name contains the substring (case-insensitive, via SQL LIKE).
    async fn objects_in_folder(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
        folder: &str,
        filter: Option<&str>,
        counts: bool,
    ) -> Result<Vec<SchemaNode>> {
        // Routine folders live in information_schema.routines (not .tables) and
        // their leaves have no children — clicking one opens its DDL in Structure.
        if folder == "procedures" || folder == "functions" {
            return self.routines_in_folder(cfg, db, folder, filter).await;
        }
        // Triggers live in information_schema.triggers; leaves open their DDL.
        if folder == "triggers" {
            return self.triggers_in_folder(cfg, db, filter).await;
        }
        let (table_type, kind, seg) = match folder {
            "tables" => ("BASE TABLE", NodeKind::Table, "table"),
            "views" => ("VIEW", NodeKind::View, "view"),
            other => return Err(types::invalid(format!("unknown folder: {other}"))),
        };
        let pool = self.pool(cfg).await?;
        // List by NAME only. We intentionally do NOT read `table_rows` here:
        // computing the row-count statistic for every table is the slow part of
        // expanding a database on big servers. The per-table count still shows
        // up in `object_detail` (a single table). CAST text → CHAR (MySQL 8
        // returns information_schema text as VARBINARY) so sqlx decodes String.
        //
        // The optional LIKE clause implements the server-side case-insensitive
        // substring filter passed from `schema_children`.
        let (sql, name_filter): (&str, Option<String>) = match filter {
            Some(f) if !f.is_empty() => (
                "SELECT CAST(table_name AS CHAR) FROM information_schema.tables \
                 WHERE table_schema = ? AND table_type = ? \
                 AND LOWER(CAST(table_name AS CHAR)) LIKE LOWER(?) \
                 ORDER BY table_name",
                Some(format!("%{f}%")),
            ),
            _ => (
                "SELECT CAST(table_name AS CHAR) FROM information_schema.tables \
                 WHERE table_schema = ? AND table_type = ? ORDER BY table_name",
                None,
            ),
        };
        let rows: Vec<(String,)> = if let Some(pat) = name_filter {
            sqlx::query_as(sql)
                .bind(db)
                .bind(table_type)
                .bind(pat)
                .fetch_all(&pool)
                .await
        } else {
            sqlx::query_as(sql)
                .bind(db)
                .bind(table_type)
                .fetch_all(&pool)
                .await
        }
        .map_err(types::upstream)?;
        // The db node only carries `db:<n>`, not the folder; build children off
        // a clean db path so ids are `db:<n>/<seg>:<name>`.
        let db_path = NodePath::parse(&format!("db:{db}"));
        // Opt-in row counts: ONE extra catalog query for the whole folder (never
        // per table, never COUNT(*)). `table_rows` is InnoDB's ESTIMATE — good
        // enough to spot which tables hold data, and the reason this is a toggle
        // rather than the default is that collecting it is what makes expanding
        // a big server slow. Best-effort: a failure just omits the detail.
        let mut est: HashMap<String, i64> = HashMap::new();
        if counts {
            let rows: Vec<(String, Option<i64>)> = sqlx::query_as(
                "SELECT CAST(table_name AS CHAR), CAST(table_rows AS SIGNED) \
                 FROM information_schema.tables WHERE table_schema = ? AND table_type = ?",
            )
            .bind(db)
            .bind(table_type)
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
            for (name, n) in rows {
                est.insert(name, n.unwrap_or(0));
            }
        }
        Ok(rows
            .into_iter()
            .map(|(name,)| {
                let node =
                    SchemaNode::new(db_path.child(seg, &name).to_id(), &name, kind).expandable();
                match est.get(&name) {
                    Some(n) => node.with_detail(compact_count(*n)),
                    None => node,
                }
            })
            .collect())
    }

    /// List stored procedures / functions in a database (the `Procedures` /
    /// `Functions` tree folders), with an optional case-insensitive substring
    /// filter. Leaves are NOT expandable — a routine has no child nodes; clicking
    /// one opens its parameters + DDL in the Structure view.
    async fn routines_in_folder(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
        folder: &str,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let (routine_type, kind, seg) = match folder {
            "procedures" => ("PROCEDURE", NodeKind::Procedure, "procedure"),
            "functions" => ("FUNCTION", NodeKind::Function, "function"),
            other => return Err(types::invalid(format!("unknown routine folder: {other}"))),
        };
        let pool = self.pool(cfg).await?;
        // CAST text → CHAR (MySQL 8 returns information_schema text as VARBINARY).
        let (sql, name_filter): (&str, Option<String>) = match filter {
            Some(f) if !f.is_empty() => (
                "SELECT CAST(routine_name AS CHAR) FROM information_schema.routines \
                 WHERE routine_schema = ? AND routine_type = ? \
                 AND LOWER(CAST(routine_name AS CHAR)) LIKE LOWER(?) \
                 ORDER BY routine_name",
                Some(format!("%{f}%")),
            ),
            _ => (
                "SELECT CAST(routine_name AS CHAR) FROM information_schema.routines \
                 WHERE routine_schema = ? AND routine_type = ? ORDER BY routine_name",
                None,
            ),
        };
        let rows: Vec<(String,)> = if let Some(pat) = name_filter {
            sqlx::query_as(sql)
                .bind(db)
                .bind(routine_type)
                .bind(pat)
                .fetch_all(&pool)
                .await
        } else {
            sqlx::query_as(sql)
                .bind(db)
                .bind(routine_type)
                .fetch_all(&pool)
                .await
        }
        .map_err(types::upstream)?;
        let db_path = NodePath::parse(&format!("db:{db}"));
        Ok(rows
            .into_iter()
            .map(|(name,)| SchemaNode::new(db_path.child(seg, &name).to_id(), name, kind))
            .collect())
    }

    async fn indexes_of(
        &self,
        pool: &sqlx::MySqlPool,
        db: &str,
        table: &str,
    ) -> Result<Vec<IndexDef>> {
        let sql = format!("SHOW INDEX FROM `{}`.`{}`", esc_ident(db), esc_ident(table));
        let rows = match sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            // Views and permission edge-cases: no indexes rather than a hard error.
            Err(_) => return Ok(Vec::new()),
        };
        // Preserve first-seen order of Key_name, columns ordered by Seq_in_index.
        let mut order: Vec<String> = Vec::new();
        #[allow(clippy::type_complexity)]
        let mut by_name: std::collections::HashMap<
            String,
            (bool, Option<String>, Vec<(i64, String)>),
        > = std::collections::HashMap::new();
        for row in rows {
            let key_name: String = row.try_get("Key_name").unwrap_or_default();
            let non_unique: i64 = try_get_int(&row, "Non_unique").unwrap_or(1);
            let seq: i64 = try_get_int(&row, "Seq_in_index").unwrap_or(0);
            let col: String = row.try_get("Column_name").unwrap_or_default();
            let index_type: Option<String> = row.try_get("Index_type").ok();
            let entry = by_name.entry(key_name.clone()).or_insert_with(|| {
                order.push(key_name.clone());
                (non_unique == 0, index_type, Vec::new())
            });
            entry.2.push((seq, col));
        }
        let mut indexes = Vec::with_capacity(order.len());
        for name in order {
            if let Some((unique, method, mut cols)) = by_name.remove(&name) {
                cols.sort_by_key(|(seq, _)| *seq);
                indexes.push(IndexDef {
                    name,
                    columns: cols.into_iter().map(|(_, c)| c).collect(),
                    unique,
                    method,
                    definition: None,
                });
            }
        }
        Ok(indexes)
    }

    async fn foreign_keys_of(
        &self,
        pool: &sqlx::MySqlPool,
        db: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        // key_column_usage gives the local/ref columns; join referential_constraints
        // to scope to actual FK constraints. Order by position for composite keys.
        let rows: Vec<FkRow> = sqlx::query_as(
            "SELECT CAST(kcu.constraint_name AS CHAR) AS constraint_name, \
                    CAST(kcu.column_name AS CHAR) AS column_name, \
                    CAST(kcu.referenced_table_schema AS CHAR) AS referenced_table_schema, \
                    CAST(kcu.referenced_table_name AS CHAR) AS referenced_table_name, \
                    CAST(kcu.referenced_column_name AS CHAR) AS referenced_column_name \
             FROM information_schema.key_column_usage kcu \
             JOIN information_schema.referential_constraints rc \
               ON rc.constraint_schema = kcu.table_schema \
              AND rc.constraint_name = kcu.constraint_name \
             WHERE kcu.table_schema = ? AND kcu.table_name = ? \
               AND kcu.referenced_table_name IS NOT NULL \
             ORDER BY kcu.constraint_name, kcu.ordinal_position",
        )
        .bind(db)
        .bind(table)
        .fetch_all(pool)
        .await
        .map_err(types::upstream)?;

        let mut order: Vec<String> = Vec::new();
        let mut by_name: std::collections::HashMap<String, ForeignKey> =
            std::collections::HashMap::new();
        for r in rows {
            let entry = by_name.entry(r.constraint_name.clone()).or_insert_with(|| {
                order.push(r.constraint_name.clone());
                ForeignKey {
                    name: r.constraint_name.clone(),
                    columns: Vec::new(),
                    ref_table: r.referenced_table_name.clone().unwrap_or_default(),
                    ref_columns: Vec::new(),
                    ref_schema: r.referenced_table_schema.clone(),
                }
            });
            entry.columns.push(r.column_name);
            if let Some(rc) = r.referenced_column_name {
                entry.ref_columns.push(rc);
            }
        }
        Ok(order
            .into_iter()
            .filter_map(|n| by_name.remove(&n))
            .collect())
    }

    async fn show_create(
        &self,
        pool: &sqlx::MySqlPool,
        db: &str,
        table: &str,
        is_view: bool,
    ) -> Result<String> {
        let kw = if is_view { "VIEW" } else { "TABLE" };
        let sql = format!(
            "SHOW CREATE {kw} `{}`.`{}`",
            esc_ident(db),
            esc_ident(table)
        );
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .fetch_one(pool)
            .await
            .map_err(types::upstream)?;
        // The DDL is the 2nd column for tables; for views it's "Create View"
        // (also the 2nd column). Read by index for robustness.
        let ddl: String = row.try_get(1).map_err(types::upstream)?;
        Ok(ddl)
    }

    /// Structure of a stored procedure / function: its parameters (mapped into the
    /// [`ColumnDef`] shape the Structure view already renders — labelled
    /// "Parameters" in the UI) plus the full `SHOW CREATE` DDL.
    async fn routine_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        let db = path
            .get("db")
            .ok_or_else(|| types::invalid("routine_detail: path has no database segment"))?
            .to_string();
        let (name, is_function, kind) = if let Some(n) = path.get("function") {
            (n.to_string(), true, NodeKind::Function)
        } else {
            let n = path
                .get("procedure")
                .ok_or_else(|| types::invalid("routine_detail: path has no routine segment"))?;
            (n.to_string(), false, NodeKind::Procedure)
        };
        let pool = self.pool(cfg).await?;

        // Parameters, in declaration order. For a FUNCTION, ordinal_position 0 is
        // the RETURN type (no name / mode). CAST(... AS SIGNED/CHAR) so sqlx
        // decodes ordinal as i64 and the text columns as String (MySQL 8 returns
        // information_schema text as VARBINARY).
        let param_rows: Vec<RoutineParamRow> = sqlx::query_as(
            "SELECT CAST(ordinal_position AS SIGNED) AS ordinal_position, \
                    CAST(parameter_mode AS CHAR) AS parameter_mode, \
                    CAST(parameter_name AS CHAR) AS parameter_name, \
                    CAST(dtd_identifier AS CHAR) AS dtd_identifier \
             FROM information_schema.parameters \
             WHERE specific_schema = ? AND specific_name = ? \
             ORDER BY ordinal_position",
        )
        .bind(&db)
        .bind(&name)
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

        let mut columns = Vec::with_capacity(param_rows.len());
        for p in param_rows {
            let is_return = p.ordinal_position == 0; // function return value
            columns.push(ColumnDef {
                name: if is_return {
                    "(return)".to_string()
                } else {
                    p.parameter_name.unwrap_or_default()
                },
                data_type: p.dtd_identifier.unwrap_or_default(),
                nullable: true,
                default: None,
                key: None,
                extra: if is_return {
                    Some("RETURNS".to_string())
                } else {
                    p.parameter_mode.filter(|s| !s.is_empty())
                },
                comment: None,
                collation: None,
            });
        }

        // `.ok().flatten()`: a hard error (routine dropped concurrently, or SHOW
        // CREATE itself denied) OR a NULL "Create …" column (definer privilege
        // missing) both collapse to `None` → the UI shows a privilege hint.
        let ddl = self
            .show_create_routine(&pool, &db, &name, is_function)
            .await
            .ok()
            .flatten();

        let mut detail = ObjectDetail::new(name, kind);
        detail.columns = columns;
        detail.ddl = ddl;
        Ok(detail)
    }

    /// `SHOW CREATE PROCEDURE|FUNCTION` DDL. Unlike `SHOW CREATE TABLE`, the DDL
    /// here is the **3rd** column ("Create Procedure"/"Create Function"), read by
    /// index for robustness. Returns `Ok(None)` when that column is NULL — MySQL
    /// blanks it (rather than erroring) when the account lacks the privilege to
    /// view the routine's definition (`SHOW_ROUTINE` / SELECT on the routine).
    async fn show_create_routine(
        &self,
        pool: &sqlx::MySqlPool,
        db: &str,
        name: &str,
        is_function: bool,
    ) -> Result<Option<String>> {
        let kw = if is_function { "FUNCTION" } else { "PROCEDURE" };
        let sql = format!("SHOW CREATE {kw} `{}`.`{}`", esc_ident(db), esc_ident(name));
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .fetch_one(pool)
            .await
            .map_err(types::upstream)?;
        let ddl: Option<String> = row.try_get(2).map_err(types::upstream)?;
        Ok(ddl)
    }

    /// List a database's triggers (the `Triggers` tree folder), optional
    /// case-insensitive substring filter. Leaves aren't expandable — clicking one
    /// opens its event/timing/table + DDL in Structure.
    async fn triggers_in_folder(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let pool = self.pool(cfg).await?;
        let (sql, name_filter): (&str, Option<String>) = match filter {
            Some(f) if !f.is_empty() => (
                "SELECT CAST(trigger_name AS CHAR) FROM information_schema.triggers \
                 WHERE trigger_schema = ? \
                 AND LOWER(CAST(trigger_name AS CHAR)) LIKE LOWER(?) \
                 ORDER BY trigger_name",
                Some(format!("%{f}%")),
            ),
            _ => (
                "SELECT CAST(trigger_name AS CHAR) FROM information_schema.triggers \
                 WHERE trigger_schema = ? ORDER BY trigger_name",
                None,
            ),
        };
        let rows: Vec<(String,)> = if let Some(pat) = name_filter {
            sqlx::query_as(sql)
                .bind(db)
                .bind(pat)
                .fetch_all(&pool)
                .await
        } else {
            sqlx::query_as(sql).bind(db).fetch_all(&pool).await
        }
        .map_err(types::upstream)?;
        let db_path = NodePath::parse(&format!("db:{db}"));
        Ok(rows
            .into_iter()
            .map(|(name,)| {
                SchemaNode::new(
                    db_path.child("trigger", &name).to_id(),
                    name,
                    NodeKind::Trigger,
                )
            })
            .collect())
    }

    /// Trigger structure: its event/timing/table metadata (in `extra`) plus the
    /// full `SHOW CREATE TRIGGER` DDL.
    async fn trigger_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        let db = path
            .get("db")
            .ok_or_else(|| types::invalid("trigger_detail: path has no database segment"))?
            .to_string();
        let name = path
            .get("trigger")
            .ok_or_else(|| types::invalid("trigger_detail: path has no trigger segment"))?
            .to_string();
        let pool = self.pool(cfg).await?;
        let meta: Option<TriggerRow> = sqlx::query_as(
            "SELECT CAST(event_manipulation AS CHAR) AS event_manipulation, \
                    CAST(action_timing AS CHAR) AS action_timing, \
                    CAST(event_object_table AS CHAR) AS event_object_table \
             FROM information_schema.triggers \
             WHERE trigger_schema = ? AND trigger_name = ?",
        )
        .bind(&db)
        .bind(&name)
        .fetch_optional(&pool)
        .await
        .map_err(types::upstream)?;

        // `.ok().flatten()`: a hard error or a NULL definition (privilege missing)
        // both collapse to `None`.
        let ddl = self
            .show_create_trigger(&pool, &db, &name)
            .await
            .ok()
            .flatten();

        let mut detail = ObjectDetail::new(name, NodeKind::Trigger);
        if let Some(m) = meta {
            detail.extra = serde_json::json!({
                "timing": m.action_timing,
                "event": m.event_manipulation,
                "table": m.event_object_table,
            });
        }
        detail.ddl = ddl;
        Ok(detail)
    }

    /// `SHOW CREATE TRIGGER` DDL (the "SQL Original Statement" column, index 2).
    /// `SHOW CREATE TRIGGER` resolves against the default database, so we `USE`
    /// the trigger's schema on a scratch connection first. Returns `Ok(None)` when
    /// the definition column is NULL (definer privilege missing).
    async fn show_create_trigger(
        &self,
        pool: &sqlx::MySqlPool,
        db: &str,
        name: &str,
    ) -> Result<Option<String>> {
        let mut conn = acquire(pool).await?;
        (&mut *conn)
            .execute(sqlx::raw_sql(sqlx::AssertSqlSafe(use_db_sql(db))))
            .await
            .map_err(types::upstream)?;
        let sql = format!("SHOW CREATE TRIGGER `{}`", esc_ident(name));
        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .fetch_one(&mut *conn)
            .await
            .map_err(types::upstream)?;
        let ddl: Option<String> = row.try_get(2).map_err(types::upstream)?;
        Ok(ddl)
    }
}

// --- Connection -------------------------------------------------------------

impl MysqlDriver {
    /// Get (or lazily build) the cached completion snapshot for `(connection, db)`.
    /// Built once and reused until refresh; a connection failure yields an empty
    /// snapshot cached for `COMPLETION_NEGATIVE_TTL` (retried after that, or on refresh).
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

    /// Introspect `information_schema` into a [`SchemaSnapshot`]: databases, the
    /// scoped db's tables/views, and columns ranked by index membership
    /// (PRIMARY → Pk, unique → Unique, other index → Index) so completion can put
    /// indexed columns first. Four bulk queries, paid once per refresh.
    async fn build_completion_snapshot(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
    ) -> Option<crate::complete::SchemaSnapshot> {
        use crate::complete::{FieldSnap, ObjKind, ObjectSnap, Rank, RoutineSnap, SchemaSnapshot};

        let pool = self.pool(cfg).await.ok()?;

        // No db scope ⇒ only the (cheap) database list is needed.
        if db.is_empty() {
            let databases: Vec<String> = sqlx::query_as::<_, (String,)>(
                "SELECT CAST(schema_name AS CHAR) FROM information_schema.schemata ORDER BY schema_name",
            )
            .fetch_all(&pool)
            .await
            .ok()?
            .into_iter()
            .map(|(d,)| d)
            .collect();
            return Some(SchemaSnapshot {
                databases,
                objects: Vec::new(),
                routines: Vec::new(),
            });
        }

        // Five INDEPENDENT information_schema introspections: databases, the
        // scoped db's tables/views, its columns, its routines, and its index
        // membership. They were once run sequentially — five serial round-trips,
        // which over a remote / tunneled MySQL (a real RTT per query) is what made
        // the FIRST completion after a (re)connect stall ~1s+. Fire them all
        // concurrently so the cold snapshot build costs ~one round-trip of
        // wall-clock instead of five; the pool is sized (POOL_MAX_CONNECTIONS) to
        // run them in a single wave. Per-query failures still degrade to empty
        // (`unwrap_or_default`), exactly as before; only a failed schemata list
        // (the one hard dependency) aborts the whole snapshot.
        let (databases, tables, cols, routines_raw, stats) = tokio::join!(
            sqlx::query_as::<_, (String,)>(
                "SELECT CAST(schema_name AS CHAR) FROM information_schema.schemata ORDER BY schema_name",
            )
            .fetch_all(&pool),
            sqlx::query_as::<_, (String, String)>(
                "SELECT CAST(table_name AS CHAR), table_type FROM information_schema.tables \
                 WHERE table_schema = ? ORDER BY table_name",
            )
            .bind(db)
            .fetch_all(&pool),
            sqlx::query_as::<_, (String, String, String)>(
                "SELECT CAST(table_name AS CHAR), CAST(column_name AS CHAR), CAST(column_type AS CHAR) \
                 FROM information_schema.columns WHERE table_schema = ? \
                 ORDER BY table_name, ordinal_position",
            )
            .bind(db)
            .fetch_all(&pool),
            // Stored procedures / functions — for routine-name completion after
            // `SHOW CREATE PROCEDURE`/`FUNCTION`, `CALL`, `DROP PROCEDURE`/`FUNCTION`.
            sqlx::query_as::<_, (String, String)>(
                "SELECT CAST(routine_name AS CHAR), CAST(routine_type AS CHAR) \
                 FROM information_schema.routines WHERE routine_schema = ? ORDER BY routine_name",
            )
            .bind(db)
            .fetch_all(&pool),
            // (table, column) → strongest index rank, from information_schema.statistics
            // (which lists every index member column, so composite-index members all rank).
            sqlx::query_as::<_, (String, String, String, i64)>(
                "SELECT CAST(table_name AS CHAR), CAST(column_name AS CHAR), \
                 CAST(index_name AS CHAR), non_unique FROM information_schema.statistics \
                 WHERE table_schema = ?",
            )
            .bind(db)
            .fetch_all(&pool),
        );

        // Databases is the one hard dependency; the rest degrade to empty.
        let databases: Vec<String> = databases.ok()?.into_iter().map(|(d,)| d).collect();
        let tables = tables.unwrap_or_default();
        let cols = cols.unwrap_or_default();
        let routines: Vec<RoutineSnap> = routines_raw
            .unwrap_or_default()
            .into_iter()
            .map(|(name, rtype)| RoutineSnap {
                name,
                is_function: rtype.eq_ignore_ascii_case("FUNCTION"),
            })
            .collect();
        let stats = stats.unwrap_or_default();

        let mut rank: HashMap<(String, String), Rank> = HashMap::new();
        for (t, c, idx, non_unique) in stats {
            let r = if idx.eq_ignore_ascii_case("PRIMARY") {
                Rank::Pk
            } else if non_unique == 0 {
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

        // Group columns by table (preserving ordinal order from the query).
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
        // Any tables that appeared only in columns (shouldn't happen, but be safe).
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

    /// Get (or lazily build + cache) the pool for `cfg`. The cache is keyed by
    /// [`ResolvedConfig::cache_key`], so any session-affecting difference
    /// (endpoint/creds/db/TLS/timezone) gets its own pool. A `MySqlPool` clone
    /// is an `Arc` bump, so callers get a cheap handle to a shared pool and the
    /// expensive handshake is paid once. Initializers serialize only their own
    /// key; unrelated warm pools never wait for this handshake.
    async fn pool(&self, cfg: &ResolvedConfig) -> Result<sqlx::MySqlPool> {
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

/// Build a fresh `MySqlPool` from a resolved config. Never called directly by
/// the driver methods — they go through [`MysqlDriver::pool`] for caching.
async fn build_pool(cfg: &ResolvedConfig) -> Result<sqlx::MySqlPool> {
    let mut opts = MySqlConnectOptions::new().host(&cfg.host).port(cfg.port);
    if let Some(user) = cfg.user.as_deref().filter(|s| !s.is_empty()) {
        opts = opts.username(user);
    }
    if let Some(password) = cfg.password.as_deref() {
        opts = opts.password(password);
    }
    if let Some(db) = cfg.database.as_deref().filter(|s| !s.is_empty()) {
        opts = opts.database(db);
    }

    // TLS. When verification is on, verify the HOSTNAME too (VerifyIdentity) —
    // VerifyCa alone accepts any CA-signed cert for a different host (MITM).
    // Exception: through an SSH tunnel the TCP endpoint is 127.0.0.1 (the
    // cert's name can never match), and an explicit `tls.server_name` override
    // has no sqlx surface to honour — both keep chain-only verification.
    let hostname_checkable = cfg.param_str("__tunnel_host").is_none()
        && cfg
            .tls
            .server_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .is_none();
    let ssl_mode = match cfg.tls.mode {
        types::TlsMode::Disabled => MySqlSslMode::Disabled,
        types::TlsMode::Preferred => MySqlSslMode::Preferred,
        types::TlsMode::Required => {
            if cfg.tls.verify && hostname_checkable {
                MySqlSslMode::VerifyIdentity
            } else if cfg.tls.verify {
                MySqlSslMode::VerifyCa
            } else {
                MySqlSslMode::Required
            }
        }
    };
    opts = opts.ssl_mode(ssl_mode);

    if cfg.tls.enabled() {
        let files = TlsFiles::materialize(&cfg.tls)?;
        if let Some(ca) = files.ca {
            opts = opts.ssl_ca(ca);
        }
        if let Some(cert) = files.client_cert {
            opts = opts.ssl_client_cert(cert);
        }
        if let Some(key) = files.client_key {
            opts = opts.ssl_client_key(key);
        }
    }

    // Align the session time zone (default UTC) so TIMESTAMP values and NOW()
    // render in the user's configured zone — "see values as actually treated".
    let tz = mysql_session_tz(cfg);
    MySqlPoolOptions::new()
        .max_connections(POOL_MAX_CONNECTIONS)
        .idle_timeout(POOL_IDLE_TIMEOUT)
        .acquire_timeout(POOL_ACQUIRE_TIMEOUT)
        .after_connect(move |conn, _meta| {
            let setup = connect_setup_sql(&tz);
            let fallback = connect_setup_fallback_sql(&tz);
            Box::pin(async move {
                // ONE round trip, unprepared (COM_QUERY): both settings in a
                // single statement. Best-effort: ignore errors so a bad zone
                // never breaks the session — on failure retry the variables
                // one by one (a server lacking the stats var, or one whose tz
                // tables aren't loaded, must still get the other setting).
                if sqlx::raw_sql(sqlx::AssertSqlSafe(setup.as_str()))
                    .execute(&mut *conn)
                    .await
                    .is_err()
                {
                    for stmt in fallback {
                        let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(stmt.as_str()))
                            .execute(&mut *conn)
                            .await;
                    }
                }
                Ok(())
            })
        })
        .connect_with(opts)
        .await
        .map_err(types::upstream)
}

/// The MySQL `SET time_zone` value for a connection. Defaults to UTC
/// (`+00:00`); `UTC` is normalized to the offset form, anything else (offset
/// like `+03:00` or a named zone) is passed through.
/// The per-connection session setup as ONE statement (one round trip): the
/// session time zone plus cached `information_schema` statistics (avoids the
/// expensive per-table stats recomputation that slows the tree on big servers).
fn connect_setup_sql(tz: &str) -> String {
    format!(
        "SET time_zone = '{}', SESSION information_schema_stats_expiry = 86400",
        tz.replace('\'', "''")
    )
}

/// The same settings one per statement — used only when the combined `SET`
/// failed (MySQL applies a multi-assignment `SET` all-or-nothing).
fn connect_setup_fallback_sql(tz: &str) -> [String; 2] {
    [
        format!("SET time_zone = '{}'", tz.replace('\'', "''")),
        "SET SESSION information_schema_stats_expiry = 86400".to_string(),
    ]
}

fn mysql_session_tz(cfg: &ResolvedConfig) -> String {
    match cfg.param_str("timezone") {
        Some(tz) if !tz.eq_ignore_ascii_case("UTC") => tz,
        _ => "+00:00".to_string(),
    }
}

// --- Query execution --------------------------------------------------------

/// First-keyword detection of read (returns rows) vs write statements.
fn is_read_statement(statement: &str) -> bool {
    let kw = first_keyword(statement);
    matches!(
        kw.as_str(),
        "SELECT" | "SHOW" | "DESC" | "DESCRIBE" | "EXPLAIN" | "WITH"
    )
}

/// The first SQL keyword (uppercased), skipping leading whitespace and
/// line/block comments.
fn first_keyword(statement: &str) -> String {
    let mut s = statement.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            // Line comment: skip to end of line.
            s = rest
                .split_once('\n')
                .map(|x| x.1)
                .unwrap_or("")
                .trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            // Block comment: skip to closing.
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

/// Acquire a pooled session, mapping sqlx's pool timeout to a clear message.
async fn acquire(pool: &sqlx::MySqlPool) -> Result<sqlx::pool::PoolConnection<sqlx::MySql>> {
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

/// The ONE text-protocol round trip that scopes a pooled session for a
/// request and reads what cancel / the no-default-db check need:
///
/// - `db` → `USE` it and read `CONNECTION_ID()`;
/// - none → read `CONNECTION_ID()` and `DATABASE()` (the caller retires a
///   session that still carries a default database).
///
/// It used to be two or three trips (`USE` / `SELECT DATABASE()`, then
/// `SELECT CONNECTION_ID()`). Text protocol is required: MySQL rejects `USE`
/// as a prepared statement (see [`use_db_sql`]); sqlx enables
/// `CLIENT_MULTI_STATEMENTS`, so the pair travels as one `COM_QUERY`.
fn session_setup_sql(db: Option<&str>) -> String {
    match db {
        Some(db) => format!("{}; SELECT CONNECTION_ID(), DATABASE()", use_db_sql(db)),
        None => "SELECT CONNECTION_ID(), DATABASE()".to_string(),
    }
}

/// `CONNECTION_ID()` from a text-protocol row (decoded as whichever integer /
/// string form the server sent).
fn conn_id_of(row: &MySqlRow) -> Option<u64> {
    row.try_get::<u64, _>(0)
        .ok()
        .or_else(|| row.try_get::<i64, _>(0).ok().map(|n| n as u64))
        .or_else(|| {
            row.try_get::<String, _>(0)
                .ok()
                .and_then(|s| s.trim().parse().ok())
        })
}

/// The database a request runs in: the selected one, else the profile default
/// (what a FRESH session starts in — `build_pool` connects with it).
fn effective_db<'a>(active_db: Option<&'a str>, cfg: &'a ResolvedConfig) -> Option<&'a str> {
    active_db.or_else(|| cfg.database.as_deref().filter(|s| !s.is_empty()))
}

/// Acquire a pooled session and set its default database EXPLICITLY for this
/// request. Pooled sessions keep whatever an earlier request left — its `USE`
/// (ours, or one the user typed) — so a request without a selected database
/// used to run in whichever database the connection it happened to get had
/// last been switched to. Every user-SQL path acquires through here:
///
/// - `db` (the selected database, else the profile default) → `USE` it;
/// - neither → the session must have NO default database, like a fresh one.
///   MySQL cannot unselect one, so a session still carrying one is closed and
///   another taken (bounded by the pool size: each pass retires one).
async fn acquire_scoped(
    pool: &sqlx::MySqlPool,
    db: Option<&str>,
    token: Option<&CancelToken>,
) -> Result<sqlx::pool::PoolConnection<sqlx::MySql>> {
    let setup = session_setup_sql(db);
    let mut conn = acquire(pool).await?;
    for _ in 0..=POOL_MAX_CONNECTIONS {
        let rows = sqlx::raw_sql(sqlx::AssertSqlSafe(setup.as_str()))
            .fetch_all(&mut *conn)
            .await
            .map_err(types::upstream)?;
        let row = rows.last();
        // Whether the session still carries a default database: judged on the
        // raw NULL flag, so a value that won't decode as text can't pass as
        // "none".
        let has_default_db = row
            .and_then(|r| r.try_get_raw(1).ok())
            .is_some_and(|v| !sqlx::ValueRef::is_null(&v));
        if db.is_some() || !has_default_db {
            if let (Some(token), Some(id)) = (token, row.and_then(conn_id_of)) {
                token.set(QueryHandle::MysqlConnId(id));
            }
            return Ok(conn);
        }
        let _ = conn.close().await;
        conn = acquire(pool).await?;
    }
    Err(types::upstream(
        "mysql: could not obtain a session without a default database",
    ))
}

async fn run_read(
    pool: &sqlx::MySqlPool,
    statement: &str,
    max_rows: usize,
    server_bounded: bool,
    active_db: Option<&str>,
    token: &CancelToken,
) -> Result<QueryResult> {
    // Acquire a single connection so the `USE <db>` and the statement share
    // the same session — the default schema must apply to the query.
    // The backend id is captured in the same round trip as the `USE`, so a
    // concurrent cancel can `KILL QUERY <id>` it. Best-effort — a query with
    // no captured id simply can't be server-cancelled.
    let mut conn = acquire_scoped(pool, active_db, Some(token)).await?;
    // Reads leave no session state, except the explicit lock functions.
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
        // Rows left on the wire: discard the session instead of letting its
        // next use drain them. (A server-bounded read at the row cap drains
        // its leftover and keeps the session — DB2-01.)
        conn.close_on_drop();
    }
    Ok(out.result)
}

async fn run_write(
    pool: &sqlx::MySqlPool,
    statement: &str,
    active_db: Option<&str>,
    token: &CancelToken,
) -> Result<QueryResult> {
    // Same as run_read: `USE <db>` and the statement must share one session.
    let mut conn = acquire_scoped(pool, active_db, Some(token)).await?;
    // A `SET`/`BEGIN`/`LOCK`/… would outlive this request on a pooled session:
    // close it afterwards instead of returning it to the pool.
    if types::sql_leaves_session_state(statement) {
        conn.close_on_drop();
    }
    exec_write_conn(&mut conn, statement).await
}

/// Execute a true multi-statement batch (>1 statement) on ONE shared pooled
/// session, in order. The optional `USE <db>` and the captured backend id apply
/// once to the whole batch (so a concurrent cancel `KILL QUERY`s whichever
/// statement is running). Each statement's result carries its preview label; on
/// the first failing statement execution stops and an `errored` entry is appended
/// — the completed results are returned, not discarded (§2.2). Eligible SELECTs
/// are bounded at the server, without enabling the single-statement pager.
/// Non-rewritable reads preserve their SQL and session semantics; if clipped,
/// SQLx drains their remainder before the next statement on this same session.
async fn run_batch(
    pool: &sqlx::MySqlPool,
    spans: &[StatementSpan],
    max_rows: usize,
    active_db: Option<&str>,
    token: &CancelToken,
) -> Result<QueryResult> {
    let mut conn = acquire_scoped(pool, active_db, Some(token)).await?;
    if spans
        .iter()
        .any(|span| types::sql_leaves_session_state(&span.text))
    {
        conn.close_on_drop();
    }
    let mut results: Vec<QueryResult> = Vec::with_capacity(spans.len());
    // One budget for the whole response, shared by every statement's rows.
    let mut budget = types::ByteBudget::default();
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
                // A LATER statement would drain the unread rows anyway (same
                // session); the last one's are skipped by closing it.
                if out.unread {
                    conn.close_on_drop();
                }
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
                // Stop at the first failure; keep the completed results and flag
                // this one. The service returns 200 with the partial batch.
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

/// Rows per decode task handed to the blocking pool.
const DECODE_CHUNK: usize = 1024;
/// A tail of at most this many cells is decoded inline (a spawn costs more).
const INLINE_DECODE_CELLS: usize = 16 * 1024;

/// A shaped read plus whether rows were left UNREAD on the wire.
struct ReadOut {
    result: QueryResult,
    /// The read stopped at the row cap / byte budget before the server finished
    /// sending: the session must be discarded (`close_on_drop`), or sqlx drains
    /// every remaining row on the connection's next use.
    unread: bool,
}

/// Run a row-returning statement on an already-prepared connection (`USE` +
/// conn-id capture done by the caller) and shape the rows into a `QueryResult`,
/// capping at `max_rows` rows and at the response `budget`.
///
/// - Stops pulling at `max_rows + 1` (or the budget) instead of draining the
///   rest: a non-LIMIT-able read (UNION, a batch statement, SHOW…) past the cap
///   used to fetch and discard the WHOLE server result (~1.5 µs/row — a 50M-row
///   UNION took over a minute to show 1,000 rows). The caller closes the session
///   when `unread` — EXCEPT a `server_bounded` read (injected `LIMIT
///   max_rows+1`) stopped at the row cap: the probe row was the server's last,
///   so the reader drains the end-of-result packet ([`types::drain_leftover`])
///   and the session goes back to the pool. Without that, the default "open
///   table" view and every page closed its session and the next Run paid a
///   full reconnect (DB2-01).
/// - Ad-hoc SQL is NOT kept in the statement cache (`persistent(false)`): every
///   distinct statement/page used to be cached per session, evicting the
///   tree/completion statements that are actually reused (DB2-02).
/// - Each column's decoder is chosen ONCE from its type ([`CellDecoder`]), not by
///   trying up to 11 typed `try_get`s per cell (2.1–2.5 s → ~0.2 s CPU per
///   100k×30 rows) — which also stops text that merely LOOKS like JSON being
///   returned as JSON values (SH-02).
/// - Decoding runs in chunks on the blocking pool while the next rows stream in.
async fn exec_read_conn(
    conn: &mut sqlx::MySqlConnection,
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
    let mut decoders: std::sync::Arc<[CellDecoder]> = std::sync::Arc::from(Vec::new());
    let mut chunk: Vec<MySqlRow> = Vec::new();
    let mut decoding: Vec<tokio::task::JoinHandle<(Vec<Vec<Value>>, bool)>> = Vec::new();
    let mut kept = 0usize;
    let mut truncated = false;
    let mut truncated_reason = None;
    let mut unread = false;
    while let Some(row) = stream.try_next().await.map_err(types::upstream)? {
        if columns.is_empty() {
            for col in row.columns() {
                columns.push(Column::typed(col.name(), col.type_info().name()));
            }
            decoders = column_decoders(&row).into();
        }
        if kept >= max_rows {
            // Row max_rows+1 exists: the result is capped. Stop here — and,
            // when the server bounded the result, finish it so the session
            // stays poolable.
            truncated = true;
            unread = !(server_bounded
                && types::drain_leftover(&mut stream, types::LEFTOVER_DRAIN_ROWS).await);
            break;
        }
        // Budget on the raw wire size, per row, BEFORE decoding. Always keep
        // at least one row so a single huge row still shows.
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
                decode_rows(&rows, &dec)
            }));
        }
    }
    drop(stream);

    let mut out_rows: Vec<Vec<Value>> = Vec::with_capacity(kept);
    let mut cells_truncated = false;
    for task in decoding {
        let (rows, capped) = task
            .await
            .map_err(|e| otto_core::Error::Internal(format!("decode task failed: {e}")))?;
        out_rows.extend(rows);
        cells_truncated |= capped;
    }
    if !chunk.is_empty() {
        let (rows, capped) = if chunk.len() * decoders.len() <= INLINE_DECODE_CELLS {
            decode_rows(&chunk, &decoders)
        } else {
            let dec = decoders.clone();
            tokio::task::spawn_blocking(move || decode_rows(&chunk, &dec))
                .await
                .map_err(|e| otto_core::Error::Internal(format!("decode task failed: {e}")))?
        };
        out_rows.extend(rows);
        cells_truncated |= capped;
    }

    Ok(ReadOut {
        result: QueryResult {
            cells_truncated,
            columns,
            rows: out_rows,
            truncated,
            truncated_reason,
            ..QueryResult::empty()
        },
        unread,
    })
}

/// Decode fetched rows with their per-column decoders (capping oversized cells
/// like ClickHouse does — an uncapped multi-MB cell freezes the grid).
fn decode_rows(rows: &[MySqlRow], decoders: &[CellDecoder]) -> (Vec<Vec<Value>>, bool) {
    let mut cells_truncated = false;
    let rows = rows
        .iter()
        .map(|row| {
            decoders
                .iter()
                .enumerate()
                .map(|(i, dec)| {
                    types::cap_cell_tracked(mysql_cell(row, i, *dec), &mut cells_truncated)
                })
                .collect()
        })
        .collect();
    (rows, cells_truncated)
}

/// Estimated JSON size of a row, from its raw wire bytes (no decoding): text
/// is ~1:1, binary grows by base64's 4/3, fixed-width binary-protocol values
/// (ints, floats, dates) are counted at a typical rendered width.
fn raw_row_json_len(row: &MySqlRow, decoders: &[CellDecoder]) -> usize {
    let mut n = 2;
    for (i, dec) in decoders.iter().enumerate() {
        let raw = row
            .try_get_raw(i)
            .ok()
            .filter(|v| !sqlx::ValueRef::is_null(v))
            .and_then(|v| <&[u8] as sqlx::Decode<sqlx::MySql>>::decode(v).ok())
            .map(|b| b.len().min(types::MAX_CELL_CHARS));
        n += 3 + match (raw, dec) {
            (None, _) => 4,
            (Some(len), CellDecoder::Bytes) => len.div_ceil(3) * 4,
            (Some(len), CellDecoder::Text | CellDecoder::Json | CellDecoder::Fallback) => len,
            (Some(len), _) => len.max(24),
        };
    }
    n
}

/// Run a write/DDL statement on an already-prepared connection and return the
/// affected-row acknowledgement.
async fn exec_write_conn(conn: &mut sqlx::MySqlConnection, statement: &str) -> Result<QueryResult> {
    let res = sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(&mut *conn)
        .await
        .map_err(types::upstream)?;
    let affected = res.rows_affected();
    let mut result = QueryResult::message(format!("{affected} row(s) affected"));
    result.rows_affected = Some(affected);
    Ok(result)
}

/// How one MySQL result column's cells decode, chosen ONCE per column from its
/// type name (sqlx `MySqlTypeInfo::name()`), so each cell pays exactly one
/// typed `try_get` — every failed `try_get` allocates a formatted mismatch
/// error, and text columns used to fail 10 of them per cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellDecoder {
    /// Signed integers (incl. `BOOLEAN` = TINYINT(1)).
    Int,
    /// `… UNSIGNED` integers.
    UInt,
    Float,
    /// DECIMAL/NUMERIC → exact string.
    Decimal,
    /// The native `JSON` type — the ONLY column whose text is parsed as JSON.
    Json,
    DateTime,
    Timestamp,
    Date,
    Time,
    /// CHAR/VARCHAR/TEXT/ENUM (non-binary collation) → the string AS IS.
    Text,
    /// BINARY/VARBINARY/BLOB → base64.
    Bytes,
    /// Anything else (YEAR, BIT, SET, GEOMETRY, NULL, future types): the typed
    /// cascade, then raw text / base64.
    Fallback,
}

/// The decoder for a column type name (see [`CellDecoder`]).
fn cell_decoder(type_name: &str) -> CellDecoder {
    use CellDecoder::*;
    match type_name {
        "BOOLEAN" | "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => Int,
        "TINYINT UNSIGNED" | "SMALLINT UNSIGNED" | "MEDIUMINT UNSIGNED" | "INT UNSIGNED"
        | "BIGINT UNSIGNED" => UInt,
        "FLOAT" | "DOUBLE" => Float,
        "DECIMAL" => Decimal,
        "JSON" => Json,
        "DATETIME" => DateTime,
        "TIMESTAMP" => Timestamp,
        "DATE" => Date,
        "TIME" => Time,
        "CHAR" | "VARCHAR" | "TINYTEXT" | "TEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" => Text,
        "BINARY" | "VARBINARY" | "TINYBLOB" | "BLOB" | "MEDIUMBLOB" | "LONGBLOB" => Bytes,
        _ => Fallback,
    }
}

/// One decoder per column of `row` (every row of a result shares the columns).
fn column_decoders(row: &MySqlRow) -> Vec<CellDecoder> {
    row.columns()
        .iter()
        .map(|c| cell_decoder(c.type_info().name()))
        .collect()
}

/// Decode one cell with its column's decoder. A decoder that unexpectedly
/// rejects the value falls back to the typed cascade — never to Null.
fn mysql_cell(row: &MySqlRow, idx: usize, dec: CellDecoder) -> Value {
    use sqlx::types::chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
    let hit = match dec {
        CellDecoder::Int => row.try_get::<Option<i64>, _>(idx).map(int_to_json).ok(),
        CellDecoder::UInt => row
            .try_get::<Option<u64>, _>(idx)
            .map(|v| v.map(types::u64_to_json).unwrap_or(Value::Null))
            .ok(),
        CellDecoder::Float => row.try_get::<Option<f64>, _>(idx).map(float_to_json).ok(),
        CellDecoder::Decimal => row
            .try_get::<Option<sqlx::types::BigDecimal>, _>(idx)
            .map(|v| {
                v.map(|n| Value::String(n.to_string()))
                    .unwrap_or(Value::Null)
            })
            .ok(),
        CellDecoder::Json => row
            .try_get::<Option<Value>, _>(idx)
            .map(|v| v.unwrap_or(Value::Null))
            .ok(),
        CellDecoder::DateTime => row
            .try_get::<Option<NaiveDateTime>, _>(idx)
            .map(temporal_to_json)
            .ok(),
        CellDecoder::Timestamp => row
            .try_get::<Option<DateTime<Utc>>, _>(idx)
            .map(|v| temporal_to_json(v.map(|d| d.naive_utc())))
            .ok(),
        CellDecoder::Date => row
            .try_get::<Option<NaiveDate>, _>(idx)
            .map(temporal_to_json)
            .ok(),
        CellDecoder::Time => row
            .try_get::<Option<NaiveTime>, _>(idx)
            .map(temporal_to_json)
            .ok(),
        CellDecoder::Text => row
            .try_get::<Option<String>, _>(idx)
            .map(string_to_json)
            .ok(),
        CellDecoder::Bytes => row
            .try_get::<Option<Vec<u8>>, _>(idx)
            .map(|v| {
                v.map(|b| Value::String(B64.encode(b)))
                    .unwrap_or(Value::Null)
            })
            .ok(),
        CellDecoder::Fallback => None,
    };
    hit.unwrap_or_else(|| mysql_value_to_json(row, idx))
}

/// Decode a single cell of a MySQL row to a `serde_json::Value`, trying a
/// sequence of native types and finally falling back to base64 bytes or Null.
/// The per-cell fallback for [`CellDecoder::Fallback`] columns; ordinary
/// columns go through [`mysql_cell`]. Never parses text as JSON: sqlx's JSON
/// type accepts EVERY string/blob column, so a VARCHAR holding `null`, `123`,
/// `true` or a 30-digit id came back as JSON null/number/bool (SH-02) — only a
/// native `JSON` column ([`CellDecoder::Json`]) is parsed.
fn mysql_value_to_json(row: &MySqlRow, idx: usize) -> Value {
    // Integers (covers TINYINT..BIGINT, signed). Try i64 first.
    if let Ok(v) = row.try_get::<Option<i64>, _>(idx) {
        return int_to_json(v);
    }
    // Unsigned BIGINT (exact digits beyond 2^53 — see `types::u64_to_json`).
    if let Ok(v) = row.try_get::<Option<u64>, _>(idx) {
        return v.map(types::u64_to_json).unwrap_or(Value::Null);
    }
    // Floating point / decimal-as-f64.
    if let Ok(v) = row.try_get::<Option<f64>, _>(idx) {
        return float_to_json(v);
    }
    // Booleans (TINYINT(1)).
    if let Ok(v) = row.try_get::<Option<bool>, _>(idx) {
        return v.map(Value::Bool).unwrap_or(Value::Null);
    }
    // JSON columns decode straight to a Value — gated on the native type:
    // sqlx's JSON `compatible()` also accepts every text/blob column.
    if row.columns()[idx].type_info().name() == "JSON" {
        if let Ok(Some(val)) = row.try_get::<Option<Value>, _>(idx) {
            return val;
        }
    }
    // NUMERIC / DECIMAL → exact string (never lossy f64). sqlx's f64 branch
    // explicitly EXCLUDES Decimal columns (`real_compatible` matches only
    // Float|Double) and its String/Vec<u8> gates lack Decimal|NewDecimal, so
    // without this branch a DECIMAL cell — i.e. every money/balance column —
    // fell through to Null. Same class as the DATETIME fix above.
    if let Ok(v) = row.try_get::<Option<sqlx::types::BigDecimal>, _>(idx) {
        return v
            .map(|n| Value::String(n.to_string()))
            .unwrap_or(Value::Null);
    }
    // Temporal types (DATETIME / TIMESTAMP / DATE / TIME). sqlx's MySQL driver
    // does NOT decode these as String or Vec<u8> — their `Type::compatible` check
    // rejects the temporal SQL types — so without an explicit chrono attempt they
    // fall through to Null (the old "text covers date/time" assumption was wrong,
    // which is why raw `SELECT *` datetime columns rendered as null). Mirror the
    // Postgres driver and shape to an ISO-ish string. The compatibility gate means
    // these only fire for real temporal columns; a VARCHAR holding a date-like
    // string still falls through to the String branch below.
    use sqlx::types::chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
    if let Ok(v) = row.try_get::<Option<NaiveDateTime>, _>(idx) {
        return temporal_to_json(v);
    }
    // TIMESTAMP specifically: sqlx's `NaiveDateTime` impl has no custom
    // `compatible()` — the default only accepts DATETIME — so TIMESTAMP columns
    // rejected every attempt above and fell into the raw fallback, rendering
    // the binary wire payload as base64 ("B+oHBwYQOQA="). Only `DateTime<Utc>`
    // (and Local) accepts ColumnType::Timestamp. Render the naive part: it is
    // byte-for-byte what the server sent in the session time zone, so the text
    // matches the server's own display regardless of the profile's tz param.
    if let Ok(v) = row.try_get::<Option<DateTime<Utc>>, _>(idx) {
        return temporal_to_json(v.map(|d| d.naive_utc()));
    }
    if let Ok(v) = row.try_get::<Option<NaiveDate>, _>(idx) {
        return temporal_to_json(v);
    }
    if let Ok(v) = row.try_get::<Option<NaiveTime>, _>(idx) {
        return temporal_to_json(v);
    }
    // Text (covers VARCHAR/TEXT/ENUM and the CAST-to-CHAR helper queries).
    if let Ok(v) = row.try_get::<Option<String>, _>(idx) {
        return string_to_json(v);
    }
    // Raw bytes -> base64 string.
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(idx) {
        return match v {
            Some(bytes) => Value::String(B64.encode(bytes)),
            None => Value::Null,
        };
    }
    // Last resort — NEVER silently render a non-NULL cell as Null. Any column
    // type every typed attempt above rejected (future/unknown types) renders as
    // its raw text when UTF-8, else base64. Only a true SQL NULL stays Null.
    raw_cell_fallback(row.try_get_raw(idx))
}

/// Decode-of-last-resort for a raw MySQL value: SQL NULL → Null, UTF-8 payload
/// → its text, binary payload → base64. Uses `try_decode_unchecked` — the
/// checked decode would re-reject through the very `compatible()` gates that
/// routed us here. Errors (no such column) → Null.
fn raw_cell_fallback(
    raw: std::result::Result<sqlx::mysql::MySqlValueRef<'_>, sqlx::Error>,
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

/// Pure shaping of an optional integer into a JSON value (Null if absent);
/// beyond ±(2^53 − 1) its exact digits as a string (see `types::i64_to_json`).
fn int_to_json(v: Option<i64>) -> Value {
    v.map(types::i64_to_json).unwrap_or(Value::Null)
}

/// Pure shaping of an optional string into a JSON value (Null if absent).
fn string_to_json(v: Option<String>) -> Value {
    v.map(Value::String).unwrap_or(Value::Null)
}

/// Pure shaping of an optional temporal value (DATETIME/DATE/TIME decoded via
/// chrono) into a JSON string via its `Display` — `NaiveDateTime` renders as
/// `YYYY-MM-DD HH:MM:SS` (MySQL's native form), `NaiveDate`/`NaiveTime` as their
/// date/time parts. Null if absent.
fn temporal_to_json<T: ToString>(v: Option<T>) -> Value {
    v.map(|t| Value::String(t.to_string()))
        .unwrap_or(Value::Null)
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

/// Escape a backtick identifier for interpolation into `\`db\`.\`tbl\`` (the
/// only unavoidable interpolation — these come from the schema tree, not user
/// input, but we still double any backticks defensively).
fn esc_ident(ident: &str) -> String {
    ident.replace('`', "``")
}

/// Select the active database on a pooled connection before running a query.
///
/// MUST use the simple/TEXT query protocol (`sqlx::raw_sql`), NOT the prepared-
/// statement protocol (`sqlx::query`): MySQL rejects `USE` (and a handful of
/// other commands) when sent as a prepared statement, with
/// `1295 (HY000): This command is not supported in the prepared statement
/// protocol yet`. The text protocol runs it as a plain command, which is the
/// only way `USE` works. This is why the active-DB scoping is built as a raw
/// (text-protocol) statement; see [`use_db_sql`] for the SQL it runs.
///
/// Returns the `USE` statement for `db`, escaped. Callers run it via
/// `sqlx::raw_sql(..)` (the simple/text protocol) inline — a shared async helper
/// taking the connection trips the `Executor`-HRTB / `async_trait` Send bound,
/// so we keep the one-liner at each call site and only share the SQL.
fn use_db_sql(db: &str) -> String {
    format!("USE `{}`", esc_ident(db))
}

/// Read an integer column that MySQL may return as a signed/unsigned int or a
/// string (driver-dependent for SHOW output).
fn try_get_int(row: &MySqlRow, name: &str) -> Option<i64> {
    if let Ok(v) = row.try_get::<i64, _>(name) {
        return Some(v);
    }
    if let Ok(v) = row.try_get::<u64, _>(name) {
        return Some(v as i64);
    }
    if let Ok(v) = row.try_get::<String, _>(name) {
        return v.trim().parse().ok();
    }
    None
}

// --- Introspection row structs ----------------------------------------------

#[derive(sqlx::FromRow)]
struct ColumnRow {
    column_name: String,
    data_type: String,
    column_type: String,
    is_nullable: String,
    column_default: Option<String>,
    column_key: Option<String>,
    extra: Option<String>,
    column_comment: Option<String>,
    collation_name: Option<String>,
}

#[derive(sqlx::FromRow)]
struct RoutineParamRow {
    ordinal_position: i64,
    parameter_mode: Option<String>,
    parameter_name: Option<String>,
    dtd_identifier: Option<String>,
}

#[derive(sqlx::FromRow)]
struct TriggerRow {
    event_manipulation: Option<String>,
    action_timing: Option<String>,
    event_object_table: Option<String>,
}

#[derive(sqlx::FromRow)]
struct FkRow {
    constraint_name: String,
    column_name: String,
    referenced_table_schema: Option<String>,
    referenced_table_name: Option<String>,
    referenced_column_name: Option<String>,
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
    "INDEX",
    "DATABASE",
    "SCHEMA",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "OUTER",
    "CROSS",
    "ON",
    "USING",
    "GROUP",
    "BY",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "AS",
    "DISTINCT",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS",
    "IN",
    "LIKE",
    "BETWEEN",
    "EXISTS",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "UNION",
    "ALL",
    "ANY",
    "ASC",
    "DESC",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "UNIQUE",
    "DEFAULT",
    "AUTO_INCREMENT",
    "CONSTRAINT",
    "WITH",
    "RECURSIVE",
    "TRUNCATE",
    "REPLACE",
    "IGNORE",
    "DUPLICATE",
    "INTERVAL",
    "CAST",
    "CONVERT",
    "USE",
    "SHOW",
    "DESCRIBE",
    "EXPLAIN",
    "GRANT",
    "REVOKE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "TRANSACTION",
    "ENGINE",
    "CHARSET",
    "COLLATE",
    "TEMPORARY",
    "IF",
];

const FUNCTIONS: &[(&str, &str)] = &[
    ("COUNT", "COUNT(expr) — number of rows"),
    ("SUM", "SUM(expr) — sum of values"),
    ("AVG", "AVG(expr) — average of values"),
    ("MIN", "MIN(expr) — minimum value"),
    ("MAX", "MAX(expr) — maximum value"),
    (
        "GROUP_CONCAT",
        "GROUP_CONCAT(expr) — concatenated group values",
    ),
    ("CONCAT", "CONCAT(str1, str2, ...) — concatenate strings"),
    (
        "CONCAT_WS",
        "CONCAT_WS(sep, str1, ...) — concat with separator",
    ),
    ("SUBSTRING", "SUBSTRING(str, pos, len) — substring"),
    ("SUBSTR", "SUBSTR(str, pos, len) — substring"),
    ("LENGTH", "LENGTH(str) — byte length"),
    ("CHAR_LENGTH", "CHAR_LENGTH(str) — character length"),
    ("UPPER", "UPPER(str) — uppercase"),
    ("LOWER", "LOWER(str) — lowercase"),
    ("TRIM", "TRIM(str) — strip whitespace"),
    ("LTRIM", "LTRIM(str) — strip leading whitespace"),
    ("RTRIM", "RTRIM(str) — strip trailing whitespace"),
    ("REPLACE", "REPLACE(str, from, to) — replace substring"),
    ("LEFT", "LEFT(str, len) — leftmost chars"),
    ("RIGHT", "RIGHT(str, len) — rightmost chars"),
    ("LPAD", "LPAD(str, len, pad) — left pad"),
    ("RPAD", "RPAD(str, len, pad) — right pad"),
    ("LOCATE", "LOCATE(substr, str) — position of substring"),
    ("INSTR", "INSTR(str, substr) — position of substring"),
    ("FORMAT", "FORMAT(num, decimals) — formatted number"),
    ("ROUND", "ROUND(num, decimals) — round"),
    ("FLOOR", "FLOOR(num) — round down"),
    ("CEIL", "CEIL(num) — round up"),
    ("CEILING", "CEILING(num) — round up"),
    ("ABS", "ABS(num) — absolute value"),
    ("MOD", "MOD(a, b) — modulo"),
    ("POW", "POW(base, exp) — power"),
    ("POWER", "POWER(base, exp) — power"),
    ("SQRT", "SQRT(num) — square root"),
    ("RAND", "RAND() — random 0..1"),
    ("NOW", "NOW() — current datetime"),
    ("CURDATE", "CURDATE() — current date"),
    ("CURTIME", "CURTIME() — current time"),
    (
        "CURRENT_TIMESTAMP",
        "CURRENT_TIMESTAMP() — current datetime",
    ),
    ("UNIX_TIMESTAMP", "UNIX_TIMESTAMP(date) — epoch seconds"),
    ("FROM_UNIXTIME", "FROM_UNIXTIME(ts) — datetime from epoch"),
    ("DATE", "DATE(expr) — date part"),
    ("TIME", "TIME(expr) — time part"),
    ("YEAR", "YEAR(date) — year"),
    ("MONTH", "MONTH(date) — month"),
    ("DAY", "DAY(date) — day of month"),
    ("HOUR", "HOUR(time) — hour"),
    ("MINUTE", "MINUTE(time) — minute"),
    ("SECOND", "SECOND(time) — second"),
    ("DATE_ADD", "DATE_ADD(date, INTERVAL n unit) — add interval"),
    (
        "DATE_SUB",
        "DATE_SUB(date, INTERVAL n unit) — subtract interval",
    ),
    ("DATEDIFF", "DATEDIFF(d1, d2) — days between"),
    ("DATE_FORMAT", "DATE_FORMAT(date, fmt) — format date"),
    ("COALESCE", "COALESCE(a, b, ...) — first non-null"),
    ("IFNULL", "IFNULL(expr, alt) — alt if null"),
    ("NULLIF", "NULLIF(a, b) — null if equal"),
    ("IF", "IF(cond, a, b) — conditional"),
    ("GREATEST", "GREATEST(a, b, ...) — largest value"),
    ("LEAST", "LEAST(a, b, ...) — smallest value"),
    ("CAST", "CAST(expr AS type) — type cast"),
    ("CONVERT", "CONVERT(expr, type) — type conversion"),
    (
        "JSON_EXTRACT",
        "JSON_EXTRACT(json, path) — extract from JSON",
    ),
    ("JSON_OBJECT", "JSON_OBJECT(k, v, ...) — build JSON object"),
    ("JSON_ARRAY", "JSON_ARRAY(v, ...) — build JSON array"),
    ("MD5", "MD5(str) — MD5 hash"),
    ("SHA2", "SHA2(str, bits) — SHA-2 hash"),
    ("UUID", "UUID() — generate UUID"),
];

// --- Unit tests -------------------------------------------------------------

/// Native read-only transaction protects against effects hidden in a read
/// expression. RAII rollback also cleans up cancelled or failed reads.
async fn governed_read(
    pool: &sqlx::MySqlPool,
    cfg: &ResolvedConfig,
    req: &QueryRequest,
    token: &CancelToken,
) -> Result<QueryResult> {
    let scope_db = req.scope_database();
    let mut conn =
        acquire_scoped(pool, effective_db(scope_db.as_deref(), cfg), Some(token)).await?;
    let mut tx = conn
        .begin_with("START TRANSACTION READ ONLY")
        .await
        .map_err(types::upstream)?;
    let spans = split_statements(req.statement.trim(), SqlDialect::Mysql);
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
        let limited = types::inject_row_limit(
            &span.text,
            max_rows.saturating_add(1),
            if single { req.offset } else { None },
        );
        let mut sql = limited.sql;
        if let Some(ms) = req.timeout_ms.filter(|ms| *ms > 0) {
            if sql.trim_start().to_uppercase().starts_with("SELECT") {
                sql = sql.replacen(
                    "SELECT",
                    &format!("SELECT /*+ MAX_EXECUTION_TIME({ms}) */"),
                    1,
                );
            }
        }
        let bounded = limited.limited;
        let out = match exec_read_conn(&mut tx, &sql, max_rows, bounded, &mut budget).await {
            Ok(out) => out,
            // A batch keeps its completed results and flags the failing
            // statement (same contract as `run_batch`); a single statement's
            // failure is the request's error.
            Err(e) if !single => {
                results.push(types::errored_batch_entry(
                    types::statement_preview(&span.text),
                    e.to_string(),
                ));
                break;
            }
            Err(e) => return Err(e),
        };
        unread |= out.unread;
        let mut result = out.result;
        result.stats.duration_ms = started.elapsed().as_millis() as u64;
        result.stats.row_count = result.rows.len();
        if single {
            result.auto_limited = limited.limited.then_some(max_rows as u64)
        } else {
            result.statement = Some(types::statement_preview(&span.text));
        }
        results.push(result);
    }
    if unread {
        // A ROLLBACK would first drain the unread rows. Drop the transaction
        // (its rollback is queued) and discard the session: the server rolls
        // the read-only transaction back when the connection closes.
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

    /// SH-01/SH-02: the decoder is picked from the column type, and ONLY the
    /// native JSON type is parsed as JSON — text that looks like JSON (`null`,
    /// `123`, `true`, a 30-digit id) must come back as the string it is.
    #[test]
    fn cell_decoder_is_chosen_by_column_type() {
        use CellDecoder::*;
        for (name, want) in [
            ("BOOLEAN", Int),
            ("TINYINT", Int),
            ("SMALLINT", Int),
            ("MEDIUMINT", Int),
            ("INT", Int),
            ("BIGINT", Int),
            ("TINYINT UNSIGNED", UInt),
            ("INT UNSIGNED", UInt),
            ("BIGINT UNSIGNED", UInt),
            ("FLOAT", Float),
            ("DOUBLE", Float),
            ("DECIMAL", Decimal),
            ("JSON", Json),
            ("DATETIME", DateTime),
            ("TIMESTAMP", Timestamp),
            ("DATE", Date),
            ("TIME", Time),
            ("CHAR", Text),
            ("VARCHAR", Text),
            ("TINYTEXT", Text),
            ("TEXT", Text),
            ("MEDIUMTEXT", Text),
            ("LONGTEXT", Text),
            ("ENUM", Text),
            ("BINARY", Bytes),
            ("VARBINARY", Bytes),
            ("BLOB", Bytes),
            ("LONGBLOB", Bytes),
            ("YEAR", Fallback),
            ("BIT", Fallback),
            ("SET", Fallback),
            ("GEOMETRY", Fallback),
            ("NULL", Fallback),
            ("SOMETHING NEW", Fallback),
        ] {
            assert_eq!(cell_decoder(name), want, "{name}");
        }
        // No text/binary type may reach the JSON parser.
        for name in [
            "CHAR",
            "VARCHAR",
            "TEXT",
            "LONGTEXT",
            "ENUM",
            "BLOB",
            "VARBINARY",
        ] {
            assert_ne!(cell_decoder(name), Json, "{name} must not decode as JSON");
        }
    }

    #[test]
    fn capabilities_are_honest() {
        let c = MysqlDriver::default().capabilities();
        assert_eq!(c.engine, Engine::Mysql);
        assert!(c.sql && c.joins && c.multi_statement);
        // Pooled connections ⇒ no session-pinned transactions (was over-promised).
        assert!(!c.transactions);
        // Server-side cancel (KILL QUERY) + EXPLAIN both supported.
        assert!(c.cancel);
        assert!(c.explain);
    }

    #[test]
    fn detects_read_statements() {
        assert!(is_read_statement("SELECT 1"));
        assert!(is_read_statement("  select * from t"));
        assert!(is_read_statement("SHOW TABLES"));
        assert!(is_read_statement("DESC users"));
        assert!(is_read_statement("DESCRIBE users"));
        assert!(is_read_statement("EXPLAIN SELECT 1"));
        assert!(is_read_statement(
            "WITH cte AS (SELECT 1) SELECT * FROM cte"
        ));
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
    fn first_keyword_skips_comments() {
        assert_eq!(first_keyword("-- a comment\nSELECT 1"), "SELECT");
        assert_eq!(first_keyword("/* block */ UPDATE t"), "UPDATE");
        assert_eq!(first_keyword("   \n  select"), "SELECT");
    }

    #[test]
    fn esc_ident_doubles_backticks() {
        assert_eq!(esc_ident("plain"), "plain");
        assert_eq!(esc_ident("we`ird"), "we``ird");
    }

    #[test]
    fn value_shaping_ints_strings_null() {
        // Ints.
        assert_eq!(int_to_json(Some(42)), serde_json::json!(42));
        assert_eq!(int_to_json(Some(-7)), serde_json::json!(-7));
        assert_eq!(int_to_json(None), Value::Null);
        // Strings.
        assert_eq!(
            string_to_json(Some("ada@example.com".into())),
            serde_json::json!("ada@example.com")
        );
        assert_eq!(string_to_json(None), Value::Null);
        // Floats (and non-finite -> Null).
        assert_eq!(float_to_json(Some(1.5)), serde_json::json!(1.5));
        assert_eq!(float_to_json(None), Value::Null);
        assert_eq!(float_to_json(Some(f64::NAN)), Value::Null);
    }

    #[test]
    fn value_shaping_temporal() {
        use sqlx::types::chrono::{NaiveDate, NaiveDateTime, NaiveTime};
        // DATETIME → "YYYY-MM-DD HH:MM:SS" (space separator, MySQL's native form).
        let dt = NaiveDate::from_ymd_opt(2024, 1, 15)
            .unwrap()
            .and_hms_opt(10, 30, 0)
            .unwrap();
        assert_eq!(
            temporal_to_json(Some(dt)),
            serde_json::json!("2024-01-15 10:30:00")
        );
        // DATE → "YYYY-MM-DD", TIME → "HH:MM:SS".
        assert_eq!(
            temporal_to_json(Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap())),
            serde_json::json!("2024-01-15")
        );
        assert_eq!(
            temporal_to_json(Some(NaiveTime::from_hms_opt(10, 30, 0).unwrap())),
            serde_json::json!("10:30:00")
        );
        // Absent → Null (temporal column with NULL value).
        assert_eq!(temporal_to_json::<NaiveDateTime>(None), Value::Null);
    }
}

#[cfg(test)]
mod cache_isolation_tests {
    use super::*;
    use std::sync::Arc;

    /// A request without a selected database runs in the PROFILE default —
    /// set explicitly with `USE`, never whatever a pooled session was last
    /// switched to — and a selection always wins.
    #[test]
    fn effective_db_is_selection_then_profile_default() {
        let mut cfg = ResolvedConfig {
            lifecycle: None,
            engine: Engine::Mysql,
            host: "127.0.0.1".into(),
            port: 3306,
            user: None,
            password: None,
            database: Some("shop".into()),
            tls: Default::default(),
            params: serde_json::json!({}),
        };
        assert_eq!(effective_db(Some("analytics"), &cfg), Some("analytics"));
        assert_eq!(effective_db(None, &cfg), Some("shop"));
        cfg.database = Some(String::new());
        assert_eq!(effective_db(None, &cfg), None);
        cfg.database = None;
        assert_eq!(effective_db(None, &cfg), None);
    }

    /// DB-02 round-trip guard: scoping a pooled session and capturing its
    /// cancel id is ONE text-protocol `COM_QUERY` (it used to be `USE` or
    /// `SELECT DATABASE()`, then `SELECT CONNECTION_ID()`).
    #[test]
    fn per_run_setup_is_one_round_trip() {
        assert_eq!(
            session_setup_sql(Some("sh`op")),
            "USE `sh``op`; SELECT CONNECTION_ID(), DATABASE()"
        );
        assert_eq!(
            session_setup_sql(None),
            "SELECT CONNECTION_ID(), DATABASE()"
        );
        for sql in [session_setup_sql(Some("x")), session_setup_sql(None)] {
            assert_eq!(sql.matches("SELECT").count(), 1, "{sql}");
        }
    }

    /// DB2-01 round-trip guard: a NEW connection's setup is ONE `SET` (it was
    /// two sequential prepared statements), escaped, with a per-variable
    /// fallback for servers that reject one of the settings.
    #[test]
    fn connect_setup_is_one_statement() {
        let sql = connect_setup_sql("Europe/O'X");
        assert_eq!(
            sql,
            "SET time_zone = 'Europe/O''X', SESSION information_schema_stats_expiry = 86400"
        );
        assert_eq!(sql.matches("SET ").count(), 1, "{sql}");
        assert!(
            !sql.contains(';'),
            "one statement, no multi-statement batch"
        );
        let [tz, stats] = connect_setup_fallback_sql("+00:00");
        assert_eq!(tz, "SET time_zone = '+00:00'");
        assert!(stats.contains("information_schema_stats_expiry"));
    }

    #[test]
    fn session_changing_statements_retire_the_pooled_session() {
        for sql in [
            "USE other",
            "SET autocommit = 0",
            "SET sql_safe_updates = 0",
            "START TRANSACTION",
            "BEGIN",
            "LOCK TABLES t WRITE",
            "CREATE TEMPORARY TABLE t (id INT)",
            "SELECT GET_LOCK('x', 10)",
            "CALL refresh_stats()",
            "SELECT id INTO @last FROM t LIMIT 1",
            "SELECT @n := COUNT(*) FROM t",
        ] {
            assert!(types::sql_leaves_session_state(sql), "{sql}");
        }
        for sql in [
            "SELECT * FROM t",
            "DELETE FROM t WHERE id = 1",
            "SHOW TABLES",
            "SELECT into_count FROM t",
            "INSERT INTO t VALUES (1)",
        ] {
            assert!(!types::sql_leaves_session_state(sql), "{sql}");
        }
    }

    #[tokio::test]
    async fn cache_warm_mysql_pool_does_not_wait_for_other_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg = ResolvedConfig {
            lifecycle: None,
            engine: Engine::Mysql,
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            user: Some("fixture".into()),
            password: None,
            database: None,
            tls: Default::default(),
            params: serde_json::json!({}),
        };
        let mut warm_cfg = cfg.clone();
        warm_cfg.port = 1;
        let warm = sqlx::mysql::MySqlPoolOptions::new()
            .connect_lazy("mysql://fixture@127.0.0.1:1")
            .unwrap();
        let driver = Arc::new(MysqlDriver::default());
        driver.pools.insert_ready(warm_cfg.cache_key(), warm);
        let slow_driver = driver.clone();
        let slow = tokio::spawn(async move { slow_driver.pool(&cfg).await });
        let (_socket, _) = listener.accept().await.unwrap();
        // A has started its handshake and the fixture deliberately never replies.
        // B must finish BEFORE A, not merely become a little faster.
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(2), driver.pool(&warm_cfg)).await;
        slow.abort();
        let _ = slow.await;
        assert!(
            result.is_ok(),
            "warm B waited behind unrelated pending handshake A"
        );
        assert!(result.unwrap().is_ok());
    }
}
