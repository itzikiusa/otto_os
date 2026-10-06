//! Test fixtures for building a full [`ServerCtx`] without a daemon.
//!
//! Before this module every integration suite copied a ~70-field `ServerCtx`
//! struct literal, so adding a field to the context meant editing a dozen test
//! files. [`ServerCtx::for_tests`] is now the ONE place that wires a
//! self-contained context (the shared literal itself is
//! [`ServerCtx::from_parts`], which the daemon's `boot::build_ctx` uses too) (in-memory secrets, no-op connection spawner, usage
//! engine disabled, no telemetry, no channel bridge). Suites that need a
//! different value override the public field afterwards:
//!
//! ```ignore
//! let mut ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
//! ctx.base_url = format!("http://{addr}");
//! ```
//!
//! Compiled for this crate's unit tests and, through the `test-util` feature
//! (enabled by the crate's self dev-dependency), for `tests/*.rs`. Never part
//! of a release build.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_state::DbPool;
use tokio::sync::broadcast;

use crate::boot::CtxParts;
use crate::state::ServerCtx;

/// Process-local secret store (the Keychain is never touched by tests).
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

/// Connection spawner that refuses every spawn (tests never open SSH/DB PTYs).
pub struct NoopSpawner;

impl otto_connections::Spawner for NoopSpawner {
    fn spawn_connection<'a>(
        &'a self,
        _ws_id: &'a Id,
        _user_id: &'a Id,
        _conn: &'a otto_core::domain::Connection,
        _spec: otto_pty::CommandSpec,
        _first_command: Option<String>,
        _title: Option<String>,
    ) -> otto_core::auth::BoxFuture<'a, Result<otto_core::domain::Session>> {
        Box::pin(async { Err(Error::Internal("noop spawner".into())) })
    }
}

impl ServerCtx {
    /// A complete, isolated context over `pool` with every on-disk root under
    /// `data_dir`. Uses [`MemorySecrets`]; see [`Self::for_tests_with_secrets`].
    pub async fn for_tests(pool: &DbPool, data_dir: impl Into<PathBuf>) -> ServerCtx {
        Self::for_tests_with_secrets(pool, data_dir, Arc::new(MemorySecrets::default())).await
    }

    /// Switch this fixture's authenticator to the daemon's CACHED one (over
    /// [`ServerCtx::auth_cache`]), for tests that must prove a revoke reaches
    /// the shared auth cache (S8-302) — fixtures are uncached by default.
    pub fn with_cached_auth(mut self, pool: &DbPool) -> ServerCtx {
        self.authenticator = Arc::new(otto_rbac::RbacAuthenticator::new_with_cache(
            pool.clone(),
            self.auth_cache.clone(),
        ));
        self
    }

    /// [`Self::for_tests`] with a caller-supplied secret store.
    pub async fn for_tests_with_secrets(
        pool: &DbPool,
        data_dir: impl Into<PathBuf>,
        secrets: Arc<dyn SecretStore>,
    ) -> ServerCtx {
        let data_dir: PathBuf = data_dir.into();
        let (events, _rx) = broadcast::channel(256);
        let manager = Arc::new(otto_sessions::SessionManager::new(
            otto_state::SessionsRepo::new(pool.clone()),
            events.clone(),
            otto_sessions::ProviderRegistry::new(None),
        ));
        let orchestrator = Arc::new(otto_orchestrator::Orchestrator::new("claude"));
        let improve_engine = Arc::new(otto_improve::ImprovementEngine {
            improvements: otto_state::ImprovementsRepo::new(pool.clone()),
            sessions: otto_state::SessionsRepo::new(pool.clone()),
            workspaces: otto_state::WorkspacesRepo::new(pool.clone()),
            producer: Arc::new(otto_improve::RealProposalProducer::new(
                orchestrator.clone(),
            )),
            events: events.clone(),
            library_root: data_dir.join("lib"),
        });
        let brokers = Arc::new(otto_brokers::BrokersService::new(
            otto_state::BrokerClustersRepo::new(pool.clone()),
            secrets.clone(),
            None,
        ));
        let usage = otto_usage::UsageEngine::start(
            otto_usage::UsageConfig {
                enabled: false, // Fixtures never start ClickHouse.
                ..Default::default()
            },
            data_dir.join("usage"),
        )
        .await;

        ServerCtx::from_parts(CtxParts {
            pool: pool.clone(),
            secrets,
            events,
            version: "test".into(),
            base_url: "http://127.0.0.1:0".into(),
            data_dir: data_dir.clone(),
            plugins: Arc::new(crate::plugins::PluginManager::new(
                otto_state::PluginsRepo::new(pool.clone()),
                data_dir.join("plugins"),
                data_dir.clone(),
                "http://127.0.0.1:7700/api/v1/plugin-host".into(),
            )),
            manager,
            spawner: Arc::new(NoopSpawner),
            brokers,
            channel_bridge: None,
            orchestrator,
            improve_engine,
            context_library: otto_context::Library::new(data_dir.join("ctx")),
            usage,
            telemetry: None,
            memory: Arc::new(otto_memory::MemoryService::with_defaults(pool.clone())),
            proof_media_dir: None,
            browser: Arc::new(crate::routes::browser::BrowserEngineHandle::new(
                None,
                data_dir.join("browser"),
            )),
            cache_auth_lookups: false,
        })
    }
}

/// A fresh in-memory SQLite pool with every migration applied (one connection,
/// so the in-memory DB is shared by every query).
pub async fn mem_pool() -> DbPool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}
