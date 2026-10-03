//! Perf budgets for the service layer (perf DB-06 / DB-10): state-DB reads per
//! Run and per tree/search request, and single-flight schema-graph builds.
//! Reads are counted by `access::reads` (a per-thread counter bumped by every
//! connection-row / policy / user read the access path makes); the default
//! `#[tokio::test]` runtime is single-threaded, so the count is this test's.

use super::*;
use crate::types::{CompletionResponse, GraphTable, ObjectDetail, SchemaNode};
use std::sync::atomic::{AtomicUsize, Ordering};

struct NoSecrets;
impl SecretStore for NoSecrets {
    fn put(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn get(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn delete(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

/// A connection with a fresh ENFORCED policy (what `ConnectionsRepo::create`
/// gives a new connection), or — `legacy` — with the policy row removed, the
/// rollout-compatible Legacy mode older connections run in.
async fn fixture_mode(legacy: bool) -> (DbViewerService, Id, Id) {
    let (service, conn, user) = fixture().await;
    if legacy {
        sqlx::query("DELETE FROM resource_access_policies WHERE resource_id = ?")
            .bind(&conn)
            .execute(&service.connections.pool())
            .await
            .unwrap();
        assert!(!service.is_enforced(&conn).await.unwrap());
    } else {
        assert!(service.is_enforced(&conn).await.unwrap());
    }
    (service, conn, user)
}

async fn fixture() -> (DbViewerService, Id, Id) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let user = otto_state::UsersRepo::new(pool.clone())
        .create("budget", "", "Budget", true)
        .await
        .unwrap();
    let conns = ConnectionsRepo::new(pool.clone());
    let conn = conns
        .create(otto_state::NewConnection {
            workspace_id: None,
            name: "budget".into(),
            kind: otto_core::domain::ConnectionKind::Mysql,
            params: serde_json::json!({"host":"127.0.0.1","port":1}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: otto_core::domain::Environment::Dev,
            read_only: false,
            created_by: user.id.clone(),
        })
        .await
        .unwrap();
    (
        DbViewerService::new(conns, Arc::new(NoSecrets), DbExplorerRepo::new(pool)),
        conn.id,
        user.id,
    )
}

/// Answers runs instantly, lists `roots` databases, and counts bulk graph
/// builds (each parks briefly so concurrent callers overlap).
struct Stub {
    roots: usize,
    bulk_calls: AtomicUsize,
}

#[async_trait::async_trait]
impl Driver for Stub {
    fn engine(&self) -> Engine {
        Engine::Mysql
    }
    fn capabilities(&self) -> Capabilities {
        crate::drivers::mysql::MysqlDriver::default().capabilities()
    }
    async fn test(&self, _: &ResolvedConfig) -> Result<TestResult> {
        unreachable!()
    }
    async fn schema_root(&self, _: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
        Ok((0..self.roots)
            .map(|i| SchemaNode::new(format!("db:d{i}"), format!("d{i}"), NodeKind::Database))
            .collect())
    }
    async fn schema_children(
        &self,
        _: &ResolvedConfig,
        _: &NodePath,
        _: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        unreachable!("the bulk path must not walk the tree")
    }
    async fn object_detail(&self, _: &ResolvedConfig, _: &NodePath) -> Result<ObjectDetail> {
        unreachable!("the bulk path must not introspect per object")
    }
    async fn run(&self, _: &ResolvedConfig, _: &QueryRequest) -> Result<QueryResult> {
        Ok(QueryResult::message("ok"))
    }
    async fn completion(
        &self,
        _: &ResolvedConfig,
        _: &crate::types::CompletionContext,
    ) -> Result<CompletionResponse> {
        unreachable!()
    }
    async fn schema_graph_bulk(
        &self,
        _: &ResolvedConfig,
        schema: &str,
        _: usize,
    ) -> Result<Option<SchemaGraph>> {
        self.bulk_calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(Some(SchemaGraph {
            schema: schema.into(),
            tables: vec![GraphTable {
                id: format!("db:{schema}/table:t"),
                schema: schema.into(),
                name: "t".into(),
                kind: NodeKind::Table,
                columns: Vec::new(),
            }],
            edges: Vec::new(),
            relationships: true,
            truncated: false,
        }))
    }
}

fn stub(roots: usize) -> Arc<Stub> {
    Arc::new(Stub {
        roots,
        bulk_calls: AtomicUsize::new(0),
    })
}

async fn run_reads(legacy: bool) -> u64 {
    let (mut service, conn, user) = fixture_mode(legacy).await;
    service.registry.set_for_test(Engine::Mysql, stub(0));
    crate::access::reads::take();
    let req = QueryRequest {
        statement: "SELECT 1".into(),
        ..Default::default()
    };
    service.run(&conn, &user, &req).await.unwrap();
    let reads = crate::access::reads::take();
    eprintln!("Run state reads (legacy={legacy}): {reads}");
    reads
}

/// A legacy Run: one snapshot for the pre-execution phase, one for the
/// eligibility check before the driver call and one after — 6 counted state
/// reads (was ~13: conn + policy re-read by every guard, scope check and
/// resolution, then again by both eligibility checks).
#[tokio::test]
async fn legacy_run_stays_within_state_read_budget() {
    let reads = run_reads(true).await;
    assert!(reads <= 6, "legacy Run made {reads} state reads (budget 6)");
}

/// An enforced Run (root caller) re-validates the caller and scope before and
/// after the driver call; the conn/policy rows are shared per phase. Ratchet.
#[tokio::test]
async fn enforced_run_stays_within_state_read_budget() {
    let reads = run_reads(false).await;
    assert!(
        reads <= 13,
        "enforced Run made {reads} state reads (budget 13)"
    );
}

async fn root_reads(legacy: bool, roots: usize) -> u64 {
    let (mut service, conn, user) = fixture_mode(legacy).await;
    service.registry.set_for_test(Engine::Mysql, stub(roots));
    crate::access::reads::take();
    let nodes = service.schema_root(&conn, &user).await.unwrap();
    assert_eq!(nodes.len(), roots);
    crate::access::reads::take()
}

/// The tree root authorizes every database with ONE snapshot and one caller
/// load: the read count must not grow with the number of databases (500
/// databases used to cost ~1,000 reads).
#[tokio::test]
async fn schema_root_reads_do_not_scale_with_database_count() {
    for legacy in [true, false] {
        let few = root_reads(legacy, 5).await;
        let many = root_reads(legacy, 500).await;
        assert_eq!(few, many, "legacy={legacy}: reads grew with the root count");
        let budget = if legacy { 4 } else { 8 };
        assert!(
            many <= budget,
            "legacy={legacy}: {many} reads (budget {budget})"
        );
    }
}

/// Ten concurrent diagram/assistant requests build the graph once (bulk read,
/// no per-table walk); a later request is served from cache; refresh clears it.
#[tokio::test]
async fn schema_graph_is_single_flight_and_cached() {
    let (mut service, conn, user) = fixture().await;
    let driver = stub(0);
    service.registry.set_for_test(Engine::Mysql, driver.clone());
    let calls = (0..10).map(|_| service.schema_graph(&conn, &user, "shop", 300));
    for graph in futures_util::future::join_all(calls).await {
        assert_eq!(graph.unwrap().tables.len(), 1);
    }
    assert_eq!(driver.bulk_calls.load(Ordering::SeqCst), 1);
    service
        .schema_graph(&conn, &user, "shop", 300)
        .await
        .unwrap();
    assert_eq!(driver.bulk_calls.load(Ordering::SeqCst), 1);
    service
        .refresh_completion_cache(&conn, &user)
        .await
        .unwrap();
    service
        .schema_graph(&conn, &user, "shop", 300)
        .await
        .unwrap();
    assert_eq!(driver.bulk_calls.load(Ordering::SeqCst), 2);
}
