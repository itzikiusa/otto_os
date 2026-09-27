use super::*;
use crate::resource_cache::ResourceCache;
use crate::types::{Capabilities, CompletionResponse, ObjectDetail, SchemaNode};
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
        .create("fixture", "", "Fixture", true)
        .await
        .unwrap();
    let conns = ConnectionsRepo::new(pool.clone());
    let conn = conns
        .create(otto_state::NewConnection {
            workspace_id: None,
            name: "fixture".into(),
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

#[tokio::test]
async fn lifecycle_close_after_resolve_before_acquire_cannot_reopen() {
    let (service, conn, user) = fixture().await;
    let resolved = service
        .resolve(&conn, &user, None, "db_query")
        .await
        .unwrap();
    service.close_connection(&conn).await.unwrap();
    let calls = AtomicUsize::new(0);
    let cache = ResourceCache::default();
    let stale = resolved
        .with_lifecycle(async {
            cache
                .get_or_try_init(
                    resolved.config.cache_key(),
                    resolved.config.lifecycle.as_ref(),
                    |_| true,
                    async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(7)
                    },
                )
                .await
        })
        .await;
    assert!(stale.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let fresh = service
        .resolve(&conn, &user, None, "db_query")
        .await
        .unwrap();
    assert_eq!(
        fresh
            .with_lifecycle(async {
                cache
                    .get_or_try_init(
                        fresh.config.cache_key(),
                        fresh.config.lifecycle.as_ref(),
                        |_| true,
                        async { Ok(8) },
                    )
                    .await
            })
            .await
            .unwrap(),
        8
    );
}

struct DropCount(Arc<AtomicUsize>);
impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct CancelFixture {
    held: ResourceCache<Arc<DropCount>>,
    started: Arc<tokio::sync::Semaphore>,
    release: Arc<tokio::sync::Semaphore>,
}
#[async_trait::async_trait]
impl Driver for CancelFixture {
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
        _: &crate::types::NodePath,
        _: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        unreachable!()
    }
    async fn object_detail(
        &self,
        _: &ResolvedConfig,
        _: &crate::types::NodePath,
    ) -> Result<ObjectDetail> {
        unreachable!()
    }
    async fn run(&self, _: &ResolvedConfig, _: &QueryRequest) -> Result<QueryResult> {
        unreachable!()
    }
    async fn completion(
        &self,
        _: &ResolvedConfig,
        _: &crate::types::CompletionContext,
    ) -> Result<CompletionResponse> {
        unreachable!()
    }
    fn detach(&self, key: &str) -> Option<Arc<dyn Driver>> {
        let old = Self {
            held: ResourceCache::default(),
            started: self.started.clone(),
            release: self.release.clone(),
        };
        if let Some(value) = self.held.remove(key) {
            old.held.insert_ready(key.into(), value);
        }
        Some(Arc::new(old))
    }
    async fn cancel(&self, _: &ResolvedConfig, _: &QueryHandle) -> Result<()> {
        self.started.add_permits(1);
        let permit = self.release.acquire().await.unwrap();
        permit.forget();
        Ok(())
    }
    async fn close(&self, key: &str) {
        self.held.remove(key);
    }
}
#[tokio::test]
async fn lifecycle_cancelled_close_caller_keeps_owned_cleanup_alive() {
    let (mut service, conn, user) = fixture().await;
    let driver = Arc::new(CancelFixture {
        held: ResourceCache::default(),
        started: Arc::new(tokio::sync::Semaphore::new(0)),
        release: Arc::new(tokio::sync::Semaphore::new(0)),
    });
    service.registry.set_for_test(Engine::Mysql, driver.clone());
    let resolved = service
        .resolve(&conn, &user, None, "db_query")
        .await
        .unwrap();
    let key = resolved.config.cache_key();
    let dropped = Arc::new(AtomicUsize::new(0));
    driver
        .held
        .insert_ready(key.clone(), Arc::new(DropCount(dropped.clone())));
    let token = CancelToken::new();
    token.set(QueryHandle::MysqlConnId(42));
    service.in_flight.lock().unwrap().insert(
        "fixture".into(),
        InFlightQuery {
            conn_id: conn.clone(),
            user_id: user.clone(),
            resolved: Some(resolved),
            token,
            abort: None,
        },
    );
    let closing = service.clone();
    let close_conn = conn.clone();
    let caller = tokio::spawn(async move { closing.close_connection(&close_conn).await });
    driver.started.acquire().await.unwrap().forget();
    caller.abort();
    let _ = caller.await;
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        0,
        "native cancellation still owns transport"
    );
    driver.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), service.close_connection(&conn))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    let fresh = service
        .resolve(&conn, &user, None, "db_query")
        .await
        .unwrap();
    driver.held.insert_ready(
        fresh.config.cache_key(),
        Arc::new(DropCount(dropped.clone())),
    );
    tokio::task::yield_now().await;
    assert!(driver.held.get_ready(&fresh.config.cache_key()).is_some());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_native_cancel_deadline_releases_old_ownership() {
    let (mut service, conn, user) = fixture().await;
    service.close_phase_timeout = Duration::from_millis(10);
    let driver = Arc::new(CancelFixture {
        held: ResourceCache::default(),
        started: Arc::new(tokio::sync::Semaphore::new(0)),
        release: Arc::new(tokio::sync::Semaphore::new(0)),
    });
    service.registry.set_for_test(Engine::Mysql, driver.clone());
    let resolved = service
        .resolve(&conn, &user, None, "db_query")
        .await
        .unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));
    driver.held.insert_ready(
        resolved.config.cache_key(),
        Arc::new(DropCount(dropped.clone())),
    );
    let token = CancelToken::new();
    token.set(QueryHandle::MysqlConnId(42));
    service.in_flight.lock().unwrap().insert(
        "blocked".into(),
        InFlightQuery {
            conn_id: conn.clone(),
            user_id: user.clone(),
            resolved: Some(resolved),
            token,
            abort: None,
        },
    );
    tokio::time::timeout(Duration::from_secs(2), service.close_connection(&conn))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(service
        .resolve(&conn, &user, None, "db_query")
        .await
        .is_ok());
}

/// A Stop with no native handle (a mongosh script, a Mongo write, a Redis
/// command) must really drop the detached execution and say so — never report
/// success while the work keeps running.
#[tokio::test]
async fn cancel_without_native_handle_aborts_the_task_and_reports_it() {
    let (service, conn, user) = fixture().await;
    assert_eq!(
        service
            .cancel(&conn, &user, "unknown")
            .await
            .unwrap()
            .status,
        CancelStatus::NotRunning
    );

    // `cancel` looks the query up under the same key the run path registers
    // it with: `<user>:<id>` on an access-enforced connection, else `<id>`.
    let enforced = service.is_enforced(&conn).await.unwrap();
    let in_flight_key = |id: &str| {
        if enforced {
            format!("{user}:{id}")
        } else {
            id.to_string()
        }
    };

    let dropped = Arc::new(AtomicUsize::new(0));
    let held = DropCount(dropped.clone());
    let task = tokio::spawn(async move {
        let _held = held;
        std::future::pending::<()>().await;
    });
    service.in_flight.lock().unwrap().insert(
        in_flight_key("script"),
        InFlightQuery {
            conn_id: conn.clone(),
            user_id: user.clone(),
            resolved: None,
            token: CancelToken::new(),
            abort: Some(task.abort_handle()),
        },
    );
    let outcome = service.cancel(&conn, &user, "script").await.unwrap();
    assert_eq!(outcome.status, CancelStatus::Aborted);
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        1,
        "the execution future (and a mongosh child it owns) is dropped"
    );

    // Inline execution without a native handle: nothing can stop it from
    // here, and the caller is told so.
    service.in_flight.lock().unwrap().insert(
        in_flight_key("inline"),
        InFlightQuery {
            conn_id: conn.clone(),
            user_id: user.clone(),
            resolved: None,
            token: CancelToken::new(),
            abort: None,
        },
    );
    assert_eq!(
        service.cancel(&conn, &user, "inline").await.unwrap().status,
        CancelStatus::NotStoppable
    );
}

/// A secret store that counts reads and serves a swappable value.
struct CountingSecrets {
    reads: AtomicUsize,
    value: std::sync::Mutex<String>,
}
impl SecretStore for CountingSecrets {
    fn put(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn get(&self, _: &str) -> Result<Option<String>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(Some(self.value.lock().unwrap().clone()))
    }
    fn delete(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn completion_reuses_a_recent_secret_read_and_fresh_reads_refresh_it() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let secrets = Arc::new(CountingSecrets {
        reads: AtomicUsize::new(0),
        value: std::sync::Mutex::new("old".into()),
    });
    let service = DbViewerService::new(
        ConnectionsRepo::new(pool.clone()),
        secrets.clone(),
        DbExplorerRepo::new(pool),
    );
    let cached = SecretRead::CompletionCached;

    // Completion: one Keychain read serves the next requests.
    assert_eq!(
        service.read_secret("k", cached).await.unwrap().as_deref(),
        Some("old")
    );
    assert_eq!(
        service.read_secret("k", cached).await.unwrap().as_deref(),
        Some("old")
    );
    assert_eq!(secrets.reads.load(Ordering::SeqCst), 1);

    // A credential edit, then an ordinary operation: it reads fresh AND
    // refreshes completion's copy, so completion never lags behind it.
    *secrets.value.lock().unwrap() = "new".into();
    assert_eq!(
        service
            .read_secret("k", SecretRead::Fresh)
            .await
            .unwrap()
            .as_deref(),
        Some("new")
    );
    assert_eq!(
        service.read_secret("k", cached).await.unwrap().as_deref(),
        Some("new")
    );
    assert_eq!(secrets.reads.load(Ordering::SeqCst), 2);

    // Ordinary operations never populate the completion cache on their own.
    service
        .read_secret("other", SecretRead::Fresh)
        .await
        .unwrap();
    service
        .read_secret("other", SecretRead::Fresh)
        .await
        .unwrap();
    assert_eq!(secrets.reads.load(Ordering::SeqCst), 4);
    assert!(!service
        .completion_secrets
        .lock()
        .unwrap()
        .contains_key("other"));
}

/// A driver whose completion blocks until released, counting finished runs.
struct SlowCompletion {
    started: Arc<tokio::sync::Semaphore>,
    release: Arc<tokio::sync::Semaphore>,
    finished: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl Driver for SlowCompletion {
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
        _: &crate::types::NodePath,
        _: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        unreachable!()
    }
    async fn object_detail(
        &self,
        _: &ResolvedConfig,
        _: &crate::types::NodePath,
    ) -> Result<ObjectDetail> {
        unreachable!()
    }
    async fn run(&self, _: &ResolvedConfig, _: &QueryRequest) -> Result<QueryResult> {
        unreachable!()
    }
    async fn completion(
        &self,
        _: &ResolvedConfig,
        _: &crate::types::CompletionContext,
    ) -> Result<CompletionResponse> {
        // Stands in for the first schema-snapshot build of a slow remote DB.
        self.started.add_permits(1);
        self.release.acquire().await.unwrap().forget();
        self.finished.fetch_add(1, Ordering::SeqCst);
        Ok(CompletionResponse { items: Vec::new() })
    }
}

/// BUG-1: the editor aborts a superseded completion request on every keystroke,
/// which drops the HTTP handler future. The work behind it (the first schema
/// build) must still run to completion so the next request finds it cached.
#[tokio::test]
async fn completion_work_survives_the_request_being_dropped() {
    let (mut service, conn, user) = fixture().await;
    let driver = Arc::new(SlowCompletion {
        started: Arc::new(tokio::sync::Semaphore::new(0)),
        release: Arc::new(tokio::sync::Semaphore::new(0)),
        finished: Arc::new(AtomicUsize::new(0)),
    });
    service.registry.set_for_test(Engine::Mysql, driver.clone());
    // New connections start Enforced (which answers completion from the schema
    // graph, not the driver); a Legacy connection takes the driver path.
    let repo = otto_state::resource_access::ResourceAccessRepo::new(service.connections.pool());
    let current = repo
        .get_policy(otto_core::access::ResourceKind::Connection, &conn)
        .await
        .unwrap();
    repo.put_policy(
        &otto_core::access::AccessPolicy {
            revision: current.revision,
            ..otto_core::access::AccessPolicy::legacy(
                otto_core::access::ResourceKind::Connection,
                conn.clone(),
            )
        },
        current.revision,
        &otto_core::access::AccessActor {
            real_user_id: user.clone(),
            effective_user_id: None,
        },
    )
    .await
    .unwrap();
    let svc = service.clone();
    let (c, u) = (conn.clone(), user.clone());
    let mut request = tokio::spawn(async move {
        svc.completion(&c, &u, &crate::types::CompletionContext::default())
            .await
    });
    tokio::select! {
        permit = driver.started.acquire() => permit.unwrap().forget(),
        early = &mut request => panic!("completion never reached the driver: {early:?}"),
        _ = tokio::time::sleep(Duration::from_secs(5)) => panic!("completion never started"),
    }
    // The client gave up (next keystroke): the request future is dropped.
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    driver.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), async {
        while driver.finished.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the detached completion work finished after its request was dropped");
}

#[tokio::test]
async fn forget_secret_evicts_completions_cached_copy() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let secrets = Arc::new(CountingSecrets {
        reads: AtomicUsize::new(0),
        value: std::sync::Mutex::new("old".into()),
    });
    let service = DbViewerService::new(
        ConnectionsRepo::new(pool.clone()),
        secrets.clone(),
        DbExplorerRepo::new(pool),
    );
    let cached = SecretRead::CompletionCached;
    assert_eq!(
        service.read_secret("k", cached).await.unwrap().as_deref(),
        Some("old")
    );
    // The credential is edited: the connections route evicts completion's copy.
    *secrets.value.lock().unwrap() = "new".into();
    service.forget_secret("k");
    assert_eq!(
        service.read_secret("k", cached).await.unwrap().as_deref(),
        Some("new")
    );
    assert_eq!(secrets.reads.load(Ordering::SeqCst), 2);
}
