//! Integration tests for backend Local History + deleted-file/folder recovery,
//! `@` path completion across 10k-file multi-root workspaces, named multi-repo
//! projects, detected SDKs, and per-module Python `.venv` environments.

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

    // 2. Folder-level snapshot (`POST /ide/history/snapshot` without `content`):
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
    //    plus rename & delete `src/calc.go`:
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

    // 4. Reboot a fresh `IdeEngine` on the same `data_dir` to verify durable persistence:
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

    // 6. Restore deleted file (and its deleted parent directory) via `POST /ide/history/restore`:
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

    // Warm the index once, then check fuzzy `@needle` on the 10,000-file repo:
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
        elapsed < Duration::from_millis(500),
        "10k-file @ completion took {elapsed:?} (expected < 500ms)"
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

#[tokio::test]
async fn module_sdks_roundtrip_target_path_runs_and_debug_launch_overrides_work() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let ws = tmp.path().join("ws-root");
    let mod_go = tmp.path().join("mod-go");
    let mod_py = tmp.path().join("mod-py");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&mod_go).unwrap();
    std::fs::create_dir_all(&mod_py).unwrap();
    std::fs::write(
        mod_go.join("go.mod"),
        "module example.com/modgo\n\ngo 1.22\n",
    )
    .unwrap();

    let (ctx, app) = build_test_app(data_dir.clone(), &db_path).await;

    // 1. `POST /ide/environments/python-venv` with optional `root` creates `<mod_py>/.venv`:
    let (st_ev, ev_body) = send_json(
        &app,
        Method::POST,
        "/ide/environments/python-venv",
        Some(json!({
            "target": { "kind": "local", "root": ws.to_string_lossy() },
            "root": mod_py.to_string_lossy(),
            "venv_dir": ".venv",
        })),
    )
    .await;
    assert_eq!(st_ev, StatusCode::OK, "env python-venv failed: {ev_body}");
    let py_interp = ev_body["interpreter_path"].as_str().unwrap().to_string();
    assert!(
        py_interp.contains("mod-py/.venv/bin/python"),
        "expected mod-py venv interpreter, got {py_interp}"
    );
    assert!(std::path::Path::new(&py_interp).is_file());

    // 2. `PUT /ide/environments` + `GET /ide/environments` round-trip `module_sdks`:
    let mod_go_canon = std::fs::canonicalize(&mod_go)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mod_py_canon = std::fs::canonicalize(&mod_py)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let fake_go_sdk = tmp.path().join("custom-go-1.24");
    std::fs::create_dir_all(fake_go_sdk.join("bin")).unwrap();
    let fake_go_str = fake_go_sdk.to_string_lossy().into_owned();

    let (st_put, put_body) = send_json(
        &app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws.to_string_lossy() },
            "additional_roots": [mod_go_canon, mod_py_canon],
            "module_sdks": {
                &mod_go_canon: &fake_go_str,
                &mod_py_canon: &py_interp,
            },
            "env_vars": [
                { "key": "TEAM_NAME", "value": "platform", "secret": false, "has_secret": false }
            ]
        })),
    )
    .await;
    assert_eq!(
        st_put,
        StatusCode::OK,
        "PUT /ide/environments failed: {put_body}"
    );
    assert_eq!(put_body["env"]["module_sdks"][&mod_go_canon], fake_go_str);
    assert_eq!(put_body["env"]["module_sdks"][&mod_py_canon], py_interp);
    assert!(
        !put_body["env"]["env_vars"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["key"] == "__otto_module_sdks__"),
        "reserved key leaked into env_vars: {put_body}"
    );

    // Subsequent PUT with `module_sdks` omitted (`None`) preserves existing `module_sdks`:
    let (st_put2, put2_body) = send_json(
        &app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws.to_string_lossy() },
            "env_vars": [
                { "key": "TEAM_NAME", "value": "core-ide", "secret": false, "has_secret": false }
            ]
        })),
    )
    .await;
    assert_eq!(st_put2, StatusCode::OK);
    assert_eq!(put2_body["env"]["module_sdks"][&mod_go_canon], fake_go_str);

    let (st_get, get_body) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/environments?root={}",
            urlencode(&ws.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(st_get, StatusCode::OK);
    assert_eq!(get_body["env"]["module_sdks"][&mod_py_canon], py_interp);
    assert!(get_body["effective_env"]["__otto_module_sdks__"].is_null());

    // Sync snapshot (`EnvironmentManager::cached_effective_env_for_local_root`) sees module SDK:
    let sync_go_env = ctx
        .ide
        .environments
        .cached_effective_env_for_local_root(&mod_go_canon);
    assert_eq!(
        sync_go_env.get("GOROOT").map(String::as_str),
        Some(fake_go_str.as_str())
    );

    // 3. Ad-hoc `POST /ide/runs` with `target_path` inside `mod_py` (`cwd: None`) uses `module_sdks`:
    let py_probe = mod_py.join("check_venv.py");
    std::fs::write(
        &py_probe,
        "import os, sys\nprint('VENV_OUT=' + os.environ.get('VIRTUAL_ENV', '') + '|' + sys.executable)\n",
    )
    .unwrap();
    let (st_run, run_body) = send_json(
        &app,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": ws.to_string_lossy() },
            "adhoc": {
                "name": "Check mod-py venv",
                "runner": "python",
                "target_path": py_probe.to_string_lossy(),
                "args": [],
            }
        })),
    )
    .await;
    assert_eq!(st_run, StatusCode::OK, "ad-hoc run failed: {run_body}");
    let run_id = run_body["run"]["id"].as_str().unwrap().to_string();
    let mut run_output = String::new();
    for _ in 0..40 {
        let (_, poll) = send_json(&app, Method::GET, &format!("/ide/runs/{run_id}"), None).await;
        if matches!(
            poll["run"]["status"].as_str(),
            Some("passed" | "failed" | "stopped")
        ) {
            run_output = poll["run"]["output"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            break;
        }
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
    assert!(
        run_output.contains("mod-py/.venv"),
        "expected module_sdks VIRTUAL_ENV on target_path run (`cwd: None`), got: {run_output}"
    );

    // 4. `POST /ide/debug/sessions` honors `launch.sdk_path`, `launch.env_file`,
    //    `launch.inherit_shell_env`, and `launch.build_flags`:
    let dbg_script = mod_py.join("dbg_launch.py");
    std::fs::write(
        mod_py.join(".env.debug"),
        "DEBUG_ENV_TOKEN=from_launch_dotenv\n",
    )
    .unwrap();
    std::fs::write(
        &dbg_script,
        "import os, sys\ntok = os.environ.get('DEBUG_ENV_TOKEN', '')\nexe = sys.executable\nprint(f'DBG_PY:{tok}:{exe}', flush=True)\n",
    )
    .unwrap();
    let (st_dbg, dbg_body) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws.to_string_lossy() },
            "language": "python",
            "launch": {
                "program": dbg_script.to_string_lossy(),
                "cwd": mod_py.to_string_lossy(),
                "sdk_path": py_interp,
                "env_file": ".env.debug",
                "inherit_shell_env": false,
            },
            "breakpoints": [
                { "path": dbg_script.to_string_lossy(), "line": 4 }
            ]
        })),
    )
    .await;
    assert_eq!(
        st_dbg,
        StatusCode::OK,
        "python debug start failed: {dbg_body}"
    );
    let sid = dbg_body["session"]["id"].as_str().unwrap().to_string();
    let mut paused = false;
    for _ in 0..50 {
        let (_, sb) = send_json(
            &app,
            Method::GET,
            &format!("/ide/debug/sessions/{sid}"),
            None,
        )
        .await;
        if sb["session"]["state"] == "paused" {
            paused = true;
            let vars = collect_scope_vars(&sb["session"]);
            assert!(
                vars.iter().any(|v| v["name"] == "tok"
                    && v["value"]
                        .as_str()
                        .unwrap_or("")
                        .contains("from_launch_dotenv")),
                "expected launch.env_file var in locals: {vars:?}"
            );
            assert!(
                vars.iter().any(|v| v["name"] == "exe"
                    && v["value"].as_str().unwrap_or("").contains("mod-py/.venv")),
                "expected launch.sdk_path interpreter in sys.executable: {vars:?}"
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    assert!(paused, "expected Python debug session to pause at line 4");
    let _ = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/command"),
        Some(json!({ "action": "continue" })),
    )
    .await;

    // Also verify Go `dlv` honors `launch.build_flags` (`-tags=customdbg`) if `dlv` is installed:
    let home = std::env::var("HOME").unwrap_or_default();
    let dlv_path = [
        format!("{home}/go/bin/dlv"),
        "/opt/homebrew/bin/dlv".into(),
        "/usr/local/bin/dlv".into(),
    ]
    .into_iter()
    .find(|p| std::path::Path::new(p).is_file());
    if dlv_path.is_some() {
        let goroot_out = tokio::process::Command::new("go")
            .args(["env", "GOROOT"])
            .output()
            .await
            .ok();
        let real_goroot = goroot_out
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        std::fs::write(
            mod_go.join("tagged_on.go"),
            "//go:build customdbg\n\npackage main\n\nfunc tagLabel() string { return \"CUSTOM_TAG_ACTIVE\" }\n",
        )
        .unwrap();
        std::fs::write(
            mod_go.join("tagged_off.go"),
            "//go:build !customdbg\n\npackage main\n\nfunc tagLabel() string { return \"DEFAULT_NO_TAG\" }\n",
        )
        .unwrap();
        let go_main = mod_go.join("main.go");
        std::fs::write(
            &go_main,
            "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tlbl := tagLabel()\n\tfmt.Println(\"GO_TAG_RESULT:\" + lbl)\n}\n",
        )
        .unwrap();
        let (st_go, go_body) = send_json(
            &app,
            Method::POST,
            "/ide/debug/sessions",
            Some(json!({
                "target": { "kind": "local", "root": ws.to_string_lossy() },
                "language": "go",
                "launch": {
                    "program": go_main.to_string_lossy(),
                    "cwd": mod_go.to_string_lossy(),
                    "sdk_path": real_goroot,
                    "build_flags": ["-tags=customdbg"],
                },
                "breakpoints": [
                    { "path": go_main.to_string_lossy(), "line": 7 }
                ]
            })),
        )
        .await;
        assert_eq!(st_go, StatusCode::OK, "go debug start failed: {go_body}");
        let gsid = go_body["session"]["id"].as_str().unwrap().to_string();
        let mut go_paused = false;
        for _ in 0..80 {
            let (_, sb) = send_json(
                &app,
                Method::GET,
                &format!("/ide/debug/sessions/{gsid}"),
                None,
            )
            .await;
            if sb["session"]["state"] == "paused" {
                go_paused = true;
                let vars = collect_scope_vars(&sb["session"]);
                assert!(
                    vars.iter().any(|v| v["name"] == "lbl"
                        && v["value"].as_str().unwrap_or("").contains("CUSTOM_TAG_ACTIVE")),
                    "expected -tags=customdbg build flag to select CUSTOM_TAG_ACTIVE, got: {vars:?}"
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(go_paused, "expected Go debug session to pause at line 7");
        let _ = send_json(
            &app,
            Method::POST,
            &format!("/ide/debug/sessions/{gsid}/command"),
            Some(json!({ "action": "continue" })),
        )
        .await;
    }
}

fn collect_scope_vars(sess: &serde_json::Value) -> Vec<&serde_json::Value> {
    sess["scopes"]
        .as_array()
        .map(|s| {
            s.iter()
                .filter_map(|sc| sc["variables"].as_array())
                .flatten()
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn chained_go_selector_hover_and_in_place_rename_usages_work_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let ws = tmp.path().join("workspace");
    let pkg_concurrency = ws.join("concurrency");
    let pkg_ants = ws.join("ants");
    let pkg_http = ws.join("httpclient");
    std::fs::create_dir_all(&pkg_concurrency).unwrap();
    std::fs::create_dir_all(&pkg_ants).unwrap();
    std::fs::create_dir_all(&pkg_http).unwrap();

    std::fs::write(ws.join("go.mod"), "module example.com/demo\n\ngo 1.22\n").unwrap();
    let ants_file = pkg_ants.join("pool.go");
    std::fs::write(
        &ants_file,
        "package ants\n\ntype Pool struct{}\n\n// Submit schedules task f onto the worker pool.\nfunc (p *Pool) Submit(task func()) error {\n\treturn nil\n}\n",
    )
    .unwrap();
    let helper_file = pkg_concurrency.join("helper.go");
    let helper_src = concat!(
        "package concurrency\n\n",
        "import \"example.com/demo/ants\"\n\n",
        "type GoRoutineConcurrencyHelper struct {\n\tpool *ants.Pool\n}\n\n",
        "// Submit submits a task with context.\n",
        "func (g *GoRoutineConcurrencyHelper) Submit(ctx string, f func()) {\n",
        "\terr := g.pool.Submit(f)\n",
        "\t_ = err\n",
        "}\n",
    );
    std::fs::write(&helper_file, helper_src).unwrap();

    let (_ctx, app) = build_test_app(data_dir, &db_path).await;
    let ws_str = ws.to_string_lossy().into_owned();

    // 1. Definition & Hover on `g.pool.Submit(f)` (line 11, col 16) must resolve `ants.Pool.Submit`,
    //    NEVER `GoRoutineConcurrencyHelper.Submit`!
    let (st_def, def_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/definition",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": helper_file.to_string_lossy(),
            "line": 11,
            "column": 16,
            "content": helper_src,
        })),
    )
    .await;
    assert_eq!(st_def, StatusCode::OK, "definition failed: {def_body}");
    let locs = def_body["locations"].as_array().expect("locations");
    assert!(
        !locs.is_empty(),
        "expected ants.Pool.Submit location: {def_body}"
    );
    assert!(
        locs[0]["path"]
            .as_str()
            .unwrap_or("")
            .ends_with("ants/pool.go"),
        "expected first definition in ants/pool.go, got: {locs:?}"
    );

    let (st_hov, hov_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/hover",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": helper_file.to_string_lossy(),
            "line": 11,
            "column": 16,
            "content": helper_src,
        })),
    )
    .await;
    assert_eq!(st_hov, StatusCode::OK, "hover failed: {hov_body}");
    assert_eq!(hov_body["found"], true);
    let sig = hov_body["signature"].as_str().unwrap_or("");
    assert!(
        sig.contains("(p *Pool) Submit(task func()) error"),
        "expected ants.Pool.Submit signature on hover, got: {hov_body}"
    );
    assert!(
        !sig.contains("GoRoutineConcurrencyHelper"),
        "hover must never show enclosing GoRoutineConcurrencyHelper.Submit: {hov_body}"
    );

    // Also verify that if `ants` is from an unindexed external package, `g.extPool.Submit(f)`
    // returns `found: false` instead of falsely falling back to `GoRoutineConcurrencyHelper.Submit`.
    let unindexed_src = helper_src.replace(
        "\"example.com/demo/ants\"",
        "\"github.com/panjf2000/ants/v2\"",
    );
    let (st_ext, ext_hov) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/hover",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": helper_file.to_string_lossy(),
            "line": 11,
            "column": 16,
            "content": unindexed_src,
        })),
    )
    .await;
    assert_eq!(st_ext, StatusCode::OK);
    let ext_sig = ext_hov["signature"].as_str().unwrap_or("");
    assert!(
        !ext_sig.contains("GoRoutineConcurrencyHelper"),
        "unindexed external selector must never fall back to enclosing method: {ext_hov}"
    );

    // 2. In-place `R✎ Rename usages`: disk has `Headers string`, editor buffer already edited
    //    declaration to `aaHeaders string`, and `caller.go` still references `.Headers`.
    let types_file = pkg_http.join("types.go");
    let caller_file = pkg_http.join("caller.go");
    std::fs::write(
        &types_file,
        "package httpclient\n\ntype GenericHttpResponse struct {\n\tHeaders string\n\tBody string\n}\n",
    )
    .unwrap();
    std::fs::write(
        &caller_file,
        "package httpclient\n\nfunc ReadHdr(r GenericHttpResponse) string {\n\treturn r.Headers\n}\n",
    )
    .unwrap();
    let unsaved_types =
        "package httpclient\n\ntype GenericHttpResponse struct {\n\taaHeaders string\n\tBody string\n}\n";

    let (st_ren, ren_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/rename",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": types_file.to_string_lossy(),
            "line": 4,
            "column": 2,
            "symbol": "Headers",
            "content": unsaved_types,
            "new_name": "aaHeaders",
            "apply": true,
        })),
    )
    .await;
    assert_eq!(st_ren, StatusCode::OK, "rename failed: {ren_body}");
    assert_eq!(ren_body["applied"], true);
    assert_eq!(ren_body["edits"].as_array().map(|a| a.len()), Some(2));
    assert!(std::fs::read_to_string(&types_file)
        .unwrap()
        .contains("aaHeaders string"));
    assert!(std::fs::read_to_string(&caller_file)
        .unwrap()
        .contains("return r.aaHeaders"));
}

#[tokio::test]
async fn rename_text_fallback_stays_in_the_language_and_lsp_edits_are_applied_as_given() {
    let tmp = tempfile::tempdir().unwrap();
    let (_ctx, app) =
        build_test_app(tmp.path().join("otto-data"), &tmp.path().join("state.db")).await;
    std::fs::create_dir_all(tmp.path().join("ws")).unwrap();
    let ws = tmp.path().join("ws").canonicalize().unwrap();
    let lib = ws.join("lib.rs");
    let other = ws.join("other.rs");
    std::fs::write(
        &lib,
        "pub fn add(a: i32) -> i32 { a }\nfn main() { add(1); }\n",
    )
    .unwrap();
    std::fs::write(&other, "fn f() { crate::add(2); }\n").unwrap();
    std::fs::write(ws.join("README.md"), "Call add to add.\n").unwrap();
    std::fs::write(ws.join("tool.py"), "def add(x):\n    return x\n").unwrap();
    let ws_str = ws.to_string_lossy().into_owned();
    let rename = |extra: serde_json::Value| {
        let mut body = json!({
            "target": { "kind": "local", "root": ws_str },
            "path": lib.to_string_lossy(),
            "line": 1, "column": 8, "new_name": "sum",
        });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };

    // Text fallback preview: only the two .rs files, labelled as a text match.
    let (st, body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/rename",
        Some(rename(json!({}))),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["source"], "text");
    let mut paths: Vec<String> = body["edits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            e["path"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    paths.sort();
    assert_eq!(paths, ["lib.rs", "other.rs"], "{body}");

    // The language server's edits are applied exactly (only the definition here).
    let lsp_edit = json!([{ "path": lib.to_string_lossy(), "start_line": 1, "start_col": 8,
        "end_line": 1, "end_col": 11, "new_text": "sum" }]);
    let (st, body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/rename",
        Some(rename(json!({ "lsp_edits": lsp_edit, "apply": true }))),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["source"], "lsp");
    assert_eq!(
        body["edits"][0]["preview_lines"][0]["after"],
        "pub fn sum(a: i32) -> i32 { a }"
    );
    assert_eq!(
        std::fs::read_to_string(&lib).unwrap(),
        "pub fn sum(a: i32) -> i32 { a }\nfn main() { add(1); }\n"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("README.md")).unwrap(),
        "Call add to add.\n"
    );
}
