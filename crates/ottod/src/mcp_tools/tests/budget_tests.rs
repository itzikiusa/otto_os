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
