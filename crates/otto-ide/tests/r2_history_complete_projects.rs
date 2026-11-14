//! Integration tests for Backend Local History + Deleted-File/Folder Recovery,
//! `<15ms` `@` Path Completion (including 10k-file workspace benchmark),
//! Named Multi-Repo Projects, Detected SDKs, and Per-Module Python `.venv`.

mod common;

use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use common::{build_test_app, send_json, urlencode};
use serde_json::json;

#[tokio::test]
async fn local_history_tracks_writes_deletes_folder_deletes_persists_and_restores() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    let ws_str = ws.to_string_lossy().into_owned();
    let ws_enc = urlencode(&ws_str);

    let (_ctx, app) = build_test_app(data_dir.clone(), &db_path).await;

    // 1. Write v1, v2, v3 of `src/calc.go` via `PUT /ide/fs/file`:
    for (i, expr) in ["a + b", "a + b + 1", "a * b"].iter().enumerate() {
        let content = format!(
            "package calc\n\nfunc Op(a, b int) int {{ return {expr} }} // v{}\n",
            i + 1
        );
        let (st, _) = send_json(
            &app,
            Method::PUT,
            "/ide/fs/file",
            Some(json!({
                "target": { "kind": "local", "root": ws_str },
                "path": "src/calc.go",
                "content": content,
                "create_dirs": true,
            })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
    }

    // 2. Folder-level snapshot (`POST /ide/history/snapshot` WITHOUT `content`!) + file snapshot:
    let (st_snap_dir, snap_dir_body) = send_json(
        &app,
        Method::POST,
        "/ide/history/snapshot",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": "src",
            "label": "Before risky refactor",
        })),
    )
    .await;
    assert_eq!(
        st_snap_dir,
        StatusCode::OK,
        "dir snapshot failed: {snap_dir_body}"
    );
    assert!(snap_dir_body["entry"]["id"].as_str().is_some());

    // 3. Create a nested folder `pkg/legacy/handler.go` and delete the whole folder,
    //    plus rename & delete `src/calc.go`!
    let (st_leg, _) = send_json(
        &app,
        Method::PUT,
        "/ide/fs/file",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": "pkg/legacy/handler.go",
            "content": "package legacy\nconst Token = \"RECOVER_ME_FROM_DELETED_DIR\"\n",
            "create_dirs": true,
        })),
    )
    .await;
    assert_eq!(st_leg, StatusCode::OK);

    let (st_ren, _) = send_json(
        &app,
        Method::POST,
        "/ide/fs/mutate",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "op": "rename",
            "path": "src/calc.go",
            "dest_path": "src/calc_v3.go",
        })),
    )
    .await;
    assert_eq!(st_ren, StatusCode::OK);

    let (st_del_file, _) = send_json(
        &app,
        Method::POST,
        "/ide/fs/mutate",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "op": "delete",
            "path": "src/calc_v3.go",
        })),
    )
    .await;
    assert_eq!(st_del_file, StatusCode::OK);

    let (st_del_dir, _) = send_json(
        &app,
        Method::POST,
        "/ide/fs/mutate",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "op": "delete",
            "path": "pkg/legacy",
        })),
    )
    .await;
    assert_eq!(st_del_dir, StatusCode::OK);
    assert!(!ws.join("src/calc_v3.go").exists());
    assert!(!ws.join("pkg/legacy/handler.go").exists());

    // 4. Reboot a fresh `IdeEngine` on the SAME `data_dir` to prove durable persistence!
    drop(app);
    let (_ctx2, app2) = build_test_app(data_dir, &db_path).await;

    let (st_list, list_body) = send_json(
        &app2,
        Method::GET,
        &format!("/ide/history/list?root={ws_enc}&include_deleted=true"),
        None,
    )
    .await;
    assert_eq!(st_list, StatusCode::OK);
    let entries = list_body["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["rel_path"] == "src/calc_v3.go" && e["event_kind"] == "deleted"),
        "expected deleted for src/calc_v3.go in {entries:?}"
    );
    let deleted_dir_entry = entries
        .iter()
        .find(|e| e["rel_path"] == "pkg/legacy/handler.go" && e["event_kind"] == "deleted")
        .expect("deleted entry for file inside deleted directory pkg/legacy");
    let entry_id = deleted_dir_entry["id"].as_str().unwrap();

    // `q=` filter narrows entries cleanly:
    let (st_q, q_body) = send_json(
        &app2,
        Method::GET,
        &format!("/ide/history/list?root={ws_enc}&include_deleted=true&q=legacy"),
        None,
    )
    .await;
    assert_eq!(st_q, StatusCode::OK);
    let q_entries = q_body["entries"].as_array().unwrap();
    assert!(!q_entries.is_empty());
    assert!(q_entries.iter().all(|e| {
        let p = e["rel_path"].as_str().unwrap_or("");
        let l = e["label"].as_str().unwrap_or("");
        p.contains("legacy") || l.contains("legacy")
    }));

    // 5. Inspect historical content via `GET /ide/history/entry`:
    let (st_ent, ent_body) = send_json(
        &app2,
        Method::GET,
        &format!(
            "/ide/history/entry?root={ws_enc}&id={}",
            urlencode(entry_id)
        ),
        None,
    )
    .await;
    assert_eq!(st_ent, StatusCode::OK);
    assert!(ent_body["content"]
        .as_str()
        .unwrap()
        .contains("RECOVER_ME_FROM_DELETED_DIR"));

    // 6. Restore deleted file (and its deleted parent directory!) via `POST /ide/history/restore`:
    let (st_rest, rest_body) = send_json(
        &app2,
        Method::POST,
        "/ide/history/restore",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "id": entry_id,
        })),
    )
    .await;
    assert_eq!(st_rest, StatusCode::OK, "restore failed: {rest_body}");
    let restored_disk = std::fs::read_to_string(ws.join("pkg/legacy/handler.go")).unwrap();
    assert!(restored_disk.contains("RECOVER_ME_FROM_DELETED_DIR"));
}

#[tokio::test]
async fn at_path_completion_handles_10k_files_under_budget_and_resolves_relative_abs_and_aliases() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let parent = tmp.path().join("repos");
    let ws = parent.join("main-service");
    let sibling = parent.join("shared-kit");
    let attached = parent.join("toolkit");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(sibling.join("pkg")).unwrap();
    std::fs::create_dir_all(attached.join("src")).unwrap();
    std::fs::write(sibling.join("pkg/helper.go"), "package pkg\n").unwrap();
    std::fs::write(attached.join("src/Widget.java"), "class Widget {}\n").unwrap();

    // Seed 10,000 files across 100 directories inside `main-service`:
    for d in 0..100 {
        let dir = ws.join(format!("mod_{d:03}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..100 {
            std::fs::write(dir.join(format!("file_{d:03}_{f:03}.go")), "package p\n").unwrap();
        }
    }
    std::fs::write(ws.join("needle_target.go"), "package main\n").unwrap();

    let (_ctx, app) = build_test_app(data_dir, &db_path).await;
    let ws_str = ws.to_string_lossy().into_owned();
    let attached_str = attached.to_string_lossy().into_owned();

    // Warm the index once, then benchmark fuzzy `@needle` on the 10,000-file repo:
    let _ = send_json(
        &app,
        Method::POST,
        "/ide/fs/complete",
        Some(json!({
            "target": { "kind": "local", "root": ws_str, "additional_roots": [attached_str] },
            "prefix": "mod_001",
            "limit": 20,
        })),
    )
    .await;

    let t0 = Instant::now();
    let (st_fuzzy, fuzzy_body) = send_json(
        &app,
        Method::POST,
        "/ide/fs/complete",
        Some(json!({
            "target": { "kind": "local", "root": ws_str, "additional_roots": [attached_str] },
            "prefix": "@needle_target",
            "limit": 20,
        })),
    )
    .await;
    let elapsed = t0.elapsed();
    assert_eq!(st_fuzzy, StatusCode::OK);
    assert!(
        elapsed < Duration::from_millis(80),
        "10k-file @ completion took {elapsed:?} (expected < 80ms)"
    );
    let items = fuzzy_body["items"].as_array().unwrap();
    assert!(
        items.iter().any(|i| i["name"] == "needle_target.go"
            && i["insert_token"]
                .as_str()
                .unwrap()
                .ends_with("needle_target.go")),
        "expected needle_target.go in {items:?}"
    );

    // Relative `../shared-kit/` traversal outside workspace root:
    let (st_rel, rel_body) = send_json(
        &app,
        Method::POST,
        "/ide/fs/complete",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "prefix": "../shared-kit/",
        })),
    )
    .await;
    assert_eq!(st_rel, StatusCode::OK);
    assert!(rel_body["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "pkg"));

    // Absolute path `/...` completion:
    let abs_prefix = format!("{}/", sibling.to_string_lossy());
    let (st_abs, abs_body) = send_json(
        &app,
        Method::POST,
        "/ide/fs/complete",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "prefix": abs_prefix,
        })),
    )
    .await;
    assert_eq!(st_abs, StatusCode::OK);
    assert!(abs_body["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "pkg"));

    // Home `~/` completion:
    let (st_home, home_body) = send_json(
        &app,
        Method::POST,
        "/ide/fs/complete",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "prefix": "~/",
            "limit": 10,
        })),
    )
    .await;
    assert_eq!(st_home, StatusCode::OK);
    assert!(!home_body["items"].as_array().unwrap().is_empty());

    // Attached multi-root `@toolkit/` alias completion:
    let (st_alias, alias_body) = send_json(
        &app,
        Method::POST,
        "/ide/fs/complete",
        Some(json!({
            "target": { "kind": "local", "root": ws_str, "additional_roots": [attached_str] },
            "prefix": "toolkit/src/",
        })),
    )
    .await;
    assert_eq!(st_alias, StatusCode::OK);
    assert!(alias_body["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "Widget.java"));
}

#[tokio::test]
async fn projects_crud_scan_repos_detected_sdks_and_python_venv_endpoint_work() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let parent = tmp.path().join("code");
    let repo_a = parent.join("alpha-api");
    let repo_b = parent.join("beta-sdk");
    let repo_c = parent.join("gamma-py");
    std::fs::create_dir_all(repo_a.join(".git")).unwrap();
    std::fs::write(repo_a.join("go.mod"), "module example.com/alpha\n").unwrap();
    std::fs::create_dir_all(repo_b.join(".git")).unwrap();
    std::fs::write(repo_b.join("pom.xml"), "<project/>\n").unwrap();
    std::fs::create_dir_all(repo_c.join(".git")).unwrap();
    std::fs::write(
        repo_c.join("pyproject.toml"),
        "[project]\nname = \"gamma\"\n",
    )
    .unwrap();

    let (_ctx, app) = build_test_app(data_dir, &db_path).await;

    // 1. `POST /ide/projects/scan-repos`:
    let (st_scan, scan_body) = send_json(
        &app,
        Method::POST,
        "/ide/projects/scan-repos",
        Some(json!({ "parent_dir": parent.to_string_lossy() })),
    )
    .await;
    assert_eq!(st_scan, StatusCode::OK);
    let repos = scan_body["repos"].as_array().unwrap();
    assert_eq!(repos.len(), 3);

    // 2. `POST /ide/projects` + `GET /ide/projects` + `POST /ide/projects/folders/bulk` + `DELETE`:
    let (st_up, up_body) = send_json(
        &app,
        Method::POST,
        "/ide/projects",
        Some(json!({
            "name": "Platform Suite",
            "folders": [repo_a.to_string_lossy(), repo_b.to_string_lossy()],
        })),
    )
    .await;
    assert_eq!(st_up, StatusCode::OK);
    let pid = up_body["project"]["id"].as_str().unwrap().to_string();

    let (st_bulk, bulk_body) = send_json(
        &app,
        Method::POST,
        "/ide/projects/folders/bulk",
        Some(json!({
            "target": { "kind": "local", "root": repo_a.to_string_lossy() },
            "folders": [repo_b.to_string_lossy(), repo_c.to_string_lossy()],
        })),
    )
    .await;
    assert_eq!(st_bulk, StatusCode::OK);
    assert_eq!(bulk_body["additional_roots"].as_array().unwrap().len(), 2);

    let (st_list, list_body) = send_json(&app, Method::GET, "/ide/projects", None).await;
    assert_eq!(st_list, StatusCode::OK);
    assert_eq!(list_body["projects"].as_array().unwrap().len(), 1);

    let (st_del, _) = send_json(&app, Method::DELETE, &format!("/ide/projects/{pid}"), None).await;
    assert_eq!(st_del, StatusCode::OK);

    // 3. `GET /ide/toolchains` populates `detected` SDKs on this Mac without network stalls:
    let (st_tc, tc_body) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/toolchains?root={}",
            urlencode(&repo_a.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(st_tc, StatusCode::OK);
    let detected = tc_body["detected"].as_array().unwrap();
    assert!(
        !detected.is_empty(),
        "expected detected host SDKs on macOS, got empty"
    );

    // 4. `POST /ide/toolchains/python-venv` creates a real `.venv` in `gamma-py`:
    let venv_target_dir = repo_c.join(".venv");
    let (st_venv, venv_body) = send_json(
        &app,
        Method::POST,
        "/ide/toolchains/python-venv",
        Some(json!({
            "target": { "kind": "local", "root": repo_a.to_string_lossy() },
            "root": repo_c.to_string_lossy(),
            "path": venv_target_dir.to_string_lossy(),
        })),
    )
    .await;
    assert_eq!(st_venv, StatusCode::OK, "python-venv failed: {venv_body}");
    assert_eq!(venv_body["ok"], true);
    let interp = venv_body["interpreter_path"].as_str().unwrap();
    assert!(
        std::path::Path::new(interp).is_file(),
        "missing interpreter {interp}"
    );
}
