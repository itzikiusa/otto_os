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
use sqlx::{Column as _, Row, TypeInfo};
use tokio::sync::Mutex;

use crate::driver::Driver;
use crate::tls::TlsFiles;
use crate::types::{
    self, Capabilities, Column, ColumnDef, CompletionContext, CompletionItem, CompletionKind,
    CompletionResponse, Engine, ForeignKey, IndexDef, NodeKind, NodePath, ObjectDetail,
    QueryRequest, QueryResult, QueryStats, ResolvedConfig, SchemaNode, TestResult,
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
    ) -> Result<Vec<SchemaNode>> {
        let db = parent
            .get("db")
            .ok_or_else(|| types::invalid("schema_children: parent has no database segment"))?
            .to_string();

        // db:<n>/table:<t> -> columns of the table.
        if let Some(table) = parent.get("table") {
            return self.columns_of(cfg, &db, table, parent).await;
        }

        // db:<n>/folder:tables | folder:views -> the objects in that folder.
        if let Some(folder) = parent.get("folder") {
            return self.objects_in_folder(cfg, &db, folder).await;
        }

        // db:<n> -> the two folders (Tables, Views).
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
        // detail.row_count stays None — no estimated counts (see above).
        detail.ddl = ddl;
        Ok(detail)
    }

    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        let statement = req.statement.trim();
        if statement.is_empty() {
            return Err(types::invalid("empty statement"));
        }
        let max_rows = req.max_rows.unwrap_or(DEFAULT_MAX_ROWS);
        let pool = self.pool(cfg).await?;
        let started = Instant::now();

        let result = if is_read_statement(statement) {
            let limited = types::inject_row_limit(statement, max_rows.saturating_add(1));
            run_read(&pool, &limited, max_rows).await
        } else {
            run_write(&pool, statement).await
        };
        let duration_ms = started.elapsed().as_millis() as u64;

        let mut result = result?;
        result.stats.duration_ms = duration_ms;
        result.stats.row_count = result.rows.len();
        Ok(result)
    }

    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        let mut items: Vec<CompletionItem> = Vec::new();

        // (a) Keywords.
        for kw in KEYWORDS {
            items.push(CompletionItem::new(*kw, CompletionKind::Keyword));
        }
        // (b) Builtin functions with signatures.
        for (name, sig) in FUNCTIONS {
            items.push(CompletionItem::detailed(*name, CompletionKind::Function, *sig));
        }

        // (c) Live identifiers — best-effort; connection failures don't break
        // the keyword/function list.
        if let Ok(pool) = self.pool(cfg).await {
            // Database names.
            if let Ok(dbs) = sqlx::query_as::<_, (String,)>(
                "SELECT CAST(schema_name AS CHAR) FROM information_schema.schemata ORDER BY schema_name",
            )
            .fetch_all(&pool)
            .await
            {
                for (db,) in dbs {
                    items.push(CompletionItem::new(db, CompletionKind::Database));
                }
            }

            let scope_db = ctx
                .database
                .clone()
                .or_else(|| cfg.database.clone())
                .filter(|s| !s.is_empty());
            if let Some(db) = scope_db {
                // Table names in the scoped database.
                if let Ok(tables) = sqlx::query_as::<_, (String,)>(
                    "SELECT CAST(table_name AS CHAR) FROM information_schema.tables \
                     WHERE table_schema = ? ORDER BY table_name",
                )
                .bind(&db)
                .fetch_all(&pool)
                .await
                {
                    for (t,) in tables {
                        items.push(CompletionItem::new(t, CompletionKind::Table));
                    }
                }
                // Column names across the scoped database (bounded).
                if let Ok(cols) = sqlx::query_as::<_, (String, String)>(
                    "SELECT CAST(column_name AS CHAR), CAST(column_type AS CHAR) \
                     FROM information_schema.columns \
                     WHERE table_schema = ? ORDER BY table_name, ordinal_position LIMIT 500",
                )
                .bind(&db)
                .fetch_all(&pool)
                .await
                {
                    for (name, ty) in cols {
                        items.push(CompletionItem::detailed(name, CompletionKind::Column, ty));
                    }
                }
            }
        }

        Ok(CompletionResponse { items })
    }
}

impl MysqlDriver {
    /// Columns of a table (lazy expansion of a `table:` node).
    async fn columns_of(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
        table: &str,
        parent: &NodePath,
    ) -> Result<Vec<SchemaNode>> {
        let pool = self.pool(cfg).await?;
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT CAST(column_name AS CHAR), CAST(column_type AS CHAR) \
             FROM information_schema.columns \
             WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
        )
        .bind(db)
        .bind(table)
        .fetch_all(&pool)
        .await
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

    /// Tables or views in a database folder.
    async fn objects_in_folder(
        &self,
        cfg: &ResolvedConfig,
        db: &str,
        folder: &str,
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
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT CAST(table_name AS CHAR) FROM information_schema.tables \
             WHERE table_schema = ? AND table_type = ? ORDER BY table_name",
        )
        .bind(db)
        .bind(table_type)
        .fetch_all(&pool)
        .await
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
            s = rest.splitn(2, '\n').nth(1).unwrap_or("").trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            // Block comment: skip to closing.
            s = rest.splitn(2, "*/").nth(1).unwrap_or("").trim_start();
        } else {
            break;
        }
    }
    s.chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase()
}

async fn run_read(
    pool: &sqlx::MySqlPool,
    statement: &str,
    max_rows: usize,
) -> Result<QueryResult> {
    let rows = sqlx::query(statement)
        .fetch_all(pool)
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
    })
}

async fn run_write(pool: &sqlx::MySqlPool, statement: &str) -> Result<QueryResult> {
    let res = sqlx::query(statement)
        .execute(pool)
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
    if let Ok(v) = row.try_get::<Option<Value>, _>(idx) {
        if let Some(val) = v {
            return val;
        }
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
