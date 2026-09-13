//! Performance regressions exercise the real engine against isolated files and SQLite.
use super::*;
use std::sync::atomic::Ordering::Relaxed;

async fn fixture() -> (Arc<VaultEngine>, tempfile::TempDir, i64) {
    let engine = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.md"), "# Before\n[[b]]").unwrap();
    std::fs::write(dir.path().join("b.md"), "# Target").unwrap();
    let id = engine
        .store
        .create_vault("ws", "Test", dir.path().to_str().unwrap(), false)
        .await
        .unwrap();
    engine.scan(id).await.unwrap();
    (engine, dir, id)
}

#[tokio::test]
async fn warm_save_does_not_scan_unrelated_files() {
    let (e, _dir, id) = fixture().await;
    e.dir("ws", id, "").await.unwrap();
    let walks = e.walks.load(Relaxed);
    let reads = e.store.all_reads.load(Relaxed);
    let links = e.store.link_reads.load(Relaxed);
    e.write_note(
        "ws",
        id,
        "a.md",
        "---\ntitle: After\naliases: [New Alias]\ntags: [new]\n---\n[[b]]",
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        e.walks.load(Relaxed),
        walks,
        "ordinary save must not walk unrelated files"
    );
    assert_eq!(
        e.store.all_reads.load(Relaxed),
        reads,
        "ordinary save must not reload all notes"
    );
    assert_eq!(
        e.store.link_reads.load(Relaxed),
        links,
        "content edits do not change resolver candidates"
    );
    let meta = e.store.note_meta(id, "a.md").await.unwrap();
    assert_eq!(meta.title, "After");
    assert_eq!(meta.aliases, vec!["New Alias"]);
    assert_eq!(e.store.backlinks(id, "b.md").await.unwrap().len(), 1);
    assert_eq!(
        e.dir("ws", id, "")
            .await
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.path == "a.md")
            .unwrap()
            .title
            .as_deref(),
        Some("After")
    );
    assert_eq!(
        e.switcher("ws", id, "New Alias").await.unwrap()[0].path,
        "a.md"
    );
    let state = e.index_state(id);
    let cache = state.cache.read().unwrap();
    assert!(
        cache
            .as_ref()
            .unwrap()
            .resolver
            .resolve("b.md", "New Alias")
            .is_none(),
        "metadata aliases do not introduce new resolver semantics"
    );
    assert!(cache
        .as_ref()
        .unwrap()
        .resolver
        .resolve("b.md", "After")
        .is_none());
}

#[tokio::test]
async fn warm_directory_reads_do_not_reload_all_rows() {
    let (e, _dir, id) = fixture().await;
    e.dir("ws", id, "").await.unwrap();
    let reads = e.store.all_reads.load(Relaxed);
    for _ in 0..10 {
        assert_eq!(e.dir("ws", id, "").await.unwrap().entries.len(), 2);
    }
    assert_eq!(
        e.store.all_reads.load(Relaxed),
        reads,
        "warm directories must read cached children"
    );
}

#[tokio::test]
async fn oversized_note_is_metadata_only() {
    let (e, dir, id) = fixture().await;
    let body = format!("[[b]] #oldtag\n{}", "x".repeat(MAX_FTS_BYTES as usize));
    std::fs::write(dir.path().join("a.md"), &body).unwrap();
    e.scan(id).await.unwrap();
    let meta = e.store.note_meta(id, "a.md").await.unwrap();
    assert!(
        meta.tags.is_empty(),
        "oversized metadata-only index must not parse tags"
    );
    assert!(e.store.outgoing(id, "a.md").await.unwrap().is_empty());
    assert_eq!(meta.hash, hex_sha256(body.as_bytes()));
    assert_eq!(
        serde_json::to_value(meta).unwrap()["content_index_status"],
        "size_limited"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
        body
    );
}

#[tokio::test]
async fn unchanged_scan_keeps_indexes_and_generation() {
    let (e, _dir, id) = fixture().await;
    e.dir("ws", id, "").await.unwrap();
    let generation = e.generation(id).load(Relaxed);
    let reads = e.store.all_reads.load(Relaxed);
    e.scan(id).await.unwrap();
    assert_eq!(e.generation(id).load(Relaxed), generation);
    assert_eq!(
        e.store.all_reads.load(Relaxed),
        reads,
        "unchanged scan must not rebuild switcher or directory indexes"
    );
}

#[tokio::test]
async fn scan_many_added_targets_reconciles_links_once() {
    let (e, dir, id) = fixture().await;
    let links = (0..100)
        .map(|n| format!("[[target{n}]]"))
        .collect::<Vec<_>>()
        .join(" ");
    e.write_note("ws", id, "a.md", &links, None).await.unwrap();
    for n in 0..100 {
        std::fs::write(dir.path().join(format!("target{n}.md")), "# Target").unwrap();
    }
    let passes = e.store.link_reads.load(Relaxed);
    e.scan(id).await.unwrap();
    assert_eq!(
        e.store.link_reads.load(Relaxed),
        passes + 1,
        "one structural scan, not one global link pass per added file"
    );
    assert!(e
        .store
        .outgoing(id, "a.md")
        .await
        .unwrap()
        .iter()
        .all(|l| l.dst_path.is_some()));
    assert_eq!(e.dir("ws", id, "").await.unwrap().entries.len(), 102);
}

fn pause_scan(
    e: &VaultEngine,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let (started, ready) = tokio::sync::oneshot::channel();
    let (resume, receiver) = tokio::sync::oneshot::channel();
    *e.scan_pause.lock().unwrap() = Some((started, receiver));
    (ready, resume)
}

#[tokio::test]
async fn cold_hydration_serializes_save() {
    let (e, _dir, id) = fixture().await;
    let state = e.index_state(id);
    state.invalidate();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (resume, receiver) = tokio::sync::oneshot::channel();
    *state.hydration_pause.lock().unwrap() = Some((started, receiver));
    let reads = e.store.all_reads.load(Relaxed);
    let reader = {
        let e = e.clone();
        tokio::spawn(async move { e.dir("ws", id, "").await })
    };
    ready.await.unwrap();
    let writer = {
        let e = e.clone();
        tokio::spawn(async move { e.write_note("ws", id, "new.md", "# New", None).await })
    };
    tokio::task::yield_now().await;
    assert!(
        !writer.is_finished(),
        "writer cannot pass gate-owned hydration"
    );
    resume.send(()).unwrap();
    reader.await.unwrap().unwrap();
    writer.await.unwrap().unwrap();
    assert_eq!(e.store.all_reads.load(Relaxed), reads + 1);
    assert_eq!(e.dir("ws", id, "").await.unwrap().entries.len(), 3);
    assert_eq!(e.store.note_meta(id, "new.md").await.unwrap().title, "New");
}

#[tokio::test]
async fn scan_cannot_overwrite_newer_api_delta() {
    let (e, dir, id) = fixture().await;
    std::fs::write(dir.path().join("a.md"), "# External old").unwrap();
    let (ready, resume) = pause_scan(&e);
    let scan = {
        let e = e.clone();
        tokio::spawn(async move { e.scan(id).await })
    };
    ready.await.unwrap();
    e.write_note("ws", id, "a.md", "# API newest", None)
        .await
        .unwrap();
    e.write_note("ws", id, "new.md", "# New", None)
        .await
        .unwrap();
    resume.send(()).unwrap();
    scan.await.unwrap().unwrap();
    assert_eq!(
        e.store.note_meta(id, "a.md").await.unwrap().title,
        "API newest"
    );
    assert_eq!(e.dir("ws", id, "").await.unwrap().entries.len(), 3);
}

#[tokio::test]
async fn incomplete_walk_preserves_indexed_notes() {
    let (e, dir, id) = fixture().await;
    std::fs::remove_file(dir.path().join("a.md")).unwrap();
    let mut walk = scan::walk(dir.path()).unwrap();
    walk.complete = false;
    *e.scan_override.lock().unwrap() = Some(walk);
    assert!(e.scan(id).await.is_err());
    assert!(e.store.note_meta(id, "a.md").await.is_ok());
    assert_eq!(e.dir("ws", id, "").await.unwrap().entries.len(), 2);
}

#[tokio::test]
async fn reappeared_removal_candidate_is_retained() {
    let (e, dir, id) = fixture().await;
    std::fs::remove_file(dir.path().join("a.md")).unwrap();
    let (ready, resume) = pause_scan(&e);
    let scanner = {
        let e = e.clone();
        tokio::spawn(async move { e.scan(id).await })
    };
    ready.await.unwrap();
    std::fs::write(dir.path().join("a.md"), "# Returned").unwrap();
    resume.send(()).unwrap();
    assert!(scanner.await.unwrap().is_err());
    assert!(e.store.note_meta(id, "a.md").await.is_ok());
    e.scan(id).await.unwrap();
    assert_eq!(
        e.store.note_meta(id, "a.md").await.unwrap().title,
        "Returned"
    );
}

#[tokio::test]
async fn scan_folder_rename_fences_descendants() {
    let (e, dir, id) = fixture().await;
    std::fs::create_dir(dir.path().join("old")).unwrap();
    std::fs::write(dir.path().join("old/child.md"), "# Child").unwrap();
    e.scan(id).await.unwrap();
    let (ready, resume) = pause_scan(&e);
    let scanner = {
        let e = e.clone();
        tokio::spawn(async move { e.scan(id).await })
    };
    ready.await.unwrap();
    let rename = {
        let e = e.clone();
        tokio::spawn(async move { e.rename("ws", id, "old", "new").await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !dir.path().join("new/child.md").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let state = e.index_state(id);
    let old_fenced = state.changed_since("old/child.md", 0);
    let new_fenced = state.changed_since("new/child.md", 0);
    resume.send(()).unwrap();
    let _ = scanner.await.unwrap();
    rename.await.unwrap().unwrap();
    assert!(
        old_fenced && new_fenced,
        "folder epochs must cover both old and new descendant paths"
    );
    assert!(e.store.note_meta(id, "old/child.md").await.is_err());
    assert_eq!(
        e.store.note_meta(id, "new/child.md").await.unwrap().title,
        "Child"
    );
}

#[tokio::test]
async fn index_delta_rolls_back_derived_rows_and_keeps_source() {
    let (e, dir, id) = fixture().await;
    let before = e.store.note_meta(id, "a.md").await.unwrap();
    sqlx::query("CREATE TRIGGER reject_new_vault_tags BEFORE INSERT ON vault_tags BEGIN SELECT RAISE(FAIL,'fixture indexing failure'); END").execute(e.store.pool()).await.unwrap();
    assert!(e
        .write_note("ws", id, "a.md", "# After #new", Some(&before.hash))
        .await
        .is_err());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
        "# After #new"
    );
    assert_eq!(
        e.store.note_meta(id, "a.md").await.unwrap().hash,
        before.hash
    );
    assert_eq!(e.store.outgoing(id, "a.md").await.unwrap().len(), 1);
    sqlx::query("DROP TRIGGER reject_new_vault_tags")
        .execute(e.store.pool())
        .await
        .unwrap();
    e.scan(id).await.unwrap();
    assert_eq!(
        e.store.note_meta(id, "a.md").await.unwrap().hash,
        hex_sha256(b"# After #new")
    );
}

#[tokio::test]
async fn size_limited_note_shrinking_restores_content_index() {
    let (e, dir, id) = fixture().await;
    let large = "x".repeat(MAX_FTS_BYTES as usize + 1);
    std::fs::write(dir.path().join("a.md"), &large).unwrap();
    e.scan(id).await.unwrap();
    assert_eq!(
        e.store
            .note_meta(id, "a.md")
            .await
            .unwrap()
            .content_index_status,
        ContentIndexStatus::SizeLimited
    );
    assert!(e.store.backlinks(id, "b.md").await.unwrap().is_empty());
    let raw = e.note("ws", id, "a.md").await.unwrap();
    assert_eq!(raw.raw, large);
    assert_eq!(
        raw.meta.content_index_status,
        ContentIndexStatus::SizeLimited
    );
    std::fs::write(dir.path().join("a.md"), "# Small\n[[b]] #tag").unwrap();
    e.scan(id).await.unwrap();
    let note = e.store.note_meta(id, "a.md").await.unwrap();
    assert_eq!(note.content_index_status, ContentIndexStatus::Full);
    assert_eq!(note.tags, vec!["tag"]);
    assert_eq!(e.store.backlinks(id, "b.md").await.unwrap().len(), 1);
}

#[tokio::test]
async fn warm_directories_scale_to_twenty_thousand_indexed_files() {
    let (e, _dir, id) = fixture().await;
    let mut tx = e.store.pool().begin().await.unwrap();
    for n in 0..20_000 {
        sqlx::query("INSERT INTO vault_notes(vault_id,path,title) VALUES(?,?,?)")
            .bind(id)
            .bind(format!("dir{}/note{n}.md", n % 200))
            .bind(format!("Note {n}"))
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    e.index_state(id).invalidate();
    e.last_scan_cell(id)
        .store(chrono::Utc::now().timestamp() + 3600, Relaxed);
    assert_eq!(e.dir("ws", id, "").await.unwrap().entries.len(), 202);
    let reads = e.store.all_reads.load(Relaxed);
    for n in 0..100 {
        assert_eq!(
            e.dir("ws", id, &format!("dir{n}"))
                .await
                .unwrap()
                .entries
                .len(),
            100
        );
    }
    assert_eq!(e.store.all_reads.load(Relaxed), reads);
    let walks = e.walks.load(Relaxed);
    let links = e.store.link_reads.load(Relaxed);
    e.write_note("ws", id, "a.md", "# Saved among 20000 notes", None)
        .await
        .unwrap();
    assert_eq!(e.store.all_reads.load(Relaxed), reads);
    assert_eq!(e.walks.load(Relaxed), walks);
    assert_eq!(e.store.link_reads.load(Relaxed), links);
}

#[tokio::test]
async fn unregister_releases_cached_indexes() {
    let (e, _dir, id) = fixture().await;
    e.dir("ws", id, "").await.unwrap();
    assert!(e.indexes.lock().unwrap().contains_key(&id));
    e.unregister("ws", id).await.unwrap();
    assert!(
        !e.indexes.lock().unwrap().contains_key(&id),
        "unregistered Vault must release cache ownership"
    );
}

#[tokio::test]
async fn failed_structural_scan_retries_link_reconciliation() {
    let (e, dir, id) = fixture().await;
    e.write_note("ws", id, "a.md", "[[target]]", None)
        .await
        .unwrap();
    std::fs::write(dir.path().join("target.md"), "# Target").unwrap();
    std::fs::write(dir.path().join("zz-fail.md"), "# Fails").unwrap();
    sqlx::query("CREATE TRIGGER fail_late_note BEFORE INSERT ON vault_notes WHEN NEW.path='zz-fail.md' BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(e.store.pool()).await.unwrap();
    assert!(e.scan(id).await.is_err());
    assert!(e.store.note_meta(id, "target.md").await.is_ok());
    sqlx::query("DROP TRIGGER fail_late_note")
        .execute(e.store.pool())
        .await
        .unwrap();
    std::fs::remove_file(dir.path().join("zz-fail.md")).unwrap();
    e.scan(id).await.unwrap();
    assert_eq!(
        e.store.backlinks(id, "target.md").await.unwrap().len(),
        1,
        "retry must repair links even when no new membership delta remains"
    );
}

#[tokio::test]
async fn removal_failure_keeps_derived_note_atomic() {
    let (e, dir, id) = fixture().await;
    sqlx::query("CREATE TRIGGER fail_link_delete BEFORE DELETE ON vault_links BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(e.store.pool()).await.unwrap();
    std::fs::remove_file(dir.path().join("a.md")).unwrap();
    assert!(e.scan(id).await.is_err());
    assert!(
        e.store.note_meta(id, "a.md").await.is_ok(),
        "failed removal must roll back note metadata too"
    );
    sqlx::query("DROP TRIGGER fail_link_delete")
        .execute(e.store.pool())
        .await
        .unwrap();
    e.scan(id).await.unwrap();
    assert!(e.store.note_meta(id, "a.md").await.is_err());
    assert!(e.store.backlinks(id, "b.md").await.unwrap().is_empty());
}

#[tokio::test]
async fn unregister_retires_a_paused_scan() {
    let (e, dir, id) = fixture().await;
    std::fs::write(dir.path().join("new.md"), "# New").unwrap();
    let (ready, resume) = pause_scan(&e);
    let scan = {
        let e = e.clone();
        tokio::spawn(async move { e.scan(id).await })
    };
    ready.await.unwrap();
    e.unregister("ws", id).await.unwrap();
    resume.send(()).unwrap();
    assert!(scan.await.unwrap().is_err());
    assert!(!e.indexes.lock().unwrap().contains_key(&id));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM vault_notes WHERE vault_id=?")
        .bind(id)
        .fetch_one(e.store.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(dir.path().join("new.md").exists());
}

#[tokio::test]
async fn external_content_only_change_avoids_global_pass_and_delta_keeps_scan_clock() {
    let (e, dir, id) = fixture().await;
    let passes = e.store.link_reads.load(Relaxed);
    let prior = e.last_scan_cell(id).load(Relaxed);
    e.write_note("ws", id, "b.md", "# Updated", None)
        .await
        .unwrap();
    assert_eq!(
        e.last_scan_cell(id).load(Relaxed),
        prior,
        "local save must not postpone unrelated external detection"
    );
    std::fs::write(dir.path().join("a.md"), "# Changed externally\n[[missing]]").unwrap();
    e.scan(id).await.unwrap();
    assert_eq!(e.store.link_reads.load(Relaxed), passes);
    assert_eq!(
        e.store.note_meta(id, "a.md").await.unwrap().title,
        "Changed externally"
    );
    assert!(e.store.backlinks(id, "b.md").await.unwrap().is_empty());
}

#[tokio::test]
async fn basename_candidate_changes_reconcile_existing_incoming_links() {
    let (e, dir, id) = fixture().await;
    std::fs::create_dir(dir.path().join("x")).unwrap();
    std::fs::create_dir(dir.path().join("y")).unwrap();
    e.write_note("ws", id, "x/Topic.md", "# One", None)
        .await
        .unwrap();
    e.write_note("ws", id, "a.md", "[[Topic]]", None)
        .await
        .unwrap();
    assert_eq!(
        e.store.outgoing(id, "a.md").await.unwrap()[0]
            .dst_path
            .as_deref(),
        Some("x/Topic.md")
    );
    e.write_note("ws", id, "y/Topic.md", "# Two", None)
        .await
        .unwrap();
    assert_eq!(
        e.store.outgoing(id, "a.md").await.unwrap()[0].dst_path,
        None
    );
    e.delete_note("ws", id, "y/Topic.md").await.unwrap();
    assert_eq!(
        e.store.outgoing(id, "a.md").await.unwrap()[0]
            .dst_path
            .as_deref(),
        Some("x/Topic.md")
    );
}

#[tokio::test]
async fn legacy_oversized_rows_reindex_without_signature_change() {
    let (e, dir, id) = fixture().await;
    std::fs::write(
        dir.path().join("a.md"),
        "x".repeat(MAX_FTS_BYTES as usize + 1),
    )
    .unwrap();
    e.scan(id).await.unwrap();
    sqlx::query("UPDATE vault_notes SET content_index_status='full',tags_json='[\"legacy\"]' WHERE vault_id=? AND path='a.md'").bind(id).execute(e.store.pool()).await.unwrap();
    e.scan(id).await.unwrap();
    let meta = e.store.note_meta(id, "a.md").await.unwrap();
    assert_eq!(meta.content_index_status, ContentIndexStatus::SizeLimited);
    assert!(meta.tags.is_empty());
    let mut wire = serde_json::to_value(meta).unwrap();
    wire.as_object_mut().unwrap().remove("content_index_status");
    assert_eq!(
        serde_json::from_value::<NoteMeta>(wire)
            .unwrap()
            .content_index_status,
        ContentIndexStatus::Full
    );
}

#[tokio::test]
async fn artifact_index_failure_marks_cache_for_repair() {
    let (e, dir, id) = fixture().await;
    sqlx::query("CREATE TRIGGER fail_artifact BEFORE INSERT ON vault_files BEGIN SELECT RAISE(ABORT,'test failure'); END").execute(e.store.pool()).await.unwrap();
    assert!(e
        .write_text_file("ws", id, "artifact.json", "{}", None)
        .await
        .is_err());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("artifact.json")).unwrap(),
        "{}"
    );
    assert!(
        e.index_state(id).cache.read().unwrap().is_none(),
        "failed artifact publication must invalidate the apparently fresh cache"
    );
    assert_eq!(e.last_scan_cell(id).load(Relaxed), 0);
    sqlx::query("DROP TRIGGER fail_artifact")
        .execute(e.store.pool())
        .await
        .unwrap();
    e.scan(id).await.unwrap();
    assert!(e
        .dir("ws", id, "")
        .await
        .unwrap()
        .entries
        .iter()
        .any(|entry| entry.path == "artifact.json"));
}

#[tokio::test]
async fn canceled_after_sql_commit_never_leaves_a_fresh_stale_cache() {
    let (e, dir, id) = fixture().await;
    let state = e.index_state(id);
    let (started, ready) = tokio::sync::oneshot::channel();
    let (_resume, receiver) = tokio::sync::oneshot::channel();
    *state.publication_pause.lock().unwrap() = Some((started, receiver));
    let writer = {
        let e = e.clone();
        tokio::spawn(async move { e.write_note("ws", id, "a.md", "# Committed", None).await })
    };
    ready.await.unwrap();
    assert_eq!(
        e.store.note_meta(id, "a.md").await.unwrap().title,
        "Committed"
    );
    writer.abort();
    let _ = writer.await;
    assert!(
        state.cache.read().unwrap().is_none(),
        "canceled publication must not leave the old cached title marked current"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
        "# Committed"
    );
    e.scan(id).await.unwrap();
    assert_eq!(
        e.dir("ws", id, "")
            .await
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.path == "a.md")
            .unwrap()
            .title
            .as_deref(),
        Some("Committed")
    );
}

#[tokio::test]
async fn canceled_scan_file_and_removal_publications_invalidate_the_cache() {
    for case in ["add", "remove_file", "remove_note"] {
        let (e, dir, id) = fixture().await;
        let path = if case == "remove_note" {
            "a.md"
        } else {
            "artifact.json"
        };
        if case == "remove_file" {
            e.write_text_file("ws", id, path, "{}", None).await.unwrap();
        }
        if case == "add" {
            std::fs::write(dir.path().join(path), "{}").unwrap();
        } else {
            std::fs::remove_file(dir.path().join(path)).unwrap();
        }
        let state = e.index_state(id);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (_resume, receiver) = tokio::sync::oneshot::channel();
        *state.publication_pause.lock().unwrap() = Some((started, receiver));
        let scan = {
            let e = e.clone();
            tokio::spawn(async move { e.scan(id).await })
        };
        ready.await.unwrap();
        scan.abort();
        let _ = scan.await;
        assert!(
            state.cache.read().unwrap().is_none(),
            "canceled file publication: {case}"
        );
        e.scan(id).await.unwrap();
        let present = e
            .dir("ws", id, "")
            .await
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.path == path);
        assert_eq!(present, case == "add");
    }
}

#[test]
fn held_file_stat_timestamp_matches_standard_metadata_precision() {
    let file = tempfile::tempfile().unwrap();
    let modified = std::time::UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 123_456_789);
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let stat = rustix::fs::fstat(&file).unwrap();
    let expected = file
        .metadata()
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;
    assert_eq!(
        stat_mtime_ns(&stat),
        expected,
        "held-fd index signature must retain subsecond precision"
    );
}
