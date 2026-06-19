//! E2E: the Obsidian-compatible vault — write-through to markdown notes, and
//! re-indexing a (shared/git-synced) vault folder back into the store.

use otto_memory::vault::{parse_to_new, to_markdown};
use otto_memory::{MemoryQuery, MemoryService, NewMemory, Scope, SearchMode};

fn nm(body: &str) -> NewMemory {
    NewMemory {
        collection: "product".into(),
        record_type: "item".into(),
        visibility: "shared".into(),
        scope: Scope::Workspace,
        story_id: None,
        kind: "fact".into(),
        title: "Settlement window".into(),
        body: body.into(),
        entities: vec![],
        tags: vec!["payments".into(), "ops".into()],
        source_kind: "manual".into(),
        source_ref: None,
        refs: vec![],
        confidence: None,
        salience: None,
    }
}

#[tokio::test]
async fn save_writes_an_obsidian_note() {
    let dir = tempfile::tempdir().unwrap();
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool).with_vault(dir.path());

    let saved = svc
        .save(&ws, &user, vec![nm("settlement runs nightly at 02:00 UTC")])
        .await
        .unwrap();
    let m = &saved[0];

    // A markdown note exists under <root>/<ws>/ with frontmatter + body.
    let ws_dir = dir.path().join(&ws);
    let files: Vec<_> = std::fs::read_dir(&ws_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    assert_eq!(files.len(), 1, "one note written");
    let content = std::fs::read_to_string(&files[0]).unwrap();
    assert!(content.starts_with("---\n"), "has YAML frontmatter");
    assert!(content.contains(&format!("id: {}", m.id)));
    assert!(content.contains("visibility: shared"));
    assert!(content.contains("tags: [payments, ops]"));
    assert!(content.contains("settlement runs nightly"));
}

#[test]
fn markdown_round_trips() {
    // A note rendered then parsed preserves the meaningful fields.
    let body = "deposits over 10000 trigger enhanced KYC";
    let note = format!(
        "---\nid: x1\nkind: constraint\ncollection: product\nscope: story\nsource_kind: analysis\nvisibility: private\ntags: [kyc, compliance]\ntitle: KYC threshold\n---\n\n[[Compliance]] [[KYC]]\n\n{body}\n"
    );
    let nm = parse_to_new(&note).expect("parse");
    assert_eq!(nm.kind, "constraint");
    assert_eq!(nm.collection, "product");
    assert_eq!(nm.visibility, "private");
    assert_eq!(nm.scope, Scope::Story);
    assert_eq!(nm.tags, vec!["kyc".to_string(), "compliance".into()]);
    assert!(nm.body.contains("enhanced KYC"));
    assert!(!nm.body.contains("[[")); // wikilink line stripped from body
}

#[tokio::test]
async fn reindex_shared_vault_makes_it_searchable() {
    // Simulate a teammate's vault folder synced via git: hand-written notes.
    let dir = tempfile::tempdir().unwrap();
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);

    // Render two notes (as if authored elsewhere) and drop them in the folder.
    let m_dummy = svc
        .save(&ws, &user, vec![nm("a temporary in-db note")])
        .await
        .unwrap();
    let vault_dir = dir.path().join(&ws);
    std::fs::create_dir_all(&vault_dir).unwrap();
    let note = to_markdown(&m_dummy[0], &[]);
    std::fs::write(vault_dir.join("a.md"), note).unwrap();
    std::fs::write(
        vault_dir.join("b.md"),
        "---\nid: ext1\nkind: decision\ncollection: product\nscope: workspace\nsource_kind: manual\nvisibility: shared\ntags: []\ntitle: Webhook choice\n---\n\nuse webhooks not polling for settlement\n",
    )
    .unwrap();

    // A fresh instance re-indexes the synced vault.
    let (pool2, ws2, user2) = otto_memory::test_support::mem_pool().await;
    let svc2 = MemoryService::with_defaults(pool2);
    // Re-index under the same logical workspace id is fine; we use ws2 here.
    let n = svc2.reindex_vault(&ws2, &user2, &vault_dir).await.unwrap();
    assert!(n >= 2, "re-indexed both notes");

    let hits = svc2
        .search(
            &ws2,
            MemoryQuery {
                text: Some("webhooks settlement polling".into()),
                mode: SearchMode::Hybrid,
                k: 5,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        hits.iter().any(|h| h.memory.body.contains("use webhooks")),
        "re-indexed shared note should be searchable"
    );
}
