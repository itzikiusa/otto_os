//! The engine-agnostic driver contract. Each engine implements this in
//! `drivers/<engine>.rs`. Methods receive a fully [`ResolvedConfig`] — any SSH
//! tunnel is already established by the service, so `host`/`port` are reachable.

use async_trait::async_trait;
use otto_core::Result;

use crate::types::{
    Capabilities, CancelToken, CompletionContext, CompletionResponse, Engine, NodePath,
    ObjectDetail, QueryHandle, QueryRequest, QueryResult, ResolvedConfig, SchemaNode, TestResult,
};

#[async_trait]
pub trait Driver: Send + Sync {
    /// The engine this driver serves.
    fn engine(&self) -> Engine;

    /// Static capabilities (drives UI affordances).
    fn capabilities(&self) -> Capabilities;

    /// Connect and run a cheap probe (ping / SELECT 1). Reports latency +
    /// server version.
    async fn test(&self, cfg: &ResolvedConfig) -> Result<TestResult>;

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

    /// Autocomplete items for the editor, scoped to the given context.
    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse>;
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
}
