use super::*;
use crate::chat_status::{render_final, render_status, ChatView};
use crate::overview::run_overview;
use chrono::{FixedOffset, TimeZone, Utc};
use otto_core::workflows::{RunStatus, WorkflowCheckpoint, WorkflowRun};
use serde_json::json;

fn t(secs: i64) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 26, 9, 0, 0).unwrap() + chrono::Duration::seconds(secs)
}

fn two_loop_graph() -> WorkflowGraph {
    let review_crit = json!({
        "id": "review-passes",
        "text": "A fresh independent review passes",
        "verify": "expr",
        "expr": "steps.review.passed == true"
    });
    serde_json::from_value(json!({
        "nodes": [
            {"id": "start", "kind": "manual_trigger", "name": "Start"},
            {"id": "ui", "kind": "loop", "name": "UI", "params": {
                "max_iterations": 5,
                "steps": [
                    {"kind": "review_run", "name": "review"},
                    {"kind": "check", "name": "check", "params": {"criteria": [review_crit]}},
                    {"kind": "agent_prompt", "name": "fix"}
                ]
            }},
            {"id": "server", "kind": "loop", "name": "server", "params": {
                "max_iterations": 5,
                "steps": [
                    {"kind": "review_run", "name": "review"},
                    {"kind": "check", "name": "check", "params": {"criteria": [review_crit]}},
                    {"kind": "agent_prompt", "name": "fix"}
                ]
            }}
        ],
        "edges": [
            {"id": "e1", "source": "start", "target": "ui"},
            {"id": "e2", "source": "start", "target": "server"}
        ]
    }))
    .unwrap()
}

fn check_cp(
    loop_id: &str,
    iteration: u64,
    met: bool,
    score: f64,
    threshold: f64,
    label: Option<&str>,
) -> WorkflowCheckpoint {
    let mut prog = json!({
        "value": score,
        "target": threshold,
        "values": {
            "steps.review.passed": met,
            "score": score,
            "threshold": threshold
        }
    });
    if let Some(l) = label {
        prog["label"] = json!(l);
    }
    WorkflowCheckpoint {
        node_id: format!("{loop_id}#{iteration}.1"),
        loop_id: loop_id.into(),
        iteration,
        step_index: 1,
        kind: "check".into(),
        name: "check".into(),
        status: NodeStatus::Success,
        attempts: 1,
        input: json!({}),
        output: Some(json!({
            "criteria": [{
                "id": "review-passes",
                "text": "A fresh independent review passes",
                "verify": "expr",
                "met": met,
                "evidence": format!("steps.review.passed == true is {met}"),
                "verified_by": "rule",
                "progress": prog
            }],
            "all_met": met,
            "met": if met { 1 } else { 0 },
            "total": 1,
            "progress_pct": if met { 100 } else { 0 },
            "verdict": if met { "achieved" } else { "continue" },
            "feedback": "",
            "feedback_md": "",
            "repeat_signature": "",
            "repeated_failures": 0
        })),
        error: None,
        logs: vec![],
        updated_at: t(iteration as i64 * 60),
        active_ms: 15_000,
        thread_id: None,
        start_ordinal: None,
    }
}

#[test]
fn two_loops_with_same_criterion_id_stay_distinct_with_history_headline_and_chat() {
    let g = two_loop_graph();
    assert!(has_goal_checks(&g));
    let mut run: WorkflowRun = serde_json::from_value(json!({
        "id": "r1", "workflow_id": "wf", "workspace_id": "ws", "status": "running",
        "input": {}, "started_at": t(0), "rev": 10,
        "nodes": [
            {"node_id": "start", "status": "success", "started_at": t(0), "step_no": 1},
            {"node_id": "ui", "status": "running", "started_at": t(1), "step_no": 2, "logs": ["🔁 iteration 3/5"]},
            {"node_id": "server", "status": "running", "started_at": t(1), "step_no": 3, "logs": ["🔁 iteration 3/5"]}
        ]
    }))
    .unwrap();
    run.checkpoints = vec![
        check_cp("ui", 1, false, 35.0, 95.0, Some("4 must-fix findings left")),
        check_cp("ui", 2, false, 58.0, 95.0, Some("3 must-fix findings left")),
        check_cp("ui", 3, false, 72.0, 95.0, Some("2 must-fix findings left")),
        check_cp(
            "server",
            1,
            false,
            60.0,
            95.0,
            Some("2 must-fix findings left"),
        ),
        check_cp(
            "server",
            2,
            false,
            80.0,
            95.0,
            Some("1 must-fix finding left"),
        ),
        check_cp(
            "server",
            3,
            false,
            81.0,
            95.0,
            Some("1 must-fix finding left"),
        ),
    ];

    let ov = run_overview(&run, &g, None, None, t(300));
    let goal = ov.goal.as_ref().expect("goal progress");
    assert_eq!((goal.iteration, goal.max_iterations), (3, 5));
    assert_eq!((goal.criteria_met, goal.criteria_total), (0, 2));
    assert_eq!(goal.criteria.len(), 2);
    assert_eq!(
        (
            goal.criteria[0].step_id.as_str(),
            goal.criteria[0].step_name.as_str(),
            goal.criteria[0].id.as_str()
        ),
        ("ui.1", "UI", "review-passes")
    );
    assert_eq!(
        (
            goal.criteria[1].step_id.as_str(),
            goal.criteria[1].step_name.as_str(),
            goal.criteria[1].id.as_str()
        ),
        ("server.1", "server", "review-passes")
    );
    assert_eq!(
        goal.criteria[0]
            .history
            .iter()
            .map(|p| (p.iteration, p.value))
            .collect::<Vec<_>>(),
        vec![(1, Some(35.0)), (2, Some(58.0)), (3, Some(72.0))]
    );
    assert_eq!(
        goal.criteria[1]
            .history
            .iter()
            .map(|p| (p.iteration, p.value))
            .collect::<Vec<_>>(),
        vec![(1, Some(60.0)), (2, Some(80.0)), (3, Some(81.0))]
    );
    assert_eq!(
        goal.criteria[0]
            .progress
            .as_ref()
            .and_then(|p| p.label.as_deref()),
        Some("2 must-fix findings left")
    );
    assert_eq!(ov.headline, "Round 3 of 5 — UI 72 / 95, server 81 / 95");

    // Chat status card: goal line + one line per round with "(was X)" deltas.
    let view = ChatView {
        workflow: "Review & Fix",
        now: t(300),
        tz: FixedOffset::east_opt(0).unwrap(),
    };
    let card = render_status(&view, &ov, &run.nodes, &g);
    let lines: Vec<&str> = card.lines().collect();
    assert_eq!(lines[1], "🎯 Round 3 of 5 — UI 72 / 95, server 81 / 95");
    assert_eq!(lines[2], "Round 1: UI 35 / 95, server 60 / 95");
    assert_eq!(
        lines[3],
        "Round 2: UI 58 / 95 (was 35), server 80 / 95 (was 60)"
    );
    assert_eq!(
        lines[4],
        "Round 3: UI 72 / 95 (was 58), server 81 / 95 (was 80)"
    );

    // When server passes while UI is still unmet, counts and states stay independent.
    run.checkpoints
        .push(check_cp("server", 4, true, 96.0, 95.0, None));
    let ov2 = run_overview(&run, &g, None, None, t(360));
    let g2 = ov2.goal.as_ref().unwrap();
    assert_eq!((g2.criteria_met, g2.criteria_total), (1, 2));
    assert_eq!(g2.criteria[0].met, Some(false));
    assert_eq!(g2.criteria[1].met, Some(true));
    assert_eq!(ov2.headline, "Round 3 of 5 — UI 72 / 95, server 96 / 95");

    // When both reach target and run succeeds, headline and final post carry the numbers.
    run.checkpoints
        .push(check_cp("ui", 4, true, 97.0, 95.0, None));
    run.status = RunStatus::Success;
    run.finished_at = Some(t(420));
    run.nodes[1].status = NodeStatus::Success;
    run.nodes[1].output = Some(json!({"iterations": 4, "max_iterations": 5, "satisfied": true}));
    run.nodes[2].status = NodeStatus::Success;
    run.nodes[2].output = Some(json!({"iterations": 4, "max_iterations": 5, "satisfied": true}));
    let ov3 = run_overview(&run, &g, None, None, t(420));
    assert_eq!(
        ov3.headline,
        "Goal reached — UI 97 / 95, server 96 / 95 in 4 rounds"
    );
    let fin = render_final(&view, &ov3, &run.nodes, &g, None);
    assert!(
        fin.contains("🎯 Goal reached — UI 97 / 95, server 96 / 95"),
        "{fin}"
    );
    assert!(
        fin.contains("Round 4: UI 97 / 95 (was 72), server 96 / 95 (was 81)"),
        "{fin}"
    );
}

#[test]
fn history_is_bounded_to_fifty_rounds_and_chat_folds_earlier_rounds() {
    let g = two_loop_graph();
    let mut run: WorkflowRun = serde_json::from_value(json!({
        "id": "r-cap", "workflow_id": "wf", "workspace_id": "ws", "status": "running",
        "input": {}, "started_at": t(0), "rev": 1,
        "nodes": [
            {"node_id": "ui", "status": "running", "started_at": t(0), "logs": ["🔁 iteration 55/55"]},
            {"node_id": "server", "status": "pending"}
        ]
    }))
    .unwrap();
    for iter in 1..=55u64 {
        run.checkpoints
            .push(check_cp("ui", iter, false, iter as f64, 95.0, None));
    }
    let ov = run_overview(&run, &g, None, None, t(600));
    let ui_hist = &ov.goal.as_ref().unwrap().criteria[0].history;
    assert_eq!(ui_hist.len(), MAX_HISTORY);
    assert_eq!(ui_hist.first().unwrap().iteration, 6);
    assert_eq!(ui_hist.last().unwrap().iteration, 55);

    let rounds = chat_round_lines(ov.goal.as_ref().unwrap());
    assert_eq!(rounds.len(), MAX_CHAT_ROUNDS + 1);
    assert_eq!(rounds[0], "… 42 earlier rounds");
    assert_eq!(rounds.last().unwrap(), "Round 55: UI 55 / 95 (was 54)");
}

#[test]
fn bug1_held_loop_reports_its_own_iteration_and_active_time_not_completed_sibling() {
    let g: WorkflowGraph = serde_json::from_value(json!({
        "nodes": [
            {"id": "loop_a", "kind": "loop", "name": "Loop A", "params": {
                "max_iterations": 4,
                "max_active_s": 3600,
                "steps": [
                    {"kind": "check", "name": "check", "params": {
                        "criteria": [{"id": "a1", "text": "A passes", "verify": "expr", "expr": "true"}]
                    }}
                ]
            }},
            {"id": "loop_b", "kind": "loop", "name": "Loop B", "params": {
                "max_iterations": 50,
                "max_active_s": 7200,
                "steps": [
                    {"kind": "check", "name": "check", "params": {
                        "criteria": [{"id": "b1", "text": "B passes", "verify": "expr", "expr": "true"}]
                    }}
                ]
            }}
        ],
        "edges": []
    }))
    .unwrap();
    let mut run: WorkflowRun = serde_json::from_value(json!({
        "id": "r-held", "workflow_id": "wf", "workspace_id": "ws", "status": "error",
        "input": {}, "started_at": t(0), "finished_at": t(600), "rev": 5,
        "hold": {
            "kind": "exhausted",
            "reason": "Used all 4 iterations",
            "since": t(600),
            "node_ids": ["loop_a"],
            "iteration": 4
        },
        "nodes": [
            {"node_id": "loop_a", "status": "pending", "started_at": t(0)},
            {"node_id": "loop_b", "status": "success", "started_at": t(0), "output": {
                "iterations": 50, "max_iterations": 50, "active_ms": 5_000_000, "max_active_s": 7200
            }}
        ]
    }))
    .unwrap();
    run.checkpoints = vec![
        WorkflowCheckpoint {
            node_id: "loop_a#4.0".into(),
            loop_id: "loop_a".into(),
            iteration: 4,
            step_index: 0,
            kind: "check".into(),
            name: "check".into(),
            status: NodeStatus::Success,
            attempts: 1,
            input: json!({}),
            output: Some(json!({
                "criteria": [{"id": "a1", "text": "A passes", "verify": "expr", "met": false, "evidence": "no", "verified_by": "rule"}],
                "all_met": false, "met": 0, "total": 1, "progress_pct": 0,
                "verdict": "continue", "feedback": "", "feedback_md": "", "repeat_signature": "", "repeated_failures": 0
            })),
            error: None,
            logs: vec![],
            updated_at: t(400),
            active_ms: 120_000,
            thread_id: None,
            start_ordinal: None,
        },
        WorkflowCheckpoint {
            node_id: "loop_b#50.0".into(),
            loop_id: "loop_b".into(),
            iteration: 50,
            step_index: 0,
            kind: "check".into(),
            name: "check".into(),
            status: NodeStatus::Success,
            attempts: 1,
            input: json!({}),
            output: Some(json!({
                "criteria": [{"id": "b1", "text": "B passes", "verify": "expr", "met": true, "evidence": "ok", "verified_by": "rule"}],
                "all_met": true, "met": 1, "total": 1, "progress_pct": 100,
                "verdict": "achieved", "feedback": "", "feedback_md": "", "repeat_signature": "", "repeated_failures": 0
            })),
            error: None,
            logs: vec![],
            updated_at: t(500),
            active_ms: 5_000_000,
            thread_id: None,
            start_ordinal: None,
        },
    ];
    let ov = run_overview(&run, &g, None, None, t(600));
    let goal = ov.goal.as_ref().expect("goal");
    // Extension form reads the held loop's own limits (4 / 4, 120s / 3600s), not sibling's 50 / 5000s.
    assert_eq!((goal.iteration, goal.max_iterations), (4, 4));
    assert_eq!(goal.active_ms, 120_000);
    assert_eq!(goal.max_active_ms, Some(3_600_000));
    // Criterion summary still spans both loops.
    assert_eq!((goal.criteria_met, goal.criteria_total), (1, 2));
}

#[test]
fn bug2_counts_all_criteria_past_forty_row_display_cap() {
    let make_crits = |prefix: &str, count: usize| -> Vec<Value> {
        (0..count)
            .map(|i| json!({"id": format!("{prefix}{i}"), "text": format!("{prefix} {i}"), "verify": "expr", "expr": "true"}))
            .collect()
    };
    let g: WorkflowGraph = serde_json::from_value(json!({
        "nodes": [
            {"id": "l1", "kind": "loop", "name": "L1", "params": {
                "max_iterations": 3,
                "steps": [{"kind": "check", "name": "c1", "params": {"criteria": make_crits("a", 25)}}]
            }},
            {"id": "l2", "kind": "loop", "name": "L2", "params": {
                "max_iterations": 3,
                "steps": [{"kind": "check", "name": "c2", "params": {"criteria": make_crits("b", 25)}}]
            }}
        ],
        "edges": []
    }))
    .unwrap();
    let make_rows =
        |prefix: &str, count: usize, fail_idx: Option<usize>| -> Vec<Value> {
            (0..count)
            .map(|i| json!({
                "id": format!("{prefix}{i}"), "text": format!("{prefix} {i}"), "verify": "expr",
                "met": fail_idx != Some(i), "evidence": "e", "verified_by": "rule"
            }))
            .collect()
        };
    let mut run: WorkflowRun = serde_json::from_value(json!({
        "id": "r-50", "workflow_id": "wf", "workspace_id": "ws", "status": "running",
        "input": {}, "started_at": t(0), "rev": 1,
        "nodes": [
            {"node_id": "l1", "status": "running", "started_at": t(0), "logs": ["🔁 iteration 1/3"]},
            {"node_id": "l2", "status": "running", "started_at": t(0), "logs": ["🔁 iteration 1/3"]}
        ]
    }))
    .unwrap();
    run.checkpoints = vec![
        WorkflowCheckpoint {
            node_id: "l1#1.0".into(),
            loop_id: "l1".into(),
            iteration: 1,
            step_index: 0,
            kind: "check".into(),
            name: "c1".into(),
            status: NodeStatus::Success,
            attempts: 1,
            input: json!({}),
            output: Some(json!({
                "criteria": make_rows("a", 25, None),
                "all_met": true, "met": 25, "total": 25, "progress_pct": 100,
                "verdict": "achieved", "feedback": "", "feedback_md": "", "repeat_signature": "", "repeated_failures": 0
            })),
            error: None,
            logs: vec![],
            updated_at: t(10),
            active_ms: 1000,
            thread_id: None,
            start_ordinal: None,
        },
        WorkflowCheckpoint {
            node_id: "l2#1.0".into(),
            loop_id: "l2".into(),
            iteration: 1,
            step_index: 0,
            kind: "check".into(),
            name: "c2".into(),
            status: NodeStatus::Success,
            attempts: 1,
            input: json!({}),
            output: Some(json!({
                "criteria": make_rows("b", 25, Some(20)),
                "all_met": false, "met": 24, "total": 25, "progress_pct": 96,
                "verdict": "continue", "feedback": "", "feedback_md": "", "repeat_signature": "", "repeated_failures": 0
            })),
            error: None,
            logs: vec![],
            updated_at: t(20),
            active_ms: 1000,
            thread_id: None,
            start_ordinal: None,
        },
    ];
    let ov = run_overview(&run, &g, None, None, t(30));
    let goal = ov.goal.as_ref().unwrap();
    assert_eq!(goal.criteria.len(), MAX_GOAL_CRITERIA);
    assert_eq!((goal.criteria_met, goal.criteria_total), (49, 50));
    let line = chat_goal_line(goal).unwrap();
    assert!(
        !line.contains("Goal reached"),
        "unmet 49/50 must not claim Goal reached: {line}"
    );
}

#[test]
fn bug5_rejected_completion_review_and_count_only_do_not_say_goal_reached() {
    let g = two_loop_graph();
    let mut run: WorkflowRun = serde_json::from_value(json!({
        "id": "r-blk", "workflow_id": "wf", "workspace_id": "ws", "status": "error",
        "input": {}, "started_at": t(0), "finished_at": t(120), "rev": 4,
        "hold": {
            "kind": "blocked",
            "reason": "The completion review has unresolved findings: missing error path",
            "since": t(120),
            "node_ids": ["ui"],
            "iteration": 2
        },
        "nodes": [
            {"node_id": "start", "status": "success", "started_at": t(0)},
            {"node_id": "ui", "status": "pending", "started_at": t(1)},
            {"node_id": "server", "status": "success", "started_at": t(1), "output": {"iterations": 2, "max_iterations": 5}}
        ]
    }))
    .unwrap();
    let mut blocked_cp = check_cp("ui", 2, true, 96.0, 95.0, None);
    blocked_cp.output.as_mut().unwrap()["verdict"] = json!("blocked");
    run.checkpoints = vec![blocked_cp, check_cp("server", 2, true, 97.0, 95.0, None)];

    let ov = run_overview(&run, &g, None, None, t(120));
    let goal = ov.goal.as_ref().unwrap();
    assert_eq!((goal.criteria_met, goal.criteria_total), (2, 2));
    assert_eq!(goal.verdict.as_deref(), Some("blocked"));

    let view = ChatView {
        workflow: "Review & Fix",
        now: t(120),
        tz: FixedOffset::east_opt(0).unwrap(),
    };
    let live = render_status(&view, &ov, &run.nodes, &g);
    let fin = render_final(&view, &ov, &run.nodes, &g, None);
    assert!(!live.contains("Goal reached"), "blocked live card: {live}");
    assert!(
        live.contains("All criteria met"),
        "blocked live card: {live}"
    );
    assert!(!fin.contains("Goal reached"), "blocked final post: {fin}");
    assert!(
        fin.contains("All criteria met"),
        "blocked final post: {fin}"
    );

    // Count-only GoalProgress (no verdict) says "All criteria met", not "Goal reached".
    let count_only = GoalProgress {
        iteration: 2,
        max_iterations: 5,
        criteria_met: 2,
        criteria_total: 2,
        verdict: None,
        ..Default::default()
    };
    let line = chat_goal_line(&count_only).unwrap();
    assert!(line.contains("All criteria met"), "{line}");
    assert!(!line.contains("Goal reached"), "{line}");
}

#[test]
fn bug4_long_goal_history_stays_under_telegram_limit_and_keeps_outcome_and_link() {
    let g = two_loop_graph();
    let mut criteria = Vec::new();
    for i in 0..25 {
        let history = (1..=8)
            .map(|iter| CriterionPoint {
                iteration: iter,
                met: iter == 8,
                value: Some(60.0 + iter as f64 * 4.0),
            })
            .collect();
        criteria.push(GoalCriterion {
            step_id: format!("check_{i}"),
            step_name: format!("service-component-{i:02}"),
            id: format!("quality_gate_{i:02}"),
            text: format!("Quality gate {i}"),
            met: Some(true),
            awaiting: false,
            progress: Some(CriterionProgress {
                value: Some(96.0),
                target: Some(95.0),
                ..Default::default()
            }),
            history,
        });
    }
    let ov = otto_core::workflows::RunOverview {
        state: otto_core::workflows::RunState::Succeeded,
        goal: Some(GoalProgress {
            iteration: 8,
            max_iterations: 10,
            criteria_met: 25,
            criteria_total: 25,
            verdict: Some("achieved".into()),
            criteria,
            ..Default::default()
        }),
        deliverables: vec![otto_core::workflows::Deliverable {
            label: "PR review".into(),
            state: otto_core::workflows::DeliverableState::Done,
            url: Some("https://example.com/pr/42".into()),
            detail: Some("posted".into()),
        }],
        ..Default::default()
    };
    let view = ChatView {
        workflow: "Release verification",
        now: t(500),
        tz: FixedOffset::east_opt(0).unwrap(),
    };
    let fin = render_final(&view, &ov, &[], &g, Some("https://otto.local/runs/r-long"));
    assert!(
        fin.chars().count() <= crate::chat_status::CHAT_MAX_CHARS,
        "final post exceeded budget: {} chars",
        fin.chars().count()
    );
    assert!(fin.starts_with("✅ *Release verification* done"), "{fin}");
    assert!(fin.contains("PR review"), "deliverables preserved: {fin}");
    assert!(
        fin.contains("[Open in Otto](https://otto.local/runs/r-long)"),
        "link preserved: {fin}"
    );
}

#[test]
fn api1_ws_projection_stays_within_four_kib_budget() {
    let mut criteria = Vec::new();
    for i in 0..4 {
        let mut values = BTreeMap::new();
        values.insert("score".into(), json!(90));
        values.insert("threshold".into(), json!(95));
        let history = (1..=30)
            .map(|iter| CriterionPoint {
                iteration: iter,
                met: false,
                value: Some(50.0 + iter as f64),
            })
            .collect();
        criteria.push(GoalCriterion {
            step_id: format!("loop_{i}.1"),
            step_name: format!("Component {i}"),
            id: format!("review-passes-{i}"),
            text: format!("A fresh independent review passes for component {i}"),
            met: Some(false),
            awaiting: false,
            progress: Some(CriterionProgress {
                value: Some(80.0),
                target: Some(95.0),
                unit: None,
                label: Some("2 must-fix findings left".into()),
                values,
            }),
            history,
        });
    }
    let full = otto_core::workflows::RunOverview {
        rev: 42,
        state: otto_core::workflows::RunState::Running,
        headline: "Round 30 of 50 — Component 0 80 / 95".into(),
        goal: Some(GoalProgress {
            iteration: 30,
            max_iterations: 50,
            criteria_met: 0,
            criteria_total: 4,
            verdict: Some("continue".into()),
            criteria,
            ..Default::default()
        }),
        ..Default::default()
    };
    let raw_bytes = serde_json::to_vec(&full).unwrap().len();
    assert!(
        raw_bytes > crate::overview::WS_OVERVIEW_MAX_BYTES,
        "fixture must exceed 4 KiB raw (got {raw_bytes})"
    );
    let projected = crate::overview::ws_projection(&full);
    let proj_bytes = serde_json::to_vec(&projected).unwrap().len();
    assert!(
        proj_bytes <= crate::overview::WS_OVERVIEW_MAX_BYTES,
        "projected overview must fit 4 KiB (got {proj_bytes})"
    );
    let pg = projected.goal.as_ref().unwrap();
    assert_eq!(pg.criteria.len(), 4);
    assert!(pg.criteria[0].history.len() <= 3);
    assert!(pg.criteria[0].progress.as_ref().unwrap().values.is_empty());
    // Full HTTP overview remains untouched.
    assert_eq!(full.goal.as_ref().unwrap().criteria[0].history.len(), 30);
}
