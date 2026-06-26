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

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use otto_core::Result;
use serde_json::Value;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlRow, MySqlSslMode};
use sqlx::{Column as _, Executor as _, Row, TypeInfo};
use tokio::sync::Mutex;

use crate::driver::Driver;
use crate::export::{open_sink, ExportCounts, ExportFormat};
use crate::tls::TlsFiles;
use crate::types::{
    self, Capabilities, CancelToken, Column, ColumnDef, CompletionContext, CompletionResponse,
    Engine, ForeignKey, IndexDef, NodeKind, NodePath, ObjectDetail, QueryHandle, QueryRequest,
    QueryResult, QueryStats, ResolvedConfig, SchemaNode, TestResult,
};

const DEFAULT_MAX_ROWS: usize = 1000;
/// Max pooled connections per cached config. Small — the DB Explorer issues
/// short, mostly-serial introspection/query calls per connection.
const POOL_MAX_CONNECTIONS: u32 = 4;
/// Drop idle pooled connections after this long so a long-lived cached pool
/// doesn't pin server-side connections forever.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// MySQL driver. Holds a per-`cache_key` pool cache so connections are reused
/// across calls instead of re-handshaking every time. `Mutex<HashMap>` is
/// `Default`-constructible, so `#[derive(Default)]` (used by the registry)
/// still works.
#[derive(Default)]
pub struct MysqlDriver {
    pools: Mutex<HashMap<String, sqlx::MySqlPool>>,
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
            transactions: true,
            multi_statement: true,
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
        let version: String = match sqlx::query_scalar("SELECT VERSION()").fetch_one(&pool).await {
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
            return self.objects_in_folder(cfg, &db, folder, filter).await;
        }

        // db:<n> -> the two folders (Tables, Views); no per-folder filter at this level.
        let tables = SchemaNode::new(
            parent.child("folder", "tables").to_id(),
            "Tables",
            NodeKind::Folder,
        )
        .expandable();
        let views = SchemaNode::new(
            parent.child("folder", "views").to_id(),
            "Views",
            NodeKind::Folder,
        )
        .expandable();
        Ok(vec![tables, views])
    }

    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
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

        // Columns from information_schema.
        let col_rows: Vec<ColumnRow> = sqlx::query_as(
            // CAST text columns to CHAR (MySQL 8 returns information_schema text
            // as VARBINARY); keep the original names so FromRow still matches.
            "SELECT CAST(column_name AS CHAR) AS column_name, \
                    CAST(data_type AS CHAR) AS data_type, \
                    CAST(column_type AS CHAR) AS column_type, \
                    CAST(is_nullable AS CHAR) AS is_nullable, \
                    CAST(column_default AS CHAR) AS column_default, \
                    CAST(column_key AS CHAR) AS column_key, \
                    CAST(extra AS CHAR) AS extra, \
                    CAST(column_comment AS CHAR) AS column_comment \
             FROM information_schema.columns \
             WHERE table_schema = ? AND table_name = ? \
             ORDER BY ordinal_position",
        )
        .bind(&db)
        .bind(&table)
        .fetch_all(&pool)
        .await
        .map_err(types::upstream)?;

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
            });
        }

        // Indexes via SHOW INDEX, grouped by Key_name (ordered by Seq_in_index).
        let indexes = self.indexes_of(&pool, &db, &table).await?;

        // Foreign keys (CRITICAL for the JOIN builder).
        let foreign_keys = self.foreign_keys_of(&pool, &db, &table).await?;

        // Row count is intentionally left empty: the only cheap source here is
        // information_schema.tables.table_rows, which is an InnoDB *estimate*
        // (often wildly off) — the user doesn't want estimated counts shown.

        // DDL via SHOW CREATE TABLE / VIEW (2nd column).
        let ddl = self.show_create(&pool, &db, &table, is_view).await.ok();

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

    async fn run_tracked(
        &self,
        cfg: &ResolvedConfig,
        req: &QueryRequest,
        token: &CancelToken,
    ) -> Result<QueryResult> {
        let statement = req.statement.trim();
        if statement.is_empty() {
            return Err(types::invalid("empty statement"));
        }
        let max_rows = req.max_rows.unwrap_or(DEFAULT_MAX_ROWS);
        let pool = self.pool(cfg).await?;
        let started = Instant::now();

        // The active database (if the user selected one) scopes unqualified
        // table names: we `USE` it on the connection before running the query.
        let active_db = req.node.as_deref().map(str::trim).filter(|s| !s.is_empty());

        let result = if is_read_statement(statement) {
            let limited = types::inject_row_limit(statement, max_rows.saturating_add(1));
            // Inject MySQL's MAX_EXECUTION_TIME(ms) optimizer hint when a per-statement
            // timeout is requested. The hint goes right after the SELECT keyword so it
            // is valid even after LIMIT injection. Non-SELECT reads (e.g. SHOW, EXPLAIN)
            // are passed through unchanged — MySQL only honours the hint on SELECTs.
            let limited = if let Some(ms) = req.timeout_ms.filter(|&t| t > 0) {
                if limited.trim_start().to_uppercase().starts_with("SELECT") {
                    // "SELECT /*+ MAX_EXECUTION_TIME(N) */ ..."
                    limited.replacen("SELECT", &format!("SELECT /*+ MAX_EXECUTION_TIME({ms}) */"), 1)
                } else {
                    limited
                }
            } else {
                limited
            };
            run_read(&pool, &limited, max_rows, active_db, token).await
        } else {
            run_write(&pool, statement, active_db, token).await
        };
        let duration_ms = started.elapsed().as_millis() as u64;

        let mut result = result?;
        result.stats.duration_ms = duration_ms;
        result.stats.row_count = result.rows.len();
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
        let pool = self.pool(cfg).await?;
        // `KILL QUERY <id>` takes a bare integer; the id came from CONNECTION_ID()
        // (a u64), so there's nothing to escape.
        let sql = format!("KILL QUERY {conn_id}");
        // Best-effort: an already-finished query yields "Unknown thread id 1234"
        // — that's a successful no-op cancel, not a failure to report.
        let _ = sqlx::query(&sql).execute(&pool).await;
        Ok(())
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
        Ok(CompletionResponse { items })
    }

    async fn invalidate_completion_cache(&self, cfg: &ResolvedConfig) {
        self.completions.invalidate(&cfg.cache_key());
    }

    /// Streaming export: use sqlx's row CURSOR (`.fetch(&mut conn)`) so the
    /// (potentially huge) result is pulled one row at a time and written straight
    /// to the file — daemon memory stays bounded (NOT `fetch_all`). Only
    /// row-returning statements are exportable; a write/DDL is rejected (the
    /// service's write-gate already blocks guarded writes, but an export of a
    /// non-read makes no sense).
    async fn export_to_path(
        &self,
        cfg: &ResolvedConfig,
        statement: &str,
        node: Option<&str>,
        format: ExportFormat,
        max_rows: Option<usize>,
        dest: &std::path::Path,
    ) -> Result<ExportCounts> {
        use futures_util::TryStreamExt as _;

        let statement = statement.trim();
        if statement.is_empty() {
            return Err(types::invalid("empty statement"));
        }
        if !is_read_statement(statement) {
            return Err(types::invalid("export supports row-returning statements only"));
        }

        let pool = self.pool(cfg).await?;
        let mut conn = pool.acquire().await.map_err(types::upstream)?;
        if let Some(db) = node.map(str::trim).filter(|s| !s.is_empty()) {
            (&mut *conn).execute(sqlx::raw_sql(&use_db_sql(db)))
                .await
                .map_err(types::upstream)?;
        }

        // A real cursor over the wire: each `try_next().await` fetches the next
        // row; nothing buffers the whole result.
        let mut rows = sqlx::query(statement).fetch(&mut *conn);
        let mut sink = open_sink(dest, format)
            .map_err(|e| otto_core::Error::Internal(format!("create export file: {e}")))?;

        let mut header_written = false;
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
            let cells: Vec<Value> = (0..row.columns().len())
                .map(|i| mysql_value_to_json(&row, i))
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
            sqlx::query_as(sql).bind(db).bind(table).bind(pat).fetch_all(&pool).await
        } else {
            sqlx::query_as(sql).bind(db).bind(table).fetch_all(&pool).await
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
    ) -> Result<Vec<SchemaNode>> {
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
            sqlx::query_as(sql).bind(db).bind(table_type).bind(pat).fetch_all(&pool).await
        } else {
            sqlx::query_as(sql).bind(db).bind(table_type).fetch_all(&pool).await
        }
        .map_err(types::upstream)?;
        // The db node only carries `db:<n>`, not the folder; build children off
        // a clean db path so ids are `db:<n>/<seg>:<name>`.
        let db_path = NodePath::parse(&format!("db:{db}"));
        Ok(rows
            .into_iter()
            .map(|(name,)| {
                SchemaNode::new(db_path.child(seg, &name).to_id(), name, kind).expandable()
            })
            .collect())
    }

    async fn indexes_of(
        &self,
        pool: &sqlx::MySqlPool,
        db: &str,
        table: &str,
    ) -> Result<Vec<IndexDef>> {
        let sql = format!("SHOW INDEX FROM `{}`.`{}`", esc_ident(db), esc_ident(table));
        let rows = match sqlx::query(&sql).fetch_all(pool).await {
            Ok(rows) => rows,
            // Views and permission edge-cases: no indexes rather than a hard error.
            Err(_) => return Ok(Vec::new()),
        };
        // Preserve first-seen order of Key_name, columns ordered by Seq_in_index.
        let mut order: Vec<String> = Vec::new();
        #[allow(clippy::type_complexity)]
        let mut by_name: std::collections::HashMap<String, (bool, Option<String>, Vec<(i64, String)>)> =
            std::collections::HashMap::new();
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
        let mut by_name: std::collections::HashMap<String, ForeignKey> = std::collections::HashMap::new();
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
        let sql = format!("SHOW CREATE {kw} `{}`.`{}`", esc_ident(db), esc_ident(table));
        let row = sqlx::query(&sql).fetch_one(pool).await.map_err(types::upstream)?;
        // The DDL is the 2nd column for tables; for views it's "Create View"
        // (also the 2nd column). Read by index for robustness.
        let ddl: String = row.try_get(1).map_err(types::upstream)?;
        Ok(ddl)
    }
}

// --- Connection -------------------------------------------------------------

impl MysqlDriver {
    /// Get (or lazily build) the cached completion snapshot for `(connection, db)`.
    /// Built once and reused until refresh; a connection failure yields an empty
    /// snapshot that is NOT cached (so it retries next keystroke).
    async fn completion_snapshot(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
    ) -> std::sync::Arc<crate::complete::SchemaSnapshot> {
        let cache_key = cfg.cache_key();
        if let Some(s) = self.completions.get_snapshot(&cache_key, db) {
            return s;
        }
        match self.build_completion_snapshot(cfg, db).await {
            Some(snap) => self.completions.put_snapshot(&cache_key, db, snap),
            None => std::sync::Arc::new(crate::complete::SchemaSnapshot::default()),
        }
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
        use crate::complete::{FieldSnap, ObjKind, ObjectSnap, Rank, SchemaSnapshot};

        let pool = self.pool(cfg).await.ok()?;
        let databases: Vec<String> = sqlx::query_as::<_, (String,)>(
            "SELECT CAST(schema_name AS CHAR) FROM information_schema.schemata ORDER BY schema_name",
        )
        .fetch_all(&pool)
        .await
        .ok()?
        .into_iter()
        .map(|(d,)| d)
        .collect();

        if db.is_empty() {
            return Some(SchemaSnapshot {
                databases,
                objects: Vec::new(),
            });
        }

        let tables = sqlx::query_as::<_, (String, String)>(
            "SELECT CAST(table_name AS CHAR), table_type FROM information_schema.tables \
             WHERE table_schema = ? ORDER BY table_name",
        )
        .bind(db)
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

        let cols = sqlx::query_as::<_, (String, String, String)>(
            "SELECT CAST(table_name AS CHAR), CAST(column_name AS CHAR), CAST(column_type AS CHAR) \
             FROM information_schema.columns WHERE table_schema = ? \
             ORDER BY table_name, ordinal_position",
        )
        .bind(db)
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

        // (table, column) → strongest index rank, from information_schema.statistics
        // (which lists every index member column, so composite-index members all rank).
        let stats = sqlx::query_as::<_, (String, String, String, i64)>(
            "SELECT CAST(table_name AS CHAR), CAST(column_name AS CHAR), \
             CAST(index_name AS CHAR), non_unique FROM information_schema.statistics \
             WHERE table_schema = ?",
        )
        .bind(db)
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

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

        Some(SchemaSnapshot { databases, objects })
    }

    /// Get (or lazily build + cache) the pool for `cfg`. The cache is keyed by
    /// [`ResolvedConfig::cache_key`], so any session-affecting difference
    /// (endpoint/creds/db/TLS/timezone) gets its own pool. A `MySqlPool` clone
    /// is an `Arc` bump, so callers get a cheap handle to a shared pool and the
    /// expensive handshake is paid once. We hold the tokio mutex across the
    /// build await — that only briefly serializes concurrent *first* connects
    /// to the same key; steady-state cache hits return immediately.
    async fn pool(&self, cfg: &ResolvedConfig) -> Result<sqlx::MySqlPool> {
        let key = cfg.cache_key();
        let mut cache = self.pools.lock().await;
        if let Some(pool) = cache.get(&key) {
            return Ok(pool.clone());
        }
        let pool = build_pool(cfg).await?;
        cache.insert(key, pool.clone());
        Ok(pool)
    }
}

/// Build a fresh `MySqlPool` from a resolved config. Never called directly by
/// the driver methods — they go through [`MysqlDriver::pool`] for caching.
async fn build_pool(cfg: &ResolvedConfig) -> Result<sqlx::MySqlPool> {
    let mut opts = MySqlConnectOptions::new()
        .host(&cfg.host)
        .port(cfg.port);
    if let Some(user) = cfg.user.as_deref().filter(|s| !s.is_empty()) {
        opts = opts.username(user);
    }
    if let Some(password) = cfg.password.as_deref() {
        opts = opts.password(password);
    }
    if let Some(db) = cfg.database.as_deref().filter(|s| !s.is_empty()) {
        opts = opts.database(db);
    }

    // TLS.
    let ssl_mode = match cfg.tls.mode {
        types::TlsMode::Disabled => MySqlSslMode::Disabled,
        types::TlsMode::Preferred => MySqlSslMode::Preferred,
        types::TlsMode::Required => {
            if cfg.tls.verify {
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
        .after_connect(move |conn, _meta| {
            let tz_stmt = format!("SET time_zone = '{}'", tz.replace('\'', "''"));
            Box::pin(async move {
                // Best-effort: ignore errors (e.g. a named zone when the server's
                // tz tables aren't loaded) so a bad zone never breaks the session.
                let _ = sqlx::query(&tz_stmt).execute(&mut *conn).await;
                // Use cached information_schema statistics (avoids the expensive
                // per-table stats recomputation that slows the tree on big
                // servers). Best-effort: harmless if the server lacks the var.
                let _ = sqlx::query("SET SESSION information_schema_stats_expiry = 86400")
                    .execute(&mut *conn)
                    .await;
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
            s = rest.split_once('\n').map(|x| x.1).unwrap_or("").trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            // Block comment: skip to closing.
            s = rest.split_once("*/").map(|x| x.1).unwrap_or("").trim_start();
        } else {
            break;
        }
    }
    s.chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Read the backend connection id (`CONNECTION_ID()`) for the acquired session
/// and record it in `token`, so a concurrent cancel can `KILL QUERY <id>` this
/// exact connection. Best-effort: a failure leaves the token empty (the query
/// just can't be server-cancelled), never an error to the caller.
async fn capture_conn_id(conn: &mut sqlx::pool::PoolConnection<sqlx::MySql>, token: &CancelToken) {
    if let Ok(id) = sqlx::query_scalar::<_, u64>("SELECT CONNECTION_ID()")
        .fetch_one(&mut **conn)
        .await
    {
        token.set(QueryHandle::MysqlConnId(id));
    }
}

async fn run_read(
    pool: &sqlx::MySqlPool,
    statement: &str,
    max_rows: usize,
    active_db: Option<&str>,
    token: &CancelToken,
) -> Result<QueryResult> {
    // Acquire a single connection so the optional `USE <db>` and the statement
    // share the same session — the default schema must apply to the query.
    let mut conn = pool.acquire().await.map_err(types::upstream)?;
    // Capture this connection's backend id so a concurrent cancel can
    // `KILL QUERY <id>` it. Best-effort — a query with no captured id simply
    // can't be server-cancelled.
    capture_conn_id(&mut conn, token).await;
    if let Some(db) = active_db {
        (&mut *conn).execute(sqlx::raw_sql(&use_db_sql(db)))
                .await
                .map_err(types::upstream)?;
    }
    let rows = sqlx::query(statement)
        .fetch_all(&mut *conn)
        .await
        .map_err(types::upstream)?;

    let mut columns: Vec<Column> = Vec::new();
    if let Some(first) = rows.first() {
        for col in first.columns() {
            columns.push(Column::typed(col.name(), col.type_info().name()));
        }
    }

    let truncated = rows.len() > max_rows;
    let take = rows.len().min(max_rows);
    let mut out_rows: Vec<Vec<Value>> = Vec::with_capacity(take);
    for row in rows.iter().take(take) {
        let mut cells = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            cells.push(mysql_value_to_json(row, i));
        }
        out_rows.push(cells);
    }

    Ok(QueryResult {
        columns,
        rows: out_rows,
        rows_affected: None,
        stats: QueryStats::default(),
        message: None,
        truncated,
        masked: false,
    })
}

async fn run_write(
    pool: &sqlx::MySqlPool,
    statement: &str,
    active_db: Option<&str>,
    token: &CancelToken,
) -> Result<QueryResult> {
    // Same as run_read: `USE <db>` and the statement must share one session.
    let mut conn = pool.acquire().await.map_err(types::upstream)?;
    capture_conn_id(&mut conn, token).await;
    if let Some(db) = active_db {
        (&mut *conn).execute(sqlx::raw_sql(&use_db_sql(db)))
                .await
                .map_err(types::upstream)?;
    }
    let res = sqlx::query(statement)
        .execute(&mut *conn)
        .await
        .map_err(types::upstream)?;
    let affected = res.rows_affected();
    let mut result = QueryResult::message(format!("{affected} row(s) affected"));
    result.rows_affected = Some(affected);
    Ok(result)
}

/// Decode a single cell of a MySQL row to a `serde_json::Value`, trying a
/// sequence of native types and finally falling back to base64 bytes or Null.
fn mysql_value_to_json(row: &MySqlRow, idx: usize) -> Value {
    // Integers (covers TINYINT..BIGINT, signed). Try i64 first.
    if let Ok(v) = row.try_get::<Option<i64>, _>(idx) {
        return int_to_json(v);
    }
    // Unsigned BIGINT.
    if let Ok(v) = row.try_get::<Option<u64>, _>(idx) {
        return v.map(Value::from).unwrap_or(Value::Null);
    }
    // Floating point / decimal-as-f64.
    if let Ok(v) = row.try_get::<Option<f64>, _>(idx) {
        return float_to_json(v);
    }
    // Booleans (TINYINT(1)).
    if let Ok(v) = row.try_get::<Option<bool>, _>(idx) {
        return v.map(Value::Bool).unwrap_or(Value::Null);
    }
    // JSON columns decode straight to a Value.
    if let Ok(Some(val)) = row.try_get::<Option<Value>, _>(idx) {
        return val;
    }
    // Text (also covers most date/time types serialized to string).
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
    Value::Null
}

/// Pure shaping of an optional integer into a JSON value (Null if absent).
fn int_to_json(v: Option<i64>) -> Value {
    v.map(Value::from).unwrap_or(Value::Null)
}

/// Pure shaping of an optional string into a JSON value (Null if absent).
fn string_to_json(v: Option<String>) -> Value {
    v.map(Value::String).unwrap_or(Value::Null)
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
    "SELECT", "FROM", "WHERE", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE",
    "ALTER", "DROP", "TABLE", "VIEW", "INDEX", "DATABASE", "SCHEMA", "JOIN", "INNER", "LEFT",
    "RIGHT", "OUTER", "CROSS", "ON", "USING", "GROUP", "BY", "ORDER", "HAVING", "LIMIT", "OFFSET",
    "AS", "DISTINCT", "AND", "OR", "NOT", "NULL", "IS", "IN", "LIKE", "BETWEEN", "EXISTS", "CASE",
    "WHEN", "THEN", "ELSE", "END", "UNION", "ALL", "ANY", "ASC", "DESC", "PRIMARY", "KEY",
    "FOREIGN", "REFERENCES", "UNIQUE", "DEFAULT", "AUTO_INCREMENT", "CONSTRAINT", "WITH",
    "RECURSIVE", "TRUNCATE", "REPLACE", "IGNORE", "DUPLICATE", "INTERVAL", "CAST", "CONVERT",
    "USE", "SHOW", "DESCRIBE", "EXPLAIN", "GRANT", "REVOKE", "BEGIN", "COMMIT", "ROLLBACK",
    "TRANSACTION", "ENGINE", "CHARSET", "COLLATE", "TEMPORARY", "IF",
];

const FUNCTIONS: &[(&str, &str)] = &[
    ("COUNT", "COUNT(expr) — number of rows"),
    ("SUM", "SUM(expr) — sum of values"),
    ("AVG", "AVG(expr) — average of values"),
    ("MIN", "MIN(expr) — minimum value"),
    ("MAX", "MAX(expr) — maximum value"),
    ("GROUP_CONCAT", "GROUP_CONCAT(expr) — concatenated group values"),
    ("CONCAT", "CONCAT(str1, str2, ...) — concatenate strings"),
    ("CONCAT_WS", "CONCAT_WS(sep, str1, ...) — concat with separator"),
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
    ("CURRENT_TIMESTAMP", "CURRENT_TIMESTAMP() — current datetime"),
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
    ("DATE_SUB", "DATE_SUB(date, INTERVAL n unit) — subtract interval"),
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
    ("JSON_EXTRACT", "JSON_EXTRACT(json, path) — extract from JSON"),
    ("JSON_OBJECT", "JSON_OBJECT(k, v, ...) — build JSON object"),
    ("JSON_ARRAY", "JSON_ARRAY(v, ...) — build JSON array"),
    ("MD5", "MD5(str) — MD5 hash"),
    ("SHA2", "SHA2(str, bits) — SHA-2 hash"),
    ("UUID", "UUID() — generate UUID"),
];

// --- Unit tests -------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_read_statements() {
        assert!(is_read_statement("SELECT 1"));
        assert!(is_read_statement("  select * from t"));
        assert!(is_read_statement("SHOW TABLES"));
        assert!(is_read_statement("DESC users"));
        assert!(is_read_statement("DESCRIBE users"));
        assert!(is_read_statement("EXPLAIN SELECT 1"));
        assert!(is_read_statement("WITH cte AS (SELECT 1) SELECT * FROM cte"));
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
}
