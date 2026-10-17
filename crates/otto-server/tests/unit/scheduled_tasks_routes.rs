use super::*;

/// S3-05: a task runs as its owner, so another Editor may retime/pause it
/// but not change its prompt or destination (403); the owner and a
/// workspace admin may. A full-form PATCH resending unchanged values passes.
#[tokio::test]
async fn non_owner_editor_cannot_retarget_a_task() {
    use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "ws").await;
    let now = chrono::Utc::now();
    let mk = |id: &str| otto_core::domain::User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root: false,
        disabled: false,
        created_at: now,
    };
    for (id, role) in [("owner", "editor"), ("bob", "editor"), ("adm", "admin")] {
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, 'x', ?, 0, ?)")
            .bind(id).bind(id).bind(id).bind(now.to_rfc3339()).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws', ?, ?)",
        )
        .bind(id)
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    }
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let mut new = otto_state::NewScheduledTask::defaults("ws".into(), "t".into());
    new.prompt = "daily digest".into();
    new.created_by = Some("owner".into());
    new.schedule = json!({"cadence":"interval","every_min":60});
    let task = ctx.scheduled_tasks.create(new).await.unwrap();
    let app = routes().with_state(ctx.clone());
    let patch = |who: &str, body: Value| {
        let mut req = Request::builder()
            .method("PATCH")
            .uri(format!("/scheduled-tasks/{}", task.id))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        req.extensions_mut()
            .insert(otto_core::auth::AuthUser(mk(who)));
        let app = app.clone();
        async move { app.oneshot(req).await.unwrap().status() }
    };
    let evil = json!({"destination":{"type":"email","to":"bob@evil.test"}});
    assert_eq!(patch("bob", evil.clone()).await, StatusCode::FORBIDDEN);
    assert_eq!(
        patch("bob", json!({"prompt":"leak secrets"})).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        ctx.scheduled_tasks.get(&task.id).await.unwrap().prompt,
        "daily digest"
    );
    // Schedule / enable edits, and unchanged values, stay open to Editors.
    assert_eq!(
        patch("bob", json!({"enabled":false,"prompt":"daily digest"})).await,
        StatusCode::OK
    );
    assert_eq!(
        patch("owner", json!({"prompt":"weekly digest"})).await,
        StatusCode::OK
    );
    assert_eq!(patch("adm", evil).await, StatusCode::OK);
}

/// Deleting a task with a run in flight stops that run (and waits for it
/// to wind down) before the row goes — the run used to outlive the delete
/// and still deliver its report.
#[tokio::test]
async fn deleting_a_task_stops_its_running_run_first() {
    use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "ws").await;
    let now = chrono::Utc::now();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('owner', 'owner', 'x', 'owner', 1, ?)")
        .bind(now.to_rfc3339()).execute(&pool).await.unwrap();
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let mut new = otto_state::NewScheduledTask::defaults("ws".into(), "t".into());
    new.prompt = "sleep 30".into();
    new.provider = "shell".into();
    new.cwd = tmp.path().to_string_lossy().into_owned();
    new.created_by = Some("owner".into());
    new.schedule = json!({"cadence":"interval","every_min":60});
    let task = ctx.scheduled_tasks.create(new).await.unwrap();
    let run = scheduled_tasks_engine::spawn_run(&ctx, &task, "manual")
        .await
        .unwrap();
    let owner = otto_core::domain::User {
        id: "owner".into(),
        username: "owner".into(),
        display_name: "owner".into(),
        is_root: true,
        disabled: false,
        created_at: now,
    };
    let mut req = Request::builder()
        .method("DELETE")
        .uri(format!("/scheduled-tasks/{}", task.id))
        .body(axum::body::Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(otto_core::auth::AuthUser(owner));
    let started = std::time::Instant::now();
    let status = routes()
        .with_state(ctx.clone())
        .oneshot(req)
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::OK);
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    // The run's execution has ended: nothing is left to cancel.
    assert!(!scheduled_tasks_engine::cancel_run(&run.id));
    assert!(ctx.scheduled_tasks.get(&task.id).await.is_err());
}

#[test]
fn provider_check_allows_known_and_custom_slugs() {
    for p in ["claude", "codex", "agy", "shell", "", "my-custom-agent"] {
        assert!(
            check_provider(p).is_ok(),
            "provider '{p}' should be accepted"
        );
    }
    assert!(check_provider("bad provider!").is_err());
}

#[test]
fn field_validators() {
    assert!(check_sandbox("none").is_ok());
    assert!(check_sandbox("worktree").is_ok());
    assert!(check_sandbox("vm").is_err());
    assert!(check_kind("agent_prompt").is_ok());
    assert!(check_kind("workflow").is_ok());
    assert!(check_kind("magic").is_err());
    assert!(check_retries(0).is_ok());
    assert!(check_retries(5).is_ok());
    assert!(check_retries(6).is_err());
    assert!(check_timezone("UTC").is_ok());
    assert!(check_timezone("Europe/London").is_ok());
    assert!(check_timezone("").is_ok());
    assert!(check_timezone("Mars/Phobos").is_err());
}

#[test]
fn preview_lists_the_cron_weekday_actually_typed() {
    use chrono::{Datelike, TimeZone};
    // 2026-10-05 is a Monday. `0 9 * * 0` is SUNDAY 09:00 — the preview must
    // say so before Save (S20-17), not "Next in 5 days" after it.
    let from = chrono::Utc.with_ymd_and_hms(2026, 10, 5, 12, 0, 0).unwrap();
    let spec = json!({ "cadence": "cron", "expr": "0 9 * * 0" });
    let fires = preview_fire_times(&spec, chrono_tz::UTC, from, PREVIEW_FIRES);
    assert_eq!(fires.len(), PREVIEW_FIRES);
    let first = chrono::DateTime::parse_from_rfc3339(&fires[0]).unwrap();
    assert_eq!(first.weekday(), chrono::Weekday::Sun);
    assert_eq!(fires[0], "2026-10-11T09:00:00+00:00");
    assert_eq!(fires[1], "2026-10-18T09:00:00+00:00");
}

#[test]
fn preview_respects_timezone_and_spent_once() {
    use chrono::TimeZone;
    let from = chrono::Utc.with_ymd_and_hms(2026, 10, 5, 12, 0, 0).unwrap();
    let daily = json!({ "cadence": "daily", "at": "09:00" });
    let fires = preview_fire_times(&daily, chrono_tz::Asia::Jerusalem, from, 2);
    // 09:00 in Jerusalem (UTC+3 in October) is 06:00 UTC, the next day.
    assert_eq!(fires[0], "2026-10-06T06:00:00+00:00");
    let spent = json!({ "cadence": "once", "run_at": "2026-01-01T00:00:00Z" });
    assert!(preview_fire_times(&spent, chrono_tz::UTC, from, 5).is_empty());
}

#[test]
fn presets_present_and_shaped() {
    let p = builtin_presets();
    assert!(p.iter().any(|x| x.id == "ticket-followup-review"));
    assert!(p.iter().any(|x| x.id == "weekly-security-scan"));
    assert!(p.iter().any(|x| x.id == "weekly-code-review"));
    assert!(p.iter().all(|x| x.prompt.contains("---")));
    // cron preset is valid.
    let sec = p.iter().find(|x| x.id == "weekly-security-scan").unwrap();
    assert!(cadence::validate(&sec.schedule).is_ok());
}
