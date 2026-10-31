//! architecture-1-05 characterization: what a session sees and what each
//! native `otto_x` ⇄ governed `otto.x` twin dispatches, pinned as JSON
//! snapshots taken on the hand-written catalog / native `run_tool` arms
//! BEFORE the twins were generated from the control-plane catalog and routed
//! through `route_for`. Both must hold unchanged after that move:
//!
//! - `tools_list_snapshot.json`: per session source, every `tools/list` name
//!   (every control-plane tool enabled, UI grant off) and, for the default
//!   source, each tool's `inputSchema`;
//! - `twin_dispatch_snapshot.json`: for representative arguments of every
//!   twin, the `(method, path?query, body)` sequence `run_tool` sends to the
//!   daemon and the value it returns.
//!
//! Re-bless (only for a deliberate change, reviewed in the diff):
//! `OTTO_BLESS_TWIN_SNAPSHOTS=1 cargo test -p ottod twin_tests`.

use super::*;
use std::sync::{Arc, Mutex};

const TOOLS_LIST_SNAPSHOT: &str = include_str!("tools_list_snapshot.json");
const TWIN_DISPATCH_SNAPSHOT: &str = include_str!("twin_dispatch_snapshot.json");

/// Canonical ids (ULID / short numeric), so no friendly-reference lookup runs.
const ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
/// What the stub daemon resolves any repo reference to.
const REPO: &str = "01ARZ3NDEKTSV4RRFFQ69G5REP";

fn bless(file: &str, value: &Value) -> bool {
    if std::env::var_os("OTTO_BLESS_TWIN_SNAPSHOTS").is_none() {
        return false;
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/mcp_tools/tests")
        .join(file);
    let text = serde_json::to_string_pretty(value).unwrap() + "\n";
    std::fs::write(&path, text).unwrap();
    true
}

fn assert_snapshot(file: &str, pinned: &str, now: &Value) {
    if bless(file, now) {
        return;
    }
    let pinned: Value = serde_json::from_str(pinned).expect("snapshot parses");
    if &pinned == now {
        return;
    }
    // Name the first differing entry instead of dumping two large documents.
    let (Value::Object(p), Value::Object(n)) = (&pinned, now) else {
        panic!("{file}: shape changed");
    };
    for (k, v) in p {
        assert_eq!(n.get(k), Some(v), "{file}: `{k}` changed");
    }
    let extra: Vec<&String> = n.keys().filter(|k| !p.contains_key(*k)).collect();
    panic!("{file}: new entries {extra:?}");
}

/// The `tools/list` a session of `source` is shown with every control-plane
/// tool enabled and no UI grant (the narrow-source scoping applied).
fn listing(source: Option<&str>) -> Vec<Value> {
    let info = EnabledInfo {
        names: GOVERNED_SPECS
            .iter()
            .filter_map(|s| s["name"].as_str().map(str::to_string))
            .collect(),
        ui_granted: false,
        filter: None,
    };
    let mut cat = tool_catalog_for_source(source);
    let tools = cat["tools"].as_array_mut().unwrap();
    tools.extend(governed_tools_for_session(&info));
    scope_listing_for_source(tools, source);
    tools.clone()
}

#[test]
fn tools_list_names_and_schemas_per_source_are_pinned() {
    let mut out = serde_json::Map::new();
    let sources: [(&str, Option<&str>); 5] = [
        ("default", None),
        ("review", Some("review")),
        ("workflow", Some("workflow")),
        ("vault-docs-review", Some("vault-docs-review")),
        ("skilleval", Some("skilleval")),
    ];
    for (label, source) in sources {
        let mut names: Vec<String> = listing(source)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        out.insert(format!("names:{label}"), json!(names));
    }
    let schemas: serde_json::Map<String, Value> = listing(None)
        .into_iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["inputSchema"].clone(),
            )
        })
        .collect();
    out.insert("schemas:default".into(), Value::Object(schemas));
    assert_snapshot(
        "tools_list_snapshot.json",
        TOOLS_LIST_SNAPSHOT,
        &Value::Object(out),
    );
}

type Seen = Arc<Mutex<Vec<Value>>>;

/// A stub daemon that answers every reference lookup with a canonical id and
/// records every other request as `[method, path?query (no /api/v1), body]`.
async fn recorder() -> (String, Seen, tokio::task::JoinHandle<()>) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    // A fallback, not a `.route`: the route-inventory scanner sees no
    // fictitious daemon path.
    let app = axum::Router::new().fallback(
        move |method: axum::http::Method, uri: axum::http::Uri, body: axum::body::Bytes| {
            let log = log.clone();
            async move {
                let pq = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
                let pq = pq.strip_prefix("/api/v1").unwrap_or(pq).to_string();
                if pq.starts_with("/git/repos/resolve") {
                    return axum::Json(json!({"repo": {"id": REPO}}));
                }
                if pq.starts_with("/refs/resolve") {
                    return axum::Json(json!({"id": ID}));
                }
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                log.lock().unwrap().push(json!([method.as_str(), pq, body]));
                axum::Json(json!({"ok": true}))
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen, server)
}

/// Representative arguments for every twin (several for the ones with
/// defaults or aliases).
fn twin_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("otto_list_connections", json!({})),
        ("otto_list_connections", json!({"workspace_id": ID})),
        ("otto_list_workspaces", json!({})),
        ("otto_list_workflows", json!({})),
        ("otto_list_workflow_runs", json!({"workflow_id": ID})),
        (
            "otto_list_workflow_runs",
            json!({"workflow_id": ID, "summary": false}),
        ),
        ("otto_get_workflow_run", json!({"run_id": "run-1"})),
        ("otto_list_goal_loops", json!({})),
        ("otto_list_broker_clusters", json!({})),
        ("otto_list_broker_topics", json!({"cluster_id": ID})),
        ("otto_list_issue_accounts", json!({})),
        (
            "otto_search_issues",
            json!({"account_id": ID, "query": "bug", "project": "OT", "limit": 5, "start_at": 10}),
        ),
        (
            "otto_search_issues",
            json!({"account_id": ID, "jql": "project = OT", "filter": {"state": "open"}}),
        ),
        (
            "otto_list_issue_transitions",
            json!({"account_id": ID, "key": "OT-1"}),
        ),
        ("otto_list_repos", json!({})),
        ("otto_list_repos", json!({"workspace_id": ID})),
        ("otto_list_sessions", json!({})),
        ("otto_list_sessions", json!({"workspace_id": ID})),
        ("otto_get_session", json!({"session_id": "s1"})),
        (
            "otto_wait_session",
            json!({"session_id": "s1", "status": "idle", "timeout_secs": 1}),
        ),
        (
            "otto_open_session",
            json!({"provider": "claude", "title": "t", "prompt": "p", "model": ""}),
        ),
        (
            "otto_send_message",
            json!({"session_id": "s1", "text": "hi"}),
        ),
        ("otto_list_product_stories", json!({})),
        ("otto_list_findings", json!({"review_id": "r1"})),
        ("otto_list_prs", json!({"repo_id": "web", "state": "open"})),
        ("otto_get_pr", json!({"repo_id": "web", "number": 7})),
        ("otto_get_pr", json!({"repo_id": "web", "pr_number": "8"})),
        (
            "otto_comment_pr",
            json!({"repo_id": "web", "number": 7, "body": "b", "path": "a.rs", "line": 3}),
        ),
        (
            "otto_comment_pr",
            json!({"repo_id": "web", "pr_number": 7, "body": "b"}),
        ),
        ("otto_list_improvement_runs", json!({})),
        ("otto_list_improvement_edits", json!({})),
        ("otto_list_improvement_edits", json!({"status": "pending"})),
        ("otto_vault_list", json!({})),
        ("otto_vault_dir", json!({"vault_id": 3, "path": "docs"})),
        ("otto_vault_dir", json!({"vault_id": 3})),
        ("otto_vault_read", json!({"vault_id": 3, "path": "a b.md"})),
        ("otto_vault_search", json!({"vault_id": 3, "query": "q"})),
        (
            "otto_vault_search",
            json!({"vault_id": 3, "query": "q", "limit": "7"}),
        ),
        (
            "otto_vault_backlinks",
            json!({"vault_id": 3, "path": "a.md"}),
        ),
        ("otto_vault_tags", json!({"vault_id": 3})),
        ("otto_vault_graph", json!({"vault_id": 3})),
        (
            "otto_vault_graph",
            json!({"vault_id": 3, "path": "a.md", "depth": 2}),
        ),
        ("otto_vault_okf_validate", json!({"vault_id": 3})),
        (
            "otto_vault_write",
            json!({"vault_id": 3, "path": "a.md", "content": "x", "if_hash": "h"}),
        ),
        (
            "otto_vault_write_file",
            json!({"vault_id": 3, "path": "a.txt", "content": ""}),
        ),
        (
            "otto_vault_rename",
            json!({"vault_id": 3, "from": "a.md", "to": "b.md"}),
        ),
        ("otto_vault_delete", json!({"vault_id": 3, "path": "a.md"})),
        ("otto_list_agent_rooms", json!({})),
        ("otto_room_post", json!({"room_id": ID, "text": "hi"})),
        ("otto_room_read", json!({"room_id": ID})),
        (
            "otto_room_read",
            json!({"room_id": ID, "after": "m1", "limit": 10}),
        ),
        ("otto_room_read", json!({"room_id": ID, "before": "m9"})),
        ("otto_api_list", json!({})),
        ("otto_api_list", json!({"q": "x", "kind": "requests"})),
        ("otto_api_get_request", json!({"request_id": ID})),
        ("otto_api_history", json!({"limit": 5})),
        ("otto_api_execute", json!({"request_id": ID})),
        (
            "otto_api_upsert_request",
            json!({"name": "n", "method": "GET", "url": "http://example.test"}),
        ),
        ("otto_api_run_automation", json!({"automation_id": ID})),
    ]
}

/// Every twin has a case: a new twin must be characterized too.
#[test]
fn every_twin_has_a_dispatch_case() {
    let cased: std::collections::HashSet<&str> = twin_cases().iter().map(|(n, _)| *n).collect();
    let twins: Vec<String> = GOVERNED_SPECS
        .iter()
        .filter_map(|s| s["name"].as_str())
        .map(governed_stdio_name)
        .filter(|n| NATIVE_TOOL_NAMES.contains(n))
        .collect();
    assert_eq!(twins.len(), 45, "{twins:?}");
    let missing: Vec<&String> = twins
        .iter()
        .filter(|n| !cased.contains(n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "twins with no dispatch case: {missing:?}"
    );
}

#[tokio::test]
async fn twin_dispatch_is_pinned() {
    let (base, seen, server) = recorder().await;
    let mut ctx = test_ctx();
    ctx.base = base;
    let mut out = serde_json::Map::new();
    for (tool, args) in twin_cases() {
        seen.lock().unwrap().clear();
        let result = match run_tool(&ctx, tool, &args).await {
            Ok((v, rows)) => json!({"ok": v, "rows": rows}),
            Err(e) => json!({"err": e}),
        };
        let requests = std::mem::take(&mut *seen.lock().unwrap());
        out.insert(
            format!("{tool} {args}"),
            json!({"args": args, "requests": requests, "result": result}),
        );
    }
    server.abort();
    assert_snapshot(
        "twin_dispatch_snapshot.json",
        TWIN_DISPATCH_SNAPSHOT,
        &Value::Object(out),
    );
}
