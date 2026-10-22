//! Per-route upstream budgets (integrations-3-01): a route that blocks by
//! design must not be cut off by the bridge's default [`CALL_TIMEOUT`].

use super::*;

#[test]
fn wait_session_budget_covers_the_daemon_wait_plus_margin() {
    let s = |v: Value| wait_session_budget(&v).as_secs();
    // The daemon waits 20 s by default and at most 25 s.
    assert_eq!(s(json!({})), 25);
    assert_eq!(s(json!({"timeout_secs": 25})), 30);
    assert_eq!(s(json!({"timeout_secs": 600})), 30);
    assert_eq!(s(json!({"timeout_secs": 0})), 6);
    assert_eq!(s(json!({"timeout_secs": "14"})), 19);
    assert!(wait_session_budget(&json!({})) > CALL_TIMEOUT);
    let call = read_route("otto_wait_session", &json!({"session_id": "s"}), Some("ws")).unwrap();
    assert_eq!(call.budget, Duration::from_secs(25));
}

/// A stub daemon that answers the wait after 21 s — past the old 20 s
/// bridge timer — still reaches the agent as the upstream body, not
/// "upstream timeout after 20s".
#[tokio::test]
async fn default_wait_session_returns_a_body_that_arrives_after_21s() {
    // A fallback, not a `.route`, so the route-inventory scanner sees no
    // fictitious daemon path. A reference lookup (if any) answers at once.
    let app = axum::Router::new().fallback(
        |axum::extract::OriginalUri(uri): axum::extract::OriginalUri| async move {
            if uri.path().ends_with("/refs/resolve") {
                return axum::Json(json!({"id": "s"}));
            }
            tokio::time::sleep(Duration::from_secs(21)).await;
            axum::Json(json!({"id": "s", "status": "working", "reached": false}))
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut ctx = test_ctx();
    ctx.base = format!("http://{addr}");
    let out = run_tool(&ctx, "otto_wait_session", &json!({"session_id": "s"})).await;
    server.abort();
    let (body, _) = out.expect("the late daemon answer is returned, not a timeout");
    assert_eq!(body["reached"], json!(false));
}

/// integrations-4-01: a downstream gateway call (`mcp__<server>__<tool>`) the
/// daemon answers after 21 s — a cold open plus a slow tool — reaches the
/// agent as the executed envelope, not "upstream timeout after 20s".
#[tokio::test]
async fn gateway_call_returns_an_envelope_that_arrives_after_21s() {
    assert!(otto_mcp::client::GATEWAY_CALL_TIMEOUT > Duration::from_secs(21));
    let app = axum::Router::new().fallback(
        |axum::extract::OriginalUri(uri): axum::extract::OriginalUri| async move {
            if uri.path().ends_with("/mcp/gateway/tools") {
                return axum::Json(json!({"tools": [
                    {"name": "mcp__jira__create", "server_id": "srv", "tool": "create"}
                ]}));
            }
            tokio::time::sleep(Duration::from_secs(21)).await;
            axum::Json(json!({"decision": "allowed", "executed": true,
                              "content": {"key": "OT-1"}}))
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut ctx = test_ctx();
    ctx.base = format!("http://{addr}");
    let out = gateway_call(&ctx, "mcp__jira__create", &json!({"summary": "x"})).await;
    server.abort();
    let (envelope, is_error) = out.expect("the late daemon answer is returned, not a timeout");
    assert!(!is_error);
    assert_eq!(envelope["executed"], json!(true));
    assert_eq!(envelope["content"]["key"], json!("OT-1"));
}

/// integrations-4-03: the client-side cap the spawner hands codex covers the
/// gateway budget too (it lives in otto-mcp, which otto-core can't see).
#[test]
fn the_client_tool_timeout_covers_the_gateway_budget() {
    assert!(
        Duration::from_secs(otto_core::mcp_budgets::CLIENT_TOOL_TIMEOUT_SECS)
            > otto_mcp::client::GATEWAY_CALL_TIMEOUT
    );
}
