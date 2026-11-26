use super::*;
use otto_core::workflows::{WorkflowEdge, WorkflowNode};
use serde_json::json;

fn t(s: u64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000 + s as i64, 0).unwrap()
}

fn graph() -> WorkflowGraph {
    let node = |id: &str, kind: &str, name: &str| WorkflowNode {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        x: 0.0,
        y: 0.0,
        params: Value::Null,
        retry: None,
    };
    WorkflowGraph {
        nodes: vec![
            node("n1", "manual_trigger", "Start"),
            node("n2", "agent_prompt", "Gather context from the Vault"),
            node("n3", "script", "Go PR gates"),
            node("n4", "agent_prompt", "Cross-repo contract check"),
            node("n5", "review_run", "Multi-agent PR review"),
            node("n6", "post_review_comments", ""),
        ],
        edges: Vec::<WorkflowEdge>::new(),
    }
}

/// A run from JSON node states (every other node pending).
fn run(status: &str, nodes: Value) -> WorkflowRun {
    let mut nodes: Vec<Value> = nodes.as_array().unwrap().clone();
    for id in ["n1", "n2", "n3", "n4", "n5", "n6"] {
        if !nodes.iter().any(|n| n["node_id"] == id) {
            nodes.push(json!({"node_id": id, "status": "pending"}));
        }
    }
    serde_json::from_value(json!({
        "id": "r1", "workflow_id": "wf", "workspace_id": "ws", "status": status,
        "input": {"trigger": "chat", "user": "U1", "chat": "C1"},
        "nodes": nodes, "started_at": t(0), "rev": 7,
    }))
    .unwrap()
}

fn ov(run: &WorkflowRun) -> RunOverview {
    run_overview(run, &graph(), None, Some(1_500_000), t(100))
}

fn done(id: &str) -> Value {
    json!({"node_id": id, "status": "success", "started_at": t(0), "duration_ms": 1000, "step_no": id[1..].parse::<u32>().unwrap()})
}

fn running(id: &str, phase: &str) -> Value {
    json!({"node_id": id, "status": "running", "started_at": t(10), "phase": phase})
}

#[test]
fn effective_hold_matrix() {
    let hold = |kind: HoldKind| RunHold {
        kind,
        reason: "r".into(),
        since: t(0),
        by: None,
        node_ids: vec![],
        iteration: None,
    };
    let statuses = [
        RunStatus::Pending,
        RunStatus::Running,
        RunStatus::Success,
        RunStatus::Error,
        RunStatus::Canceled,
    ];
    for kind in [
        HoldKind::Paused,
        HoldKind::Blocked,
        HoldKind::Exhausted,
        HoldKind::Unknown,
    ] {
        for status in statuses {
            let h = hold(kind);
            let counts = matches!(
                (kind, status),
                (HoldKind::Paused, RunStatus::Canceled)
                    | (HoldKind::Blocked | HoldKind::Exhausted, RunStatus::Error)
            );
            assert_eq!(
                effective_hold(status, Some(&h)).is_some(),
                counts,
                "{kind:?} × {status:?}"
            );
        }
    }
    assert!(effective_hold(RunStatus::Error, None).is_none());
}

#[test]
fn queued_says_its_place_in_line() {
    let r = run("pending", json!([]));
    let o = run_overview(
        &r,
        &graph(),
        Some(QueueInfo {
            position: 2,
            ahead: 1,
            cap: 2,
            lane: None,
        }),
        None,
        t(30),
    );
    assert_eq!(o.state, RunState::Queued);
    assert_eq!(o.headline, "Queued — 2nd in line");
    assert_eq!(o.queue.as_ref().unwrap().position, 2);
    assert_eq!(o.rev, 7);
    assert_eq!(
        o.time.queued_ms, 30_000,
        "a pending run's time is queued time"
    );
    assert_eq!(ordinal(1), "1st");
    assert_eq!(ordinal(3), "3rd");
    assert_eq!(ordinal(11), "11th");
    assert_eq!(ordinal(22), "22nd");
}

#[test]
fn one_running_step_says_what_it_does() {
    let mut step = running("n2", "working");
    step["activity"] =
        json!({"phase": "reading services/payouts.md", "updated_at": t(20), "pending_tasks": 0});
    let o = ov(&run("running", json!([done("n1"), step])));
    assert_eq!(o.state, RunState::Running);
    assert_eq!(
        o.headline,
        "Gather context from the Vault — reading services/payouts.md"
    );
    let now = o.now.unwrap();
    assert_eq!(now.node_ids, ["n2"]);
    assert_eq!(now.phase, StepPhase::Working);
    let o = ov(&run(
        "running",
        json!([done("n1"), running("n3", "working")]),
    ));
    assert_eq!(o.headline, "Go PR gates — working");
}

#[test]
fn a_review_says_how_many_reviewers_are_done() {
    let mut review = running("n5", "working");
    let child = |i: u32, status: &str| json!({"id": format!("c{i}"), "label": format!("Lens {i}"), "provider": "claude", "status": status});
    review["children"] = Value::Array(
        (0..12)
            .map(|i| child(i, if i < 9 { "success" } else { "running" }))
            .collect(),
    );
    let o = ov(&run("running", json!([done("n1"), review])));
    assert_eq!(o.headline, "Multi-agent PR review — 9 of 12 reviewers done");
    assert_eq!(
        o.now.unwrap().fan,
        Some(FanProgress {
            done: 9,
            total: 12,
            running: 3
        })
    );
}

#[test]
fn several_running_steps_are_named() {
    let o = ov(&run(
        "running",
        json!([
            done("n1"),
            running("n2", "working"),
            running("n3", "starting"),
            running("n4", "waiting")
        ]),
    ));
    assert_eq!(o.state, RunState::Running);
    assert_eq!(
        o.headline,
        "3 steps running: Gather context from the Vault, Go PR gates, Cross-repo contract check"
    );
    let o = ov(&run(
        "running",
        json!([
            running("n1", "working"),
            running("n2", "working"),
            running("n3", "working"),
            running("n4", "working")
        ]),
    ));
    assert!(
        o.headline.starts_with("4 steps running: Start, ") && o.headline.ends_with(" …"),
        "{}",
        o.headline
    );
}

#[test]
fn waiting_when_every_running_step_waits() {
    let mut step = running("n2", "waiting");
    step["wait"] = json!({"code": "slots", "text": "Waiting for a free agent slot — 6 of 6 running", "since": t(10)});
    let o = ov(&run(
        "running",
        json!([done("n1"), step, running("n3", "queued")]),
    ));
    assert_eq!(o.state, RunState::Waiting);
    assert_eq!(
        o.headline,
        "Gather context from the Vault: Waiting for a free agent slot — 6 of 6 running"
    );
    assert_eq!(
        o.now.unwrap().wait_text.as_deref(),
        Some("Waiting for a free agent slot — 6 of 6 running")
    );
}

#[test]
fn needs_you_leads_with_the_oldest_ask() {
    let mut a = running("n2", "needs_you");
    a["attention"] = json!({"kind": "question", "text": "Gather context from the Vault asks which repository holds the shared DTOs", "since": t(40)});
    let mut b = running("n6", "needs_you");
    b["attention"] = json!({"kind": "approval", "via": "mcp", "text": "approve posting 6 comments on PR #412", "since": t(30)});
    let o = ov(&run("running", json!([done("n1"), a, b])));
    assert_eq!(o.state, RunState::NeedsYou);
    assert_eq!(o.attention.len(), 2);
    assert_eq!(o.attention[0].node_id, "n6", "oldest first");
    assert_eq!(
        o.attention[0].name, "post_review_comments",
        "an unnamed step shows its kind"
    );
    assert_eq!(
        o.headline,
        "Needs you — approve posting 6 comments on PR #412"
    );

    let mut limit = running("n2", "needs_you");
    limit["attention"] = json!({"kind": "usage_limit", "text": "Claude's usage limit stopped Gather context from the Vault — resets at 14:00", "since": t(30), "resets_at": t(3600)});
    let o = ov(&run("running", json!([limit])));
    assert_eq!(
        o.headline,
        "Claude's usage limit stopped Gather context from the Vault — resets at 14:00"
    );

    // An approval parked the old way (run row only).
    let mut r = run("running", json!([done("n1"), running("n6", "working")]));
    r.waiting_approval = true;
    r.approval_node_id = Some("n6".into());
    assert_eq!(ov(&r).state, RunState::NeedsYou);
    assert_eq!(
        ov(&r).headline,
        "Needs you — approve post_review_comments to continue"
    );
}

#[test]
fn failed_names_the_failing_step_and_its_own_error() {
    let mut f = done("n5");
    f["status"] = json!("error");
    f["error"] =
        json!("the summarizer passed its 20-minute deadline\n65 reviewer findings are saved");
    f["error_class"] = json!("timeout");
    let skipped = json!({"node_id": "n6", "status": "skipped"});
    let o = ov(&run("error", json!([done("n1"), f, skipped])));
    assert_eq!(o.state, RunState::Failed);
    assert_eq!(
        o.headline,
        "Failed at Multi-agent PR review: the summarizer passed its 20-minute deadline"
    );
    let fs = o.failed_step.unwrap();
    assert_eq!(fs.node_id, "n5");
    assert_eq!(fs.error_class.as_deref(), Some("timeout"));
    let mut r = run("error", json!([]));
    r.error = Some("workflow active time budget exhausted".into());
    assert_eq!(
        ov(&r).headline,
        "Failed: workflow active time budget exhausted"
    );
}

#[test]
fn a_passed_review_leads_with_its_score_and_what_was_posted() {
    let mut review = done("n5");
    review["output"] = json!({"score": 86, "passed": true});
    let mut post = done("n6");
    post["deliverables"] =
        json!([{"label": "PR comments", "state": "done", "detail": "5 of 5 posted"}]);
    let o = ov(&run("success", json!([done("n1"), review, post])));
    assert_eq!(o.state, RunState::Succeeded);
    assert_eq!(o.headline, "Passed — 86/100, 5 of 5 PR comments posted");
    assert_eq!(o.deliverables.len(), 1);

    let mut review = done("n5");
    review["output"] = json!({"score": 54.4, "passed": false});
    let o = ov(&run("success", json!([done("n1"), review])));
    assert_eq!(o.headline, "Not passed — 54/100");
}

#[test]
fn a_deliverable_that_did_not_land_leads() {
    let mut post = done("n4");
    post["deliverables"] = json!([
        {"label": "PR comments", "state": "done", "detail": "5 of 5 posted"},
        {"label": "Jira comment", "state": "failed", "detail": "no permission to comment on STOR-311"}
    ]);
    let o = ov(&run("success", json!([done("n1"), post])));
    assert_eq!(
        o.headline,
        "Done, but the Jira comment was not posted: no permission to comment on STOR-311"
    );
    let mut post = done("n4");
    post["deliverables"] = json!([{"label": "Confluence comment", "state": "pending_approval"}]);
    assert_eq!(
        ov(&run("success", json!([post]))).headline,
        "Done, but the Confluence comment is waiting for approval"
    );
    let mut post = done("n4");
    post["deliverables"] =
        json!([{"label": "PR comments", "state": "partial", "detail": "3 of 4 posted; 1 failed"}]);
    assert_eq!(
        ov(&run("success", json!([post]))).headline,
        "Done, but the PR comments were only partly posted: 3 of 4 posted; 1 failed"
    );
}

#[test]
fn done_counts_the_steps_that_ran() {
    let branch = json!({"node_id": "n4", "status": "skipped"});
    let chat_skip = json!({"node_id": "n3", "status": "skipped", "started_at": t(5)});
    let o = ov(&run(
        "success",
        json!([
            done("n1"),
            done("n2"),
            chat_skip,
            branch,
            done("n5"),
            done("n6")
        ]),
    ));
    assert_eq!(
        (o.steps_done, o.steps_total),
        (5, 5),
        "a branch-skipped step drops out; a chat skip ran"
    );
    assert_eq!(o.headline, "Done — 5 steps");
    let o = ov(&run(
        "running",
        json!([done("n1"), running("n2", "working")]),
    ));
    assert_eq!((o.steps_done, o.steps_total), (1, 6));
}

#[test]
fn canceled_and_held_runs() {
    let stopped = json!({"node_id": "n3", "status": "skipped", "started_at": t(5), "error_class": "canceled"});
    let o = ov(&run("canceled", json!([done("n1"), stopped])));
    assert_eq!(o.state, RunState::Canceled);
    assert_eq!(o.headline, "Canceled at Go PR gates");

    let mut r = run("canceled", json!([done("n1")]));
    r.hold = Some(RunHold {
        kind: HoldKind::Paused,
        reason: "paused".into(),
        since: t(9),
        by: Some("Dana".into()),
        node_ids: vec!["n2".into()],
        iteration: None,
    });
    let o = ov(&r);
    assert_eq!(o.state, RunState::Paused);
    assert_eq!(
        o.headline,
        "Paused by Dana at Gather context from the Vault"
    );
    assert!(o.hold.is_some());

    let mut r = run("error", json!([done("n1")]));
    r.hold = Some(RunHold {
        kind: HoldKind::Exhausted,
        reason: "3 of 3 iterations used".into(),
        since: t(9),
        by: None,
        node_ids: vec![],
        iteration: Some(3),
    });
    assert_eq!(
        ov(&r).headline,
        "Out of budget: 3 of 3 iterations used — extend to continue"
    );
    r.hold.as_mut().unwrap().kind = HoldKind::Blocked;
    r.hold.as_mut().unwrap().reason = "the same check failed twice".into();
    assert_eq!(ov(&r).headline, "Blocked: the same check failed twice");

    // A stale hold (the previous build retried the row) is ignored.
    let mut r = run("success", json!([done("n1")]));
    r.hold = Some(RunHold {
        kind: HoldKind::Paused,
        reason: "old".into(),
        since: t(1),
        by: None,
        node_ids: vec![],
        iteration: None,
    });
    let o = ov(&r);
    assert_eq!(o.state, RunState::Succeeded);
    assert!(o.hold.is_none());
}

/// Overlapping phases: each instant counts once, in the best bucket; the
/// buckets sum to the wall clock.
#[test]
fn time_buckets_sum_to_the_wall_clock() {
    let a = json!({"node_id": "n2", "status": "success", "started_at": t(0), "duration_ms": 60_000, "phases": [
        {"phase": "queued", "at": t(0)}, {"phase": "waiting", "at": t(10)},
        {"phase": "starting", "at": t(20)}, {"phase": "working", "at": t(25)}]});
    let b = json!({"node_id": "n3", "status": "success", "started_at": t(30), "duration_ms": 50_000, "phases": [
        {"phase": "working", "at": t(30)}, {"phase": "needs_you", "at": t(40)}]});
    let mut r = run("success", json!([a, b]));
    r.finished_at = Some(t(100));
    let o = run_overview(&r, &graph(), None, None, t(500));
    assert_eq!(
        o.time,
        TimeBuckets {
            queued_ms: 10_000,
            waiting_ms: 10_000,
            with_you_ms: 20_000,
            working_ms: 60_000
        }
    );
    let tb = &o.time;
    assert_eq!(
        tb.queued_ms + tb.waiting_ms + tb.with_you_ms + tb.working_ms,
        o.elapsed_ms
    );
    assert_eq!(
        o.elapsed_ms, 100_000,
        "a finished run's clock stops at finished_at"
    );

    // A still-running step without phases (an old row) counts as working.
    let r = run(
        "running",
        json!([{"node_id": "n2", "status": "running", "started_at": t(40)}]),
    );
    let o = run_overview(&r, &graph(), None, None, t(100));
    assert_eq!((o.time.queued_ms, o.time.working_ms), (40_000, 60_000));
}

#[test]
fn usage_sums_steps_and_children_and_origin_comes_from_the_input() {
    let mut a = done("n2");
    a["thread"] = json!({"thread_id": "t1", "runner": "threads", "provider": "claude", "usage": {"input_tokens": 10, "output_tokens": 2, "cost_usd": 0.5}});
    let mut b = done("n5");
    b["children"] = json!([{"id": "c", "label": "L", "provider": "codex", "status": "success", "usage": {"input_tokens": 5, "output_tokens": 1, "cached_input_tokens": 3}}]);
    let o = ov(&run("success", json!([a, b])));
    assert_eq!(o.usage.input_tokens, 15);
    assert_eq!(o.usage.output_tokens, 3);
    assert_eq!(o.usage.cached_input_tokens, Some(3));
    assert_eq!(o.usage.cost_usd, Some(0.5));
    let origin = o.origin.unwrap();
    assert_eq!(
        (origin.kind.as_str(), origin.user_name.as_deref()),
        ("chat", Some("U1"))
    );
    assert_eq!(o.typical_ms, Some(1_500_000));

    // Origin and subject recorded at admission are carried over.
    let mut r = run("running", json!([]));
    r.overview = Some(RunOverview {
        origin: Some(Origin {
            kind: "chat".into(),
            channel_name: Some("#pr-reviews".into()),
            user_name: Some("Eli".into()),
            thread_url: None,
        }),
        subject: Some(Default::default()),
        ..Default::default()
    });
    let o = ov(&r);
    assert_eq!(o.origin.unwrap().user_name.as_deref(), Some("Eli"));
    assert!(o.subject.is_some());
}

#[test]
fn the_overview_stays_small() {
    let mut post = done("n6");
    post["deliverables"] = Value::Array(
        (0..20)
            .map(|i| json!({"label": format!("D{i}"), "state": "done"}))
            .collect(),
    );
    let mut step = running("n2", "needs_you");
    step["attention"] = json!({"kind": "question", "text": "x".repeat(500), "since": t(1)});
    let o = ov(&run("running", json!([post, step])));
    assert_eq!(o.deliverables.len(), 12);
    assert_eq!(o.attention[0].attention.text.chars().count(), 300);
    assert_eq!(o.headline.chars().count(), 160);
    assert!(o.headline.ends_with('…'));
    assert!(serde_json::to_string(&o).unwrap().len() < 4096);
}

/// goal.md §2.7: a goal run's overview carries its progress from the loop
/// around the check, and the success headline says the goal was reached.
#[test]
fn a_goal_run_carries_its_progress_and_says_it_reached_the_goal() {
    let mut g = graph();
    g.nodes.push(WorkflowNode {
        id: "goal".into(),
        kind: "loop".into(),
        name: "Work toward the goal".into(),
        x: 0.0,
        y: 0.0,
        params: json!({"steps": [{"kind": "agent_prompt"}, {"kind": "check"}]}),
        retry: None,
    });
    let check = json!({"all_met": true, "met": 2, "total": 2});
    let r: WorkflowRun = serde_json::from_value(json!({
        "id": "r1", "workflow_id": "wf", "workspace_id": "ws", "status": "success",
        "input": {}, "started_at": t(0), "finished_at": t(90), "rev": 3,
        "nodes": [{"node_id": "goal", "status": "success", "started_at": t(0), "step_no": 1,
            "output": {"iterations": 3, "max_iterations": 6, "active_ms": 81000, "max_active_s": 7200,
                "satisfied": true, "history": [{"steps": {"Work": {}, "Check criteria": check}}]}}],
    }))
    .unwrap();
    let o = run_overview(&r, &g, None, None, t(100));
    let goal = o.goal.clone().expect("goal progress");
    assert_eq!((goal.iteration, goal.max_iterations), (3, 6));
    assert_eq!((goal.criteria_met, goal.criteria_total), (2, 2));
    assert_eq!(
        (goal.active_ms, goal.max_active_ms),
        (81_000, Some(7_200_000))
    );
    assert_eq!(
        o.headline,
        "Goal reached — 2 of 2 criteria met in 3 iterations"
    );
}
