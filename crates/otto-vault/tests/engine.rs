//! End-to-end engine tests over a real temp-dir vault + in-memory SQLite:
//! register → scan → read/search/backlinks/tags/graph → write/delete/rename
//! (with link rewrite on disk) → OKF validation.

use std::sync::Arc;

use otto_vault::types::*;
use otto_vault::VaultEngine;

const WS: &str = "ws-test";

async fn engine() -> Arc<VaultEngine> {
    let pool = otto_state::db::test_pool().await;
    Arc::new(VaultEngine::new(pool))
}

/// A small OKF-ish fixture bundle with wikilinks, md links, tags, aliases,
/// an attachment, a broken link, and reserved files.
fn write_fixture(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("services")).unwrap();
    std::fs::create_dir_all(root.join("runbooks")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(
        root.join("index.md"),
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Auth API](services/auth-api.md) - Issues JWTs.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("log.md"),
        "# Update Log\n\n## 2026-07-10\n* **Creation**: seeded bundle.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("services/auth-api.md"),
        "---\ntype: Service\ntitle: Auth API\ndescription: Issues and verifies JWTs.\ntags: [auth, security]\naliases: [The Auth Service]\ntimestamp: 2026-07-10T10:00:00Z\n---\n\n# Overview\n\nTrust root for [Orders API](/services/orders-api.md) and [[deploy|Deploy Runbook]].\nDiagram: ![[assets/arch.png]] and a broken [[Missing Note]].\nAlso #jwt inline.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("services/orders-api.md"),
        "---\ntype: Service\ntitle: Orders API\ndescription: Takes orders.\ntags: [orders]\ntimestamp: 2026-07-10T10:00:00Z\n---\n\nCalls [auth](auth-api.md) before charging.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("runbooks/deploy.md"),
        "---\ntype: Runbook\ntitle: Deploy Runbook\ndescription: How we ship.\ntimestamp: 2026-07-10T10:00:00Z\n---\n\nShip [[auth-api]] carefully. #oncall\n",
    )
    .unwrap();
    std::fs::write(root.join("assets/arch.png"), [137u8, 80, 78, 71]).unwrap();
}

async fn fixture_vault(eng: &Arc<VaultEngine>) -> (tempfile::TempDir, i64) {
    let td = tempfile::tempdir().unwrap();
    write_fixture(td.path());
    let v = eng
        .register(WS, "Test Vault", Some(td.path().to_string_lossy().to_string()), true)
        .await
        .unwrap();
    eng.scan(v.id).await.unwrap();
    (td, v.id)
}

#[tokio::test(flavor = "multi_thread")]
async fn scan_indexes_notes_links_tags() {
    let eng = engine().await;
    let (_td, id) = fixture_vault(&eng).await;

    let st = eng.store().status(id).await.unwrap();
    assert_eq!(st.notes, 5, "5 md files");
    assert_eq!(st.attachments, 1);
    assert_eq!(st.unresolved, 1, "exactly the [[Missing Note]]");

    let note = eng.note(WS, id, "services/auth-api.md").await.unwrap();
    assert_eq!(note.meta.okf_type.as_deref(), Some("Service"));
    assert_eq!(note.meta.tags, vec!["auth", "security", "jwt"]);
    assert_eq!(note.meta.aliases, vec!["The Auth Service"]);
    let orders = note
        .outgoing
        .iter()
        .find(|l| l.raw_target == "/services/orders-api.md")
        .expect("md link");
    assert_eq!(orders.dst_path.as_deref(), Some("services/orders-api.md"));
    let embed = note.outgoing.iter().find(|l| l.kind == "embed").unwrap();
    assert_eq!(embed.dst_path.as_deref(), Some("assets/arch.png"));

    // Backlinks of orders-api: auth-api links to it.
    let bl = eng.backlinks(WS, id, "services/orders-api.md").await.unwrap();
    assert_eq!(bl.len(), 1);
    assert_eq!(bl[0].path, "services/auth-api.md");
    assert!(bl[0].context.contains("Orders API"), "context: {}", bl[0].context);

    // Tags aggregate.
    let tags = eng.tags(WS, id).await.unwrap();
    let names: Vec<&str> = tags.iter().map(|t| t.tag.as_str()).collect();
    assert!(names.contains(&"auth") && names.contains(&"oncall") && names.contains(&"jwt"));
}

#[tokio::test(flavor = "multi_thread")]
async fn wikilink_basename_resolution() {
    let eng = engine().await;
    let (_td, id) = fixture_vault(&eng).await;
    // [[auth-api]] in runbooks/deploy.md resolves by unique basename.
    let deploy = eng.note(WS, id, "runbooks/deploy.md").await.unwrap();
    let l = deploy.outgoing.iter().find(|l| l.raw_target == "auth-api").unwrap();
    assert_eq!(l.dst_path.as_deref(), Some("services/auth-api.md"));
}

#[tokio::test(flavor = "multi_thread")]
async fn dir_listing_and_switcher_and_search() {
    let eng = engine().await;
    let (_td, id) = fixture_vault(&eng).await;

    let root = eng.dir(WS, id, "").await.unwrap();
    let names: Vec<(&str, &str)> =
        root.entries.iter().map(|e| (e.kind.as_str(), e.name.as_str())).collect();
    assert!(names.contains(&("dir", "services")));
    assert!(names.contains(&("note", "index.md")));
    let services = eng.dir(WS, id, "services").await.unwrap();
    assert_eq!(services.entries.len(), 2);

    // Switcher: title, alias, fuzzy.
    let hits = eng.switcher(WS, id, "auth").await.unwrap();
    assert_eq!(hits[0].path, "services/auth-api.md");
    let alias_hits = eng.switcher(WS, id, "the auth service").await.unwrap();
    assert!(alias_hits.iter().any(|h| h.alias.is_some()), "{alias_hits:?}");

    // FTS search finds body text; reserved files excluded from switcher.
    let res = eng
        .search(WS, id, &SearchReq { query: "charging".into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].path, "services/orders-api.md");

    // Operator filters.
    let res = eng
        .search(WS, id, &SearchReq { query: "tag:oncall".into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].path, "runbooks/deploy.md");
    let res = eng
        .search(WS, id, &SearchReq { query: "type:Runbook".into(), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(res.len(), 1, "{res:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn write_read_conflict_delete_roundtrip() {
    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;

    // Create (with parent folder auto-created).
    let meta = eng
        .write_note(WS, id, "notes/new one.md", "---\ntype: Reference\n---\n\nHello [[auth-api]].\n", Some(""))
        .await
        .unwrap();
    assert_eq!(meta.title, "new one");
    assert!(td.path().join("notes/new one.md").is_file());

    // Its link resolved immediately (write triggers a scan).
    let n = eng.note(WS, id, "notes/new one.md").await.unwrap();
    assert_eq!(n.outgoing[0].dst_path.as_deref(), Some("services/auth-api.md"));

    // Optimistic concurrency: stale hash → Conflict.
    let err = eng
        .write_note(WS, id, "notes/new one.md", "clobber", Some("deadbeef"))
        .await
        .unwrap_err();
    assert!(matches!(err, otto_core::Error::Conflict(_)), "{err:?}");
    // Correct hash → ok.
    eng.write_note(WS, id, "notes/new one.md", "fresh body", Some(&n.meta.hash))
        .await
        .unwrap();

    // Delete → .trash, index row gone, file preserved.
    eng.delete_note(WS, id, "notes/new one.md").await.unwrap();
    assert!(!td.path().join("notes/new one.md").exists());
    assert!(td.path().join(".trash/notes/new one.md").is_file());
    assert!(eng.note(WS, id, "notes/new one.md").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_rewrites_links_on_disk() {
    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;

    let r = eng
        .rename(WS, id, "services/auth-api.md", "services/identity-api.md")
        .await
        .unwrap();
    // orders-api.md (relative md link) + runbooks/deploy.md (basename wikilink)
    // + index.md (root-relative md link).
    assert_eq!(r.links_updated, 3, "{r:?}");

    let orders = std::fs::read_to_string(td.path().join("services/orders-api.md")).unwrap();
    assert!(orders.contains("[auth](identity-api.md)"), "{orders}");
    let deploy = std::fs::read_to_string(td.path().join("runbooks/deploy.md")).unwrap();
    assert!(deploy.contains("[[identity-api]]"), "{deploy}");
    let index = std::fs::read_to_string(td.path().join("index.md")).unwrap();
    assert!(index.contains("services/identity-api.md"), "{index}");

    // Backlinks follow the new path.
    let bl = eng.backlinks(WS, id, "services/identity-api.md").await.unwrap();
    assert_eq!(bl.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_rename_updates_absolute_and_relative_links() {
    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;

    let r = eng.rename(WS, id, "services", "platform").await.unwrap();
    assert!(r.links_updated >= 2, "{r:?}");

    // index.md pointed at services/auth-api.md (relative from root).
    let index = std::fs::read_to_string(td.path().join("index.md")).unwrap();
    assert!(index.contains("platform/auth-api.md"), "{index}");
    // auth-api's own /-absolute link to its sibling must follow the folder.
    let auth = std::fs::read_to_string(td.path().join("platform/auth-api.md")).unwrap();
    assert!(auth.contains("/platform/orders-api.md"), "{auth}");
    // basename wikilink from runbooks still resolves (basename unchanged).
    let deploy = eng.note(WS, id, "runbooks/deploy.md").await.unwrap();
    let l = deploy.outgoing.iter().find(|l| l.raw_target == "auth-api").unwrap();
    assert_eq!(l.dst_path.as_deref(), Some("platform/auth-api.md"));
}

#[tokio::test(flavor = "multi_thread")]
async fn path_traversal_rejected() {
    let eng = engine().await;
    let (_td, id) = fixture_vault(&eng).await;
    for bad in ["../escape.md", "/etc/passwd", "a/../../b.md", ".trash/x.md", "notes/.hidden.md"] {
        assert!(
            eng.note(WS, id, bad).await.is_err(),
            "path must be rejected: {bad}"
        );
        assert!(
            eng.write_note(WS, id, bad, "x", None).await.is_err(),
            "write must be rejected: {bad}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn workspace_scoping() {
    let eng = engine().await;
    let (_td, id) = fixture_vault(&eng).await;
    assert!(eng.note("other-ws", id, "index.md").await.is_err());
    assert!(eng.list("other-ws").await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn graph_full_local_and_flags() {
    let eng = engine().await;
    let (_td, id) = fixture_vault(&eng).await;

    let g = eng
        .graph(WS, id, &GraphOpts { ghosts: true, tags: true, reserved: false, ..Default::default() })
        .await
        .unwrap();
    assert!(!g.truncated);
    // Reserved files excluded by default.
    assert!(!g.paths.iter().any(|p| p == "index.md" || p == "log.md"), "{:?}", g.paths);
    // Ghost node for the unresolved wikilinks.
    assert!(g.flags.iter().any(|f| f & NODE_GHOST != 0));
    // Tag nodes present.
    assert!(g.paths.iter().any(|p| p.starts_with("tag:")));
    // Edges are valid index pairs.
    assert_eq!(g.edges.len() % 2, 0);
    assert!(g.edges.iter().all(|e| (*e as usize) < g.paths.len()));
    // Groups map into labels.
    assert_eq!(g.groups.len(), g.paths.len());
    assert!(g.groups.iter().all(|gi| (*gi as usize) < g.group_labels.len()));

    // Local graph around auth-api at depth 1: itself + direct neighbors.
    let lg = eng
        .graph(
            WS,
            id,
            &GraphOpts {
                mode: "local".into(),
                path: Some("services/auth-api.md".into()),
                depth: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(lg.paths.contains(&"services/auth-api.md".to_string()));
    assert!(lg.paths.contains(&"services/orders-api.md".to_string()));
    assert!(lg.paths.len() < g.paths.len() + 3);

    // Edge budget truncation flag.
    let tg = eng
        .graph(WS, id, &GraphOpts { edge_budget: 1, ..Default::default() })
        .await
        .unwrap();
    assert!(tg.truncated);
    assert_eq!(tg.edges.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn okf_validation_and_indexes() {
    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;

    // Fixture is conformant (E-free); broken link + missing dir indexes warn.
    let rep = eng.okf_validate(WS, id).await.unwrap();
    assert!(rep.conformant, "errors: {:?}", rep.errors);
    assert!(rep.warnings.iter().any(|w| w.rule == "W2"));
    assert!(rep.warnings.iter().any(|w| w.rule == "W4"));

    // Break it: concept without frontmatter (E1), concept without type (E2),
    // index.md with frontmatter (E3), log with a bad date (W5).
    std::fs::write(td.path().join("services/raw.md"), "no frontmatter here\n").unwrap();
    std::fs::write(td.path().join("services/untyped.md"), "---\ntitle: X\n---\nbody\n").unwrap();
    std::fs::write(td.path().join("services/index.md"), "---\ntype: nope\n---\n# S\n").unwrap();
    std::fs::write(
        td.path().join("log.md"),
        "# Log\n\n## July 10th\n* **Update**: bad date.\n",
    )
    .unwrap();
    eng.scan(id).await.unwrap();
    let rep = eng.okf_validate(WS, id).await.unwrap();
    assert!(!rep.conformant);
    let rules: Vec<&str> = rep.errors.iter().map(|e| e.rule.as_str()).collect();
    assert!(rules.contains(&"E1"), "{rules:?}");
    assert!(rules.contains(&"E2"), "{rules:?}");
    assert!(rules.contains(&"E3"), "{rules:?}");
    assert!(rep.warnings.iter().any(|w| w.rule == "W5"));

    // Index generation writes per-dir index.md files with descriptions.
    std::fs::remove_file(td.path().join("services/index.md")).unwrap();
    eng.scan(id).await.unwrap();
    let written = eng.okf_indexes(WS, id).await.unwrap();
    assert!(written >= 3, "root + services + runbooks, got {written}");
    let sidx = std::fs::read_to_string(td.path().join("services/index.md")).unwrap();
    assert!(sidx.contains("[Auth API](auth-api.md) - Issues and verifies JWTs."), "{sidx}");
    let root_idx = std::fs::read_to_string(td.path().join("index.md")).unwrap();
    assert!(root_idx.contains("okf_version"), "{root_idx}");
    assert!(root_idx.contains("[services](services/index.md)"), "{root_idx}");
}

#[tokio::test(flavor = "multi_thread")]
async fn external_edit_is_picked_up_by_rescan() {
    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;
    // Simulate an Obsidian edit on disk.
    std::fs::write(
        td.path().join("services/orders-api.md"),
        "---\ntype: Service\ntitle: Orders API v2\ndescription: Takes orders.\n---\n\nNow links [[deploy]] too.\n",
    )
    .unwrap();
    // Force a different mtime signature (fs timestamp granularity).
    let f = std::fs::OpenOptions::new()
        .append(true)
        .open(td.path().join("services/orders-api.md"))
        .unwrap();
    drop(f);
    eng.scan(id).await.unwrap();
    let n = eng.note(WS, id, "services/orders-api.md").await.unwrap();
    assert_eq!(n.meta.title, "Orders API v2");
    let l = n.outgoing.iter().find(|l| l.raw_target == "deploy").unwrap();
    assert_eq!(l.dst_path.as_deref(), Some("runbooks/deploy.md"));
}
