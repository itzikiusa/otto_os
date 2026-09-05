//! End-to-end test of the MCP Control Plane governance pipeline against a REAL
//! mock stdio MCP server (a small `sh` script that speaks JSON-RPC 2.0). This
//! exercises the outbound client + discovery + risk-labeling + per-tool
//! permission + allowlist + policy + approval gate + dry-run + audit + stats —
//! i.e. control-plane requirements 2,3,4,5,7,8,9,10,11,12 in one flow, with no
//! external dependency.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use otto_core::secrets::SecretStore;
use otto_core::{new_id, Result};
use otto_mcp::{InvokeCtx, InvokeOutcome, McpService};
use otto_state::{
    McpAllowlistRepo, NewAllowlistEntry, NewPolicy, NewServerRow, SettingsRepo, SqlitePool,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// Trivial in-memory secret store (the pipeline only resolves secrets for servers
/// that declare them; this test uses none, but the service still needs a store).
#[derive(Default)]
struct MemSecrets(Mutex<HashMap<String, String>>);
impl SecretStore for MemSecrets {
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

async fn pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
    let p = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap();
    sqlx::migrate!("../otto-state/migrations").run(&p).await.unwrap();
    p
}

async fn seed_ws(pool: &SqlitePool) -> (String, String) {
    let user = new_id();
    let ws = new_id();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, 'u', 'x', 'U', 0, ?)")
        .bind(&user).bind(&now).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'w', '/tmp', ?)")
        .bind(&ws).bind(&now).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?, 'mcp', 'edit')").bind(&user).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO workspace_members (workspace_id,user_id,role) VALUES (?, ?, 'editor')").bind(&ws).bind(&user).execute(pool).await.unwrap();
    (ws, user)
}

/// A mock MCP server: a read tool and a `delete_thing` (dangerous-by-name) tool.
const MOCK_SERVER: &str = r#"
while IFS= read -r line; do
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"1"}}}\n' ;;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"list_items","description":"list items","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}},{"name":"delete_thing","description":"delete a thing","inputSchema":{"type":"object"}}]}}\n' ;;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"done"}],"isError":false}}\n' ;;
    *'"notifications/initialized"'*) : ;;
  esac
done
"#;

async fn register_mock(svc: &McpService, pool: &SqlitePool, ws: &str, user: &str) -> otto_state::McpServerDetail {
    let server = svc.registry()
        .create(NewServerRow {
            workspace_id: ws.into(),
            name: "mock".into(),
            transport: "stdio".into(),
            command: "sh".into(),
            args: vec!["-c".into(), MOCK_SERVER.into()],
            env: Default::default(),
            url: None,
            description: None,
            headers: Default::default(),
            secret_ref: None,
            secret_env_keys: vec![],
            secret_header_keys: vec![],
            injection_risk: "low".into(),
            default_tool_access: "allow".into(),
            enabled: true,
            created_by: user.into(),
        })
        .await
        .unwrap();
    let repo = otto_state::ResourceAccessRepo::new(pool.clone());
    let old = repo.get_policy(otto_core::access::ResourceKind::McpServer,&server.id).await.unwrap();
    let mut legacy = old.clone(); legacy.mode = otto_core::access::AccessMode::Legacy;
    repo.put_policy(&legacy,old.revision,&otto_core::access::AccessActor {real_user_id:user.into(),effective_user_id:None}).await.unwrap();
    server
}

fn ctx(ws: &str, dry_run: bool) -> InvokeCtx {
    InvokeCtx {
        workspace_id: Some(ws.into()),
        dry_run,
        caller_user_id: Some("u".into()),
        caller_kind: "ui".into(),
        direction: "outbound".into(),
    }
}

#[tokio::test]
async fn discover_labels_risk_and_health_probes() {
    let pool = pool().await;
    let (ws, user) = seed_ws(&pool).await;
    let svc = McpService::new(pool.clone(), Arc::new(MemSecrets::default()));
    let server = register_mock(&svc, &pool, &ws, &user).await;

    // Discovery (req 3) + risk labeling (req 7).
    let tools = svc.discover(&server.id).await.unwrap();
    assert_eq!(tools.len(), 2);
    let del = tools.iter().find(|t| t.name == "delete_thing").unwrap();
    assert_eq!(del.risk_label, "dangerous");
    assert!(del.require_approval, "dangerous tools default to require_approval");
    let list = tools.iter().find(|t| t.name == "list_items").unwrap();
    assert_eq!(list.risk_label, "read");

    // Health (req 2): the mock answers initialize → healthy with a latency.
    let probed = svc.health_check(&server.id).await.unwrap();
    assert_eq!(probed.health_status, "healthy");
    assert!(probed.health_latency_ms.is_some());
}

#[tokio::test]
async fn full_governance_flow() {
    let pool = pool().await;
    let (ws, user) = seed_ws(&pool).await;
    let svc = McpService::new(pool.clone(), Arc::new(MemSecrets::default()));
    let server = register_mock(&svc, &pool, &ws, &user).await;
    svc.discover(&server.id).await.unwrap();

    // 1. A read tool runs straight through (req 12 stats source is populated).
    let out = svc.invoke(&server.id, "list_items", &serde_json::json!({}), &ctx(&ws, false)).await.unwrap();
    assert!(matches!(out, InvokeOutcome::Executed { is_error: false, .. }));

    // 2. The dangerous tool is approval-gated (req 9): first call → pending.
    let args = serde_json::json!({"id": 7});
    let out = svc.invoke(&server.id, "delete_thing", &args, &ctx(&ws, false)).await.unwrap();
    let approval_id = match out {
        InvokeOutcome::Pending { approval_id, .. } => approval_id,
        other => panic!("expected pending approval, got {other:?}"),
    };

    // Approve it (a different principal), then the SAME args execute.
    svc.approvals().decide(&approval_id, true, "approver", None).await.unwrap();
    let out = svc.invoke(&server.id, "delete_thing", &args, &ctx(&ws, false)).await.unwrap();
    assert!(matches!(out, InvokeOutcome::Executed { is_error: false, .. }), "approved call should execute");

    // 3. Single-use (req 9 / F2): the approval is consumed — a replay re-gates.
    let out = svc.invoke(&server.id, "delete_thing", &args, &ctx(&ws, false)).await.unwrap();
    assert!(matches!(out, InvokeOutcome::Pending { .. }), "consumed approval must not be reusable");

    // 4. Dry-run (req 10): pure preview, never executes, regardless of risk.
    let out = svc.invoke(&server.id, "delete_thing", &args, &ctx(&ws, true)).await.unwrap();
    match out {
        InvokeOutcome::DryRun { preview } => assert_eq!(preview["executed"], serde_json::json!(false)),
        other => panic!("expected dry-run, got {other:?}"),
    }

    // 5. Per-tool permission (req 4): disabling list_items denies it.
    let lt = svc.tools().get_by_name(&server.id, "list_items").await.unwrap();
    svc.tools().patch(&lt.id, Some(false), None, None, None).await.unwrap();
    let out = svc.invoke(&server.id, "list_items", &serde_json::json!({}), &ctx(&ws, false)).await.unwrap();
    assert!(matches!(out, InvokeOutcome::Denied { .. }), "disabled tool must be denied");

    // 6. Audit (req 8): every terminal path wrote a row; deny + pending are present.
    let log = svc.call_log().list(&otto_state::CallLogQuery { limit: 100, ..Default::default() }).await.unwrap();
    assert!(log.len() >= 6, "expected an audit row per terminal decision, got {}", log.len());
    assert!(log.iter().any(|r| r.decision == "denied"));
    assert!(log.iter().any(|r| r.decision == "pending_approval"));
    assert!(log.iter().any(|r| r.decision == "approved"));
    assert!(log.iter().any(|r| r.decision == "dry_run"));

    // Stats (req 12) aggregate the executed calls.
    let stats = svc.call_log().stats(None).await.unwrap();
    assert!(stats.iter().any(|s| s.tool == "list_items" && s.calls >= 1));
}

#[tokio::test]
async fn allowlist_and_policy_deny() {
    let pool = pool().await;
    let (ws, user) = seed_ws(&pool).await;
    let svc = McpService::new(pool.clone(), Arc::new(MemSecrets::default()));
    let server = register_mock(&svc, &pool, &ws, &user).await;
    svc.discover(&server.id).await.unwrap();

    // Per-workspace allowlist (req 5): deny list_items in this workspace.
    McpAllowlistRepo::new(pool.clone())
        .replace_for_ws(
            &ws,
            &[NewAllowlistEntry { server_id: server.id.clone(), tool_name: Some("list_items".into()), mode: "deny".into() }],
            &user,
        )
        .await
        .unwrap();
    let out = svc.invoke(&server.id, "list_items", &serde_json::json!({}), &ctx(&ws, false)).await.unwrap();
    assert!(matches!(out, InvokeOutcome::Denied { .. }), "allowlist deny must block");

    // Policy-as-code (req 11): a global deny rule on injection-high tools etc.
    // Here: deny anything on this server by name via a most-restrictive rule.
    SettingsRepo::new(pool.clone()).put("mcp_require_approval_dangerous", &serde_json::json!(false)).await.unwrap();
    svc.policies()
        .create(NewPolicy {
            workspace_id: None,
            name: "block-deletes".into(),
            enabled: true,
            priority: 10,
            match_json: serde_json::json!({ "tool_glob": "delete_*" }),
            effect: "deny".into(),
            reason: Some("no deletes via MCP".into()),
            created_by: user.clone(),
        })
        .await
        .unwrap();
    let out = svc.invoke(&server.id, "delete_thing", &serde_json::json!({}), &ctx(&ws, false)).await.unwrap();
    match out {
        InvokeOutcome::Denied { reason } => assert!(reason.contains("policy"), "got: {reason}"),
        other => panic!("expected policy deny, got {other:?}"),
    }
}

#[tokio::test]
async fn resource_denial_blocks_even_readonly_tool_and_dry_run() {
    use otto_core::access::{AccessActor, AccessMode, AccessPolicy, AccessRule, ResourceKind, RuleEffect, SubjectKind};
    use otto_state::resource_access::ResourceAccessRepo;
    let pool = pool().await;
    let (ws, uid) = seed_ws(&pool).await;
    let svc = McpService::new(pool.clone(), Arc::new(MemSecrets::default()));
    let server = register_mock(&svc, &pool, &ws, &uid).await;
    svc.discover(&server.id).await.unwrap();
    let repo = ResourceAccessRepo::new(pool.clone());
    let old = repo.get_policy(ResourceKind::McpServer,&server.id).await.unwrap();
    let policy = AccessPolicy { kind:ResourceKind::McpServer,resource_id:server.id.clone(),mode:AccessMode::Enforced,revision:old.revision,
        rules:vec![AccessRule { id:"navigation-only".into(),subject_kind:SubjectKind::User,subject_id:uid.clone(),effect:RuleEffect::Allow,
            operations:vec!["discover".into()],children:None,grantable_operations:vec![],credential_connection_id:None }] };
    repo.put_policy(&policy,old.revision,&AccessActor { real_user_id:uid.clone(),effective_user_id:None }).await.unwrap();
    for dry_run in [false,true] {
        let mut context = ctx(&ws,dry_run);context.caller_user_id=Some(uid.clone());
        let out=svc.invoke(&server.id,"list_items",&serde_json::json!({}),&context).await.unwrap();
        assert!(matches!(out,InvokeOutcome::Denied{..}),"navigation access must not authorize invocation: {out:?}");
    }
}

#[derive(Clone)]
struct HttpCtx {
    service: Arc<McpService>, pool:SqlitePool, secrets:Arc<dyn SecretStore>, roles:Arc<dyn otto_core::auth::RoleChecker>,
}
impl otto_mcp::McpCtx for HttpCtx {
    fn mcp(&self)->&Arc<McpService> {&self.service}
    fn mcp_pool(&self)->&SqlitePool {&self.pool}
    fn mcp_secrets(&self)->&Arc<dyn SecretStore> {&self.secrets}
    fn roles(&self)->&Arc<dyn otto_core::auth::RoleChecker> {&self.roles}
}
#[tokio::test]
async fn http_filters_servers_tools_configuration_and_guessed_actions() {
    use otto_core::access::*;
    use tower::ServiceExt;
    let pool=pool().await;
    let (ws,uid)=seed_ws(&pool).await;
    let secrets:Arc<dyn SecretStore>=Arc::new(MemSecrets::default());
    let svc=Arc::new(McpService::new(pool.clone(),secrets.clone()));
    let visible=register_mock(&svc,&pool,&ws,&uid).await;
    svc.discover(&visible.id).await.unwrap();
    sqlx::query("UPDATE mcp_servers SET name='visible' WHERE id=?").bind(&visible.id).execute(&pool).await.unwrap();
    let hidden=register_mock(&svc,&pool,&ws,&uid).await;
    let repo=otto_state::ResourceAccessRepo::new(pool.clone());
    for id in [&visible.id,&hidden.id] {
        let mut p=repo.get_policy(ResourceKind::McpServer,id).await.unwrap();
        p.mode=AccessMode::Enforced;
        if id==&visible.id { p.rules.push(AccessRule {id:"reader".into(),subject_kind:SubjectKind::User,subject_id:uid.clone(),effect:RuleEffect::Allow,operations:vec!["discover".into(),"invoke".into()],children:Some(vec!["list_items".into()]),grantable_operations:vec![],credential_connection_id:None}); }
        repo.put_policy(&p,p.revision,&AccessActor {real_user_id:uid.clone(),effective_user_id:None}).await.unwrap();
    }
    sqlx::query("UPDATE mcp_servers SET managed=0 WHERE id=?").bind(&visible.id).execute(&pool).await.unwrap();
    let mut invoke_ctx=ctx(&ws,false); invoke_ctx.caller_user_id=Some(uid.clone());
    assert!(matches!(svc.invoke(&visible.id,"list_items",&serde_json::json!({}),&invoke_ctx).await.unwrap(),InvokeOutcome::Executed {..}),"an enforced raw registration must use the gateway even if its legacy managed flag is false");
    let user=otto_state::UsersRepo::new(pool.clone()).get(&uid).await.unwrap();
    let ctx=HttpCtx {service:svc,pool:pool.clone(),secrets,roles:Arc::new(otto_rbac::RbacRoleChecker::new(pool))};
    let app=otto_mcp::api_router::<HttpCtx>().layer(axum::Extension(otto_core::auth::AuthUser(user))).with_state(ctx);
    for (method,path,expected) in [
        ("GET",format!("/workspaces/{ws}/mcp/servers"),200),
        ("GET",format!("/mcp/servers/{}",visible.id),200),
        ("GET",format!("/mcp/servers/{}",hidden.id),404),
        ("POST",format!("/mcp/servers/{}/tools/delete_thing/invoke",visible.id),404),
        ("DELETE",format!("/mcp/servers/{}",visible.id),403),
    ] {
        let response=app.clone().oneshot(axum::http::Request::builder().method(method).uri(&path).header("content-type","application/json").body(axum::body::Body::from("{} ")).unwrap()).await.unwrap();
        assert_eq!(response.status().as_u16(),expected,"{method} {path}");
        let data=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
        let value:serde_json::Value=serde_json::from_slice(&data).unwrap();
        if expected==200 && path.ends_with("/servers") { assert_eq!(value.as_array().unwrap().len(),1); assert_eq!(value[0]["command"],""); }
        if expected==200 && path.ends_with(&visible.id) { assert_eq!(value["tools"].as_array().unwrap().len(),1); assert_eq!(value["tools"][0]["name"],"list_items"); assert_eq!(value["server"]["command"],""); }
    }
}

#[tokio::test]
async fn delegated_configure_cannot_attach_or_repoint_native_mcp_credentials() {
    use otto_core::access::*;
    use tower::ServiceExt;
    let pool=pool().await;
    let (ws,uid)=seed_ws(&pool).await;
    let secrets:Arc<dyn SecretStore>=Arc::new(MemSecrets::default());
    let svc=Arc::new(McpService::new(pool.clone(),secrets.clone()));
    let server=register_mock(&svc,&pool,&ws,&uid).await;
    let repo=otto_state::ResourceAccessRepo::new(pool.clone());
    let mut p=repo.get_policy(ResourceKind::McpServer,&server.id).await.unwrap();
    p.mode=AccessMode::Enforced;
    p.rules=vec![AccessRule{id:"configure".into(),subject_kind:SubjectKind::User,subject_id:uid.clone(),effect:RuleEffect::Allow,operations:vec!["discover".into(),"configure".into()],children:None,grantable_operations:vec![],credential_connection_id:None}];
    repo.put_policy(&p,p.revision,&AccessActor{real_user_id:uid.clone(),effective_user_id:None}).await.unwrap();
    let user=otto_state::UsersRepo::new(pool.clone()).get(&uid).await.unwrap();
    let ctx=HttpCtx{service:svc,pool:pool.clone(),secrets,roles:Arc::new(otto_rbac::RbacRoleChecker::new(pool))};
    let app=otto_mcp::api_router::<HttpCtx>().layer(axum::Extension(otto_core::auth::AuthUser(user))).with_state(ctx);
    for (body,expected) in [
        (serde_json::json!({"command":"hidden-command"}),403),
        (serde_json::json!({"env":{"AWS_PROFILE":"hidden"}}),403),
        (serde_json::json!({"url":"https://example.com/hidden"}),403),
        (serde_json::json!({"secret_env":{"TOKEN":"replacement"}}),403),
        (serde_json::json!({"description":"cosmetic change"}),200),
    ] {
        let response=app.clone().oneshot(axum::http::Request::builder().method("PATCH").uri(format!("/mcp/servers/{}",server.id)).header("content-type","application/json").body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status().as_u16(),expected,"{body}");
    }
    let response=app.oneshot(axum::http::Request::builder().method("POST").uri(format!("/workspaces/{ws}/mcp/servers")).header("content-type","application/json").body(axum::body::Body::from(serde_json::json!({"name":"alias","transport":"stdio","command":"hidden-command"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status().as_u16(),403);
}
