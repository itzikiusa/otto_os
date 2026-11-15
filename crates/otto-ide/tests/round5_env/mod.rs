//! Round 5 integration tests for debug DormantNetBridge restart, non-root
//! environment/venv/loader-env hardening, and `module_go_build_tags` + Run
//! Config wire-parity roundtrips.

use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use axum::Router;
use serde_json::json;
use tokio::net::TcpListener;

use crate::common::{build_test_app, non_root_user, router_as, send_json, urlencode};

#[tokio::test]
async fn debug_restart_reacquires_dormant_netbridge_after_target_exit() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("nb-debug");
    std::fs::create_dir_all(&ws).unwrap();
    let (ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;

    let dummy_svc = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let svc_port = dummy_svc.local_addr().unwrap().port();

    let script = ws.join("probe_nb.sh");
    std::fs::write(
        &script,
        "#!/usr/bin/env bash\necho \"SOCKS=${OTTO_NETBRIDGE_SOCKS5:-missing}\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
    }

    let ws_str = std::fs::canonicalize(&ws)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let script_str = std::fs::canonicalize(&script)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    // Configure an active Network Bridge on this project environment.
    let (st_env, env_resp) = send_json(
        &app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "network_bridge": {
                "enabled": true,
                "host_mappings": [{
                    "from_host": "svc.internal.test",
                    "to_host": "127.0.0.1",
                    "to_port": svc_port
                }]
            }
        })),
    )
    .await;
    assert_eq!(st_env, StatusCode::OK, "put env failed: {env_resp}");

    // Start a real Bash debug session (no breakpoints -> runs to exit and transitions to `terminated`,
    // releasing active NetBridgeLease into `DormantNetBridge`).
    let (st_dbg, dbg_resp) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": "shell",
            "launch": { "program": script_str, "cwd": ws_str }
        })),
    )
    .await;
    assert_eq!(st_dbg, StatusCode::OK, "start debug failed: {dbg_resp}");
    let sid = dbg_resp["session"]["id"].as_str().unwrap().to_string();

    let wait_terminated = |sid: String, app: Router| async move {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let (st, body) = send_json(
                &app,
                Method::GET,
                &format!("/ide/debug/sessions/{sid}"),
                None,
            )
            .await;
            assert_eq!(st, StatusCode::OK);
            if body["session"]["state"] == json!("terminated") {
                return body["session"].clone();
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for debug session {sid} to terminate: {body}"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    };

    let first_snap = wait_terminated(sid.clone(), app.clone()).await;
    let first_out: String = first_snap["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["text"].as_str())
        .collect();
    assert!(
        first_out.contains("SOCKS=socks5h://otto:") && first_out.contains("@127.0.0.1:"),
        "initial debug run must inject OTTO_NETBRIDGE_SOCKS5: {first_out}"
    );

    // Restart the terminated session via `POST /ide/debug/sessions/{id}/command` (`action: "restart"`).
    let (st_restart, restart_resp) = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/command"),
        Some(json!({ "action": "restart" })),
    )
    .await;
    assert_eq!(
        st_restart,
        StatusCode::OK,
        "restart debug failed: {restart_resp}"
    );

    let second_snap = wait_terminated(sid, app).await;
    let second_out: String = second_snap["output"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["text"].as_str())
        .collect();
    assert!(
        second_out.contains("SOCKS=socks5h://otto:") && second_out.contains("@127.0.0.1:"),
        "restarted debug session must reacquire DormantNetBridge and inject OTTO_NETBRIDGE_SOCKS5: {second_out}"
    );

    ctx.ide.shutdown().await;
    drop(dummy_svc);
}

#[tokio::test]
async fn non_root_security_blocks_local_venv_dotenv_escape_and_loader_env_hijack() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("editor-ws");
    let outside = tmp.path().join("outside-secrets");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join(".env"), "LEAKED=1\n").unwrap();
    // `.env` (secret-path denied for non-root) and `app.vars` (allowed in-workspace env file
    // attempting loader/PATH injection):
    std::fs::write(ws.join(".env"), "SECRET_DOTENV=1\n").unwrap();
    std::fs::write(
        ws.join("app.vars"),
        "SAFE_APP_FLAG=enabled\nPATH=/tmp/evil\nDYLD_INSERT_LIBRARIES=/tmp/evil.dylib\nPYTHONPATH=/tmp/evil_py\n",
    )
    .unwrap();

    let (ctx, _root_app) =
        build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;
    let editor = non_root_user("u_editor_r5");
    let editor_app = router_as(&ctx, editor, vec![ws.clone()]);
    let ws_str = std::fs::canonicalize(&ws)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    // 1. Non-root `POST /ide/environments/python-venv` & `/ide/toolchains/python-venv` on Local -> 403!
    for endpoint in [
        "/ide/environments/python-venv",
        "/ide/toolchains/python-venv",
    ] {
        let (st, body) = send_json(
            &editor_app,
            Method::POST,
            endpoint,
            Some(json!({
                "target": { "kind": "local", "root": ws_str },
                "venv_dir": ".venv"
            })),
        )
        .await;
        assert_eq!(
            st,
            StatusCode::FORBIDDEN,
            "non-root local python-venv at {endpoint} must return 403: {body}"
        );
    }

    // 2. Non-root `PUT /ide/environments` with out-of-workspace `dotenv_path` -> 403/400!
    let (st_escape, escape_body) = send_json(
        &editor_app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "dotenv_path": outside.join(".env").to_string_lossy()
        })),
    )
    .await;
    assert!(
        matches!(st_escape, StatusCode::FORBIDDEN | StatusCode::BAD_REQUEST),
        "out-of-workspace dotenv_path must be rejected for non-root: {st_escape} {escape_body}"
    );

    // 3. Non-root `PUT /ide/environments` with dangerous loader/PATH env keys -> rejected!
    for bad_key in [
        "PATH",
        "VIRTUAL_ENV",
        "DYLD_INSERT_LIBRARIES",
        "NODE_OPTIONS",
        "PYTHONPATH",
        "GIT_SSH_COMMAND",
    ] {
        let (st_bad, bad_body) = send_json(
            &editor_app,
            Method::PUT,
            "/ide/environments",
            Some(json!({
                "target": { "kind": "local", "root": ws_str },
                "env_vars": [{ "key": bad_key, "value": "hijack", "secret": false }]
            })),
        )
        .await;
        assert!(
            matches!(st_bad, StatusCode::FORBIDDEN | StatusCode::BAD_REQUEST),
            "non-root env_vars key '{bad_key}' must be rejected: {st_bad} {bad_body}"
        );
    }

    // 4a. `.env` inside workspace is blocked for non-root by `secret_paths::is_denied_file`.
    let (st_secret_dotenv, _) = send_json(
        &editor_app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "dotenv_path": ".env",
            "inherit_shell_env": false
        })),
    )
    .await;
    assert_eq!(st_secret_dotenv, StatusCode::FORBIDDEN);

    // 4b. Valid non-secret in-workspace `app.vars` loads `SAFE_APP_FLAG` while stripping `PATH` / `DYLD_*` / `PYTHONPATH` for non-root!
    let (st_ok, ok_body) = send_json(
        &editor_app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "dotenv_path": "app.vars",
            "inherit_shell_env": false
        })),
    )
    .await;
    assert_eq!(st_ok, StatusCode::OK, "app.vars put failed: {ok_body}");

    let (st_get, get_body) = send_json(
        &editor_app,
        Method::GET,
        &format!("/ide/environments?kind=local&root={}", urlencode(&ws_str)),
        None,
    )
    .await;
    assert_eq!(st_get, StatusCode::OK);
    let eff = &get_body["effective_env"];
    assert_eq!(eff["SAFE_APP_FLAG"], json!("enabled"));
    assert!(eff.get("DYLD_INSERT_LIBRARIES").is_none());
    assert!(eff.get("PYTHONPATH").is_none());
    assert_ne!(eff.get("PATH"), Some(&json!("/tmp/evil")));
}

#[tokio::test]
async fn empty_module_go_build_tags_and_run_config_fields_roundtrip_over_http() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("go-mono");
    let submod = ws.join("submod");
    std::fs::create_dir_all(&submod).unwrap();
    std::fs::write(ws.join("go.mod"), "module example.com/mono\n\ngo 1.23\n").unwrap();
    std::fs::write(
        submod.join("go.mod"),
        "module example.com/submod\n\ngo 1.23\n",
    )
    .unwrap();

    let (ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;
    let ws_str = std::fs::canonicalize(&ws)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let sub_str = std::fs::canonicalize(&submod)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    // 1. `PUT /ide/environments` with `module_go_build_tags: { <submod>: [] }` preserves `[]`
    //    both in the `PUT` response AND subsequent `GET /ide/environments`!
    let mut mod_tags_body = serde_json::Map::new();
    mod_tags_body.insert(sub_str.clone(), json!([]));
    let (st_put, put_resp) = send_json(
        &app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "go_build_tags": ["integration"],
            "module_go_build_tags": mod_tags_body
        })),
    )
    .await;
    assert_eq!(st_put, StatusCode::OK, "put env failed: {put_resp}");
    assert_eq!(put_resp["env"]["go_build_tags"], json!(["integration"]));
    assert_eq!(
        put_resp["env"]["module_go_build_tags"][&sub_str],
        json!([]),
        "PUT /ide/environments response must preserve explicit empty [] module_go_build_tags"
    );

    let (st_get, get_resp) = send_json(
        &app,
        Method::GET,
        &format!("/ide/environments?kind=local&root={}", urlencode(&ws_str)),
        None,
    )
    .await;
    assert_eq!(st_get, StatusCode::OK);
    assert_eq!(
        get_resp["env"]["module_go_build_tags"][&sub_str],
        json!([]),
        "GET /ide/environments must preserve explicit empty [] module_go_build_tags after DB roundtrip"
    );
    assert_eq!(
        ctx.ide
            .environments
            .effective_go_build_tags_for_local_root(&ws_str),
        vec!["integration".to_string()]
    );
    assert!(
        ctx.ide
            .environments
            .effective_go_build_tags_for_local_root(&sub_str)
            .is_empty(),
        "explicit empty [] module override must clear inherited workspace Go build tags"
    );

    // 2. `POST /ide/run-configs` + `GET /ide/run-configs` preserves `coverage`, `compound_configs`, and `toolchain_id`!
    let (st_rc, rc_resp) = send_json(
        &app,
        Method::POST,
        "/ide/run-configs",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "name": "Suite With Coverage",
            "runner": "go_test",
            "coverage": true,
            "toolchain_id": "go-1.23.4",
            "compound_configs": ["rc_alpha", "rc_beta"]
        })),
    )
    .await;
    assert_eq!(st_rc, StatusCode::OK, "upsert run-config failed: {rc_resp}");
    let saved_id = rc_resp["config"]["id"].as_str().unwrap().to_string();
    assert_eq!(rc_resp["config"]["coverage"], json!(true));
    assert_eq!(rc_resp["config"]["toolchain_id"], json!("go-1.23.4"));
    assert_eq!(
        rc_resp["config"]["compound_configs"],
        json!(["rc_alpha", "rc_beta"])
    );

    let (st_list, list_resp) = send_json(
        &app,
        Method::GET,
        &format!("/ide/run-configs?kind=local&root={}", urlencode(&ws_str)),
        None,
    )
    .await;
    assert_eq!(st_list, StatusCode::OK);
    let found = list_resp["configs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == json!(saved_id))
        .expect("saved run config in list");
    assert_eq!(found["coverage"], json!(true));
    assert_eq!(found["toolchain_id"], json!("go-1.23.4"));
    assert_eq!(found["compound_configs"], json!(["rc_alpha", "rc_beta"]));
}
