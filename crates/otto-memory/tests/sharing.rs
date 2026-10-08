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

    svc.save(
        &ws,
        "alice",
        vec![mk("shared", "shared settlement runbook")],
    )
    .await
    .unwrap();
    svc.save(
        &ws,
        "alice",
        vec![mk("private", "alice secret settlement note")],
    )
    .await
    .unwrap();

    // Bob (another team member) sees the shared one, not Alice's private one.
    let bob = svc.search(&ws, query("bob")).await.unwrap();
    assert!(bob
        .iter()
        .any(|h| h.memory.body.contains("shared settlement runbook")));
    assert!(
        !bob.iter().any(|h| h.memory.body.contains("secret")),
        "bob must not see alice's private memory"
    );

    // Alice sees both (her own private + the shared one).
    let alice = svc.search(&ws, query("alice")).await.unwrap();
    assert!(
        alice.iter().any(|h| h.memory.body.contains("secret")),
        "alice sees her own private"
    );
    assert!(alice
        .iter()
        .any(|h| h.memory.body.contains("shared settlement runbook")));
}

#[tokio::test]
async fn dedup_never_returns_or_reactivates_another_users_private_row() {
    let (pool, ws, _) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let row = svc
        .save(&ws, "alice", vec![mk("private", "same known content")])
        .await
        .unwrap()
        .remove(0);
    assert!(svc
        .save(&ws, "bob", vec![mk("shared", "same known content")])
        .await
        .is_err());
    svc.soft_forget(&ws, &row.id).await.unwrap();
    assert!(svc
        .save(&ws, "bob", vec![mk("private", "same known content")])
        .await
        .is_err());
    assert!(!svc.get(&ws, &row.id).await.unwrap().active);
    assert_eq!(
        svc.save(&ws, "alice", vec![mk("private", "same known content")])
            .await
            .unwrap()[0]
            .id,
        row.id
    );
}

#[tokio::test]
async fn malformed_visibility_is_rejected_instead_of_becoming_shared() {
    let (pool, ws, _) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    assert!(svc
        .save(&ws, "alice", vec![mk("Private", "private-intended note")])
        .await
        .is_err());
}

#[tokio::test]
async fn merging_shared_and_private_sources_keeps_result_private() {
    let (pool, ws, _) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let shared = svc
        .save(&ws, "alice", vec![mk("shared", "public source")])
        .await
        .unwrap()
        .remove(0);
    let private = svc
        .save(&ws, "alice", vec![mk("private", "private source")])
        .await
        .unwrap()
        .remove(0);
    let merged = svc
        .merge(
            &ws,
            "alice",
            otto_memory::governance::MergeReq {
                ids: vec![shared.id, private.id],
                title: "Merged".into(),
                body: "combined sources".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(merged.visibility, "private");
    assert!(svc.get_visible(&ws, &merged.id, Some("bob")).await.is_err());
    let parts = svc
        .split(
            &ws,
            "alice",
            &merged.id,
            otto_memory::governance::SplitReq {
                parts: vec![
                    otto_memory::governance::SplitPart {
                        title: "One".into(),
                        body: "first separate piece".into(),
                    },
                    otto_memory::governance::SplitPart {
                        title: "Two".into(),
                        body: "second separate piece".into(),
                    },
                ],
            },
        )
        .await
        .unwrap();
    assert!(parts
        .memories
        .iter()
        .all(|memory| memory.visibility == "private"));
}
