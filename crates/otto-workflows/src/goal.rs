//! Goals are workflows (goal.md §3.2, §4.1): the pure builders that turn a
//! [`GoalSpec`], a Goal Loops request or a stored review into the fixed shape
//! Start → loop { Work → Check criteria }, and the one review→fix loop the
//! bundled templates and Git's "Fix review" share.

use otto_core::agent_runtime::AttentionKind;
use otto_core::api::{AgentChoice, ExtendReq, GoalBudget, GoalSpec, GoalWork, NamedAgentChoice};
use otto_core::domain::{
    CommentState, GoalLoop, GoalLoopConfig, GoalLoopDefinition, GoalLoopLimits, Review,
    DEFAULT_EVALUATOR_PROMPT,
};
use otto_core::workflows::{
    Criterion, HoldKind, HumanEntry, NodeRunState, NodeStatus, RunLimits, VerifyKind, WorkflowEdge,
    WorkflowGraph, WorkflowNode, WorkflowRun,
};
use otto_core::Error;
use serde_json::{json, Value};

/// A built goal: what the server stores as the workflow row and the run input.
#[derive(Debug, Clone)]
pub struct BuiltGoal {
    pub name: String,
    pub description: String,
    pub graph: WorkflowGraph,
    pub input: Value,
}

/// The repository a build goal works in: its id, its path (the `repos[]`
/// entry's name, DR-102) and the commit the run worktree is cut from.
pub struct GoalRepo<'a> {
    pub id: &'a str,
    pub path: &'a str,
    pub source: &'a str,
}

/// Every rule a spec must meet before anything is stored (goal.md §3.1).
pub fn validate_spec(spec: &GoalSpec) -> Result<(), String> {
    if !(1..=120).contains(&spec.title.trim().chars().count()) {
        return Err("title: 1 to 120 characters".into());
    }
    if !(1..=8_000).contains(&spec.goal.trim().chars().count()) {
        return Err("goal: 1 to 8,000 characters".into());
    }
    if !(1..=40).contains(&spec.criteria.len()) {
        return Err("criteria: add 1 to 40 criteria".into());
    }
    let mut seen = std::collections::HashSet::new();
    for (i, c) in spec.criteria.iter().enumerate() {
        let id_ok = (1..=64).contains(&c.id.len())
            && c.id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok || !seen.insert(c.id.as_str()) {
            return Err(format!("criteria[{i}].id: a unique id of a-z, 0-9 and '-'"));
        }
        if c.text.trim().is_empty() {
            return Err(format!("criteria[{i}].text: say what must be true"));
        }
        let blank = |v: &Option<String>| v.as_deref().is_none_or(|s| s.trim().is_empty());
        match c.verify {
            VerifyKind::Command if blank(&c.command) => {
                return Err(format!(
                    "criteria[{i}].command: a command criterion needs a command"
                ))
            }
            VerifyKind::Expr if blank(&c.expr) => {
                return Err(format!("criteria[{i}].expr: a rule criterion needs a rule"))
            }
            VerifyKind::Unknown => return Err(format!("criteria[{i}].verify: unknown kind")),
            _ => {}
        }
    }
    let GoalBudget {
        iterations,
        active_s,
        step_timeout_s,
    } = spec.budget;
    if !(1..=50).contains(&iterations) {
        return Err("budget.iterations: 1 to 50".into());
    }
    if !(900..=28_800).contains(&active_s) {
        return Err("budget.active_s: 15 minutes to 8 hours".into());
    }
    if !(300..=7_200).contains(&step_timeout_s) {
        return Err("budget.step_timeout_s: 5 minutes to 2 hours".into());
    }
    Ok(())
}

/// The rules every goal worker keeps (Goal Loops' executor rules).
const WORK_RULES: &str = "Working rules: local edits only in this working directory. Never stage, commit, push, publish, merge, open pull requests or send messages. Respect changes that were already there. Budget exhaustion or a check you could not run is incomplete, never success.";

fn list(out: &mut String, head: &str, items: &[String]) {
    if !items.is_empty() {
        out.push_str(&format!("\n\n{head}:\n"));
        for item in items {
            out.push_str(&format!("- {}\n", item.trim()));
        }
    }
}

fn how_checked(c: &Criterion) -> String {
    let or_empty = |v: &Option<String>| v.clone().unwrap_or_default();
    match c.verify {
        VerifyKind::Command => format!("a command will be run: {}", or_empty(&c.command)),
        VerifyKind::Human => format!("a person will check: {}", c.how),
        VerifyKind::Expr => format!("a rule over this iteration: {}", or_empty(&c.expr)),
        _ => format!("an independent checker will look for evidence: {}", c.how),
    }
}

/// The Work step's prompt: the only authored text of a goal.
pub fn work_prompt(spec: &GoalSpec, instructions: Option<&str>) -> String {
    let mut out = format!("Goal: {}\n\n{}", spec.title.trim(), spec.goal.trim());
    list(&mut out, "Objectives", &spec.objectives);
    list(&mut out, "Constraints", &spec.constraints);
    list(&mut out, "Out of scope", &spec.out_of_scope);
    list(&mut out, "Links", &spec.links);
    list(&mut out, "Skills to use", &spec.skills);
    out.push_str("\n\nDone when every criterion is met with evidence:\n");
    for c in &spec.criteria {
        out.push_str(&format!(
            "- [{}] {} ({})\n",
            c.id,
            c.text.trim(),
            how_checked(c)
        ));
    }
    out.push_str(&format!("\n{WORK_RULES}\n\nFrom the second iteration on, your input holds the last check's results (feedback_md) and the full table is in the previous Check criteria step file. Close the unmet criteria first."));
    if let Some(extra) = instructions.map(str::trim).filter(|s| !s.is_empty()) {
        out.push_str(&format!("\n\n{extra}"));
    }
    out
}

/// `provider` / `model` / `effort` of a choice, the empty ones left out.
fn picks(choice: &AgentChoice) -> serde_json::Map<String, Value> {
    [
        ("provider", &choice.provider),
        ("model", &choice.model),
        ("effort", &choice.effort),
    ]
    .into_iter()
    .filter_map(|(k, v)| {
        v.as_deref()
            .filter(|s| !s.is_empty())
            .map(|v| (k.to_string(), json!(v)))
    })
    .collect()
}

fn agent_params(choice: &AgentChoice, prompt: String, timeout_s: Option<u64>) -> Value {
    let mut p = picks(choice);
    p.insert("prompt".into(), json!(prompt));
    if let Some(t) = timeout_s {
        p.insert("timeout_s".into(), json!(t));
    }
    Value::Object(p)
}

fn node(id: &str, kind: &str, name: &str, x: f64, params: Value) -> WorkflowNode {
    WorkflowNode {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        x,
        y: 80.0,
        params,
        retry: None,
    }
}

/// Start (`lane: long`) → `body`: the two-node graph every goal has.
fn goal_graph(body: WorkflowNode) -> WorkflowGraph {
    let edge = WorkflowEdge {
        id: format!("trigger-{}", body.id),
        source: "trigger".into(),
        target: body.id.clone(),
        condition: None,
    };
    WorkflowGraph {
        nodes: vec![
            node(
                "trigger",
                "manual_trigger",
                "Start",
                40.0,
                json!({"lane": "long"}),
            ),
            body,
        ],
        edges: vec![edge],
    }
}

fn goal_input(title: &str, repo: Option<&GoalRepo<'_>>) -> Value {
    match repo {
        Some(r) => json!({"goal": {"title": title},
            "repos": [{"repo": r.id, "type": "worktree", "name": r.path, "source": r.source}]}),
        None => json!({"goal": {"title": title}, "research_dir": true}),
    }
}

/// The goal's workflow (goal.md §3.2). `repo` is used by a build goal only.
pub fn goal_workflow(spec: &GoalSpec, repo: Option<&GoalRepo<'_>>) -> BuiltGoal {
    let budget = &spec.budget;
    let work = |name: &str, choice: &AgentChoice| {
        let prompt = work_prompt(spec, choice.instructions.as_deref());
        json!({"kind": "agent_prompt", "name": name, "same_thread": true,
            "params": agent_params(choice, prompt, Some(budget.step_timeout_s))})
    };
    let mut steps = vec![work("Work", &spec.worker)];
    for w in &spec.extra_workers {
        let choice = AgentChoice {
            provider: w.provider.clone(),
            model: w.model.clone(),
            effort: w.effort.clone(),
            instructions: w.instructions.clone(),
        };
        steps.push(work(&w.name, &choice));
    }
    let mut criteria = spec.criteria.clone();
    let research = spec.work == GoalWork::Research;
    if research && !criteria.iter().any(|c| c.id == "report") {
        criteria.push(Criterion {
            id: "report".into(),
            text: "findings.md exists and is not empty".into(),
            verify: VerifyKind::Command,
            how: String::new(),
            command: Some("test -s findings.md".into()),
            expr: None,
            timeout_s: None,
        });
    }
    steps.push(
        json!({"kind": "check", "name": "Check criteria", "params": {
        "criteria": criteria, "evaluator": picks(&spec.checker),
        "instructions": spec.checker.instructions.clone().unwrap_or_default(),
        "final_review": spec.final_review}}),
    );
    let body = node(
        "goal",
        "loop",
        "Work toward the goal",
        300.0,
        json!({
        "max_iterations": budget.iterations, "max_active_s": budget.active_s,
        "stop_on_repeat_failure": true, "hold_when_unmet": true, "steps": steps}),
    );
    let repo = repo.filter(|_| !research);
    BuiltGoal {
        name: format!("Goal · {}", spec.title.trim()),
        description: spec.goal.trim().lines().next().unwrap_or("").to_string(),
        graph: goal_graph(body),
        input: goal_input(spec.title.trim(), repo),
    }
}

/// Which step of the review→fix loop comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewFixStart {
    /// The work already exists (templates): review, then fix.
    Review,
    /// A review already exists (Fix review): fix, then review.
    Fix,
}

pub struct ReviewFixOpts {
    pub max_iterations: u32,
    pub max_active_s: Option<u64>,
    pub start_with: ReviewFixStart,
    /// The review_run params (threshold, reviewers, summarizer, scoring, base,
    /// previous_review_id, chain_iterations…).
    pub review_params: Value,
    pub fix_prompt: String,
    pub extra_criteria: Vec<Criterion>,
    pub hold_when_unmet: bool,
    pub stop_on_repeat_failure: bool,
    pub fix_agent: AgentChoice,
    pub checker: AgentChoice,
    /// Canvas position of the loop node.
    pub x: f64,
}

/// The one review→fix rule (goal.md §4.1): loop "iterate" { [fix] → review →
/// check (rule "the review passes" + extra) → [fix] }. A passing check ends
/// the loop, so `require_pass` is refused (DR-103).
pub fn review_fix_loop(opts: ReviewFixOpts) -> Result<WorkflowNode, String> {
    if opts.review_params.get("require_pass").is_some() {
        return Err("review_fix_loop: the check is the gate, drop require_pass".into());
    }
    let mut criteria = vec![Criterion {
        id: "review-passes".into(),
        text: "A fresh independent review passes".into(),
        verify: VerifyKind::Expr,
        how: String::new(),
        command: None,
        expr: Some("steps.review.passed == true".into()),
        timeout_s: None,
    }];
    criteria.extend(opts.extra_criteria);
    let review = json!({"kind": "review_run", "name": "review", "params": opts.review_params});
    let check = json!({"kind": "check", "name": "check", "params": {"criteria": criteria, "evaluator": picks(&opts.checker)}});
    let fix = json!({"kind": "agent_prompt", "name": "fix", "params": agent_params(&opts.fix_agent, opts.fix_prompt, None)});
    let steps = match opts.start_with {
        ReviewFixStart::Review => vec![review, check, fix],
        ReviewFixStart::Fix => vec![fix, review, check],
    };
    let mut params = json!({"max_iterations": opts.max_iterations, "steps": steps});
    if let Some(s) = opts.max_active_s {
        params["max_active_s"] = json!(s);
    }
    if opts.hold_when_unmet {
        params["hold_when_unmet"] = json!(true);
    }
    if opts.stop_on_repeat_failure {
        params["stop_on_repeat_failure"] = json!(true);
    }
    Ok(node(
        "iterate",
        "loop",
        "Review → fix until passing",
        opts.x,
        params,
    ))
}

const FIX_RULES: &str = "Local isolated-worktree edits only. Never stage, commit, push, publish, merge, open PRs, or send messages. Respect existing user changes. Only fix issues newly caused by the reviewed PR; summarize preexisting widened issues once without opening finding threads. Budget exhaustion or unavailable verification is incomplete, never success. Scores must not override unresolved findings.";

/// Git's "Fix review" as a goal (goal.md §4.1): from the reviewed commit,
/// fix → review against the merge-base, chained to the original review →
/// check, up to 6 iterations or 2 hours of active time.
pub fn fix_review_workflow(
    review: &Review,
    target: f64,
    provider: &str,
    repo_path: &str,
    head: &str,
) -> Result<BuiltGoal, String> {
    if !target.is_finite() || !(1.0..=10.0).contains(&target) {
        return Err("Target score must be between 1 and 10".into());
    }
    let plan = review
        .scope_plan
        .as_ref()
        .ok_or("Re-run this legacy review to assess scope before fixing")?;
    let findings: Vec<_> = review
        .comments
        .iter()
        .filter(|c| c.state != CommentState::Declined)
        .collect();
    let reviewers: Vec<Value> = plan
        .lenses
        .iter()
        .map(|l| json!({"lens": l.name, "providers": [provider], "instructions": l.instructions}))
        .collect();
    let fix_prompt = format!(
        "Fix the findings of review {} of PR #{} (at {}, merge-base {}). Treat them as hypotheses to verify, not orders to blindly patch; batch related fixes by cause, must-fix and should-fix before optional polish. From the second iteration on, your input holds the latest review: address what it still reports.\n\nScope: {}\n\nFindings:\n{}\n\n{FIX_RULES}",
        review.id, review.pr_number, plan.head_sha, plan.base_sha, plan.summary,
        serde_json::to_string(&findings).unwrap_or_default());
    let critical = Criterion {
        id: "critical-checks".into(),
        text: "Relevant critical-path checks pass with recorded evidence".into(),
        verify: VerifyKind::Agent,
        how: "Run the relevant repository checks for changed critical paths. Record exact commands, exit status and output summary. Tests must fail for the original defect; do not demand exhaustive hypothetical branch coverage. Skipped or unavailable material checks leave this criterion unmet.".into(),
        command: None,
        expr: None,
        timeout_s: None,
    };
    let agent = AgentChoice {
        provider: Some(provider.into()),
        ..Default::default()
    };
    let body = review_fix_loop(ReviewFixOpts {
        max_iterations: 6,
        max_active_s: Some(7_200),
        start_with: ReviewFixStart::Fix,
        review_params: json!({"base": plan.base_sha, "previous_review_id": review.id,
            "chain_iterations": true, "threshold": (target * 10.0).round() as u64,
            "reviewers": reviewers, "summarizer": {"provider": provider}}),
        fix_prompt,
        extra_criteria: vec![critical],
        hold_when_unmet: true,
        stop_on_repeat_failure: true,
        fix_agent: agent.clone(),
        checker: agent,
        x: 300.0,
    })?;
    let title = format!("Fix PR #{} review findings", review.pr_number);
    Ok(BuiltGoal {
        name: format!("Goal · {title}"),
        description: format!(
            "Fix the findings of review {} until a fresh review scores {target:.1}/10",
            review.id
        ),
        graph: goal_graph(body),
        input: goal_input(
            &title,
            Some(&GoalRepo {
                id: &review.repo_id,
                path: repo_path,
                source: head,
            }),
        ),
    })
}

/// A Goal Loops criterion id as a check criterion id (`[a-z0-9-]{1,64}`).
fn criterion_id(raw: &str, i: usize) -> String {
    let id: String = raw
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    match id.trim_matches('-') {
        "" => format!("c{}", i + 1),
        id => id.to_string(),
    }
}

fn some(s: &str) -> Option<String> {
    Some(s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Goal Loops' shapes (MCP `otto.run_goal_loop`, Continue as a goal) as a
/// [`GoalSpec`] plus the roles a goal no longer has (goal.md §3.2). `repo_id`
/// is the registered repository of a build loop.
pub fn goal_spec_from_loop_args(
    definition: &GoalLoopDefinition,
    limits: &GoalLoopLimits,
    config: &GoalLoopConfig,
    repo_id: Option<&str>,
) -> Result<(GoalSpec, Vec<String>), String> {
    if config.graph.is_some() {
        return Err("Engineering-graph goals are not supported any more: put the tasks on a workflow canvas".into());
    }
    if limits.max_cost_usd.is_some() {
        return Err(
            "max_cost_usd is not supported: a goal's cost is not tracked; leave it null".into(),
        );
    }
    let criteria = definition
        .acceptance_criteria
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let verify = match c.verify_kind.as_str() {
                "command" => VerifyKind::Command,
                "human" => VerifyKind::Human,
                _ => VerifyKind::Agent,
            };
            let command = (verify == VerifyKind::Command).then(|| {
                c.verify_cmd
                    .as_deref()
                    .and_then(some)
                    .unwrap_or_else(|| c.verify.clone())
            });
            let how = if command.is_some() {
                String::new()
            } else {
                c.verify.clone()
            };
            Criterion {
                id: criterion_id(&c.id, i),
                text: c.text.clone(),
                verify,
                how,
                command,
                expr: None,
                timeout_s: None,
            }
        })
        .collect();
    let work = if config.mode == "research" {
        GoalWork::Research
    } else {
        GoalWork::Build {
            repo_id: repo_id
                .ok_or("repo_path must be a registered repository")?
                .into(),
            base: None,
        }
    };
    let mut executors = config.executors.iter().map(|e| NamedAgentChoice {
        name: e.name.clone(),
        provider: some(&e.provider),
        model: some(&e.model),
        effort: None,
        instructions: some(&e.prompt_extra),
    });
    let worker = executors
        .next()
        .map(|e| AgentChoice {
            provider: e.provider,
            model: e.model,
            effort: None,
            instructions: e.instructions,
        })
        .unwrap_or_default();
    let ev = &config.evaluator;
    let checker = AgentChoice {
        provider: some(&ev.provider),
        model: some(&ev.model),
        effort: None,
        instructions: some(&ev.prompt).filter(|p| p != DEFAULT_EVALUATOR_PROMPT),
    };
    let mut goal = some(&definition.summary).unwrap_or_else(|| definition.title.clone());
    if let Some(signal) = some(&definition.success_signal) {
        goal.push_str(&format!("\n\nSuccess looks like: {signal}"));
    }
    let spec = GoalSpec {
        title: definition.title.trim().chars().take(120).collect(),
        goal,
        objectives: definition.objectives.clone(),
        constraints: definition.constraints.clone(),
        out_of_scope: definition.out_of_scope.clone(),
        criteria,
        work,
        budget: GoalBudget {
            iterations: limits.max_iterations.clamp(1, 50),
            active_s: limits.max_runtime_secs.clamp(900, 28_800),
            step_timeout_s: limits.per_phase_timeout_secs.clamp(300, 7_200),
        },
        worker,
        extra_workers: executors.collect(),
        checker,
        final_review: config.require_review,
        links: config.source_links.clone(),
        skills: config.skills.clone(),
        continue_loop_id: None,
        from: None,
    };
    Ok((
        spec,
        vec!["planner".into(), "digester".into(), "definer".into()],
    ))
}

/// Continue as a goal (goal.md §5.3, DR-116): what the older loop had learnt
/// goes into the goal text; a graph loop's tasks are listed in order.
pub fn carry_loop_context(spec: &mut GoalSpec, loop_: &GoalLoop) {
    if let Some(graph) = &loop_.config.graph {
        let tasks: Vec<String> = graph
            .nodes
            .iter()
            .map(|n| format!("{}: {}", n.title, n.instructions))
            .collect();
        spec.goal
            .push_str(&format!("\n\nTasks, in order:\n- {}", tasks.join("\n- ")));
    }
    if let Some(digest) = some(&loop_.context_digest) {
        spec.goal
            .push_str(&format!("\n\nWhere the earlier loop got to: {digest}"));
    }
    if let Some(next) = some(&loop_.ledger.next_action) {
        spec.goal.push_str(&format!("\n\nNext action: {next}"));
    }
    spec.goal = spec.goal.chars().take(8_000).collect();
}

/// Resume of a held run (goal.md §2.3): the loop's limits after Extend and
/// the step states to re-enter with.
#[derive(Debug)]
pub struct ResumePlan {
    /// `None`: the graph has no loop, nothing to write.
    pub limits: Option<RunLimits>,
    /// The run's steps, a `running` one (left by a crash) back to `pending`.
    pub nodes: Vec<NodeRunState>,
    /// Every step not settled: what the retry re-enters.
    pub unsettled: Vec<String>,
}

/// Err: why the run cannot resume (no hold, or an exhausted run without the
/// room to go on — Goal Loops' wording).
pub fn resume_plan(
    run: &WorkflowRun,
    graph: &WorkflowGraph,
    extend: &ExtendReq,
) -> Result<ResumePlan, String> {
    let hold = run
        .hold
        .as_ref()
        .ok_or("This run is not paused, blocked or out of budget")?;
    if let (HoldKind::Exhausted, Some(g)) = (
        hold.kind,
        run.overview.as_ref().and_then(|o| o.goal.as_ref()),
    ) {
        let max = extend.iterations.unwrap_or(g.max_iterations);
        if max <= g.iteration {
            return Err(format!(
                "Raise the iteration limit to resume: {} of {max} iterations are used.",
                g.iteration
            ));
        }
        let max_ms = extend.active_s.map(|s| s * 1_000).or(g.max_active_ms);
        if let Some(max_ms) = max_ms.filter(|m| *m <= g.active_ms) {
            let min = |ms: u64| ms / 60_000;
            return Err(format!(
                "Raise the active-time limit to resume: {} of {} minutes are used.",
                min(g.active_ms),
                min(max_ms)
            ));
        }
    }
    let loop_id = hold.node_ids.first().cloned().or_else(|| {
        graph
            .nodes
            .iter()
            .find(|n| n.kind == "loop")
            .map(|n| n.id.clone())
    });
    let limits = loop_id.map(|id| {
        let mut limits = run.limits.clone().unwrap_or_default();
        let l = limits.loops.entry(id).or_default();
        l.max_iterations = extend.iterations.or(l.max_iterations);
        l.max_active_s = extend.active_s.or(l.max_active_s);
        if hold.kind == HoldKind::Blocked {
            l.unblocked_at = hold.iteration.or(l.unblocked_at);
        }
        limits
    });
    let mut nodes = run.nodes.clone();
    for n in nodes.iter_mut().filter(|n| n.status == NodeStatus::Running) {
        n.status = NodeStatus::Pending;
    }
    let unsettled = nodes
        .iter()
        .filter(|n| {
            !matches!(
                n.status,
                NodeStatus::Success | NodeStatus::Error | NodeStatus::Skipped
            )
        })
        .map(|n| n.node_id.clone())
        .collect();
    Ok(ResumePlan {
        limits,
        nodes,
        unsettled,
    })
}

/// The criteria of the check at `node_id`, or of the checks inside that loop.
pub fn check_criteria(graph: &WorkflowGraph, node_id: &str) -> Vec<Criterion> {
    let Some(node) = graph.nodes.iter().find(|n| n.id == node_id) else {
        return vec![];
    };
    let inner = node
        .params
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| s["kind"] == "check")
        .map(|s| &s["params"]);
    std::iter::once(&node.params)
        .chain(inner)
        .filter_map(|p| serde_json::from_value::<Vec<Criterion>>(p.get("criteria")?.clone()).ok())
        .flatten()
        .collect()
}

/// A person's record for criterion `criterion_id` that the check at the
/// top-level `node_id` waits on (goal.md §2.4, DR-107), bound to the
/// criterion's revision and the current iteration.
pub fn verify_entry(
    run: &WorkflowRun,
    graph: &WorkflowGraph,
    (node_id, criterion_id): (&str, &str),
    met: bool,
    evidence: &str,
    by: &str,
) -> otto_core::Result<HumanEntry> {
    let evidence = evidence.trim();
    if evidence.is_empty() || evidence.chars().count() > 2_000 {
        return Err(Error::Invalid(
            "evidence: say what you checked (at most 2,000 characters)".into(),
        ));
    }
    let waiting = run
        .nodes
        .iter()
        .find(|n| n.node_id == node_id)
        .and_then(|n| n.attention.as_ref())
        .is_some_and(|a| {
            a.kind == AttentionKind::Verify && a.criterion_id.as_deref() == Some(criterion_id)
        });
    if !waiting {
        return Err(Error::Conflict(format!(
            "{node_id} is not waiting for a person on {criterion_id}"
        )));
    }
    let criterion = check_criteria(graph, node_id)
        .into_iter()
        .find(|c| c.id == criterion_id)
        .ok_or_else(|| Error::NotFound(format!("no criterion {criterion_id}")))?;
    Ok(HumanEntry {
        kind: "verify".into(),
        ask: Some(criterion_id.into()),
        text: evidence.into(),
        met: Some(met),
        revision: Some(crate::criteria::revision(&criterion)),
        iteration: run
            .overview
            .as_ref()
            .and_then(|o| o.goal.as_ref())
            .map(|g| g.iteration),
        by: by.into(),
        at: chrono::Utc::now(),
    })
}

#[cfg(test)]
#[path = "goal_tests.rs"]
mod tests;
