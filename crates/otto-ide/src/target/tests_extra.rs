use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::*;
use crate::types::{IdeFsCompleteReq, IdeReplaceReq, IdeSearchMode, IdeSearchReq};

#[cfg(unix)]
#[tokio::test]
async fn complete_fs_paths_confines_non_root_to_authorized_roots_and_blocks_secret_symlinks() {
    let ws = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let fake_ssh = outside.path().join(".ssh");
    std::fs::create_dir_all(&fake_ssh).unwrap();
    std::fs::write(fake_ssh.join("id_ed25519"), "PRIVATE\n").unwrap();
    std::fs::write(outside.path().join("secret_plan.md"), "TOP SECRET\n").unwrap();
    std::fs::write(ws.path().join("main.rs"), "fn main() {}\n").unwrap();
    std::os::unix::fs::symlink(&fake_ssh, ws.path().join("keys_link")).unwrap();

    let ws_str = ws.path().to_string_lossy().into_owned();
    let outside_str = outside.path().to_string_lossy().into_owned();
    let mut aliases = BTreeMap::new();
    aliases.insert("@ext".into(), outside_str.clone());

    let root_items = complete::complete_fs_paths(
        &IdeFsCompleteReq {
            query: String::new(),
            cwd: Some(ws_str.clone()),
            roots: vec![ws_str.clone()],
            aliases: aliases.clone(),
            limit: 50,
            ..Default::default()
        },
        false,
    )
    .items;
    assert!(
        !root_items
            .iter()
            .any(|i| i.path.contains("keys_link") || i.display.contains("keys_link")),
        "symlink to secret dir must be omitted: {root_items:?}"
    );

    // Every escape vector (`keys_link/`, `~/`, `/outside/`, `../`, `@ext/`) must return empty for non-root:
    for probe in [
        "keys_link/".to_string(),
        "~/".to_string(),
        format!("{outside_str}/"),
        "../".to_string(),
        "@ext/".to_string(),
    ] {
        let escaped = complete::complete_fs_paths(
            &IdeFsCompleteReq {
                query: probe.clone(),
                cwd: Some(ws_str.clone()),
                roots: vec![ws_str.clone()],
                aliases: aliases.clone(),
                limit: 50,
                ..Default::default()
            },
            false,
        )
        .items;
        assert!(
            escaped.is_empty(),
            "non-root completion probe '{probe}' escaped workspace roots: {escaped:?}"
        );
    }
}

#[tokio::test]
async fn multi_root_write_and_disabled_history_preference_work() {
    let data_dir = tempfile::tempdir().unwrap();
    let primary = tempfile::tempdir().unwrap();
    let attached = tempfile::tempdir().unwrap();
    std::fs::write(primary.path().join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(attached.path().join("b.rs"), "fn b() { 1 }\n").unwrap();

    let history = LocalHistoryStore::new(data_dir.path().to_path_buf());
    let mgr = TargetManager::with_history(history.clone());
    let mut spec = IdeTargetSpec::local(primary.path().to_string_lossy().into_owned());
    spec.additional_roots = vec![attached.path().to_string_lossy().into_owned()];
    let target = mgr.resolve_local(&spec, true).unwrap();

    // Writing with `HistoryWriteOpts { enabled: false }` must NOT record snapshots:
    mgr.write_file_with_history(
        &target,
        "a.rs",
        "fn a() { 42 }\n",
        None,
        false,
        HistoryWriteOpts {
            enabled: false,
            retention_days: Some(7),
        },
    )
    .await
    .unwrap();
    let primary_hist = history
        .list_entries(
            &target.local_project_roots()[0],
            &[],
            true,
            None,
            true,
            Some(20),
            None,
        )
        .unwrap();
    assert!(
        primary_hist.entries.is_empty(),
        "disabled Local History must skip snapshot creation"
    );

    // Default `write_file` records snapshots under the matched attached root:
    let attached_file = attached.path().join("b.rs").to_string_lossy().into_owned();
    mgr.write_file(&target, &attached_file, "fn b() { 2 }\n", None, false)
        .await
        .unwrap();
    let listed = history
        .list_entries(
            &target.local_project_roots()[0],
            &target.local_project_roots()[1..],
            true,
            Some(&attached_file),
            true,
            Some(20),
            None,
        )
        .unwrap();
    assert!(listed.entries.iter().any(|e| e.rel_path == "b.rs"));
}

#[tokio::test]
async fn tracked_nested_target_and_build_packages_remain_searchable_while_generated_dirs_skip() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(
        ws.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(ws.path().join("target/debug")).unwrap();
    std::fs::write(
        ws.path().join("target/debug/generated.rs"),
        "pub fn needle_token() {}\n",
    )
    .unwrap();
    std::fs::create_dir_all(ws.path().join("src/target")).unwrap();
    std::fs::write(
        ws.path().join("src/target/handler.rs"),
        "pub fn needle_token() {}\n",
    )
    .unwrap();
    std::fs::create_dir_all(ws.path().join("pkg/build")).unwrap();
    std::fs::write(
        ws.path().join("pkg/build/compile.rs"),
        "pub fn needle_token() {}\n",
    )
    .unwrap();
    std::fs::write(ws.path().join("__debug_bin12345"), "ELF").unwrap();

    let mgr = TargetManager::new();
    let spec = IdeTargetSpec::local(ws.path().to_string_lossy().into_owned());
    let target = mgr.resolve_local(&spec, true).unwrap();

    // `__debug_bin*` is hidden from tree unless `show_hidden == true`:
    let tree_default = mgr.list_tree(&target, None, false).await.unwrap();
    assert!(!tree_default
        .entries
        .iter()
        .any(|e| e.name.starts_with("__debug_bin")));
    let tree_hidden = mgr.list_tree(&target, None, true).await.unwrap();
    assert!(tree_hidden
        .entries
        .iter()
        .any(|e| e.name == "__debug_bin12345"));

    let hits = mgr
        .search(
            &target,
            &IdeSearchReq {
                target: spec,
                query: "needle_token".into(),
                mode: IdeSearchMode::Text,
                case_sensitive: true,
                regex: false,
                whole_word: true,
                file_glob: Some("*.rs".into()),
                scope_path: None,
                limit: Some(50),
            },
        )
        .await
        .unwrap();
    let hit_paths: Vec<&str> = hits.items.iter().map(|i| i.path.as_str()).collect();
    assert!(
        hit_paths
            .iter()
            .any(|p| p.ends_with("src/target/handler.rs")),
        "tracked src/target/handler.rs must be searched: {hit_paths:?}"
    );
    assert!(
        hit_paths
            .iter()
            .any(|p| p.ends_with("pkg/build/compile.rs")),
        "tracked pkg/build/compile.rs must be searched: {hit_paths:?}"
    );
    assert!(
        !hit_paths
            .iter()
            .any(|p| p.ends_with("target/debug/generated.rs")),
        "Cargo build artifact dir target/debug must be skipped: {hit_paths:?}"
    );
}

#[tokio::test]
async fn crlf_line_anchors_and_utf16_columns_work_in_search_and_replace() {
    let ws = tempfile::tempdir().unwrap();
    // "café😀 = " -> 'c'(1)+'a'(1)+'f'(1)+'é'(1)+'😀'(2 surrogate code units)+' '(1)+'='(1)+' '(1) = 9 UTF-16 code units; 'v' starts at 1-based column 10!
    std::fs::write(
        ws.path().join("win.ts"),
        "const a = 1;\r\ncafé😀 = val_end\r\n",
    )
    .unwrap();

    let mgr = TargetManager::new();
    let spec = IdeTargetSpec::local(ws.path().to_string_lossy().into_owned());
    let target = mgr.resolve_local(&spec, true).unwrap();

    let hits = mgr
        .search(
            &target,
            &IdeSearchReq {
                target: spec.clone(),
                query: "val_end$".into(),
                mode: IdeSearchMode::Text,
                case_sensitive: true,
                regex: true,
                whole_word: false,
                file_glob: None,
                scope_path: None,
                limit: Some(20),
            },
        )
        .await
        .unwrap();
    assert_eq!(hits.items.len(), 1, "CRLF line must match `$` anchor");
    assert_eq!(hits.items[0].line, Some(2));
    assert_eq!(hits.items[0].column, Some(10));

    let rep = mgr
        .replace(
            &target,
            &IdeReplaceReq {
                target: spec,
                query: "val_end$".into(),
                replacement: "updated".into(),
                case_sensitive: true,
                regex: true,
                whole_word: false,
                file_glob: None,
                scope_path: None,
                paths: None,
                dry_run: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(rep.replacements, 1);
    let content = std::fs::read_to_string(ws.path().join("win.ts")).unwrap();
    assert_eq!(content, "const a = 1;\r\ncafé😀 = updated\r\n");
}

#[test]
fn non_root_resolve_local_never_admits_unvalidated_parent_go_work_roots() {
    let outer = tempfile::tempdir().unwrap();
    let sub_ws = outer.path().join("subservice");
    let sibling_secret_repo = outer.path().join("billing");
    std::fs::create_dir_all(&sub_ws).unwrap();
    std::fs::create_dir_all(&sibling_secret_repo).unwrap();
    std::fs::write(
        outer.path().join("go.work"),
        "go 1.23\nuse (\n\t./subservice\n\t./billing\n)\n",
    )
    .unwrap();
    std::fs::write(sub_ws.join("go.mod"), "module example.com/sub\ngo 1.23\n").unwrap();
    std::fs::write(
        sibling_secret_repo.join("go.mod"),
        "module example.com/billing\ngo 1.23\n",
    )
    .unwrap();

    let mgr = TargetManager::new();
    let spec = IdeTargetSpec::local(sub_ws.to_string_lossy().into_owned());
    let non_root = mgr.resolve_local(&spec, false).unwrap();
    let sibling_canon = sibling_secret_repo.canonicalize().unwrap();
    let outer_canon = outer.path().canonicalize().unwrap();
    assert!(!non_root.local_project_roots().contains(&sibling_canon));
    assert!(!non_root.local_project_roots().contains(&outer_canon));
    assert!(!non_root.local_readonly_roots().contains(&sibling_canon));
    assert!(!non_root.local_readonly_roots().contains(&outer_canon));
}

#[cfg(unix)]
#[tokio::test]
async fn ssh_eof_kill_wrappers_stay_alive_while_stdin_open_and_terminate_on_eof() {
    // 1. Null-stdin run wrapper: must survive > 1.1s while outer stdin stays open, then exit on EOF.
    let wrapped_run = ssh_exec::wrap_remote_null_stdin_with_eof_kill("echo READY; sleep 30");
    let mut run_child = Command::new("/bin/sh")
        .args(["-c", &wrapped_run])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let run_stdin = run_child.stdin.take().unwrap();
    let mut run_stdout = BufReader::new(run_child.stdout.take().unwrap()).lines();
    let first = tokio::time::timeout(Duration::from_secs(2), run_stdout.next_line())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.as_deref(), Some("READY"));
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        run_child.try_wait().unwrap().is_none(),
        "remote run wrapper must not prematurely SIGTERM while ssh stdin is open"
    );
    drop(run_stdin);
    let t_eof = Instant::now();
    let _ = tokio::time::timeout(Duration::from_secs(3), run_child.wait())
        .await
        .expect("remote run wrapper must terminate promptly when ssh stdin closes");
    assert!(t_eof.elapsed() < Duration::from_secs(3));

    // 2. Stdio FIFO wrapper: forwards stdin frames intact, stays alive > 1.1s, then terminates on EOF.
    let wrapped_stdio =
        ssh_exec::wrap_remote_stdio_with_eof_kill("read line && echo \"ECHO:$line\" && sleep 30");
    let mut stdio_child = Command::new("/bin/sh")
        .args(["-c", &wrapped_stdio])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdio_in = stdio_child.stdin.take().unwrap();
    let mut stdio_out = BufReader::new(stdio_child.stdout.take().unwrap()).lines();
    stdio_in.write_all(b"dap-frame\n").await.unwrap();
    stdio_in.flush().await.unwrap();
    let echoed = tokio::time::timeout(Duration::from_secs(2), stdio_out.next_line())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(echoed.as_deref(), Some("ECHO:dap-frame"));
    tokio::time::sleep(Duration::from_millis(1150)).await;
    assert!(
        stdio_child.try_wait().unwrap().is_none(),
        "stdio wrapper must survive past 1.1s while stdin remains open"
    );
    drop(stdio_in);
    let _ = tokio::time::timeout(Duration::from_secs(3), stdio_child.wait())
        .await
        .expect("stdio wrapper must terminate after stdin closes");
}

#[test]
fn build_remote_exec_line_expands_remote_home_and_preserves_remote_path() {
    let mut env = BTreeMap::new();
    env.insert("GOROOT".into(), "$HOME/.otto/toolchains/go/current".into());
    env.insert("CUSTOM_LITERAL".into(), "$DO_NOT_EXPAND".into());
    let line = ssh_exec::build_remote_exec_line("/srv/app", "go", &["env".into()], &env);
    assert!(
        line.contains("\"$HOME\"/.otto/toolchains/go/current/bin"),
        "expected remote $HOME toolchain bin expansion: {line}"
    );
    assert!(
        line.contains(":\"$PATH\";"),
        "expected remote login $PATH preserved at tail: {line}"
    );
    assert!(
        line.contains("GOROOT=\"$HOME\"/'.otto/toolchains/go/current'")
            || line.contains("GOROOT=\"$HOME\"/.otto/toolchains/go/current"),
        "expected unquoted $HOME prefix in GOROOT: {line}"
    );
    assert!(
        line.contains("CUSTOM_LITERAL='$DO_NOT_EXPAND'"),
        "arbitrary env vars must remain single-quoted: {line}"
    );
}
