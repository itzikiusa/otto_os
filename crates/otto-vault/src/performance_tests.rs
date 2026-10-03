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

#[tokio::test]
async fn backlink_context_is_cached_per_source_hash() {
    let (e, dir, id) = fixture().await;
    let first = e.backlinks("ws", id, "b.md").await.unwrap();
    assert_eq!(first.len(), 1);
    let ctx0 = first[0].context.clone();
    assert!(!ctx0.is_empty());
    // Same indexed hash → served from the cache, even if the file is not
    // re-read (prove it by making the file unreadable as text meanwhile).
    std::fs::write(dir.path().join("a.md"), [0xff_u8, 0xfe]).unwrap();
    let cached = e.backlinks("ws", id, "b.md").await.unwrap();
    assert_eq!(cached[0].context, ctx0);
    // A save moves the hash → the context is recomputed from the new body.
    e.write_note("ws", id, "a.md", "# A\nsee [[b]] here", None)
        .await
        .unwrap();
    let fresh = e.backlinks("ws", id, "b.md").await.unwrap();
    assert_eq!(fresh[0].context, "see [[b]] here");
}

#[tokio::test]
async fn quiet_rescan_writes_no_scan_state() {
    let (e, dir, id) = fixture().await;
    let before = e.store.get_vault(id).await.unwrap();
    assert_eq!(before.scan_state, "idle");
    // Mark the row so a write would be visible.
    sqlx::query("UPDATE vaults SET scan_state = 'marker' WHERE id = ?")
        .bind(id)
        .execute(e.store.pool())
        .await
        .unwrap();
    e.scan(id).await.unwrap();
    assert_eq!(
        e.store.get_vault(id).await.unwrap().scan_state,
        "marker",
        "a no-op rescan must not touch scan_state"
    );
    // A scan that changes the index does write `idle` again.
    std::fs::write(dir.path().join("c.md"), "# C").unwrap();
    e.scan(id).await.unwrap();
    assert_eq!(e.store.get_vault(id).await.unwrap().scan_state, "idle");
}

/// SD-15 (listing half): history for one note in a vault with many revisions
/// reads the index, not every `meta.json` — and the index is a cache that
/// learns revisions it doesn't know (older history, other writers) once. The
/// meta-read counter is the load-bearing check and is size-independent, so the
/// everyday run uses 5k revisions; the 50k-scale timing budget is the ignored
/// variant below (CI runs it in its own step).
#[tokio::test(flavor = "multi_thread")]
async fn revision_listing_reads_only_the_path_index() {
    revision_listing_reads_only_the_path_index_at(5_000).await;
}

/// The SD-15 scale gate: the same scenario at 50k revisions (~40 s of file
/// creation in a debug build). `cargo nextest run --run-ignored only -E
/// 'test(/at_50k_revisions/)'` — CI runs it in a dedicated step.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "50k-scale perf gate; run with --run-ignored (CI: dedicated step)"]
async fn revision_listing_at_50k_revisions_reads_only_the_path_index() {
    revision_listing_reads_only_the_path_index_at(50_000).await;
}

async fn revision_listing_reads_only_the_path_index_at(n: usize) {
    use crate::recovery::INDEX_META_READS;
    // INDEX_META_READS is process-global: keep the two sizes from interleaving
    // under `cargo test --include-ignored` (threads in one process).
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _serial = SERIAL.lock().await;
    let (e, dir, id) = fixture().await;
    let hist = dir.path().join(".otto-history");
    std::fs::create_dir_all(&hist).unwrap();
    // Every `stride`-th revision is `a.md`: always 50 hits, whatever the size.
    let stride = n / 50;
    for i in 0..n {
        // Time-sortable ids like `new_id()`.
        let rid = format!("r{i:08}");
        let path = if i % stride == 0 { "a.md" } else { "other.md" };
        let rdir = hist.join(&rid);
        std::fs::create_dir(&rdir).unwrap();
        let rev = VaultRevision {
            id: rid.clone(),
            path: path.into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            before_hash: None,
            after_hash: "x".into(),
            reason: "note write".into(),
            committed: true,
        };
        std::fs::write(rdir.join("meta.json"), serde_json::to_vec(&rev).unwrap()).unwrap();
    }

    // First listing learns every revision once (pre-index history) and
    // persists the index.
    let first = e.revisions("ws", id, Some("a.md")).await.unwrap();
    assert_eq!(first.len(), n / stride);
    assert_eq!(first[0].id, format!("r{:08}", n - stride), "newest first");
    assert!(hist.join(".path-index.jsonl").is_file());

    // Warm: no meta read beyond the 50 hits themselves, well under 50 ms.
    let reads = INDEX_META_READS.load(Relaxed);
    let t = std::time::Instant::now();
    let warm = e.revisions("ws", id, Some("a.md")).await.unwrap();
    let warm_ms = t.elapsed().as_millis();
    assert_eq!(warm.len(), n / stride);
    assert_eq!(
        INDEX_META_READS.load(Relaxed),
        reads,
        "index hit: no meta scan"
    );
    // 50 ms is the release budget; unoptimized test builds get headroom (the
    // meta-read counter above is the load-bearing regression check).
    let budget = if cfg!(debug_assertions) { 250 } else { 50 };
    assert!(
        warm_ms < budget,
        "warm {n}-revision listing took {warm_ms} ms"
    );

    // A revision the index has never seen (another writer) is learned once.
    let rid = format!("r{:08}", n);
    std::fs::create_dir(hist.join(&rid)).unwrap();
    let rev = VaultRevision {
        id: rid.clone(),
        path: "a.md".into(),
        created_at: "2026-01-02T00:00:00Z".into(),
        before_hash: None,
        after_hash: "y".into(),
        reason: "note write".into(),
        committed: true,
    };
    std::fs::write(
        hist.join(&rid).join("meta.json"),
        serde_json::to_vec(&rev).unwrap(),
    )
    .unwrap();
    let next = e.revisions("ws", id, Some("a.md")).await.unwrap();
    assert_eq!(next[0].id, rid);
    assert_eq!(INDEX_META_READS.load(Relaxed), reads + 1);

    // Paging by cursor still works on the index.
    let older = e
        .revisions_page("ws", id, Some("a.md"), Some(&next[1].id))
        .await
        .unwrap();
    assert_eq!(older.len(), n / stride - 1);
}

fn graph_opts(mode: &str, path: Option<&str>, depth: usize) -> GraphOpts {
    GraphOpts {
        mode: mode.into(),
        path: path.map(Into::into),
        depth,
        ..Default::default()
    }
}

#[tokio::test]
async fn body_only_save_keeps_graph_generation_and_cached_graph() {
    let (e, _dir, id) = fixture().await;
    let full = graph_opts("full", None, 1);
    let first = e.graph("ws", id, &full).await.unwrap();
    let builds = e.graph_builds.load(Relaxed);
    let graph_gen = e.graph_generation(id).load(Relaxed);
    e.write_note("ws", id, "a.md", "# Before\n[[b]]\n\nMore prose.", None)
        .await
        .unwrap();
    assert_eq!(
        e.graph_generation(id).load(Relaxed),
        graph_gen,
        "a body-only save must not move the graph generation"
    );
    let again = e.graph("ws", id, &full).await.unwrap();
    assert_eq!(e.graph_builds.load(Relaxed), builds, "served from cache");
    assert_eq!(again.paths, first.paths);
    // A link change moves the graph generation and rebuilds.
    e.write_note("ws", id, "a.md", "# Before\nno links now", None)
        .await
        .unwrap();
    assert_ne!(e.graph_generation(id).load(Relaxed), graph_gen);
    let rebuilt = e.graph("ws", id, &full).await.unwrap();
    assert_eq!(e.graph_builds.load(Relaxed), builds + 1);
    assert!(rebuilt.edges.is_empty(), "{:?}", rebuilt.edges);
    let status = e.status("ws", id).await.unwrap();
    assert_eq!(
        status.graph_generation,
        Some(e.graph_generation(id).load(Relaxed).to_string())
    );
}

#[tokio::test]
async fn local_graph_bfs_uses_indexed_neighbourhood() {
    let engine = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in [
        ("a.md", "[[b]]"),
        ("b.md", "[[c]] [[a]]"),
        ("c.md", "[[d]]"),
        ("d.md", "end"),
        ("x.md", "[[y]]"),
        ("y.md", "far away"),
        ("index.md", "[[a]] [[x]]"),
    ] {
        std::fs::write(dir.path().join(name), body).unwrap();
    }
    let id = engine
        .store
        .create_vault("ws", "T", dir.path().to_str().unwrap(), true)
        .await
        .unwrap();
    engine.scan(id).await.unwrap();
    for depth in 1..=3 {
        let o = graph_opts("local", Some("a.md"), depth);
        let indexed = engine.graph_local_indexed(id, &o).await.unwrap();
        let built = engine.graph_build(id, &o).await.unwrap();
        let mut a: Vec<_> = indexed.paths.clone();
        let mut b: Vec<_> = built.paths.clone();
        a.sort();
        b.sort();
        assert_eq!(a, b, "depth {depth}");
        let pairs = |g: &GraphPayload| {
            let mut v: Vec<(String, String)> = g
                .edges
                .chunks(2)
                .map(|p| {
                    (
                        g.paths[p[0] as usize].clone(),
                        g.paths[p[1] as usize].clone(),
                    )
                })
                .collect();
            v.sort();
            v.dedup();
            v
        };
        assert_eq!(pairs(&indexed), pairs(&built), "depth {depth}");
    }
    let o = graph_opts("local", Some("index.md"), 1);
    assert!(
        engine.graph_local_indexed(id, &o).await.is_err(),
        "a reserved focus is hidden unless reserved=true"
    );
}

#[tokio::test]
async fn search_filters_apply_before_the_limit_without_full_reads() {
    let engine = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("deep")).unwrap();
    for i in 0..260 {
        std::fs::write(
            dir.path().join(format!("n{i:03}.md")),
            "alpha alpha alpha alpha",
        )
        .unwrap();
    }
    let filler = "lorem ipsum dolor sit amet ".repeat(80);
    std::fs::write(
        dir.path().join("deep/rare.md"),
        format!("---\ntype: Runbook\ntags: [rare/sub]\n---\nalpha {filler}"),
    )
    .unwrap();
    let id = engine
        .store
        .create_vault("ws", "T", dir.path().to_str().unwrap(), false)
        .await
        .unwrap();
    engine.scan(id).await.unwrap();
    let reads = engine.store.all_reads.load(Relaxed);
    let search = |q: &str| SearchReq {
        query: q.into(),
        limit: 10,
        ..Default::default()
    };
    for q in [
        "alpha tag:rare",
        "alpha tag:#rare/sub",
        "alpha path:deep/",
        "alpha type:runbook",
        "tag:rare",
    ] {
        let hits = engine.search("ws", id, &search(q)).await.unwrap();
        assert_eq!(
            hits.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(),
            vec!["deep/rare.md"],
            "{q}"
        );
    }
    assert!(engine
        .search("ws", id, &search("alpha tag:rar"))
        .await
        .unwrap()
        .is_empty());
    let plain = engine.search("ws", id, &search("alpha")).await.unwrap();
    assert_eq!(plain.len(), 10);
    assert_eq!(
        engine.store.all_reads.load(Relaxed),
        reads,
        "search must not read the whole notes table"
    );
}

#[tokio::test]
async fn rename_survives_a_failed_link_rewrite() {
    let (e, dir, id) = fixture().await;
    *e.rename_fail.lock().unwrap() = Some("a.md".into());
    let r = e.rename("ws", id, "b.md", "sub/c.md").await.unwrap();
    assert_eq!(r.links_failed, vec!["a.md".to_string()]);
    assert_eq!(r.links_updated, 0);
    assert!(dir.path().join("sub/c.md").is_file());
    assert!(
        e.store.note_meta(id, "sub/c.md").await.is_ok(),
        "the moved note is indexed"
    );
    assert!(e.store.note_meta(id, "b.md").await.is_err());
    // The source kept its (now unresolved) link; nothing was half-written.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
        "# Before\n[[b]]"
    );
}

/// F1: 100 autosaves of one note inside the coalescing window make ONE
/// revision directory and a bounded set of bodies; the revision still
/// restores the original `before` and the latest `after`.
#[tokio::test]
async fn autosaves_coalesce_into_one_deduped_revision() {
    let (e, dir, id) = fixture().await;
    let root = dir.path().join(".otto-history");
    let mut last = String::new();
    for i in 0..100 {
        last = format!("# Before\n[[b]]\nedit {i}");
        e.write_note_opts("ws", id, "a.md", &last, None, true)
            .await
            .unwrap();
    }
    let revision_dirs = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|d| d.ok())
        .filter(|d| !d.file_name().to_string_lossy().starts_with('.'))
        .collect::<Vec<_>>();
    assert_eq!(
        revision_dirs.len(),
        1,
        "one revision per window, not per save"
    );
    let files_in_dir = std::fs::read_dir(revision_dirs[0].path())
        .unwrap()
        .filter_map(|d| d.ok())
        .count();
    assert_eq!(
        files_in_dir, 2,
        "meta.json + the latest after; superseded afters retired"
    );
    let blobs = std::fs::read_dir(root.join(".blobs")).unwrap().count();
    assert_eq!(blobs, 1, "only the original before is a blob");
    let revs = e.revisions("ws", id, Some("a.md")).await.unwrap();
    assert_eq!(revs.len(), 1);
    assert!(revs[0].committed);
    let detail = e.revision("ws", id, &revs[0].id).await.unwrap();
    assert_eq!(detail.before.as_deref(), Some("# Before\n[[b]]"));
    assert_eq!(detail.after, last);

    // An agent (non-autosave) write gets its own revision; its `before` is
    // hard-linked from the coalesced revision's `after` (no new bytes).
    e.write_note("ws", id, "a.md", "# Agent", None)
        .await
        .unwrap();
    let revs = e.revisions("ws", id, Some("a.md")).await.unwrap();
    assert_eq!(revs.len(), 2);
    let agent = e.revision("ws", id, &revs[0].id).await.unwrap();
    assert_eq!(agent.before.as_deref(), Some(last.as_str()));
    assert_eq!(agent.after, "# Agent");
    let blob = root
        .join(".blobs")
        .join(revs[0].before_hash.as_deref().unwrap());
    let linked = std::fs::read_dir(root.join(&revs[1].id))
        .unwrap()
        .filter_map(|d| d.ok())
        .find(|d| d.file_name().to_string_lossy().starts_with("a-"))
        .unwrap()
        .path();
    use std::os::unix::fs::MetadataExt;
    assert_eq!(
        std::fs::metadata(&blob).unwrap().ino(),
        std::fs::metadata(&linked).unwrap().ino(),
        "the next before is a hard link to the previous after"
    );
    // Autosave right after an agent write does not coalesce INTO it.
    e.write_note_opts("ws", id, "a.md", "# Agent\nmore", None, true)
        .await
        .unwrap();
    assert_eq!(e.revisions("ws", id, Some("a.md")).await.unwrap().len(), 3);
}

/// F1: legacy revisions (in-dir `before`/`after` copies) still read back.
#[tokio::test]
async fn legacy_revision_layout_still_restores() {
    let (e, dir, id) = fixture().await;
    let rev_dir = dir.path().join(".otto-history").join("legacy-1");
    std::fs::create_dir_all(&rev_dir).unwrap();
    std::fs::write(rev_dir.join("before"), "old").unwrap();
    std::fs::write(rev_dir.join("after"), "new").unwrap();
    let hash = |s: &str| hex_sha256(s.as_bytes());
    let meta = VaultRevision {
        id: "legacy-1".into(),
        path: "a.md".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        before_hash: Some(hash("old")),
        after_hash: hash("new"),
        reason: "note write".into(),
        committed: true,
    };
    std::fs::write(
        rev_dir.join("meta.json"),
        serde_json::to_vec(&meta).unwrap(),
    )
    .unwrap();
    let detail = e.revision("ws", id, "legacy-1").await.unwrap();
    assert_eq!(detail.before.as_deref(), Some("old"));
    assert_eq!(detail.after, "new");
}

/// F2: repeated status polls run the aggregate COUNTs once per index
/// generation, and with a healthy watcher they never walk the vault.
#[tokio::test]
async fn status_polls_reuse_counts_and_skip_walks_when_watched() {
    let (e, dir, id) = fixture().await;
    e.watch_enabled.store(true, Relaxed);
    e.status("ws", id).await.unwrap(); // starts the watcher
    for _ in 0..50 {
        if e.stale_after(id).is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(e.stale_after(id).is_some(), "watcher should be running");
    // Force "stale by the 30 s rule" — a watched vault must not walk.
    e.last_scan_cell(id)
        .store(chrono::Utc::now().timestamp() - 120, Relaxed);
    let walks = e.walks.load(Relaxed);
    let counts = e.store.status_count_reads.load(Relaxed);
    for _ in 0..10 {
        e.status("ws", id).await.unwrap();
    }
    tokio::task::yield_now().await;
    assert_eq!(
        e.walks.load(Relaxed),
        walks,
        "watched vault: no polling walks"
    );
    assert!(
        e.store.status_count_reads.load(Relaxed) <= counts + 1,
        "status COUNTs cached per generation"
    );
    // Our own guarded write re-indexes the path; the watcher's probe sees a
    // matching signature and does not kick a scan.
    let kicks = e.watch_kicks.load(Relaxed);
    e.write_note("ws", id, "a.md", "# Self write", None)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert_eq!(
        e.watch_kicks.load(Relaxed),
        kicks,
        "self-writes do not rescan"
    );
    // An external edit does.
    std::fs::write(dir.path().join("c.md"), "# External").unwrap();
    let mut seen = false;
    for _ in 0..100 {
        if e.store.note_meta(id, "c.md").await.is_ok() {
            seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(seen, "external note indexed by the watcher-triggered scan");
    assert!(e.watch_kicks.load(Relaxed) > kicks);
}

/// F11: a scale fixture — `n` linked notes, cold-indexed — with wall-clock
/// budgets for the hot reads. Budgets are deliberately loose (shared CI
/// runners); the counters are the strict part.
async fn scale_vault(
    n: usize,
) -> (
    Arc<VaultEngine>,
    tempfile::TempDir,
    i64,
    std::time::Duration,
) {
    let engine = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("bulk")).unwrap();
    for i in 0..n {
        std::fs::write(
            dir.path().join(format!("bulk/note-{i}.md")),
            format!(
                "---\ntitle: Note {i}\ntags: [t{}]\n---\nSynthetic note {i}. Links [[note-{}]].\n",
                i % 50,
                (i + 1) % n
            ),
        )
        .unwrap();
    }
    let id = engine
        .store
        .create_vault("ws", "Scale", dir.path().to_str().unwrap(), false)
        .await
        .unwrap();
    let t0 = std::time::Instant::now();
    engine.scan(id).await.unwrap();
    (engine, dir, id, t0.elapsed())
}

async fn assert_scale_budgets(n: usize, read_budget_ms: u128) {
    let (e, _dir, id, cold) = scale_vault(n).await;
    let mut worst = [0u128; 3];
    for i in 0..20 {
        let k = (i * 331) % n;
        let t = std::time::Instant::now();
        assert!(!e
            .switcher("ws", id, &format!("Note {k}"))
            .await
            .unwrap()
            .is_empty());
        worst[0] = worst[0].max(t.elapsed().as_millis());
        let t = std::time::Instant::now();
        let req = SearchReq {
            query: format!("Synthetic {k}"),
            tag: None,
            path_prefix: None,
            okf_type: None,
            limit: 20,
        };
        e.search("ws", id, &req).await.unwrap();
        worst[1] = worst[1].max(t.elapsed().as_millis());
        let t = std::time::Instant::now();
        e.note("ws", id, &format!("bulk/note-{k}.md"))
            .await
            .unwrap();
        worst[2] = worst[2].max(t.elapsed().as_millis());
    }
    let walks = e.walks.load(Relaxed);
    let t = std::time::Instant::now();
    for _ in 0..10 {
        e.status("ws", id).await.unwrap();
    }
    let status_ms = t.elapsed().as_millis();
    eprintln!(
        "[vault-scale n={n}] cold index {:?}; worst switcher={}ms search={}ms open={}ms; 10 status={}ms",
        cold, worst[0], worst[1], worst[2], status_ms
    );
    assert_eq!(
        e.walks.load(Relaxed),
        walks,
        "fresh vault: status polls never walk"
    );
    for w in worst {
        assert!(w < read_budget_ms, "hot read over budget: {worst:?}");
    }
    assert!(
        status_ms < read_budget_ms * 2,
        "status polls: {status_ms}ms"
    );
}

#[tokio::test]
async fn scale_1k_notes_hot_reads_within_budget() {
    assert_scale_budgets(1_000, 500).await;
}

/// `cargo test -p otto-vault --lib scale_10k -- --ignored --nocapture`
#[tokio::test]
#[ignore = "scale bench: 10k-note cold index (~tens of seconds)"]
async fn scale_10k_notes_hot_reads_within_budget() {
    assert_scale_budgets(10_000, 150).await;
}

/// F9: creating a note re-resolves only links that mention its stem — no
/// global link read — and still captures a previously unresolved link.
#[tokio::test]
async fn new_note_reconciles_only_candidate_links() {
    let (e, _dir, id) = fixture().await;
    e.write_note("ws", id, "b.md", "# Target\n[[Fresh-Note]] [[other]]", None)
        .await
        .unwrap();
    let passes = e.store.link_reads.load(Relaxed);
    e.write_note("ws", id, "sub/fresh-note.md", "# Fresh", None)
        .await
        .unwrap();
    assert_eq!(
        e.store.link_reads.load(Relaxed),
        passes,
        "no full link scan"
    );
    let back = e.store.backlinks(id, "sub/fresh-note.md").await.unwrap();
    assert_eq!(back.len(), 1, "[[Fresh-Note]] now resolves to the new note");
}
