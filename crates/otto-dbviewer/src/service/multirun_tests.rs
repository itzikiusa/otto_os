//! End-to-end multi-run tests over a real (in-memory) state DB and stub
//! drivers: every run goes through `DbViewerService::run`, so these exercise
//! the write-guard, history and cancel plumbing a real run uses.

use super::*;
use crate::multirun::{MultiRunParam, MultiRunTarget, VarType};
use crate::types::{Capabilities, CompletionResponse, ObjectDetail, SchemaNode};
use otto_core::domain::{ConnectionKind, Environment};

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

/// Records every executed (node, statement); fails a statement containing
/// `boom`; parks a statement containing `slow` until the run is dropped.
/// Answers the ClickHouse topology probes with a 3-host `main` cluster.
struct Stub {
    engine: Engine,
    ran: std::sync::Mutex<Vec<(Option<String>, String)>>,
}

impl Stub {
    fn new(engine: Engine) -> Arc<Self> {
        Arc::new(Self {
            engine,
            ran: std::sync::Mutex::new(Vec::new()),
        })
    }
    fn ran(&self) -> Vec<(Option<String>, String)> {
        self.ran.lock().unwrap().clone()
    }
}

fn rows(cols: &[&str], data: Vec<Vec<Value>>) -> QueryResult {
    QueryResult {
        columns: cols.iter().map(|c| crate::types::Column::new(*c)).collect(),
        stats: crate::types::QueryStats {
            row_count: data.len(),
            ..Default::default()
        },
        rows: data,
        ..QueryResult::empty()
    }
}

#[async_trait::async_trait]
impl Driver for Stub {
    fn engine(&self) -> Engine {
        self.engine
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
        let s = req.statement.clone();
        if s.contains("system.macros") {
            return Ok(rows(
                &["macro", "substitution"],
                vec![vec![json!("cluster"), json!("main")]],
            ));
        }
        if s.contains("system.clusters") {
            return Ok(rows(
                &["cluster", "hosts", "local_hosts", "remote_hosts"],
                vec![
                    vec![json!("default"), json!("1"), json!("1"), json!("0")],
                    vec![json!("main"), json!("3"), json!("1"), json!("3")],
                ],
            ));
        }
        if s.contains("system.databases") {
            return Ok(rows(&["engine"], vec![vec![json!("Atomic")]]));
        }
        self.ran.lock().unwrap().push((req.node.clone(), s.clone()));
        if s.contains("slow") {
            std::future::pending::<()>().await;
        }
        if s.contains("boom") {
            return Err(Error::Upstream("boom: table missing".into()));
        }
        Ok(rows(&["v"], vec![vec![json!(1)]]))
    }
    async fn completion(
        &self,
        _: &ResolvedConfig,
        _: &CompletionContext,
    ) -> Result<CompletionResponse> {
        unreachable!()
    }
}

use serde_json::json;

struct Fx {
    svc: DbViewerService,
    user: Id,
    pool: sqlx::SqlitePool,
}

async fn fixture() -> Fx {
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
        .create("fixture", "", "Fixture", true)
        .await
        .unwrap();
    let svc = DbViewerService::new(
        ConnectionsRepo::new(pool.clone()),
        Arc::new(NoSecrets),
        DbExplorerRepo::new(pool.clone()),
    );
    Fx {
        svc,
        user: user.id,
        pool,
    }
}

impl Fx {
    async fn conn(&self, name: &str, kind: ConnectionKind, env: Environment) -> Id {
        ConnectionsRepo::new(self.pool.clone())
            .create(otto_state::NewConnection {
                workspace_id: None,
                name: name.into(),
                kind,
                params: json!({"host": "127.0.0.1", "port": 1}),
                secret_ref: None,
                first_command: None,
                section_id: None,
                environment: env,
                read_only: false,
                created_by: self.user.clone(),
            })
            .await
            .unwrap()
            .id
    }

    async fn wait(&self, id: &str) -> crate::multirun::MultiRunJobView {
        for _ in 0..400 {
            let j = self.svc.multi_run_get(&self.user, false, id).unwrap();
            if j.status != JobStatus::Running {
                return j;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("multi-run did not finish");
    }
}

fn target(conn: &Id, node: Option<&str>) -> MultiRunTarget {
    MultiRunTarget {
        connection_id: conn.clone(),
        node: node.map(str::to_string),
        cluster_mode: ClusterMode::Auto,
        cluster_name: None,
    }
}

fn spec(statement: &str, targets: Vec<MultiRunTarget>, params: Vec<MultiRunParam>) -> MultiRunSpec {
    MultiRunSpec {
        statement: statement.into(),
        targets,
        params,
        max_rows: None,
        timeout_ms: None,
        mask: None,
        read_only: false,
    }
}

fn param(name: &str, values: &[&str], ty: VarType) -> MultiRunParam {
    MultiRunParam {
        name: name.into(),
        values: values.iter().map(|s| s.to_string()).collect(),
        var_type: ty,
        escape: true,
    }
}

fn start(s: MultiRunSpec) -> StartMultiRunReq {
    StartMultiRunReq {
        spec: s,
        concurrency: None,
        stop_on_error: true,
        confirm_write: false,
        plan_hash: None,
    }
}

#[tokio::test]
async fn targets_times_values_each_run_separately_and_land_in_history() {
    let mut fx = fixture().await;
    let stub = Stub::new(Engine::Mongodb);
    fx.svc.registry.set_for_test(Engine::Mongodb, stub.clone());
    let stg = fx
        .conn("mongo-stg", ConnectionKind::Mongodb, Environment::Staging)
        .await;
    let dev = fx
        .conn("mongo-dev", ConnectionKind::Mongodb, Environment::Dev)
        .await;
    let s = spec(
        "db.orders.find({brand: :brand})",
        vec![target(&stg, Some("shop")), target(&dev, None)],
        vec![param("brand", &["1", "2"], VarType::Number)],
    );
    let plan = fx.svc.multi_run_plan(&fx.user, &s).await.unwrap();
    assert_eq!(plan.runs.len(), 4);
    assert_eq!(plan.runs[0].statement, "db.orders.find({brand: 1})");
    assert_eq!(plan.runs[0].label, "mongo-stg · shop · brand=1");
    assert_eq!(plan.runs[3].label, "mongo-dev · brand=2");
    assert!(!plan.needs_confirm);

    let (job, _) = fx.svc.multi_run_start(&fx.user, &start(s)).await.unwrap();
    let done = fx.wait(&job.id).await;
    assert_eq!(done.status, JobStatus::Done);
    assert_eq!((done.summary.ok, done.summary.failed), (4, 0));
    assert_eq!(
        stub.ran(),
        vec![
            (Some("shop".into()), "db.orders.find({brand: 1})".into()),
            (Some("shop".into()), "db.orders.find({brand: 2})".into()),
            (None, "db.orders.find({brand: 1})".into()),
            (None, "db.orders.find({brand: 2})".into()),
        ]
    );
    let item = fx.svc.multi_run_item(&fx.user, false, &job.id, 2).unwrap();
    assert_eq!(item.connection_id, dev);
    assert_eq!(item.result.unwrap().rows, vec![vec![json!(1)]]);
    // Every run is an ordinary history row on its own connection.
    assert_eq!(fx.svc.list_history(&stg, 10).await.unwrap().len(), 2);
    assert_eq!(fx.svc.list_history(&dev, 10).await.unwrap().len(), 2);
    // Another user cannot see the job.
    assert!(
        fx.svc
            .multi_run_get(&"someone-else".to_string(), false, &job.id)
            .is_err()
    );
    assert_eq!(fx.svc.multi_run_list(&fx.user, false).len(), 1);
}

#[tokio::test]
async fn stop_on_error_skips_the_rest_and_continue_runs_everything() {
    let mut fx = fixture().await;
    let stub = Stub::new(Engine::Mysql);
    fx.svc.registry.set_for_test(Engine::Mysql, stub.clone());
    let a = fx.conn("a", ConnectionKind::Mysql, Environment::Dev).await;
    let s = spec(
        "SELECT {{t}}",
        vec![target(&a, Some("d1"))],
        vec![param("t", &["'ok1'", "boom", "'ok2'"], VarType::Raw)],
    );
    let (job, _) = fx
        .svc
        .multi_run_start(&fx.user, &start(s.clone()))
        .await
        .unwrap();
    let done = fx.wait(&job.id).await;
    let st: Vec<RunStatus> = done.items.iter().map(|i| i.status).collect();
    assert_eq!(
        st,
        vec![RunStatus::Ok, RunStatus::Failed, RunStatus::Skipped]
    );
    assert!(done.items[1].error.as_deref().unwrap().contains("boom"));

    let mut keep_going = start(s);
    keep_going.stop_on_error = false;
    let (job, _) = fx.svc.multi_run_start(&fx.user, &keep_going).await.unwrap();
    let done = fx.wait(&job.id).await;
    assert_eq!((done.summary.ok, done.summary.failed), (2, 1));
}

#[tokio::test]
async fn guarded_writes_are_refused_up_front_without_confirmation() {
    let mut fx = fixture().await;
    let stub = Stub::new(Engine::Mysql);
    fx.svc.registry.set_for_test(Engine::Mysql, stub.clone());
    let stg = fx
        .conn("stg", ConnectionKind::Mysql, Environment::Staging)
        .await;
    let prod = fx
        .conn("prod", ConnectionKind::Mysql, Environment::Prod)
        .await;
    let s = spec(
        "UPDATE t SET a = 1",
        vec![target(&stg, None), target(&prod, None)],
        vec![],
    );
    let plan = fx.svc.multi_run_plan(&fx.user, &s).await.unwrap();
    assert!(plan.needs_confirm);
    assert_eq!(
        plan.runs
            .iter()
            .map(|r| r.needs_confirm)
            .collect::<Vec<_>>(),
        vec![false, true]
    );
    let err = fx
        .svc
        .multi_run_start(&fx.user, &start(s.clone()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains(WRITE_BLOCKED_PREFIX), "{err}");
    assert!(
        stub.ran().is_empty(),
        "nothing runs when a guarded write is unconfirmed"
    );

    // A read-only multi-run refuses writes outright.
    let mut ro = start(s.clone());
    ro.spec.read_only = true;
    let err = fx.svc.multi_run_start(&fx.user, &ro).await.unwrap_err();
    assert!(err.to_string().contains(READ_ONLY_PREFIX), "{err}");

    // A stale preview is refused.
    let mut stale = start(s.clone());
    stale.confirm_write = true;
    stale.plan_hash = Some("deadbeef".into());
    let err = fx.svc.multi_run_start(&fx.user, &stale).await.unwrap_err();
    assert!(err.to_string().contains("plan_changed"), "{err}");

    let mut ok = start(s);
    ok.confirm_write = true;
    ok.plan_hash = Some(plan.plan_hash.clone());
    let (job, _) = fx.svc.multi_run_start(&fx.user, &ok).await.unwrap();
    let done = fx.wait(&job.id).await;
    if fx.svc.is_enforced(&prod).await.unwrap() {
        // An access-enforced production connection still refuses the direct
        // write (reviewed-change flow) — the confirmation is not a grant, and
        // the preview said so up front.
        assert!(plan.warnings.iter().any(|w| w.contains("access-enforced")));
        assert_eq!(done.items[0].status, RunStatus::Ok);
        assert_eq!(done.items[1].status, RunStatus::Failed);
        assert!(
            done.items[1]
                .error
                .as_deref()
                .unwrap()
                .contains("review_required")
        );
        assert_eq!(stub.ran().len(), 1);
    } else {
        assert_eq!(done.summary.ok, 2);
        assert_eq!(stub.ran().len(), 2);
    }
}

#[tokio::test]
async fn clickhouse_ddl_gets_on_cluster_and_dml_does_not() {
    let mut fx = fixture().await;
    let stub = Stub::new(Engine::Clickhouse);
    fx.svc
        .registry
        .set_for_test(Engine::Clickhouse, stub.clone());
    let ch = fx
        .conn("ch-stg", ConnectionKind::Clickhouse, Environment::Staging)
        .await;
    let mut off = target(&ch, Some("b"));
    off.cluster_mode = ClusterMode::Off;
    let mut custom = target(&ch, Some("c"));
    custom.cluster_mode = ClusterMode::Custom;
    custom.cluster_name = Some("other-c".into());
    let s = spec(
        "ALTER TABLE events ADD COLUMN x UInt8; INSERT INTO events VALUES (1)",
        vec![target(&ch, Some("a")), off, custom],
        vec![],
    );
    let plan = fx.svc.multi_run_plan(&fx.user, &s).await.unwrap();
    let auto = plan.targets[0].cluster.as_ref().unwrap();
    assert_eq!(auto.source, ClusterSource::Macro);
    assert_eq!(auto.applied.as_deref(), Some("main"));
    assert_eq!(auto.database_engine.as_deref(), Some("Atomic"));
    assert_eq!(
        plan.runs[0].statement,
        "ALTER TABLE events ON CLUSTER main ADD COLUMN x UInt8; INSERT INTO events VALUES (1)"
    );
    assert_eq!(plan.runs[0].on_cluster, vec!["ALTER TABLE events"]);
    assert_eq!(
        plan.runs[1].statement,
        "ALTER TABLE events ADD COLUMN x UInt8; INSERT INTO events VALUES (1)"
    );
    assert_eq!(
        plan.runs[2].statement,
        "ALTER TABLE events ON CLUSTER 'other-c' ADD COLUMN x UInt8; INSERT INTO events VALUES (1)"
    );
    // A pure read never probes or rewrites.
    let read = spec("SELECT 1", vec![target(&ch, None)], vec![]);
    let plan = fx.svc.multi_run_plan(&fx.user, &read).await.unwrap();
    assert_eq!(
        plan.targets[0].cluster.as_ref().unwrap().source,
        ClusterSource::NotNeeded
    );
    assert_eq!(plan.runs[0].statement, "SELECT 1");
}

#[tokio::test]
async fn cancel_stops_the_running_run_and_the_pending_ones() {
    let mut fx = fixture().await;
    let stub = Stub::new(Engine::Mysql);
    fx.svc.registry.set_for_test(Engine::Mysql, stub.clone());
    let a = fx.conn("a", ConnectionKind::Mysql, Environment::Dev).await;
    let s = spec(
        "SELECT {{v}}",
        vec![target(&a, None)],
        vec![param("v", &["'slow'", "2", "3"], VarType::Raw)],
    );
    let (job, _) = fx.svc.multi_run_start(&fx.user, &start(s)).await.unwrap();
    for _ in 0..200 {
        if !stub.ran().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let view = fx.svc.multi_run_cancel(&fx.user, false, &job.id).unwrap();
    assert_eq!(view.items[1].status, RunStatus::Cancelled);
    let done = fx.wait(&job.id).await;
    assert_eq!(done.status, JobStatus::Cancelled);
    assert!(done.items.iter().all(|i| i.status == RunStatus::Cancelled));
    assert_eq!(stub.ran().len(), 1, "no run starts after a cancel");
}

#[tokio::test]
async fn invalid_specs_are_refused_before_anything_runs() {
    let mut fx = fixture().await;
    let stub = Stub::new(Engine::Mysql);
    fx.svc.registry.set_for_test(Engine::Mysql, stub.clone());
    let my = fx.conn("my", ConnectionKind::Mysql, Environment::Dev).await;
    let mongo = fx
        .conn("mo", ConnectionKind::Mongodb, Environment::Dev)
        .await;
    let cases = [
        spec(
            "SELECT 1",
            vec![target(&my, None), target(&mongo, None)],
            vec![],
        ),
        spec("SELECT :x", vec![target(&my, None)], vec![]),
        spec(
            "SELECT 1",
            vec![target(&my, None)],
            vec![param("unused", &["1", "2"], VarType::String)],
        ),
        spec(
            "SELECT :n",
            vec![target(&my, None)],
            vec![param("n", &["1; DROP TABLE t"], VarType::Number)],
        ),
        spec(
            "SELECT 1",
            vec![target(&my, Some("a")), target(&my, Some("a"))],
            vec![],
        ),
        spec("  ", vec![target(&my, None)], vec![]),
    ];
    for s in cases {
        let err = fx.svc.multi_run_plan(&fx.user, &s).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err}");
    }
    // A string value is quoted + escaped, never spliced raw.
    let s = spec(
        "SELECT * FROM t WHERE name = :n",
        vec![target(&my, None)],
        vec![param("n", &["x' OR '1'='1"], VarType::String)],
    );
    let plan = fx.svc.multi_run_plan(&fx.user, &s).await.unwrap();
    assert_eq!(
        plan.runs[0].statement,
        "SELECT * FROM t WHERE name = 'x'' OR ''1''=''1'"
    );
    assert!(stub.ran().is_empty());
}

#[tokio::test]
async fn since_returns_only_the_runs_changed_after_that_seq() {
    let mut fx = fixture().await;
    let stub = Stub::new(Engine::Mongodb);
    fx.svc.registry.set_for_test(Engine::Mongodb, stub.clone());
    let dev = fx
        .conn("mongo-dev", ConnectionKind::Mongodb, Environment::Dev)
        .await;
    let s = spec(
        "db.orders.find({brand: :brand})",
        vec![target(&dev, None)],
        vec![param("brand", &["1", "2", "3"], VarType::Number)],
    );
    let (job, _) = fx.svc.multi_run_start(&fx.user, &start(s)).await.unwrap();
    // The start answer is the full view at seq 0.
    assert_eq!(job.seq, 0);
    assert!(!job.partial);
    assert_eq!(job.items.len(), 3);
    let done = fx.wait(&job.id).await;
    // Each run changes twice (running, then finished) + the job settles.
    assert!(done.seq >= 7, "seq {}", done.seq);
    assert!(!done.partial);
    assert_eq!(done.targets.len(), 1);

    // Nothing changed after the final seq: an empty, partial delta that
    // still carries the status and summary.
    let none = fx
        .svc
        .multi_run_get_since(&fx.user, false, &job.id, Some(done.seq))
        .unwrap();
    assert!(none.partial);
    assert!(none.items.is_empty());
    assert!(none.targets.is_empty());
    assert_eq!(none.status, JobStatus::Done);
    assert_eq!(none.summary.ok, 3);

    // From seq 0 every run changed.
    let all = fx
        .svc
        .multi_run_get_since(&fx.user, false, &job.id, Some(0))
        .unwrap();
    assert!(all.partial);
    assert_eq!(all.items.len(), 3);

    // Runs execute in order (sequential): past the first run's last change
    // only the later runs come back.
    let after_first = {
        let j = fx.svc.multi_run_job(&fx.user, false, &job.id).unwrap();
        let j = lock(&j);
        j.items[0].seq
    };
    let later = fx
        .svc
        .multi_run_get_since(&fx.user, false, &job.id, Some(after_first))
        .unwrap();
    assert_eq!(
        later.items.iter().map(|i| i.index).collect::<Vec<_>>(),
        vec![1, 2]
    );

    // A seq from the future (another daemon) gets the full view.
    let ahead = fx
        .svc
        .multi_run_get_since(&fx.user, false, &job.id, Some(done.seq + 100))
        .unwrap();
    assert!(!ahead.partial);
    assert_eq!(ahead.items.len(), 3);
}
