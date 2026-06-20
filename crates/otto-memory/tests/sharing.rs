//! E2E: `shared` vs `private` memory within a workspace/team.

use otto_memory::{MemoryQuery, MemoryService, NewMemory, Scope, SearchMode};

fn mk(vis: &str, body: &str) -> NewMemory {
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

fn query(viewer: &str) -> MemoryQuery {
    MemoryQuery {
        text: Some("settlement".into()),
        mode: SearchMode::Hybrid,
        k: 10,
        viewer: Some(viewer.into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn private_memory_is_hidden_from_other_members() {
    let (pool, ws, _seed) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);

    svc.save(&ws, "alice", vec![mk("shared", "shared settlement runbook")]).await.unwrap();
    svc.save(&ws, "alice", vec![mk("private", "alice secret settlement note")]).await.unwrap();

    // Bob (another team member) sees the shared one, not Alice's private one.
    let bob = svc.search(&ws, query("bob")).await.unwrap();
    assert!(bob.iter().any(|h| h.memory.body.contains("shared settlement runbook")));
    assert!(
        !bob.iter().any(|h| h.memory.body.contains("secret")),
        "bob must not see alice's private memory"
    );

    // Alice sees both (her own private + the shared one).
    let alice = svc.search(&ws, query("alice")).await.unwrap();
    assert!(alice.iter().any(|h| h.memory.body.contains("secret")), "alice sees her own private");
    assert!(alice.iter().any(|h| h.memory.body.contains("shared settlement runbook")));
}
