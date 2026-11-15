//! End-to-end cross-workspace run-config isolation, path-completion confinement,
//! local-history symlink & preference enforcement, legacy project migration,
//! LSP pool status, and process-run SSE tests.

mod common;

use axum::http::{Method, StatusCode};
use common::{build_test_app, non_root_user, router_as, send_json, send_raw, urlencode};
use otto_ide::types::{IdeRunConfig, IdeRunnerKind, IdeStartRunReq, IdeTargetSpec};
use serde_json::json;

#[tokio::test]
async fn run_config_scope_complete_confinement_history_sse_and_lsp_status() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let ws_alice = tmp.path().join("ws-alice");
    let ws_bob = tmp.path().join("ws-bob");
    std::fs::create_dir_all(&ws_alice).unwrap();
    std::fs::create_dir_all(&ws_bob).unwrap();
    std::fs::write(ws_alice.join("main.go"), "package main\nfunc main() {}\n").unwrap();
    std::fs::write(ws_bob.join("secret.go"), "package bob\n").unwrap();

    let alice_str = ws_alice.to_string_lossy().into_owned();
    let bob_str = ws_bob.to_string_lossy().into_owned();
    std::fs::write(
        data_dir.join("ide-projects.json"),
        serde_json::to_vec(&json!([{
            "id": "proj_legacy_1",
            "name": "Legacy Migrated",
            "primary_root": alice_str,
            "folders": [alice_str],
            "pinned": true,
            "updated_at": "2026-11-14T00:00:00Z"
        }]))
        .unwrap(),
    )
    .unwrap();

    let (ctx, _) = build_test_app(data_dir.clone(), &tmp.path().join("db.sqlite")).await;
    let app_alice = router_as(&ctx, non_root_user("u_alice"), vec![ws_alice.clone()]);
    let app_bob = router_as(&ctx, non_root_user("u_bob"), vec![ws_bob.clone()]);

    let (st_proj, proj_body) = send_json(&app_alice, Method::GET, "/ide/projects", None).await;
    assert_eq!(st_proj, StatusCode::OK);
    assert!(proj_body["projects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "Legacy Migrated"));
    assert!(!data_dir.join("ide-projects.json").exists());

    let (st_rc_custom_forbid, _) = send_json(
        &app_bob,
        Method::POST,
        "/ide/run-configs",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "id": "rc_shared",
            "name": "Bob Custom Blocked",
            "runner": "custom",
            "command": "echo",
            "args": ["bob"],
        })),
    )
    .await;
    assert_eq!(st_rc_custom_forbid, StatusCode::FORBIDDEN);

    let (st_rc_b, _) = send_json(
        &app_bob,
        Method::POST,
        "/ide/run-configs",
        Some(json!({
            "target": { "kind": "local", "root": bob_str },
            "id": "rc_shared",
            "name": "Bob Config",
            "runner": "go_test",
            "args": ["./..."],
        })),
    )
    .await;
    assert_eq!(st_rc_b, StatusCode::OK);

    let (st_rc_a, _) = send_json(
        &app_alice,
        Method::POST,
        "/ide/run-configs",
        Some(json!({
            "target": { "kind": "local", "root": alice_str },
            "id": "rc_shared",
            "name": "Alice Collision",
            "runner": "go_test",
            "args": ["./..."],
        })),
    )
    .await;
    assert_eq!(st_rc_a, StatusCode::FORBIDDEN);

    let (st_del_scoped, _) = send_json(
        &app_alice,
        Method::DELETE,
        &format!("/ide/run-configs/rc_shared?root={}", urlencode(&alice_str)),
        None,
    )
    .await;
    assert_eq!(st_del_scoped, StatusCode::NOT_FOUND);

    let (st_del_unscoped, _) = send_json(
        &app_alice,
        Method::DELETE,
        "/ide/run-configs/rc_shared",
        None,
    )
    .await;
    assert_eq!(st_del_unscoped, StatusCode::FORBIDDEN);

    for bad_query in ["@~/", "@../ws-bob/", &format!("@{bob_str}/"), "@b/"] {
        let (st_c, c_body) = send_json(
            &app_alice,
            Method::POST,
            "/ide/fs/complete",
            Some(json!({
                "target": { "kind": "local", "root": alice_str },
                "query": bad_query,
                "aliases": { "b": bob_str },
            })),
        )
        .await;
        assert_eq!(st_c, StatusCode::OK);
        assert!(
            c_body["items"].as_array().unwrap().is_empty(),
            "expected empty items for confined query {bad_query}: {c_body}"
        );
    }
    let (_, ok_comp) = send_json(
        &app_alice,
        Method::POST,
        "/ide/fs/complete",
        Some(json!({ "target": { "kind": "local", "root": alice_str }, "query": "@main" })),
    )
    .await;
    assert!(ok_comp["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "main.go"));

    let (_, snap_empty) = send_json(
        &app_alice,
        Method::POST,
        "/ide/history/snapshot",
        Some(json!({ "root": alice_str, "path": "main.go", "content": "" })),
    )
    .await;
    assert_eq!(snap_empty["entry"]["size_bytes"], 0);

    #[cfg(unix)]
    {
        let sub = ws_alice.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::os::unix::fs::symlink(ws_bob.join("secret.go"), sub.join("link.go")).unwrap();
        let (st_del_dir, _) = send_json(
            &app_alice,
            Method::POST,
            "/ide/fs/mutate",
            Some(json!({
                "target": { "kind": "local", "root": alice_str },
                "op": "delete",
                "path": "sub",
            })),
        )
        .await;
        assert_eq!(st_del_dir, StatusCode::OK);
        let (_, hist_list) = send_json(
            &app_alice,
            Method::GET,
            &format!(
                "/ide/history/list?root={}&include_deleted=true",
                urlencode(&alice_str)
            ),
            None,
        )
        .await;
        assert!(!hist_list["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["rel_path"].as_str().unwrap_or("").contains("link.go")));
    }

    let (st_lsp, lsp_body) = send_json(&app_alice, Method::GET, "/ide/lsp/status", None).await;
    assert_eq!(st_lsp, StatusCode::OK);
    assert_eq!(lsp_body["max_servers_per_owner"], 3);
    assert_eq!(lsp_body["max_global_servers"], 3);
    assert_eq!(lsp_body["rss_budget_bytes"], 1_288_490_188u64);

    let (st_local_run_forbid, _) = send_json(
        &app_alice,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": alice_str },
            "adhoc": {
                "name": "sse-echo",
                "runner": "go_test",
                "args": ["./..."],
            },
        })),
    )
    .await;
    assert_eq!(st_local_run_forbid, StatusCode::FORBIDDEN);

    let alice_user = non_root_user("u_alice");
    let started_run = ctx
        .ide
        .environments
        .start_run_for_user(
            &ctx.pool,
            ctx.secrets.as_ref(),
            &ctx.ide.targets,
            Some(&ctx.ide.projects),
            Some(&alice_user),
            IdeStartRunReq {
                target: IdeTargetSpec::local(alice_str.clone()),
                adhoc: Some(IdeRunConfig {
                    name: "sse-echo".into(),
                    runner: IdeRunnerKind::Shell,
                    command: Some("sh".into()),
                    args: vec!["-c".into(), "echo SSE_STREAM_OK".into()],
                    ..Default::default()
                }),
                ..Default::default()
            },
            None,
        )
        .await
        .expect("start seeded run for alice SSE test");
    let run_id = started_run.id;

    let (st_bob_sse, _, _) =
        send_raw(&app_bob, Method::GET, &format!("/ide/runs/{run_id}/events")).await;
    assert_eq!(st_bob_sse, StatusCode::FORBIDDEN);

    let (st_sse, ct_sse, sse_bytes) = send_raw(
        &app_alice,
        Method::GET,
        &format!("/ide/runs/{run_id}/events"),
    )
    .await;
    assert_eq!(st_sse, StatusCode::OK);
    assert!(ct_sse.unwrap_or_default().contains("text/event-stream"));
    let sse_text = String::from_utf8_lossy(&sse_bytes);
    assert!(
        sse_text.contains("SSE_STREAM_OK") && sse_text.contains("\"passed\""),
        "unexpected run SSE payload: {sse_text}"
    );
}

#[tokio::test]
async fn local_history_disabled_preference_suppresses_snapshots_across_writes_and_deletes() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws-nohist");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("note.txt"), "v1\n").unwrap();
    let ws_str = ws.to_string_lossy().into_owned();

    let (ctx, _) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let app = router_as(&ctx, non_root_user("u_alice"), vec![ws.clone()]);

    let (st_w, _) = send_json(
        &app,
        Method::PUT,
        "/ide/fs/file",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": "note.txt",
            "content": "v2\n",
            "local_history_enabled": false,
        })),
    )
    .await;
    assert_eq!(st_w, StatusCode::OK);

    let (st_w_legacy_alias, _) = send_json(
        &app,
        Method::PUT,
        "/ide/fs/file",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": "note.txt",
            "content": "v2b\n",
            "record_local_history": false,
        })),
    )
    .await;
    assert_eq!(st_w_legacy_alias, StatusCode::OK);

    let (st_del, _) = send_json(
        &app,
        Method::POST,
        "/ide/fs/mutate",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "op": "delete",
            "path": "note.txt",
            "local_history_enabled": false,
        })),
    )
    .await;
    assert_eq!(st_del, StatusCode::OK);

    let (st_snap, _) = send_json(
        &app,
        Method::POST,
        "/ide/history/snapshot",
        Some(json!({
            "root": ws_str,
            "path": "note.txt",
            "content": "v3\n",
            "local_history_enabled": false,
        })),
    )
    .await;
    assert_eq!(st_snap, StatusCode::BAD_REQUEST);

    let (_, list_body) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/history/list?root={}&include_deleted=true",
            urlencode(&ws_str)
        ),
        None,
    )
    .await;
    assert!(list_body["entries"].as_array().unwrap().is_empty());
}
