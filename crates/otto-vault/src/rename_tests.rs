//! Rename rewrites inbound links, or names the sources it could not rewrite.
use super::*;

async fn vault(files: &[(&str, &[u8])]) -> (Arc<VaultEngine>, tempfile::TempDir, i64) {
    let e = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
    let dir = tempfile::tempdir().unwrap();
    for (path, body) in files {
        let abs = dir.path().join(path);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(abs, body).unwrap();
    }
    let id = e
        .store
        .create_vault("ws", "Rename", dir.path().to_str().unwrap(), false)
        .await
        .unwrap();
    e.scan(id).await.unwrap();
    (e, dir, id)
}

/// Every inbound form — wikilink, alias, heading anchor, embed, markdown
/// link — follows the note to its new path; aliases/anchors survive, the
/// index resolves all of them to the new path, nothing is left dangling.
#[tokio::test]
async fn rename_rewrites_every_inbound_link() {
    let (e, dir, id) = vault(&[
        ("target.md", b"# Target\n## Head\n"),
        (
            "a.md",
            b"See [[target]], [[target|the alias]], [[target#Head]], ![[target]] and [md](target.md).\n",
        ),
        ("other/b.md", b"Up: [t](../target.md)\n"),
    ])
    .await;
    let r = e
        .rename("ws", id, "target.md", "sub/moved.md")
        .await
        .unwrap();
    assert!(r.links_failed.is_empty(), "{:?}", r.links_failed);
    assert_eq!(r.links_updated, 6);
    let a = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
    assert!(!a.contains("target"), "{a}");
    assert!(a.contains("|the alias]]") && a.contains("#Head]]"), "{a}");
    for src in ["a.md", "other/b.md"] {
        let out = e.store.outgoing(id, src).await.unwrap();
        assert!(!out.is_empty());
        assert!(
            out.iter()
                .all(|l| l.dst_path.as_deref() == Some("sub/moved.md")),
            "{src}: {out:?}"
        );
    }
    let back = e.store.linking_sources(id, "sub/moved.md").await.unwrap();
    assert_eq!(back.len(), 2);
}

/// A linking source that cannot be read as text is not "0 links, success":
/// it is listed in `links_failed` and left byte-for-byte untouched.
#[tokio::test]
async fn an_unreadable_source_is_reported_not_skipped() {
    let bad: &[u8] = b"Link [[target]] \xff\xfe not utf-8\n";
    let (e, dir, id) = vault(&[("target.md", b"# Target\n"), ("bad.md", bad)]).await;
    assert_eq!(
        e.store.linking_sources(id, "target.md").await.unwrap(),
        vec!["bad.md".to_string()],
        "the lossy parse indexed the link"
    );
    let r = e.rename("ws", id, "target.md", "renamed.md").await.unwrap();
    assert_eq!(r.links_failed, vec!["bad.md".to_string()]);
    assert_eq!(r.links_updated, 0);
    assert_eq!(std::fs::read(dir.path().join("bad.md")).unwrap(), bad);
    assert!(dir.path().join("renamed.md").is_file());
}

/// A rename the filesystem refuses after its target's parents were made (a
/// folder moved into itself) leaves none of those new folders behind, and
/// never touches folders that existed before. It used to leave them empty.
#[tokio::test]
async fn a_failed_rename_takes_back_only_the_folders_it_created() {
    let (e, dir, id) = vault(&[("dir/had/keep.md", b"# K\n"), ("dir/n.md", b"# N\n")]).await;
    let err = e
        .rename("ws", id, "dir", "dir/had/new/deeper/moved")
        .await
        .expect_err("a folder cannot move into itself");
    assert!(matches!(err, Error::Internal(_)), "{err:?}");
    assert!(dir.path().join("dir/n.md").is_file(), "the source stays");
    assert!(
        dir.path().join("dir/had/keep.md").is_file(),
        "existing folders stay"
    );
    assert!(
        !dir.path().join("dir/had/new").exists(),
        "folders this rename made are taken back"
    );
}

/// Restoring from trash obeys the same protected-path rule as every write,
/// a restore onto an existing file is a conflict that keeps the trash entry,
/// and a restore that fails after making its target's folders removes them.
#[tokio::test]
async fn trash_restore_is_guarded_and_cleans_up_after_a_failure() {
    let (e, dir, id) = vault(&[("notes/a.md", b"# A\n"), ("taken/sub/a.md", b"theirs")]).await;
    e.delete_note("ws", id, "notes/a.md").await.unwrap();
    let item = e.trash_entries("ws", id).await.unwrap().remove(0);
    let err = e
        .restore_trash("ws", id, &item.id, Some("keys/server.pem"))
        .await
        .expect_err("a key-like name is protected");
    assert!(matches!(err, Error::Forbidden(_)), "{err:?}");
    assert!(!dir.path().join("keys").exists(), "nothing was created");
    let err = e
        .restore_trash("ws", id, &item.id, Some("taken/sub/a.md"))
        .await
        .expect_err("the target exists");
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert_eq!(
        std::fs::read(dir.path().join("taken/sub/a.md")).unwrap(),
        b"theirs"
    );
    assert!(
        dir.path().join("taken/sub").is_dir(),
        "existing folders stay"
    );
    let entries = e.trash_entries("ws", id).await.unwrap();
    assert!(entries.iter().any(|t| t.id == item.id), "still restorable");
    // The trashed file vanishes (emptied by hand): the move fails after the
    // destination's folders were made, and they are taken back.
    std::fs::remove_file(dir.path().join(".trash").join(&item.stored_path)).unwrap();
    let err = e
        .restore_trash("ws", id, &item.id, Some("made/here/a.md"))
        .await
        .expect_err("nothing left to restore");
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert!(
        !dir.path().join("made").exists(),
        "created folders are taken back"
    );
}
