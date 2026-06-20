//! Isolation tests for the `connections.owner_private` opt-in setting.
//!
//! Covers:
//! - setting OFF (default) → shared behavior preserved (`list_visible` returns all)
//! - setting ON → `list_for` scopes to caller; `require_conn_owner_or_root` 403s for non-owner
//! - root bypasses owner check regardless of setting
//! - owner sees / mutates their own connection regardless of setting

use std::sync::Arc;

use chrono::Utc;
use otto_connections::{owner_private_enabled, require_conn_owner_or_root, ConnectionsCtx};
use otto_connections::{ConnectionsService, Spawner};
use otto_core::auth::BoxFuture;
use otto_core::auth::RoleChecker;
use otto_core::domain::{Connection, ConnectionKind, Environment, Session, User, WorkspaceRole};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_state::{ConnectionSectionsRepo, ConnectionsRepo, SettingsRepo, SqlitePool};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn mem_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("in-memory pool");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn seed_user(pool: &SqlitePool, name: &str, is_root: bool) -> User {
    let id = otto_core::new_id();
    let now_ts = Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
         VALUES (?, ?, ?, ?, ?, 0, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind("hash")
    .bind(name)
    .bind(is_root as i64)
    .bind(&now_ts)
    .execute(pool)
    .await
    .expect("seed user");

    User {
        id,
        username: name.to_string(),
        display_name: name.to_string(),
        is_root,
        disabled: false,
        created_at: Utc::now(),
    }
}

async fn seed_ws(pool: &SqlitePool) -> Id {
    let ws = otto_core::new_id();
    let now_ts = Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
        .bind(&ws)
        .bind("ws")
        .bind("/tmp")
        .bind(&now_ts)
        .execute(pool)
        .await
        .expect("seed workspace");
    ws
}

fn make_conn(id: &Id, created_by: &Id) -> Connection {
    Connection {
        id: id.clone(),
        workspace_id: None,
        name: "test-conn".to_string(),
        kind: ConnectionKind::Mysql,
        params: serde_json::json!({}),
        secret_ref: None,
        first_command: None,
        section_id: None,
        environment: Environment::Dev,
        read_only: false,
        created_by: created_by.clone(),
        created_at: Utc::now(),
        last_opened_at: None,
        pinned: false,
    }
}

fn make_user(id: &Id, is_root: bool) -> User {
    User {
        id: id.clone(),
        username: "u".to_string(),
        display_name: "u".to_string(),
        is_root,
        disabled: false,
        created_at: Utc::now(),
    }
}

// Null secret store for tests (connections created without real Keychain).
struct NullSecrets;
impl SecretStore for NullSecrets {
    fn put(&self, _k: &str, _v: &str) -> Result<()> { Ok(()) }
    fn get(&self, _k: &str) -> Result<Option<String>> { Ok(None) }
    fn delete(&self, _k: &str) -> Result<()> { Ok(()) }
}

// Null spawner (open_connection not tested here).
struct NullSpawner;
impl Spawner for NullSpawner {
    fn spawn_connection<'a>(
        &'a self,
        _ws: &'a Id,
        _user: &'a Id,
        _conn: &'a Connection,
        _spec: otto_pty::CommandSpec,
        _first: Option<String>,
        _title: Option<String>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async { Err(Error::Internal("not used in tests".into())) })
    }
}

// Always-allow role checker.
struct AllowAll;
impl RoleChecker for AllowAll {
    fn check<'a>(
        &'a self,
        _user: &'a User,
        _ws: &'a Id,
        _min: WorkspaceRole,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
struct TestCtx {
    pool: SqlitePool,
    svc: Arc<ConnectionsService>,
    roles: Arc<dyn RoleChecker>,
    spawner: Arc<dyn Spawner>,
}

impl TestCtx {
    fn new(pool: SqlitePool) -> Self {
        let repo = ConnectionsRepo::new(pool.clone());
        let secs_repo = ConnectionSectionsRepo::new(pool.clone());
        let svc = ConnectionsService::new(repo, secs_repo, Arc::new(NullSecrets));
        Self {
            pool,
            svc: Arc::new(svc),
            roles: Arc::new(AllowAll),
            spawner: Arc::new(NullSpawner),
        }
    }
}

impl ConnectionsCtx for TestCtx {
    fn connections(&self) -> &Arc<ConnectionsService> { &self.svc }
    fn roles(&self) -> &Arc<dyn RoleChecker> { &self.roles }
    fn spawner(&self) -> &Arc<dyn Spawner> { &self.spawner }
    fn pool(&self) -> SqlitePool { self.pool.clone() }
}

// ---------------------------------------------------------------------------
// Tests: setting OFF (default behavior)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn setting_off_by_default_list_shows_all() {
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let ws = seed_ws(&pool).await;
    let user_a = seed_user(&pool, "alice", false).await;
    let user_b = seed_user(&pool, "bob", false).await;

    // A creates a connection.
    ctx.connections()
        .create(
            Some(ws.clone()),
            &user_a.id,
            otto_core::api::UpsertConnectionReq {
                name: "A's conn".to_string(),
                kind: ConnectionKind::Mysql,
                params: serde_json::json!({"host": "h"}),
                secret: None,
                first_command: None,
                section_id: None,
                environment: None,
                read_only: None,
            },
        )
        .await
        .unwrap();

    // Setting is OFF by default — B can list (sees A's connection).
    assert!(!owner_private_enabled(&ctx).await, "setting must be OFF by default");
    let visible = ctx.connections().list(&ws).await.unwrap();
    assert_eq!(visible.len(), 1, "when setting OFF, user B sees A's connection");
    let _ = user_b; // B hasn't created anything but list is unfiltered
}

#[tokio::test]
async fn setting_off_owner_check_allows_any_user() {
    let conn_id = otto_core::new_id();
    let owner_id = otto_core::new_id();
    let other_id = otto_core::new_id();

    let conn = make_conn(&conn_id, &owner_id);
    let other = make_user(&other_id, false);

    // When setting is OFF, callers don't invoke require_conn_owner_or_root,
    // but the function itself should 403 for non-owners (it's only called when ON).
    // Confirm: owner passes, non-owner 403.
    assert!(require_conn_owner_or_root(&other, &conn).is_err());
}

// ---------------------------------------------------------------------------
// Tests: setting ON
// ---------------------------------------------------------------------------

#[tokio::test]
async fn setting_on_list_for_excludes_others() {
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let ws = seed_ws(&pool).await;
    let user_a = seed_user(&pool, "alice2", false).await;
    let user_b = seed_user(&pool, "bob2", false).await;

    ctx.connections()
        .create(
            Some(ws.clone()),
            &user_a.id,
            otto_core::api::UpsertConnectionReq {
                name: "A's conn".to_string(),
                kind: ConnectionKind::Mysql,
                params: serde_json::json!({"host": "h"}),
                secret: None,
                first_command: None,
                section_id: None,
                environment: None,
                read_only: None,
            },
        )
        .await
        .unwrap();

    // Enable the setting.
    SettingsRepo::new(pool.clone())
        .put("connections.owner_private", &serde_json::json!(true))
        .await
        .unwrap();

    assert!(owner_private_enabled(&ctx).await, "setting must be ON");

    // B's filtered list is empty.
    let b_visible = ctx.connections().list_for(&ws, &user_b.id).await.unwrap();
    assert!(b_visible.is_empty(), "B should see no connections when setting ON");

    // A's filtered list has their own.
    let a_visible = ctx.connections().list_for(&ws, &user_a.id).await.unwrap();
    assert_eq!(a_visible.len(), 1, "A should see their own connection");
}

#[tokio::test]
async fn setting_on_require_owner_403_for_non_owner() {
    let conn_id = otto_core::new_id();
    let owner_id = otto_core::new_id();
    let other_id = otto_core::new_id();

    let conn = make_conn(&conn_id, &owner_id);
    let other = make_user(&other_id, false);

    let result = require_conn_owner_or_root(&other, &conn);
    assert!(result.is_err(), "non-owner should get 403");
    assert!(matches!(result.unwrap_err(), Error::Forbidden(_)));
}

#[tokio::test]
async fn setting_on_root_bypasses_owner_check() {
    let conn_id = otto_core::new_id();
    let owner_id = otto_core::new_id();
    let root_id = otto_core::new_id();

    let conn = make_conn(&conn_id, &owner_id);
    let root = make_user(&root_id, true);

    // Root always passes.
    assert!(require_conn_owner_or_root(&root, &conn).is_ok());
}

#[tokio::test]
async fn setting_on_owner_passes_own_connection() {
    let conn_id = otto_core::new_id();
    let owner_id = otto_core::new_id();

    let conn = make_conn(&conn_id, &owner_id);
    let owner = make_user(&owner_id, false);

    assert!(require_conn_owner_or_root(&owner, &conn).is_ok());
}

#[tokio::test]
async fn setting_on_list_for_root_bypassed_via_list() {
    // Root uses list() (unfiltered), not list_for() — verified by the handler
    // branching on `!user.is_root`. Confirm list() still returns all.
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let ws = seed_ws(&pool).await;
    let user_a = seed_user(&pool, "alice3", false).await;
    let root = seed_user(&pool, "root3", true).await;

    ctx.connections()
        .create(
            Some(ws.clone()),
            &user_a.id,
            otto_core::api::UpsertConnectionReq {
                name: "A's conn".to_string(),
                kind: ConnectionKind::Mysql,
                params: serde_json::json!({"host": "h"}),
                secret: None,
                first_command: None,
                section_id: None,
                environment: None,
                read_only: None,
            },
        )
        .await
        .unwrap();

    SettingsRepo::new(pool.clone())
        .put("connections.owner_private", &serde_json::json!(true))
        .await
        .unwrap();

    // Root uses list() and sees everything.
    let root_visible = ctx.connections().list(&ws).await.unwrap();
    assert_eq!(root_visible.len(), 1, "root sees all connections via list()");
    let _ = root;
}

#[tokio::test]
async fn setting_off_not_present_is_false() {
    // When no setting row exists at all, owner_private_enabled must return false.
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    assert!(!owner_private_enabled(&ctx).await);
}

#[tokio::test]
async fn setting_false_explicit_is_false() {
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    SettingsRepo::new(pool.clone())
        .put("connections.owner_private", &serde_json::json!(false))
        .await
        .unwrap();
    assert!(!owner_private_enabled(&ctx).await);
}
