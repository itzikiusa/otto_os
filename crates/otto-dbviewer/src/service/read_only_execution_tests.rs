//! A Read-only connection runs every statement in the engine's own read-only
//! mode (`__read_only_execution`: PostgreSQL / MySQL `READ ONLY` transaction,
//! ClickHouse `readonly=2`), not just on the MCP path: the classifier in
//! `read_only_guard` cannot see a write inside a function
//! (`SELECT my_writing_function()`), the database can.

use super::*;
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

/// Records, per run, the statement and whether the driver was told to run it
/// in the engine's read-only mode.
#[derive(Default)]
struct Recorder {
    ran: std::sync::Mutex<Vec<(String, bool)>>,
}

#[async_trait::async_trait]
impl Driver for Recorder {
    fn engine(&self) -> Engine {
        Engine::Postgres
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
    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        let native_read_only = cfg
            .params
            .get("__read_only_execution")
            .and_then(Value::as_bool)
            == Some(true);
        self.ran
            .lock()
            .unwrap()
            .push((req.statement.clone(), native_read_only));
        Ok(QueryResult::empty())
    }
    async fn completion(
        &self,
        _: &ResolvedConfig,
        _: &CompletionContext,
    ) -> Result<CompletionResponse> {
        unreachable!()
    }
}

/// A service with the recording Postgres driver and two Legacy connections
/// (policy row removed, so no access scope sets the flag for its own reasons):
/// one Read-only, one not.
async fn fixture() -> (DbViewerService, Arc<Recorder>, Id, Id, Id) {
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
        .create("ro-exec", "", "Read-only exec", true)
        .await
        .unwrap();
    let conns = ConnectionsRepo::new(pool.clone());
    let mut ids = Vec::new();
    for read_only in [true, false] {
        let conn = conns
            .create(otto_state::NewConnection {
                workspace_id: None,
                name: format!("pg-{read_only}"),
                kind: ConnectionKind::Postgres,
                params: serde_json::json!({"host":"127.0.0.1","port":1}),
                secret_ref: None,
                first_command: None,
                section_id: None,
                environment: Environment::Dev,
                read_only,
                created_by: user.id.clone(),
            })
            .await
            .unwrap();
        sqlx::query("DELETE FROM resource_access_policies WHERE resource_id = ?")
            .bind(&conn.id)
            .execute(&pool)
            .await
            .unwrap();
        ids.push(conn.id);
    }
    let mut service = DbViewerService::new(conns, Arc::new(NoSecrets), DbExplorerRepo::new(pool));
    let recorder = Arc::new(Recorder::default());
    service
        .registry
        .set_for_test(Engine::Postgres, recorder.clone());
    let writable = ids.pop().unwrap();
    let read_only = ids.pop().unwrap();
    (service, recorder, read_only, writable, user.id)
}

fn query(statement: &str) -> QueryRequest {
    QueryRequest {
        statement: statement.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_read_only_connection_runs_every_statement_in_native_read_only_mode() {
    let (svc, recorder, read_only, writable, user) = fixture().await;
    assert!(!svc.is_enforced(&read_only).await.unwrap());
    // The editor path (not the MCP `read_only` request) on a Read-only profile.
    svc.run(&read_only, &user, &query("SELECT my_writing_function()"))
        .await
        .expect("a classified read runs");
    // A write the classifier sees never reaches the driver.
    let refused = svc
        .run(&read_only, &user, &query("DELETE FROM t"))
        .await
        .expect_err("a read-only connection refuses a write");
    assert!(matches!(refused, Error::Forbidden(_)), "{refused:?}");
    // A connection that is not Read-only keeps its normal run path.
    svc.run(&writable, &user, &query("SELECT my_writing_function()"))
        .await
        .expect("a read runs");
    assert_eq!(
        *recorder.ran.lock().unwrap(),
        vec![
            ("SELECT my_writing_function()".to_string(), true),
            ("SELECT my_writing_function()".to_string(), false),
        ]
    );
}
