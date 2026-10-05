//! Persistence integrity of the memory store (r10-state): FTS written in the
//! memory's own transaction and reconciled on start, duplicates of inactive
//! rows revived instead of 500ing, edits colliding with the dedup index
//! surfacing as 409, and governance re-imports leaving live memories alone.

use otto_core::Error;
use otto_memory::{ImportReq, MemoryPatch, MemoryQuery, MemoryService, NewMemory, Scope};

fn nm(title: &str, body: &str) -> NewMemory {
    NewMemory {
        collection: "product".into(),
        record_type: "item".into(),
        visibility: "shared".into(),
        scope: Scope::Workspace,
        story_id: None,
        kind: "fact".into(),
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

async fn search_ids(svc: &MemoryService, ws: &str, text: &str) -> Vec<String> {
    svc.search(
        ws,
        MemoryQuery {
            text: Some(text.into()),
            k: 10,
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .into_iter()
    .map(|h| h.memory.id)
    .collect()
}

async fn fts_mids(svc: &MemoryService) -> Vec<String> {
    sqlx::query_scalar("SELECT mid FROM memories_fts ORDER BY mid")
        .fetch_all(svc.pool())
        .await
        .unwrap()
}

/// Re-saving content identical to a forgotten memory used to hit the dedup
/// unique index (it covers inactive rows) with a plain INSERT → 500, and a
/// batch import stopped half-applied. Now the row is revived.
#[tokio::test]
async fn resaving_a_forgotten_memory_reactivates_it() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let a = svc
        .save(&ws, &user, vec![nm("A", "the deploy needs cmake")])
        .await
        .unwrap()
        .remove(0);
    svc.soft_forget(&ws, &a.id).await.unwrap();
    let batch = svc
        .save(
            &ws,
            &user,
            vec![nm("A", "the deploy needs cmake"), nm("B", "fresh second fact")],
        )
        .await
        .expect("a duplicate of a forgotten memory must not fail the batch");
    assert_eq!(batch.len(), 2, "the whole batch applied");
    assert_eq!(batch[0].id, a.id, "the forgotten row is revived, not cloned");
    assert!(batch[0].active);
    assert_eq!(batch[0].state, "accepted");
    assert!(batch[0].undo_token.is_none());
    assert_eq!(search_ids(&svc, &ws, "cmake").await, vec![a.id.clone()]);
}

/// An edit whose new body duplicates another memory is a 409, not a 500.
#[tokio::test]
async fn update_into_a_duplicate_body_is_a_conflict() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let saved = svc
        .save(&ws, &user, vec![nm("A", "alpha body"), nm("B", "beta body")])
        .await
        .unwrap();
    let err = svc
        .update(
            &ws,
            &saved[1].id,
            MemoryPatch {
                body: Some("Alpha   BODY".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "got {err:?}");
    // The failed edit rolled back whole: B is unchanged and still indexed.
    assert_eq!(svc.get(&ws, &saved[1].id).await.unwrap().body, "beta body");
    assert_eq!(search_ids(&svc, &ws, "beta").await, vec![saved[1].id.clone()]);
}

/// The FTS row is written in the memory's own transaction, and drift left by
/// older builds (an unindexed row, an orphan, stale text) is repaired by the
/// reconcile that runs when FTS is first readied.
#[tokio::test]
async fn fts_is_written_with_the_row_and_drift_is_reconciled_on_start() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool.clone());
    let saved = svc
        .save(
            &ws,
            &user,
            vec![nm("A", "kafka offsets reset"), nm("B", "redis eviction")],
        )
        .await
        .unwrap();
    let mut ids: Vec<String> = saved.iter().map(|m| m.id.clone()).collect();
    ids.sort();
    assert_eq!(fts_mids(&svc).await, ids, "indexed on write");

    // Simulate drift: drop A's index entry, orphan a ghost, stale B's text.
    let a = &saved[0].id;
    let b = &saved[1].id;
    sqlx::query("DELETE FROM memories_fts WHERE mid = ?")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM memories_fts_ids WHERE mid = ?")
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    svc.repo()
        .fts_index("ghost", &ws, "G", "ghost text")
        .await
        .unwrap();
    sqlx::query("UPDATE memories_fts SET body = 'stale' WHERE mid = ?")
        .bind(b)
        .execute(&pool)
        .await
        .unwrap();

    // A fresh service (daemon restart) reconciles when FTS is first readied.
    let svc = MemoryService::with_defaults(pool.clone());
    assert_eq!(search_ids(&svc, &ws, "kafka").await, vec![a.clone()]);
    assert_eq!(search_ids(&svc, &ws, "eviction").await, vec![b.clone()]);
    assert_eq!(fts_mids(&svc).await, ids, "orphan dropped, missing row back");
    let map: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories_fts_ids")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(map, 2, "map mirrors the index");
    let body: String = sqlx::query_scalar("SELECT body FROM memories_fts WHERE mid = ?")
        .bind(b)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(body, "redis eviction", "stale text rewritten");
    assert_eq!(svc.repo().reconcile_fts().await.unwrap(), 0, "idempotent");
}

/// Re-importing the same governance file must not demote memories a person
/// already accepted back to `suggested`, nor count them as imported.
#[tokio::test]
async fn reimporting_a_governance_file_keeps_accepted_memories() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let md = "## Build\n\ncargo build --workspace\n\n## Lint\n\ncargo clippy\n";
    let req = || ImportReq {
        kind: "agents-md".into(),
        content: md.into(),
        label: None,
    };
    let first = svc.import_governed(&ws, &user, req()).await.unwrap();
    assert_eq!(first.imported, 2);
    let all = svc.list(&ws, Default::default()).await.unwrap();
    assert!(all.iter().all(|m| m.state == "suggested"));
    for m in &all {
        svc.set_state(&ws, &m.id, "accepted").await.unwrap();
    }

    let again = svc.import_governed(&ws, &user, req()).await.unwrap();
    assert_eq!(again.imported, 0, "nothing new was imported");
    let all = svc.list(&ws, Default::default()).await.unwrap();
    assert_eq!(all.len(), 2);
    assert!(
        all.iter().all(|m| m.state == "accepted"),
        "accepted memories stay accepted: {:?}",
        all.iter().map(|m| &m.state).collect::<Vec<_>>()
    );
}

/// Concurrent saves on a real (file, split-pool) database: each save reads
/// (dedup lookup, FTS map probe) and then writes. Under DEFERRED transactions
/// the read→write upgrade failed with SQLITE_BUSY whenever another writer
/// committed in between; `BEGIN IMMEDIATE` waits in the busy handler instead.
#[tokio::test]
async fn concurrent_saves_on_a_file_database_all_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::db::open(&dir.path().join("otto.db"))
        .await
        .unwrap();
    let ws = otto_core::new_id();
    let user = otto_core::new_id();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at) \
         VALUES (?, ?, 'x', 'T', 0, 0, '2026-01-01T00:00:00Z')",
    )
    .bind(&user)
    .bind(format!("u_{user}"))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) \
         VALUES (?, 'W', '/tmp/w', '2026-01-01T00:00:00Z')",
    )
    .bind(&ws)
    .execute(&pool)
    .await
    .unwrap();
    let svc = std::sync::Arc::new(MemoryService::with_defaults(pool));
    let mut tasks = Vec::new();
    for t in 0..4 {
        let (svc, ws, user) = (svc.clone(), ws.clone(), user.clone());
        tasks.push(tokio::spawn(async move {
            for i in 0..15 {
                svc.save(
                    &ws,
                    &user,
                    vec![nm("T", &format!("writer {t} fact number {i}"))],
                )
                .await?;
            }
            Ok::<_, Error>(())
        }));
    }
    for t in tasks {
        t.await.unwrap().expect("no SQLITE_BUSY under concurrent saves");
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories_fts_ids")
        .fetch_one(svc.pool())
        .await
        .unwrap();
    assert_eq!(n, 60, "every save indexed in its own transaction");
}
