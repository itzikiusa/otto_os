use super::*;
use crate::validation::{validate, IssueClass};
use serde_json::json;

fn spec() -> GoalSpec {
    serde_json::from_value(json!({
        "title": "Green build", "goal": "Make the build pass\nand keep it so",
        "objectives": ["fast"], "constraints": ["no new deps"],
        "criteria": [
            {"id": "tests", "text": "tests pass", "verify": "command", "command": "make test"},
            {"id": "docs", "text": "docs say how", "verify": "agent", "how": "read README"},
            {"id": "ok", "text": "looks right", "verify": "human", "how": "open the page"}
        ],
        "work": {"kind": "build", "repo_id": "r1"},
        "worker": {"provider": "codex", "model": "m1", "instructions": "be brief"},
        "checker": {"provider": "claude", "instructions": "be strict"},
        "budget": {"iterations": 7, "active_s": 7200, "step_timeout_s": 900}
    }))
    .unwrap()
}

fn repo() -> GoalRepo<'static> {
    GoalRepo {
        id: "r1",
        path: "/src/app",
        source: "abc",
    }
}

/// Save and run blocking issues of a built graph (warnings are fine).
fn blocking(graph: &WorkflowGraph) -> Vec<String> {
    validate(graph)
        .into_iter()
        .filter(|i| i.class != IssueClass::Warning)
        .map(|i| i.message)
        .collect()
}

#[test]
fn a_spec_is_checked_field_by_field() {
    assert_eq!(validate_spec(&spec()), Ok(()));
    let bad = |f: fn(&mut GoalSpec)| {
        let mut s = spec();
        f(&mut s);
        validate_spec(&s).unwrap_err()
    };
    assert!(bad(|s| s.title = " ".into()).starts_with("title"));
    assert!(bad(|s| s.goal = "x".repeat(8_001)).starts_with("goal"));
    assert!(bad(|s| s.criteria.clear()).starts_with("criteria"));
    assert!(bad(|s| s.criteria[1].id = "tests".into()).starts_with("criteria[1].id"));
    assert!(bad(|s| s.criteria[0].id = "Bad Id".into()).starts_with("criteria[0].id"));
    assert!(bad(|s| s.criteria[0].command = Some(" ".into())).starts_with("criteria[0].command"));
    assert!(bad(|s| s.budget.iterations = 51).starts_with("budget.iterations"));
    assert!(bad(|s| s.budget.active_s = 899).starts_with("budget.active_s"));
    assert!(bad(|s| s.budget.step_timeout_s = 7_201).starts_with("budget.step_timeout_s"));
}

#[test]
fn a_build_goal_is_start_then_one_loop_of_work_and_check() {
    let built = goal_workflow(&spec(), Some(&repo()));
    assert_eq!(built.name, "Goal · Green build");
    assert_eq!(built.description, "Make the build pass");
    let g = &built.graph;
    assert_eq!(g.nodes[0].params, json!({"lane": "long"}));
    assert_eq!(
        (
            g.edges.len(),
            g.edges[0].source.as_str(),
            g.edges[0].target.as_str()
        ),
        (1, "trigger", "goal")
    );
    let p = &g.nodes[1].params;
    assert_eq!(
        (p["max_iterations"].clone(), p["max_active_s"].clone()),
        (json!(7), json!(7200))
    );
    assert_eq!(
        (
            p["stop_on_repeat_failure"].clone(),
            p["hold_when_unmet"].clone()
        ),
        (json!(true), json!(true))
    );
    let work = &p["steps"][0];
    assert_eq!(
        (work["kind"].clone(), work["same_thread"].clone()),
        (json!("agent_prompt"), json!(true))
    );
    assert_eq!(work["params"]["provider"], "codex");
    assert_eq!(work["params"]["timeout_s"], 900);
    let prompt = work["params"]["prompt"].as_str().unwrap();
    for part in [
        "Make the build pass",
        "- no new deps",
        "[tests] tests pass (a command will be run: make test)",
        "a person will check: open the page",
        "Never stage, commit, push",
        "be brief",
    ] {
        assert!(prompt.contains(part), "{part} missing in {prompt}");
    }
    let check = &p["steps"][1];
    assert_eq!(check["kind"], "check");
    assert_eq!(check["params"]["criteria"], json!(spec().criteria));
    assert_eq!(check["params"]["evaluator"], json!({"provider": "claude"}));
    assert_eq!(check["params"]["instructions"], "be strict");
    // DR-102: the repos entry names the repo path, so the run gets a worktree.
    assert_eq!(
        built.input,
        json!({"goal": {"title": "Green build"},
        "repos": [{"repo": "r1", "type": "worktree", "name": "/src/app", "source": "abc"}]})
    );
    assert_eq!(blocking(g), Vec::<String>::new());
}

#[test]
fn a_research_goal_has_a_research_folder_and_no_repository() {
    let mut s = spec();
    s.work = GoalWork::Research;
    s.extra_workers = vec![NamedAgentChoice {
        name: "Second".into(),
        provider: Some("claude".into()),
        ..Default::default()
    }];
    let built = goal_workflow(&s, Some(&repo()));
    assert_eq!(
        built.input,
        json!({"goal": {"title": "Green build"}, "research_dir": true})
    );
    let steps = built.graph.nodes[1].params["steps"]
        .as_array()
        .unwrap()
        .clone();
    let names: Vec<_> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Work", "Second", "Check criteria"]);
    // A research goal checks its report (goal.md §2.5).
    let last = steps[2]["params"]["criteria"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(
        (last["id"].clone(), last["command"].clone()),
        (json!("report"), json!("test -s findings.md"))
    );
}

fn review() -> Review {
    serde_json::from_value(json!({"id":"r1","repo_id":"repo","pr_number":3,"status":"done","error":null,
        "comments":[{"id":"c1","review_id":"r1","path":"auth.rs","line":2,"severity":"bug","body":"Permission bypass at auth.rs:2","state":"draft","posted":false,"created_at":"2026-11-07T00:00:00Z"},
                    {"id":"c2","review_id":"r1","path":"a.rs","line":1,"severity":"info","body":"declined one","state":"declined","posted":false,"created_at":"2026-11-07T00:00:00Z"}],
        "agents":[],"created_at":"2026-11-07T00:00:00Z",
        "scope_plan":{"base_sha":"base1","head_sha":"abc","summary":"Permission change","related_modules":["auth"],"risks":["bypass"],
            "lenses":[{"name":"Security","reason":"Permission boundary changed","instructions":"Trace caller permissions"}]}})).unwrap()
}

#[test]
fn fix_review_fixes_first_then_reviews_against_the_merge_base_chained_to_the_review() {
    let built = fix_review_workflow(&review(), 9.5, "codex", "/src/app", "abc").unwrap();
    assert_eq!(built.name, "Goal · Fix PR #3 review findings");
    let lp = &built.graph.nodes[1];
    assert_eq!(
        (
            lp.id.as_str(),
            lp.params["max_iterations"].clone(),
            lp.params["max_active_s"].clone()
        ),
        ("iterate", json!(6), json!(7200))
    );
    assert_eq!(lp.params["hold_when_unmet"], true);
    let steps = lp.params["steps"].as_array().unwrap();
    let kinds: Vec<_> = steps.iter().map(|s| s["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["agent_prompt", "review_run", "check"]);
    let rp = &steps[1]["params"];
    assert_eq!(
        (
            rp["base"].clone(),
            rp["previous_review_id"].clone(),
            rp["chain_iterations"].clone(),
            rp["threshold"].clone()
        ),
        (json!("base1"), json!("r1"), json!(true), json!(95))
    );
    assert!(rp.get("require_pass").is_none());
    assert_eq!(
        rp["reviewers"],
        json!([{"lens": "Security", "providers": ["codex"], "instructions": "Trace caller permissions"}])
    );
    let ids: Vec<_> = steps[2]["params"]["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["review-passes", "critical-checks"]);
    let fix = steps[0]["params"]["prompt"].as_str().unwrap();
    assert!(
        fix.contains("Permission bypass") && !fix.contains("declined one"),
        "{fix}"
    );
    assert_eq!(
        built.input["repos"],
        json!([{"repo": "repo", "type": "worktree", "name": "/src/app", "source": "abc"}])
    );
    assert_eq!(blocking(&built.graph), Vec::<String>::new());
    assert!(fix_review_workflow(&review(), 10.5, "codex", "/p", "h").is_err());
    let mut legacy = review();
    legacy.scope_plan = None;
    assert!(fix_review_workflow(&legacy, 9.5, "codex", "/p", "h").is_err());
}

fn opts(start_with: ReviewFixStart, review_params: Value) -> ReviewFixOpts {
    ReviewFixOpts {
        max_iterations: 3,
        max_active_s: None,
        start_with,
        review_params,
        fix_prompt: "fix it".into(),
        extra_criteria: vec![],
        hold_when_unmet: false,
        stop_on_repeat_failure: false,
        fix_agent: AgentChoice::default(),
        checker: AgentChoice::default(),
        x: 900.0,
    }
}

#[test]
fn review_fix_loop_reviews_first_for_templates_and_refuses_require_pass() {
    let n = review_fix_loop(opts(ReviewFixStart::Review, json!({"threshold": 80}))).unwrap();
    let names: Vec<_> = n.params["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["review", "check", "fix"]);
    assert!(n.params.get("until").is_none() && n.params.get("hold_when_unmet").is_none());
    assert_eq!(
        n.params["steps"][1]["params"]["criteria"][0]["expr"],
        "steps.review.passed == true"
    );
    assert!(review_fix_loop(opts(ReviewFixStart::Review, json!({"require_pass": true}))).is_err());
}

fn loop_args(
    kind: &str,
    cmd: Option<&str>,
) -> (GoalLoopDefinition, GoalLoopLimits, GoalLoopConfig) {
    let definition: GoalLoopDefinition = serde_json::from_value(json!({"title": "T", "summary": "S", "success_signal": "green",
        "acceptance_criteria": [{"id": "AC 1", "text": "x", "verify": "cargo test", "verify_kind": kind, "verify_cmd": cmd}]})).unwrap();
    (
        definition,
        GoalLoopLimits {
            max_iterations: 99,
            max_runtime_secs: 60,
            per_phase_timeout_secs: 600,
            ..Default::default()
        },
        GoalLoopConfig::default(),
    )
}

#[test]
fn goal_loop_args_map_onto_a_goal_spec() {
    let (d, l, mut c) = loop_args("command", None);
    c.executors.push(otto_core::domain::GoalLoopAgentCfg {
        name: "B".into(),
        provider: "claude".into(),
        model: String::new(),
        prompt_extra: String::new(),
    });
    c.require_review = true;
    let (s, ignored) = goal_spec_from_loop_args(&d, &l, &c, Some("r1")).unwrap();
    assert_eq!(ignored, ["planner", "digester", "definer"]);
    assert_eq!(
        (
            s.criteria[0].id.as_str(),
            s.criteria[0].verify,
            s.criteria[0].command.as_deref()
        ),
        ("ac-1", VerifyKind::Command, Some("cargo test"))
    );
    assert_eq!(
        (
            s.budget.iterations,
            s.budget.active_s,
            s.budget.step_timeout_s
        ),
        (50, 900, 600)
    );
    assert_eq!(s.goal, "S\n\nSuccess looks like: green");
    assert_eq!(
        (
            s.worker.provider.as_deref(),
            s.extra_workers.len(),
            s.extra_workers[0].name.as_str()
        ),
        (Some("claude"), 1, "B")
    );
    // The default evaluator prompt is not carried as instructions; its model is.
    assert_eq!(
        (
            s.checker.model.as_deref(),
            s.checker.instructions.as_deref()
        ),
        (Some("sonnet"), None)
    );
    assert!(s.final_review);
    assert_eq!(
        s.work,
        GoalWork::Build {
            repo_id: "r1".into(),
            base: None
        }
    );
    assert_eq!(validate_spec(&s), Ok(()));

    for (kind, want) in [
        ("agent", VerifyKind::Agent),
        ("manual", VerifyKind::Agent),
        ("human", VerifyKind::Human),
    ] {
        let (d, l, c) = loop_args(kind, None);
        let c0 = &goal_spec_from_loop_args(&d, &l, &c, Some("r1"))
            .unwrap()
            .0
            .criteria[0];
        assert_eq!(
            (c0.verify, c0.how.as_str(), c0.command.is_none()),
            (want, "cargo test", true)
        );
    }
    let (d, l, mut c) = loop_args("agent", None);
    c.mode = "research".into();
    assert_eq!(
        goal_spec_from_loop_args(&d, &l, &c, None).unwrap().0.work,
        GoalWork::Research
    );
    c.mode = "build".into();
    assert!(goal_spec_from_loop_args(&d, &l, &c, None).is_err());
    c.graph = Some(otto_core::domain::GoalLoopGraph { nodes: vec![] });
    assert!(goal_spec_from_loop_args(&d, &l, &c, Some("r1"))
        .unwrap_err()
        .contains("workflow canvas"));
    let (d, mut l, c) = loop_args("agent", None);
    l.max_cost_usd = Some(5.0);
    assert!(goal_spec_from_loop_args(&d, &l, &c, Some("r1"))
        .unwrap_err()
        .contains("max_cost_usd"));
}

fn held_run(hold: Value, goal: Option<Value>) -> otto_core::workflows::WorkflowRun {
    serde_json::from_value(json!({"id": "run", "workflow_id": "wf", "workspace_id": "ws", "status": "error",
        "input": {}, "started_at": "2026-11-07T00:00:00Z", "hold": hold,
        "overview": goal.map(|g| json!({"goal": g})),
        "nodes": [{"node_id": "goal", "status": "running"}, {"node_id": "done", "status": "success"}]}))
    .unwrap()
}

#[test]
fn resume_needs_a_hold_and_room_and_writes_the_extension() {
    let graph = goal_workflow(&spec(), Some(&repo())).graph;
    let exhausted = json!({"kind": "exhausted", "reason": "Out of iterations", "since": "2026-11-07T00:00:00Z", "node_ids": ["goal"], "iteration": 7});
    let used = json!({"iteration": 7, "max_iterations": 7, "criteria_met": 1, "criteria_total": 3, "active_ms": 3_600_000, "max_active_ms": 7_200_000});
    let run = held_run(exhausted, Some(used));
    let none = ExtendReq::default();
    assert_eq!(
        resume_plan(&run, &graph, &none).unwrap_err(),
        "Raise the iteration limit to resume: 7 of 7 iterations are used."
    );
    let more = ExtendReq {
        iterations: Some(9),
        active_s: Some(3_000),
    };
    assert!(resume_plan(&run, &graph, &more)
        .unwrap_err()
        .starts_with("Raise the active-time limit to resume: 60 of 50 minutes"));
    let more = ExtendReq {
        iterations: Some(9),
        active_s: None,
    };
    let plan = resume_plan(&run, &graph, &more).unwrap();
    let l = &plan.limits.unwrap().loops["goal"];
    assert_eq!(
        (l.max_iterations, l.max_active_s, l.unblocked_at),
        (Some(9), None, None)
    );
    // A crash left the loop running: it re-enters as pending, the rest is adopted.
    assert_eq!(plan.unsettled, ["goal"]);
    assert_eq!(plan.nodes[0].status, NodeStatus::Pending);

    let blocked = json!({"kind": "blocked", "reason": "stuck", "since": "2026-11-07T00:00:00Z", "node_ids": [], "iteration": 4});
    let plan = resume_plan(&held_run(blocked, None), &graph, &none).unwrap();
    assert_eq!(plan.limits.unwrap().loops["goal"].unblocked_at, Some(4));
    let mut free = held_run(json!(null), None);
    free.hold = None;
    assert!(resume_plan(&free, &graph, &none).is_err());
}

#[test]
fn a_person_records_only_the_criterion_the_check_waits_on() {
    let graph = goal_workflow(&spec(), Some(&repo())).graph;
    assert_eq!(
        check_criteria(&graph, "goal")
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["tests", "docs", "ok"]
    );
    assert!(check_criteria(&graph, "trigger").is_empty());
    let mut run = held_run(
        json!(null),
        Some(json!({"iteration": 2, "max_iterations": 7, "criteria_met": 2, "criteria_total": 3})),
    );
    let at = ("goal", "ok");
    let err = verify_entry(&run, &graph, at, true, "saw it", "Dana").unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    run.nodes[0].attention = Some(serde_json::from_value(json!({"kind": "verify", "text": "Check it", "since": "2026-11-07T00:00:00Z", "criterion_id": "ok"})).unwrap());
    assert!(matches!(
        verify_entry(&run, &graph, at, true, "  ", "Dana"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        verify_entry(&run, &graph, ("goal", "docs"), true, "x", "Dana"),
        Err(Error::Conflict(_))
    ));
    let e = verify_entry(&run, &graph, at, false, " not yet ", "Dana").unwrap();
    assert_eq!(
        (
            e.kind.as_str(),
            e.ask.as_deref(),
            e.text.as_str(),
            e.met,
            e.iteration,
            e.by.as_str()
        ),
        (
            "verify",
            Some("ok"),
            "not yet",
            Some(false),
            Some(2),
            "Dana"
        )
    );
    assert_eq!(
        e.revision.unwrap(),
        crate::criteria::revision(&spec().criteria[2])
    );
}

#[test]
fn continuing_a_loop_carries_its_digest_next_action_and_tasks() {
    let mut lp: GoalLoop = serde_json::from_value(json!({"id": "l1", "workspace_id": "w", "name": "n", "repo_path": "/p",
        "definition": {"title": "T", "acceptance_criteria": []}, "limits": GoalLoopLimits::default(),
        "config": GoalLoopConfig::default(), "status": "paused", "phase": "waiting", "iterations_started": 2,
        "current_iteration": 2, "progress_pct": 40, "context_digest": "half done", "ledger": {"next_action": "wire it"},
        "elapsed_secs": 10, "cost_usd": 0.0, "created_by": "u", "created_at": "2026-11-07T00:00:00Z", "updated_at": "2026-11-07T00:00:00Z"})).unwrap();
    lp.config.graph = Some(
        serde_json::from_value(json!({"nodes": [
        {"id": "a", "title": "Parse", "instructions": "parse it", "criterion_ids": []}]}))
        .unwrap(),
    );
    let mut s = spec();
    carry_loop_context(&mut s, &lp);
    assert!(s.goal.ends_with("Tasks, in order:\n- Parse: parse it\n\nWhere the earlier loop got to: half done\n\nNext action: wire it"), "{}", s.goal);
}
