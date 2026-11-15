//! Security, RBAC ownership, symlink path-mapping, and external Go build-cache
//! unit tests for the DAP engine.

use std::collections::BTreeMap;

use chrono::Utc;
use otto_core::domain::User;
use otto_core::Error;

use crate::dap::adapters::{
    build_effective_env, display_target_path, normalize_target_path, resolve_dap_adapter,
};
use crate::dap::{DapManager, DapStartContext};
use crate::types::{
    IdeBreakpoint, IdeDebugLaunchSpec, IdeProjectEnvironment, IdeStartDebugReq, IdeTargetSpec,
};

fn make_user(id: &str, is_root: bool) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root,
        disabled: false,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn non_root_debug_rejects_local_pid_attach_port_attach_and_outside_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(ws.join("main.py"), "print(1)\n").unwrap();
    std::fs::write(outside.join("secret.py"), "print(2)\n").unwrap();

    let non_root_ctx = DapStartContext::new()
        .with_caller_is_root(false)
        .with_owner_user_id("u_alice");

    // 1. Local port attach forbidden for non-root.
    let port_req = IdeStartDebugReq {
        target: IdeTargetSpec::local(ws.to_string_lossy().into_owned()),
        language: "python".into(),
        launch: IdeDebugLaunchSpec {
            attach_port: Some(5678),
            ..IdeDebugLaunchSpec::default()
        },
        ..IdeStartDebugReq::default()
    };
    assert!(matches!(
        resolve_dap_adapter(tmp.path(), &port_req, &non_root_ctx).await,
        Err(Error::Forbidden(_))
    ));

    // 2. Local DLV_ATTACH_PID forbidden for non-root.
    let mut env = BTreeMap::new();
    env.insert("DLV_ATTACH_PID".into(), "1".into());
    let pid_req = IdeStartDebugReq {
        target: IdeTargetSpec::local(ws.to_string_lossy().into_owned()),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: ws.to_string_lossy().into_owned(),
            env,
            ..IdeDebugLaunchSpec::default()
        },
        ..IdeStartDebugReq::default()
    };
    assert!(matches!(
        resolve_dap_adapter(tmp.path(), &pid_req, &non_root_ctx).await,
        Err(Error::Forbidden(_))
    ));

    // 3. Program outside authorized workspace root forbidden for non-root.
    let outside_prog_req = IdeStartDebugReq {
        target: IdeTargetSpec::local(ws.to_string_lossy().into_owned()),
        language: "python".into(),
        launch: IdeDebugLaunchSpec {
            program: outside.join("secret.py").to_string_lossy().into_owned(),
            cwd: Some(ws.to_string_lossy().into_owned()),
            ..IdeDebugLaunchSpec::default()
        },
        ..IdeStartDebugReq::default()
    };
    assert!(matches!(
        resolve_dap_adapter(tmp.path(), &outside_prog_req, &non_root_ctx).await,
        Err(Error::Forbidden(_))
    ));

    // 4. cwd outside authorized workspace root forbidden for non-root.
    let outside_cwd_req = IdeStartDebugReq {
        target: IdeTargetSpec::local(ws.to_string_lossy().into_owned()),
        language: "python".into(),
        launch: IdeDebugLaunchSpec {
            program: ws.join("main.py").to_string_lossy().into_owned(),
            cwd: Some(outside.to_string_lossy().into_owned()),
            ..IdeDebugLaunchSpec::default()
        },
        ..IdeStartDebugReq::default()
    };
    assert!(matches!(
        resolve_dap_adapter(tmp.path(), &outside_cwd_req, &non_root_ctx).await,
        Err(Error::Forbidden(_))
    ));
}

#[test]
fn non_root_debug_env_scrubs_daemon_and_shell_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned()),
        language: "python".into(),
        launch: IdeDebugLaunchSpec {
            program: tmp.path().join("app.py").to_string_lossy().into_owned(),
            ..IdeDebugLaunchSpec::default()
        },
        ..IdeStartDebugReq::default()
    };

    let mut leaked_ctx_env = BTreeMap::new();
    leaked_ctx_env.insert("OTTO_INTERNAL_TOKEN".into(), "super-secret".into());
    leaked_ctx_env.insert("AWS_SECRET_ACCESS_KEY".into(), "aws-secret".into());
    leaked_ctx_env.insert("GITHUB_TOKEN".into(), "ghp_secret".into());
    leaked_ctx_env.insert("APP_MODE".into(), "dev".into());

    let mut shell_snap = BTreeMap::new();
    shell_snap.insert("OPENAI_API_KEY".into(), "sk-secret".into());

    let ctx = DapStartContext::new()
        .with_caller_is_root(false)
        .with_effective_env(leaked_ctx_env)
        .with_shell_env_snapshot(shell_snap)
        .with_project_env(Some(IdeProjectEnvironment::default()));

    let built = build_effective_env(tmp.path(), &req, &ctx);
    assert!(!built.contains_key("OTTO_INTERNAL_TOKEN"));
    assert!(!built.contains_key("AWS_SECRET_ACCESS_KEY"));
    assert!(!built.contains_key("GITHUB_TOKEN"));
    assert!(!built.contains_key("OPENAI_API_KEY"));
    assert_eq!(built.get("APP_MODE").map(String::as_str), Some("dev"));
    assert!(built.contains_key("PATH"));
}

#[cfg(unix)]
#[test]
fn display_target_path_maps_canonical_dwarf_paths_back_to_symlinked_workspace_root() {
    let tmp = tempfile::tempdir().unwrap();
    let real_dir = tmp.path().join("real_repo");
    let link_dir = tmp.path().join("symlink_repo");
    std::fs::create_dir_all(real_dir.join("cmd")).unwrap();
    std::fs::write(real_dir.join("cmd/main.go"), "package main\n").unwrap();
    std::os::unix::fs::symlink(&real_dir, &link_dir).unwrap();

    let target = IdeTargetSpec::local(link_dir.to_string_lossy().into_owned());
    let editor_tab_path = link_dir.join("cmd/main.go").to_string_lossy().into_owned();
    let dwarf_path = normalize_target_path(&target, &editor_tab_path);

    let real_canon = std::fs::canonicalize(real_dir.join("cmd/main.go"))
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(dwarf_path, real_canon);

    let mapped = display_target_path(&target, &dwarf_path);
    assert_eq!(mapped, editor_tab_path);
}

#[tokio::test]
async fn dap_manager_tracks_session_ownership_without_external_map_drift() {
    use crate::dap::wire::{DapMessage, DapReader, DapWriter};
    use serde_json::json;

    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("main.go");
    std::fs::write(&src, "package main\nfunc main() {}\n").unwrap();

    let (client_io, server_io) = tokio::io::duplex(16384);
    tokio::spawn(async move {
        let (r, w) = tokio::io::split(server_io);
        let mut reader = DapReader::new(r);
        let mut writer = DapWriter::new(w);
        while let Ok(Some(DapMessage::Request(req))) = reader.next_message().await {
            match req.command.as_str() {
                "initialize" => {
                    let _ = writer
                        .send_response(req.seq, "initialize", Ok(Some(json!({}))))
                        .await;
                    let _ = writer.send_event("initialized", Some(json!({}))).await;
                }
                "disconnect" => {
                    let _ = writer
                        .send_response(req.seq, "disconnect", Ok(Some(json!({}))))
                        .await;
                    break;
                }
                other => {
                    let _ = writer
                        .send_response(req.seq, other, Ok(Some(json!({}))))
                        .await;
                }
            }
        }
    });

    let mgr = DapManager::new(tmp.path().to_path_buf());
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned()),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: src.to_string_lossy().into_owned(),
            ..IdeDebugLaunchSpec::default()
        },
        breakpoints: vec![IdeBreakpoint {
            path: src.to_string_lossy().into_owned(),
            line: 2,
            enabled: true,
            condition: None,
            hit_condition: None,
            log_message: None,
            verified: None,
            message: None,
        }],
        ..IdeStartDebugReq::default()
    };

    let snap = mgr
        .start_session_with_stream(req, "dlv", client_io, false)
        .await
        .unwrap();

    let empty_owner = make_user("", false);
    let bob = make_user("u_bob", false);
    let root = make_user("u_root", true);

    assert!(mgr.authorize_session(&empty_owner, &snap.id).is_ok());
    assert!(matches!(
        mgr.authorize_session(&bob, &snap.id),
        Err(Error::Forbidden(_))
    ));
    assert!(mgr.authorize_session(&root, &snap.id).is_ok());
    assert!(mgr.list_sessions_for_user(&bob).is_empty());
    assert_eq!(mgr.list_sessions_for_user(&root).len(), 1);
}
