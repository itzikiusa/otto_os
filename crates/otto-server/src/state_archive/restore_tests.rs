use super::*;

/// backend-performance-4-01: a staged file is moved into place without
/// replacing an existing one, and a symlinked parent is refused.
#[test]
fn staged_files_are_renamed_into_place_without_replacing() {
    let data = tempfile::TempDir::new().unwrap();
    let stage = data.path().join("stage");
    std::fs::create_dir(&stage).unwrap();
    std::fs::write(stage.join("0"), b"new").unwrap();
    std::fs::write(stage.join("1"), b"other").unwrap();
    let stage_fd = rustix::fs::open(&stage, dir_flags(), rustix::fs::Mode::empty()).unwrap();
    assert_eq!(
        publish_renamed(data.path(), &stage_fd, "0", "a/b/f.txt").unwrap(),
        Some(true)
    );
    assert_eq!(
        std::fs::read(data.path().join("a/b/f.txt")).unwrap(),
        b"new"
    );
    assert!(!stage.join("0").exists(), "moved, not copied");
    assert_eq!(
        publish_renamed(data.path(), &stage_fd, "1", "a/b/f.txt").unwrap(),
        Some(false)
    );
    assert_eq!(
        std::fs::read(data.path().join("a/b/f.txt")).unwrap(),
        b"new"
    );
    std::os::unix::fs::symlink(data.path().join("a"), data.path().join("link")).unwrap();
    assert!(publish_renamed(data.path(), &stage_fd, "1", "link/f.txt").is_err());
}

#[tokio::test]
async fn ide_run_configs_and_environments_scrub_cli_and_override_secrets_without_corrupting_otto_json(
) {
    let source = otto_state::db::test_pool().await;
    sqlx::query(
        "INSERT INTO workspaces(id,name,root_path,created_at) VALUES('ws','Test','/external/repo','now')",
    )
    .execute(&source)
    .await
    .unwrap();

    let mut conn = source.acquire().await.unwrap();
    let tables = schema::schema(&mut conn).await.unwrap();

    let env_vars = json!([
        {
            "key": "__otto_module_sdks__",
            "value": json!({"auth-secret-svc": "go-1.23"}).to_string(),
            "secret": false
        },
        {
            "key": "__otto_go_build_tags__",
            "value": json!(["integration"]).to_string(),
            "secret": false
        },
        {
            "key": "__otto_module_go_build_tags__",
            "value": json!({"auth-secret-svc": []}).to_string(),
            "secret": false
        },
        {
            "key": "API_TOKEN",
            "value": "env_tok_sec_0",
            "secret": false
        }
    ]);
    sqlx::query(
        "INSERT INTO ide_project_environments (project_key, target_kind, root_path, env_vars_json, updated_at)
         VALUES ('local:/external/repo', 'local', '/external/repo', ?, 'now')",
    )
    .bind(env_vars.to_string())
    .execute(&mut *conn)
    .await
    .unwrap();

    let args_json = json!([
        "--api-key=arg_sec_1",
        "--token",
        "arg_sec_2",
        "-token=arg_sec_goflag_2b",
        "--port",
        "8080"
    ]);
    let build_flags_json = json!([
        "-ldflags=-X main.apiSecret=flag_sec_3",
        "-Ddb.password=flag_sec_4",
        "-tags=integration"
    ]);
    let env_overrides_json = json!({
        "__otto_coverage__": "1",
        "SERVICE_TOKEN": "ovr_sec_5",
        "DATABASE_URL": "postgres://u:ovr_pw_6@db.local:5432/app?token=ovr_q_7&sslmode=disable",
        "RUST_LOG": "debug"
    });
    sqlx::query(
        "INSERT INTO ide_run_configs
            (id, project_key, target_kind, root_path, name, runner, command, args_json, build_flags_json, env_overrides_json, before_launch, created_at, updated_at)
         VALUES
            ('rc-json-only', 'local:/external/repo', 'local', '/external/repo', 'Benign Command With Secret JSON', 'go', './app', ?, ?, ?, '[]', 'now', 'now')",
    )
    .bind(args_json.to_string())
    .bind(build_flags_json.to_string())
    .bind(env_overrides_json.to_string())
    .execute(&mut *conn)
    .await
    .unwrap();
    let _ = tables;
    drop(conn);

    let data_dir = tempfile::tempdir().unwrap();
    let archive = build_snapshot_parts(
        &source,
        data_dir.path(),
        "test",
        SnapshotOptions {
            portable: true,
            include_files: false,
        },
    )
    .await
    .unwrap();

    let serialized = serde_json::to_string(&archive).unwrap();
    for leaked in [
        "env_tok_sec_0",
        "arg_sec_1",
        "arg_sec_2",
        "arg_sec_goflag_2b",
        "flag_sec_3",
        "flag_sec_4",
        "ovr_sec_5",
        "ovr_pw_6",
        "ovr_q_7",
    ] {
        assert!(
            !serialized.contains(leaked),
            "portable archive leaked {leaked}: {serialized}"
        );
    }

    let env_row = &archive.records["ide_project_environments"][0];
    let parsed_env: Vec<Value> =
        serde_json::from_str(env_row["env_vars_json"].as_str().unwrap()).unwrap();
    let by_key = |k: &str| {
        parsed_env
            .iter()
            .find(|e| e["key"].as_str() == Some(k))
            .unwrap_or_else(|| panic!("missing env entry {k}"))
    };
    assert_eq!(
        serde_json::from_str::<Value>(
            by_key("__otto_module_go_build_tags__")["value"]
                .as_str()
                .unwrap()
        )
        .unwrap(),
        json!({"auth-secret-svc": []})
    );
    assert_eq!(
        serde_json::from_str::<Value>(by_key("__otto_module_sdks__")["value"].as_str().unwrap())
            .unwrap(),
        json!({"auth-secret-svc": "go-1.23"})
    );

    let rc_row = &archive.records["ide_run_configs"][0];
    let parsed_args: Vec<String> =
        serde_json::from_str(rc_row["args_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        parsed_args,
        vec![
            "--api-key=[REDACTED]",
            "--token",
            "[REDACTED]",
            "-token=[REDACTED]",
            "--port",
            "8080",
        ]
    );
    let parsed_flags: Vec<String> =
        serde_json::from_str(rc_row["build_flags_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        parsed_flags,
        vec![
            "-ldflags=-X main.apiSecret=[REDACTED]",
            "-Ddb.password=[REDACTED]",
            "-tags=integration",
        ]
    );
    let parsed_ovr: Value =
        serde_json::from_str(rc_row["env_overrides_json"].as_str().unwrap()).unwrap();
    assert_eq!(parsed_ovr["__otto_coverage__"], json!("1"));
    assert_eq!(parsed_ovr["SERVICE_TOKEN"], json!(""));
    assert_eq!(parsed_ovr["RUST_LOG"], json!("debug"));
    assert!(parsed_ovr["DATABASE_URL"]
        .as_str()
        .unwrap()
        .contains("sslmode=disable"));

    assert!(
        archive
            .reconnect
            .iter()
            .any(|m| m.starts_with("ide_project_environments/")),
        "expected ide_project_environments reconnect notice: {:?}",
        archive.reconnect
    );
    assert!(
        archive
            .reconnect
            .iter()
            .any(|m| m.starts_with("ide_run_configs/") && m.contains("rc-json-only")),
        "expected ide_run_configs reconnect notice from JSON columns even with benign command: {:?}",
        archive.reconnect
    );
}

#[tokio::test]
async fn restore_snapshot_disables_imported_ide_plugins_until_root_enables_them() {
    let source_pool = otto_state::db::test_pool().await;
    let source_dir = tempfile::tempdir().unwrap();

    // Seed two managed IDE plugins in the source archive:
    // 1. `explicit-pack`: explicitly marked `"enabled": true` in `ide-plugins/state.json`.
    // 2. `implicit-pack`: omitted from `ide-plugins/state.json` (which defaults to enabled on normal load).
    let plugins_root = source_dir.path().join("ide-plugins");
    std::fs::create_dir_all(plugins_root.join("explicit-pack")).unwrap();
    std::fs::write(
        plugins_root.join("explicit-pack/otto-ide-plugin.json"),
        serde_json::to_vec(&json!({
            "slug": "explicit-pack",
            "name": "Explicit Pack",
            "version": "0.1.0",
            "description": "Imported pack with explicit true state"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(plugins_root.join("implicit-pack")).unwrap();
    std::fs::write(
        plugins_root.join("implicit-pack/otto-ide-plugin.json"),
        serde_json::to_vec(&json!({
            "slug": "implicit-pack",
            "name": "Implicit Pack",
            "version": "0.1.0",
            "description": "Imported pack omitted from state.json"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        plugins_root.join("state.json"),
        serde_json::to_vec(&json!({
            "enabled": { "explicit-pack": true }
        }))
        .unwrap(),
    )
    .unwrap();

    let archive = build_snapshot_parts(
        &source_pool,
        source_dir.path(),
        "source-host",
        SnapshotOptions {
            portable: true,
            include_files: true,
        },
    )
    .await
    .unwrap();
    assert!(
        !archive
            .files
            .iter()
            .any(|f| f.root == "data-ide-plugins" && f.path == "state.json"),
        "host ide-plugins/state.json must never be exported in archives"
    );

    let target_pool = otto_state::db::test_pool().await;
    let target_dir = tempfile::tempdir().unwrap();
    // Pre-existing enabled plugin on target should remain enabled across SkipExisting restore.
    let target_plugins = target_dir.path().join("ide-plugins");
    std::fs::create_dir_all(target_plugins.join("preexisting-pack")).unwrap();
    std::fs::write(
        target_plugins.join("preexisting-pack/otto-ide-plugin.json"),
        serde_json::to_vec(&json!({
            "slug": "preexisting-pack",
            "name": "Preexisting Pack",
            "version": "0.1.0"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        target_plugins.join("state.json"),
        serde_json::to_vec(&json!({
            "enabled": { "preexisting-pack": true }
        }))
        .unwrap(),
    )
    .unwrap();

    let target_ctx = ServerCtx::for_tests(&target_pool, target_dir.path()).await;
    let preview = preview_restore(&target_ctx, &archive, ConflictPolicy::SkipExisting)
        .await
        .unwrap();
    assert!(preview.can_restore);

    let result = restore_snapshot(
        &target_ctx,
        &archive,
        RestoreOptions {
            conflicts: ConflictPolicy::SkipExisting,
            preview_token: preview.preview_token,
            confirm: true,
        },
    )
    .await
    .unwrap();
    assert!(result.files_restored >= 2);

    let reloaded = target_ctx.ide.plugins.reload();
    let find_slug = |slug: &str| {
        reloaded
            .iter()
            .find(|p| p.slug == slug)
            .unwrap_or_else(|| panic!("expected plugin '{slug}' in {reloaded:?}"))
    };
    assert!(
        !find_slug("explicit-pack").enabled,
        "imported plugin with state.json enabled=true must be forced disabled after restore"
    );
    assert!(
        !find_slug("implicit-pack").enabled,
        "imported plugin omitted from state.json must be forced disabled after restore"
    );
    assert!(
        find_slug("preexisting-pack").enabled,
        "pre-existing enabled target plugin should stay enabled"
    );
}

#[test]
fn archive_inert_settles_runtime_netbridge_and_rejects_plugin_state_spoofing() {
    let mut rt_sess: ArchiveRow = [
        ("id".into(), json!("rt-1")),
        ("ui_control".into(), json!("{\"allowed\":true}")),
        ("native_thread_id".into(), json!("foreign-thread-99")),
    ]
    .into_iter()
    .collect();
    schema::inert("agent_runtime_sessions", &mut rt_sess);
    assert_eq!(rt_sess["ui_control"], Value::Null);
    assert_eq!(rt_sess["native_thread_id"], Value::Null);

    let running_turn = otto_core::agent_runtime::ProviderTurn {
        id: "t-1".into(),
        ordinal: 1,
        status: otto_core::agent_runtime::ProviderTurnStatus::Running,
        started_at: Some(chrono::Utc::now()),
        completed_at: None,
        turn_token_usage: None,
        origin: otto_core::agent_runtime::TurnOrigin::User,
        model: None,
        effort: None,
    };
    let mut rt_ent: ArchiveRow = [
        ("kind".into(), json!("turn")),
        (
            "payload_json".into(),
            json!(serde_json::to_string(&running_turn).unwrap()),
        ),
    ]
    .into_iter()
    .collect();
    schema::inert("agent_runtime_entities", &mut rt_ent);
    let settled: otto_core::agent_runtime::ProviderTurn =
        serde_json::from_str(rt_ent["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        settled.status,
        otto_core::agent_runtime::ProviderTurnStatus::Interrupted
    );
    assert!(settled.completed_at.is_some());

    let mut ide_env: ArchiveRow = [(
        "env_vars_json".into(),
        json!(json!([{
            "key": "__otto_network_bridge__",
            "value": "{\"enabled\":true,\"k8s_cluster_id\":\"c1\"}",
            "secret": false
        }])
        .to_string()),
    )]
    .into_iter()
    .collect();
    schema::inert("ide_project_environments", &mut ide_env);
    let env_entries: Vec<Value> =
        serde_json::from_str(ide_env["env_vars_json"].as_str().unwrap()).unwrap();
    let bridge: Value = serde_json::from_str(env_entries[0]["value"].as_str().unwrap()).unwrap();
    assert_eq!(bridge["enabled"], json!(false));

    let mk_archive = |path: &str, bytes: &[u8]| StateArchive {
        archive_format: 2,
        schema_version: 1,
        daemon_version: "test".into(),
        snapshot_at: "now".into(),
        records: BTreeMap::new(),
        roots: vec![ArchiveRoot {
            id: "data-ide-plugins".into(),
            kind: "data".into(),
            owner_id: Some("ide-plugins".into()),
        }],
        files: vec![ArchiveFile {
            root: "data-ide-plugins".into(),
            path: path.into(),
            sha256: digest(bytes),
            content_base64: STANDARD.encode(bytes),
        }],
        excluded: vec![],
        reconnect: vec![],
    };
    assert!(files::validate_files(&mk_archive("state.json", b"{}")).is_err());
    assert!(files::validate_files(&mk_archive(
        "evil-dir/otto-ide-plugin.json",
        br#"{"slug":"trusted-slug"}"#,
    ))
    .is_err());
}
