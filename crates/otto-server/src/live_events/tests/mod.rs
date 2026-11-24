use super::*;
use axum::body::Body;
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::Router;
use tower::ServiceExt;

#[test]
fn classifies_access_writes() {
    let put_ = Method::PUT;
    assert_eq!(
        access_change_for(&put_, "/api/v1/access/connection/01ABC"),
        Some((Some("connection".into()), Some("01ABC".into())))
    );
    assert_eq!(
        access_change_for(&Method::POST, "/api/v1/access/connection/01ABC/preview"),
        None
    );
    assert_eq!(
        access_change_for(&Method::GET, "/api/v1/access/connection/01ABC"),
        None
    );
    assert_eq!(
        access_change_for(&Method::DELETE, "/api/v1/access/groups/g1"),
        Some((None, None))
    );
    assert_eq!(
        access_change_for(&put_, "/api/v1/access/groups/g1/members/u1"),
        Some((None, None))
    );
    assert_eq!(
        access_change_for(&Method::POST, "/api/v1/access/roles"),
        Some((None, None))
    );
    assert_eq!(
        access_change_for(&put_, "/api/v1/users/u1/grants"),
        Some((None, None))
    );
    assert_eq!(
        access_change_for(&Method::PATCH, "/api/v1/users/u1"),
        Some((None, None))
    );
    assert_eq!(
        access_change_for(&Method::POST, "/api/v1/workspaces/w1/members"),
        Some((None, None))
    );
    assert_eq!(
        access_change_for(&Method::POST, "/api/v1/workspaces/w1/sessions"),
        None
    );
    assert_eq!(access_change_for(&Method::POST, "/api/v1/users"), None);
    assert_eq!(access_change_for(&put_, "/access/connection/x"), None);
}

#[test]
fn classifies_connection_library_writes() {
    let yes = [
        (Method::POST, "/api/v1/workspaces/w1/connections"),
        (
            Method::POST,
            "/api/v1/workspaces/w1/connections/import/create",
        ),
        (Method::PATCH, "/api/v1/connections/c1"),
        (Method::DELETE, "/api/v1/connections/c1"),
        (Method::POST, "/api/v1/connections/c1/duplicate"),
        (Method::PATCH, "/api/v1/connections/c1/pin"),
        (Method::POST, "/api/v1/workspaces/w1/connection-sections"),
        (
            Method::POST,
            "/api/v1/workspaces/w1/connection-sections/reorder",
        ),
        (Method::PATCH, "/api/v1/connection-sections/s1"),
        (Method::POST, "/api/v1/connection-sections/s1/move"),
    ];
    for (m, p) in yes {
        assert!(connections_change_for(&m, p), "{m} {p}");
    }
    let no = [
        (Method::GET, "/api/v1/workspaces/w1/connections"),
        (
            Method::POST,
            "/api/v1/workspaces/w1/connections/import/scan",
        ),
        (Method::POST, "/api/v1/connections/c1/test"),
        (Method::POST, "/api/v1/connections/c1/open"),
        (Method::POST, "/api/v1/connections/unsaved/test"),
        (Method::POST, "/api/v1/connections/c1/db/query"),
        (Method::POST, "/api/v1/connections/c1/sftp/mkdir"),
        (Method::PATCH, "/connections/c1"),
    ];
    for (m, p) in no {
        assert!(!connections_change_for(&m, p), "{m} {p}");
    }
}

#[tokio::test]
async fn emits_connections_changed_after_a_successful_library_write() {
    let (tx, mut rx) = broadcast::channel::<Event>(8);
    let app = Router::new()
        .route(
            "/api/v1/connections/{id}",
            axum::routing::patch(|| async { StatusCode::OK })
                .delete(|| async { StatusCode::NOT_FOUND }),
        )
        .layer(axum::middleware::from_fn_with_state(
            tx.clone(),
            notify_access_changes,
        ));
    let call = |m: Method| {
        app.clone().oneshot(
            Request::builder()
                .method(m)
                .uri("/api/v1/connections/c1")
                .body(Body::empty())
                .unwrap(),
        )
    };
    call(Method::DELETE).await.unwrap(); // 404 → nothing
    call(Method::PATCH).await.unwrap();
    assert!(matches!(
        rx.try_recv().unwrap(),
        Event::ConnectionsChanged {}
    ));
    assert!(rx.try_recv().is_err(), "exactly one event");
}

#[tokio::test]
async fn emits_only_after_a_successful_write() {
    let (tx, mut rx) = broadcast::channel::<Event>(8);
    let app = Router::new()
        .route(
            "/api/v1/access/{kind}/{id}",
            put(|| async { StatusCode::OK }).get(|| async { "p" }),
        )
        .route(
            "/api/v1/users/{id}/grants",
            put(|| async { StatusCode::FORBIDDEN }),
        )
        .route("/api/v1/health", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            tx.clone(),
            notify_access_changes,
        ));
    let call = |m: Method, uri: &'static str| {
        app.clone().oneshot(
            Request::builder()
                .method(m)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
    };
    call(Method::GET, "/api/v1/access/connection/c1")
        .await
        .unwrap();
    call(Method::PUT, "/api/v1/users/u1/grants").await.unwrap(); // 403 → nothing
    call(Method::GET, "/api/v1/health").await.unwrap();
    call(Method::PUT, "/api/v1/access/connection/c1")
        .await
        .unwrap();
    match rx.try_recv().unwrap() {
        Event::ResourceAccessChanged { kind, resource_id } => {
            assert_eq!(kind.as_deref(), Some("connection"));
            assert_eq!(resource_id.as_deref(), Some("c1"));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(rx.try_recv().is_err(), "exactly one event");
}
