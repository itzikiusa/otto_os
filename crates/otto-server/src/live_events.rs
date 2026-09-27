//! Invalidation events derived from successful HTTP writes (TRANSPORT_PLAN
//! stage 2). Some UI caches have no natural single emit point: effective
//! resource access depends on resource policies, access groups/roles, user
//! grants and workspace roles, all written through different route families.
//! Rather than threading the event bus through each handler, one outer
//! middleware watches for a SUCCESSFUL write on those paths and broadcasts
//! `resource_access_changed`, so the UI's access cache refreshes on change
//! instead of polling every resource every 15 s.

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use otto_core::event::Event;
use tokio::sync::broadcast;

/// `(kind, resource_id)` for a write that may change effective access, or
/// `None` when the request cannot. `(None, None)` = "anything may have
/// changed" (a group/role/grant/membership write). `path` is the full request
/// path (`/api/v1/...`).
pub fn access_change_for(method: &Method, path: &str) -> Option<(Option<String>, Option<String>)> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return None;
    }
    let rest = path.strip_prefix("/api/v1/")?;
    let segs: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    match segs.as_slice() {
        // Resource policy (`PUT /access/{kind}/{id}`); `…/preview` is a dry run.
        ["access", "groups" | "roles", ..] => Some((None, None)),
        ["access", kind, id] => Some((Some((*kind).to_string()), Some((*id).to_string()))),
        // A user's feature grants, plugin grants, or the user row (disable/delete).
        ["users", _, "grants" | "plugin-grants"] | ["users", _] => Some((None, None)),
        // Workspace membership / role changes.
        ["workspaces", _, "members", ..] => Some((None, None)),
        _ => None,
    }
}

/// Outer middleware: after a 2xx write that [`access_change_for`] flags,
/// broadcast `resource_access_changed`.
pub async fn notify_access_changes(
    State(tx): State<broadcast::Sender<Event>>,
    req: Request,
    next: Next,
) -> Response {
    let hit = access_change_for(req.method(), req.uri().path());
    let resp = next.run(req).await;
    if let Some((kind, resource_id)) = hit {
        if resp.status().is_success() {
            let _ = tx.send(Event::ResourceAccessChanged { kind, resource_id });
        }
    }
    resp
}

#[cfg(test)]
mod tests {
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
}
