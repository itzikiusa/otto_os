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
