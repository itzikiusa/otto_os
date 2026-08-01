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
async fn write_text_file_is_guarded_versioned_and_scanned() {
    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;

    let out = eng
        .write_text_file(WS, id, "api/openapi.yaml", "openapi: 3.1.0\n", Some(""))
        .await
        .unwrap();
    assert_eq!(out.path, "api/openapi.yaml");
    assert_eq!(out.size, 15);
    assert!(!out.hash.is_empty());
    assert_eq!(
        std::fs::read_to_string(td.path().join("api/openapi.yaml")).unwrap(),
        "openapi: 3.1.0\n"
    );
    let listing = eng.dir(WS, id, "api").await.unwrap();
    assert!(listing.entries.iter().any(|e| e.path == "api/openapi.yaml"));

    for extension in ["yaml", "yml", "json", "d2", "mmd", "txt", "csv"] {
        let path = format!("formats/artifact.{extension}");
        let written = eng
            .write_text_file(WS, id, &path, extension, Some(""))
            .await
            .unwrap_or_else(|e| panic!(".{extension} must be writable: {e}"));
        assert_eq!(written.path, path);
        assert_eq!(std::fs::read_to_string(td.path().join(&path)).unwrap(), extension);
    }

    let err = eng
        .write_text_file(WS, id, "api/openapi.yaml", "clobber", Some("deadbeef"))
        .await
        .unwrap_err();
    assert!(matches!(err, otto_core::Error::Conflict(_)), "{err:?}");
    eng.write_text_file(WS, id, "api/openapi.yaml", "openapi: 3.1.1\n", Some(&out.hash))
        .await
        .unwrap();

    for bad in ["notes/no.md", "bin/app.exe", "../escape.yaml", ".trash/x.json"] {
        assert!(
            eng.write_text_file(WS, id, bad, "x", None).await.is_err(),
            "text artifact path must be rejected: {bad}"
        );
    }
    let oversized = "x".repeat(4 * 1024 * 1024 + 1);
    let err = eng
        .write_text_file(WS, id, "api/huge.json", &oversized, None)
        .await
        .unwrap_err();
    assert!(matches!(err, otto_core::Error::PayloadTooLarge(_)), "{err:?}");

    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), td.path().join("escape")).unwrap();
        let err = eng
            .write_text_file(WS, id, "escape/nested/openapi.yaml", "x", None)
            .await
            .unwrap_err();
        assert!(matches!(err, otto_core::Error::Forbidden(_)), "{err:?}");
        assert!(!outside.path().join("nested/openapi.yaml").exists());

        let outside_file = outside.path().join("outside.json");
        std::fs::write(&outside_file, "outside must stay unchanged").unwrap();
        std::os::unix::fs::symlink(&outside_file, td.path().join("final-link.json")).unwrap();
        let err = eng
            .write_text_file(WS, id, "final-link.json", "escaped", None)
            .await
            .unwrap_err();
        assert!(matches!(err, otto_core::Error::Forbidden(_)), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(&outside_file).unwrap(),
            "outside must stay unchanged"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_text_file_writes_with_one_hash_yield_exactly_one_conflict() {
    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;
    let old_content = "z".repeat(4 * 1024 * 1024);
    let initial = eng
        .write_text_file(WS, id, "api/concurrent.json", &old_content, Some(""))
        .await
        .unwrap();

    let first = eng.write_text_file(WS, id, "api/concurrent.json", "a", Some(&initial.hash));
    let second = eng.write_text_file(WS, id, "api/concurrent.json", "b", Some(&initial.hash));
    let (a, b) = tokio::join!(first, second);

    let conflicts = [&a, &b]
        .into_iter()
        .filter(|r| matches!(r, Err(otto_core::Error::Conflict(_))))
        .count();
    let successes = [&a, &b].into_iter().filter(|r| r.is_ok()).count();
    assert_eq!(successes, 1, "one writer must win: a={a:?}, b={b:?}");
    assert_eq!(conflicts, 1, "one writer must observe a stale hash: a={a:?}, b={b:?}");
    assert_eq!(
        std::fs::metadata(td.path().join("api/concurrent.json"))
            .unwrap()
            .len(),
        1,
        "the winning write must be atomically complete"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn create_only_text_write_does_not_replace_an_unreadable_existing_file() {
    use std::os::unix::fs::PermissionsExt;

    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;
    let target = td.path().join("api/protected.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, "protected").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o000)).unwrap();

    let result = eng
        .write_text_file(WS, id, "api/protected.json", "replacement", Some(""))
        .await;

    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(result.is_err(), "an unreadable existing file must not look absent");
    assert_eq!(std::fs::read_to_string(target).unwrap(), "protected");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn parent_swap_after_validation_cannot_redirect_text_write_outside_vault() {
    use sha2::{Digest, Sha256};
    use std::io::Write;

    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;
    let parent = td.path().join("api");
    let held_parent = td.path().join("api-held");
    let outside = tempfile::tempdir().unwrap();
    let fifo = parent.join("race.json");
    std::fs::create_dir(&parent).unwrap();
    let status = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success(), "mkfifo failed: {status}");

    let seed = b"existing";
    let expected_hash = format!("{:x}", Sha256::digest(seed));
    let writer_engine = eng.clone();
    let write = tokio::spawn(async move {
        writer_engine
            .write_text_file(WS, id, "api/race.json", "replacement", Some(&expected_hash))
            .await
    });

    // Opening the FIFO for write completes only once the engine has validated
    // the parent and opened the existing leaf for its hash read.
    let fifo_for_open = fifo.clone();
    let fifo_writer = tokio::task::spawn_blocking(move || {
        std::fs::OpenOptions::new().write(true).open(fifo_for_open)
    })
    .await
    .unwrap()
    .unwrap();

    std::fs::rename(&parent, &held_parent).unwrap();
    std::os::unix::fs::symlink(outside.path(), &parent).unwrap();
    tokio::task::spawn_blocking(move || {
        let mut fifo_writer = fifo_writer;
        fifo_writer.write_all(seed).unwrap();
    })
    .await
    .unwrap();

    let result = write.await.unwrap();
    assert!(result.is_ok(), "capability-held parent write failed: {result:?}");
    assert!(
        !outside.path().join("race.json").exists(),
        "a swapped parent symlink redirected the write outside the vault"
    );
    assert_eq!(
        std::fs::read_to_string(held_parent.join("race.json")).unwrap(),
        "replacement"
    );
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
async fn vaults_are_global_across_workspaces() {
    // Vaults are a GLOBAL library (like connections): another workspace sees
    // and can address the same vault — ws_id is provenance, not a boundary.
    let eng = engine().await;
    let (_td, id) = fixture_vault(&eng).await;
    assert!(eng.note("other-ws", id, "index.md").await.is_ok());
    assert_eq!(eng.list("other-ws").await.unwrap().len(), 1);
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
    // Every node attribute is parallel to `paths` and indexes its label table.
    assert_eq!(g.types.len(), g.paths.len());
    assert_eq!(g.services.len(), g.paths.len());
    assert!(g.types.iter().all(|t| (*t as usize) < g.type_labels.len()));
    assert!(g.services.iter().all(|s| (*s as usize) < g.service_labels.len()));
    // Tags are CSR: n + 1 offsets, the last one closing the id list.
    assert_eq!(g.tag_off.len(), g.paths.len() + 1);
    assert_eq!(g.tag_off[g.paths.len()] as usize, g.tag_ids.len());
    assert!(g.tag_off.windows(2).all(|w| w[0] <= w[1]));
    assert!(g.tag_ids.iter().all(|t| (*t as usize) < g.tag_labels.len()));

    // Attributes carry real values: notes take their top-level folder as the
    // service, and the ghost/tag nodes land in their synthetic buckets.
    let svc_of = |path: &str| {
        let i = g.paths.iter().position(|p| p == path).expect(path);
        g.service_labels[g.services[i] as usize].as_str()
    };
    assert_eq!(svc_of("services/auth-api.md"), "services");
    assert_eq!(svc_of("runbooks/deploy.md"), "runbooks");
    let type_of = |path: &str| {
        let i = g.paths.iter().position(|p| p == path).expect(path);
        g.type_labels[g.types[i] as usize].as_str()
    };
    assert_eq!(type_of("services/auth-api.md"), "Service");
    assert!(g
        .paths
        .iter()
        .zip(&g.services)
        .any(|(p, s)| p.starts_with("tag:") && g.service_labels[*s as usize] == SERVICE_TAGS));
    assert!(g
        .paths
        .iter()
        .zip(&g.types)
        .any(|(p, t)| p.starts_with("ghost:") && g.type_labels[*t as usize] == TYPE_GHOST));

    // The auth-api note's own tags ride along as an attribute, independent of
    // the tag NODES the `tags` flag draws.
    let ai = g.paths.iter().position(|p| p == "services/auth-api.md").unwrap();
    let auth_tags: Vec<&str> = (g.tag_off[ai]..g.tag_off[ai + 1])
        .map(|t| g.tag_labels[g.tag_ids[t as usize] as usize].as_str())
        .collect();
    assert!(auth_tags.contains(&"auth"), "{auth_tags:?}");
    assert!(auth_tags.contains(&"security"), "{auth_tags:?}");

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
async fn graph_attributes_survive_pruning_and_fold_type_case() {
    let eng = engine().await;
    let (td, id) = fixture_vault(&eng).await;

    // Two notes whose `type` differs only in casing must share ONE bucket, and
    // a note with no frontmatter type must land in `untyped`.
    std::fs::write(
        td.path().join("services/legacy-api.md"),
        "---\ntype: service\ntitle: Legacy API\ndescription: Old one.\n---\n\nSee [[auth-api]].\n",
    )
    .unwrap();
    std::fs::write(td.path().join("stray.md"), "# Stray\n\nNo frontmatter here.\n").unwrap();
    eng.scan(id).await.unwrap();

    let g = eng.graph(WS, id, &GraphOpts { tags: true, ..Default::default() }).await.unwrap();
    let label_count = |want: &str| {
        g.type_labels.iter().filter(|l| l.eq_ignore_ascii_case(want)).count()
    };
    assert_eq!(label_count("service"), 1, "{:?}", g.type_labels);
    assert!(g.type_labels.iter().any(|l| l == TYPE_UNTYPED), "{:?}", g.type_labels);
    // Root-level notes group under the root service bucket.
    let si = g.paths.iter().position(|p| p == "stray.md").unwrap();
    assert_eq!(g.service_labels[g.services[si] as usize], SERVICE_ROOT);

    // Orphan pruning remaps every parallel array in lockstep, not just paths.
    let pruned = eng
        .graph(WS, id, &GraphOpts { orphans: Some(false), tags: true, ..Default::default() })
        .await
        .unwrap();
    assert!(pruned.paths.len() < g.paths.len());
    assert!(!pruned.paths.contains(&"stray.md".to_string()));
    assert_eq!(pruned.types.len(), pruned.paths.len());
    assert_eq!(pruned.services.len(), pruned.paths.len());
    assert_eq!(pruned.tag_off.len(), pruned.paths.len() + 1);
    assert_eq!(pruned.tag_off[pruned.paths.len()] as usize, pruned.tag_ids.len());
    let ai = pruned.paths.iter().position(|p| p == "services/auth-api.md").unwrap();
    let auth_tags: Vec<&str> = (pruned.tag_off[ai]..pruned.tag_off[ai + 1])
        .map(|t| pruned.tag_labels[pruned.tag_ids[t as usize] as usize].as_str())
        .collect();
    assert!(auth_tags.contains(&"auth"), "{auth_tags:?}");

    // Local mode remaps the same way.
    let lg = eng
        .graph(
            WS,
            id,
            &GraphOpts {
                mode: "local".into(),
                path: Some("services/auth-api.md".into()),
                depth: 1,
                tags: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(lg.types.len(), lg.paths.len());
    assert_eq!(lg.services.len(), lg.paths.len());
    assert_eq!(lg.tag_off.len(), lg.paths.len() + 1);
    assert_eq!(lg.tag_off[lg.paths.len()] as usize, lg.tag_ids.len());
    assert!(lg.tag_ids.iter().all(|t| (*t as usize) < lg.tag_labels.len()));
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
