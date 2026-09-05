use super::*;
use otto_core::access::{AccessMode, ResourceKind};

fn user() -> User {
    User {
        id: "admin".into(),
        username: "admin".into(),
        display_name: "Admin".into(),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    }
}
fn rule(id: &str, subject: &str, operations: &[&str]) -> AccessRule {
    AccessRule {
        id: id.into(),
        subject_kind: SubjectKind::User,
        subject_id: subject.into(),
        effect: RuleEffect::Allow,
        operations: operations.iter().map(|s| (*s).into()).collect(),
        children: None,
        grantable_operations: vec![],
        credential_connection_id: None,
    }
}
fn policy() -> AccessPolicy {
    let mut authority = rule("delegation", "admin", &["discover", "manage_access"]);
    authority.grantable_operations = vec!["discover".into(), "db_query".into()];
    AccessPolicy {
        kind: ResourceKind::Connection,
        resource_id: "connection".into(),
        mode: AccessMode::Enforced,
        revision: 1,
        rules: vec![authority],
    }
}
#[test]
fn delegated_admin_cannot_grant_schema_changes_beyond_ceiling() {
    let old = policy();
    let mut proposed = old.clone();
    proposed.rules.push(rule("schema", "bob", &["db_schema"]));
    assert!(validate_delegation(&user(), &[], &old, &proposed).is_err());
}
#[test]
fn delegated_admin_cannot_grant_self_through_group_membership() {
    let old = policy();
    let mut proposed = old.clone();
    let mut grant = rule("query", "admins", &["db_query"]);
    grant.subject_kind = SubjectKind::Group;
    proposed.rules.push(grant);
    assert!(validate_delegation(&user(), &["admins".into()], &old, &proposed).is_err());
}
#[test]
fn delegated_admin_can_grant_query_to_other_user() {
    let old = policy();
    let mut proposed = old.clone();
    proposed.rules.push(rule("query", "bob", &["db_query"]));
    assert!(validate_delegation(&user(), &[], &old, &proposed).is_ok());
}
#[test]
fn delegated_admin_cannot_remove_an_existing_restriction() {
    let mut old = policy();
    let mut deny = rule("restriction", "bob", &["db_query"]);
    deny.effect = RuleEffect::Deny;
    old.rules.push(deny);
    let mut proposed = old.clone();
    proposed.rules.pop();
    assert!(validate_delegation(&user(), &[], &old, &proposed).is_err());
}
#[test]
fn child_scoped_delegation_cannot_grant_all_databases() {
    let mut old = policy();
    old.rules[0].children = Some(vec!["development".into()]);
    let mut proposed = old.clone();
    proposed.rules.push(rule("query", "bob", &["db_query"]));
    assert!(validate_delegation(&user(), &[], &old, &proposed).is_err());
}
#[test]
fn delegated_admin_cannot_disable_enforcement() {
    let old = policy();
    let mut proposed = old.clone();
    proposed.mode = AccessMode::Legacy;
    assert!(validate_delegation(&user(), &[], &old, &proposed).is_err());
}

#[derive(Clone)]
struct TestAccess(sqlx::SqlitePool);
impl AccessCtx for TestAccess {
    fn access_pool(&self) -> SqlitePool {
        self.0.clone()
    }
}
async fn setup() -> (TestAccess, User, User, Id) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let users = UsersRepo::new(pool.clone());
    let root = users.create("root", "hash", "Root", true).await.unwrap();
    let viewer = users
        .create("reader", "hash", "Reader", false)
        .await
        .unwrap();
    for feature in ["connections", "database"] {
        sqlx::query(
            "INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?,?,'view')",
        )
        .bind(&viewer.id)
        .bind(feature)
        .execute(&pool)
        .await
        .unwrap();
    }
    let id = otto_core::new_id();
    sqlx::query("INSERT INTO connections (id,name,kind,params_json,created_by,created_at) VALUES (?,'Test','mysql','{}',?,'2026-09-05T00:00:00Z')").bind(&id).bind(&root.id).execute(&pool).await.unwrap();
    (TestAccess(pool), root, viewer, id)
}
fn auth(user: &User) -> otto_core::auth::AuthContext {
    otto_core::auth::AuthContext {
        real_user: user.clone(),
        effective_user: user.clone(),
        scope: None,
        mcp_only: false,
        mcp_scope: None,
        mcp_internal: false,
        mcp_session_id: None,
    }
}
async fn request<S: AccessCtx>(
    ctx: &S,
    user: &User,
    method: &str,
    path: &str,
    body: Value,
) -> (u16, Value) {
    use tower::ServiceExt;
    let app = api_router::<S>()
        .layer(axum::Extension(auth(user)))
        .with_state(ctx.clone());
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
#[tokio::test]
async fn resource_management_is_private_and_effective_access_is_scoped() {
    let (ctx, root, viewer, id) = setup().await;
    let repo = ResourceAccessRepo::new(ctx.0.clone());
    let mut p = repo
        .get_policy(ResourceKind::Connection, &id)
        .await
        .unwrap();
    let mut grant = rule("reader", &viewer.id, &["discover", "db_query"]);
    grant.children = Some(vec!["finance".into()]);
    p.rules.push(grant);
    repo.put_policy(&p, p.revision, &actor(&auth(&root)))
        .await
        .unwrap();
    let base = format!("/access/connection/{id}");
    assert_eq!(
        request(&ctx, &viewer, "GET", &base, Value::Null).await.0,
        403
    );
    let (status, body) = request(
        &ctx,
        &viewer,
        "GET",
        &format!("{base}/capabilities?child=finance"),
        Value::Null,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["operations"]["db_query"]["allowed"], true);
    assert_eq!(
        body["operations"]["db_query"]["matched_rule_ids"],
        json!([])
    );
    let (_, body) = request(
        &ctx,
        &viewer,
        "GET",
        &format!("{base}/capabilities?child=payroll"),
        Value::Null,
    )
    .await;
    assert_eq!(body["operations"]["db_query"]["allowed"], false);
    assert_eq!(
        request(
            &ctx,
            &viewer,
            "POST",
            "/access/groups",
            json!({"name":"escalate"})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        request(
            &ctx,
            &root,
            "GET",
            "/access/connection/missing",
            Value::Null
        )
        .await
        .0,
        404
    );
}
#[tokio::test]
async fn activation_requires_current_preview_and_stale_saves_conflict() {
    let (ctx, root, _, id) = setup().await;
    let repo = ResourceAccessRepo::new(ctx.0.clone());
    let mut p = repo
        .get_policy(ResourceKind::Connection, &id)
        .await
        .unwrap();
    p.mode = AccessMode::Legacy;
    let base = format!("/access/connection/{id}");
    assert_eq!(
        request(&ctx, &root, "PUT", &base, json!({"policy":p}))
            .await
            .0,
        409
    );
    let (status, preview) = request(
        &ctx,
        &root,
        "POST",
        &format!("{base}/preview"),
        json!({"policy":p}),
    )
    .await;
    assert_eq!(status, 200);
    let body = json!({"policy":p,"preview_token":preview["token"]});
    assert_eq!(
        request(&ctx, &root, "PUT", &base, body.clone()).await.0,
        200
    );
    assert_eq!(request(&ctx, &root, "PUT", &base, body).await.0, 409);
}

#[derive(Clone)]
struct FailedRetirement(SqlitePool);
impl AccessCtx for FailedRetirement {
    fn access_pool(&self) -> SqlitePool {
        self.0.clone()
    }
    fn retire_mcp<'a>(
        &'a self,
        _id: &'a Id,
        apply: bool,
    ) -> otto_core::auth::BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if apply {
                Err(Error::Conflict("launcher is not writable".into()))
            } else {
                Ok(())
            }
        })
    }
}
#[tokio::test]
async fn failed_direct_launcher_retirement_does_not_activate_policy() {
    let (base, root, _, _) = setup().await;
    let ws = otto_state::WorkspacesRepo::new(base.0.clone())
        .create("MCP", "/tmp", &root.id)
        .await
        .unwrap();
    let server = otto_state::McpServersRepo::new(base.0.clone())
        .create(otto_state::NewMcpServer {
            workspace_id: ws.id,
            name: "retirement".into(),
            command: "mock".into(),
            args: vec![],
            env: Default::default(),
            enabled: true,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let repo = ResourceAccessRepo::new(base.0.clone());
    let mut policy = repo
        .get_policy(ResourceKind::McpServer, &server.id)
        .await
        .unwrap();
    policy.mode = AccessMode::Legacy;
    let old = repo
        .put_policy(&policy, policy.revision, &actor(&auth(&root)))
        .await
        .unwrap();
    let mut proposed = old.clone();
    proposed.mode = AccessMode::Enforced;
    let ctx = FailedRetirement(base.0);
    let path = format!("/access/mcp_server/{}", server.id);
    let (status, preview) = request(
        &ctx,
        &root,
        "POST",
        &format!("{path}/preview"),
        serde_json::json!({"policy":proposed}),
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) = request(
        &ctx,
        &root,
        "PUT",
        &path,
        serde_json::json!({"policy":proposed,"preview_token":preview["token"]}),
    )
    .await;
    assert_eq!(status, 409);
    let actual = repo
        .get_policy(ResourceKind::McpServer, &server.id)
        .await
        .unwrap();
    assert_eq!(actual, old, "failed cleanup must not publish enforcement");
}
