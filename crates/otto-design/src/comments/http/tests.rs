use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request};
use http_body_util::BodyExt;
use otto_core::auth::{BoxFuture, RoleChecker};
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use crate::store::{NewArtifactRow, NewVersion};

/// `viewer` may only view, `admin` is a workspace admin, everyone else an
/// editor; `w-denied` refuses everybody.
struct Roles;

impl RoleChecker for Roles {
    fn check<'a>(
        &'a self,
        user: &'a User,
        workspace_id: &'a Id,
        min: WorkspaceRole,
    ) -> BoxFuture<'a, otto_core::Result<()>> {
        let has = match user.id.as_str() {
            "viewer" => WorkspaceRole::Viewer,
            "admin" => WorkspaceRole::Admin,
            _ => WorkspaceRole::Editor,
        };
        let ok = workspace_id != "w-denied" && has >= min;
        Box::pin(async move {
            if ok {
                Ok(())
            } else {
                Err(Error::Forbidden("not allowed".into()))
            }
        })
    }
}

#[derive(Clone)]
struct TestCtx {
    pool: otto_state::DbPool,
    dir: Arc<tempfile::TempDir>,
    roles: Arc<dyn RoleChecker>,
    events: tokio::sync::broadcast::Sender<Event>,
}

impl DesignCtx for TestCtx {
    fn design(&self) -> DesignService {
        DesignService::new(
            self.pool.clone(),
            self.dir.path(),
            Some(self.events.clone()),
        )
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

async fn ctx() -> TestCtx {
    TestCtx {
        pool: otto_state::db::test_pool().await,
        dir: Arc::new(tempfile::tempdir().unwrap()),
        roles: Arc::new(Roles),
        events: tokio::sync::broadcast::channel(64).0,
    }
}

fn app_as(ctx: &TestCtx, uid: &str) -> Router {
    let user = User {
        id: uid.into(),
        username: uid.into(),
        display_name: format!("User {uid}"),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    };
    routes::<TestCtx>()
        .with_state(ctx.clone())
        .layer(Extension(AuthUser(user)))
}

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// An artifact in `ws` with two versions; returns their ids (v1, v2 = head).
async fn seed(ctx: &TestCtx, id: &str, ws: &str, source_kind: Option<&str>) -> (String, String) {
    let s = ctx.design();
    s.store()
        .insert_artifact(&NewArtifactRow {
            id: id.into(),
            project_id: None,
            workspace_id: ws.into(),
            studio: "frames".into(),
            format: "html".into(),
            mime: "text/html".into(),
            title: format!("Artifact {id}"),
            status: "draft".into(),
            tags: vec![],
            meta: json!({}),
            source_kind: source_kind.map(str::to_string),
            source_id: source_kind.map(|_| "src1".to_string()),
            created_by: "u1".into(),
            created_by_kind: "user".into(),
            created_session_id: None,
            created_at: None,
        })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for sha in ["s1", "s2"] {
        let v = s
            .store()
            .commit_version(
                NewVersion {
                    artifact_id: id.into(),
                    blob_sha256: sha.into(),
                    size_bytes: 3,
                    kind: "autosave".into(),
                    branch: "main".into(),
                    author_kind: "user".into(),
                    author_id: "u1".into(),
                    session_id: None,
                    message: String::new(),
                    provenance: json!({}),
                },
                None,
            )
            .await
            .unwrap();
        ids.push(v.id);
    }
    (ids[0].clone(), ids[1].clone())
}

#[tokio::test]
async fn comments_roundtrip_with_versions_pins_replies_and_events() {
    let ctx = ctx().await;
    let (v1, v2) = seed(&ctx, "A", "w1", None).await;
    let mut rx = ctx.events.subscribe();
    let u1 = app_as(&ctx, "u1");
    let u2 = app_as(&ctx, "u2");

    // Default version: the head (v2).
    let (st, c) = call(
        &u1,
        Method::POST,
        "/design/artifacts/A/comments",
        Some(json!({ "body": " The CTA is too small ", "pin": { "x": 0.4, "y": 0.2 } })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{c}");
    assert_eq!(c["version_id"], v2.as_str());
    assert_eq!(c["version_seq"], 2);
    assert_eq!(c["body"], "The CTA is too small");
    assert_eq!(c["author_name"], "User u1");
    assert_eq!(c["pin"]["x"], 0.4);
    match rx.try_recv().unwrap() {
        Event::DesignArtifactUpdated {
            change,
            artifact_id,
            ..
        } => assert_eq!((change.as_str(), artifact_id.as_str()), ("comments", "A")),
        other => panic!("unexpected {other:?}"),
    }
    let cid = c["id"].as_str().unwrap().to_string();

    // On an older version, explicitly.
    let (st, old) = call(
        &u2,
        Method::POST,
        "/design/artifacts/A/comments",
        Some(json!({ "body": "Was better here", "version_id": v1 })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{old}");
    assert_eq!(old["version_seq"], 1);
    assert!(old["pin"].is_null());

    // A reply inherits the thread's version and may not carry a pin.
    let (st, reply) = call(
        &u2,
        Method::POST,
        "/design/artifacts/A/comments",
        Some(json!({ "body": "Agreed", "parent_id": cid, "version_id": v1 })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{reply}");
    assert_eq!(reply["version_id"], v2.as_str());
    let (st, _) = call(
        &u2,
        Method::POST,
        "/design/artifacts/A/comments",
        Some(json!({ "body": "x", "parent_id": cid, "pin": { "x": 0.1, "y": 0.1 } })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // One level deep.
    let (st, _) = call(
        &u2,
        Method::POST,
        "/design/artifacts/A/comments",
        Some(json!({ "body": "x", "parent_id": reply["id"] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // Version scoping on the list.
    let (st, on1) = call(
        &u1,
        Method::GET,
        &format!("/design/artifacts/A/comments?version={v1}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(on1.as_array().unwrap().len(), 1);
    let (_, all) = call(&u1, Method::GET, "/design/artifacts/A/comments", None).await;
    assert_eq!(all.as_array().unwrap().len(), 3);

    // Edit own; another editor may not edit it but may resolve it.
    let uri = format!("/design/artifacts/A/comments/{cid}");
    let (st, _) = call(&u2, Method::PATCH, &uri, Some(json!({ "body": "hijack" }))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, e) = call(
        &u1,
        Method::PATCH,
        &uri,
        Some(json!({ "body": "Make it 44px" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{e}");
    assert_eq!(e["body"], "Make it 44px");
    assert!(!e["edited_at"].is_null());
    let (st, r) = call(&u2, Method::PATCH, &uri, Some(json!({ "resolved": true }))).await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["resolved"], true);
    assert_eq!(r["resolved_by"], "u2");
    let (_, open) = call(
        &u1,
        Method::GET,
        "/design/artifacts/A/comments?resolved=false",
        None,
    )
    .await;
    assert_eq!(
        open.as_array().unwrap().len(),
        1,
        "only the v1 thread is open"
    );
    let (st, r) = call(&u1, Method::PATCH, &uri, Some(json!({ "resolved": false }))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(r["resolved"], false);
    // A reply has no resolved state; an empty patch is refused.
    let reply_uri = format!(
        "/design/artifacts/A/comments/{}",
        reply["id"].as_str().unwrap()
    );
    let (st, _) = call(
        &u2,
        Method::PATCH,
        &reply_uri,
        Some(json!({ "resolved": true })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(&u1, Method::PATCH, &uri, Some(json!({}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // Delete: not someone else's (unless a workspace admin); the thread takes its replies.
    let (st, _) = call(&u2, Method::DELETE, &uri, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = call(&u1, Method::DELETE, &uri, None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, left) = call(&u1, Method::GET, "/design/artifacts/A/comments", None).await;
    assert_eq!(left.as_array().unwrap().len(), 1);
    let old_uri = format!(
        "/design/artifacts/A/comments/{}",
        old["id"].as_str().unwrap()
    );
    let (st, _) = call(&app_as(&ctx, "admin"), Method::DELETE, &old_uri, None).await;
    assert_eq!(
        st,
        StatusCode::NO_CONTENT,
        "a workspace admin may delete any comment"
    );
}

#[tokio::test]
async fn comment_access_follows_the_workspace_and_the_artifact() {
    let ctx = ctx().await;
    seed(&ctx, "A", "w1", None).await;
    seed(&ctx, "B", "w1", None).await;
    seed(&ctx, "D", "w-denied", None).await;
    let viewer = app_as(&ctx, "viewer");
    let u1 = app_as(&ctx, "u1");

    // A viewer reads but does not write.
    let (st, _) = call(&viewer, Method::GET, "/design/artifacts/A/comments", None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        &viewer,
        Method::POST,
        "/design/artifacts/A/comments",
        Some(json!({ "body": "hi" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // No access to the workspace at all.
    let (st, _) = call(&u1, Method::GET, "/design/artifacts/D/comments", None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // Unknown artifact.
    let (st, _) = call(&u1, Method::GET, "/design/artifacts/nope/comments", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // A comment is addressed through ITS artifact only.
    let (_, c) = call(
        &u1,
        Method::POST,
        "/design/artifacts/A/comments",
        Some(json!({ "body": "on A" })),
    )
    .await;
    let cid = c["id"].as_str().unwrap();
    let (st, _) = call(
        &u1,
        Method::PATCH,
        &format!("/design/artifacts/B/comments/{cid}"),
        Some(json!({ "resolved": true })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // A version of another artifact is not this one's.
    let (b1, _) = seed(&ctx, "C", "w1", None).await;
    let (st, _) = call(
        &u1,
        Method::POST,
        "/design/artifacts/A/comments",
        Some(json!({ "body": "x", "version_id": b1 })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bodies_and_pins_are_validated_and_mirrors_take_comments() {
    let ctx = ctx().await;
    seed(&ctx, "M", "w1", Some(crate::import::SOURCE_ATTACHMENT)).await;
    let u1 = app_as(&ctx, "u1");
    for bad in [
        json!({ "body": "   " }),
        json!({ "body": "x".repeat(super::super::MAX_BODY_CHARS + 1) }),
        json!({ "body": "x", "pin": { "x": 1.5, "y": 0.5 } }),
        json!({ "body": "x", "pin": { "x": 0.5, "y": -0.1 } }),
    ] {
        let (st, _) = call(
            &u1,
            Method::POST,
            "/design/artifacts/M/comments",
            Some(bad.clone()),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}");
    }
    // A mirrored (read-only) design: a comment does not change it.
    let (st, c) = call(
        &u1,
        Method::POST,
        "/design/artifacts/M/comments",
        Some(json!({ "body": "Fine on a mirror", "pin": { "x": 1.0, "y": 0.0, "frame": "p2" } })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{c}");
    assert_eq!(c["pin"]["frame"], "p2");
    // The list limit is clamped, never unbounded.
    let (st, l) = call(
        &u1,
        Method::GET,
        "/design/artifacts/M/comments?limit=100000",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(l.as_array().unwrap().len(), 1);
}
