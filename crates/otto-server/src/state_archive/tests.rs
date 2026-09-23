use super::*;

fn row(value: Value) -> ArchiveRow {
    serde_json::from_value(value).unwrap()
}
async fn snapshot(pool: &sqlx::SqlitePool) -> StateArchive {
    let mut conn = pool.acquire().await.unwrap();
    let tables = schema::schema(&mut conn).await.unwrap();
    let mut records = BTreeMap::new();
    let mut reconnect = BTreeSet::new();
    for (table, spec) in &tables {
        if !schema::excluded_table(table, false) {
            let mut rows = schema::read_rows(&mut conn, table).await.unwrap();
            for row in &mut rows {
                schema::sanitize(table, spec, row, &mut reconnect);
            }
            records.insert(table.clone(), rows);
        }
    }
    StateArchive {
        archive_format: 2,
        schema_version: version(&mut conn).await.unwrap(),
        daemon_version: "test".into(),
        snapshot_at: "test".into(),
        records,
        roots: vec![],
        files: vec![],
        excluded: vec![],
        reconnect: reconnect.into_iter().collect(),
    }
}
async fn fixture(pool: &sqlx::SqlitePool) {
    for statement in [
        "INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('old-user','same-name','do-not-export',1,'now')",
        "INSERT INTO workspaces(id,name,root_path,created_at) VALUES('ws','Test','/external/repo','now')",
        "INSERT INTO workspace_members VALUES('ws','old-user','admin')",
        "INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at) VALUES('session','ws','agent','codex','Session','running','/external/repo','old-user','now','now')",
        "INSERT INTO vaults(id,ws_id,name,root_path,created_at) VALUES(300,'ws','Docs','/old/vault','now')",
        "INSERT INTO canvas_scenes(id,workspace_id,title,doc_json,created_by,created_at,updated_at) VALUES('scene','ws','Scene','{}','old-user','now','now')",
        "INSERT INTO scheduled_tasks(id,workspace_id,name,enabled,created_at,updated_at) VALUES('schedule','ws','Schedule',1,'now','now')",
        "INSERT INTO workflows(id,workspace_id,name,created_by,created_at,updated_at) VALUES('workflow','ws','Workflow','old-user','now','now')",
        "INSERT INTO workflow_runs(id,workflow_id,workspace_id,status,started_at) VALUES('run','workflow','ws','running','now')",
        "INSERT INTO swarms(id,workspace_id,name,status,created_by,created_at,updated_at) VALUES('swarm','ws','Swarm','active','old-user','now','now')",
        "INSERT INTO goal_loops(id,workspace_id,name,repo_path,definition_json,limits_json,config_json,status,created_by,created_at,updated_at) VALUES('loop','ws','Loop','/external/repo','{}','{}','{}','running','old-user','now','now')",
        "INSERT INTO goal_loop_iterations(id,loop_id,workspace_id,idx,status,started_at) VALUES('iteration','loop','ws',1,'executing','now')",
    ] {sqlx::query(statement).execute(pool).await.unwrap();}
}

#[tokio::test]
async fn cross_profile_rows_restore_with_real_constraints_and_inert_runtime() {
    let source = otto_state::db::test_pool().await;
    fixture(&source).await;
    let mut archive = snapshot(&source).await;
    assert!(!archive.records.contains_key("workspace_members"));
    assert!(!archive.records.contains_key("auth_sessions"));
    let text = serde_json::to_string(&archive).unwrap();
    assert!(!text.contains("do-not-export"));
    archive.roots.push(ArchiveRoot {
        id: "vault-300".into(),
        kind: "vault".into(),
        owner_id: Some("300".into()),
    });
    let target = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('local-user','same-name','retained',1,'now')").execute(&target).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut tx = target.begin().await.unwrap();
    let tables = validate_archive(&mut tx, &archive).await.unwrap();
    let applied = apply_rows(&mut tx, dir.path(), &archive, &tables, "restored-test")
        .await
        .unwrap();
    assert!(applied.inserted > 8);
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT created_by FROM sessions WHERE id='session'")
            .fetch_one(&target)
            .await
            .unwrap(),
        "local-user"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT password_hash FROM users WHERE id='local-user'")
            .fetch_one(&target)
            .await
            .unwrap(),
        "retained"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM workspace_members")
            .fetch_one(&target)
            .await
            .unwrap(),
        0
    );
    for (table, id, status) in [
        ("sessions", "session", "exited"),
        ("workflow_runs", "run", "error"),
        ("swarms", "swarm", "paused"),
        ("goal_loops", "loop", "paused"),
        ("goal_loop_iterations", "iteration", "error"),
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, String>(&format!("SELECT status FROM {table} WHERE id=?"))
                .bind(id)
                .fetch_one(&target)
                .await
                .unwrap(),
            status
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT enabled FROM scheduled_tasks WHERE id='schedule'")
            .fetch_one(&target)
            .await
            .unwrap(),
        0
    );
    files::publish(
        dir.path(),
        "restored/restored-test/vaults/300/readme.md",
        b"# Restored\nExact user document\n",
    )
    .unwrap();
    let vault = std::sync::Arc::new(otto_vault::VaultEngine::new(target.clone()));
    vault.scan(300).await.unwrap();
    let note = vault.note("ws", 300, "readme.md").await.unwrap();
    assert_eq!(note.raw, "# Restored\nExact user document\n");
}

#[tokio::test]
async fn preview_binding_ignores_unrelated_rows_but_changes_on_import_conflict() {
    let source = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO settings VALUES('archive-fixture','{\"value\":1}')")
        .execute(&source)
        .await
        .unwrap();
    let mut archive = snapshot(&source).await;
    archive.records.retain(|t, _| t == "settings");
    let target = otto_state::db::test_pool().await;
    let dir = tempfile::tempdir().unwrap();
    async fn signature(
        pool: &sqlx::SqlitePool,
        dir: &std::path::Path,
        archive: &StateArchive,
    ) -> String {
        let mut tx = pool.begin().await.unwrap();
        let tables = validate_archive(&mut tx, archive).await.unwrap();
        let rows = apply_rows(&mut tx, dir, archive, &tables, "test")
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        rows.target_signature
    }
    let before = signature(&target, dir.path(), &archive).await;
    sqlx::query("INSERT INTO settings VALUES('unrelated-background-update','1')")
        .execute(&target)
        .await
        .unwrap();
    assert_eq!(signature(&target, dir.path(), &archive).await, before);
    sqlx::query("INSERT INTO settings VALUES('archive-fixture','2')")
        .execute(&target)
        .await
        .unwrap();
    assert_ne!(signature(&target, dir.path(), &archive).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT value_json FROM settings WHERE key='archive-fixture'"
        )
        .fetch_one(&target)
        .await
        .unwrap(),
        "2"
    );
}

#[tokio::test]
async fn invalid_foreign_keys_roll_back_every_insert_and_permissions_are_rejected() {
    let pool = otto_state::db::test_pool().await;
    let mut archive = snapshot(&pool).await;
    archive.records.clear();
    archive.records.insert("sessions".into(),vec![row(json!({"id":"bad","workspace_id":"missing","title":"bad","kind":"agent","provider":"codex","status":"running","cwd":"/tmp","created_by":"missing","created_at":"now","last_active_at":"now"}))]);
    let dir = tempfile::tempdir().unwrap();
    let mut tx = pool.begin().await.unwrap();
    let tables = validate_archive(&mut tx, &archive).await.unwrap();
    assert!(apply_rows(&mut tx, dir.path(), &archive, &tables, "test")
        .await
        .is_err());
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE id='bad'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    archive.records.insert("workspace_members".into(), vec![]);
    let mut conn = pool.acquire().await.unwrap();
    assert!(validate_archive(&mut conn, &archive).await.is_err());
}

#[tokio::test]
async fn credentials_are_removed_by_structure_and_declared_secret_names() {
    let pool = otto_state::db::test_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    let tables = schema::schema(&mut conn).await.unwrap();
    let mut mcp_row = row(
        json!({"id":"mcp","env_json":"{\"NONSTANDARD\":\"env-secret\",\"PUBLIC\":\"keep\"}","headers_json":"{\"X-Custom\":\"header-secret\"}","secret_env_keys":"[\"NONSTANDARD\"]","secret_header_keys":"[\"X-Custom\"]"}),
    );
    schema::sanitize(
        "mcp_servers",
        &tables["mcp_servers"],
        &mut mcp_row,
        &mut BTreeSet::new(),
    );
    let output = serde_json::to_string(&mcp_row).unwrap();
    assert!(!output.contains("env-secret"));
    assert!(!output.contains("header-secret"));
    assert!(output.contains("keep"));
    let mut workflow = row(
        json!({"id":"workflow","graph_json":json!({"nodes":[{"id":"http","params":{"headers":[{"name":"Authorization","value":"Bearer workflow-secret"}],"prompt":"Keep the complete prompt","max_tokens":120,"token_budget":400}}]}).to_string()}),
    );
    schema::sanitize(
        "workflows",
        &tables["workflows"],
        &mut workflow,
        &mut BTreeSet::new(),
    );
    let output = serde_json::to_string(&workflow).unwrap();
    assert!(!output.contains("workflow-secret"));
    assert!(output.contains("Keep the complete prompt"));
    assert!(output.contains("max_tokens"));
    assert!(output.contains("token_budget"));
    let mut config = json!({"mongodb":"mongodb://user:uri-secret@a:27017,b:27017/db?api_key=query-secret&replicaSet=r", "url":"https://user:web-secret@example.test/path?api_key=query-secret&public=yes", "headers":[{"name":"Authorization","value":"Bearer auth-secret"}], "opaque":"keychain:some-ref", "$nested":{"$secret":"ref"},"api_auth":{"type":"api_key","key":"X-Custom","value":"apikey-secret"},"custom_headers":[{"name":"X-API-Key","value":"apikey-header-secret"}]});
    schema::scrub_json(&mut config);
    let output = config.to_string();
    for secret in [
        "uri-secret",
        "web-secret",
        "query-secret",
        "auth-secret",
        "some-ref",
        "apikey-secret",
        "apikey-header-secret",
    ] {
        assert!(!output.contains(secret), "{output}");
    }
    assert!(output.contains("public=yes"));
    assert!(output.contains("replicaSet=r"));
}

#[test]
fn assets_are_exclusive_and_symlinks_cannot_redirect_publish_or_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    assert!(files::publish(dir.path(), "canvas/scene/canvas.d2", b"a -> b").unwrap());
    assert!(!files::publish(dir.path(), "canvas/scene/canvas.d2", b"replacement").unwrap());
    assert_eq!(
        std::fs::read(dir.path().join("canvas/scene/canvas.d2")).unwrap(),
        b"a -> b"
    );
    files::remove_matching(dir.path(), "canvas/scene/canvas.d2", &digest(b"different")).unwrap();
    assert!(dir.path().join("canvas/scene/canvas.d2").exists());
    std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
    assert!(files::publish(dir.path(), "linked/private", b"bad").is_err());
    assert!(files::remove_matching(dir.path(), "linked/private", &digest(b"bad")).is_err());
    assert!(!outside.path().join("private").exists());
    files::remove_matching(dir.path(), "canvas/scene/canvas.d2", &digest(b"a -> b")).unwrap();
    assert!(!dir.path().join("canvas/scene/canvas.d2").exists());
    assert!(files::relative("../escape").is_err());
    assert!(files::relative("a//b").is_err());
}

#[test]
fn partial_publication_is_removed_and_manifest_rejects_unsafe_assets() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    assert!(files::publish_using(dir.path(), "product/partial", |file| {
        file.write_all(b"partial")?;
        Err(std::io::Error::other("simulated sync failure"))
    })
    .is_err());
    assert!(!dir.path().join("product/partial").exists());
    let mut archive = StateArchive {
        archive_format: 2,
        schema_version: 0,
        daemon_version: "test".into(),
        snapshot_at: "test".into(),
        records: BTreeMap::new(),
        roots: vec![ArchiveRoot {
            id: "data-library".into(),
            kind: "data".into(),
            owner_id: Some("library".into()),
        }],
        files: vec![ArchiveFile {
            root: "data-library".into(),
            path: "safe.md".into(),
            sha256: digest(b"safe"),
            content_base64: STANDARD.encode(b"safe"),
        }],
        excluded: vec![],
        reconnect: vec![],
    };
    assert!(files::validate_files(&archive).is_ok());
    for path in [
        "../escape",
        "nested/PRIVATE.PEM",
        "nested/.env.production",
        "nested/id_ed25519",
        ".otto-sync/snapshot.json",
    ] {
        archive.files[0].path = path.into();
        assert!(files::validate_files(&archive).is_err(), "{path}");
    }
    archive.files[0].path = "safe.md".into();
    archive.files[0].sha256 = digest(b"tampered");
    assert!(files::validate_files(&archive).is_err());
}

#[test]
fn design_blobs_are_archived_best_effort_and_restore_by_hash() {
    let dir = tempfile::tempdir().unwrap();
    let blobs = dir.path().join(files::DESIGN_BLOBS_DIR);
    std::fs::create_dir_all(&blobs).unwrap();
    let (version, thumb) = (b"version-bytes".as_slice(), b"thumb-bytes".as_slice());
    for bytes in [version, thumb] {
        std::fs::write(blobs.join(digest(bytes)), bytes).unwrap();
    }
    // Not referenced by any row: never archived.
    std::fs::write(blobs.join(".tmp-partial"), b"partial").unwrap();
    let mut records = BTreeMap::new();
    records.insert(
        "design_versions".to_string(),
        vec![
            row(json!({"id": "v1", "blob_sha256": digest(version)})),
            row(json!({"id": "v2", "blob_sha256": "0".repeat(64)})),
        ],
    );
    records.insert(
        "design_artifacts".to_string(),
        vec![
            row(json!({"id": "A", "thumb_blob": digest(thumb)})),
            row(json!({"id": "B", "thumb_blob": null})),
        ],
    );
    let refs = files::design_blob_refs(&records);
    assert_eq!(refs.len(), 3);

    let (mut out, mut excluded, mut total) = (vec![], vec![], 0usize);
    let n =
        files::design_blob_files(dir.path(), &refs, &mut out, &mut excluded, &mut total).unwrap();
    assert_eq!(n, 2);
    assert!(out
        .iter()
        .all(|f| f.root == files::DESIGN_BLOBS_ROOT_ID && f.path == f.sha256));
    assert!(
        excluded.iter().any(|e| e.contains("missing")),
        "{excluded:?}"
    );
    assert!(total > 0);

    // Over the budget: skipped with ONE note, never an error.
    let (mut out2, mut excluded2, mut total2) = (vec![], vec![], MAX_ARCHIVE_BYTES - 1);
    let n = files::design_blob_files(dir.path(), &refs, &mut out2, &mut excluded2, &mut total2)
        .unwrap();
    assert_eq!(n, 0);
    assert!(out2.is_empty());
    assert_eq!(total2, MAX_ARCHIVE_BYTES - 1);
    assert!(
        excluded2.iter().any(|e| e.contains("archive cap")),
        "{excluded2:?}"
    );

    // No blob dir at all: noted, not an error.
    let empty = tempfile::tempdir().unwrap();
    let mut excluded3 = vec![];
    let n =
        files::design_blob_files(empty.path(), &refs, &mut vec![], &mut excluded3, &mut 0).unwrap();
    assert_eq!(n, 0);
    assert!(!excluded3.is_empty());

    // The manifest takes hash-named blobs only; they restore under design/blobs.
    let mut archive = StateArchive {
        archive_format: 2,
        schema_version: 0,
        daemon_version: "test".into(),
        snapshot_at: "test".into(),
        records: BTreeMap::new(),
        roots: vec![files::design_blobs_root()],
        files: out,
        excluded: vec![],
        reconnect: vec![],
    };
    assert!(files::validate_files(&archive).is_ok());
    assert_eq!(
        files::root_relative(&archive.roots[0], "r").unwrap(),
        "design/blobs"
    );
    archive.files[0].path = format!("nested/{}", archive.files[0].sha256);
    assert!(files::validate_files(&archive).is_err());
}

#[test]
fn imported_terminal_history_keeps_its_status() {
    for status in [
        "draft",
        "paused",
        "blocked",
        "succeeded",
        "exhausted",
        "failed",
        "stopped",
    ] {
        let mut record = row(json!({"id":"loop","status":status}));
        schema::inert("goal_loops", &mut record);
        assert_eq!(
            record["status"],
            json!(status),
            "historical goal-loop status {status} must survive restore"
        );
    }
    let mut aborted = row(json!({"id":"swarm","status":"aborted"}));
    schema::inert("swarms", &mut aborted);
    assert_eq!(aborted["status"], "aborted");
    for (table, status) in [
        ("goal_loops", "running"),
        ("swarms", "active"),
        ("swarm_agents", "active"),
    ] {
        let mut record = row(json!({"id":"active","status":status}));
        schema::inert(table, &mut record);
        assert_eq!(record["status"], "paused");
    }
}
