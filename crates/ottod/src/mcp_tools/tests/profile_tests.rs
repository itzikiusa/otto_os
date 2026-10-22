//! MCP profiles on the session stdio surface (`mcp_tools/profile.rs`).

use super::*;
use otto_server::mcp_profiles::{canonical, GROUPS};

/// Every tool this surface can list — the native catalog (assistant tools
/// included), the UI-control request stub and every governed tool under its
/// stdio name — belongs to exactly one profile group. A native tool added
/// without a group fails here (`otto_mcp::profiles` holds the table).
#[test]
fn native_catalog_tools_have_exactly_one_group() {
    let mut names: Vec<String> = native_tool_names();
    names.push(UI_REQUEST_CONTROL_TOOL.to_string());
    names.extend(
        GOVERNED_SPECS
            .iter()
            .filter_map(|s| s["name"].as_str().map(governed_stdio_name)),
    );
    let bad: Vec<String> = names
        .iter()
        .filter_map(|n| {
            let hits: Vec<&str> = GROUPS
                .iter()
                .filter(|g| g.contains(canonical(n)))
                .map(|g| g.id)
                .collect();
            (hits.len() != 1).then(|| format!("{n} → {hits:?}"))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "tools without exactly one MCP profile group (add them to otto_mcp::profiles::GROUPS):\n{}",
        bad.join("\n")
    );
}

#[test]
fn profile_ids_ride_the_creds_and_the_enable_read() {
    let c = creds::parse_creds(r#"{"token":"t","profiles":"database,issues"}"#)
        .ok()
        .unwrap();
    assert_eq!(c.profiles.as_deref(), Some("database,issues"));
    assert_eq!(
        creds::parse_creds(r#"{"token":"t"}"#)
            .ok()
            .unwrap()
            .profiles,
        None
    );
    assert_eq!(
        profile::enabled_path(Some("database, is&sues,")),
        "/mcp/otto-server/enabled?profiles=database,issues"
    );
    assert_eq!(profile::enabled_path(None), "/mcp/otto-server/enabled");
}

/// A mock daemon (fallback handler — no fictitious `.route` literals for the
/// route-inventory scanner) that resolves `?profiles=git` to the Git group,
/// counts governed invokes and records the bridge's audit appends.
#[derive(Default)]
struct Daemon {
    invokes: std::sync::Mutex<Vec<String>>,
    audits: std::sync::Mutex<Vec<Value>>,
    enabled_queries: std::sync::Mutex<Vec<String>>,
}

async fn daemon(state: Arc<Daemon>) -> String {
    use axum::extract::OriginalUri;
    let app = axum::Router::new().fallback(move |OriginalUri(uri): OriginalUri, body: String| {
        let m = state.clone();
        async move {
            let path = uri.path().to_string();
            if path.ends_with("/mcp/otto-server/enabled") {
                let q = uri.query().unwrap_or_default().to_string();
                m.enabled_queries.lock().unwrap().push(q.clone());
                let profile = q.contains("profiles=git").then(|| {
                    json!({"ids": ["git"], "label": "Git & PRs", "profiles": [
                        {"id": "git", "name": "Git & PRs", "groups": ["git", "code_review"]}]})
                });
                axum::Json(json!({
                    "enabled": ["otto.create_pr", "otto.open_pr_draft", "otto.run_workflow"],
                    "outward_enabled": true,
                    "ui_granted": false,
                    "profile": profile,
                }))
            } else if path.ends_with("/mcp/otto-tools/invoke") {
                let v: Value = serde_json::from_str(&body).unwrap_or_default();
                m.invokes
                    .lock()
                    .unwrap()
                    .push(v["tool"].as_str().unwrap_or_default().into());
                axum::Json(
                    json!({"decision": "allowed", "executed": true, "content": {"ok": true}}),
                )
            } else if path.ends_with("/mcp/tool-calls") {
                m.audits
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&body).unwrap_or_default());
                axum::Json(json!({}))
            } else {
                axum::Json(json!({"error": format!("unexpected {path}")}))
            }
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn listed(ctx: &Ctx) -> Vec<String> {
    let resp = handle(
        ctx,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await
    .unwrap();
    resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

async fn call(ctx: &Ctx, name: &str) -> Value {
    handle(
        ctx,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": name, "arguments": {}}}),
    )
    .await
    .unwrap()["result"]
        .clone()
}

#[tokio::test]
async fn stdio_lists_and_dispatches_only_the_profile_tools() {
    let state = Arc::new(Daemon::default());
    let mut ctx = ctx_at(&daemon(state.clone()).await);
    ctx.audit = true;
    ctx.profiles = Some("git".into());

    let names = listed(&ctx).await;
    assert!(state.enabled_queries.lock().unwrap()[0].contains("profiles=git"));
    for want in [
        "otto_list_repos",
        "otto_git_pr_review",
        "otto_create_pr",
        "otto_open_pr_draft",
    ] {
        assert!(
            names.iter().any(|n| n == want),
            "{want} missing from {names:?}"
        );
    }
    for hidden in [
        "otto_db_query",
        "k8s_logs",
        "aws_s3_list_buckets",
        "otto_run_workflow",
        "otto_vault_read",
    ] {
        assert!(
            !names.iter().any(|n| n == hidden),
            "{hidden} leaked into a Git profile"
        );
    }
    assert!(names.iter().all(|n| otto_server::mcp_profiles::group_of(n)
        .is_some_and(|g| g.id == "git" || g.id == "code_review")));

    // A native tool outside the profile: refused before any daemon route,
    // naming the profile, and audited as a failed call.
    let r = call(&ctx, "k8s_logs").await;
    assert_eq!(r["isError"], true);
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("k8s_logs") && text.contains("MCP profile") && text.contains("Git & PRs"),
        "{text}"
    );
    // A governed tool outside it never reaches the governed invoke.
    assert_eq!(call(&ctx, "otto_run_workflow").await["isError"], true);
    assert!(state.invokes.lock().unwrap().is_empty());
    // One inside it runs the normal governed path.
    assert_eq!(call(&ctx, "otto_create_pr").await["isError"], false);
    assert_eq!(*state.invokes.lock().unwrap(), ["otto.create_pr"]);
    ctx.drain_audit().await;
    let audited: Vec<(String, bool)> = state
        .audits
        .lock()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["tool"].as_str().unwrap().to_string(),
                a["ok"].as_bool().unwrap(),
            )
        })
        .collect();
    assert!(audited.contains(&("k8s_logs".into(), false)), "{audited:?}");
}

#[tokio::test]
async fn no_profile_leaves_the_listing_and_calls_untouched() {
    let state = Arc::new(Daemon::default());
    let ctx = ctx_at(&daemon(state.clone()).await);
    let names = listed(&ctx).await;
    for n in [
        "k8s_logs",
        "otto_db_query",
        "otto_run_workflow",
        "otto_create_pr",
    ] {
        assert!(
            names.iter().any(|x| x == n),
            "{n} missing without a profile"
        );
    }
    assert_eq!(state.enabled_queries.lock().unwrap()[0], "");
    assert_eq!(call(&ctx, "otto_run_workflow").await["isError"], false);
    assert_eq!(*state.invokes.lock().unwrap(), ["otto.run_workflow"]);
}
