use super::*;
use serde_json::json;

async fn mem_pool() -> DbPool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    pool.into()
}

/// A task created with the given `status` (no swarm/project FKs needed — the
/// swarm tables carry no FK constraints).
fn new_task(swarm: &Id, status: &str) -> NewTask {
    NewTask {
        project_id: new_id(),
        swarm_id: swarm.clone(),
        workspace_id: new_id(),
        title: "hand-added".into(),
        description: String::new(),
        assignee_agent_id: None,
        status: status.into(),
        priority: "medium".into(),
        parent_task_id: None,
        depends_on: json!([]),
        labels: json!([]),
        order_idx: 0,
        created_by: new_id(),
    }
}

/// D2 regression: a hand-added task defaults to "todo" (set by
/// `SwarmService::create_task`), and `ready_tasks` MUST pick it up so the
/// swarm actually runs it. An explicitly-"backlog" task stays unscheduled.
#[tokio::test]
async fn ready_tasks_picks_up_todo_excludes_backlog() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();

    let todo = repo.create_task(new_task(&swarm, "todo")).await.unwrap();
    assert_eq!(todo.status, "todo");
    let _backlog = repo.create_task(new_task(&swarm, "backlog")).await.unwrap();

    let ready = repo.ready_tasks(&swarm).await.unwrap();
    assert_eq!(ready.len(), 1, "only the todo task should be schedulable");
    assert_eq!(ready[0].id, todo.id);
    assert_eq!(ready[0].status, "todo");
}

fn new_swarm(ws: &Id) -> NewSwarm {
    NewSwarm {
        workspace_id: ws.clone(),
        name: "team".into(),
        description: String::new(),
        preset_slug: None,
        config: json!({}),
        max_total_runs: None,
        max_cost_usd: None,
        max_runtime_secs: None,
        max_attempts: None,
        created_by: new_id(),
    }
}

fn new_run(swarm: &Id) -> NewRun {
    NewRun {
        swarm_id: swarm.clone(),
        workspace_id: new_id(),
        project_id: None,
        task_id: None,
        agent_id: new_id(),
        kind: "task".into(),
        trigger: "coordinator".into(),
    }
}

/// B6: the lite run list drops `result` (recruit runs keep it), and the
/// graph projection maps each task to its NEWEST run that has a session.
#[tokio::test]
async fn lite_runs_and_latest_task_sessions() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let ws = new_id();
    let swarm = repo.create_swarm(new_swarm(&ws)).await.unwrap();
    let task = repo.create_task(new_task(&swarm.id, "todo")).await.unwrap();
    let mk = |kind: &str| NewRun {
        swarm_id: swarm.id.clone(),
        workspace_id: swarm.workspace_id.clone(),
        project_id: Some(task.project_id.clone()),
        task_id: Some(task.id.clone()),
        agent_id: new_id(),
        kind: kind.into(),
        trigger: "coordinator".into(),
    };
    let with = |sid: Option<&str>| RunPatch {
        session_id: sid.map(|s| Some(s.to_string())),
        result: Some(Some(serde_json::json!({"summary": "big"}))),
        ..Default::default()
    };
    let old = repo.create_run(mk("task")).await.unwrap();
    repo.update_run(&old.id, with(Some("sess-old")))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let recruit = repo.create_run(mk("recruit")).await.unwrap();
    repo.update_run(&recruit.id, with(None)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let newest = repo.create_run(mk("task")).await.unwrap();
    repo.update_run(&newest.id, with(Some("sess-new")))
        .await
        .unwrap();

    let f = RunFilter {
        swarm_id: Some(swarm.id.clone()),
        ..Default::default()
    };
    let full = repo.list_runs(&f).await.unwrap();
    let lite = repo.list_runs_lite(&f).await.unwrap();
    assert_eq!(
        full.iter().map(|r| &r.id).collect::<Vec<_>>(),
        lite.iter().map(|r| &r.id).collect::<Vec<_>>(),
        "same rows, same order"
    );
    assert!(full.iter().all(|r| r.result.is_some()));
    for r in &lite {
        assert_eq!(r.result.is_some(), r.kind == "recruit", "run {}", r.kind);
    }
    let sessions = repo.latest_task_sessions(&swarm.id).await.unwrap();
    assert_eq!(sessions.get(&task.id).map(String::as_str), Some("sess-new"));
    assert_eq!(sessions.len(), 1);
}

#[tokio::test]
async fn reservation_serializes_competing_producers_and_stop() {
    let repo = SwarmRepo::new(mem_pool().await);
    let ws = new_id();
    let mut input = new_swarm(&ws);
    input.config = json!({"max_parallel_sessions": 1});
    let swarm = repo.create_swarm(input).await.unwrap();
    repo.set_swarm_status(&swarm.id, "active").await.unwrap();
    let make = || {
        let mut r = new_run(&swarm.id);
        r.workspace_id = ws.clone();
        r.agent_id = "same-agent".into();
        r
    };
    let (first, second) = tokio::join!(
        repo.reserve_run(make(), false),
        repo.reserve_run(make(), false)
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(repo.active_run_count(&swarm.id).await.unwrap(), 1);
    repo.set_swarm_status(&swarm.id, "paused").await.unwrap();
    repo.stop_active_runs(&swarm.id).await.unwrap();
    assert!(repo.reserve_run(make(), false).await.is_err());
    assert!(
        repo.reserve_run(make(), true).await.is_ok(),
        "explicit manual run on ordinarily paused swarm"
    );
    repo.set_swarm_status(&swarm.id, "aborted").await.unwrap();
    repo.stop_active_runs(&swarm.id).await.unwrap();
    assert!(repo.reserve_run(make(), true).await.is_err());
}

/// An active row with a leftover pause reason is not runnable, including
/// for the manual `allow_paused` path. An ordinary pause (null reason)
/// still admits only that path. A scheduled reserve that loses the
/// predicate does not advance `last_run`.
#[tokio::test]
async fn active_pause_reason_refuses_reserve_and_does_not_advance_schedule() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool.clone());
    let ws = new_id();
    let swarm = repo.create_swarm(new_swarm(&ws)).await.unwrap();
    repo.set_swarm_status(&swarm.id, "active").await.unwrap();
    sqlx::query("UPDATE swarms SET status = 'active', pause_reason = ? WHERE id = ?")
        .bind("cost budget reached")
        .bind(&swarm.id)
        .execute(&pool)
        .await
        .unwrap();
    let make = || {
        let mut run = new_run(&swarm.id);
        run.workspace_id = ws.clone();
        run
    };
    assert!(repo.reserve_run(make(), false).await.is_err());
    assert!(
        repo.reserve_run(make(), true).await.is_err(),
        "allow_paused must not admit an active swarm that still has a pause reason"
    );
    let sched = json!({"cadence":"once","run_at":"2026-10-05T09:00:00+00:00","enabled":true});
    let agent = repo
        .create_agent(new_agent(&swarm, Some(sched.clone())))
        .await
        .unwrap();
    let seen = agent.schedule.clone().unwrap();
    let scheduled = repo
        .reserve_scheduled_run(
            sched_run(&swarm, &agent.id),
            &seen,
            "2026-10-05T09:00:01+00:00",
        )
        .await;
    assert!(matches!(scheduled, Err(otto_core::Error::Conflict(_))));
    let stored = repo.get_agent(&agent.id).await.unwrap().schedule.unwrap();
    assert!(stored.get("last_run").is_none());
    assert_eq!(repo.active_run_count(&swarm.id).await.unwrap(), 0);

    sqlx::query("UPDATE swarms SET status = 'paused', pause_reason = ? WHERE id = ?")
        .bind("cost budget reached")
        .bind(&swarm.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.reserve_run(make(), true).await.is_err());

    sqlx::query("UPDATE swarms SET status = 'active', pause_reason = '   ' WHERE id = ?")
        .bind(&swarm.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.reserve_run(make(), true).await.is_err());

    sqlx::query("UPDATE swarms SET status = 'paused', pause_reason = NULL WHERE id = ?")
        .bind(&swarm.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.reserve_run(make(), false).await.is_err());
    assert!(repo.reserve_run(make(), true).await.is_ok());
}

/// A stop before spawn gives the occurrence back. A session, a started
/// status, or a cursor that moved does not rewind.
#[tokio::test]
async fn unstarted_scheduled_occurrence_rewinds_only_its_cursor() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool.clone());
    let swarm = active_swarm(&pool, &repo).await;
    let fired = "2026-10-05T09:00:01+00:00";

    let once = json!({"cadence":"once","run_at":"2026-10-05T09:00:00+00:00","enabled":true,"directive":"keep"});
    let once_agent = repo
        .create_agent(new_agent(&swarm, Some(once)))
        .await
        .unwrap();
    let once_seen = once_agent.schedule.clone().unwrap();
    let once_run = repo
        .reserve_scheduled_run(sched_run(&swarm, &once_agent.id), &once_seen, fired)
        .await
        .unwrap()
        .unwrap();
    assert!(repo
        .release_unstarted_scheduled_occurrence(&once_run.id, &once_agent.id, fired, None)
        .await
        .unwrap());
    let once_after = repo
        .get_agent(&once_agent.id)
        .await
        .unwrap()
        .schedule
        .unwrap();
    assert!(
        once_after.get("last_run").is_none(),
        "once cursor is the fired flag"
    );
    assert_eq!(once_after["directive"], json!("keep"));
    assert_eq!(repo.get_run(&once_run.id).await.unwrap().status, "stopped");
    assert_eq!(repo.active_run_count(&swarm.id).await.unwrap(), 0);

    let previous = "2026-10-05T08:00:00+00:00";
    let interval = json!({"cadence":"interval","every_min":60,"enabled":true,"directive":"keep","last_run": previous});
    let interval_agent = repo
        .create_agent(new_agent(&swarm, Some(interval)))
        .await
        .unwrap();
    let interval_seen = interval_agent.schedule.clone().unwrap();
    let interval_run = repo
        .reserve_scheduled_run(sched_run(&swarm, &interval_agent.id), &interval_seen, fired)
        .await
        .unwrap()
        .unwrap();
    assert!(repo
        .release_unstarted_scheduled_occurrence(
            &interval_run.id,
            &interval_agent.id,
            fired,
            Some(previous),
        )
        .await
        .unwrap());
    let interval_after = repo
        .get_agent(&interval_agent.id)
        .await
        .unwrap()
        .schedule
        .unwrap();
    assert_eq!(interval_after["last_run"], json!(previous));
    assert_eq!(interval_after["directive"], json!("keep"));

    let started = repo
        .reserve_scheduled_run(
            sched_run(&swarm, &interval_agent.id),
            &interval_after,
            fired,
        )
        .await
        .unwrap()
        .unwrap();
    repo.update_run(
        &started.id,
        RunPatch {
            session_id: Some(Some("sess".into())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(!repo
        .release_unstarted_scheduled_occurrence(
            &started.id,
            &interval_agent.id,
            fired,
            Some(previous),
        )
        .await
        .unwrap());
    let kept = repo
        .get_agent(&interval_agent.id)
        .await
        .unwrap()
        .schedule
        .unwrap();
    assert_eq!(kept["last_run"], json!(fired));
    assert_eq!(repo.get_run(&started.id).await.unwrap().status, "queued");

    let running = repo
        .reserve_scheduled_run(sched_run(&swarm, &once_agent.id), &once_after, fired)
        .await
        .unwrap()
        .unwrap();
    repo.update_run(
        &running.id,
        RunPatch {
            status: Some("running".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(!repo
        .release_unstarted_scheduled_occurrence(&running.id, &once_agent.id, fired, None)
        .await
        .unwrap());
    assert_eq!(
        repo.get_agent(&once_agent.id)
            .await
            .unwrap()
            .schedule
            .unwrap()["last_run"],
        json!(fired)
    );

    let moved_agent = repo
        .create_agent(new_agent(
            &swarm,
            Some(json!({"cadence":"interval","every_min":60,"enabled":true,"directive":"old"})),
        ))
        .await
        .unwrap();
    let moved_seen = moved_agent.schedule.clone().unwrap();
    let moved = repo
        .reserve_scheduled_run(sched_run(&swarm, &moved_agent.id), &moved_seen, fired)
        .await
        .unwrap()
        .unwrap();
    sqlx::query(
        "UPDATE swarm_agents SET schedule_json = json_set(json_set(schedule_json, '$.last_run', ?), '$.directive', ?) WHERE id = ?",
    )
    .bind("2099-01-01T00:00:00+00:00")
    .bind("newer")
    .bind(&moved_agent.id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(repo
        .release_unstarted_scheduled_occurrence(&moved.id, &moved_agent.id, fired, None)
        .await
        .unwrap());
    let newer = repo
        .get_agent(&moved_agent.id)
        .await
        .unwrap()
        .schedule
        .unwrap();
    assert_eq!(newer["last_run"], json!("2099-01-01T00:00:00+00:00"));
    assert_eq!(newer["directive"], json!("newer"));
    assert_eq!(repo.get_run(&moved.id).await.unwrap().status, "stopped");
}

/// A one-run cap gives a never-started scheduled occurrence back, including
/// after retention deletes the row. A run that had a session still spends
/// the cap once retention rolls it into `pruned_runs`.
#[tokio::test]
async fn released_scheduled_occurrence_does_not_spend_the_lifetime_cap() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool.clone());
    let swarm = active_swarm(&pool, &repo).await;
    sqlx::query("UPDATE swarms SET max_total_runs = 1 WHERE id = ?")
        .bind(&swarm.id)
        .execute(&pool)
        .await
        .unwrap();
    let fired = "2026-10-05T09:00:01+00:00";
    let once = json!({"cadence":"once","run_at":"2026-10-05T09:00:00+00:00","enabled":true});
    let agent = repo
        .create_agent(new_agent(&swarm, Some(once.clone())))
        .await
        .unwrap();
    let seen = agent.schedule.clone().unwrap();
    let first = repo
        .reserve_scheduled_run(sched_run(&swarm, &agent.id), &seen, fired)
        .await
        .unwrap()
        .unwrap();
    assert!(repo
        .release_unstarted_scheduled_occurrence(&first.id, &agent.id, fired, None)
        .await
        .unwrap());
    assert_eq!(repo.total_run_count(&swarm.id).await.unwrap(), 0);
    assert_eq!(repo.swarm_spend(&swarm.id).await.unwrap().total_runs, 0);
    let again = repo.get_agent(&agent.id).await.unwrap().schedule.unwrap();
    let second = repo
        .reserve_scheduled_run(sched_run(&swarm, &agent.id), &again, fired)
        .await
        .unwrap()
        .unwrap();
    assert!(repo
        .release_unstarted_scheduled_occurrence(&second.id, &agent.id, fired, None)
        .await
        .unwrap());
    let old = fmt(Utc::now() - chrono::Duration::days(40));
    sqlx::query("UPDATE swarm_runs SET finished_at = ? WHERE swarm_id = ?")
        .bind(&old)
        .bind(&swarm.id)
        .execute(&pool)
        .await
        .unwrap();
    let policy = crate::retention::RetentionPolicy {
        run_history_days: 14,
        ..crate::retention::RetentionPolicy::default()
    };
    crate::retention::RetentionRepo::new(pool.clone())
        .prune(&policy)
        .await
        .unwrap();
    assert_eq!(repo.swarm_spend(&swarm.id).await.unwrap().total_runs, 0);
    let after = repo.get_agent(&agent.id).await.unwrap().schedule.unwrap();
    assert!(repo
        .reserve_scheduled_run(sched_run(&swarm, &agent.id), &after, fired)
        .await
        .unwrap()
        .is_some());

    let spent = active_swarm(&pool, &repo).await;
    sqlx::query("UPDATE swarms SET max_total_runs = 1 WHERE id = ?")
        .bind(&spent.id)
        .execute(&pool)
        .await
        .unwrap();
    let started = repo
        .create_run(NewRun {
            swarm_id: spent.id.clone(),
            workspace_id: spent.workspace_id.clone(),
            project_id: None,
            task_id: None,
            agent_id: new_id(),
            kind: "task".into(),
            trigger: "coordinator".into(),
        })
        .await
        .unwrap();
    repo.update_run(
        &started.id,
        RunPatch {
            status: Some("done".into()),
            session_id: Some(Some("sess".into())),
            finished_at: Some(Some(Utc::now() - chrono::Duration::days(40))),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(repo.swarm_spend(&spent.id).await.unwrap().total_runs, 1);
    let spent_agent = repo
        .create_agent(new_agent(&spent, Some(once.clone())))
        .await
        .unwrap();
    let spent_seen = spent_agent.schedule.clone().unwrap();
    assert!(repo
        .reserve_scheduled_run(sched_run(&spent, &spent_agent.id), &spent_seen, fired)
        .await
        .is_err());
    crate::retention::RetentionRepo::new(pool.clone())
        .prune(&policy)
        .await
        .unwrap();
    assert_eq!(repo.swarm_spend(&spent.id).await.unwrap().total_runs, 1);
    assert!(repo
        .reserve_scheduled_run(sched_run(&spent, &spent_agent.id), &spent_seen, fired)
        .await
        .is_err());

    // Negative control: a stopped, sessionless, scheduled row whose error is
    // SQL NULL is not the release marker. `error IS` keeps it in the lifetime
    // count, and retention rolls it into `pruned_runs`.
    let null_err = active_swarm(&pool, &repo).await;
    sqlx::query("UPDATE swarms SET max_total_runs = 1 WHERE id = ?")
        .bind(&null_err.id)
        .execute(&pool)
        .await
        .unwrap();
    let null_agent = repo
        .create_agent(new_agent(&null_err, Some(once)))
        .await
        .unwrap();
    let null_seen = null_agent.schedule.clone().unwrap();
    let ordinary = repo
        .create_run(NewRun {
            swarm_id: null_err.id.clone(),
            workspace_id: null_err.workspace_id.clone(),
            project_id: None,
            task_id: None,
            agent_id: null_agent.id.clone(),
            kind: "scheduled".into(),
            trigger: "scheduled".into(),
        })
        .await
        .unwrap();
    repo.update_run(
        &ordinary.id,
        RunPatch {
            status: Some("stopped".into()),
            error: Some(None),
            session_id: Some(None),
            finished_at: Some(Some(Utc::now() - chrono::Duration::days(40))),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let stored_error: Option<String> =
        sqlx::query_scalar("SELECT error FROM swarm_runs WHERE id = ?")
            .bind(&ordinary.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(stored_error.is_none());
    assert_eq!(repo.total_run_count(&null_err.id).await.unwrap(), 1);
    assert_eq!(repo.swarm_spend(&null_err.id).await.unwrap().total_runs, 1);
    assert!(repo
        .reserve_scheduled_run(sched_run(&null_err, &null_agent.id), &null_seen, fired)
        .await
        .is_err());
    crate::retention::RetentionRepo::new(pool.clone())
        .prune(&policy)
        .await
        .unwrap();
    assert_eq!(repo.swarm_spend(&null_err.id).await.unwrap().total_runs, 1);
    assert!(repo
        .reserve_scheduled_run(sched_run(&null_err, &null_agent.id), &null_seen, fired)
        .await
        .is_err());
}

/// A stopped, sessionless scheduled row still spends the one-run cap when
/// its error is NULL or any text other than the never-started marker,
/// including after retention rolls that row into `pruned_runs`.
#[tokio::test]
async fn null_error_stopped_scheduled_run_still_spends_the_lifetime_cap() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool.clone());
    let fired = "2026-10-05T09:00:01+00:00";
    let once = json!({"cadence":"once","run_at":"2026-10-05T09:00:00+00:00","enabled":true});

    for error in [None, Some("boom".to_string())] {
        let swarm = active_swarm(&pool, &repo).await;
        sqlx::query("UPDATE swarms SET max_total_runs = 1 WHERE id = ?")
            .bind(&swarm.id)
            .execute(&pool)
            .await
            .unwrap();
        let agent = repo
            .create_agent(new_agent(&swarm, Some(once.clone())))
            .await
            .unwrap();
        let seen = agent.schedule.clone().unwrap();
        let ordinary = repo
            .create_run(NewRun {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                project_id: None,
                task_id: None,
                agent_id: agent.id.clone(),
                kind: "scheduled".into(),
                trigger: "scheduled".into(),
            })
            .await
            .unwrap();
        repo.update_run(
            &ordinary.id,
            RunPatch {
                status: Some("stopped".into()),
                error: Some(error),
                session_id: Some(None),
                finished_at: Some(Some(Utc::now() - chrono::Duration::days(40))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(repo.swarm_spend(&swarm.id).await.unwrap().total_runs, 1);
        assert_eq!(repo.total_run_count(&swarm.id).await.unwrap(), 1);
        assert!(repo
            .reserve_scheduled_run(sched_run(&swarm, &agent.id), &seen, fired)
            .await
            .is_err());
        crate::retention::RetentionRepo::new(pool.clone())
            .prune(&crate::retention::RetentionPolicy {
                run_history_days: 14,
                ..crate::retention::RetentionPolicy::default()
            })
            .await
            .unwrap();
        assert_eq!(repo.swarm_spend(&swarm.id).await.unwrap().total_runs, 1);
        assert!(repo
            .reserve_scheduled_run(sched_run(&swarm, &agent.id), &seen, fired)
            .await
            .is_err());
    }
}

/// D3: a fresh swarm defaults to `max_attempts = 3` and unlimited budgets,
/// and the budget columns round-trip through create/update.
#[tokio::test]
async fn swarm_budget_columns_roundtrip() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let ws = new_id();

    let s = repo.create_swarm(new_swarm(&ws)).await.unwrap();
    assert_eq!(s.max_attempts, 3, "default attempt ceiling");
    assert!(s.max_total_runs.is_none() && s.max_cost_usd.is_none() && s.max_runtime_secs.is_none());
    assert!(s.run_started_at.is_none() && s.pause_reason.is_none());

    // Set limits via the patch (Some(Some(..)) = set the value).
    let s = repo
        .update_swarm(
            &s.id,
            SwarmPatch {
                max_total_runs: Some(Some(5)),
                max_cost_usd: Some(Some(2.5)),
                max_runtime_secs: Some(Some(600)),
                max_attempts: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(s.max_total_runs, Some(5));
    assert_eq!(s.max_cost_usd, Some(2.5));
    assert_eq!(s.max_runtime_secs, Some(600));
    assert_eq!(s.max_attempts, 2);

    // Clear a limit via Some(None) = NULL = unlimited.
    let s = repo
        .update_swarm(
            &s.id,
            SwarmPatch {
                max_cost_usd: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(s.max_cost_usd.is_none(), "null clears the cost budget");
    assert_eq!(s.max_total_runs, Some(5), "other limits untouched");
}

/// D3: `swarm_spend` sums per-run cost (NULLs as 0) and counts every run —
/// the basis for the `max_total_runs` / `max_cost_usd` budget checks.
#[tokio::test]
async fn swarm_spend_sums_cost_and_counts_runs() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();

    let spend = repo.swarm_spend(&swarm).await.unwrap();
    assert_eq!(spend.total_runs, 0);
    assert_eq!(spend.cost_usd, 0.0);

    let r1 = repo.create_run(new_run(&swarm)).await.unwrap();
    let r2 = repo.create_run(new_run(&swarm)).await.unwrap();
    let _r3 = repo.create_run(new_run(&swarm)).await.unwrap(); // cost stays NULL
    repo.update_run(
        &r1.id,
        RunPatch {
            cost_usd: Some(Some(1.25)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    repo.update_run(
        &r2.id,
        RunPatch {
            cost_usd: Some(Some(0.75)),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let spend = repo.swarm_spend(&swarm).await.unwrap();
    assert_eq!(spend.total_runs, 3, "every run counts, even with null cost");
    assert!(
        (spend.cost_usd - 2.0).abs() < 1e-9,
        "1.25 + 0.75 + (null=0)"
    );
}

/// D8: the per-task attempt counter starts at 0 and increments, returning the
/// new value (the Coordinator compares it against `max_attempts`).
#[tokio::test]
async fn bump_task_attempt_increments() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();

    let task = repo.create_task(new_task(&swarm, "todo")).await.unwrap();
    assert_eq!(task.attempts, 0);
    assert_eq!(repo.bump_task_attempt(&task.id).await.unwrap(), 1);
    assert_eq!(repo.bump_task_attempt(&task.id).await.unwrap(), 2);
    assert_eq!(repo.get_task(&task.id).await.unwrap().attempts, 2);
}

/// Lifecycle bookkeeping: going active anchors `run_started_at` + clears the
/// pause reason; an auto-pause records the reason and drops the anchor.
#[tokio::test]
async fn status_manages_runtime_anchor_and_pause_reason() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let ws = new_id();
    let s = repo.create_swarm(new_swarm(&ws)).await.unwrap();

    repo.set_swarm_status(&s.id, "active").await.unwrap();
    let s = repo.get_swarm(&s.id).await.unwrap();
    assert_eq!(s.status, "active");
    assert!(s.run_started_at.is_some(), "active anchors the wall clock");
    assert!(s.pause_reason.is_none());

    repo.pause_swarm_with_reason(&s.id, "cost budget reached")
        .await
        .unwrap();
    let s = repo.get_swarm(&s.id).await.unwrap();
    assert_eq!(s.status, "paused");
    assert_eq!(s.pause_reason.as_deref(), Some("cost budget reached"));
    assert!(s.run_started_at.is_none(), "pause drops the anchor");

    // Resuming clears the reason and re-anchors.
    repo.set_swarm_status(&s.id, "active").await.unwrap();
    let s = repo.get_swarm(&s.id).await.unwrap();
    assert!(s.pause_reason.is_none() && s.run_started_at.is_some());
}

/// Crash recovery: non-terminal runs are failed; terminal ones are untouched.
#[tokio::test]
async fn fail_running_only_hits_non_terminal_runs() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();

    let queued = repo.create_run(new_run(&swarm)).await.unwrap(); // status 'queued'
    let running = repo.create_run(new_run(&swarm)).await.unwrap();
    repo.update_run(
        &running.id,
        RunPatch {
            status: Some("running".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let done = repo.create_run(new_run(&swarm)).await.unwrap();
    repo.update_run(
        &done.id,
        RunPatch {
            status: Some("done".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let n = repo.fail_running("interrupted").await.unwrap();
    assert_eq!(n, 2, "queued + running stopped; done untouched");
    assert_eq!(repo.get_run(&queued.id).await.unwrap().status, "stopped");
    assert_eq!(repo.get_run(&running.id).await.unwrap().status, "stopped");
    assert_eq!(repo.get_run(&done.id).await.unwrap().status, "done");
}

#[tokio::test]
async fn fail_running_preserves_success_waiting_for_routing() {
    let repo = SwarmRepo::new(mem_pool().await);
    let swarm = new_id();
    let task = repo
        .create_task(new_task(&swarm, "in_progress"))
        .await
        .unwrap();
    let mut request = new_run(&swarm);
    request.task_id = Some(task.id.clone());
    let run = repo.create_run(request).await.unwrap();
    repo.update_run(
        &run.id,
        RunPatch {
            status: Some("done".into()),
            result: Some(Some(
                json!({"status":"done", "summary":"already implemented"}),
            )),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    repo.fail_running("restart").await.unwrap();
    assert_eq!(
        repo.get_task(&task.id).await.unwrap().status,
        "in_progress",
        "recover the saved result instead of dispatching the successful task again"
    );
    assert!(repo.ready_tasks(&swarm).await.unwrap().is_empty());
    assert_eq!(repo.pending_task_results(&swarm).await.unwrap().len(), 1);
    repo.finish_task_routing(&run.id).await.unwrap();
    assert!(repo.pending_task_results(&swarm).await.unwrap().is_empty());
}

#[tokio::test]
async fn recovery_never_replays_success_from_an_older_attempt() {
    let repo = SwarmRepo::new(mem_pool().await);
    let swarm = new_id();
    let task = repo
        .create_task(new_task(&swarm, "in_progress"))
        .await
        .unwrap();
    let mut request = new_run(&swarm);
    request.task_id = Some(task.id.clone());
    let run = repo.create_run(request).await.unwrap();
    repo.update_run(
        &run.id,
        RunPatch {
            status: Some("done".into()),
            result: Some(Some(json!({"status":"done"}))),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut next = new_run(&swarm);
    next.task_id = Some(task.id.clone());
    repo.create_run(next).await.unwrap();
    repo.fail_running("restart").await.unwrap();
    assert!(repo.pending_task_results(&swarm).await.unwrap().is_empty());
    assert_eq!(repo.get_task(&task.id).await.unwrap().status, "todo");
}

/// A "todo" task whose dependency isn't done yet is NOT ready; once the
/// dependency is done it becomes ready (guards that the default status
/// doesn't bypass the dependency gate).
#[tokio::test]
async fn ready_tasks_respects_dependencies() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();

    let dep = repo.create_task(new_task(&swarm, "todo")).await.unwrap();
    let mut blocked = new_task(&swarm, "todo");
    blocked.depends_on = json!([dep.id]);
    let blocked = repo.create_task(blocked).await.unwrap();

    // Both are "todo", but `blocked` waits on `dep`.
    let ready = repo.ready_tasks(&swarm).await.unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, dep.id);

    // Finish the dependency → the blocked task becomes ready.
    repo.update_task(
        &dep.id,
        TaskPatch {
            status: Some("done".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let ready = repo.ready_tasks(&swarm).await.unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, blocked.id);
}

/// D3: the budget query building blocks. `total_run_count`/`total_cost` sum a
/// swarm's runs; `task_run_count` (and `create_run`'s `attempt`) count per task.
#[tokio::test]
async fn budget_run_queries_and_attempt_numbering() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let ws = new_id();
    let swarm = repo.create_swarm(new_swarm(&ws)).await.unwrap();
    let task = repo.create_task(new_task(&swarm.id, "todo")).await.unwrap();

    assert_eq!(repo.total_run_count(&swarm.id).await.unwrap(), 0);
    assert_eq!(repo.task_run_count(&task.id).await.unwrap(), 0);

    let mk_run = || NewRun {
        swarm_id: swarm.id.clone(),
        workspace_id: swarm.workspace_id.clone(),
        project_id: Some(task.project_id.clone()),
        task_id: Some(task.id.clone()),
        agent_id: new_id(),
        kind: "task".into(),
        trigger: "coordinator".into(),
    };

    // First run for the task → attempt 0.
    let r0 = repo.create_run(mk_run()).await.unwrap();
    assert_eq!(r0.attempt, 0, "first run is attempt 0");
    // Second run for the same task → attempt 1 (prior run count).
    let r1 = repo.create_run(mk_run()).await.unwrap();
    assert_eq!(r1.attempt, 1, "attempt = prior run count");

    assert_eq!(repo.total_run_count(&swarm.id).await.unwrap(), 2);
    assert_eq!(repo.task_run_count(&task.id).await.unwrap(), 2);

    // Cost sums (best-effort): set cost on the runs and confirm SUM.
    repo.update_run(
        &r0.id,
        RunPatch {
            cost_usd: Some(Some(1.50)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    repo.update_run(
        &r1.id,
        RunPatch {
            cost_usd: Some(Some(2.25)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let spent = repo.total_cost(&swarm.id).await.unwrap();
    assert!(
        (spent - 3.75).abs() < 1e-9,
        "summed cost = 3.75, got {spent}"
    );
}
fn new_project(swarm: &Id, story_id: Option<Id>) -> NewProject {
    NewProject {
        swarm_id: swarm.clone(),
        workspace_id: new_id(),
        name: "proj".into(),
        description: String::new(),
        repo_path: None,
        goal_md: None,
        story_id,
        order_idx: 0,
        created_by: new_id(),
    }
}

/// Epic-tree ingest race: linking a second project to a story that already
/// roots one surfaces as `Conflict` (not a generic DB error) from
/// `update_project`, exactly like `create_project` already does.
#[tokio::test]
async fn update_project_maps_story_unique_violation_to_conflict() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();
    let story = new_id();
    let _winner = repo
        .create_project(new_project(&swarm, Some(story.clone())))
        .await
        .unwrap();
    let loser = repo
        .create_project(new_project(&swarm, None))
        .await
        .unwrap();
    let err = repo
        .update_project(
            &loser.id,
            ProjectPatch {
                story_id: Some(Some(story.clone())),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, otto_core::Error::Conflict(_)), "got {err:?}");
    // The loser is untouched and can re-read the winner via the story.
    let linked = repo.project_for_story(&story).await.unwrap().unwrap();
    assert_ne!(linked.id, loser.id);
}

#[tokio::test]
async fn link_story_if_unlinked_is_first_writer_wins() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();
    let project = repo
        .create_project(new_project(&swarm, None))
        .await
        .unwrap();
    let a = new_id();
    let b = new_id();
    assert!(repo.link_story_if_unlinked(&project.id, &a).await.unwrap());
    assert!(!repo.link_story_if_unlinked(&project.id, &b).await.unwrap());
    assert_eq!(
        repo.get_project(&project.id).await.unwrap().story_id,
        Some(a.clone())
    );
    // A story rooting another project → Conflict.
    let other = repo
        .create_project(new_project(&swarm, None))
        .await
        .unwrap();
    let err = repo
        .link_story_if_unlinked(&other.id, &a)
        .await
        .unwrap_err();
    assert!(matches!(err, otto_core::Error::Conflict(_)), "got {err:?}");
}

/// S5: the claim is atomic — only the first caller moves a `todo` task to
/// `in_progress`; a racing tick sees `false` and must not dispatch it.
#[tokio::test]
async fn claim_task_is_first_writer_wins_and_keeps_an_assignee() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();
    let agent = new_id();
    let task = repo.create_task(new_task(&swarm, "todo")).await.unwrap();
    assert!(repo.claim_task(&task.id, Some(&agent)).await.unwrap());
    assert!(!repo.claim_task(&task.id, Some(&new_id())).await.unwrap());
    let t = repo.get_task(&task.id).await.unwrap();
    assert_eq!(t.status, "in_progress");
    assert_eq!(t.assignee_agent_id.as_deref(), Some(agent.as_str()));
    // An existing assignee is never overwritten by the claim.
    let mut nt = new_task(&swarm, "todo");
    let owner = new_id();
    nt.assignee_agent_id = Some(owner.clone());
    let owned = repo.create_task(nt).await.unwrap();
    assert!(repo.claim_task(&owned.id, Some(&agent)).await.unwrap());
    assert_eq!(
        repo.get_task(&owned.id).await.unwrap().assignee_agent_id,
        Some(owner)
    );
    // Not todo → never claimed.
    let blocked = repo.create_task(new_task(&swarm, "blocked")).await.unwrap();
    assert!(!repo.claim_task(&blocked.id, Some(&agent)).await.unwrap());
}

#[tokio::test]
async fn refund_task_attempt_never_goes_negative() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let task = repo.create_task(new_task(&new_id(), "todo")).await.unwrap();
    repo.bump_task_attempt(&task.id).await.unwrap();
    repo.refund_task_attempt(&task.id).await.unwrap();
    repo.refund_task_attempt(&task.id).await.unwrap();
    assert_eq!(repo.get_task(&task.id).await.unwrap().attempts, 0);
}

/// S4-24: the standing-goal replace is a full swap, never an append.
#[tokio::test]
async fn replace_standing_goals_swaps_the_set() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let sid = new_id();
    let goal = |t: &str| NewGoal {
        swarm_id: sid.clone(),
        workspace_id: "w".into(),
        project_id: None,
        task_id: None,
        kind: "standing".into(),
        title: t.into(),
        description: String::new(),
        metric: None,
        comparator: None,
        target_value: None,
        block_value: None,
        verify_cmd: None,
        max_retries: 2,
        blocking: false,
        order_idx: 0,
        created_by: "u".into(),
    };
    let a = repo
        .replace_standing_goals(&sid, vec![goal("a"), goal("b")])
        .await
        .unwrap();
    assert_eq!(a.len(), 2);
    let b = repo
        .replace_standing_goals(&sid, vec![goal("c")])
        .await
        .unwrap();
    assert_eq!(
        b.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(),
        ["c"]
    );
}

/// S4-07: the task-status CAS only writes from an expected status.
#[tokio::test]
async fn set_task_status_if_is_a_compare_and_set() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let task = repo
        .create_task(new_task(&new_id(), "verifying"))
        .await
        .unwrap();
    assert!(repo
        .set_task_status_if(&task.id, &["verifying"], "blocked")
        .await
        .unwrap());
    // The operator moved it on; a late controller write is a no-op.
    assert!(!repo
        .set_task_status_if(&task.id, &["verifying"], "done")
        .await
        .unwrap());
    assert_eq!(repo.get_task(&task.id).await.unwrap().status, "blocked");
}

/// S1: a stop never clobbers a finished run, and a late finish never
/// overwrites `stopped`.
#[tokio::test]
async fn update_run_if_status_is_a_compare_and_set() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let run = repo.create_run(new_run(&new_id())).await.unwrap();
    let stopped = repo
        .update_run_if_status(
            &run.id,
            &["queued", "running", "waiting"],
            RunPatch {
                status: Some("stopped".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(stopped.map(|r| r.status).as_deref(), Some("stopped"));
    let late_done = repo
        .update_run_if_status(
            &run.id,
            &["queued", "running", "waiting"],
            RunPatch {
                status: Some("done".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(late_done.is_none());
    assert_eq!(repo.get_run(&run.id).await.unwrap().status, "stopped");
}

/// S2: a pause stops in-flight runs with the "paused" reason.
#[tokio::test]
async fn stop_active_runs_with_reason_marks_only_in_flight_runs() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();
    let live = repo.create_run(new_run(&swarm)).await.unwrap();
    let done = repo.create_run(new_run(&swarm)).await.unwrap();
    repo.update_run(
        &done.id,
        RunPatch {
            status: Some("done".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let ids = repo
        .stop_active_runs_with_reason(&swarm, "paused")
        .await
        .unwrap();
    assert_eq!(ids, vec![live.id.clone()]);
    let r = repo.get_run(&live.id).await.unwrap();
    assert_eq!(
        (r.status.as_str(), r.error.as_deref()),
        ("stopped", Some("paused"))
    );
    assert_eq!(repo.get_run(&done.id).await.unwrap().status, "done");
}

/// S4: a restart re-queues tasks its dead turns left `in_progress`, but a
/// delegated parent whose children are still open stays in progress.
#[tokio::test]
async fn fail_running_requeues_stranded_in_progress_tasks() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let swarm = new_id();
    let stranded = repo
        .create_task(new_task(&swarm, "in_progress"))
        .await
        .unwrap();
    let parent = repo
        .create_task(new_task(&swarm, "in_progress"))
        .await
        .unwrap();
    let mut child = new_task(&swarm, "todo");
    child.parent_task_id = Some(parent.id.clone());
    repo.create_task(child).await.unwrap();
    let mut nr = new_run(&swarm);
    nr.task_id = Some(stranded.id.clone());
    repo.create_run(nr).await.unwrap(); // queued — killed by the restart

    repo.fail_running("interrupted").await.unwrap();
    assert_eq!(repo.get_task(&stranded.id).await.unwrap().status, "todo");
    assert_eq!(
        repo.get_task(&parent.id).await.unwrap().status,
        "in_progress"
    );
}

/// Perf §15 F3/F2: busy agents in one read; task counts by status in SQL.
#[tokio::test]
async fn busy_agents_and_task_status_counts() {
    let repo = SwarmRepo::new(mem_pool().await);
    let swarm = repo.create_swarm(new_swarm(&new_id())).await.unwrap();
    let task = repo.create_task(new_task(&swarm.id, "todo")).await.unwrap();
    repo.create_task(new_task(&swarm.id, "todo")).await.unwrap();
    repo.create_task(new_task(&swarm.id, "done")).await.unwrap();
    let (a1, a2): (Id, Id) = (new_id(), new_id());
    let mk = |agent: &Id| NewRun {
        swarm_id: swarm.id.clone(),
        workspace_id: swarm.workspace_id.clone(),
        project_id: Some(task.project_id.clone()),
        task_id: Some(task.id.clone()),
        agent_id: agent.clone(),
        kind: "task".into(),
        trigger: "coordinator".into(),
    };
    repo.create_run(mk(&a1)).await.unwrap();
    let r2 = repo.create_run(mk(&a2)).await.unwrap();
    repo.update_run(
        &r2.id,
        RunPatch {
            status: Some("done".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let busy = repo.busy_agents(&swarm.id).await.unwrap();
    assert!(busy.contains(&a1));
    assert!(
        !busy.contains(&a2),
        "a finished run doesn't keep its agent busy"
    );
    assert_eq!(
        busy.contains(&a1),
        repo.agent_has_active_run(&a1).await.unwrap()
    );
    let counts = repo.task_status_counts(&swarm.id).await.unwrap();
    assert_eq!(counts.get("todo"), Some(&2));
    assert_eq!(counts.get("done"), Some(&1));
    assert_eq!(repo.count_tasks(&swarm.id).await.unwrap(), 3);
}

/// Perf §15 F1/M1: every per-tick / per-event swarm query is an index
/// range read (covering where it aggregates) — no table scan, no temp
/// B-tree sort.
#[tokio::test]
async fn hot_swarm_queries_use_indexes() {
    let repo = SwarmRepo::new(mem_pool().await);
    for (sql, want) in [
        // workgraph projector: project cost on every swarm event
        (
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM swarm_runs WHERE project_id = 'p'",
            "USING COVERING INDEX idx_swarm_runs_project_cost",
        ),
        // swarm_spend / total_cost: every tick
        (
            "SELECT COUNT(*) AS n, COALESCE(SUM(cost_usd), 0.0) AS spend \
             FROM swarm_runs WHERE swarm_id = 's'",
            "USING COVERING INDEX idx_swarm_runs_swarm_cost",
        ),
        // ready_tasks: every tick
        (
            "SELECT * FROM swarm_tasks WHERE swarm_id = 's' AND status = 'todo' \
             ORDER BY order_idx, created_at",
            "idx_swarm_tasks_swarm_status_order",
        ),
        (
            "SELECT id FROM swarm_tasks WHERE swarm_id = 's' AND status = 'done'",
            "idx_swarm_tasks_swarm_status_order",
        ),
        // busy_agents: once per tick
        (
            "SELECT DISTINCT agent_id FROM swarm_runs \
             WHERE swarm_id = 's' AND status IN ('queued','running','waiting')",
            "USING INDEX",
        ),
        (
            "SELECT status, COUNT(*) FROM swarm_tasks WHERE swarm_id = 's' GROUP BY status",
            "USING COVERING INDEX idx_swarm_tasks_swarm_status_order",
        ),
    ] {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
            .fetch_all(&repo.pool)
            .await
            .unwrap();
        let plan = rows
            .iter()
            .map(|r| r.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(plan.contains(want), "{sql}: {plan}");
        assert!(!plan.contains("SCAN swarm_"), "{sql}: {plan}");
        // Acceptable temp B-trees, bounded by the matched rows rather
        // than the table: `ready_tasks` sorts its few `todo` rows by
        // created_at after the index order, and `busy_agents`' DISTINCT
        // runs over the live runs only (≤ the parallel cap).
        if !sql.contains("created_at") && !sql.contains("DISTINCT") {
            assert!(!plan.contains("TEMP B-TREE"), "{sql}: {plan}");
        }
    }
}
fn new_agent(swarm: &Swarm, schedule: Option<Value>) -> NewAgent {
    NewAgent {
        swarm_id: swarm.id.clone(),
        workspace_id: swarm.workspace_id.clone(),
        name: "researcher".into(),
        title: String::new(),
        reports_to: None,
        provider: "claude".into(),
        model: None,
        soul_name: None,
        soul_md: None,
        specialization: String::new(),
        scope_md: String::new(),
        skills: json!([]),
        schedule,
        cwd_mode: None,
        avatar: String::new(),
        order_idx: 0,
        created_by: new_id(),
    }
}

async fn active_swarm(pool: &DbPool, repo: &SwarmRepo) -> Swarm {
    let swarm = repo.create_swarm(new_swarm(&new_id())).await.unwrap();
    sqlx::query("UPDATE swarms SET status = 'active' WHERE id = ?")
        .bind(&swarm.id)
        .execute(pool)
        .await
        .unwrap();
    swarm
}

fn sched_run(swarm: &Swarm, agent: &Id) -> NewRun {
    NewRun {
        swarm_id: swarm.id.clone(),
        workspace_id: swarm.workspace_id.clone(),
        project_id: None,
        task_id: None,
        agent_id: agent.clone(),
        kind: "scheduled".into(),
        trigger: "scheduled".into(),
    }
}

/// Finding 1: `schedule: Some(None)` (the editor's unchecked "Scheduled
/// runs" → JSON `null`) clears the schedule; an absent key keeps it.
#[tokio::test]
async fn agent_schedule_null_clears_absent_keeps() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool.clone());
    let swarm = active_swarm(&pool, &repo).await;
    let sched = json!({"cadence":"daily","at":"09:00","enabled":true,"directive":"d"});
    let agent = repo
        .create_agent(new_agent(&swarm, Some(sched)))
        .await
        .unwrap();
    assert!(
        agent.schedule.as_ref().unwrap().get("armed_at").is_some(),
        "create arms"
    );

    let kept = repo
        .update_agent(
            &agent.id,
            AgentPatch {
                name: Some("x".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(kept.schedule, agent.schedule);

    let cleared = repo
        .update_agent(
            &agent.id,
            AgentPatch {
                schedule: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        cleared.schedule.is_none(),
        "null must clear the stored schedule"
    );
}

/// Finding 2: an edit that re-sends the schedule without its cursor (the
/// editor rebuilds it from form fields) must keep the stored `last_run`
/// (and not re-arm on a directive-only change); a cadence change re-arms.
#[tokio::test]
async fn agent_edit_carries_schedule_cursor() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool.clone());
    let swarm = active_swarm(&pool, &repo).await;
    let sched = json!({"cadence":"interval","every_min":60,"enabled":true,"directive":"a"});
    let agent = repo
        .create_agent(new_agent(&swarm, Some(sched)))
        .await
        .unwrap();
    let seen = agent.schedule.clone().unwrap();
    let fired = "2026-10-05T09:00:00+00:00";
    let run = repo
        .reserve_scheduled_run(sched_run(&swarm, &agent.id), &seen, fired)
        .await
        .unwrap();
    assert!(run.is_some());
    let after_fire = repo.get_agent(&agent.id).await.unwrap().schedule.unwrap();
    assert_eq!(after_fire["last_run"], json!(fired));

    let edited = json!({"cadence":"interval","every_min":60,"enabled":true,"directive":"b",
                        "last_run": null});
    let a2 = repo
        .update_agent(
            &agent.id,
            AgentPatch {
                schedule: Some(Some(edited)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let s2 = a2.schedule.unwrap();
    assert_eq!(
        s2["last_run"],
        json!(fired),
        "edit must not erase the cursor"
    );
    assert_eq!(
        s2["armed_at"], seen["armed_at"],
        "directive edit must not re-arm"
    );
    assert_eq!(s2["directive"], json!("b"));

    let recadenced = json!({"cadence":"interval","every_min":30,"enabled":true,"directive":"b"});
    let a3 = repo
        .update_agent(
            &agent.id,
            AgentPatch {
                schedule: Some(Some(recadenced)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let s3 = a3.schedule.unwrap();
    assert_eq!(s3["last_run"], json!(fired));
    assert_ne!(s3["armed_at"], seen["armed_at"], "cadence change re-arms");
}

/// Finding 4: the scheduler's cursor write is a compare-and-set on the
/// schedule it judged due — a concurrent edit makes it stand down instead
/// of reverting the edit, and no run is reserved.
#[tokio::test]
async fn scheduled_reservation_never_reverts_a_concurrent_edit() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool.clone());
    let swarm = active_swarm(&pool, &repo).await;
    let sched = json!({"cadence":"interval","every_min":60,"enabled":true,"directive":"a"});
    let agent = repo
        .create_agent(new_agent(&swarm, Some(sched)))
        .await
        .unwrap();
    let seen = agent.schedule.clone().unwrap();
    // The user edits between the scheduler's read and its admission.
    let edited = json!({"cadence":"interval","every_min":60,"enabled":true,"directive":"new"});
    repo.update_agent(
        &agent.id,
        AgentPatch {
            schedule: Some(Some(edited)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let run = repo
        .reserve_scheduled_run(
            sched_run(&swarm, &agent.id),
            &seen,
            "2026-10-05T09:00:00+00:00",
        )
        .await
        .unwrap();
    assert!(run.is_none(), "stale tick must stand down");
    let now = repo.get_agent(&agent.id).await.unwrap().schedule.unwrap();
    assert_eq!(now["directive"], json!("new"));
    assert!(now.get("last_run").is_none());
    assert_eq!(repo.active_run_count(&swarm.id).await.unwrap(), 0);
}

/// Finding 9: a daemon restart refunds the attempt its interrupted run was
/// charged (floor 0) while stopping the run.
#[tokio::test]
async fn fail_running_refunds_interrupted_attempt() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool.clone());
    let swarm = active_swarm(&pool, &repo).await;
    let task = repo
        .create_task(new_task(&swarm.id, "in_progress"))
        .await
        .unwrap();
    let idle = repo.create_task(new_task(&swarm.id, "todo")).await.unwrap();
    assert_eq!(repo.bump_task_attempt(&task.id).await.unwrap(), 1);
    assert_eq!(repo.bump_task_attempt(&idle.id).await.unwrap(), 1);
    let mut r = new_run(&swarm.id);
    r.task_id = Some(task.id.clone());
    repo.create_run(r).await.unwrap();
    assert_eq!(repo.fail_running("restart").await.unwrap(), 1);
    assert_eq!(repo.get_task(&task.id).await.unwrap().attempts, 0);
    assert_eq!(
        repo.get_task(&idle.id).await.unwrap().attempts,
        1,
        "no run, no refund"
    );
}

/// A cut-short verify / fix turn was never charged an attempt, so a
/// restart must not refund one.
#[tokio::test]
async fn fail_running_does_not_refund_uncharged_verify_turns() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool.clone());
    let swarm = active_swarm(&pool, &repo).await;
    let task = repo
        .create_task(new_task(&swarm.id, "verifying"))
        .await
        .unwrap();
    assert_eq!(repo.bump_task_attempt(&task.id).await.unwrap(), 1);
    let mut r = new_run(&swarm.id);
    r.task_id = Some(task.id.clone());
    r.kind = "verify".into();
    repo.create_run(r).await.unwrap();
    repo.fail_running("restart").await.unwrap();
    assert_eq!(repo.get_task(&task.id).await.unwrap().attempts, 1);
}

/// data-integrity-4-06: deleting a task, or clearing its project's board,
/// takes the task's goals with it (no FK cascades on `swarm_goals.task_id`),
/// while a project-level goal survives a board clear.
#[tokio::test]
async fn task_delete_and_board_clear_remove_task_goals() {
    let pool = mem_pool().await;
    let repo = SwarmRepo::new(pool);
    let sid = new_id();
    let goal = |project: Option<Id>, task: Option<Id>| NewGoal {
        swarm_id: sid.clone(),
        workspace_id: "w".into(),
        project_id: project,
        task_id: task,
        kind: "explicit".into(),
        title: "g".into(),
        description: String::new(),
        metric: None,
        comparator: None,
        target_value: None,
        block_value: None,
        verify_cmd: None,
        max_retries: 2,
        blocking: true,
        order_idx: 0,
        created_by: "u".into(),
    };
    // delete_task
    let lone = repo.create_task(new_task(&sid, "todo")).await.unwrap();
    let lone_goal = repo
        .create_goal(goal(Some(lone.project_id.clone()), Some(lone.id.clone())))
        .await
        .unwrap();
    repo.delete_task(&lone.id).await.unwrap();
    assert!(
        repo.get_goal(&lone_goal.id).await.is_err(),
        "task goal kept"
    );

    // clear_project_board
    let nt = new_task(&sid, "todo");
    let pid = nt.project_id.clone();
    let t = repo.create_task(nt).await.unwrap();
    let mut nt2 = new_task(&sid, "todo");
    nt2.project_id = pid.clone();
    nt2.title = "second".into();
    let t2 = repo.create_task(nt2).await.unwrap();
    let g1 = repo
        .create_goal(goal(Some(pid.clone()), Some(t.id.clone())))
        .await
        .unwrap();
    let g2 = repo
        .create_goal(goal(Some(pid.clone()), Some(t2.id.clone())))
        .await
        .unwrap();
    let project_goal = repo
        .create_goal(goal(Some(pid.clone()), None))
        .await
        .unwrap();
    let (tasks, _) = repo.clear_project_board(&pid).await.unwrap();
    assert_eq!(tasks, 2);
    assert!(repo.get_goal(&g1.id).await.is_err());
    assert!(repo.get_goal(&g2.id).await.is_err());
    assert!(
        repo.get_goal(&project_goal.id).await.is_ok(),
        "a project-level goal belongs to the project, not the board"
    );
}

/// access-control-5-01 (migration 0195): a task nobody rewrote has no prompt
/// editor; the recorded one reads back and a later editor replaces it. An
/// ordinary `update_task` (status move) does not touch it.
#[tokio::test]
async fn task_prompt_editor_round_trips_and_survives_a_status_move() {
    let repo = SwarmRepo::new(mem_pool().await);
    let swarm = new_id();
    let t = repo.create_task(new_task(&swarm, "todo")).await.unwrap();
    assert_eq!(repo.task_prompt_editor(&t.id).await.unwrap(), None);
    repo.set_task_prompt_editor(&t.id, "bob").await.unwrap();
    repo.update_task(
        &t.id,
        TaskPatch {
            status: Some("in_progress".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        repo.task_prompt_editor(&t.id).await.unwrap().as_deref(),
        Some("bob")
    );
    repo.set_task_prompt_editor(&t.id, "carol").await.unwrap();
    assert_eq!(
        repo.task_prompt_editor(&t.id).await.unwrap().as_deref(),
        Some("carol")
    );
}
