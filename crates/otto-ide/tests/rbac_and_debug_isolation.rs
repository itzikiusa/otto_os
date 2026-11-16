//! End-to-end workspace viewer/editor RBAC, non-root debug session ownership
//! isolation, and collaborative room debug-owner binding tests.

mod common;
mod rbac_netbridge;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use axum::Router;
use common::{
    build_test_app, guest_get_json, non_root_user, public_router_for, router_as,
    router_as_with_write_roots, send_json, send_raw, urlencode, TestIdeCtx,
};
use otto_core::Error;
use otto_ide::dap::DapStartContext;
use otto_ide::types::{IdeBreakpoint, IdeDebugLaunchSpec, IdeStartDebugReq, IdeTargetSpec};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;

async fn start_owned_debug_session(
    ctx: &TestIdeCtx,
    owner_id: &str,
    root: &str,
    script: &str,
) -> String {
    let target = IdeTargetSpec::local(root.to_string());
    let req = IdeStartDebugReq {
        target: target.clone(),
        language: "shell".into(),
        launch: IdeDebugLaunchSpec {
            program: script.to_string(),
            cwd: Some(root.to_string()),
            ..Default::default()
        },
        breakpoints: vec![IdeBreakpoint {
            path: script.to_string(),
            line: 3,
            enabled: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let start_ctx = DapStartContext::new()
        .with_caller_is_root(false)
        .with_owner_user_id(owner_id);
    let (session, mut rx) = ctx
        .ide
        .dap
        .start_session_async(req, start_ctx)
        .expect("start seeded debug session");
    let sid = session.id.clone();
    let owner_owned = owner_id.to_string();
    ctx.ide
        .collab
        .publish_debug_snapshot(Some(&target), Some(&owner_owned), &session);
    let engine = ctx.ide.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(snap) => {
                    engine
                        .collab
                        .publish_debug_snapshot(Some(&target), Some(&owner_owned), &snap);
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });
    sid
}

async fn wait_debug_paused(app: &Router, sid: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let (st, body) = send_json(
            app,
            Method::GET,
            &format!("/ide/debug/sessions/{sid}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "get debug session failed: {body}");
        let sess = &body["session"];
        match sess["state"].as_str().unwrap_or("") {
            "paused" => return sess.clone(),
            "terminated" | "error" => panic!("debug session {sid} exited early: {sess}"),
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for debug session {sid} to pause: {sess}"
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

fn write_bash_debug_fixture(dir: &Path, name: &str) -> String {
    let script = dir.join(name);
    std::fs::write(
        &script,
        "#!/bin/bash\nPORT_VAL=7700\nNEXT_VAL=7701\necho \"READY:$PORT_VAL\"\n",
    )
    .unwrap();
    script.to_string_lossy().into_owned()
}

#[tokio::test]
async fn viewer_rbac_blocks_writes_runs_debug_history_and_unwritable_folder_attach() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_edit = tmp.path().join("ws-edit");
    let ws_view = tmp.path().join("ws-view");
    let outside = tmp.path().join("outside-secret-mod");
    std::fs::create_dir_all(&ws_edit).unwrap();
    std::fs::create_dir_all(&ws_view).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    std::fs::write(ws_view.join("readme.md"), "# Read Only\n").unwrap();
    std::fs::write(outside.join("go.mod"), "module example.com/sec\ngo 1.22\n").unwrap();
    std::fs::write(outside.join("sec.go"), "package sec\nconst Token = \"x\"\n").unwrap();
    std::fs::write(
        ws_edit.join("go.mod"),
        format!(
            "module example.com/app\ngo 1.22\nreplace example.com/sec => {}\n",
            outside.display()
        ),
    )
    .unwrap();
    std::fs::write(ws_edit.join("main.go"), "package main\nfunc main() {}\n").unwrap();

    let (ctx, _) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let app = router_as_with_write_roots(
        &ctx,
        non_root_user("u_alice"),
        vec![ws_edit.clone(), ws_view.clone()],
        vec![ws_edit.clone()],
    );
    let edit_str = ws_edit.to_string_lossy().into_owned();
    let view_str = ws_view.to_string_lossy().into_owned();

    let (st_tree, _) = send_json(
        &app,
        Method::GET,
        &format!("/ide/fs/tree?root={}", urlencode(&view_str)),
        None,
    )
    .await;
    assert_eq!(st_tree, StatusCode::OK);
    let (st_read, _) = send_json(
        &app,
        Method::GET,
        &format!("/ide/fs/file?root={}&path=readme.md", urlencode(&view_str)),
        None,
    )
    .await;
    assert_eq!(st_read, StatusCode::OK);

    let view_script = write_bash_debug_fixture(&ws_view, "view.sh");
    for (method, uri, payload) in [
        (
            Method::PUT,
            "/ide/fs/file",
            json!({
                "target": { "kind": "local", "root": view_str },
                "path": "readme.md",
                "content": "tampered\n",
            }),
        ),
        (
            Method::POST,
            "/ide/fs/mutate",
            json!({
                "target": { "kind": "local", "root": view_str },
                "op": "create_file",
                "path": "new.txt",
            }),
        ),
        (
            Method::POST,
            "/ide/runs",
            json!({
                "target": { "kind": "local", "root": view_str },
                "adhoc": { "name": "viewer-cmd", "runner": "custom", "command": "echo", "args": ["no"] },
            }),
        ),
        (
            Method::POST,
            "/ide/debug/sessions",
            json!({
                "target": { "kind": "local", "root": view_str },
                "language": "shell",
                "launch": { "program": view_script },
            }),
        ),
        (
            Method::POST,
            "/ide/history/snapshot",
            json!({ "root": view_str, "path": "readme.md", "content": "x" }),
        ),
        (
            Method::POST,
            "/ide/history/restore",
            json!({ "root": view_str, "id": "lh_missing" }),
        ),
        (
            Method::POST,
            "/ide/projects/folders",
            json!({ "target": { "kind": "local", "root": edit_str }, "folder_path": view_str }),
        ),
        (
            Method::POST,
            "/ide/projects/folders/bulk",
            json!({ "target": { "kind": "local", "root": edit_str }, "folder_paths": [view_str] }),
        ),
        (
            Method::POST,
            "/ide/projects",
            json!({ "name": "Mixed", "primary_root": edit_str, "folders": [edit_str, view_str] }),
        ),
        (
            Method::PUT,
            "/ide/environments",
            json!({ "target": { "kind": "local", "root": edit_str }, "additional_roots": [view_str] }),
        ),
    ] {
        let (st, body) = send_json(&app, method, uri, Some(payload)).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "expected 403 on {uri}: {body}");
    }

    let outside_file = outside.join("sec.go").to_string_lossy().into_owned();
    let (st_rep, _) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/file?root={}&path={}",
            urlencode(&edit_str),
            urlencode(&outside_file)
        ),
        None,
    )
    .await;
    assert_eq!(st_rep, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn non_root_debug_session_ownership_isolation_and_attach_confinement() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_alice = tmp.path().join("ws-alice");
    let ws_bob = tmp.path().join("ws-bob");
    std::fs::create_dir_all(&ws_alice).unwrap();
    std::fs::create_dir_all(&ws_bob).unwrap();
    let alice_script = write_bash_debug_fixture(&ws_alice, "alice.sh");
    let bob_script = write_bash_debug_fixture(&ws_bob, "bob.sh");

    let (ctx, _) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let app_alice = router_as(&ctx, non_root_user("u_alice"), vec![ws_alice.clone()]);
    let app_bob = router_as(&ctx, non_root_user("u_bob"), vec![ws_bob.clone()]);
    let alice_root = ws_alice.to_string_lossy().into_owned();

    let (st_port, _) = send_json(
        &app_alice,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": alice_root },
            "language": "go",
            "launch": { "program": ".", "attach_port": 2345 },
        })),
    )
    .await;
    assert_eq!(st_port, StatusCode::FORBIDDEN);

    let (st_escape, _) = send_json(
        &app_alice,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": alice_root },
            "language": "shell",
            "launch": { "program": bob_script },
        })),
    )
    .await;
    assert_eq!(st_escape, StatusCode::FORBIDDEN);

    // Sec-P1-2: non-root Local `POST /ide/debug/sessions` is forbidden even inside Alice's workspace:
    let (st_local_dbg_forbid, _) = send_json(
        &app_alice,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": alice_root },
            "language": "shell",
            "launch": { "program": alice_script, "cwd": alice_root },
            "breakpoints": [{ "path": alice_script, "line": 3 }],
        })),
    )
    .await;
    assert_eq!(st_local_dbg_forbid, StatusCode::FORBIDDEN);

    // Underlying DAP adapter guards also enforce non-root attach_port and workspace confinement:
    let non_root_ctx = DapStartContext::new()
        .with_caller_is_root(false)
        .with_owner_user_id("u_alice");
    assert!(matches!(
        ctx.ide
            .dap
            .start_session(
                IdeStartDebugReq {
                    target: IdeTargetSpec::local(alice_root.clone()),
                    language: "go".into(),
                    launch: IdeDebugLaunchSpec {
                        program: ".".into(),
                        attach_port: Some(2345),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                non_root_ctx.clone(),
            )
            .await,
        Err(Error::Forbidden(_))
    ));
    assert!(matches!(
        ctx.ide
            .dap
            .start_session(
                IdeStartDebugReq {
                    target: IdeTargetSpec::local(alice_root.clone()),
                    language: "shell".into(),
                    launch: IdeDebugLaunchSpec {
                        program: bob_script.clone(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                non_root_ctx,
            )
            .await,
        Err(Error::Forbidden(_))
    ));

    let sid = start_owned_debug_session(&ctx, "u_alice", &alice_root, &alice_script).await;
    let _ = wait_debug_paused(&app_alice, &sid).await;

    let (_, list_a) = send_json(&app_alice, Method::GET, "/ide/debug/sessions", None).await;
    assert_eq!(list_a["sessions"].as_array().unwrap().len(), 1);
    let (_, list_b) = send_json(&app_bob, Method::GET, "/ide/debug/sessions", None).await;
    assert!(list_b["sessions"].as_array().unwrap().is_empty());

    let (st_get_b, _) = send_json(
        &app_bob,
        Method::GET,
        &format!("/ide/debug/sessions/{sid}"),
        None,
    )
    .await;
    assert_eq!(st_get_b, StatusCode::FORBIDDEN);

    let (st_ev_b, _, _) = send_raw(
        &app_bob,
        Method::GET,
        &format!("/ide/debug/sessions/{sid}/events"),
    )
    .await;
    assert_eq!(st_ev_b, StatusCode::FORBIDDEN);

    for (method, suffix, payload) in [
        (Method::PUT, "breakpoints", json!({ "breakpoints": [] })),
        (Method::POST, "command", json!({ "action": "step_over" })),
        (
            Method::POST,
            "variables",
            json!({ "variables_reference": 1 }),
        ),
        (
            Method::POST,
            "evaluate",
            json!({ "expression": "PORT_VAL" }),
        ),
    ] {
        let (st, _) = send_json(
            &app_bob,
            method,
            &format!("/ide/debug/sessions/{sid}/{suffix}"),
            Some(payload),
        )
        .await;
        assert_eq!(
            st,
            StatusCode::FORBIDDEN,
            "expected 403 for bob on {suffix}"
        );
    }

    let (st_eval, eval_body) = send_json(
        &app_alice,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/evaluate"),
        Some(json!({ "expression": "PORT_VAL" })),
    )
    .await;
    assert_eq!(st_eval, StatusCode::OK);
    assert!(eval_body["result"].as_str().unwrap_or("").contains("7700"));

    let (st_stop, _) = send_json(
        &app_alice,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/command"),
        Some(json!({ "action": "stop" })),
    )
    .await;
    assert_eq!(st_stop, StatusCode::OK);
}

#[tokio::test]
async fn collab_room_debug_owner_binding_and_guest_co_debug_guards() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_shared = tmp.path().join("ws-shared");
    std::fs::create_dir_all(&ws_shared).unwrap();
    let script_a = write_bash_debug_fixture(&ws_shared, "host.sh");
    let script_b = write_bash_debug_fixture(&ws_shared, "other.sh");
    let shared_root = ws_shared.to_string_lossy().into_owned();

    let (ctx, _) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let app_alice = router_as(&ctx, non_root_user("u_alice"), vec![ws_shared.clone()]);
    let app_bob = router_as(&ctx, non_root_user("u_bob"), vec![ws_shared.clone()]);
    let pub_app = public_router_for(&ctx);

    let sid_alice = start_owned_debug_session(&ctx, "u_alice", &shared_root, &script_a).await;
    let _ = wait_debug_paused(&app_alice, &sid_alice).await;

    let (st_room, room_resp) = send_json(
        &app_alice,
        Method::POST,
        "/ide/collab/rooms",
        Some(json!({
            "target": { "kind": "local", "root": shared_root },
            "title": "Pair Debug",
            "default_role": "viewer",
        })),
    )
    .await;
    assert_eq!(st_room, StatusCode::OK);
    let room_id = room_resp["room"]["id"].as_str().unwrap().to_string();
    let token = room_resp["invite_token"].as_str().unwrap().to_string();
    assert_eq!(room_resp["room"]["active_debug_id"], sid_alice);

    let sid_bob = start_owned_debug_session(&ctx, "u_bob", &shared_root, &script_b).await;
    let _ = wait_debug_paused(&app_bob, &sid_bob).await;
    let _ = send_json(
        &app_bob,
        Method::POST,
        &format!("/ide/debug/sessions/{sid_bob}/command"),
        Some(json!({ "action": "step_over" })),
    )
    .await;

    // The invite token is a header, never a query parameter (E12).
    let (st_query, _) = send_json(
        &pub_app,
        Method::GET,
        &format!(
            "/ide/collab/guest/{room_id}/state?token={}",
            urlencode(&token)
        ),
        None,
    )
    .await;
    assert_eq!(st_query, StatusCode::UNAUTHORIZED);
    let (_, state_body) = guest_get_json(
        &pub_app,
        &format!("/ide/collab/guest/{room_id}/state"),
        &token,
    )
    .await;
    assert_eq!(state_body["room"]["active_debug_id"], sid_alice);
    // Guests don't learn the host's user id or connection details (E11).
    assert_eq!(state_body["room"]["host_user_id"], "");
    assert!(state_body["room"]["target"].get("connection_id").is_none());

    let (_, join_body) = send_json(
        &pub_app,
        Method::POST,
        &format!("/ide/collab/guest/{room_id}/join"),
        Some(json!({ "display_name": "Guest", "token": token })),
    )
    .await;
    let peer_id = join_body["peer_id"].as_str().unwrap().to_string();
    let peer_secret = join_body["peer_secret"].as_str().unwrap().to_string();

    let (st_viewer_dbg, _) = send_json(
        &pub_app,
        Method::POST,
        &format!("/ide/collab/guest/{room_id}/debug-action"),
        Some(json!({
            "peer_id": peer_id,
            "peer_secret": peer_secret,
            "debug_id": sid_alice,
            "action": "step_over",
        })),
    )
    .await;
    assert_eq!(st_viewer_dbg, StatusCode::FORBIDDEN);

    let _ = send_json(
        &app_alice,
        Method::POST,
        &format!("/ide/collab/rooms/{room_id}/role"),
        Some(json!({ "peer_id": peer_id, "role": "editor" })),
    )
    .await;

    for bad_payload in [
        json!({
            "peer_id": peer_id,
            "peer_secret": peer_secret,
            "debug_id": sid_bob,
            "action": "step_over",
        }),
        json!({
            "peer_id": peer_id,
            "peer_secret": peer_secret,
            "debug_id": sid_alice,
            "action": "continue",
            "breakpoints": [{ "path": "host.sh", "line": 3, "condition": "PORT_VAL == 7700" }],
        }),
        json!({
            "peer_id": peer_id,
            "peer_secret": peer_secret,
            "debug_id": sid_alice,
            "action": "continue",
            "breakpoints": [{ "path": "../escape.sh", "line": 1 }],
        }),
    ] {
        let (st_bad, _) = send_json(
            &pub_app,
            Method::POST,
            &format!("/ide/collab/guest/{room_id}/debug-action"),
            Some(bad_payload),
        )
        .await;
        assert_eq!(st_bad, StatusCode::FORBIDDEN);
    }

    let (st_ok, _) = send_json(
        &pub_app,
        Method::POST,
        &format!("/ide/collab/guest/{room_id}/debug-action"),
        Some(json!({
            "peer_id": peer_id,
            "peer_secret": peer_secret,
            "debug_id": sid_alice,
            "action": "step_over",
        })),
    )
    .await;
    assert_eq!(st_ok, StatusCode::OK);

    for (app, sid) in [(&app_alice, &sid_alice), (&app_bob, &sid_bob)] {
        let _ = send_json(
            app,
            Method::POST,
            &format!("/ide/debug/sessions/{sid}/command"),
            Some(json!({ "action": "stop" })),
        )
        .await;
    }
}

/// E1 + E2: a non-root editor never writes git's own files, git wiring or
/// agent config through the IDE — Otto's next fetch / status, the owner's
/// next commit or next agent session would run them unsandboxed — by save,
/// create, rename (into or away) or replace, also through a committed
/// `tools -> .git` symlink; and `$HOME` is never its IDE root. Root keeps
/// ordinary repo work such as editing `.gitattributes`.
#[tokio::test]
async fn non_root_ide_writes_refuse_git_and_agent_control_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("repo");
    std::fs::create_dir_all(ws.join(".git/hooks")).unwrap();
    std::fs::write(ws.join(".git/config"), "[core]\n").unwrap();
    std::fs::write(ws.join("main.go"), "package main\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(ws.join(".git"), ws.join("tools")).unwrap();

    let (ctx, root_app) =
        build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let app = router_as_with_write_roots(
        &ctx,
        non_root_user("u_nr"),
        vec![ws.clone()],
        vec![ws.clone()],
    );
    let target = json!({ "kind": "local", "root": ws.to_string_lossy() });

    for path in [
        ".git/config",
        ".git/hooks/pre-commit",
        "tools/hooks/post-checkout",
        ".gitattributes",
        "sub/.gitmodules",
        ".claude/settings.json",
        "CLAUDE.md",
        ".codex/config.toml",
    ] {
        let (st, body) = send_json(
            &app,
            Method::PUT,
            "/ide/fs/file",
            Some(json!({ "target": target, "path": path, "content": "x\n", "create_dirs": true })),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "save {path}: {body}");
    }
    for (op, path, dest) in [
        ("create_file", ".git/hooks/pre-push", None),
        ("mkdir", "nested/.git", None),
        ("rename", "main.go", Some(".git/hooks/pre-commit")),
        ("rename", ".git", Some("dotgit")),
        ("duplicate", "main.go", Some(".gitattributes")),
    ] {
        let (st, body) = send_json(
            &app,
            Method::POST,
            "/ide/fs/mutate",
            Some(json!({ "target": target, "op": op, "path": path, "dest_path": dest })),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{op} {path}: {body}");
    }
    let (st, body) = send_json(
        &app,
        Method::POST,
        "/ide/replace",
        Some(json!({
            "target": target,
            "query": "core",
            "replacement": "core]\nsshCommand = sh -c evil #",
            "paths": [ws.join(".git/config").to_string_lossy()],
            "dry_run": false,
        })),
    )
    .await;
    assert!(
        st == StatusCode::OK || st == StatusCode::FORBIDDEN,
        "{st}: {body}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join(".git/config")).unwrap(),
        "[core]\n"
    );
    assert!(!ws.join(".git/hooks/pre-commit").exists());
    assert!(!ws.join(".gitattributes").exists());

    // Ordinary files still save, and root keeps editing git wiring.
    let (st, body) = send_json(
        &app,
        Method::PUT,
        "/ide/fs/file",
        Some(json!({ "target": target, "path": "main.go", "content": "package main\n\n" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    let (st, body) = send_json(
        &root_app,
        Method::PUT,
        "/ide/fs/file",
        Some(json!({ "target": target, "path": ".gitattributes", "content": "*.go text\n" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "root edits .gitattributes: {body}");

    // E1: even when a workspace covers it, `$HOME` is not a non-root IDE root.
    if let Ok(home) = std::env::var("HOME") {
        let home_app = router_as(&ctx, non_root_user("u_home"), vec![PathBuf::from(&home)]);
        let (st, _) = send_json(
            &home_app,
            Method::GET,
            &format!("/ide/fs/tree?root={}", urlencode(&home)),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "non-root IDE root at $HOME");
    }
}
