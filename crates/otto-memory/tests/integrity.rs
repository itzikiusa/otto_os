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
            vec![
                nm("A", "the deploy needs cmake"),
                nm("B", "fresh second fact"),
            ],
        )
        .await
        .expect("a duplicate of a forgotten memory must not fail the batch");
    assert_eq!(batch.len(), 2, "the whole batch applied");
    assert_eq!(
        batch[0].id, a.id,
        "the forgotten row is revived, not cloned"
    );
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
        .save(
            &ws,
            &user,
            vec![nm("A", "alpha body"), nm("B", "beta body")],
        )
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
    assert_eq!(
        search_ids(&svc, &ws, "beta").await,
        vec![saved[1].id.clone()]
    );
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
    assert_eq!(
        fts_mids(&svc).await,
        ids,
        "orphan dropped, missing row back"
    );
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
        t.await
            .unwrap()
            .expect("no SQLITE_BUSY under concurrent saves");
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories_fts_ids")
        .fetch_one(svc.pool())
        .await
        .unwrap();
    assert_eq!(n, 60, "every save indexed in its own transaction");
}

/// S7-02: merged text equal to a source deduplicates onto that source. It
/// used to be superseded by ITSELF (active=0, superseded_by=self) — the merged
/// knowledge vanished from search. Now it survives as the merged row and only
/// the other sources are retired.
#[tokio::test]
async fn merge_whose_text_equals_a_source_keeps_that_source_live() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let saved = svc
        .save(
            &ws,
            &user,
            vec![
                nm("A", "release trains leave on tuesday"),
                nm("B", "release trains leave tuesdays"),
            ],
        )
        .await
        .unwrap();
    let (a, b) = (saved[0].id.clone(), saved[1].id.clone());
    let merged = svc
        .merge(
            &ws,
            &user,
            otto_memory::MergeReq {
                // The repeated id is one source, not two.
                ids: vec![a.clone(), b.clone(), a.clone()],
                title: "Release trains".into(),
                body: "release trains leave on tuesday".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(merged.id, a, "deduplicated onto source A");
    let a_row = svc.get(&ws, &a).await.unwrap();
    assert!(a_row.active, "the merged row stays live");
    assert_ne!(a_row.superseded_by.as_deref(), Some(a.as_str()));
    let b_row = svc.get(&ws, &b).await.unwrap();
    assert!(!b_row.active);
    assert_eq!(b_row.superseded_by.as_deref(), Some(a.as_str()));
    let hits = search_ids(&svc, &ws, "release trains").await;
    assert!(
        hits.contains(&a),
        "merged knowledge is searchable: {hits:?}"
    );
    assert!(!hits.contains(&b), "retired source is not: {hits:?}");
}

/// S7-02: a split part equal to the parent (or to another part) used to
/// resolve to the parent / one shared child — the parent then superseded
/// itself. Both are refused before anything is written.
#[tokio::test]
async fn split_refuses_parts_equal_to_the_parent_or_each_other() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let parent = svc
        .save(&ws, &user, vec![nm("P", "deploys need cmake and node")])
        .await
        .unwrap()
        .remove(0);
    let part = |t: &str, b: &str| otto_memory::SplitPart {
        title: t.into(),
        body: b.into(),
    };
    for parts in [
        vec![
            part("1", "deploys need cmake and node"),
            part("2", "deploys need node"),
        ],
        vec![
            part("1", "deploys need cmake"),
            part("2", "deploys need cmake"),
        ],
    ] {
        let err = svc
            .split(&ws, &user, &parent.id, otto_memory::SplitReq { parts })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    }
    let p = svc.get(&ws, &parent.id).await.unwrap();
    assert!(p.active, "a refused split leaves the parent untouched");
    assert!(p.superseded_by.is_none());

    let ok = svc
        .split(
            &ws,
            &user,
            &parent.id,
            otto_memory::SplitReq {
                parts: vec![
                    part("1", "deploys need cmake"),
                    part("2", "deploys need node"),
                ],
            },
        )
        .await
        .unwrap();
    assert_eq!(ok.memories.len(), 2);
    let p = svc.get(&ws, &parent.id).await.unwrap();
    assert!(!p.active);
    assert_eq!(p.superseded_by.as_deref(), Some(ok.memories[0].id.as_str()));
}

/// S7-05: a search's access bump is ONE statement for the whole hit list;
/// every listed id counts once, other workspaces are untouched.
#[tokio::test]
async fn bump_access_counts_each_hit_once_in_one_statement() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let saved = svc
        .save(
            &ws,
            &user,
            vec![nm("A", "alpha fact"), nm("B", "beta fact")],
        )
        .await
        .unwrap();
    let (a, b) = (saved[0].id.clone(), saved[1].id.clone());
    let repo = svc.repo();
    repo.bump_access(&ws, &[a.clone(), b.clone(), a.clone(), "missing".into()])
        .await
        .unwrap();
    repo.bump_access("other-ws", std::slice::from_ref(&a))
        .await
        .unwrap();
    repo.bump_access(&ws, &[]).await.unwrap();
    assert_eq!(svc.get(&ws, &a).await.unwrap().access_count, 1);
    assert_eq!(svc.get(&ws, &b).await.unwrap().access_count, 1);
    assert!(svc.get(&ws, &a).await.unwrap().last_accessed_at.is_some());
}

/// S7-06: undo tokens are random (distinct, 256-bit hex) and expire with
/// the undo window measured from `forgotten_at`.
#[tokio::test]
async fn undo_tokens_are_random_and_expire() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let saved = svc
        .save(
            &ws,
            &user,
            vec![nm("A", "gamma fact"), nm("B", "delta fact")],
        )
        .await
        .unwrap();
    let t1 = svc.soft_forget(&ws, &saved[0].id).await.unwrap().undo_token;
    let t2 = svc.soft_forget(&ws, &saved[1].id).await.unwrap().undo_token;
    assert_ne!(t1, t2);
    assert!(t1.len() == 64 && t1.chars().all(|c| c.is_ascii_hexdigit()));
    // Within the window: restores.
    assert!(svc.undo_forget(&ws, &t1).await.unwrap().active);
    // Past the window: refused, the memory stays forgotten.
    sqlx::query("UPDATE memories SET forgotten_at = ? WHERE id = ?")
        .bind(chrono::Utc::now().timestamp() - otto_state::memory::UNDO_FORGET_WINDOW_SECS - 1)
        .bind(&saved[1].id)
        .execute(svc.pool())
        .await
        .unwrap();
    assert!(matches!(
        svc.undo_forget(&ws, &t2).await.unwrap_err(),
        Error::NotFound(_)
    ));
    assert!(!svc.get(&ws, &saved[1].id).await.unwrap().active);
}
