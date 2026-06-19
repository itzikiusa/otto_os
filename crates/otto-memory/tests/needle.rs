//! E2E: prove the memory layer (a) finds a needle in a haystack without
//! returning everything, (b) works over the HTTP route (accessible), and
//! (c) the semantic/vector path works.

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt; // oneshot

use otto_core::auth::{AuthUser, BoxFuture, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::Id;

use otto_memory::{MemoryCtx, MemoryHit, MemoryQuery, MemoryService, NewMemory, Scope, SearchMode};

fn nm(kind: &str, title: &str, body: &str) -> NewMemory {
    NewMemory {
        collection: "product".into(),
        record_type: "item".into(),
        scope: Scope::Workspace,
        story_id: None,
        kind: kind.into(),
        title: title.into(),
        body: body.into(),
        entities: vec![],
        tags: vec![],
        source_kind: "manual".into(),
        source_ref: None,
        refs: vec![],
        confidence: None,
        salience: None,
    }
}

async fn haystack(svc: &MemoryService, ws: &str, user: &str, n: usize) {
    let mut batch = Vec::new();
    for i in 0..n {
        batch.push(nm(
            "fact",
            &format!("Service note {i}"),
            &format!("Routine operational detail number {i} about request logging, metrics, and dashboards."),
        ));
    }
    svc.save(ws, user, batch).await.unwrap();
}

#[tokio::test]
async fn needle_in_haystack_via_service() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);

    haystack(&svc, &ws, &user, 200).await;
    // The needle — a distinctive constraint buried among 200 distractors.
    svc.save(
        &ws,
        &user,
        vec![nm(
            "constraint",
            "Withdrawal limit",
            "Withdrawals over 2000 USD require manual compliance review and two-factor authentication.",
        )],
    )
    .await
    .unwrap();

    let hits = svc
        .search(
            &ws,
            MemoryQuery {
                text: Some("withdrawal manual compliance review".into()),
                mode: SearchMode::Hybrid,
                k: 5,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert!(!hits.is_empty(), "expected hits");
    assert!(
        hits[0].memory.body.contains("manual compliance review"),
        "needle should rank first, got: {:?}",
        hits[0].memory.title
    );
    // Did NOT return the whole haystack — recall is selective.
    assert!(hits.len() <= 5, "returned {} (should be <= k)", hits.len());
}

#[tokio::test]
async fn semantic_path_finds_related() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);

    svc.save(
        &ws,
        &user,
        vec![nm(
            "fact",
            "Auth policy",
            "two factor authentication is mandatory for all withdrawals",
        )],
    )
    .await
    .unwrap();
    haystack(&svc, &ws, &user, 50).await;

    let hits = svc
        .search(
            &ws,
            MemoryQuery {
                text: Some("two factor authentication mandatory".into()),
                mode: SearchMode::Semantic,
                k: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert!(
        hits.iter().any(|h| h.memory.body.contains("two factor authentication")),
        "semantic search should surface the auth memory"
    );
}

#[tokio::test]
async fn exact_duplicate_is_noop() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let a = svc.save(&ws, &user, vec![nm("fact", "X", "the same body text")]).await.unwrap();
    let b = svc.save(&ws, &user, vec![nm("fact", "X", "the same body text")]).await.unwrap();
    assert_eq!(a[0].id, b[0].id, "duplicate save should return the existing row");
}

// --- HTTP accessibility (router oneshot) ---

#[derive(Clone)]
struct TestCtx {
    mem: Arc<MemoryService>,
    roles: Arc<dyn RoleChecker>,
}
impl MemoryCtx for TestCtx {
    fn memory(&self) -> &Arc<MemoryService> {
        &self.mem
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

struct AllowRoles;
impl RoleChecker for AllowRoles {
    fn check<'a>(
        &'a self,
        _u: &'a User,
        _ws: &'a Id,
        _m: WorkspaceRole,
    ) -> BoxFuture<'a, otto_core::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

fn test_user(id: &str) -> User {
    User {
        id: id.into(),
        username: "tester".into(),
        display_name: "Tester".into(),
        is_root: true,
        disabled: false,
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn needle_in_haystack_over_http() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    haystack(&svc, &ws, &user, 150).await;

    let ctx = TestCtx {
        mem: Arc::new(svc),
        roles: Arc::new(AllowRoles),
    };
    let app = otto_memory::router::<TestCtx>().with_state(ctx);
    let u = test_user(&user);

    // 1) Write the needle via the HTTP route.
    let create_body = serde_json::to_string(&nm(
        "constraint",
        "KYC threshold",
        "Deposits above 10000 EUR trigger an enhanced KYC verification workflow before crediting.",
    ))
    .unwrap();
    let create_req = Request::builder()
        .method("POST")
        .uri(format!("/workspaces/{ws}/memories"))
        .header("content-type", "application/json")
        .extension(AuthUser(u.clone()))
        .body(Body::from(create_body))
        .unwrap();
    let create_resp = app.clone().oneshot(create_req).await.unwrap();
    assert_eq!(create_resp.status(), 200, "create should succeed over HTTP");

    // 2) Find the needle via the HTTP search route.
    let q = MemoryQuery {
        text: Some("enhanced KYC verification deposit".into()),
        mode: SearchMode::Hybrid,
        k: 5,
        ..Default::default()
    };
    let search_req = Request::builder()
        .method("POST")
        .uri(format!("/workspaces/{ws}/memory/search"))
        .header("content-type", "application/json")
        .extension(AuthUser(u))
        .body(Body::from(serde_json::to_string(&q).unwrap()))
        .unwrap();
    let search_resp = app.oneshot(search_req).await.unwrap();
    assert_eq!(search_resp.status(), 200, "search should succeed over HTTP");

    let bytes = axum::body::to_bytes(search_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let hits: Vec<MemoryHit> = serde_json::from_slice(&bytes).unwrap();
    assert!(!hits.is_empty(), "HTTP search returned no hits");
    assert!(
        hits[0].memory.body.contains("enhanced KYC"),
        "needle should rank first over HTTP, got {:?}",
        hits[0].memory.title
    );
    assert!(hits.len() <= 5);
}
