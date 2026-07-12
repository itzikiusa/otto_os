use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::{Extension, Router};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id, Result};
use otto_vault::{VaultCtx, VaultEngine};
use serde_json::{json, Value};
use tower::ServiceExt;

const WS: &str = "ws-http";

struct FixedRoles(WorkspaceRole);

impl RoleChecker for FixedRoles {
    fn check<'a>(
        &'a self,
        _user: &'a User,
        _workspace_id: &'a Id,
        min: WorkspaceRole,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if self.0 >= min {
                Ok(())
            } else {
                Err(Error::Forbidden("insufficient test role".into()))
            }
        })
    }
}

#[derive(Clone)]
struct TestCtx {
    vault: Arc<VaultEngine>,
    roles: Arc<dyn RoleChecker>,
}

impl VaultCtx for TestCtx {
    fn vault(&self) -> &Arc<VaultEngine> {
        &self.vault
    }

    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

fn user() -> User {
    User {
        id: "user-http".into(),
        username: "http".into(),
        display_name: "HTTP Test".into(),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    }
}

async fn fixture(role: WorkspaceRole) -> (Router, tempfile::TempDir, i64) {
    let pool = otto_state::db::test_pool().await;
    let vault = Arc::new(VaultEngine::new(pool));
    let td = tempfile::tempdir().unwrap();
    let rec = vault
        .register(
            WS,
            "HTTP Vault",
            Some(td.path().to_string_lossy().into_owned()),
            false,
        )
        .await
        .unwrap();
    vault.scan(rec.id).await.unwrap();
    let ctx = TestCtx {
        vault,
        roles: Arc::new(FixedRoles(role)),
    };
    let app = otto_vault::router::<TestCtx>()
        .with_state(ctx)
        .layer(Extension(AuthUser(user())));
    (app, td, rec.id)
}

async fn put_file(app: &Router, id: i64, body: Value) -> (StatusCode, Value) {
    let encoded = serde_json::to_vec(&body).unwrap();
    let req = Request::builder()
        .method(Method::PUT)
        .uri(format!("/workspaces/{WS}/vault/vaults/{id}/file"))
        .header("content-type", "application/json")
        .body(Body::from(encoded))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({ "raw": String::from_utf8_lossy(&bytes) }));
    (status, body)
}

#[tokio::test]
async fn text_file_route_accepts_decoded_content_through_four_mib_and_rejects_next_byte() {
    let (app, td, id) = fixture(WorkspaceRole::Editor).await;

    for (name, size) in [
        ("over-two.json", 2 * 1024 * 1024 + 1),
        ("exactly-four.json", 4 * 1024 * 1024),
    ] {
        let content = "x".repeat(size);
        let request = json!({ "path": name, "content": content, "if_hash": "" });
        assert!(
            serde_json::to_vec(&request).unwrap().len() > size,
            "test must include JSON overhead"
        );
        let (status, response) = put_file(&app, id, request).await;
        assert_eq!(status, StatusCode::OK, "response: {response}");
        assert_eq!(response["path"], name);
        assert_eq!(response["size"], size);
        assert!(response["hash"].as_str().is_some_and(|h| !h.is_empty()));
        assert_eq!(
            std::fs::metadata(td.path().join(name)).unwrap().len(),
            size as u64
        );
    }

    let size = 4 * 1024 * 1024;
    let escaped = json!({
        "path": "escaped-exactly-four.json",
        "content": "\0".repeat(size),
        "if_hash": "",
    });
    assert!(
        serde_json::to_vec(&escaped).unwrap().len() >= size * 6,
        "test must exercise worst-case JSON string escaping"
    );
    let (status, response) = put_file(&app, id, escaped).await;
    assert_eq!(status, StatusCode::OK, "response: {response}");
    assert_eq!(response["size"], size);
    assert_eq!(
        std::fs::metadata(td.path().join("escaped-exactly-four.json"))
            .unwrap()
            .len(),
        size as u64
    );

    let size = 4 * 1024 * 1024 + 1;
    let request = json!({ "path": "too-large.json", "content": "x".repeat(size) });
    assert!(
        serde_json::to_vec(&request).unwrap().len() > size,
        "test must include JSON overhead"
    );
    let (status, response) = put_file(&app, id, request).await;
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "response: {response}"
    );
    assert_eq!(response["code"], "payload_too_large");
    assert!(!td.path().join("too-large.json").exists());
}

#[tokio::test]
async fn text_file_route_requires_editor_role() {
    let (app, td, id) = fixture(WorkspaceRole::Viewer).await;
    let (status, response) =
        put_file(&app, id, json!({ "path": "denied.json", "content": "{}" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "response: {response}");
    assert!(!td.path().join("denied.json").exists());
}
