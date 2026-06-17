//! The engine-agnostic driver contract. Each engine implements this in
//! `drivers/<engine>.rs`. Methods receive a fully [`ResolvedConfig`] — any SSH
//! tunnel is already established by the service, so `host`/`port` are reachable.

use async_trait::async_trait;
use otto_core::Result;

use crate::types::{
    Capabilities, CompletionContext, CompletionResponse, Engine, NodePath, ObjectDetail,
    QueryRequest, QueryResult, ResolvedConfig, SchemaNode, TestResult,
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

    /// Children of a tree node (lazy expansion).
    async fn schema_children(&self, cfg: &ResolvedConfig, parent: &NodePath)
        -> Result<Vec<SchemaNode>>;

    /// Full structure of a selected object (columns, keys, indexes, DDL).
    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail>;

    /// Execute a query / command and return a tabular result.
    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult>;

    /// Autocomplete items for the editor, scoped to the given context.
    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse>;
}
