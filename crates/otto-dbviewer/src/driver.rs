//! The engine-agnostic driver contract. Each engine implements this in
//! `drivers/<engine>.rs`. Methods receive a fully [`ResolvedConfig`] — any SSH
//! tunnel is already established by the service, so `host`/`port` are reachable.

use async_trait::async_trait;
use otto_core::Result;

use crate::export::{ExportCounts, ExportFormat};
use crate::types::{
    Capabilities, CancelToken, CompletionContext, CompletionResponse, Engine, NodePath,
    ObjectDetail, ObjectSearchReq, ObjectSearchResult, QueryHandle, QueryRequest, QueryResult,
    ResolvedConfig, SchemaNode, TestResult,
};

/// Hard row ceiling for the buffered `export_to_writer` fallback (engines with no
/// native row stream, e.g. Redis). A "full export" (`max_rows: None`) that would
/// materialise more than this in RAM is REFUSED with a clear error rather than
/// risking a daemon OOM — the streaming drivers (MySQL/ClickHouse/MongoDB) have
/// no such cap because they never buffer the whole result.
pub const BUFFERED_EXPORT_ROW_CAP: usize = 100_000;

#[async_trait]
pub trait Driver: Send + Sync {
    /// The engine this driver serves.
    fn engine(&self) -> Engine;

    /// Static capabilities (drives UI affordances).
    fn capabilities(&self) -> Capabilities;

    /// Connect and run a cheap probe (ping / SELECT 1). Reports latency +
    /// server version.
    async fn test(&self, cfg: &ResolvedConfig) -> Result<TestResult>;

    /// Inspect the actual native credential ceiling. Unsupported engines fail
    /// closed for governed arbitrary scripts; no caller-supplied assertion is used.
    async fn native_grants(&self, _cfg: &ResolvedConfig) -> Result<Vec<crate::native_access::NativeGrant>> {
        Err(crate::native_access::setup_error("native privilege verification is unsupported for this engine"))
    }

    /// Top level of the object tree (databases / keyspaces / etc.).
    async fn schema_root(&self, cfg: &ResolvedConfig) -> Result<Vec<SchemaNode>>;

    /// Children of a tree node (lazy expansion). `filter`, when set, narrows the
    /// listing (used by Redis to `SCAN MATCH <filter>*`); SQL/Mongo ignore it.
    async fn schema_children(
        &self,
        cfg: &ResolvedConfig,
        parent: &NodePath,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>>;

    /// Like `schema_children` but may fill `SchemaNode.detail` with an
    /// engine-native ROW-COUNT ESTIMATE when `counts` is true. Opt-in on
    /// purpose: collecting the statistic for every object is the slow part of
    /// expanding a database on a big server, which is why the plain listing
    /// deliberately skips it. Default: ignore the flag.
    async fn schema_children_with_counts(
        &self,
        cfg: &ResolvedConfig,
        parent: &NodePath,
        filter: Option<&str>,
        counts: bool,
    ) -> Result<Vec<SchemaNode>> {
        let _ = counts;
        self.schema_children(cfg, parent, filter).await
    }

    /// Find objects by NAME across a schema (or every schema) without expanding
    /// the tree. Default: the engine has no object namespace to search, so the
    /// caller is told `supported: false` rather than handed an error.
    async fn search_objects(
        &self,
        cfg: &ResolvedConfig,
        req: &ObjectSearchReq,
    ) -> Result<ObjectSearchResult> {
        let _ = (cfg, req);
        Ok(ObjectSearchResult::default())
    }

    /// Full structure of a selected object (columns, keys, indexes, DDL).
    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail>;

    /// Like `object_detail` but when `approx_row_count` is `true`, the result's
    /// `row_count` is filled from an engine-native estimate (e.g. MySQL
    /// `information_schema.table_rows`). Default: calls `object_detail` and
    /// ignores the flag, so drivers that don't have cheap estimates just inherit
    /// this.
    async fn object_detail_with_opts(
        &self,
        cfg: &ResolvedConfig,
        path: &NodePath,
        approx_row_count: bool,
    ) -> Result<ObjectDetail> {
        let _ = approx_row_count;
        self.object_detail(cfg, path).await
    }

    /// Execute a query / command and return a tabular result.
    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult>;

    /// Execute a query while exposing an engine-native cancel handle through
    /// `token`. Engines with server-side cancellation (MySQL/ClickHouse) override
    /// this to capture their handle (backend connection id / `query_id`) into the
    /// token before/while running, so a concurrent [`Driver::cancel`] can target
    /// the running query. The default ignores the token and just runs — correct
    /// for engines without a native per-query cancel.
    async fn run_tracked(
        &self,
        cfg: &ResolvedConfig,
        req: &QueryRequest,
        _token: &CancelToken,
    ) -> Result<QueryResult> {
        self.run(cfg, req).await
    }

    /// Cancel an in-flight query identified by a previously-captured
    /// [`QueryHandle`], on a SEPARATE connection (you can't `KILL` on the blocked
    /// one). Engines without a native cancel use the default no-op. Cancelling an
    /// already-finished / unknown query must be a no-op success — never an error.
    async fn cancel(&self, _cfg: &ResolvedConfig, _handle: &QueryHandle) -> Result<()> {
        Ok(())
    }

    /// Tear down and evict any cached connection handle / pool for `cache_key`
    /// (a [`ResolvedConfig::cache_key`] value). Called by the service when a
    /// connection is explicitly closed, and when a config change rekeys a
    /// connection so the superseded handle would otherwise be retained forever.
    /// Must be idempotent — an unknown key is a no-op. The default is a no-op
    /// for engines that cache nothing.
    async fn close(&self, _cache_key: &str) {}

    /// Produce a normalized [`DbQueryPlan`] for `statement` by running the engine's
    /// native EXPLAIN (the statement is EXPLAIN-wrapped, **never executed raw** —
    /// read-only by construction). `node` scopes the active database (same
    /// semantics as [`QueryRequest::node`]). The default is unsupported (Redis);
    /// SQL engines + Mongo override it.
    async fn query_plan(
        &self,
        _cfg: &ResolvedConfig,
        _statement: &str,
        _node: Option<&str>,
    ) -> Result<crate::types::DbQueryPlan> {
        Err(otto_core::Error::Invalid("explain not supported".into()))
    }

    /// Bulk-import already-parsed rows into a collection/table as the engine's
    /// native batched insert, in batches of `batch_size`. Only MongoDB implements
    /// this — SQL engines import via the service's INSERT-statement path — so the
    /// default is unsupported. `node` scopes the active database. Returns
    /// `(rows_inserted, batches_run)`.
    async fn import_rows(
        &self,
        _cfg: &ResolvedConfig,
        _target: &str,
        _columns: &[String],
        _rows: &[Vec<serde_json::Value>],
        _batch_size: usize,
        _node: Option<&str>,
    ) -> Result<(u64, u64)> {
        Err(otto_core::Error::Invalid(
            "file import is not supported for this engine".into(),
        ))
    }

    /// Autocomplete items for the editor, scoped to the given context.
    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse>;

    /// Drop any cached completion snapshot for this connection so the next
    /// completion re-introspects the live schema. Called when the user refreshes
    /// the connection. The default is a no-op (engines without a snapshot cache,
    /// e.g. Redis, keep nothing to clear).
    async fn invalidate_completion_cache(&self, _cfg: &ResolvedConfig) {}

    /// Stream a (potentially huge) **uncapped** read result to an arbitrary
    /// writer `w`, in `format`, with **bounded daemon memory** — pull one
    /// row/chunk at a time from the engine's native cursor/stream and write it
    /// straight through `w`. `node` scopes the active database (same semantics as
    /// `QueryRequest::node`); `max_rows`, when set, stops the stream early —
    /// `None` means a genuinely uncapped export (the `/db/export` fix: the old
    /// path substituted each driver's small default cap and silently truncated).
    ///
    /// `w` is provided by the caller: [`Driver::export_to_path`] passes a buffered
    /// `File`; the HTTP `/db/export` handler passes a channel-backed writer so the
    /// bytes stream straight to the browser without ever buffering the full result.
    ///
    /// Engines that can stream (MySQL/ClickHouse/MongoDB) **override** this. The
    /// trait default is a **last-resort buffering fallback**: it runs the
    /// statement via [`Driver::run`] (which materialises the result in RAM) and
    /// writes it out, `tracing::warn!`ing so the buffering is never silent. To
    /// keep that fallback from trying to hold an unbounded "full export" in RAM it
    /// probes at [`BUFFERED_EXPORT_ROW_CAP`]` + 1` and REFUSES a larger result
    /// with a clear error. This default is what Redis (no row stream) uses.
    async fn export_to_writer(
        &self,
        cfg: &ResolvedConfig,
        statement: &str,
        node: Option<&str>,
        format: ExportFormat,
        max_rows: Option<usize>,
        w: Box<dyn std::io::Write + Send>,
    ) -> Result<ExportCounts> {
        tracing::warn!(
            engine = self.engine().as_str(),
            "db export: driver has no streaming path — buffering the result in memory \
             (last-resort fallback); capped at {BUFFERED_EXPORT_ROW_CAP} rows"
        );
        // Probe at the cap + 1 so an unbounded export (`max_rows: None`) or an
        // over-cap request is DETECTED rather than silently OOMing the daemon.
        let probe_max = match max_rows {
            Some(m) if m <= BUFFERED_EXPORT_ROW_CAP => m,
            _ => BUFFERED_EXPORT_ROW_CAP + 1,
        };
        let req = QueryRequest {
            statement: statement.to_string(),
            max_rows: Some(probe_max),
            node: node.map(str::to_string),
            ..QueryRequest::default()
        };
        let result = self.run(cfg, &req).await?;
        if result.rows.len() > BUFFERED_EXPORT_ROW_CAP {
            return Err(otto_core::Error::Invalid(format!(
                "export too large for this engine's buffered fallback (over {BUFFERED_EXPORT_ROW_CAP} \
                 rows) — it has no streaming export path; narrow the query or add an explicit LIMIT"
            )));
        }
        crate::export::write_buffered_result_to(w, format, &result)
            .map_err(|e| otto_core::Error::Internal(format!("write export file: {e}")))
    }

    /// Convenience wrapper: stream an export to a local file at `dest`. Opens the
    /// file behind a 64 KiB `BufWriter` and delegates to [`Driver::export_to_writer`]
    /// — the one place export bytes are produced. Drivers override `export_to_writer`,
    /// never this. Backs `POST /db/export-to-path`.
    async fn export_to_path(
        &self,
        cfg: &ResolvedConfig,
        statement: &str,
        node: Option<&str>,
        format: ExportFormat,
        max_rows: Option<usize>,
        dest: &std::path::Path,
    ) -> Result<ExportCounts> {
        let w = crate::export::open_file_sink(dest)
            .map_err(|e| otto_core::Error::Internal(format!("create export file: {e}")))?;
        self.export_to_writer(cfg, statement, node, format, max_rows, w)
            .await
    }
}

#[cfg(test)]
mod tests {
    //! Trait-level tests for the cancel plumbing's defaults and dispatch — no
    //! network. A minimal stub `Driver` records what `cancel` received and
    //! whether `run`/`run_tracked` ran, so we can prove: (1) the default
    //! `run_tracked` delegates to `run` (ignoring the token), (2) the default
    //! `cancel` is a no-op success, and (3) an overriding driver's `cancel` is
    //! dispatched with the captured handle.

    use std::sync::Mutex;

    use super::*;
    use crate::types::{
        Capabilities, CancelToken, CompletionContext, CompletionResponse, Engine, NodePath,
        ObjectDetail, QueryHandle, QueryRequest, QueryResult, ResolvedConfig, SchemaNode,
        TestResult, TlsConfig,
    };

    fn cfg() -> ResolvedConfig {
        ResolvedConfig {
            engine: Engine::Mysql,
            host: "127.0.0.1".into(),
            port: 3306,
            user: None,
            password: None,
            database: None,
            tls: TlsConfig::default(),
            params: serde_json::json!({}),
        }
    }

    /// A driver that implements ONLY the required methods — it does NOT override
    /// `run_tracked` or `cancel`, so calling those exercises the TRAIT DEFAULTS.
    /// `run` records that it ran, so we can see the default `run_tracked` delegate
    /// to it.
    #[derive(Default)]
    struct MinimalDriver {
        ran: Mutex<bool>,
    }

    #[async_trait]
    impl Driver for MinimalDriver {
        fn engine(&self) -> Engine {
            Engine::Redis
        }
        fn capabilities(&self) -> Capabilities {
            unreachable!()
        }
        async fn test(&self, _: &ResolvedConfig) -> Result<TestResult> {
            unreachable!()
        }
        async fn schema_root(&self, _: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
            unreachable!()
        }
        async fn schema_children(
            &self,
            _: &ResolvedConfig,
            _: &NodePath,
            _: Option<&str>,
        ) -> Result<Vec<SchemaNode>> {
            unreachable!()
        }
        async fn object_detail(&self, _: &ResolvedConfig, _: &NodePath) -> Result<ObjectDetail> {
            unreachable!()
        }
        async fn run(&self, _: &ResolvedConfig, _: &QueryRequest) -> Result<QueryResult> {
            *self.ran.lock().unwrap() = true;
            Ok(QueryResult::empty())
        }
        // NB: no `run_tracked`, no `cancel` — defaults are under test.
        async fn completion(
            &self,
            _: &ResolvedConfig,
            _: &CompletionContext,
        ) -> Result<CompletionResponse> {
            unreachable!()
        }
    }

    /// A driver that OVERRIDES `cancel` to record the handle it was dispatched —
    /// proving the service's `r.driver.cancel(handle)` reaches the right engine
    /// method with the captured handle.
    #[derive(Default)]
    struct CancellingDriver {
        cancelled: Mutex<Option<QueryHandle>>,
    }

    #[async_trait]
    impl Driver for CancellingDriver {
        fn engine(&self) -> Engine {
            Engine::Mysql
        }
        fn capabilities(&self) -> Capabilities {
            unreachable!()
        }
        async fn test(&self, _: &ResolvedConfig) -> Result<TestResult> {
            unreachable!()
        }
        async fn schema_root(&self, _: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
            unreachable!()
        }
        async fn schema_children(
            &self,
            _: &ResolvedConfig,
            _: &NodePath,
            _: Option<&str>,
        ) -> Result<Vec<SchemaNode>> {
            unreachable!()
        }
        async fn object_detail(&self, _: &ResolvedConfig, _: &NodePath) -> Result<ObjectDetail> {
            unreachable!()
        }
        async fn run(&self, _: &ResolvedConfig, _: &QueryRequest) -> Result<QueryResult> {
            unreachable!()
        }
        async fn cancel(&self, _: &ResolvedConfig, handle: &QueryHandle) -> Result<()> {
            *self.cancelled.lock().unwrap() = Some(handle.clone());
            Ok(())
        }
        async fn completion(
            &self,
            _: &ResolvedConfig,
            _: &CompletionContext,
        ) -> Result<CompletionResponse> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn default_run_tracked_delegates_to_run_and_ignores_token() {
        let d = MinimalDriver::default();
        let token = CancelToken::new();
        d.run_tracked(&cfg(), &QueryRequest::default(), &token)
            .await
            .expect("run_tracked ok");
        assert!(*d.ran.lock().unwrap(), "default run_tracked must call run");
        // The default doesn't touch the token (no native handle for this engine).
        assert!(token.handle().is_none());
    }

    #[tokio::test]
    async fn default_cancel_is_a_noop_success() {
        // The trait-default cancel runs (MinimalDriver doesn't override it): it
        // must succeed for any handle and do nothing observable.
        let d = MinimalDriver::default();
        d.cancel(&cfg(), &QueryHandle::ClickhouseQueryId("x".into()))
            .await
            .expect("default cancel is Ok");
    }

    #[tokio::test]
    async fn overridden_cancel_dispatches_with_the_captured_handle() {
        let d = CancellingDriver::default();
        d.cancel(&cfg(), &QueryHandle::MysqlConnId(99))
            .await
            .expect("cancel ok");
        assert!(matches!(
            *d.cancelled.lock().unwrap(),
            Some(QueryHandle::MysqlConnId(99))
        ));
    }

    // --- Export: trait-default (buffered fallback) + `export_to_path` wrapper ---

    /// A `Write` that appends into a shared `Vec<u8>` so a test can inspect the
    /// bytes the boxed writer received.
    #[derive(Clone)]
    struct SharedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A driver with NO streaming export override, so `export_to_writer` /
    /// `export_to_path` exercise the trait's buffered fallback (Redis-shaped).
    /// `run` returns `rows` two-column rows, honouring the request's `max_rows`.
    struct BufferedExportDriver {
        rows: usize,
    }

    #[async_trait]
    impl Driver for BufferedExportDriver {
        fn engine(&self) -> Engine {
            Engine::Redis
        }
        fn capabilities(&self) -> Capabilities {
            unreachable!()
        }
        async fn test(&self, _: &ResolvedConfig) -> Result<TestResult> {
            unreachable!()
        }
        async fn schema_root(&self, _: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
            unreachable!()
        }
        async fn schema_children(
            &self,
            _: &ResolvedConfig,
            _: &NodePath,
            _: Option<&str>,
        ) -> Result<Vec<SchemaNode>> {
            unreachable!()
        }
        async fn object_detail(&self, _: &ResolvedConfig, _: &NodePath) -> Result<ObjectDetail> {
            unreachable!()
        }
        async fn run(&self, _: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
            let n = self.rows.min(req.max_rows.unwrap_or(self.rows));
            let rows: Vec<Vec<serde_json::Value>> = (0..n)
                .map(|i| vec![serde_json::json!(i as i64), serde_json::json!(format!("r{i}"))])
                .collect();
            Ok(QueryResult {
                columns: vec![crate::types::Column::new("id"), crate::types::Column::new("name")],
                rows,
                ..QueryResult::empty()
            })
        }
        // NB: no `export_to_writer` override — the buffered default is under test.
        async fn completion(
            &self,
            _: &ResolvedConfig,
            _: &CompletionContext,
        ) -> Result<CompletionResponse> {
            unreachable!()
        }
    }

    /// The buffered fallback refuses a result larger than the cap (a full export
    /// — `max_rows: None` — that would materialise too much in RAM), rather than
    /// silently OOMing. The probe runs at cap + 1, so cap + 1 rows trips it.
    #[tokio::test]
    async fn default_export_to_writer_refuses_over_cap() {
        let d = BufferedExportDriver {
            rows: BUFFERED_EXPORT_ROW_CAP + 1,
        };
        let sink: Box<dyn std::io::Write + Send> = Box::new(std::io::sink());
        let err = d
            .export_to_writer(&cfg(), "GET *", None, ExportFormat::CsvWithNames, None, sink)
            .await
            .expect_err("over-cap export must be refused");
        assert!(
            matches!(err, otto_core::Error::Invalid(_)),
            "expected Error::Invalid, got {err:?}"
        );
    }

    /// The buffered fallback within the cap writes the expected bytes through the
    /// boxed writer (golden CSV-with-names).
    #[tokio::test]
    async fn default_export_to_writer_writes_buffered_bytes() {
        let d = BufferedExportDriver { rows: 3 };
        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let w: Box<dyn std::io::Write + Send> = Box::new(SharedWriter(buf.clone()));
        let counts = d
            .export_to_writer(&cfg(), "GET *", None, ExportFormat::CsvWithNames, None, w)
            .await
            .expect("export ok");
        assert_eq!(counts.rows, 3);
        let got = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert_eq!(got, "id,name\n0,r0\n1,r1\n2,r2\n");
    }

    /// `export_to_path` (the provided file wrapper) writes byte-identical output
    /// to what `export_to_writer` produced — proving the wrapper just opens a file
    /// and delegates.
    #[tokio::test]
    async fn export_to_path_wrapper_writes_identical_bytes_to_file() {
        let d = BufferedExportDriver { rows: 3 };
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("export.csv");
        let counts = d
            .export_to_path(&cfg(), "GET *", None, ExportFormat::CsvWithNames, None, &dest)
            .await
            .expect("export_to_path ok");
        assert_eq!(counts.rows, 3);
        let got = std::fs::read_to_string(&dest).unwrap();
        assert_eq!(got, "id,name\n0,r0\n1,r1\n2,r2\n");
    }
}
