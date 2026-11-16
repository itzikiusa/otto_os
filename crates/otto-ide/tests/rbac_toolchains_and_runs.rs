//! Integration tests for non-root RBAC & secret-path enforcement, cross-user
//! run/debug/project isolation, managed toolchain install + job poll + delete
//! over HTTP, run-stop and run-config-delete routes, and collab host/guest guards.

mod common;

use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use axum::routing::get;
use axum::Router;
use common::{build_test_app, non_root_user, public_router_for, router_as, send_json, urlencode};
use otto_ide::toolchains_catalog::{CatalogSdkEntry, TargetPlatform};
use otto_ide::types::IdeEcosystemKind;
use serde_json::json;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn non_root_rbac_secret_paths_and_cross_user_isolation_are_enforced() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let ws_alice = tmp.path().join("alice-ws");
    let ws_bob = tmp.path().join("bob-ws");
    std::fs::create_dir_all(&ws_alice).unwrap();
    std::fs::create_dir_all(&ws_bob).unwrap();
    std::fs::write(ws_alice.join("main.go"), "package main\n").unwrap();
    std::fs::write(ws_bob.join("secret.go"), "package bob\n").unwrap();

    let (ctx, app_root) = build_test_app(data_dir.clone(), &db_path).await;
    let app_alice = router_as(&ctx, non_root_user("u_alice"), vec![ws_alice.clone()]);
    let app_bob = router_as(&ctx, non_root_user("u_bob"), vec![ws_bob.clone()]);

    let alice_str = ws_alice.to_string_lossy().into_owned();
    let bob_str = ws_bob.to_string_lossy().into_owned();
    let bob_enc = urlencode(&bob_str);
    let data_enc = urlencode(&data_dir.to_string_lossy());

    // 1. Alice can read/write inside her granted workspace:
    let (st_ok, _) = send_json(
        &app_alice,
        Method::GET,
        &format!("/ide/fs/tree?root={}", urlencode(&alice_str)),
        None,
    )
    .await;
    assert_eq!(st_ok, StatusCode::OK);

    // 2. Alice is rejected with 403 Forbidden on Bob's ungranted workspace:
    let (st_tree_bob, _) = send_json(
        &app_alice,
        Method::GET,
        &format!("/ide/fs/tree?root={bob_enc}"),
        None,
    )
    .await;
    assert_eq!(st_tree_bob, StatusCode::FORBIDDEN);

    let (st_write_bob, _) = send_json(
        &app_alice,
        Method::PUT,
        "/ide/fs/file",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "path": "intrude.txt",
            "content": "nope",
        })),
    )
    .await;
    assert_eq!(st_write_bob, StatusCode::FORBIDDEN);

    let (st_comp_bob, _) = send_json(
        &app_alice,
        Method::POST,
        "/ide/fs/complete",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "prefix": "",
        })),
    )
    .await;
    assert_eq!(st_comp_bob, StatusCode::FORBIDDEN);

    // 3. Protected secret paths (`data_dir`, `~/.ssh`) and root-only endpoints return 403:
    let (st_data, _) = send_json(
        &app_alice,
        Method::GET,
        &format!("/ide/fs/tree?root={data_enc}"),
        None,
    )
    .await;
    assert_eq!(st_data, StatusCode::FORBIDDEN);

    if let Some(ssh_dir) = dirs::home_dir().map(|h| h.join(".ssh")) {
        let (st_ssh, _) = send_json(
            &app_alice,
            Method::GET,
            &format!(
                "/ide/fs/tree?root={}",
                urlencode(&ssh_dir.to_string_lossy())
            ),
            None,
        )
        .await;
        assert_eq!(st_ssh, StatusCode::FORBIDDEN);
    }

    let (st_shell, _) = send_json(&app_alice, Method::GET, "/ide/shell-env", None).await;
    assert_eq!(st_shell, StatusCode::FORBIDDEN);
    let (st_shell_rel, _) =
        send_json(&app_alice, Method::POST, "/ide/shell-env/reload", None).await;
    assert_eq!(st_shell_rel, StatusCode::FORBIDDEN);
    let (st_plug, _) = send_json(
        &app_alice,
        Method::POST,
        "/ide/plugins/install",
        Some(json!({ "source": "https://example.com/plugin" })),
    )
    .await;
    assert_eq!(st_plug, StatusCode::FORBIDDEN);
    let (st_tc_del, _) = send_json(
        &app_alice,
        Method::DELETE,
        "/ide/toolchains/local?id=sdk_any",
        None,
    )
    .await;
    assert_eq!(st_tc_del, StatusCode::FORBIDDEN);

    // 4. Non-root Sec-P1-2 guards: dangerous env/SDK/runner overrides & local execution are forbidden,
    //    while safe standard runner configs + Go build tags succeed and stay isolated across users:
    for bad_rc in [
        json!({
            "target": { "kind": "local", "root": bob_str },
            "name": "Bob Arbitrary Cmd",
            "runner": "shell",
            "command": "echo bob",
            "args": [],
        }),
        json!({
            "target": { "kind": "local", "root": bob_str },
            "name": "Bob Hook",
            "runner": "go_test",
            "before_launch": "sh -c id",
            "args": ["./..."],
        }),
        json!({
            "target": { "kind": "local", "root": bob_str },
            "name": "Bob Loader",
            "runner": "go_test",
            "env_overrides": { "DYLD_INSERT_LIBRARIES": "/tmp/evil.dylib" },
            "args": ["./..."],
        }),
        json!({
            "target": { "kind": "local", "root": bob_str },
            "name": "Bob Escape Cwd",
            "runner": "go_test",
            "cwd": "../alice-ws",
            "args": ["./..."],
        }),
    ] {
        let (st_bad_rc, body_bad_rc) =
            send_json(&app_bob, Method::POST, "/ide/run-configs", Some(bad_rc)).await;
        assert_eq!(
            st_bad_rc,
            StatusCode::FORBIDDEN,
            "expected 403 for non-root dangerous run-config: {body_bad_rc}"
        );
    }

    for bad_env in [
        json!({
            "target": { "kind": "local", "root": bob_str },
            "java_home": "/tmp/fake-jdk",
        }),
        json!({
            "target": { "kind": "local", "root": bob_str },
            "extra_path": "/tmp/evil-bin",
        }),
        json!({
            "target": { "kind": "local", "root": bob_str },
            "env_vars": [{ "key": "LD_PRELOAD", "value": "/tmp/evil.so", "secret": false }],
        }),
    ] {
        let (st_bad_env, body_bad_env) =
            send_json(&app_bob, Method::PUT, "/ide/environments", Some(bad_env)).await;
        assert_eq!(
            st_bad_env,
            StatusCode::FORBIDDEN,
            "expected 403 for non-root dangerous env override: {body_bad_env}"
        );
    }

    // Item B Go build tags validation & persistence over HTTP:
    let (st_bad_tag, _) = send_json(
        &app_bob,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "go_build_tags": ["-exec=/bin/sh"],
        })),
    )
    .await;
    assert_eq!(st_bad_tag, StatusCode::BAD_REQUEST);

    let (st_ok_tags, ok_tags_body) = send_json(
        &app_bob,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "go_build_tags": ["component", "integration"],
            "module_go_build_tags": { &bob_str: ["e2e"] },
        })),
    )
    .await;
    assert_eq!(
        st_ok_tags,
        StatusCode::OK,
        "save go_build_tags: {ok_tags_body}"
    );
    assert_eq!(
        ok_tags_body["env"]["go_build_tags"],
        json!(["component", "integration"])
    );
    assert_eq!(
        ok_tags_body["env"]["module_go_build_tags"][&bob_str],
        json!(["e2e"])
    );

    // Local run-config creation requires `root_authority(user)`:
    let (st_cfg_nonroot, _) = send_json(
        &app_bob,
        Method::POST,
        "/ide/run-configs",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "name": "Bob Go Tests",
            "runner": "go_test",
            "args": ["./..."],
            "coverage": true,
        })),
    )
    .await;
    assert_eq!(st_cfg_nonroot, StatusCode::FORBIDDEN);

    let (st_cfg, cfg_body) = send_json(
        &app_root,
        Method::POST,
        "/ide/run-configs",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "name": "Bob Go Tests",
            "runner": "go_test",
            "args": ["./..."],
            "coverage": true,
        })),
    )
    .await;
    assert_eq!(
        st_cfg,
        StatusCode::OK,
        "root upsert config failed: {cfg_body}"
    );
    assert_eq!(cfg_body["config"]["coverage"], true);
    let bob_cfg_id = cfg_body["config"]["id"].as_str().unwrap().to_string();

    let (st_no_root, _) = send_json(
        &app_alice,
        Method::DELETE,
        &format!("/ide/run-configs/{bob_cfg_id}"),
        None,
    )
    .await;
    assert_eq!(st_no_root, StatusCode::FORBIDDEN);

    let (st_bob_root, _) = send_json(
        &app_alice,
        Method::DELETE,
        &format!("/ide/run-configs/{bob_cfg_id}?root={bob_enc}"),
        None,
    )
    .await;
    assert_eq!(st_bob_root, StatusCode::FORBIDDEN);

    // 5. Non-root Local process execution (`POST /ide/runs` & `POST /ide/deps/action`) is forbidden,
    //    and cross-user run & named-project isolation + SQLite persistence hold across restart:
    let (st_non_root_run, _) = send_json(
        &app_bob,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "config_id": bob_cfg_id,
        })),
    )
    .await;
    assert_eq!(st_non_root_run, StatusCode::FORBIDDEN);

    let (st_non_root_deps, _) = send_json(
        &app_bob,
        Method::POST,
        "/ide/deps/action",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "path": format!("{bob_str}/go.mod"),
            "action": "tidy",
        })),
    )
    .await;
    assert_eq!(st_non_root_deps, StatusCode::FORBIDDEN);

    let (st_run, run_body) = send_json(
        &app_root,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "adhoc": {
                "name": "Root Sleep",
                "runner": "shell",
                "command": "sh",
                "args": ["-c", "sleep 5"],
            }
        })),
    )
    .await;
    assert_eq!(st_run, StatusCode::OK);
    let bob_run_id = run_body["run"]["id"].as_str().unwrap().to_string();

    let (st_alice_get_run, _) = send_json(
        &app_alice,
        Method::GET,
        &format!("/ide/runs/{bob_run_id}"),
        None,
    )
    .await;
    assert_eq!(st_alice_get_run, StatusCode::FORBIDDEN);

    let (st_alice_stop_run, _) = send_json(
        &app_alice,
        Method::POST,
        &format!("/ide/runs/{bob_run_id}/stop"),
        None,
    )
    .await;
    assert_eq!(st_alice_stop_run, StatusCode::FORBIDDEN);

    let _ = send_json(
        &app_root,
        Method::POST,
        &format!("/ide/runs/{bob_run_id}/stop"),
        None,
    )
    .await;

    let (st_proj, proj_body) = send_json(
        &app_bob,
        Method::POST,
        "/ide/projects",
        Some(json!({
            "name": "Bob Private Suite",
            "folders": [bob_str],
        })),
    )
    .await;
    assert_eq!(st_proj, StatusCode::OK, "bob project failed: {proj_body}");
    let bob_proj_id = proj_body["project"]["id"].as_str().unwrap().to_string();

    let (st_alice_projs, alice_projs) =
        send_json(&app_alice, Method::GET, "/ide/projects", None).await;
    assert_eq!(st_alice_projs, StatusCode::OK);
    assert!(
        alice_projs["projects"].as_array().unwrap().is_empty(),
        "Alice must not see Bob's projects: {alice_projs}"
    );

    let (st_alice_del_proj, _) = send_json(
        &app_alice,
        Method::DELETE,
        &format!("/ide/projects/{bob_proj_id}"),
        None,
    )
    .await;
    assert_eq!(st_alice_del_proj, StatusCode::FORBIDDEN);

    // Reboot fresh IdeEngine on same SQLite DB and verify Bob's project persisted:
    let (ctx2, _) = build_test_app(data_dir, &db_path).await;
    let app_bob2 = router_as(&ctx2, non_root_user("u_bob"), vec![ws_bob]);
    let (st_bob_projs, bob_projs) = send_json(&app_bob2, Method::GET, "/ide/projects", None).await;
    assert_eq!(st_bob_projs, StatusCode::OK);
    assert_eq!(bob_projs["projects"].as_array().unwrap().len(), 1);
    assert_eq!(bob_projs["projects"][0]["id"], bob_proj_id);

    let (st_bob_env, bob_env) = send_json(
        &app_bob2,
        Method::GET,
        &format!("/ide/environments?root={bob_enc}"),
        None,
    )
    .await;
    assert_eq!(st_bob_env, StatusCode::OK);
    assert_eq!(
        bob_env["env"]["go_build_tags"],
        json!(["component", "integration"])
    );
    assert_eq!(
        bob_env["env"]["module_go_build_tags"][&bob_str],
        json!(["e2e"])
    );
    let saved_cfg = bob_env["configs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == bob_cfg_id)
        .expect("reloaded bob_cfg_id");
    assert_eq!(saved_cfg["coverage"], true);
}

#[tokio::test]
async fn managed_toolchain_install_job_poll_and_delete_local_sdk_flow_over_http() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    let ws_str = ws.to_string_lossy().into_owned();
    let ws_enc = urlencode(&ws_str);

    // Build a real minimal Go SDK `.tar.gz` archive with an executable `go/bin/go` + `go/VERSION`:
    let stage = tmp.path().join("sdk-stage");
    let go_bin_dir = stage.join("go/bin");
    std::fs::create_dir_all(&go_bin_dir).unwrap();
    std::fs::write(stage.join("go/VERSION"), "go1.99.0\n").unwrap();
    let go_bin = go_bin_dir.join("go");
    std::fs::write(
        &go_bin,
        "#!/bin/sh\necho \"go version go1.99.0 darwin/arm64\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&go_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let archive_path = tmp.path().join("go-1.99.0.tar.gz");
    let tar_status = tokio::process::Command::new("tar")
        .args([
            "-czf",
            &archive_path.to_string_lossy(),
            "-C",
            &stage.to_string_lossy(),
            "go",
        ])
        .status()
        .await
        .unwrap();
    assert!(tar_status.success(), "tar failed building test SDK archive");
    let archive_bytes = std::fs::read(&archive_path).unwrap();
    let sha_hex = hex::encode(Sha256::digest(&archive_bytes));

    // Serve the archive from a local ephemeral HTTP server:
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let archive_router = Router::new().route(
        "/go-1.99.0.tar.gz",
        get({
            let bytes = archive_bytes.clone();
            move || {
                let b = bytes.clone();
                async move { b }
            }
        }),
    );
    let server_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, archive_router).await;
    });

    let (ctx, app) = build_test_app(data_dir.clone(), &db_path).await;
    let host_plat = TargetPlatform::host();
    ctx.ide.toolchains.seed_test_catalog_entry(
        host_plat,
        IdeEcosystemKind::Go,
        CatalogSdkEntry {
            version: "1.99.0".into(),
            label: "Go 1.99.0".into(),
            lts: true,
            os: host_plat.os.as_str().into(),
            arch: host_plat.arch.as_str().into(),
            url: format!("http://127.0.0.1:{port}/go-1.99.0.tar.gz"),
            size_hint: None,
            sha256: Some(sha_hex),
        },
    );

    // 1. `POST /ide/toolchains/install`:
    let (st_inst, inst_body) = send_json(
        &app,
        Method::POST,
        "/ide/toolchains/install",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "ecosystem": "go",
            "component": "sdk",
            "version": "1.99.0",
        })),
    )
    .await;
    assert_eq!(st_inst, StatusCode::OK, "install start failed: {inst_body}");
    let job_id = inst_body["job"]["id"].as_str().unwrap().to_string();

    // 2. Poll `GET /ide/toolchains/jobs/{job_id}` until `done`:
    let deadline = Instant::now() + Duration::from_secs(15);
    let installed_path = loop {
        let (st_job, job_body) = send_json(
            &app,
            Method::GET,
            &format!("/ide/toolchains/jobs/{job_id}"),
            None,
        )
        .await;
        assert_eq!(st_job, StatusCode::OK);
        match job_body["job"]["status"].as_str().unwrap_or("") {
            "done" => {
                break job_body["job"]["installed_path"]
                    .as_str()
                    .unwrap()
                    .to_string();
            }
            "error" => panic!("toolchain install job failed: {job_body}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "toolchain install job timed out");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    server_handle.abort();
    assert!(std::path::Path::new(&installed_path)
        .join("bin/go")
        .is_file());

    // 3. `GET /ide/toolchains` lists the newly installed managed SDK:
    let (st_list, list_body) = send_json(
        &app,
        Method::GET,
        &format!("/ide/toolchains?root={ws_enc}"),
        None,
    )
    .await;
    assert_eq!(st_list, StatusCode::OK);
    let go_eco = list_body["ecosystems"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["ecosystem"] == "go")
        .expect("go ecosystem entry");
    assert!(
        go_eco["installed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["version"] == "1.99.0" && s["source"] == "otto"),
        "expected managed Go 1.99.0 in installed: {go_eco}"
    );

    // 4. `DELETE /ide/toolchains/local` removes the managed SDK directory:
    let (st_del, del_body) = send_json(
        &app,
        Method::DELETE,
        &format!(
            "/ide/toolchains/local?root={ws_enc}&path={}",
            urlencode(&installed_path)
        ),
        None,
    )
    .await;
    assert_eq!(
        st_del,
        StatusCode::OK,
        "delete_local_sdk failed: {del_body}"
    );
    assert!(!std::path::Path::new(&installed_path).exists());
}

#[tokio::test]
async fn run_stop_and_run_config_delete_http_routes_work_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    let ws_str = ws.to_string_lossy().into_owned();
    let ws_enc = urlencode(&ws_str);

    let (_ctx, app) = build_test_app(data_dir, &db_path).await;

    // 1. Upsert a saved run config via `POST /ide/run-configs`:
    let (st_up, up_body) = send_json(
        &app,
        Method::POST,
        "/ide/run-configs",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "name": "Long Worker",
            "runner": "shell",
            "command": "sleep 30",
            "args": [],
        })),
    )
    .await;
    assert_eq!(st_up, StatusCode::OK, "upsert run config failed: {up_body}");
    let cfg_id = up_body["config"]["id"].as_str().unwrap().to_string();

    // 2. Start the run via `POST /ide/runs` and stop it via `POST /ide/runs/{run_id}/stop`:
    let (st_start, start_body) = send_json(
        &app,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "config_id": cfg_id,
        })),
    )
    .await;
    assert_eq!(st_start, StatusCode::OK, "start run failed: {start_body}");
    let run_id = start_body["run"]["id"].as_str().unwrap().to_string();

    let (st_stop, stop_body) = send_json(
        &app,
        Method::POST,
        &format!("/ide/runs/{run_id}/stop"),
        None,
    )
    .await;
    assert_eq!(st_stop, StatusCode::OK, "stop run failed: {stop_body}");
    assert_eq!(stop_body["run"]["status"], "stopped");

    let (st_poll, poll_body) =
        send_json(&app, Method::GET, &format!("/ide/runs/{run_id}"), None).await;
    assert_eq!(st_poll, StatusCode::OK);
    assert_eq!(poll_body["run"]["status"], "stopped");

    // 3. Delete the saved run config via `DELETE /ide/run-configs/{id}` and verify removal:
    let (st_del, del_body) = send_json(
        &app,
        Method::DELETE,
        &format!("/ide/run-configs/{cfg_id}?root={ws_enc}"),
        None,
    )
    .await;
    assert_eq!(
        st_del,
        StatusCode::OK,
        "delete run config failed: {del_body}"
    );
    assert_eq!(del_body["ok"], true);

    let (st_list, list_body) = send_json(
        &app,
        Method::GET,
        &format!("/ide/environments?root={ws_enc}"),
        None,
    )
    .await;
    assert_eq!(st_list, StatusCode::OK);
    assert!(
        !list_body["configs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == cfg_id),
        "deleted run config still present in environments configs list: {list_body}"
    );

    let (st_del_again, _) = send_json(
        &app,
        Method::DELETE,
        &format!("/ide/run-configs/{cfg_id}?root={ws_enc}"),
        None,
    )
    .await;
    assert_eq!(st_del_again, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn collab_room_mints_distinct_host_secret_blocks_host_takeover_and_confines_guest_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let ws = tmp.path().join("collab-ws");
    let outside = tmp.path().join("outside-secret");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(ws.join("hello.txt"), "hello world\n").unwrap();
    std::fs::write(outside.join("token.txt"), "TOP_SECRET\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.join("token.txt"), ws.join("escape_link.txt")).unwrap();

    let (ctx, app) = build_test_app(data_dir, &db_path).await;
    let pub_app = public_router_for(&ctx);
    let ws_str = ws.to_string_lossy().into_owned();

    // 1. Create viewer-default room returns `host_peer_id` + `host_peer_secret` distinct from `invite_token`:
    let (st_create, create_body) = send_json(
        &app,
        Method::POST,
        "/ide/collab/rooms",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "default_role": "viewer",
        })),
    )
    .await;
    assert_eq!(
        st_create,
        StatusCode::OK,
        "create room failed: {create_body}"
    );
    let room_id = create_body["room"]["id"].as_str().unwrap().to_string();
    let invite_token = create_body["invite_token"].as_str().unwrap().to_string();
    let host_peer_id = create_body["host_peer_id"].as_str().unwrap().to_string();
    let host_peer_secret = create_body["host_peer_secret"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!host_peer_id.is_empty());
    assert!(!host_peer_secret.is_empty());
    assert_ne!(host_peer_secret, invite_token);

    // 2. Guest joining with `display_name: "Host"` NEVER acquires `is_host: true`:
    let (st_join, join_body) = send_json(
        &pub_app,
        Method::POST,
        &format!("/ide/collab/guest/{room_id}/join"),
        Some(json!({
            "token": invite_token,
            "display_name": "Host",
        })),
    )
    .await;
    assert_eq!(st_join, StatusCode::OK, "guest join failed: {join_body}");
    let guest_peer_id = join_body["peer_id"].as_str().unwrap().to_string();
    let guest_secret = join_body["peer_secret"].as_str().unwrap().to_string();
    let joined_peer = join_body["room"]["peers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["peer_id"] == guest_peer_id)
        .expect("joined guest peer in room snapshot");
    assert_eq!(joined_peer["is_host"], false);
    assert_eq!(joined_peer["role"], "viewer");

    // 3. Viewer guest cannot edit, nor spoof `host_peer_id` using `invite_token` or `guest_secret`:
    let (st_viewer_edit, _) = send_json(
        &pub_app,
        Method::POST,
        &format!("/ide/collab/guest/{room_id}/edit"),
        Some(json!({
            "peer_id": guest_peer_id,
            "peer_secret": guest_secret,
            "path": "hello.txt",
            "content": "tampered",
            "save": true,
        })),
    )
    .await;
    assert_eq!(st_viewer_edit, StatusCode::FORBIDDEN);

    let (st_spoof_host, _) = send_json(
        &pub_app,
        Method::POST,
        &format!("/ide/collab/guest/{room_id}/edit"),
        Some(json!({
            "peer_id": host_peer_id,
            "peer_secret": invite_token,
            "path": "hello.txt",
            "content": "spoofed",
            "save": true,
        })),
    )
    .await;
    assert_eq!(st_spoof_host, StatusCode::UNAUTHORIZED);

    // 4. Host peer secret CAN edit in-workspace `hello.txt`, and promoting guest to `editor`
    //    still blocks path traversal (`../outside-secret/token.txt`) and symlink escape (`escape_link.txt`):
    let (st_host_edit, host_edit_body) = send_json(
        &pub_app,
        Method::POST,
        &format!("/ide/collab/guest/{room_id}/edit"),
        Some(json!({
            "peer_id": host_peer_id,
            "peer_secret": host_peer_secret,
            "path": "hello.txt",
            "content": "host updated\n",
            "save": true,
        })),
    )
    .await;
    assert_eq!(
        st_host_edit,
        StatusCode::OK,
        "host edit failed: {host_edit_body}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("hello.txt")).unwrap(),
        "host updated\n"
    );

    let (st_promote, _) = send_json(
        &app,
        Method::POST,
        &format!("/ide/collab/rooms/{room_id}/role"),
        Some(json!({
            "peer_id": guest_peer_id,
            "role": "editor",
        })),
    )
    .await;
    assert_eq!(st_promote, StatusCode::OK);

    let (st_trav, _) = send_json(
        &pub_app,
        Method::POST,
        &format!("/ide/collab/guest/{room_id}/edit"),
        Some(json!({
            "peer_id": guest_peer_id,
            "peer_secret": guest_secret,
            "path": "../outside-secret/token.txt",
            "content": "overwrite outside",
            "save": false,
        })),
    )
    .await;
    assert_eq!(st_trav, StatusCode::FORBIDDEN);

    #[cfg(unix)]
    {
        let (st_sym, _) = send_json(
            &pub_app,
            Method::POST,
            &format!("/ide/collab/guest/{room_id}/edit"),
            Some(json!({
                "peer_id": guest_peer_id,
                "peer_secret": guest_secret,
                "path": "escape_link.txt",
                "content": "overwrite via symlink",
                "save": true,
            })),
        )
        .await;
        assert_eq!(st_sym, StatusCode::FORBIDDEN);
        assert_eq!(
            std::fs::read_to_string(outside.join("token.txt")).unwrap(),
            "TOP_SECRET\n"
        );
    }
}
