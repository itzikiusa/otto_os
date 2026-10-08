//! Regression coverage for atomic access changes and portable restore safety.
use axum::{body::Body, http::Request, routing::put, Extension, Router};
use otto_core::auth::AuthUser;
use otto_server::{state_archive::*, ServerCtx};
use otto_state::{DbPool, UsersRepo};
use tower::ServiceExt;

#[tokio::test]
async fn rejected_goal_patch_preserves_all_previous_settings() {
    use otto_core::domain::{GoalLoopConfig, GoalLoopLimits};
    use serde_json::json;
    let pool = pool().await;
    seed(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let owner = UsersRepo::new(pool.clone())
        .get(&"owner".into())
        .await
        .unwrap();
    let goal = ctx
        .goal_loops_repo
        .create(otto_state::NewGoalLoop {
            workspace_id: "ws".into(),
            name: "Original".into(),
            repo_path: dir.path().to_string_lossy().into_owned(),
            definition: serde_json::from_value(
                json!({"title":"Fixture", "acceptance_criteria":[]}),
            )
            .unwrap(),
            limits: GoalLoopLimits::default(),
            config: GoalLoopConfig::default(),
            created_by: owner.id.clone(),
        })
        .await
        .unwrap();
    let app = otto_server::routes::goal_loops::routes()
        .layer(Extension(AuthUser(owner)))
        .with_state(ctx.clone());
    let mut bad_limits = serde_json::to_value(&goal.limits).unwrap();
    bad_limits["max_iterations"] = json!(0);
    let mut bad_config = serde_json::to_value(&goal.config).unwrap();
    bad_config["mode"] = json!("research");
    let mut no_executors = serde_json::to_value(&goal.config).unwrap();
    no_executors["executors"] = json!([]);
    for body in [
        json!({"name":"Must not persist", "limits":bad_limits}),
        json!({"name":"Must not persist", "config":bad_config}),
        json!({"name":"Must not persist", "config":no_executors}),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/goal-loops/{}", goal.id))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::BAD_REQUEST,
            "{body}"
        );
        let after = ctx.goal_loops_repo.get(&goal.id).await.unwrap();
        assert_eq!(after.name, goal.name, "a rejected PATCH changed the name");
        assert_eq!(
            serde_json::to_value(after.limits).unwrap(),
            serde_json::to_value(&goal.limits).unwrap()
        );
        assert_eq!(
            serde_json::to_value(after.config).unwrap(),
            serde_json::to_value(&goal.config).unwrap()
        );
    }

    // A database rejection of the last field must not persist earlier fields.
    sqlx::query("CREATE TRIGGER reject_goal_config BEFORE UPDATE OF config_json ON goal_loops BEGIN SELECT RAISE(ABORT, 'fixture config failure'); END")
        .execute(&pool).await.unwrap();
    let mut limits = serde_json::to_value(&goal.limits).unwrap();
    limits["max_iterations"] = json!(goal.limits.max_iterations + 1);
    let body = json!({"name":"Updated", "limits":limits, "config":goal.config});
    let request = || {
        Request::builder()
            .method("PATCH")
            .uri(format!("/goal-loops/{}", goal.id))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    let after = ctx.goal_loops_repo.get(&goal.id).await.unwrap();
    assert_eq!(after.name, goal.name);
    assert_eq!(
        serde_json::to_value(after.limits).unwrap(),
        serde_json::to_value(&goal.limits).unwrap()
    );
    sqlx::query("DROP TRIGGER reject_goal_config")
        .execute(&pool)
        .await
        .unwrap();
    let response = app.oneshot(request()).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let after = ctx.goal_loops_repo.get(&goal.id).await.unwrap();
    assert_eq!(after.name, "Updated");
    assert_eq!(after.limits.max_iterations, goal.limits.max_iterations + 1);
}

#[tokio::test]
async fn safety_posture_distinguishes_saved_configuration_from_bound_listener() {
    let pool = pool().await;
    seed(&pool).await;
    sqlx::query(
        "INSERT INTO settings VALUES('network_listener','{\"enabled\":true,\"port\":7799}')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let owner = UsersRepo::new(pool).get(&"owner".into()).await.unwrap();
    let response = otto_server::routes::audit::posture(
        axum::extract::State(ctx),
        otto_server::auth::CurrentUser(owner),
    )
    .await
    .unwrap()
    .0;
    assert!(
        !response.network_listener,
        "saving enabled does not bind a listener"
    );
    assert!(response.loopback_only);
    assert!(response.network_listener_restart_required);
}

async fn pool() -> DbPool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(true),
        )
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    pool.into()
}

async fn seed(pool: &DbPool) {
    for sql in [
        "INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('owner','owner','unused',1,'2026-10-08T00:00:00Z')",
        "INSERT INTO workspaces(id,name,root_path,created_at) VALUES('ws','Test','/external/repo','2026-10-08T00:00:00Z')",
        "INSERT INTO workspace_members VALUES('ws','owner','admin')",
    ] { sqlx::query(sql).execute(pool).await.unwrap(); }
}

#[tokio::test]
async fn failed_membership_replacement_preserves_existing_members() {
    let pool = pool().await;
    seed(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let owner = UsersRepo::new(pool.clone())
        .get(&"owner".into())
        .await
        .unwrap();
    let app = Router::new()
        .route(
            "/workspaces/{id}/members",
            put(otto_server::routes::workspaces::set_members),
        )
        .layer(Extension(AuthUser(owner)))
        .with_state(ctx);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/workspaces/ws/members")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"members":[{"user_id":"nonexistent","role":"editor"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let members: Vec<(String, String)> = sqlx::query_as(
        "SELECT user_id,role FROM workspace_members WHERE workspace_id='ws' ORDER BY user_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        members,
        vec![("owner".into(), "admin".into())],
        "failed PUT must leave prior access intact"
    );
    for (body, expected) in [
        (
            r#"{"members":[{"user_id":"owner","role":"editor"},{"user_id":"owner","role":"admin"}]}"#,
            axum::http::StatusCode::BAD_REQUEST,
        ),
        (
            r#"{"members":[{"user_id":"owner","role":"editor"}]}"#,
            axum::http::StatusCode::OK,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/workspaces/ws/members")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let role: String = sqlx::query_scalar(
            "SELECT role FROM workspace_members WHERE workspace_id='ws' AND user_id='owner'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            role,
            if expected.is_success() {
                "editor"
            } else {
                "admin"
            }
        );
    }
}

#[tokio::test]
async fn portable_archive_with_session_attached_canvas_restores_on_fresh_profile() {
    let source = pool().await;
    seed(&source).await;
    for sql in [
        "INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at) VALUES('session','ws','agent','codex','Session','exited','/external/repo','owner','2026-10-08T00:00:00Z','2026-10-08T00:00:00Z')",
        "INSERT INTO canvas_scenes(id,workspace_id,title,doc_json,created_by,created_at,updated_at) VALUES('scene','ws','Scene','{}','owner','2026-10-08T00:00:00Z','2026-10-08T00:00:00Z')",
        "INSERT INTO canvas_scene_refs VALUES('scene','session','ws','owner','2026-10-08T00:00:00Z')",
    ] { sqlx::query(sql).execute(&source).await.unwrap(); }
    let source_dir = tempfile::tempdir().unwrap();
    let source_ctx = ServerCtx::for_tests(&source, source_dir.path()).await;
    let archive = build_snapshot(
        &source_ctx,
        SnapshotOptions {
            portable: true,
            include_files: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(archive.records["canvas_scenes"].len(), 1);
    assert!(!archive.records.contains_key("sessions"));
    assert!(!archive.records.contains_key("canvas_scene_refs"));
    let full = build_snapshot(
        &source_ctx,
        SnapshotOptions {
            portable: false,
            include_files: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(full.records["canvas_scene_refs"].len(), 1);
    let target = pool().await;
    let target_dir = tempfile::tempdir().unwrap();
    let target_ctx = ServerCtx::for_tests(&target, target_dir.path()).await;
    // Older portable exports carried orphan references. Reject them explicitly
    // instead of silently losing links; preview must leave the target untouched.
    let mut old_portable = archive.clone();
    old_portable.records.insert(
        "canvas_scene_refs".into(),
        full.records["canvas_scene_refs"].clone(),
    );
    let old_preview = preview_restore(&target_ctx, &old_portable, ConflictPolicy::Abort)
        .await
        .unwrap();
    assert!(!old_preview.can_restore);
    assert!(old_preview
        .warnings
        .iter()
        .any(|w| w.contains("canvas_scene_refs")));
    let preview = preview_restore(&target_ctx, &archive, ConflictPolicy::Abort)
        .await
        .unwrap();
    assert!(
        preview.can_restore,
        "exported portable archive must restore: {:?}",
        preview.warnings
    );
    restore_snapshot(
        &target_ctx,
        &archive,
        RestoreOptions {
            conflicts: ConflictPolicy::Abort,
            preview_token: preview.preview_token,
            confirm: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM canvas_scenes WHERE id='scene'")
            .fetch_one(&target)
            .await
            .unwrap(),
        1
    );
    let full_target = pool().await;
    let full_dir = tempfile::tempdir().unwrap();
    let full_ctx = ServerCtx::for_tests(&full_target, full_dir.path()).await;
    let preview = preview_restore(&full_ctx, &full, ConflictPolicy::Abort)
        .await
        .unwrap();
    assert!(preview.can_restore, "{:?}", preview.warnings);
    restore_snapshot(
        &full_ctx,
        &full,
        RestoreOptions {
            conflicts: ConflictPolicy::Abort,
            preview_token: preview.preview_token,
            confirm: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM canvas_scene_refs WHERE session_id='session'"
        )
        .fetch_one(&full_target)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn restored_archive_does_not_activate_mcp_auto_approval() {
    let source = pool().await;
    seed(&source).await;
    sqlx::query("INSERT INTO mcp_auto_approve_rules(id,name,enabled,scope,target_kind,target,allow_irreversible,created_by,created_at,updated_at) VALUES('rule','Merge permission',1,'global','tool','merge_pr',1,'owner','2026-10-08T00:00:00Z','2026-10-08T00:00:00Z')").execute(&source).await.unwrap();
    let source_dir = tempfile::tempdir().unwrap();
    let source_ctx = ServerCtx::for_tests(&source, source_dir.path()).await;
    let mut archive = build_snapshot(
        &source_ctx,
        SnapshotOptions {
            portable: false,
            include_files: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        archive.records["mcp_auto_approve_rules"][0]["enabled"],
        serde_json::json!(0)
    );
    // Archives from older builds retained active rules: import must sanitize too.
    archive.records.get_mut("mcp_auto_approve_rules").unwrap()[0]
        .insert("enabled".into(), serde_json::json!(1));
    let target = pool().await;
    let target_dir = tempfile::tempdir().unwrap();
    let target_ctx = ServerCtx::for_tests(&target, target_dir.path()).await;
    let preview = preview_restore(&target_ctx, &archive, ConflictPolicy::Abort)
        .await
        .unwrap();
    assert!(preview.can_restore, "{:?}", preview.warnings);
    restore_snapshot(
        &target_ctx,
        &archive,
        RestoreOptions {
            conflicts: ConflictPolicy::Abort,
            preview_token: preview.preview_token,
            confirm: true,
        },
    )
    .await
    .unwrap();
    let active = otto_state::McpAutoApproveRepo::new(target.clone())
        .list_applicable(None, None)
        .await
        .unwrap();
    assert!(
        active.is_empty(),
        "restore must require renewed approval, found {active:?}"
    );
}
