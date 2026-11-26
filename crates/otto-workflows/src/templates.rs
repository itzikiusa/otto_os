//! Built-in workflow templates with no server dependency (moved out of
//! otto-server's routes): pure graph builders the template routes list.

use otto_core::workflows::{WorkflowEdge, WorkflowGraph, WorkflowNode, WorkflowTemplate};
use serde_json::{json, Value};

/// Build the three game-pipeline templates. Each chains:
/// trigger → agent (design rules) → game_engine (kind) → verifier.
pub fn game_templates() -> Vec<WorkflowTemplate> {
    fn pipeline(game: &str, design: &str) -> WorkflowGraph {
        let node =
            |id: &str, kind: &str, name: &str, x: f64, params: serde_json::Value| WorkflowNode {
                id: id.into(),
                kind: kind.into(),
                name: name.into(),
                x,
                y: 70.0,
                params,
                retry: None,
            };
        let edge = |s: &str, t: &str| WorkflowEdge {
            id: format!("{s}-{t}"),
            source: s.into(),
            target: t.into(),
            condition: None,
        };
        WorkflowGraph {
            nodes: vec![
                node(
                    "trigger",
                    "manual_trigger",
                    "Start",
                    40.0,
                    serde_json::Value::Null,
                ),
                node(
                    "design",
                    "agent_prompt",
                    "Design game",
                    320.0,
                    serde_json::json!({ "prompt": design }),
                ),
                node(
                    "game",
                    "game_engine",
                    "Build game",
                    600.0,
                    serde_json::json!({ "game": game }),
                ),
                node(
                    "verify",
                    "verifier",
                    "Verify",
                    880.0,
                    serde_json::Value::Null,
                ),
            ],
            edges: vec![
                edge("trigger", "design"),
                edge("design", "game"),
                edge("game", "verify"),
            ],
        }
    }

    vec![
        WorkflowTemplate {
            id: "game-slots".into(),
            name: "Slots game".into(),
            description: "5×3 slot machine: agent designs the paytable & RTP, then the engine assembles and verifies it.".into(),
            instructions: String::new(),
            icon: "grid".into(),
            graph: pipeline(
                "slots",
                "Design a 5x3 slot game for the given theme: reel symbols, paytable, win lines, RTP ~96%, volatility, and a bonus feature. Return a structured spec.",
            ),
        },
        WorkflowTemplate {
            id: "game-crash".into(),
            name: "Crash game (Aviator style)".into(),
            description: "Aviator-style crash game: agent designs the multiplier curve & provably-fair RNG, then build & verify.".into(),
            instructions: String::new(),
            icon: "zap".into(),
            graph: pipeline(
                "crash",
                "Design an aviator-style crash game: the multiplier growth curve, a provably-fair RNG seed/commit scheme, auto-cashout rules, house edge ~3%, and max multiplier. Return a structured spec.",
            ),
        },
        WorkflowTemplate {
            id: "game-scratch".into(),
            name: "Scratch card".into(),
            description: "Scratch-card game: agent designs prize tiers & win probabilities, then build & verify.".into(),
            instructions: String::new(),
            icon: "ticket".into(),
            graph: pipeline(
                "scratch",
                "Design a scratch-card game: prize tiers and their win probabilities, panel layout, reveal mechanic, and RTP ~95%. Return a structured spec.",
            ),
        },
    ]
}

/// The orchestrator example templates: end-to-end flows that exercise the wired
/// nodes (product/review), control flow (loop + edge conditions), goals-scored
/// review, human approval, and a drafted PR. They expect the run **input** to
/// carry `repo_id` (and optionally `base`, `story_id`, `goals`) — a Slack
/// `Action: Workflow` message or the Run dialog supplies these; the first node
/// consolidates the Jira ticket / working dir / relevant info into a brief.
pub fn flow_templates() -> Vec<WorkflowTemplate> {
    let node = |id: &str, kind: &str, name: &str, x: f64, params: serde_json::Value| WorkflowNode {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        x,
        y: 80.0,
        params,
        retry: None,
    };
    let edge = |s: &str, t: &str| WorkflowEdge {
        id: format!("{s}-{t}"),
        source: s.into(),
        target: t.into(),
        condition: None,
    };
    // A conditional edge: the target only runs when `cond` holds against the
    // source's output (ctx = { output, input, node, run }). Used to gate the PR
    // step on the review having passed.
    let edge_if = |s: &str, t: &str, cond: &str| WorkflowEdge {
        id: format!("{s}-{t}"),
        source: s.into(),
        target: t.into(),
        condition: Some(cond.into()),
    };
    let prepare = |goal: &str, x: f64| {
        node(
            "prepare",
            "agent_prompt",
            "Prepare relevant info",
            x,
            json!({ "prompt": format!(
                "{goal}\n\nFrom the input data, read the Jira ticket (if any), the working \
                 directory, and every 'relevant_info' path, plus the message and goals. Search \
                 the codebase for references. Produce a single consolidated brief: scope, where \
                 the code lives, acceptance criteria, and the goals to satisfy. This brief is \
                 passed to the following steps."
            ) }),
        )
    };
    // Loop body — REVIEW first, then FIX (design §E), as the one review→fix
    // rule (goal.md §4.1): review → check "the review passes" → fix, ending at
    // once when the review passes. The reviewers mirror PR review (per-lens
    // provider sets + a summarizer); scoring deducts per OPEN finding by
    // severity (bug=critical, warn=high, info=low).
    let fix_review_loop = |max: u32, threshold: u64, reviewers: serde_json::Value, x: f64| {
        use crate::goal::{review_fix_loop, ReviewFixOpts, ReviewFixStart};
        review_fix_loop(ReviewFixOpts {
            max_iterations: max,
            max_active_s: None,
            start_with: ReviewFixStart::Review,
            review_params: json!({"threshold": threshold, "reviewers": reviewers,
                "summarizer": {"provider": "claude"}, "scoring": {"bug": 10, "warn": 5, "info": 1}}),
            fix_prompt: "You are given the latest code review in the input (its findings and score). \
                         Address EVERY finding and make all tests pass while satisfying the goals."
                .into(),
            extra_criteria: vec![],
            hold_when_unmet: false,
            stop_on_repeat_failure: false,
            fix_agent: Default::default(),
            checker: Default::default(),
            x,
        })
        .expect("the templates' review step has no require_pass")
    };
    // A separate, terminal "offer improvements" block (design §I): after the
    // work + review loop, reflect on the session and OFFER skill/memory
    // improvements — queued for approval, never auto-applied. Placed BELOW the
    // post-loop x (y=300) because it shares that x with the `git_pr` node — the
    // two are PARALLEL branches off the loop (PR on pass, improvements always), so
    // they must not render stacked on top of each other (which hid the PR node).
    let offer_improvements = |x: f64| WorkflowNode {
        id: "improve".into(),
        kind: "self_improve".into(),
        name: "Offer improvements".into(),
        x,
        y: 300.0,
        params: Value::Null,
        retry: None,
    };

    vec![
        // 1) Writing tests for a story.
        WorkflowTemplate {
            id: "write-tests".into(),
            name: "Write tests for a story".into(),
            description: "Read the story & search references → write tests → multi-agent \
                          review-iterate until the score passes the threshold → open the PR \
                          automatically on pass (the review IS the approval). Provide repo_id \
                          (and base) in the run input."
                .into(),
            instructions: String::new(),
            icon: "check-square".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    prepare("You are preparing context to WRITE TESTS.", 300.0),
                    node("implement", "agent_prompt", "Write tests", 600.0, json!({
                        "prompt": "Using the brief, implement comprehensive tests (happy path, \
                                   meaningful validations, realistic errors). Run the suite and make them pass."
                    })),
                    fix_review_loop(
                        3,
                        80,
                        json!([
                            { "lens": "correctness-review", "providers": ["claude", "codex"] },
                            { "lens": "test-review", "providers": ["claude"] }
                        ]),
                        900.0,
                    ),
                    node("pr", "git_pr", "Open PR (on pass)", 1300.0, json!({ "open": true })),
                    offer_improvements(1300.0),
                ],
                edges: vec![
                    edge("trigger", "prepare"),
                    edge("prepare", "implement"),
                    edge("implement", "iterate"),
                    // Open the PR only when the review→fix loop passed.
                    edge_if("iterate", "pr", "output.satisfied == true"),
                    // Offer improvements after the loop, pass or fail — after the
                    // PR (DR-014: steps run side by side, two outward steps never do).
                    edge("iterate", "improve"),
                    edge("pr", "improve"),
                ],
            },
        },
        // 2) Implementing a feature from a story.
        WorkflowTemplate {
            id: "implement-feature".into(),
            name: "Implement a feature from a story".into(),
            description: "Analyze the story → search references → implement → tests → multi-agent \
                          review-iterate until passing → open the PR automatically on pass (the \
                          review IS the approval). Provide repo_id and story_id in the run input."
                .into(),
            instructions: String::new(),
            icon: "command".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    node("analyze", "product_analyze", "Analyze story", 300.0, Value::Null),
                    prepare("You are preparing context to IMPLEMENT a feature.", 600.0),
                    node("implement", "agent_prompt", "Implement", 900.0, json!({
                        "prompt": "Using the analysis + brief, implement the feature and its tests. \
                                   Run the suite and make it pass."
                    })),
                    fix_review_loop(
                        4,
                        80,
                        json!([
                            { "lens": "correctness-review", "providers": ["claude", "codex"] },
                            { "lens": "security-review", "providers": ["codex"] },
                            { "lens": "test-review", "providers": ["claude"] }
                        ]),
                        1200.0,
                    ),
                    node("pr", "git_pr", "Open PR (on pass)", 1600.0, json!({ "open": true })),
                    offer_improvements(1600.0),
                ],
                edges: vec![
                    edge("trigger", "analyze"),
                    edge("analyze", "prepare"),
                    edge("prepare", "implement"),
                    edge("implement", "iterate"),
                    // Open the PR only when the review→fix loop passed.
                    edge_if("iterate", "pr", "output.satisfied == true"),
                    // Offer improvements after the loop, pass or fail — after the
                    // PR (DR-014: steps run side by side, two outward steps never do).
                    edge("iterate", "improve"),
                    edge("pr", "improve"),
                ],
            },
        },
        // 3) PO discovery → diagram → review → refine → RFC/Jira.
        WorkflowTemplate {
            id: "po-lifecycle".into(),
            name: "PO discovery → RFC/Jira".into(),
            description: "Discovery → diagram → review → refine → publication preview → human approval → publish. \
                          Before running, configure story_id on Refine and Preview, and account plus Confluence space \
                          (or Jira project) on Preview. Publication only follows approval of that exact preview."
                .into(),
            instructions: String::new(),
            icon: "compass".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    node("discovery", "agent_prompt", "Discovery draft", 300.0, json!({
                        "prompt": "Expand the idea/message in the input into a structured product \
                                   draft: problem, target users, value, scope, out-of-scope, risks, \
                                   and open questions."
                    })),
                    node("diagram", "canvas", "Diagram", 600.0, json!({
                        "prompt": "Diagram the proposed solution flow described above.",
                        "mode": "mermaid"
                    })),
                    node("review1", "human_approval", "Review discovery + diagram", 900.0, json!({
                        "prompt": "Review the discovery draft and the diagram."
                    })),
                    node("refine", "product_rewrite", "Refine + attach info", 1200.0, json!({"persist":true})),
                    node("preview", "product_publish", "Preview — configure account/destination", 1500.0, json!({"kind":"rfc","dry_run":true})),
                    node("review2", "human_approval", "Review publication content and destination", 1800.0, json!({
                        "prompt": "Review the full publication title, content, account and destination. Approve to publish this snapshot."
                    })),
                    node("publish", "product_publish", "Publish approved snapshot", 2100.0, json!({
                        "dry_run": false
                    })),
                ],
                edges: vec![
                    edge("trigger", "discovery"),
                    edge("discovery", "diagram"),
                    edge("diagram", "review1"),
                    edge("review1", "refine"),
                    edge("refine", "preview"),
                    edge("preview", "review2"),
                    edge("review2", "publish"),
                ],
            },
        },
        // 4) UI test authoring: story → app-fetched Jira context → write UI
        // tests → review-fix loop → PR on pass → final report (both branches).
        WorkflowTemplate {
            id: "ui-test-authoring".into(),
            name: "UI test authoring".into(),
            description: "Story → app-fetched Jira context → write UI tests → review-fix loop \
                          until green → PR on pass → final report. Bind a Slack channel or run \
                          with a prompt."
                .into(),
            instructions: "# Standing instructions — UI test authoring\n\
                - Tests are Playwright specs; follow the repo's existing spec layout and naming.\n\
                - Never sleep-poll; use Playwright auto-waiting and web-first assertions.\n\
                - Selectors: prefer role/test-id selectors over CSS/text.\n\
                - Each spec must be independently runnable and idempotent.\n\
                - Run ONLY the specs you created or changed — never the full suite.\n\
                - If a jira-<KEY>.md exists in the context dir, it is the requirements source of truth.\n\
                - Write your step handoff file before finishing (path given in your prompt)."
                .into(),
            icon: "check-square".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    node("prep", "prepare_context", "Prepare relevant data", 300.0, json!({
                        "prompt": "Analyze the story and the codebase. If a jira-<KEY>.md file \
                                   exists in the context directory it is the source of truth for \
                                   requirements — read it fully (description AND comments). \
                                   Produce a test plan: what to cover, where the tests live, exact \
                                   conventions to follow per instructions.md."
                    })),
                    node("implement", "agent_prompt", "Write UI tests", 600.0, json!({
                        "prompt": "Write the UI tests per the test plan, following instructions.md \
                                   by the letter. Run only the specs you created/changed and make \
                                   them pass."
                    })),
                    fix_review_loop(
                        3,
                        80,
                        json!([
                            { "lens": "correctness-review", "providers": ["claude"] },
                            { "lens": "test-review", "providers": ["claude"] }
                        ]),
                        900.0,
                    ),
                    node("pr", "git_pr", "Open PR (on pass)", 1300.0, json!({ "open": true })),
                    node("report", "agent_prompt", "Final report", 1600.0, json!({
                        "prompt": "Write the final run report: what was requested, what was \
                                   implemented (files/specs), the final suite result, and PR links \
                                   from the input if present. If the review loop did not pass, \
                                   state exactly what is still failing. This report is the run's \
                                   final output."
                    })),
                    offer_improvements(1300.0),
                ],
                edges: vec![
                    edge("trigger", "prep"),
                    edge("prep", "implement"),
                    edge("implement", "iterate"),
                    // Open the PR only when the review→fix loop passed.
                    edge_if("iterate", "pr", "output.satisfied == true"),
                    // `report` MUST remain the run's LAST content-bearing step: on
                    // success it becomes final-output.md (workflow_context.rs's
                    // `content_step` is overwritten by every non-utility step that
                    // runs, last one wins — see `write_final_output`). This edge is
                    // what forces `report` to execution-order after `improve`
                    // (`self_improve` is content-bearing too, and both `report` and
                    // `improve` are otherwise parallel branches off `iterate`).
                    // Deleting this edge would silently make the self-improve offer
                    // the run's deliverable instead of the report.
                    edge("pr", "report"),
                    // The final report always runs, even when the loop didn't pass,
                    // and comes last: after the offer too.
                    edge("iterate", "report"),
                    edge("improve", "report"),
                    // Offer improvements after the loop, pass or fail — after the
                    // PR (DR-014: steps run side by side, two outward steps never do).
                    edge("iterate", "improve"),
                    edge("pr", "improve"),
                ],
            },
        },
        // 5) API acceptance test authoring: same shape as UI test authoring,
        // targeting the acceptance-test framework's Gateway/ServiceLocator layers.
        WorkflowTemplate {
            id: "api-acceptance-test-authoring".into(),
            name: "API acceptance test authoring".into(),
            description: "Story → app-fetched Jira context → write API acceptance tests → \
                          review-fix loop until green → PR on pass → final report. Bind a Slack \
                          channel or run with a prompt."
                .into(),
            instructions: "# Standing instructions — API acceptance test authoring\n\
                - Follow the acceptance-test framework's two layers strictly: Gateway APIs for \
                player-behavior flows, ServiceLocator for internal service features.\n\
                - Keep API-call code and validation/assertion code in their designated layers.\n\
                - Reuse existing player-creation / balance helpers; never duplicate setup utilities.\n\
                - Tests must be independently runnable and leave no dirty state.\n\
                - Run ONLY the tests you created or changed — never the full suite.\n\
                - If a jira-<KEY>.md exists in the context dir, it is the requirements source of truth.\n\
                - Write your step handoff file before finishing (path given in your prompt)."
                .into(),
            icon: "send".into(),
            graph: WorkflowGraph {
                nodes: vec![
                    node("trigger", "manual_trigger", "Start", 40.0, Value::Null),
                    node("prep", "prepare_context", "Prepare relevant data", 300.0, json!({
                        "prompt": "Analyze the story and the codebase. If a jira-<KEY>.md file \
                                   exists in the context directory it is the source of truth for \
                                   requirements — read it fully (description AND comments). \
                                   Produce a test plan: what to cover, where the tests live, exact \
                                   conventions to follow per instructions.md."
                    })),
                    node("implement", "agent_prompt", "Write API acceptance tests", 600.0, json!({
                        "prompt": "Write the API acceptance tests per the test plan, following \
                                   instructions.md by the letter. Run only the specs you \
                                   created/changed and make them pass."
                    })),
                    fix_review_loop(
                        3,
                        80,
                        json!([
                            { "lens": "correctness-review", "providers": ["claude"] },
                            { "lens": "test-review", "providers": ["claude"] }
                        ]),
                        900.0,
                    ),
                    node("pr", "git_pr", "Open PR (on pass)", 1300.0, json!({ "open": true })),
                    node("report", "agent_prompt", "Final report", 1600.0, json!({
                        "prompt": "Write the final run report: what was requested, what was \
                                   implemented (files/specs), the final suite result, and PR links \
                                   from the input if present. If the review loop did not pass, \
                                   state exactly what is still failing. This report is the run's \
                                   final output."
                    })),
                    offer_improvements(1300.0),
                ],
                edges: vec![
                    edge("trigger", "prep"),
                    edge("prep", "implement"),
                    edge("implement", "iterate"),
                    // Open the PR only when the review→fix loop passed.
                    edge_if("iterate", "pr", "output.satisfied == true"),
                    // `report` MUST remain the run's LAST content-bearing step: on
                    // success it becomes final-output.md (workflow_context.rs's
                    // `content_step` is overwritten by every non-utility step that
                    // runs, last one wins — see `write_final_output`). This edge is
                    // what forces `report` to execution-order after `improve`
                    // (`self_improve` is content-bearing too, and both `report` and
                    // `improve` are otherwise parallel branches off `iterate`).
                    // Deleting this edge would silently make the self-improve offer
                    // the run's deliverable instead of the report.
                    edge("pr", "report"),
                    // The final report always runs, even when the loop didn't pass,
                    // and comes last: after the offer too.
                    edge("iterate", "report"),
                    edge("improve", "report"),
                    // Offer improvements after the loop, pass or fail — after the
                    // PR (DR-014: steps run side by side, two outward steps never do).
                    edge("iterate", "improve"),
                    edge("pr", "improve"),
                ],
            },
        },
    ]
}

#[cfg(test)]
mod flow_tests {
    use super::*;

    #[test]
    fn flow_templates_open_a_pr() {
        // The implement/test templates must end with a git_pr node so a passing run
        // opens the PR (taking repo/branch from the run automatically).
        for id in ["write-tests", "implement-feature"] {
            let t = flow_templates().into_iter().find(|t| t.id == id).unwrap();
            assert!(
                t.graph.nodes.iter().any(|n| n.kind == "git_pr"),
                "template '{id}' must contain a git_pr node"
            );
        }
    }

    #[test]
    fn every_template_uses_known_kinds_with_an_agent() {
        for t in crate::templates::game_templates() {
            assert!(!t.graph.nodes.is_empty(), "{} has no nodes", t.id);
            for n in &t.graph.nodes {
                assert!(
                    crate::catalog::is_known_kind(&n.kind),
                    "unknown kind '{}' in {}",
                    n.kind,
                    t.id
                );
            }
            assert!(
                t.graph.nodes.iter().any(|n| n.kind == "agent_prompt"),
                "{} has no agent node",
                t.id
            );
        }
    }

    #[test]
    fn all_new_kinds_are_in_catalog() {
        let new_kinds = [
            "db_query",
            "broker_peek",
            "channel_notify",
            "budget_gate",
            "human_approval",
            "api_run",
            "product_analyze",
            "product_rewrite",
            "product_plan",
            "review_run",
            "condition",
            "loop",
            "product_publish",
            "canvas",
            "git_pr",
        ];
        for kind in new_kinds {
            assert!(
                crate::catalog::is_known_kind(kind),
                "catalog missing new kind '{kind}'"
            );
        }
    }

    #[test]
    fn review4_product_publish_template_has_preview_then_approval_then_live() {
        let template = flow_templates()
            .into_iter()
            .find(|template| template.id == "po-lifecycle")
            .unwrap();
        let preview = template
            .graph
            .nodes
            .iter()
            .find(|node| node.kind == "product_publish" && node.params["dry_run"] == true)
            .expect("preview node");
        let publish = template
            .graph
            .nodes
            .iter()
            .find(|node| node.kind == "product_publish" && node.params["dry_run"] == false)
            .expect("approved live successor");
        let gate = template
            .graph
            .edges
            .iter()
            .filter(|edge| edge.source == preview.id)
            .filter_map(|edge| {
                template
                    .graph
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.target && node.kind == "human_approval")
            })
            .find(|gate| {
                template
                    .graph
                    .edges
                    .iter()
                    .any(|edge| edge.source == gate.id && edge.target == publish.id)
            })
            .expect("preview → review → publish edges");
        assert!(!gate.params["prompt"]
            .as_str()
            .unwrap_or_default()
            .is_empty());
        assert!(
            template.description.contains("account") && template.description.contains("space"),
            "destination prerequisites visible before running"
        );
    }

    /// DR-014: steps run side by side now, so no shipped graph may run two
    /// steps with an outward effect at once — one must wait for the other.
    #[test]
    fn no_side_effect_step_beside_another_in_shipped_graphs() {
        const OUTWARD: [&str; 7] = [
            "git_pr",
            "channel_notify",
            "product_publish",
            "api_run",
            "http_request",
            "self_improve",
            "post_review_comments",
        ];
        for t in flow_templates() {
            let g = &t.graph;
            let outward: Vec<&str> = g
                .nodes
                .iter()
                .filter(|n| OUTWARD.contains(&n.kind.as_str()))
                .map(|n| n.id.as_str())
                .collect();
            for (i, a) in outward.iter().enumerate() {
                for b in &outward[i + 1..] {
                    let ordered = crate::graph::ancestors(g, a).contains(*b)
                        || crate::graph::ancestors(g, b).contains(*a);
                    assert!(ordered, "{}: {a} and {b} can run at the same time", t.id);
                }
            }
        }
    }

    #[test]
    fn flow_templates_are_valid() {
        let ids: Vec<String> = flow_templates().into_iter().map(|t| t.id).collect();
        for want in ["write-tests", "implement-feature", "po-lifecycle"] {
            assert!(
                ids.iter().any(|id| id == want),
                "missing flow template {want}"
            );
        }
        for t in flow_templates() {
            assert!(!t.graph.nodes.is_empty(), "{} has no nodes", t.id);
            // Every top-level node kind is known to the executor.
            for n in &t.graph.nodes {
                assert!(
                    crate::catalog::is_known_kind(&n.kind),
                    "unknown kind '{}' in {}",
                    n.kind,
                    t.id
                );
            }
            // Loop steps must also use known kinds.
            for n in &t.graph.nodes {
                if n.kind == "loop" {
                    let steps = n.params.get("steps").and_then(|s| s.as_array());
                    assert!(steps.is_some(), "{} loop has no steps", t.id);
                    for step in steps.unwrap() {
                        let k = step.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                        assert!(
                            crate::catalog::is_known_kind(k),
                            "loop step kind '{k}' unknown in {}",
                            t.id
                        );
                    }
                }
            }
            // Edges reference existing nodes (a precondition for a clean topo sort).
            let ids: std::collections::HashSet<&str> =
                t.graph.nodes.iter().map(|n| n.id.as_str()).collect();
            for e in &t.graph.edges {
                assert!(
                    ids.contains(e.source.as_str()),
                    "{}: dangling edge source",
                    t.id
                );
                assert!(
                    ids.contains(e.target.as_str()),
                    "{}: dangling edge target",
                    t.id
                );
            }
        }
    }

    #[test]
    fn test_flow_templates_shape() {
        let all = flow_templates();
        for id in ["ui-test-authoring", "api-acceptance-test-authoring"] {
            let t = all
                .iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("{id} missing"));
            assert!(
                !t.instructions.trim().is_empty(),
                "{id} ships standing instructions"
            );
            assert!(t.graph.nodes.iter().any(|n| n.kind == "prepare_context"));
            assert!(t
                .graph
                .nodes
                .iter()
                .any(|n| n.id == "report" && n.kind == "agent_prompt"));
            for n in &t.graph.nodes {
                assert!(
                    crate::catalog::is_known_kind(&n.kind),
                    "unknown kind {}",
                    n.kind
                );
            }
            // report is reachable from BOTH pr and iterate (runs on failure too)
            assert!(t
                .graph
                .edges
                .iter()
                .any(|e| e.source == "pr" && e.target == "report"));
            assert!(t
                .graph
                .edges
                .iter()
                .any(|e| e.source == "iterate" && e.target == "report"));
        }
    }

    /// GL12.2: the four review→fix templates share the one review→fix loop —
    /// review, check "the review passes", fix — and no `until` / `require_pass`.
    #[test]
    fn templates_use_the_one_review_fix_loop() {
        let loops: Vec<_> = flow_templates()
            .into_iter()
            .filter_map(|t| t.graph.nodes.into_iter().find(|n| n.id == "iterate"))
            .collect();
        assert_eq!(loops.len(), 4);
        for n in loops {
            let steps = n.params["steps"].as_array().unwrap();
            let kinds: Vec<_> = steps.iter().map(|s| s["kind"].as_str().unwrap()).collect();
            assert_eq!(kinds, ["review_run", "check", "agent_prompt"]);
            assert!(
                n.params.get("until").is_none() && steps[0]["params"].get("require_pass").is_none()
            );
            assert_eq!(
                steps[1]["params"]["criteria"][0]["expr"],
                "steps.review.passed == true"
            );
        }
    }
}
