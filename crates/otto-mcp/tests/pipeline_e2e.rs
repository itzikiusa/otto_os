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

async fn register_mock(svc: &McpService, ws: &str, user: &str) -> otto_state::McpServerDetail {
    svc.registry()
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
        .unwrap()
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
    let server = register_mock(&svc, &ws, &user).await;

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
    let server = register_mock(&svc, &ws, &user).await;
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
    let server = register_mock(&svc, &ws, &user).await;
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
