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
