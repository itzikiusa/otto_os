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
