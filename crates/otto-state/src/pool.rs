//! [`DbPool`]: the daemon's SQLite handle, split into a small WRITER pool and a
//! read-only READER pool behind one type (r3-06-05).
//!
//! Why: SQLite (WAL) runs any number of readers alongside ONE writer. With a
//! single 8-connection pool, a writer blocked in SQLite's busy handler keeps
//! its pooled connection for up to `busy_timeout`; a burst of queued writers
//! (hook ingest, workgraph events, MCP audit, an hourly prune) therefore held
//! most of the pool and plain UI reads waited on pool acquire even though WAL
//! would have let them run (measured: 126–504 ms concurrent waits). Now
//! writers queue for the writer pool's connections (in process, holding
//! nothing) and reads never share a connection budget with them.
//!
//! How: `&DbPool` is itself an [`sqlx::Executor`], and routes EACH statement by
//! its SQL text ([`is_read_only_sql`]): a plain `SELECT` (or a DML-free `WITH`)
//! goes to the reader pool, everything else — DML, DDL, `PRAGMA`, anything
//! ambiguous — to the writer. So `query.fetch_all(&pool)` needs no call-site
//! change and every repo is routed by one rule, not per method. Explicit
//! transactions ([`DbPool::begin`]) and raw connections ([`DbPool::acquire`])
//! always come from the writer, and statements run on a transaction never
//! pass through the router. The reader connections are opened read-only, so a
//! mis-routed write fails loudly (`attempt to write a readonly database`)
//! instead of silently racing the writer.
//!
//! Tests and tools that build their own (often `sqlite::memory:`) pool convert
//! it with `DbPool::from(pool)`: one pool serves both roles, exactly as before.

use std::fmt;

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use sqlx::error::BoxDynError;
use sqlx::sqlite::{
    Sqlite, SqliteArguments, SqliteQueryResult, SqliteRow, SqliteStatement, SqliteTypeInfo,
};
use sqlx::{Describe, Either, Execute, Executor, SqlStr, SqlitePool, Transaction};

/// The daemon's SQLite pools (see the module docs). Cheap to clone.
#[derive(Clone)]
pub struct DbPool {
    read: SqlitePool,
    write: SqlitePool,
    /// Distinct reader and writer pools ([`DbPool::split`]).
    split: bool,
    /// Process-unique identity shared by every clone (see [`DbPool::id`]).
    id: u64,
    /// Statements + transactions/connections handed out, shared by every
    /// clone (see [`DbPool::op_count`]). One relaxed atomic add per statement.
    ops: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Opt-in statement recorder shared by every clone (perf W12): unset in
    /// the daemon (one atomic load per statement), armed by tests through
    /// [`DbPool::statement_probe`] to assert query budgets.
    probe: std::sync::Arc<std::sync::OnceLock<std::sync::Arc<StatementProbe>>>,
}

/// Records the SQL of every statement routed through a [`DbPool`] once armed
/// ([`DbPool::statement_probe`]). Statements on an explicit transaction or a
/// raw `writer()`/`reader()` connection bypass the router and are not seen.
#[derive(Debug, Default)]
pub struct StatementProbe {
    stmts: std::sync::Mutex<Vec<String>>,
}

impl StatementProbe {
    fn record(&self, sql: &str) {
        if let Ok(mut v) = self.stmts.lock() {
            v.push(sql.to_string());
        }
    }

    /// Everything recorded since the last take, oldest first.
    pub fn take(&self) -> Vec<String> {
        self.stmts
            .lock()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default()
    }

    /// Forget what was recorded so far.
    pub fn reset(&self) {
        let _ = self.take();
    }

    /// Number of statements recorded so far (not consumed).
    pub fn count(&self) -> usize {
        self.stmts.lock().map(|v| v.len()).unwrap_or(0)
    }

    /// Recorded statements containing `needle` (case-sensitive), not consumed.
    pub fn matching(&self, needle: &str) -> Vec<String> {
        self.stmts
            .lock()
            .map(|v| v.iter().filter(|s| s.contains(needle)).cloned().collect())
            .unwrap_or_default()
    }
}

fn next_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl fmt::Debug for DbPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbPool")
            .field("split", &!self.is_single())
            .field("read_size", &self.read.size())
            .field("write_size", &self.write.size())
            .finish()
    }
}

impl From<SqlitePool> for DbPool {
    /// One pool in both roles — tests, in-memory databases, tools.
    fn from(pool: SqlitePool) -> Self {
        Self {
            read: pool.clone(),
            write: pool,
            split: false,
            id: next_id(),
            ops: Default::default(),
            probe: Default::default(),
        }
    }
}

impl DbPool {
    /// A split handle: `read` MUST be a read-only pool over the same database
    /// file as `write`.
    pub fn split(read: SqlitePool, write: SqlitePool) -> Self {
        Self {
            read,
            write,
            split: true,
            id: next_id(),
            ops: Default::default(),
            probe: Default::default(),
        }
    }

    /// A single (unsplit) pool on `url` — tests and tools, typically
    /// `sqlite::memory:`.
    pub async fn connect(url: &str) -> sqlx::Result<Self> {
        SqlitePool::connect(url).await.map(Self::from)
    }

    /// Process-unique identity of this database handle, shared by its
    /// clones — a key for caches that must never mix two databases (tests run
    /// many daemons' worth of state in one process).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// How many statements this handle (and its clones) has routed, plus the
    /// transactions and raw connections it handed out (each counts once; the
    /// statements run on them do not pass through the router). Monotonic. A
    /// query-budget probe for performance tests: read it before and after an
    /// operation and assert the difference (the MCP governance path budgets).
    pub fn op_count(&self) -> u64 {
        self.ops.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn tick(&self) {
        self.ops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// True when one pool serves both roles ([`From<SqlitePool>`]).
    pub fn is_single(&self) -> bool {
        !self.split
    }

    /// The writer pool. For APIs that need a concrete `sqlx::SqlitePool`
    /// (migrations, `Acquire`); every statement on it runs as a write.
    pub fn writer(&self) -> &SqlitePool {
        &self.write
    }

    /// The reader pool (read-only connections when split). Only for code that
    /// must pin a read to a plain pool; normal queries route themselves.
    pub fn reader(&self) -> &SqlitePool {
        &self.read
    }

    /// Begin a transaction on the WRITER.
    pub async fn begin(&self) -> sqlx::Result<Transaction<'static, Sqlite>> {
        self.tick();
        self.write.begin().await
    }

    /// Begin a transaction on the WRITER with a custom statement (e.g.
    /// `BEGIN IMMEDIATE`).
    pub async fn begin_with(
        &self,
        statement: impl sqlx::SqlSafeStr,
    ) -> sqlx::Result<Transaction<'static, Sqlite>> {
        self.tick();
        self.write.begin_with(statement).await
    }

    /// A raw connection from the WRITER: whatever runs on it may write.
    pub async fn acquire(&self) -> sqlx::Result<sqlx::pool::PoolConnection<Sqlite>> {
        self.tick();
        self.write.acquire().await
    }

    /// Close both pools.
    pub async fn close(&self) {
        self.write.close().await;
        self.read.close().await;
    }

    /// True once [`Self::close`] ran.
    pub fn is_closed(&self) -> bool {
        self.write.is_closed()
    }

    /// Arm (once) and return this pool's statement recorder; every clone —
    /// including ones made before arming — records into it from now on.
    pub fn statement_probe(&self) -> std::sync::Arc<StatementProbe> {
        self.probe
            .get_or_init(|| std::sync::Arc::new(StatementProbe::default()))
            .clone()
    }

    fn route(&self, sql: &str) -> &SqlitePool {
        if let Some(p) = self.probe.get() {
            p.record(sql);
        }
        if is_read_only_sql(sql) {
            &self.read
        } else {
            &self.write
        }
    }
}

/// A statement taken apart for routing. [`Execute::sql`] consumes the query,
/// so the router reads its arguments and caching flag first, takes the SQL,
/// and hands the pool this reassembled statement. A query built from a
/// prepared statement runs by its SQL text instead (SQLite's statement cache
/// makes that equivalent).
struct Routed {
    sql: SqlStr,
    arguments: Option<SqliteArguments>,
    persistent: bool,
}

impl Routed {
    fn take<'q, E: Execute<'q, Sqlite>>(mut query: E) -> Result<Self, BoxDynError> {
        let arguments = query.take_arguments()?;
        let persistent = query.persistent();
        Ok(Self {
            sql: query.sql(),
            arguments,
            persistent,
        })
    }
}

impl Execute<'_, Sqlite> for Routed {
    fn sql(self) -> SqlStr {
        self.sql
    }

    fn statement(&self) -> Option<&SqliteStatement> {
        None
    }

    fn take_arguments(&mut self) -> Result<Option<SqliteArguments>, BoxDynError> {
        Ok(self.arguments.take())
    }

    fn persistent(&self) -> bool {
        self.persistent
    }
}

impl<'p> Executor<'p> for &'p DbPool {
    type Database = Sqlite;

    fn fetch_many<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxStream<'e, Result<Either<SqliteQueryResult, SqliteRow>, sqlx::Error>>
    where
        'p: 'e,
        E: 'q + Execute<'q, Sqlite>,
    {
        match Routed::take(query) {
            Ok(routed) => {
                self.tick();
                self.route(routed.sql.as_str()).fetch_many(routed)
            }
            Err(e) => Box::pin(futures_util::stream::once(async move {
                Err(sqlx::Error::Encode(e))
            })),
        }
    }

    fn fetch_optional<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxFuture<'e, Result<Option<SqliteRow>, sqlx::Error>>
    where
        'p: 'e,
        E: 'q + Execute<'q, Sqlite>,
    {
        match Routed::take(query) {
            Ok(routed) => {
                self.tick();
                self.route(routed.sql.as_str()).fetch_optional(routed)
            }
            Err(e) => Box::pin(async move { Err(sqlx::Error::Encode(e)) }),
        }
    }

    fn prepare_with<'e>(
        self,
        sql: SqlStr,
        parameters: &'e [SqliteTypeInfo],
    ) -> BoxFuture<'e, Result<SqliteStatement, sqlx::Error>>
    where
        'p: 'e,
    {
        self.route(sql.as_str()).prepare_with(sql, parameters)
    }

    fn describe<'e>(self, sql: SqlStr) -> BoxFuture<'e, Result<Describe<Sqlite>, sqlx::Error>>
    where
        'p: 'e,
    {
        self.route(sql.as_str()).describe(sql)
    }
}

/// `&DbPool` as an [`sqlx::Acquire`] (migrations, helpers generic over
/// `Acquire`): always the WRITER — whatever runs on the connection may write.
impl<'a> sqlx::Acquire<'a> for &'_ DbPool {
    type Database = Sqlite;
    type Connection = sqlx::pool::PoolConnection<Sqlite>;

    fn acquire(self) -> BoxFuture<'static, Result<Self::Connection, sqlx::Error>> {
        self.tick();
        Box::pin(self.write.acquire())
    }

    fn begin(self) -> BoxFuture<'static, Result<Transaction<'a, Sqlite>, sqlx::Error>> {
        self.tick();
        let write = self.write.clone();
        Box::pin(async move { write.begin().await })
    }
}

/// May `sql` run on a read-only connection? Deliberately conservative: only a
/// single statement that starts with `SELECT` — or `WITH` without any DML
/// keyword — and calls none of SQLite's per-connection state functions
/// (`last_insert_rowid()`, `changes()`, `total_changes()`: their answer lives
/// on the connection that did the write). Everything else — DML, DDL,
/// `PRAGMA`, `BEGIN`, several statements, anything unrecognised — is a write.
pub fn is_read_only_sql(sql: &str) -> bool {
    let body = skip_leading_trivia(sql);
    let upper = body.to_ascii_uppercase();
    let first = upper
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or("");
    let read_verb = match first {
        "SELECT" => true,
        "WITH" => !["INSERT", "UPDATE", "DELETE", "REPLACE"]
            .iter()
            .any(|kw| has_word(&upper, kw)),
        _ => false,
    };
    if !read_verb {
        return false;
    }
    // More than one statement (anything but trivia after a `;`).
    if let Some(i) = upper.find(';') {
        if !skip_leading_trivia(&upper[i + 1..]).is_empty() {
            return false;
        }
    }
    !["LAST_INSERT_ROWID", "TOTAL_CHANGES", "CHANGES"]
        .iter()
        .any(|f| has_word(&upper, f))
}

/// `s` without leading whitespace and SQL comments (`-- …\n`, `/* … */`).
fn skip_leading_trivia(mut s: &str) -> &str {
    loop {
        let t = s.trim_start();
        if let Some(rest) = t.strip_prefix("--") {
            s = rest.find('\n').map_or("", |i| &rest[i + 1..]);
        } else if let Some(rest) = t.strip_prefix("/*") {
            s = rest.find("*/").map_or("", |i| &rest[i + 2..]);
        } else {
            return t;
        }
    }
}

/// `word` occurs in `upper` as a whole identifier-ish token.
fn has_word(upper: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    upper.match_indices(word).any(|(i, _)| {
        let before = upper[..i].chars().next_back();
        let after = upper[i + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn op_count_counts_statements_and_transactions_across_clones() {
        let pool = DbPool::connect("sqlite::memory:").await.unwrap();
        let clone = pool.clone();
        let before = pool.op_count();
        let _: i64 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        let _: Option<i64> = sqlx::query_scalar("SELECT 2")
            .fetch_optional(&clone)
            .await
            .unwrap();
        let tx = pool.begin().await.unwrap();
        drop(tx);
        assert_eq!(pool.op_count() - before, 3);
        assert_eq!(clone.op_count(), pool.op_count());
    }

    #[test]
    fn routes_only_plain_reads_to_the_reader() {
        for sql in [
            "SELECT * FROM sessions WHERE id = ?",
            "  select id from t",
            "\n-- leading comment\nSELECT 1",
            "/* hint */ SELECT COUNT(*) FROM agent_trail",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "SELECT updated_at, deleted FROM t", // column names, not DML
            "SELECT 1;",
            "SELECT 1;  -- trailing",
        ] {
            assert!(is_read_only_sql(sql), "read: {sql}");
        }
        for sql in [
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET a = 1",
            "DELETE FROM t",
            "REPLACE INTO t VALUES (1)",
            "insert or ignore into t values (1)",
            "WITH old AS (SELECT id FROM t) DELETE FROM t WHERE id IN old",
            "WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x",
            "CREATE INDEX i ON t(a)",
            "PRAGMA table_info(t)",
            "BEGIN",
            "SELECT last_insert_rowid()",
            "SELECT changes()",
            "SELECT 1; DELETE FROM t",
            "",
            "-- only a comment",
            "VACUUM",
            "ANALYZE",
        ] {
            assert!(!is_read_only_sql(sql), "write: {sql}");
        }
    }

    /// End to end on a real WAL file: reads go to the read-only reader, writes
    /// to the writer, a transaction's statements stay on its connection, and a
    /// read right after a committed write sees it.
    #[tokio::test]
    async fn split_pool_routes_reads_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::open(&dir.path().join("t.db")).await.unwrap();
        assert!(!pool.is_single());
        sqlx::query("CREATE TABLE IF NOT EXISTS pool_probe (id INTEGER PRIMARY KEY, v TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO pool_probe (v) VALUES ('a')")
            .execute(&pool)
            .await
            .unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_probe")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1, "read-after-write through the router sees the commit");

        // The reader really is read-only: a write forced onto it fails.
        let err = sqlx::query("INSERT INTO pool_probe (v) VALUES ('x')")
            .execute(pool.reader())
            .await
            .unwrap_err();
        assert!(err.to_string().to_lowercase().contains("readonly"), "{err}");

        // Transactions live on the writer and see their own writes.
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO pool_probe (v) VALUES ('b')")
            .execute(&mut *tx)
            .await
            .unwrap();
        let in_tx: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_probe")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(in_tx, 2);
        // …and are invisible to readers until commit.
        let outside: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_probe")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(outside, 1);
        tx.commit().await.unwrap();
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_probe")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(after, 2);
    }

    /// The point of the split: while a writer holds SQLite's write lock (and
    /// other writers queue behind it), reads still complete immediately.
    #[tokio::test]
    async fn reads_do_not_queue_behind_a_held_write_lock() {
        let dir = tempfile::tempdir().unwrap();
        let pool = crate::open(&dir.path().join("t.db")).await.unwrap();
        sqlx::query("CREATE TABLE IF NOT EXISTS pool_probe (id INTEGER PRIMARY KEY, v TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        // Hold the write lock on one writer connection.
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO pool_probe (v) VALUES ('held')")
            .execute(&mut *tx)
            .await
            .unwrap();
        // Queue writers: they wait (on the writer pool / busy handler).
        let mut writers = Vec::new();
        for _ in 0..6 {
            let p = pool.clone();
            writers.push(tokio::spawn(async move {
                sqlx::query("INSERT INTO pool_probe (v) VALUES ('w')")
                    .execute(&p)
                    .await
            }));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // Reads (more than the old 8-connection pool's worth) still answer now.
        let started = std::time::Instant::now();
        for _ in 0..16 {
            let _: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_probe")
                .fetch_one(&pool)
                .await
                .unwrap();
        }
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "reads queued behind writers: {:?}",
            started.elapsed()
        );
        tx.commit().await.unwrap();
        for w in writers {
            w.await.unwrap().unwrap();
        }
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_probe")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 7);
    }
}
