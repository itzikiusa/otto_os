//! E2E: a team sharing ONE memory across separate Otto instances. A host runs
//! the real memory router; two "member" Ottos use a remote-backed MemoryService
//! pointed at it. Proves: a write on one member is recalled by another (shared
//! memory), and `private` memories stay private across the wire.

use std::sync::Arc;

use axum::extract::Request;
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use axum::Router;

use otto_core::auth::{AuthUser, BoxFuture, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::Id;

use otto_memory::{MemoryCtx, MemoryQuery, MemoryService, NewMemory, Scope, SearchMode};

// --- a minimal host: the real memory router + an auth shim that turns the
// bearer token into a per-user identity (so visibility is enforced per member).

#[derive(Clone)]
struct HostCtx {
    mem: Arc<MemoryService>,
    roles: Arc<dyn RoleChecker>,
}
impl MemoryCtx for HostCtx {
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

/// Map `Authorization: Bearer <token>` → an `AuthUser` whose id is the token, so
/// different members (tokens) are distinct users for visibility checks.
async fn auth_shim(mut req: Request, next: Next) -> Response {
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("anon")
        .to_string();
    let user = User {
        id: token.clone(),
        username: token.clone(),
        display_name: token,
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    };
    req.extensions_mut().insert(AuthUser(user));
    next.run(req).await
}

fn nm(vis: &str, body: &str) -> NewMemory {
    NewMemory {
        collection: "product".into(),
        record_type: "item".into(),
        visibility: vis.into(),
        scope: Scope::Workspace,
        story_id: None,
        kind: "fact".into(),
        title: "note".into(),
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

#[tokio::test]
async fn team_shares_one_memory_across_instances() {
    // The shared host owns the SQLite + the memory service.
    let (pool, ws, _seed) = otto_memory::test_support::mem_pool().await;
    let host_pool = pool.clone();
    let host = HostCtx {
        mem: Arc::new(MemoryService::with_defaults(pool)),
        roles: Arc::new(AllowRoles),
    };
    // The real daemon mounts module routers under /api/v1; mirror that so the
    // remote client's paths resolve.
    let app: Router = Router::new()
        .nest("/api/v1", otto_memory::router::<HostCtx>())
        .layer(from_fn(auth_shim))
        .with_state(host);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");

    // Two member Ottos, each a remote-backed service authenticating as itself.
    // (The pool here is only a local handle; all ops go to the host.)
    let alice = MemoryService::remote(host_pool.clone(), base.clone(), "alice".into());
    let bob = MemoryService::remote(host_pool.clone(), base.clone(), "bob".into());

    // Alice (on her machine) writes a shared fact + a private note.
    alice
        .save(&ws, "ignored", vec![nm("shared", "settlement runs nightly at 02:00 UTC")])
        .await
        .unwrap();
    alice
        .save(&ws, "ignored", vec![nm("private", "alice private todo about settlement")])
        .await
        .unwrap();

    // Bob (a different machine) recalls the SHARED fact — that's shared memory.
    let bob_hits = bob
        .search(
            &ws,
            MemoryQuery {
                text: Some("settlement nightly".into()),
                mode: SearchMode::Hybrid,
                k: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        bob_hits.iter().any(|h| h.memory.body.contains("nightly at 02:00")),
        "bob should see alice's SHARED memory across instances"
    );
    assert!(
        !bob_hits.iter().any(|h| h.memory.body.contains("private todo")),
        "bob must NOT see alice's private memory"
    );

    // The host's own local view also has the data (single source of truth).
    let host_svc = MemoryService::with_defaults(host_pool);
    let all = host_svc
        .search(
            &ws,
            MemoryQuery {
                text: Some("settlement".into()),
                mode: SearchMode::Keyword,
                k: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(all.len() >= 2, "host holds both memories (the single shared store)");
}
