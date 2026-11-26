//! The run-view columns (migrations 0233/0234): the overview stored with the
//! progress write and ignored once stale, the person count, the "Needs you"
//! filter, holds and goal workflows under run retention, and the bounded
//! progress projection of the new node fields.

use super::*;
use chrono::TimeZone;
use otto_core::agent_runtime::{Attention, AttentionKind, TranscriptDigest};
use otto_core::workflows::{
    ChildThread, HoldKind, HumanEntry, PhaseMark, RunHold, RunLimits, RunState, StepPhase,
    StepThread,
};
use serde_json::json;

async fn mem_pool() -> DbPool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(false);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    pool.into()
}

fn at(s: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, s).unwrap()
}

/// A running step waiting on a person.
fn parked(id: &str) -> NodeRunState {
    NodeRunState {
        node_id: id.into(),
        status: NodeStatus::Running,
        phase: Some(StepPhase::NeedsYou),
        attention: Some(Attention::new(AttentionKind::Approval, "Approve", at(1))),
        ..Default::default()
    }
}

fn overview(state: RunState) -> RunOverview {
    RunOverview {
        state,
        headline: "Approve posting 6 comments".into(),
        steps_total: 3,
        ..Default::default()
    }
}

async fn fixture() -> (DbPool, WorkflowsRepo, Workflow) {
    let pool = mem_pool().await;
    let repo = WorkflowsRepo::new(pool.clone());
    let wf = repo
        .create(
            &"ws".into(),
            "WF",
            "",
            "",
            &WorkflowGraph::default(),
            &"u".into(),
        )
        .await
        .unwrap();
    (pool, repo, wf)
}

async fn run_of(repo: &WorkflowsRepo, wf: &Workflow) -> Id {
    repo.create_run(&wf.id, &wf.workspace_id, &json!({}), None)
        .await
        .unwrap()
        .id
}

async fn cols(pool: &DbPool, id: &Id) -> (i64, Option<String>, i64) {
    sqlx::query_as("SELECT attention, overview_json, rev FROM workflow_runs WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// proves: the overview is written by the progress write itself, stamped
/// with that write's `rev`, and every reader (run, active list, history,
/// /progress) serves it; `attention` counts the running steps on a person.
#[tokio::test]
async fn overview_written_with_nodes_json_in_one_statement() {
    let (pool, repo, wf) = fixture().await;
    let id = run_of(&repo, &wf).await;
    repo.update_run(&id, RunStatus::Running, &[], None, false)
        .await
        .unwrap();
    let nodes = [parked("a"), parked("b"), NodeRunState::default()];
    let rev = repo
        .update_run_progress_with(&id, &nodes, Some(&overview(RunState::NeedsYou)))
        .await
        .unwrap();
    let (attention, json, row_rev) = cols(&pool, &id).await;
    assert_eq!((attention, row_rev), (2, rev));
    let stored: RunOverview = serde_json::from_str(&json.unwrap()).unwrap();
    assert_eq!(stored.rev, rev, "stamped with the write's own rev");

    let run = repo.get_run(&id).await.unwrap();
    assert_eq!(
        run.overview.as_ref().map(|o| o.state),
        Some(RunState::NeedsYou)
    );
    let active = repo.list_active_runs(&wf.workspace_id).await.unwrap();
    assert_eq!(active[0].overview, run.overview);
    let page = crate::workflow_run_search::search_runs(
        &pool,
        &crate::workflow_run_search::RunSearch {
            workspace_id: wf.workspace_id.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.items[0].overview, run.overview);
    let progress = crate::workflow_progress::progress(&pool, &id, None)
        .await
        .unwrap();
    assert_eq!(progress["run"]["summary"], true, "DR-003: the marker stays");
    assert_eq!(progress["run"]["overview"]["state"], "needs_you");
    assert_eq!(progress["run"]["overview"]["rev"], rev);
}

/// proves: a write that does not refresh the overview (a progress write
/// without one, the previous build, a raw write the 0145 trigger bumps)
/// leaves it stale, and stale reads as absent everywhere.
#[tokio::test]
async fn a_stale_overview_reads_as_absent() {
    let (pool, repo, wf) = fixture().await;
    let id = run_of(&repo, &wf).await;
    let ov = overview(RunState::Running);
    repo.update_run_progress_with(&id, &[parked("a")], Some(&ov))
        .await
        .unwrap();
    assert!(repo.get_run(&id).await.unwrap().overview.is_some());
    // The previous build's progress write: nodes + rev, no overview.
    repo.update_run_progress(&id, &[parked("a")]).await.unwrap();
    assert!(repo.get_run(&id).await.unwrap().overview.is_none());

    repo.update_run_progress_with(&id, &[parked("a")], Some(&ov))
        .await
        .unwrap();
    // A raw write (archive restore, manual SQL): the 0145 trigger bumps rev
    // and drops the projection, so the overview no longer matches.
    sqlx::query("UPDATE workflow_runs SET nodes_json = '[]' WHERE id = ?")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, json, _) = cols(&pool, &id).await;
    assert!(json.is_some(), "the column is left as it was");
    assert!(repo.get_run(&id).await.unwrap().overview.is_none());
    let progress = crate::workflow_progress::progress(&pool, &id, None)
        .await
        .unwrap();
    assert!(progress["run"]["overview"].is_null());
    assert!(repo.list_active_runs(&wf.workspace_id).await.unwrap()[0]
        .overview
        .is_none());
}

/// proves: the 0130/0145 progress trigger still fires (raw body writes lose
/// their projection and bump rev) with the new columns on the table, and a
/// repository write still does it in one statement (projection kept).
#[tokio::test]
async fn progress_triggers_fire_with_new_columns() {
    let (pool, repo, wf) = fixture().await;
    let id = run_of(&repo, &wf).await;
    let rev = repo
        .update_run_progress_with(&id, &[parked("a")], Some(&overview(RunState::Running)))
        .await
        .unwrap();
    let projection: Option<String> =
        sqlx::query_scalar("SELECT progress_json FROM workflow_runs WHERE id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(projection.is_some());
    sqlx::query("UPDATE workflow_runs SET nodes_json = '[]' WHERE id = ?")
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    let (projection, new_rev): (Option<String>, i64) =
        sqlx::query_as("SELECT progress_json, rev FROM workflow_runs WHERE id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((projection, new_rev), (None, rev + 1));
}

/// proves (DR-014): every writer that changes a run's status or decision
/// clears the person count and the stored overview in the same statement,
/// so neither outlives the state it described.
#[tokio::test]
async fn every_status_writer_clears_attention_and_overview() {
    let (pool, repo, wf) = fixture().await;
    let id = run_of(&repo, &wf).await;
    let fresh = |id: Id| {
        let (pool, repo) = (pool.clone(), repo.clone());
        async move {
            sqlx::query(
                "UPDATE workflow_runs SET status = 'running', finished_at = NULL WHERE id = ?",
            )
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
            repo.update_run_progress_with(&id, &[parked("a")], Some(&overview(RunState::NeedsYou)))
                .await
                .unwrap();
            let (attention, json, _) = cols(&pool, &id).await;
            assert_eq!((attention, json.is_some()), (1, true));
        }
    };
    let cleared = |what: &'static str| {
        let (pool, id) = (pool.clone(), id.clone());
        async move {
            let (attention, json, _) = cols(&pool, &id).await;
            assert_eq!((attention, json), (0, None), "{what}");
        }
    };

    fresh(id.clone()).await;
    repo.update_run(&id, RunStatus::Success, &[], None, true)
        .await
        .unwrap();
    cleared("update_run (terminal)").await;

    fresh(id.clone()).await;
    repo.update_run_if(
        &id,
        &[RunStatus::Running],
        RunStatus::Error,
        &[parked("a")],
        Some("x"),
        true,
    )
    .await
    .unwrap()
    .unwrap();
    cleared("update_run_if (terminal; settled nodes are not counted)").await;

    fresh(id.clone()).await;
    repo.request_cancel(&id).await.unwrap().unwrap();
    cleared("request_cancel").await;

    repo.prepare_retry(&id, &[], false, &Default::default())
        .await
        .unwrap();
    cleared("prepare_retry").await;

    fresh(id.clone()).await;
    repo.prepare_resume(&id, &[parked("a")], "{}", true)
        .await
        .unwrap();
    cleared("prepare_resume").await;

    fresh(id.clone()).await;
    repo.fail_interrupted_runs("interrupted").await.unwrap();
    cleared("fail_interrupted_runs").await;

    repo.reopen_run(&id).await.unwrap();
    cleared("reopen_run").await;

    fresh(id.clone()).await;
    sqlx::query(
        "UPDATE workflow_runs SET waiting_approval = 1, approval_node_id = 'g' WHERE id = ?",
    )
    .bind(&id)
    .execute(&pool)
    .await
    .unwrap();
    repo.record_approval(&id, "g", Some(&"u".into()), "ok", None)
        .await
        .unwrap();
    let (_, json, _) = cols(&pool, &id).await;
    assert_eq!(json, None, "record_approval drops the needs-you overview");

    // A running status write keeps counting the parked steps it carries.
    repo.update_run_if(
        &id,
        &[RunStatus::Running],
        RunStatus::Running,
        &[parked("a"), parked("b")],
        None,
        false,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(cols(&pool, &id).await.0, 2);
}

/// proves: `set_overview` stores the overview for the row's current rev only.
#[tokio::test]
async fn set_overview_applies_only_to_the_rev_it_was_computed_for() {
    let (_pool, repo, wf) = fixture().await;
    let id = run_of(&repo, &wf).await;
    let rev = repo
        .update_run(&id, RunStatus::Success, &[], None, true)
        .await
        .unwrap();
    assert!(!repo
        .set_overview(&id, rev - 1, &overview(RunState::Succeeded))
        .await
        .unwrap());
    assert!(repo.get_run(&id).await.unwrap().overview.is_none());
    assert!(repo
        .set_overview(&id, rev, &overview(RunState::Succeeded))
        .await
        .unwrap());
    let run = repo.get_run(&id).await.unwrap();
    assert_eq!(run.rev, rev, "not a progress write");
    assert_eq!(
        run.overview.map(|o| (o.state, o.rev)),
        Some((RunState::Succeeded, rev))
    );
}

/// proves: `attention=1` lists exactly the runs waiting on a person — a
/// running step on a person, an approval pause, a blocked or exhausted hold —
/// and the scan is served by the 0233 partial index.
#[tokio::test]
async fn needs_you_filter_is_scoped_to_running_or_held_runs() {
    let (pool, repo, wf) = fixture().await;
    let mut want = Vec::new();
    for (sql, expected) in [
        (
            "UPDATE workflow_runs SET status = 'running', attention = 1 WHERE id = ?",
            true,
        ),
        (
            "UPDATE workflow_runs SET status = 'running', waiting_approval = 1 WHERE id = ?",
            true,
        ),
        (
            r#"UPDATE workflow_runs SET status = 'error', hold_json = '{"kind":"blocked"}' WHERE id = ?"#,
            true,
        ),
        (
            r#"UPDATE workflow_runs SET status = 'error', hold_json = '{"kind":"exhausted"}' WHERE id = ?"#,
            true,
        ),
        (
            r#"UPDATE workflow_runs SET status = 'canceled', hold_json = '{"kind":"paused"}' WHERE id = ?"#,
            false,
        ),
        (
            "UPDATE workflow_runs SET status = 'running' WHERE id = ?",
            false,
        ),
        // A settled run never needs you, whatever a stale count says.
        (
            "UPDATE workflow_runs SET status = 'success', attention = 3 WHERE id = ?",
            false,
        ),
    ] {
        let id = run_of(&repo, &wf).await;
        sqlx::query(sql).bind(&id).execute(&pool).await.unwrap();
        if expected {
            want.push(id);
        }
    }
    let page = crate::workflow_run_search::search_runs(
        &pool,
        &crate::workflow_run_search::RunSearch {
            workspace_id: wf.workspace_id.clone(),
            attention: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut got: Vec<Id> = page.items.into_iter().map(|i| i.id).collect();
    got.sort();
    want.sort();
    assert_eq!(got, want);

    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT id FROM workflow_runs WHERE workspace_id = ?
         AND (status = 'running' OR hold_json IS NOT NULL)
         ORDER BY started_at DESC, id DESC LIMIT 10",
    )
    .bind(&wf.workspace_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        plan.iter()
            .any(|p| p.3.contains("idx_workflow_runs_ws_attention")),
        "{plan:?}"
    );
}

/// proves (DR-032, goal.md §6.1): run retention never prunes a held run, keeps
/// no run of a goal workflow by count and deletes the goal workflow it
/// emptied, and returns the runtime threads the pruned runs name.
#[tokio::test]
async fn prune_keeps_held_runs_drops_old_goal_runs_and_names_their_threads() {
    let (pool, repo, wf) = fixture().await;
    let goal = repo
        .create(
            &"ws".into(),
            "Goal · x",
            "",
            "",
            &WorkflowGraph::default(),
            &"u".into(),
        )
        .await
        .unwrap();
    let old = fmt(Utc::now() - chrono::Duration::days(40));
    sqlx::query(
        "UPDATE workflows SET origin = 'goal', origin_ref = 'mcp', created_at = ? WHERE id = ?",
    )
    .bind(&old)
    .bind(&goal.id)
    .execute(&pool)
    .await
    .unwrap();
    let threaded = NodeRunState {
        node_id: "s".into(),
        status: NodeStatus::Success,
        thread: Some(StepThread {
            thread_id: "t-step".into(),
            runner: "threads".into(),
            provider: "claude".into(),
            model: None,
            effort: None,
            continued_from: None,
            start_ordinal: None,
            turns: 1,
            usage: Default::default(),
            digest: None,
            closed_at: None,
            available: None,
        }),
        children: vec![ChildThread {
            id: "r1".into(),
            thread_id: Some("t-reviewer".into()),
            label: "Correctness · Codex".into(),
            provider: "codex".into(),
            model: None,
            effort: None,
            status: "success".into(),
            started_at: None,
            ended_at: None,
            note: None,
            usage: Default::default(),
        }],
        ..Default::default()
    };
    let pty = NodeRunState {
        node_id: "p".into(),
        thread: Some(StepThread {
            thread_id: "pty-session".into(),
            runner: "pty".into(),
            ..threaded.thread.clone().unwrap()
        }),
        ..Default::default()
    };
    let goal_run = run_of(&repo, &goal).await;
    repo.update_run(&goal_run, RunStatus::Success, &[threaded, pty], None, true)
        .await
        .unwrap();
    let held = run_of(&repo, &wf).await;
    repo.update_run(&held, RunStatus::Error, &[], Some("blocked"), true)
        .await
        .unwrap();
    sqlx::query(r#"UPDATE workflow_runs SET hold_json = '{"kind":"blocked"}' WHERE id = ?"#)
        .bind(&held)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE workflow_runs SET started_at = ?")
        .bind(&old)
        .execute(&pool)
        .await
        .unwrap();
    // A newer old run: with keep = 1 the held run is outside the kept count.
    let newer = run_of(&repo, &wf).await;
    repo.update_run(&newer, RunStatus::Success, &[], None, true)
        .await
        .unwrap();
    sqlx::query("UPDATE workflow_runs SET started_at = ? WHERE id = ?")
        .bind(fmt(Utc::now() - chrono::Duration::days(35)))
        .bind(&newer)
        .execute(&pool)
        .await
        .unwrap();

    let pruned = repo.prune_runs(1, 30).await.unwrap();
    assert_eq!(pruned.run_ids, vec![goal_run.clone()]);
    let mut threads = pruned.thread_ids;
    threads.sort();
    assert_eq!(
        threads,
        ["t-reviewer", "t-step"],
        "PTY session ids are not threads"
    );
    assert!(repo.get_run(&held).await.is_ok(), "a held run waits");
    assert!(
        repo.get(&goal.id).await.is_err(),
        "the emptied goal workflow goes"
    );
    assert!(repo.get(&wf.id).await.is_ok());

    // Cancel clears the hold (goal.md §2.3); the next pass may prune it.
    sqlx::query("UPDATE workflow_runs SET hold_json = NULL WHERE id = ?")
        .bind(&held)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(repo.prune_runs(1, 30).await.unwrap().run_ids, vec![held]);
    assert!(repo.get_run(&newer).await.is_ok());
}

/// proves: the run row decodes hold, limits and the workflow's origin; the
/// hold rides /progress; run_statuses reads status and end without bodies;
/// the chat status message id round-trips.
#[tokio::test]
async fn holds_limits_origin_and_statuses_read_back() {
    let (pool, repo, wf) = fixture().await;
    let id = run_of(&repo, &wf).await;
    let hold = RunHold {
        kind: HoldKind::Exhausted,
        reason: "Out of iterations: 6 of 6 used".into(),
        since: at(5),
        by: None,
        node_ids: vec!["loop".into()],
        iteration: Some(6),
    };
    repo.update_run(&id, RunStatus::Error, &[], Some("out of budget"), true)
        .await
        .unwrap();
    sqlx::query("UPDATE workflow_runs SET hold_json = ?, limits_json = ? WHERE id = ?")
        .bind(serde_json::to_string(&hold).unwrap())
        .bind(r#"{"loops":{"loop":{"max_iterations":9,"unblocked_at":4}}}"#)
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE workflows SET origin = 'goal', origin_ref = 'review:r1' WHERE id = ?")
        .bind(&wf.id)
        .execute(&pool)
        .await
        .unwrap();
    let run = repo.get_run(&id).await.unwrap();
    assert_eq!(run.hold, Some(hold));
    let limits = run.limits.unwrap();
    assert_eq!(limits.loops["loop"].max_iterations, Some(9));
    assert_eq!(limits.loops["loop"].unblocked_at, Some(4));
    let w = repo.get(&wf.id).await.unwrap();
    assert_eq!(
        (w.origin.as_deref(), w.origin_ref.as_deref()),
        (Some("goal"), Some("review:r1"))
    );
    let progress = crate::workflow_progress::progress(&pool, &id, None)
        .await
        .unwrap();
    assert_eq!(progress["run"]["hold"]["kind"], "exhausted");

    let other = run_of(&repo, &wf).await;
    let statuses = repo
        .run_statuses(&[id.clone(), other.clone(), "gone".into()])
        .await
        .unwrap();
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[&id].0, RunStatus::Error);
    assert!(statuses[&id].1.is_some());
    assert_eq!(statuses[&other], (RunStatus::Pending, None));

    assert_eq!(repo.chat_status(&id).await.unwrap(), None);
    let msg = json!({"channel": "slack", "chat": "C1", "thread": "1.2", "message_id": "3.4"});
    repo.set_chat_status(&id, Some(&msg)).await.unwrap();
    assert_eq!(repo.chat_status(&id).await.unwrap(), Some(msg));
}

/// proves: /progress carries the run-view node fields, bounded — the thread
/// without its digest, children without usage, the newest 12 phases and 5
/// human entries — and a node without them projects exactly as before.
#[test]
fn progress_projection_carries_the_bounded_run_view_fields() {
    let plain = NodeRunState {
        node_id: "n".into(),
        ..Default::default()
    };
    let before = crate::workflow_progress::node_summary(&plain).unwrap();
    let mut keys: Vec<String> = before.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "attempts",
            "detail_version",
            "duration_ms",
            "error",
            "has_output",
            "log_count",
            "logs",
            "node_id",
            "output",
            "review_ids",
            "sessions",
            "status"
        ]
    );

    let mut node = parked("n");
    node.step_no = Some(3);
    node.step_file = Some("step3-checkout-pr-branch".into());
    node.error_class = Some("timeout".into());
    node.brief = Some("Checked out #12".into());
    node.phases = (0..20)
        .map(|i| PhaseMark {
            phase: StepPhase::Working,
            at: at(i),
            note: Some(format!("m{i}")),
        })
        .collect();
    node.human_log = (0..8)
        .map(|i| HumanEntry {
            kind: "steer".into(),
            ask: None,
            text: format!("h{i}"),
            met: None,
            revision: None,
            iteration: None,
            by: "A".into(),
            at: at(i),
        })
        .collect();
    node.thread = Some(StepThread {
        thread_id: "t1".into(),
        runner: "threads".into(),
        provider: "claude".into(),
        model: Some("m".into()),
        effort: None,
        continued_from: None,
        start_ordinal: Some(4),
        turns: 2,
        usage: Default::default(),
        digest: Some(TranscriptDigest {
            turns: 2,
            ..Default::default()
        }),
        closed_at: None,
        available: None,
    });
    node.children = vec![ChildThread {
        id: "r1".into(),
        thread_id: None,
        label: "Security · Claude".into(),
        provider: "claude".into(),
        model: None,
        effort: None,
        status: "running".into(),
        started_at: None,
        ended_at: None,
        note: None,
        usage: Default::default(),
    }];
    let v = crate::workflow_progress::node_summary(&node).unwrap();
    assert_eq!(v["step_no"], 3);
    assert_eq!(v["step_file"], "step3-checkout-pr-branch");
    assert_eq!(v["phase"], "needs_you");
    assert_eq!(v["attention"]["kind"], "approval");
    assert_eq!(v["error_class"], "timeout");
    assert_eq!(v["brief"], "Checked out #12");
    let phases = v["phases"].as_array().unwrap();
    assert_eq!((phases.len(), phases[0]["note"].as_str()), (12, Some("m8")));
    let log = v["human_log"].as_array().unwrap();
    assert_eq!((log.len(), log[4]["text"].as_str()), (5, Some("h7")));
    assert_eq!(v["thread"]["start_ordinal"], 4);
    assert!(v["thread"].get("digest").is_none());
    assert!(v["children"][0].get("usage").is_none());
    assert_eq!(v["children"][0]["label"], "Security · Claude");
    assert_ne!(v["detail_version"], before["detail_version"]);
}

fn hold(kind: HoldKind) -> RunHold {
    RunHold {
        kind,
        reason: "Paused by A".into(),
        since: at(9),
        by: Some("u".into()),
        node_ids: vec![],
        iteration: None,
    }
}

/// proves (DR-105): `HOLD_SQL` and `RunHold::is_effective` are the same rule
/// over every kind × status pair, and the run reader exposes only an
/// effective hold — a stale one the previous build left on a row it retried
/// reads as none, and that row is prunable.
#[tokio::test]
async fn the_effective_hold_rule_is_one_rule_in_rust_and_sql() {
    let (pool, repo, wf) = fixture().await;
    let mut cases = Vec::new();
    for kind in [HoldKind::Paused, HoldKind::Blocked, HoldKind::Exhausted] {
        for status in [
            RunStatus::Pending,
            RunStatus::Running,
            RunStatus::Success,
            RunStatus::Error,
            RunStatus::Canceled,
        ] {
            let id = run_of(&repo, &wf).await;
            sqlx::query("UPDATE workflow_runs SET status = ?, hold_json = ? WHERE id = ?")
                .bind(status.as_str())
                .bind(serde_json::to_string(&hold(kind)).unwrap())
                .bind(&id)
                .execute(&pool)
                .await
                .unwrap();
            cases.push((id, kind, status));
        }
    }
    let plain = run_of(&repo, &wf).await;
    let sql = format!("SELECT id FROM workflow_runs WHERE {HOLD_SQL}");
    let held: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(!held.contains(&plain));
    for (id, kind, status) in cases {
        let want = hold(kind).is_effective(status);
        assert_eq!(held.contains(&id), want, "{kind:?} × {status:?}");
        assert_eq!(
            repo.get_run(&id).await.unwrap().hold.is_some(),
            want,
            "{kind:?} × {status:?}"
        );
    }
    let not_held = format!("SELECT COUNT(*) FROM workflow_runs WHERE NOT {HOLD_SQL}");
    let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(not_held))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        n,
        16 - 3,
        "NOT HOLD_SQL is never NULL: a row without a hold counts"
    );

    // A stale hold on an old succeeded run does not stop retention.
    let stale = run_of(&repo, &wf).await;
    sqlx::query(
        "UPDATE workflow_runs SET status = 'success', hold_json = ?, started_at = ? WHERE id = ?",
    )
    .bind(serde_json::to_string(&hold(HoldKind::Blocked)).unwrap())
    .bind(fmt(Utc::now() - chrono::Duration::days(400)))
    .bind(&stale)
    .execute(&pool)
    .await
    .unwrap();
    let pruned = repo.prune_runs(1, 30).await.unwrap();
    assert!(pruned.run_ids.contains(&stale));
}

/// proves (DR-105): the new build's status writers drop a hold in the same
/// statement, so a retried or re-run row never reads as held again.
#[tokio::test]
async fn status_writers_clear_the_hold() {
    let (pool, repo, wf) = fixture().await;
    let id = run_of(&repo, &wf).await;
    let hold_json = || async {
        let h: Option<String> =
            sqlx::query_scalar("SELECT hold_json FROM workflow_runs WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        h
    };
    let set = |status: &'static str| {
        let (pool, id) = (pool.clone(), id.clone());
        async move {
            sqlx::query("UPDATE workflow_runs SET status = ?, hold_json = '{\"kind\":\"paused\"}' WHERE id = ?")
                .bind(status)
                .bind(&id)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    set("canceled").await;
    repo.prepare_retry(&id, &[], false, &Default::default())
        .await
        .unwrap();
    assert_eq!(hold_json().await, None, "prepare_retry");
    set("error").await;
    repo.reopen_run(&id).await.unwrap();
    assert_eq!(hold_json().await, None, "reopen_run");
    set("running").await;
    repo.update_run(&id, RunStatus::Running, &[], None, false)
        .await
        .unwrap();
    assert_eq!(hold_json().await, None, "update_run");
    set("running").await;
    repo.update_run_if(
        &id,
        &[RunStatus::Running],
        RunStatus::Success,
        &[],
        None,
        true,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(hold_json().await, None, "update_run_if");
    set("running").await;
    repo.request_cancel(&id).await.unwrap().unwrap();
    assert_eq!(hold_json().await, None, "cancel_in_transaction");
}

/// proves (DR-106): Pause writes the hold durably before the cancel is
/// announced; a settled run is not paused; the driver's `write_hold` lands
/// only while the hold stands, so a Cancel that cleared it wins; a blocked
/// hold from a running run settles it as `error`.
#[tokio::test]
async fn pause_is_durable_and_cancel_wins_over_a_late_hold_write() {
    let (_pool, repo, wf) = fixture().await;
    let id = run_of(&repo, &wf).await;
    repo.update_run(&id, RunStatus::Running, &[parked("a")], None, false)
        .await
        .unwrap();
    let mut cancels = subscribe_cancels();
    repo.request_pause(&id, &hold(HoldKind::Paused))
        .await
        .unwrap()
        .unwrap();
    let run = repo.get_run(&id).await.unwrap();
    assert_eq!(run.status, RunStatus::Canceled);
    assert_eq!(run.error.as_deref(), Some("Paused by A"));
    assert_eq!(run.hold.as_ref().map(|h| h.kind), Some(HoldKind::Paused));
    assert_eq!(cancels.try_recv().unwrap(), id, "announced after the write");
    assert!(repo
        .request_pause(&id, &hold(HoldKind::Paused))
        .await
        .unwrap()
        .is_none());

    // The driver finalizes: node ids and the pending nodes, under CAS.
    let finalized = RunHold {
        node_ids: vec!["a".into()],
        ..hold(HoldKind::Paused)
    };
    let pending = NodeRunState {
        node_id: "a".into(),
        ..Default::default()
    };
    repo.write_hold(&id, &finalized, std::slice::from_ref(&pending))
        .await
        .unwrap()
        .unwrap();
    let run = repo.get_run(&id).await.unwrap();
    assert_eq!(run.hold.unwrap().node_ids, ["a"]);
    assert_eq!(run.nodes[0].status, NodeStatus::Pending);

    // Cancel on the held run clears the hold; a late hold write is refused.
    assert!(repo.clear_hold(&id).await.unwrap());
    assert!(!repo.clear_hold(&id).await.unwrap());
    assert!(repo
        .write_hold(&id, &finalized, &[pending])
        .await
        .unwrap()
        .is_none());
    let run = repo.get_run(&id).await.unwrap();
    assert_eq!((run.status, run.hold), (RunStatus::Canceled, None));

    let looping = run_of(&repo, &wf).await;
    repo.update_run(&looping, RunStatus::Running, &[], None, false)
        .await
        .unwrap();
    repo.write_hold(&looping, &hold(HoldKind::Blocked), &[])
        .await
        .unwrap()
        .unwrap();
    let run = repo.get_run(&looping).await.unwrap();
    assert_eq!(run.status, RunStatus::Error);
    assert_eq!(run.hold.map(|h| h.kind), Some(HoldKind::Blocked));
    assert!(run.finished_at.is_some());
}

/// proves (DR-101): an effectively held run stays in the active list and is
/// what a chat `resume` / `note` finds, while the orphan sweep's list stays
/// strictly in flight; the held branch uses the 0233 partial index.
#[tokio::test]
async fn held_runs_stay_in_the_active_and_chat_lists() {
    let (pool, repo, wf) = fixture().await;
    let id = repo
        .create_run(
            &wf.id,
            &wf.workspace_id,
            &json!({"channel": "slack", "chat": "C1", "thread": "1.2"}),
            None,
        )
        .await
        .unwrap()
        .id;
    repo.update_run(&id, RunStatus::Running, &[], None, false)
        .await
        .unwrap();
    repo.request_pause(&id, &hold(HoldKind::Paused))
        .await
        .unwrap()
        .unwrap();
    let settled = run_of(&repo, &wf).await;
    repo.update_run(&settled, RunStatus::Success, &[], None, true)
        .await
        .unwrap();

    let active = repo.list_active_runs(&wf.workspace_id).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(
        (active[0].run_id.as_str(), active[0].status),
        (id.as_str(), RunStatus::Canceled)
    );
    assert!(!repo
        .list_active_run_ids_global()
        .await
        .unwrap()
        .contains(&id));
    let found = repo
        .find_active_run_for_chat(&wf.workspace_id, "slack", "C1", Some("1.2"))
        .await
        .unwrap();
    assert_eq!(found, Some(id.clone()));
    repo.clear_hold(&id).await.unwrap();
    assert!(repo
        .list_active_runs(&wf.workspace_id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        repo.find_active_run_for_chat(&wf.workspace_id, "slack", "C1", Some("1.2"))
            .await
            .unwrap(),
        None
    );

    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "EXPLAIN QUERY PLAN SELECT id FROM workflow_runs INDEXED BY idx_workflow_runs_ws_attention
         WHERE workspace_id = ? AND (status = 'running' OR hold_json IS NOT NULL) AND {HOLD_SQL}"
    )))
    .bind(&wf.workspace_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        plan.iter()
            .any(|p| p.3.contains("idx_workflow_runs_ws_attention")),
        "{plan:?}"
    );
}

/// proves (DR-110): a save based on an older version is refused by the same
/// statement that would write it, so two saves from one base never both land.
#[tokio::test]
async fn publish_checks_the_expected_version_atomically() {
    let (_pool, repo, wf) = fixture().await;
    let save = |text: &'static str| {
        let (repo, id) = (repo.clone(), wf.id.clone());
        async move {
            repo.publish(
                &id,
                None,
                None,
                Some(text),
                None,
                None,
                "edited",
                None,
                None,
                Some(1),
            )
            .await
        }
    };
    let (a, b) = tokio::join!(save("one"), save("two"));
    let (ok, err): (Vec<_>, Vec<_>) = [a, b].into_iter().partition(Result::is_ok);
    assert_eq!((ok.len(), err.len()), (1, 1));
    assert!(matches!(err[0], Err(Error::Conflict(_))), "{:?}", err[0]);
    assert_eq!(repo.get(&wf.id).await.unwrap().version, 2);
    let missing = repo
        .publish(
            &"nope".into(),
            None,
            None,
            Some("x"),
            None,
            None,
            "e",
            None,
            None,
            Some(1),
        )
        .await;
    assert!(matches!(missing, Err(Error::NotFound(_))));
}

/// proves: the goal-side writers and lookups read back (origin, limits, lane,
/// the goal a source already started, a workflow's newest run).
#[tokio::test]
async fn goal_writers_and_lookups_round_trip() {
    let (_pool, repo, wf) = fixture().await;
    repo.set_origin(&wf.id, Some("goal"), Some("review:r1"))
        .await
        .unwrap();
    let found = repo
        .find_goal_by_origin_ref(&wf.workspace_id, "review:r1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (found.id, found.origin.as_deref()),
        (wf.id.clone(), Some("goal"))
    );
    assert!(repo
        .find_goal_by_origin_ref(&"other".into(), "review:r1")
        .await
        .unwrap()
        .is_none());
    assert!(repo.latest_run_of(&wf.id).await.unwrap().is_none());
    let first = run_of(&repo, &wf).await;
    let limits: RunLimits =
        serde_json::from_value(json!({"loops": {"loop": {"max_iterations": 9}}})).unwrap();
    repo.set_run_limits(&first, &limits).await.unwrap();
    repo.set_run_lane(&first, Some("long")).await.unwrap();
    assert_eq!(
        repo.run_lane(&first).await.unwrap().as_deref(),
        Some("long")
    );
    let latest = repo.latest_run_of(&wf.id).await.unwrap().unwrap();
    assert_eq!((latest.id.clone(), latest.limits), (first, Some(limits)));
}
