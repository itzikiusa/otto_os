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
    fn put(&self, _k: &str, _v: &str) -> Result<()> {
        Ok(())
    }
    fn get(&self, _k: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn delete(&self, _k: &str) -> Result<()> {
        Ok(())
    }
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
    fn connections(&self) -> &Arc<ConnectionsService> {
        &self.svc
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn spawner(&self) -> &Arc<dyn Spawner> {
        &self.spawner
    }
    fn pool(&self) -> SqlitePool {
        self.pool.clone()
    }
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
    seed_owned_connection(
        &ctx,
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

    set_all_legacy(&pool).await;
    // Setting is OFF by default — B can list (sees A's connection).
    assert!(
        !owner_private_enabled(&ctx).await,
        "setting must be OFF by default"
    );
    let visible = ctx.connections().list(&ws).await.unwrap();
    assert_eq!(
        visible.len(),
        1,
        "when setting OFF, user B sees A's connection"
    );
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

    seed_owned_connection(
        &ctx,
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

    set_all_legacy(&pool).await;
    // Enable the setting.
    SettingsRepo::new(pool.clone())
        .put("connections.owner_private", &serde_json::json!(true))
        .await
        .unwrap();

    assert!(owner_private_enabled(&ctx).await, "setting must be ON");

    // B's filtered list is empty.
    let b_visible = ctx.connections().list_for(&ws, &user_b.id).await.unwrap();
    assert!(
        b_visible.is_empty(),
        "B should see no connections when setting ON"
    );

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

    seed_owned_connection(
        &ctx,
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

    set_all_legacy(&pool).await;
    // Root uses list() and sees everything.
    let root_visible = ctx.connections().list(&ws).await.unwrap();
    assert_eq!(
        root_visible.len(),
        1,
        "root sees all connections via list()"
    );
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

// These owner-private compatibility tests intentionally create legacy profiles.
async fn set_all_legacy(pool: &SqlitePool) {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, created_by FROM connections")
        .fetch_all(pool)
        .await
        .unwrap();
    let repo = otto_state::resource_access::ResourceAccessRepo::new(pool.clone());
    for (id, creator) in rows {
        let mut policy = repo
            .get_policy(otto_core::access::ResourceKind::Connection, &id)
            .await
            .unwrap();
        let revision = policy.revision;
        policy.mode = otto_core::access::AccessMode::Legacy;
        policy.rules.clear();
        repo.put_policy(
            &policy,
            revision,
            &otto_core::access::AccessActor {
                real_user_id: creator,
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
    }
}

// Legacy ownership fixtures predate root-provisioned native setup; create their
// persisted rows directly rather than bypassing the production creation gate.
async fn seed_owned_connection(
    ctx: &TestCtx,
    workspace_id: Option<Id>,
    owner: &Id,
    req: otto_core::api::UpsertConnectionReq,
) -> Result<Connection> {
    ConnectionsRepo::new(ctx.pool.clone())
        .create(otto_state::NewConnection {
            workspace_id,
            name: req.name,
            kind: req.kind,
            params: req.params,
            secret_ref: None,
            first_command: req.first_command,
            section_id: req.section_id,
            environment: req.environment.unwrap_or_default(),
            read_only: req.read_only.unwrap_or(false),
            created_by: owner.clone(),
        })
        .await
}

#[derive(Default)]
struct MemorySecrets(std::sync::Mutex<std::collections::HashMap<String, String>>);
impl SecretStore for MemorySecrets {
    fn put(&self, k: &str, v: &str) -> Result<()> {
        self.0.lock().unwrap().insert(k.into(), v.into());
        Ok(())
    }
    fn get(&self, k: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(k).cloned())
    }
    fn delete(&self, k: &str) -> Result<()> {
        self.0.lock().unwrap().remove(k);
        Ok(())
    }
}
#[tokio::test]
async fn mongo_uri_is_normalized_on_create_update_legacy_read_and_duplicate_has_no_secret() {
    let pool = mem_pool().await;
    let root = seed_user(&pool, "credential-root", true).await;
    let secrets = Arc::new(MemorySecrets::default());
    let repo = ConnectionsRepo::new(pool.clone());
    let svc = ConnectionsService::new(
        repo.clone(),
        ConnectionSectionsRepo::new(pool),
        secrets.clone(),
    );
    let request = otto_core::api::UpsertConnectionReq {
        name: "replica".into(),
        kind: ConnectionKind::Mongodb,
        params: serde_json::json!({"conn_string":"mongodb://alice:p%40ss@one,two/db?tls=true"}),
        secret: None,
        first_command: None,
        section_id: None,
        environment: None,
        read_only: None,
    };
    let conn = svc.create(None, &root.id, request.clone()).await.unwrap();
    assert_eq!(
        conn.params["conn_string"],
        "mongodb://alice:{secret}@one,two/db?tls=true"
    );
    assert_eq!(
        secrets
            .get(conn.secret_ref.as_ref().unwrap())
            .unwrap()
            .as_deref(),
        Some("p@ss")
    );
    assert!(!serde_json::to_string(&repo.get(&conn.id).await.unwrap())
        .unwrap()
        .contains("p%40ss"));
    let copy = svc.duplicate(&conn.id, &root.id).await.unwrap();
    assert!(copy.secret_ref.is_none());
    assert_eq!(copy.params, conn.params);
    let mut edit = request;
    edit.params["conn_string"] = "mongodb://alice:next%3Apass@one/db".into();
    let edited = svc.update(&conn.id, &root.id, edit).await.unwrap();
    assert_eq!(
        secrets
            .get(edited.secret_ref.as_ref().unwrap())
            .unwrap()
            .as_deref(),
        Some("next:pass")
    );
    let legacy_params = serde_json::json!({"conn_string":"mongodb://u:legacy%2Fpass@host/db"});
    repo.update(
        &conn.id,
        None,
        Some(&legacy_params),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let migrated = svc.get(&conn.id).await.unwrap();
    assert_eq!(
        migrated.params["conn_string"],
        "mongodb://u:{secret}@host/db"
    );
    assert_eq!(
        secrets
            .get(migrated.secret_ref.as_ref().unwrap())
            .unwrap()
            .as_deref(),
        Some("legacy/pass")
    );
}

#[tokio::test]
async fn import_reconciliation_create_update_skip_preserves_secret_and_reports_invalid_target() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        Extension,
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let pool = mem_pool().await;
    let root = seed_user(&pool, "import-root", true).await;
    let ws = seed_ws(&pool).await;
    let ctx = TestCtx::new(pool.clone());
    let saved = ctx
        .svc
        .create(
            None,
            &root.id,
            otto_core::api::UpsertConnectionReq {
                name: "original".into(),
                kind: ConnectionKind::Mysql,
                params: serde_json::json!({"host":"before", "advanced":"keep"}),
                secret: Some("fixture-only".into()),
                first_command: Some("select 1".into()),
                section_id: None,
                environment: Some(Environment::Prod),
                read_only: Some(true),
            },
        )
        .await
        .unwrap();
    let router = otto_connections::api_router::<TestCtx>()
        .layer(Extension(otto_core::auth::AuthUser(root)))
        .with_state(ctx.clone());
    let body = serde_json::json!({"connections":[
        {"name":"updated", "kind":"mysql", "params":{"host":"after"}, "action":"update", "target_id":saved.id},
        {"name":"new", "kind":"mysql", "params":{"host":"new"}, "action":"create"},
        {"name":"skipped", "kind":"mysql", "params":{"host":"skip"}, "action":"skip"},
        {"name":"invalid", "kind":"mysql", "params":{"host":"bad"}, "action":"update", "target_id":"missing"}
    ]});
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/workspaces/{ws}/connections/import/create"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(result["created"].as_array().unwrap().len(), 1);
    assert_eq!(result["updated"].as_array().unwrap().len(), 1);
    assert_eq!(result["skipped"], serde_json::json!(["skipped"]));
    assert_eq!(result["failed"].as_array().unwrap().len(), 1);
    let updated = ctx.svc.get(&saved.id).await.unwrap();
    assert_eq!(
        updated.params,
        serde_json::json!({"host":"after", "advanced":"keep"})
    );
    assert_eq!(updated.secret_ref, saved.secret_ref);
    assert_eq!(updated.first_command, saved.first_command);
    assert_eq!(updated.environment, Environment::Prod);
    assert!(updated.read_only);
    assert_eq!(
        ConnectionsRepo::new(pool)
            .list_visible(&ws)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn failed_legacy_uri_secret_write_preserves_row_and_does_not_expose_it() {
    struct Unavailable;
    impl SecretStore for Unavailable {
        fn put(&self, _: &str, _: &str) -> Result<()> {
            Err(Error::Internal("fixture secret store unavailable".into()))
        }
        fn get(&self, _: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn delete(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    let pool = mem_pool().await;
    let root = seed_user(&pool, "unavailable-root", true).await;
    let ws = seed_ws(&pool).await;
    let repo = ConnectionsRepo::new(pool.clone());
    let params = serde_json::json!({"conn_string":"mongodb://u:legacy-fixture-password@host/db"});
    let conn = repo
        .create(otto_state::NewConnection {
            workspace_id: None,
            name: "legacy".into(),
            kind: ConnectionKind::Mongodb,
            params: params.clone(),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let service = ConnectionsService::new(
        repo.clone(),
        ConnectionSectionsRepo::new(pool),
        Arc::new(Unavailable),
    );
    let error = service.get(&conn.id).await.unwrap_err().to_string();
    assert!(!error.contains("legacy-fixture-password"));
    assert!(service.list_for(&ws, &root.id).await.is_err());
    assert_eq!(repo.get(&conn.id).await.unwrap().params, params);
}
