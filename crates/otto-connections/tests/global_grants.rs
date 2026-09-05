//! Router-level tests for the **global-connection** authorization branch.
//!
//! Every connection the UI creates is global (`workspace_id IS NULL` — a
//! deliberate "global library"), so this branch is the one that decides whether
//! a non-root account can use connections at all. It used to be root-only,
//! which locked every shared user out of the whole library. It now answers on
//! the feature axis:
//!
//! - **use** (test / pin / SFTP …) → `Connections:Edit`
//! - **manage the record** (PATCH / DELETE) → `Connections:Admin`, so an
//!   Edit-level teammate can't rewrite or delete an entry everyone shares
//! - **root** → always
//!
//! The workspace role checker here is `AllowAll`, so any pass/deny below comes
//! from the grant, never from the workspace axis.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Extension;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_connections::{ConnectionsCtx, ConnectionsService, Spawner};
use otto_core::api::UpsertConnectionReq;
use otto_core::auth::{AuthUser, BoxFuture, RoleChecker};
use otto_core::domain::{Connection, ConnectionKind, Session, User, WorkspaceRole};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_state::{ConnectionSectionsRepo, ConnectionsRepo, SqlitePool};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

async fn mem_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().in_memory(true).foreign_keys(true))
        .await
        .expect("in-memory pool");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

/// Seed a user and, unless `cap` is `"none"`, their `connections` grant.
async fn seed_user(pool: &SqlitePool, name: &str, is_root: bool, cap: &str) -> User {
    let id = otto_core::new_id();
    let now_ts = Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
         VALUES (?, ?, 'hash', ?, ?, 0, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(name)
    .bind(is_root as i64)
    .bind(&now_ts)
    .execute(pool)
    .await
    .expect("seed user");

    if cap != "none" {
        sqlx::query(
            "INSERT INTO user_feature_grants (user_id, feature, capability)
             VALUES (?, 'connections', ?)",
        )
        .bind(&id)
        .bind(cap)
        .execute(pool)
        .await
        .expect("seed grant");
    }

    User {
        id,
        username: name.to_string(),
        display_name: name.to_string(),
        is_root,
        disabled: false,
        created_at: Utc::now(),
    }
}

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

/// Always-allow workspace roles — any 403 below must come from the grant.
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
        let svc = ConnectionsService::new(
            ConnectionsRepo::new(pool.clone()),
            ConnectionSectionsRepo::new(pool.clone()),
            Arc::new(NullSecrets),
        );
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

/// Create the one **global** connection every test acts on.
async fn seed_global_conn(ctx: &TestCtx, creator: &Id) -> Connection {
    let conn = ctx.connections()
        .create(
            None, // global — exactly what `POST /workspaces/{id}/connections` does
            creator,
            UpsertConnectionReq {
                name: "shared-db".into(),
                kind: ConnectionKind::Mysql,
                params: serde_json::json!({ "host": "h", "port": 3306, "user": "u" }),
                secret: None,
                first_command: None,
                section_id: None,
                environment: None,
                read_only: None,
            },
        )
        .await
        .expect("create global connection");
    // This suite covers the explicitly retained legacy feature-grant behavior.
    let repo = otto_state::resource_access::ResourceAccessRepo::new(ctx.pool.clone());
    let mut policy = repo.get_policy(otto_core::access::ResourceKind::Connection, &conn.id).await.unwrap();
    let revision = policy.revision;
    policy.mode = otto_core::access::AccessMode::Legacy;
    policy.rules.clear();
    repo.put_policy(&policy, revision, &otto_core::access::AccessActor { real_user_id: creator.clone(), effective_user_id: None }).await.unwrap();
    conn
}

/// Drive one request through the real router as `user`.
async fn call(
    ctx: &TestCtx,
    user: &User,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) -> StatusCode {
    let app = otto_connections::api_router::<TestCtx>()
        .layer(Extension(AuthUser(user.clone())))
        .with_state(ctx.clone());
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let resp = app.oneshot(req).await.expect("router response");
    let status = resp.status();
    // Drain so a failing assertion can't be blamed on an unread body.
    let _ = resp.into_body().collect().await;
    status
}

// ---------------------------------------------------------------------------
// Use tier: Connections:Edit
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edit_grant_may_use_a_global_connection() {
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let root = seed_user(&pool, "root", true, "none").await;
    let conn = seed_global_conn(&ctx, &root.id).await;

    let editor = seed_user(&pool, "editor", false, "edit").await;
    let status = call(
        &ctx,
        &editor,
        "PATCH",
        &format!("/connections/{}/pin", conn.id),
        serde_json::json!({ "pinned": true }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Connections:Edit must be able to use a global connection"
    );
}

#[tokio::test]
async fn view_grant_may_not_use_a_global_connection() {
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let root = seed_user(&pool, "root", true, "none").await;
    let conn = seed_global_conn(&ctx, &root.id).await;

    let viewer = seed_user(&pool, "viewer", false, "view").await;
    let status = call(
        &ctx,
        &viewer,
        "PATCH",
        &format!("/connections/{}/pin", conn.id),
        serde_json::json!({ "pinned": true }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "View is below the Edit bar");
}

#[tokio::test]
async fn ungranted_user_may_not_touch_a_global_connection() {
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let root = seed_user(&pool, "root", true, "none").await;
    let conn = seed_global_conn(&ctx, &root.id).await;

    let stranger = seed_user(&pool, "stranger", false, "none").await;
    let status = call(
        &ctx,
        &stranger,
        "PATCH",
        &format!("/connections/{}/pin", conn.id),
        serde_json::json!({ "pinned": true }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "default-deny: no grant row");
}

// ---------------------------------------------------------------------------
// Manage tier: Connections:Admin
// ---------------------------------------------------------------------------

fn upsert_body() -> serde_json::Value {
    serde_json::json!({
        "name": "renamed",
        "kind": "mysql",
        "params": { "host": "h2", "port": 3306, "user": "u" },
    })
}

#[tokio::test]
async fn edit_grant_may_not_rewrite_or_delete_a_shared_connection() {
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let root = seed_user(&pool, "root", true, "none").await;
    let conn = seed_global_conn(&ctx, &root.id).await;
    let editor = seed_user(&pool, "editor", false, "edit").await;

    let patched = call(
        &ctx,
        &editor,
        "PATCH",
        &format!("/connections/{}", conn.id),
        upsert_body(),
    )
    .await;
    assert_eq!(
        patched,
        StatusCode::FORBIDDEN,
        "editing a globally-shared record takes Connections:Admin"
    );

    let deleted = call(
        &ctx,
        &editor,
        "DELETE",
        &format!("/connections/{}", conn.id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(
        deleted,
        StatusCode::FORBIDDEN,
        "deleting a globally-shared record takes Connections:Admin"
    );

    // And the record really is untouched.
    let still = ctx.connections().get(&conn.id).await.expect("still exists");
    assert_eq!(still.name, "shared-db");
}

#[tokio::test]
async fn admin_grant_manages_a_global_connection() {
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let root = seed_user(&pool, "root", true, "none").await;
    let conn = seed_global_conn(&ctx, &root.id).await;
    let admin = seed_user(&pool, "admin", false, "admin").await;

    let status = call(
        &ctx,
        &admin,
        "PATCH",
        &format!("/connections/{}", conn.id),
        upsert_body(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Connections:Admin manages globals");
    assert_eq!(
        ctx.connections().get(&conn.id).await.unwrap().name,
        "renamed"
    );
}

#[tokio::test]
async fn root_still_bypasses_every_tier() {
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let root = seed_user(&pool, "root", true, "none").await;
    let conn = seed_global_conn(&ctx, &root.id).await;

    // Root holds no grant row at all and still manages the record.
    let status = call(
        &ctx,
        &root,
        "PATCH",
        &format!("/connections/{}", conn.id),
        upsert_body(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "root bypasses the grant table");
}

#[tokio::test]
async fn governed_discovery_redacts_credentials_and_does_not_grant_configuration() {
    use otto_core::access::*;
    let pool = mem_pool().await;
    let ctx = TestCtx::new(pool.clone());
    let root = seed_user(&pool,"policy-root",true,"admin").await;
    let viewer = seed_user(&pool,"policy-viewer",false,"view").await;
    let stranger = seed_user(&pool,"policy-stranger",false,"admin").await;
    let conn = seed_global_conn(&ctx,&root.id).await;
    let repo = otto_state::resource_access::ResourceAccessRepo::new(pool);
    let current = repo.get_policy(ResourceKind::Connection,&conn.id).await.unwrap();
    let policy = AccessPolicy {kind:ResourceKind::Connection,resource_id:conn.id.clone(),mode:AccessMode::Enforced,revision:current.revision,
        rules:vec![AccessRule {id:"discover-only".into(),subject_kind:SubjectKind::User,subject_id:viewer.id.clone(),effect:RuleEffect::Allow,operations:vec!["discover".into()],children:None,grantable_operations:vec![],credential_connection_id:None}]};
    repo.put_policy(&policy,current.revision,&AccessActor {real_user_id:root.id.clone(),effective_user_id:None}).await.unwrap();
    let visible = ctx.connections().visible_connection(conn.clone(),&viewer.id).await.unwrap();
    assert_eq!(visible.params,serde_json::json!({}));
    assert!(visible.secret_ref.is_none());
    assert!(visible.first_command.is_none());
    assert_eq!(call(&ctx,&viewer,"PATCH",&format!("/connections/{}/pin",conn.id),serde_json::json!({"pinned":true})).await,StatusCode::OK);
    assert!(ctx.connections().authorize(&conn.id,&viewer.id,"configure").await.is_err());
    assert!(matches!(ctx.connections().visible_connection(conn,&stranger.id).await.unwrap_err(),Error::NotFound(_)));
}

#[tokio::test]
async fn delegated_configure_cannot_rebind_native_identity_or_disable_protection() {
    use otto_core::access::*;
    let pool=mem_pool().await;let root=seed_user(&pool,"owner",true,"admin").await;let member=seed_user(&pool,"delegated",false,"view").await;
    let ctx=TestCtx::new(pool.clone());
    let request=UpsertConnectionReq{name:"Restricted DB".into(),kind:ConnectionKind::Mysql,params:serde_json::json!({"host":"example.invalid","port":3306,"user":"limited"}),secret:None,first_command:None,section_id:None,environment:Some(otto_core::domain::Environment::Prod),read_only:Some(true)};
    let conn=ctx.svc.create(None,&root.id,request.clone()).await.unwrap();
    let repo=otto_state::ResourceAccessRepo::new(pool.clone());let mut policy=repo.get_policy(ResourceKind::Connection,&conn.id).await.unwrap();
    policy.rules.push(AccessRule{id:"configure".into(),subject_kind:SubjectKind::User,subject_id:member.id.clone(),effect:RuleEffect::Allow,operations:vec!["discover".into(),"configure".into()],children:None,grantable_operations:vec![],credential_connection_id:None});repo.put_policy(&policy,policy.revision,&AccessActor{real_user_id:root.id.clone(),effective_user_id:None}).await.unwrap();
    let mut cosmetic=request.clone();cosmetic.name="Renamed".into();assert_eq!(ctx.svc.update(&conn.id,&member.id,cosmetic).await.unwrap().name,"Renamed");
    for field in ["params","secret","environment","read_only","first_command"] {
        let mut mutation=request.clone();match field {"params"=>mutation.params["user"]=serde_json::json!("hidden_admin"),"secret"=>mutation.secret=Some("replacement".into()),"environment"=>mutation.environment=Some(otto_core::domain::Environment::Dev),"read_only"=>mutation.read_only=Some(false),_=>mutation.first_command=Some("unrestricted command".into())};
        assert!(matches!(ctx.svc.update(&conn.id,&member.id,mutation).await,Err(Error::Forbidden(_))),"sensitive mutation {field}");
    }
    let stored=ConnectionsRepo::new(pool).get(&conn.id).await.unwrap();assert_eq!(stored.params,request.params);assert!(stored.read_only);assert_eq!(stored.environment,otto_core::domain::Environment::Prod);
    assert!(matches!(ctx.svc.create(None,&member.id,request).await,Err(Error::Forbidden(_))),"new aliases must be provisioned by root");
}

#[tokio::test]
async fn nonroot_admin_cannot_scan_or_import_ambient_native_identities() {
    let pool=mem_pool().await;let admin=seed_user(&pool,"setup-admin",false,"admin").await;let ctx=TestCtx::new(pool);
    for (method,path,body) in [
        ("GET","/workspaces/test/connections/import/sources",serde_json::json!({})),
        ("POST","/workspaces/test/connections/import/scan",serde_json::json!({"source":"mysql_workbench"})),
        ("POST","/workspaces/test/connections/import/create",serde_json::json!({"connections":[],"section_id":null})),
    ] {
        assert_eq!(call(&ctx,&admin,method,path,body).await,StatusCode::FORBIDDEN,"ambient setup endpoint {path}");
    }
}
