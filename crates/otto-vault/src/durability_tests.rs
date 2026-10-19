//! E10: deterministic recovery and concurrency regressions (no native watcher needed).
use super::*;

async fn fixture() -> (Arc<VaultEngine>, tempfile::TempDir, i64) {
    let e = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.md"), "# A\noriginalneedle").unwrap();
    let id = e
        .store
        .create_vault("ws", "Test", dir.path().to_str().unwrap(), true)
        .await
        .unwrap();
    e.scan(id).await.unwrap();
    (e, dir, id)
}

#[tokio::test]
async fn generated_indexes_keep_recoverable_before_images() {
    let (e, dir, id) = fixture().await;
    let original = "---\nokf_version: '0.2'\n---\n# Handwritten\nUnique author explanation\n";
    std::fs::write(dir.path().join("index.md"), original).unwrap();
    e.scan(id).await.unwrap();
    e.okf_indexes("ws", id).await.unwrap();
    let revisions = e.revisions("ws", id, Some("index.md")).await.unwrap();
    assert_eq!(revisions.len(), 1);
    let revision = e.revision("ws", id, &revisions[0].id).await.unwrap();
    assert_eq!(revision.before.as_deref(), Some(original));
    let current = e.note("ws", id, "index.md").await.unwrap();
    e.restore_revision("ws", id, &revisions[0].id, "before", &current.meta.hash)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("index.md")).unwrap(),
        original
    );
}

#[tokio::test]
async fn empty_directories_survive_listing_rescan_and_engine_restart() {
    let (e, dir, id) = fixture().await;
    e.create_folder("ws", id, "empty/nested").await.unwrap();
    let root = e.dir("ws", id, "").await.unwrap();
    assert!(root
        .entries
        .iter()
        .any(|r| r.path == "empty" && r.kind == "dir"));
    e.scan(id).await.unwrap();
    let restarted = Arc::new(VaultEngine::new(e.store.pool().clone()));
    assert!(restarted
        .dir("ws", id, "empty")
        .await
        .unwrap()
        .entries
        .iter()
        .any(|r| r.path == "empty/nested"));
    e.write_note("ws", id, "empty/last.md", "last", Some(""))
        .await
        .unwrap();
    e.delete_note("ws", id, "empty/last.md").await.unwrap();
    assert!(e
        .dir("ws", id, "")
        .await
        .unwrap()
        .entries
        .iter()
        .any(|r| r.path == "empty"));
    std::fs::create_dir(dir.path().join("external")).unwrap();
    assert!(e
        .dir("ws", id, "")
        .await
        .unwrap()
        .entries
        .iter()
        .any(|r| r.path == "external"));
    std::fs::remove_dir(dir.path().join("external")).unwrap();
    assert!(!e
        .dir("ws", id, "")
        .await
        .unwrap()
        .entries
        .iter()
        .any(|r| r.path == "external"));
}

#[tokio::test]
async fn scan_kick_during_walk_is_drained_without_another_event() {
    let (e, dir, id) = fixture().await;
    let (started, start) = tokio::sync::oneshot::channel();
    let (resume, resumed) = tokio::sync::oneshot::channel();
    *e.scan_pause.lock().unwrap() = Some((started, resumed));
    e.kick_scan(id);
    start.await.unwrap(); // snapshot already walked; new.md cannot be in this pass
    std::fs::write(dir.path().join("new.md"), "# New\nlatearrival").unwrap();
    e.kick_scan(id); // the same call the watcher makes after consuming its event
    resume.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if e.store.note_meta(id, "new.md").await.is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("a pending kick must cause another pass");
}

#[tokio::test]
async fn restart_backfills_missing_fts_without_changing_files() {
    let (e, _dir, id) = fixture().await;
    // Simulate rows committed by a prior fallback-only process.
    sqlx::query("DELETE FROM vault_fts")
        .execute(e.store.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM vault_fts_ids")
        .execute(e.store.pool())
        .await
        .unwrap();
    let restarted = Arc::new(VaultEngine::new(e.store.pool().clone()));
    restarted.scan(id).await.unwrap();
    let hits = restarted
        .search(
            "ws",
            id,
            &SearchReq {
                query: "originalneedle".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "unchanged body must be backfilled");
    assert_eq!(hits[0].path, "a.md");
}

#[tokio::test]
async fn generated_indexes_refuse_edits_after_their_snapshot() {
    let (e, dir, id) = fixture().await;
    std::fs::write(dir.path().join("index.md"), "# Original index").unwrap();
    let (started, start) = tokio::sync::oneshot::channel();
    let (resume, resumed) = tokio::sync::oneshot::channel();
    *e.okf_pause.lock().unwrap() = Some((started, resumed));
    let generate = {
        let e = e.clone();
        tokio::spawn(async move { e.okf_indexes("ws", id).await })
    };
    start.await.unwrap();
    e.write_note("ws", id, "index.md", "# Concurrent author edit", None)
        .await
        .unwrap();
    resume.send(()).unwrap();
    assert!(matches!(generate.await.unwrap(), Err(Error::Conflict(_))));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("index.md")).unwrap(),
        "# Concurrent author edit"
    );
}

#[tokio::test]
async fn generated_indexes_leave_original_untouched_when_history_is_unwritable() {
    let (e, dir, id) = fixture().await;
    std::fs::write(dir.path().join("index.md"), "# Keep this original").unwrap();
    // Deterministic filesystem failure, including when tests run as root.
    std::fs::write(dir.path().join(".otto-history"), "not a directory").unwrap();
    assert!(e.okf_indexes("ws", id).await.is_err());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("index.md")).unwrap(),
        "# Keep this original"
    );
}

#[tokio::test]
async fn transient_fts_failure_recovers_both_new_and_changed_bodies() {
    let (e, _dir, id) = fixture().await;
    e.fts_ok.store(0, Ordering::Relaxed);
    e.store.fts_fail_once.store(true, Ordering::Relaxed);
    e.write_note("ws", id, "a.md", "# A\nchangedneedle", None)
        .await
        .unwrap();
    e.write_note("ws", id, "new.md", "# New\nnewneedle", None)
        .await
        .unwrap();
    assert_eq!(
        e.fts_ok.load(Ordering::Relaxed),
        0,
        "transient failure stays retryable"
    );
    // Advance the retry clock without sleeping or depending on wall time.
    *e.fts_failed_at.lock().unwrap() = None;
    e.scan(id).await.unwrap();
    for (query, path) in [("changedneedle", "a.md"), ("newneedle", "new.md")] {
        let hits = e
            .search(
                "ws",
                id,
                &SearchReq {
                    query: query.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, path);
    }
    assert!(e
        .search(
            "ws",
            id,
            &SearchReq {
                query: "originalneedle".into(),
                ..Default::default()
            }
        )
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn unsupported_fts_does_not_force_reparsing_unchanged_notes() {
    let (e, _dir, id) = fixture().await;
    e.fts_ok.store(2, Ordering::Relaxed);
    e.write_note("ws", id, "a.md", "# A\nfallback only", None)
        .await
        .unwrap();
    let commits = e.store.index_commits.load(Ordering::Relaxed);
    e.scan(id).await.unwrap();
    e.scan(id).await.unwrap();
    assert_eq!(e.store.index_commits.load(Ordering::Relaxed), commits);
}

#[tokio::test]
async fn cross_workspace_registration_serializes_check_and_insert() {
    let e = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    let (started, start) = tokio::sync::oneshot::channel();
    let (resume, resumed) = tokio::sync::oneshot::channel();
    *e.store.registration_pause.lock().unwrap() = Some((started, resumed));
    let first = {
        let (e, root) = (e.clone(), root.clone());
        tokio::spawn(async move { e.store.create_vault("one", "First", &root, false).await })
    };
    start.await.unwrap(); // first duplicate lookup has completed, insert is held
    let mut second = {
        let (e, root) = (e.clone(), root.clone());
        tokio::spawn(async move { e.store.create_vault("two", "Second", &root, false).await })
    };
    // Old code allowed the second INSERT while the first was paused. The
    // transaction must prevent that; timeout only observes lock exclusion.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut second)
            .await
            .is_err()
    );
    resume.send(()).unwrap();
    first.await.unwrap().unwrap();
    assert!(matches!(second.await.unwrap(), Err(Error::Conflict(_))));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM vaults WHERE root_path=?")
        .bind(&root)
        .fetch_one(e.store.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn legacy_duplicate_roots_share_compare_and_set_lock() {
    let (e, dir, id) = fixture().await;
    let other = sqlx::query("INSERT INTO vaults(ws_id,name,root_path,okf,created_at) VALUES('other','Legacy',?,0,'2026-10-18')")
        .bind(dir.path().to_str().unwrap()).execute(e.store.pool()).await.unwrap().last_insert_rowid();
    e.scan(other).await.unwrap();
    let initial = e.note("ws", id, "a.md").await.unwrap();
    let (a, b) = tokio::join!(
        e.write_note("ws", id, "a.md", "first", Some(&initial.meta.hash)),
        e.write_note("other", other, "a.md", "second", Some(&initial.meta.hash))
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(a, Err(Error::Conflict(_))) || matches!(b, Err(Error::Conflict(_))));
}
