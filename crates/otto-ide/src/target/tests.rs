use super::*;
use crate::types::{IdeFsEntryKind, IdeSearchItemKind, IdeSearchMode};

#[tokio::test]
async fn local_tree_read_write_conflict_and_mutate_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let root_str = tmp.path().to_string_lossy().into_owned();
    let mgr = TargetManager::new();
    let spec = IdeTargetSpec::local(&root_str);
    let target = mgr.resolve_local(&spec, true).unwrap();

    let platform = mgr.detect_platform(&target).await.unwrap();
    assert!(!platform.os.is_empty());
    assert!(!platform.arch.is_empty());

    // Write a new file with create_dirs:
    let written = mgr
        .write_file(
            &target,
            "src/calc.go",
            "package calc\n\nfunc Add(a, b int) int { return a + b }\n",
            None,
            true,
        )
        .await
        .unwrap();
    assert_eq!(written.size, 54);

    // Read back and verify sha256 + language + line_count:
    let read = mgr.read_file(&target, "src/calc.go").await.unwrap();
    assert_eq!(read.language, "go");
    assert_eq!(read.sha256, written.sha256);
    assert_eq!(read.line_count, 3);

    // Stale expected_sha256 must fail with Error::Conflict (HTTP 409):
    let err = mgr
        .write_file(
            &target,
            "src/calc.go",
            "overwritten",
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            false,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Conflict(_)),
        "expected Conflict, got {err:?}"
    );

    // Matching expected_sha256 succeeds:
    let updated = mgr
        .write_file(
            &target,
            "src/calc.go",
            "package calc\n\nfunc Add(a, b int) int { return a + b + 0 }\n",
            Some(&read.sha256),
            false,
        )
        .await
        .unwrap();
    assert_ne!(updated.sha256, read.sha256);

    // Tree listing shows `src` dir:
    let tree = mgr.list_tree(&target, None, false).await.unwrap();
    assert_eq!(tree.entries.len(), 1);
    assert_eq!(tree.entries[0].name, "src");
    assert_eq!(tree.entries[0].kind, IdeFsEntryKind::Dir);

    // Mutate: duplicate -> rename -> delete:
    mgr.mutate_fs(
        &target,
        IdeFsMutateOp::Duplicate,
        "src/calc.go",
        Some("src/calc_copy.go"),
    )
    .await
    .unwrap();
    mgr.mutate_fs(
        &target,
        IdeFsMutateOp::Rename,
        "src/calc_copy.go",
        Some("src/calc_v2.go"),
    )
    .await
    .unwrap();
    let sub_tree = mgr.list_tree(&target, Some("src"), false).await.unwrap();
    assert_eq!(sub_tree.entries.len(), 2);

    mgr.mutate_fs(&target, IdeFsMutateOp::Delete, "src/calc_v2.go", None)
        .await
        .unwrap();
    let after_del = mgr.list_tree(&target, Some("src"), false).await.unwrap();
    assert_eq!(after_del.entries.len(), 1);
}

#[tokio::test]
async fn non_root_secret_files_and_path_traversal_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join(".env"), "SECRET=1\n").unwrap();
    std::fs::write(tmp.path().join("id_rsa"), "PRIVATE KEY\n").unwrap();
    std::fs::write(tmp.path().join("main.py"), "print('ok')\n").unwrap();

    let mgr = TargetManager::new();
    let spec = IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned());
    let non_root_target = mgr.resolve_local(&spec, false).unwrap();

    // Reading denied secret files as non-root fails with Forbidden:
    assert!(matches!(
        mgr.read_file(&non_root_target, ".env").await.unwrap_err(),
        Error::Forbidden(_)
    ));
    assert!(matches!(
        mgr.read_file(&non_root_target, "id_rsa").await.unwrap_err(),
        Error::Forbidden(_)
    ));
    // Normal source file succeeds:
    assert!(mgr.read_file(&non_root_target, "main.py").await.is_ok());

    // Path traversal escaping workspace root is refused even for root:
    let root_target = mgr.resolve_local(&spec, true).unwrap();
    assert!(mgr.read_file(&root_target, "../outside.txt").await.is_err());
    assert!(mgr
        .write_file(&root_target, "../../escape.txt", "x", None, false)
        .await
        .is_err());
}

#[tokio::test]
async fn search_and_replace_dry_run_and_apply_work() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("pkg")).unwrap();
    std::fs::write(
        tmp.path().join("pkg/service.rs"),
        "pub struct OrderEngine {}\npub fn compute_total() -> i32 { 41 }\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("pkg/helper.rs"),
        "pub fn run() -> i32 { compute_total() + 1 }\n",
    )
    .unwrap();

    let mgr = TargetManager::new();
    let spec = IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned());
    let target = mgr.resolve_local(&spec, true).unwrap();

    let sym_hits = mgr
        .search(
            &target,
            &IdeSearchReq {
                target: spec.clone(),
                query: "OrderEngine".into(),
                mode: IdeSearchMode::Symbols,
                case_sensitive: false,
                regex: false,
                file_glob: None,
                limit: Some(50),
            },
        )
        .await
        .unwrap();
    assert_eq!(sym_hits.items.len(), 1);
    assert_eq!(sym_hits.items[0].kind, IdeSearchItemKind::Symbol);

    // Dry-run replace does not touch disk:
    let preview = mgr
        .replace(
            &target,
            &IdeReplaceReq {
                target: spec.clone(),
                query: "compute_total".into(),
                replacement: "calculate_sum".into(),
                case_sensitive: true,
                regex: false,
                paths: None,
                dry_run: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(preview.files_changed, 2);
    assert_eq!(preview.replacements, 2);
    let unchanged = mgr.read_file(&target, "pkg/service.rs").await.unwrap();
    assert!(unchanged.content.contains("compute_total"));

    // Apply replace updates both files:
    let applied = mgr
        .replace(
            &target,
            &IdeReplaceReq {
                target: spec,
                query: "compute_total".into(),
                replacement: "calculate_sum".into(),
                case_sensitive: true,
                regex: false,
                paths: None,
                dry_run: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(applied.files_changed, 2);
    let after = mgr.read_file(&target, "pkg/service.rs").await.unwrap();
    assert!(after.content.contains("calculate_sum"));
}

#[test]
fn ssh_validation_and_piped_command_builder_reject_unsafe_inputs() {
    let mgr = TargetManager::new();
    let bad_ssh = IdeSshSpec {
        connection_id: "c1".into(),
        name: "bad".into(),
        host: "-oProxyCommand=evil".into(),
        port: 22,
        user: "ubuntu".into(),
        identity_file: None,
        proxy_jump: None,
    };
    let spec = IdeTargetSpec::ssh("c1".into(), "/srv/app");
    assert!(mgr.resolve_with_ssh(&spec, "u1", bad_ssh).is_err());

    let good_ssh = IdeSshSpec {
        connection_id: "c1".into(),
        name: "devbox".into(),
        host: "dev.internal".into(),
        port: 2222,
        user: "ubuntu".into(),
        identity_file: Some("/Users/me/.ssh/id_ed25519".into()),
        proxy_jump: Some("bastion.internal:22".into()),
    };
    let resolved = mgr.resolve_with_ssh(&spec, "u1", good_ssh).unwrap();
    let mut env = BTreeMap::new();
    env.insert("GOFLAGS".into(), "-mod=readonly".into());
    let cmd = mgr
        .build_piped_command(
            &resolved,
            "dlv",
            &["dap".into(), "--listen=127.0.0.1:0".into()],
            Some("/srv/app/cmd"),
            &env,
        )
        .unwrap();
    assert_eq!(cmd.as_std().get_program(), "ssh");

    // Bad env variable key must be rejected:
    let mut bad_env = BTreeMap::new();
    bad_env.insert("BAD;KEY".into(), "1".into());
    assert!(mgr
        .build_piped_command(&resolved, "dlv", &[], None, &bad_env)
        .is_err());
}
