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
    fk_scopes: Option<Vec<Option<String>>>,
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
        let mut detail = ObjectDetail::new("t", NodeKind::Table);
        detail.foreign_keys = (0..self.roots)
            .map(|i| crate::types::ForeignKey {
                name: format!("fk{i}"),
                columns: vec!["id".into()],
                ref_table: format!("target{i}"),
                ref_columns: vec!["id".into()],
                ref_schema: self
                    .fk_scopes
                    .as_ref()
                    .map_or_else(|| Some("shop".into()), |scopes| scopes[i].clone()),
            })
            .collect();
        detail.ddl = Some("CREATE TABLE t (id INT)".into());
        detail.extra = serde_json::json!({"private": true});
        Ok(detail)
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
        fk_scopes: None,
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

/// A legacy Run: one snapshot for the pre-execution phase (which the
/// eligibility check right before the driver call reuses, DB2-06) and one
/// after — 4 counted state reads (was ~13: conn + policy re-read by every
/// guard, scope check and resolution, then again by both eligibility checks;
/// 6 before the reuse).
#[tokio::test]
async fn legacy_run_stays_within_state_read_budget() {
    let reads = run_reads(true).await;
    assert!(reads <= 4, "legacy Run made {reads} state reads (budget 4)");
}

/// An enforced Run (root caller) re-validates the caller and scope before and
/// after the driver call; the conn/policy rows are shared per phase. Ratchet.
#[tokio::test]
async fn enforced_run_stays_within_state_read_budget() {
    let reads = run_reads(false).await;
    assert!(
        reads <= 11,
        "enforced Run made {reads} state reads (budget 11)"
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

// --- Driver round-trip counter (DB2-04) ---------------------------------------

/// Counts every driver entry point a service call reaches. A Run must be ONE
/// statement execution and nothing else: no tree walk, object introspection,
/// completion or graph build sneaking onto the hot path.
#[derive(Default)]
struct Counting {
    runs: AtomicUsize,
    catalog: AtomicUsize,
}

impl Counting {
    fn catalog_call(&self) {
        self.catalog.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl Driver for Counting {
    fn engine(&self) -> Engine {
        Engine::Mysql
    }
    fn capabilities(&self) -> Capabilities {
        crate::drivers::mysql::MysqlDriver::default().capabilities()
    }
    async fn test(&self, _: &ResolvedConfig) -> Result<TestResult> {
        self.catalog_call();
        Err(otto_core::Error::Invalid("counting stub".into()))
    }
    async fn native_grants(
        &self,
        _: &ResolvedConfig,
    ) -> Result<Vec<crate::native_access::NativeGrant>> {
        self.catalog_call();
        Ok(Vec::new())
    }
    async fn schema_root(&self, _: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
        self.catalog_call();
        Ok(Vec::new())
    }
    async fn schema_children(
        &self,
        _: &ResolvedConfig,
        _: &NodePath,
        _: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        self.catalog_call();
        Ok(Vec::new())
    }
    async fn schema_children_with_counts(
        &self,
        _: &ResolvedConfig,
        _: &NodePath,
        _: Option<&str>,
        _: bool,
    ) -> Result<Vec<SchemaNode>> {
        self.catalog_call();
        Ok(Vec::new())
    }
    async fn search_objects(
        &self,
        _: &ResolvedConfig,
        _: &crate::types::ObjectSearchReq,
    ) -> Result<crate::types::ObjectSearchResult> {
        self.catalog_call();
        Ok(Default::default())
    }
    async fn object_detail(&self, _: &ResolvedConfig, _: &NodePath) -> Result<ObjectDetail> {
        self.catalog_call();
        Err(otto_core::Error::Invalid("counting stub".into()))
    }
    async fn object_detail_with_opts(
        &self,
        _: &ResolvedConfig,
        _: &NodePath,
        _: bool,
    ) -> Result<ObjectDetail> {
        self.catalog_call();
        Err(otto_core::Error::Invalid("counting stub".into()))
    }
    async fn run(&self, _: &ResolvedConfig, _: &QueryRequest) -> Result<QueryResult> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(QueryResult::message("ok"))
    }
    async fn query_plan(
        &self,
        _: &ResolvedConfig,
        _: &str,
        _: Option<&str>,
    ) -> Result<crate::types::DbQueryPlan> {
        self.catalog_call();
        Err(otto_core::Error::Invalid("counting stub".into()))
    }
    async fn completion(
        &self,
        _: &ResolvedConfig,
        _: &crate::types::CompletionContext,
    ) -> Result<CompletionResponse> {
        self.catalog_call();
        Err(otto_core::Error::Invalid("counting stub".into()))
    }
    async fn schema_graph_bulk(
        &self,
        _: &ResolvedConfig,
        _: &str,
        _: usize,
    ) -> Result<Option<SchemaGraph>> {
        self.catalog_call();
        Ok(None)
    }
}

/// One Run (legacy and enforced) = exactly ONE driver execution and ZERO
/// catalog calls. `run_tracked` keeps the trait default (delegates to `run`),
/// so whichever entry point the service uses is counted once.
#[tokio::test]
async fn run_issues_one_driver_execution_and_no_catalog_calls() {
    for legacy in [true, false] {
        let (mut service, conn, user) = fixture_mode(legacy).await;
        let driver = Arc::new(Counting::default());
        service.registry.set_for_test(Engine::Mysql, driver.clone());
        let req = QueryRequest {
            statement: "SELECT * FROM t".into(),
            ..Default::default()
        };
        service.run(&conn, &user, &req).await.unwrap();
        assert_eq!(
            driver.runs.load(Ordering::SeqCst),
            1,
            "legacy={legacy}: a Run must execute the statement exactly once"
        );
        assert_eq!(
            driver.catalog.load(Ordering::SeqCst),
            0,
            "legacy={legacy}: a Run must not touch the catalog"
        );
    }
}

// --- Serialisation timing (DB2-04) --------------------------------------------

/// A synthetic `rows × cols` page shaped like a wide real table: ints, short
/// text, a decimal string, a timestamp string, nulls and a small JSON object.
pub(crate) fn synthetic_result(rows: usize, cols: usize) -> QueryResult {
    use serde_json::{json, Value};
    let columns = (0..cols)
        .map(|c| crate::types::Column::typed(format!("col_{c}"), "VARCHAR"))
        .collect();
    let rows = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| match c % 6 {
                    0 => json!(r as i64 * 31 + c as i64),
                    1 => Value::String(format!("customer-{r}-{c}@example.com")),
                    2 => Value::String(format!("{}.{:02}", r * 7, c)),
                    3 => Value::String("2026-10-03 17:14:25".into()),
                    4 => Value::Null,
                    _ => json!({ "k": r, "tags": ["a", "b"] }),
                })
                .collect()
        })
        .collect();
    QueryResult {
        columns,
        rows,
        ..QueryResult::empty()
    }
}

/// Ceiling that runs in CI: serialising a 10k×30 page (the shape of a "fetch
/// 10,000 rows" Run) stays well under a second even in a debug build. Loose on
/// purpose — it catches an accidental quadratic, not a 10% drift.
#[test]
fn query_result_serialise_10k_x_30_within_ceiling() {
    let r = synthetic_result(10_000, 30);
    let t = std::time::Instant::now();
    let bytes = serde_json::to_vec(&r).unwrap();
    let took = t.elapsed();
    assert!(bytes.len() > 10_000 * 30 * 4);
    assert!(
        took < Duration::from_secs(3),
        "10k×30 QueryResult serialise took {took:?} (ceiling 3 s)"
    );
}

/// Bench (run on demand): `cargo test -p otto-dbviewer --lib bench_ -- --ignored
/// --nocapture`. 100k×30 serialise — the WS/HTTP response cost of a big page.
#[test]
#[ignore]
fn bench_query_result_serialise_100k_x_30() {
    let r = synthetic_result(100_000, 30);
    let t = std::time::Instant::now();
    let bytes = serde_json::to_vec(&r).unwrap();
    eprintln!(
        "bench serialise QueryResult 100k×30: {:?} ({} MB)",
        t.elapsed(),
        bytes.len() / (1024 * 1024)
    );
}

#[tokio::test]
async fn object_detail_reads_do_not_scale_with_repeated_foreign_key_scopes() {
    let mut counts = Vec::new();
    for keys in [5, 500] {
        let (mut service, conn, user) = fixture().await;
        service.registry.set_for_test(Engine::Mysql, stub(keys));
        crate::access::reads::take();
        let detail = service
            .object_detail(&conn, &user, "db:shop/table:t", false)
            .await
            .unwrap();
        assert_eq!(detail.foreign_keys.len(), keys);
        let reads = crate::access::reads::take();
        eprintln!("Object detail with {keys} foreign keys: {reads} state reads");
        counts.push(reads);
    }
    assert_eq!(
        counts[0], counts[1],
        "same-schema foreign keys must share authorization reads"
    );
}

#[tokio::test]
async fn object_detail_filters_mixed_fk_scopes_without_changing_order() {
    use otto_core::access::*;
    use otto_core::domain::{Capability, Feature};
    let (mut service, conn, root) = fixture().await;
    let pool = service.connections.pool();
    let reader = otto_state::UsersRepo::new(pool.clone())
        .create("reader", "", "Reader", false)
        .await
        .unwrap();
    otto_state::GrantsRepo::new(pool.clone())
        .set_grants(&reader.id, &[(Feature::Database, Capability::View)])
        .await
        .unwrap();
    let repo = otto_state::resource_access::ResourceAccessRepo::new(pool.clone());
    let mut policy = repo
        .get_policy(ResourceKind::Connection, &conn)
        .await
        .unwrap();
    policy.rules = vec![AccessRule {
        id: "shop-only".into(),
        subject_kind: SubjectKind::User,
        subject_id: reader.id.clone(),
        effect: RuleEffect::Allow,
        operations: vec!["discover".into(), "db_browse".into()],
        children: Some(vec!["shop".into()]),
        grantable_operations: vec![],
        credential_connection_id: None,
    }];
    repo.put_policy(
        &policy,
        policy.revision,
        &AccessActor {
            real_user_id: root,
            effective_user_id: None,
        },
    )
    .await
    .unwrap();
    // Omitted references inherit the source; empty references remain global.
    let scopes = vec![
        Some("shop".into()),
        Some("hidden".into()),
        None,
        Some("db:shop".into()),
        Some("".into()),
        Some("shop".into()),
    ];
    service.registry.set_for_test(
        Engine::Mysql,
        Arc::new(Stub {
            roots: scopes.len(),
            fk_scopes: Some(scopes),
            bulk_calls: AtomicUsize::new(0),
        }),
    );
    let detail = service
        .object_detail(&conn, &reader.id, "db:shop/table:t", false)
        .await
        .unwrap();
    assert_eq!(
        detail
            .foreign_keys
            .iter()
            .map(|k| k.name.as_str())
            .collect::<Vec<_>>(),
        vec!["fk0", "fk2", "fk3", "fk5"]
    );
    assert!(detail.ddl.is_none());
    assert_eq!(detail.extra, Value::Null);
}
