//! Optimize (addendum-chat-optimize.md §2, DR-110, DR-113, DR-114, DR-202):
//! drafts a faster version of a stored workflow from rules alone — no agent,
//! no quota — and checks every draft before anything is offered. Nothing is
//! saved here: the apply route re-derives the graph from its base version
//! and the chosen change ids with [`apply`] (never a client graph).
//!
//! Each rule yields [`OptimizeChange`]s; replaced nodes keep their id and
//! name, so step files and every `stepN` reference keep resolving. Changes
//! are added one at a time in a fixed order, each kept only when the draft
//! with it still passes every check (validation, a DAG on the same triggers,
//! the numbering dry-run over condition outcomes, the reads/writes table, the
//! order of outward steps and the never-change list); one that fails is
//! listed under "Not offered" with the reason.

use crate::context::slug;
use crate::graph::{ancestors, topo_order};
use otto_core::workflows::{
    NodeStatus, RunStatus, StepPhase, Workflow, WorkflowEdge, WorkflowGraph, WorkflowNode,
    WorkflowRun,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Parallel,
    GitCheckout,
    GoGatesScript,
    StepTimeout,
    SingleRepoSkip,
    PostReviewComments,
    ReviewDeclaredLenses,
    /// Reserved: an agent pass would use the same shape; not built.
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    SameContract,
    MayChangeOutput,
    ChangesFindings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizeChange {
    /// `<rule>:<node id>` — stable for a workflow version.
    pub id: String,
    pub rule: Rule,
    pub nodes: Vec<String>,
    pub title: String,
    pub why: String,
    pub effect: Effect,
    pub preselected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saving_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotOffered {
    pub rule: Rule,
    pub nodes: Vec<String>,
    pub reason: String,
}

/// From the workflow's own successful runs; no numbers under 3 runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub runs: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typical_before_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typical_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow_before_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberingCheck {
    pub step_no: u32,
    pub node_id: String,
    pub ok: bool,
}

/// A step file whose number changes (never one a prompt refers to).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Renumbered {
    pub node_id: String,
    pub from: String,
    pub to: String,
}

/// One row of the Reads & Writes table (DR-114).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCheck {
    pub node_id: String,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub reads_added: Vec<String>,
    pub reads_lost: Vec<String>,
    /// Earlier step files no longer in its "prior step summaries" listing.
    pub listing_removed: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Checks {
    pub numbering: Vec<NumberingCheck>,
    pub renumbered: Vec<Renumbered>,
    pub files: Vec<FileCheck>,
    pub warnings: Vec<String>,
}

/// `POST /workflows/{id}/optimize`'s answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizeDraft {
    pub base_version: i64,
    pub changes: Vec<OptimizeChange>,
    pub not_offered: Vec<NotOffered>,
    pub draft_graph: WorkflowGraph,
    pub estimate: Estimate,
    pub checks: Checks,
}

/// What the caller decides: which changes (`None` = the pre-selected ones),
/// the step-slot setting, and the validation a draft must pass (the save
/// rules; a message per blocking problem).
pub struct Options<'a> {
    pub selection: Option<&'a [String]>,
    pub max_parallel_steps: usize,
    pub validate: &'a dyn Fn(&WorkflowGraph) -> Vec<String>,
}

/// The draft for `opts.selection`, with its estimate and checks.
pub fn plan(wf: &Workflow, history: &[WorkflowRun], opts: &Options<'_>) -> OptimizeDraft {
    let built = build(&wf.graph, history, opts);
    let est = estimate(&wf.graph, &built, history, opts.max_parallel_steps);
    let mut changes = built.offered.clone();
    for c in &mut changes {
        c.saving_ms = est.savings.get(&c.id).copied();
    }
    OptimizeDraft {
        base_version: wf.version,
        changes,
        not_offered: built.not_offered.clone(),
        checks: report(&wf.graph, &built.graph, opts),
        draft_graph: built.graph,
        estimate: est.estimate,
    }
}

/// The graph to save for exactly `change_ids`, re-derived and re-checked,
/// with the offered changes (for [`version_note`]); `Err` (a 422) when an
/// id is unknown or no longer passes its checks.
pub fn apply(
    wf: &Workflow,
    history: &[WorkflowRun],
    change_ids: &[String],
    opts: &Options<'_>,
) -> Result<(WorkflowGraph, Vec<OptimizeChange>), String> {
    let opts = Options {
        selection: Some(change_ids),
        ..*opts
    };
    let built = build(&wf.graph, history, &opts);
    for id in change_ids {
        if built.applied.iter().any(|a| a == id) {
            continue;
        }
        let why = built
            .not_offered
            .iter()
            .find(|n| change_id(n.rule, &n.nodes[0]) == *id)
            .map_or("it no longer applies to this version", |n| {
                n.reason.as_str()
            });
        return Err(format!("{id}: {why}"));
    }
    Ok((built.graph, built.offered))
}

/// The version note: "Optimized by <user>: <change titles>".
pub fn version_note(user: &str, changes: &[OptimizeChange], change_ids: &[String]) -> String {
    let titles: Vec<&str> = changes
        .iter()
        .filter(|c| change_ids.contains(&c.id))
        .map(|c| c.title.as_str())
        .collect();
    format!("Optimized by {user}: {}", titles.join("; "))
}

fn change_id(rule: Rule, node: &str) -> String {
    let r = serde_json::to_value(rule).ok();
    format!(
        "{}:{node}",
        r.as_ref().and_then(Value::as_str).unwrap_or_default()
    )
}

// ---------------------------------------------------------------- rules --

#[derive(Debug, Clone)]
enum Edit {
    /// Same id and name, new kind and params.
    Replace {
        node: String,
        kind: &'static str,
        params: Value,
    },
    /// `post_review_comments`, with an optional "Draft PR summary" step in front.
    Post {
        node: String,
        params: Value,
        intro: Option<WorkflowNode>,
    },
    SetParam {
        node: String,
        key: &'static str,
        value: Value,
    },
    GuardEdges {
        node: String,
        condition: &'static str,
    },
    Parallel {
        node: String,
    },
}

struct Candidate {
    change: OptimizeChange,
    edit: Edit,
}

/// The template of the one short agent "Draft PR summary" step.
pub const DRAFT_SUMMARY_PROMPT: &str = "Write ONLY the 'Gotchas' section of this pull request's review summary: changes that may affect other flows in the system, based on the context brief, the cross-repo report and the review's findings in the run context. Keep it short, cite files, and post nothing — the next step posts the summary.";

/// What the outward catalog marks as mutating, in bare form (`comment_pr`).
fn mutating_tools() -> &'static [String] {
    static TOOLS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    TOOLS.get_or_init(|| {
        otto_mcp::outward::catalog::otto_tool_specs_cached()
            .iter()
            .filter(|s| s["mutating"] == true)
            .filter_map(|s| s["name"].as_str())
            .map(|n| n.trim_start_matches("otto.").to_string())
            .collect()
    })
}

fn prompt(n: &WorkflowNode) -> String {
    n.params
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase()
        .replace(['`', '\'', '"'], "")
}

fn skills(n: &WorkflowNode) -> Vec<String> {
    n.params
        .get("skills")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_lowercase)
        .collect()
}

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `word` in `text` with no word character right before or after it.
fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(i, _)| {
        !text[..i].chars().next_back().is_some_and(word_char)
            && !text[i + word.len()..].chars().next().is_some_and(word_char)
    })
}

/// The step reaches outside Otto (DR-113): a mutating outward tool in either
/// spelling, a raw HTTP/forge/push command, the daemon's token or address,
/// or a skill that posts or publishes. Non-agent kinds by what they are.
fn outward(n: &WorkflowNode) -> bool {
    match n.kind.as_str() {
        "agent_prompt" => {
            let p = prompt(n);
            mutating_tools()
                .iter()
                .any(|t| has_word(&p, t) || p.contains(&format!("otto_{t}")))
                || ["curl", "wget", "gh", "otto_mcp_token"]
                    .iter()
                    .any(|w| has_word(&p, w))
                || p.contains("git push")
                || p.contains("127.0.0.1:7700")
                || skills(n)
                    .iter()
                    .any(|s| s.contains("post") || s.contains("publish"))
        }
        k => {
            matches!(
                k,
                "post_review_comments" | "git_pr" | "channel_notify" | "http_request" | "api_run"
            ) || k.contains("publish")
        }
    }
}

fn says_previous_step(p: &str) -> bool {
    ["previous step", "prior step", "the step before"]
        .iter()
        .any(|w| p.contains(w))
}

/// The non-parallel rules, on the base graph.
fn detect(
    g: &WorkflowGraph,
    history: &[WorkflowRun],
    out: &mut Vec<Candidate>,
    skipped: &mut Vec<NotOffered>,
) {
    let next_id = next_node_id(g);
    for n in &g.nodes {
        let p = prompt(n);
        let sk = skills(n);
        let replaced = n.kind == "agent_prompt"
            && (is_checkout(&p) || is_gates(&sk, &p) || posts_review(g, n, &p));
        let mut add = |rule, effect, preselected, title: String, why: &str, edit| {
            out.push(Candidate {
                change: OptimizeChange {
                    id: change_id(rule, &n.id),
                    rule,
                    nodes: vec![n.id.clone()],
                    title,
                    why: why.to_string(),
                    effect,
                    preselected,
                    saving_ms: None,
                },
                edit,
            })
        };
        match n.kind.as_str() {
            "agent_prompt" if is_checkout(&p) => {
                let branch = ["do not look for a pr", "don't look for a pr", "dont look for a pr"]
                    .iter()
                    .any(|w| p.contains(w));
                add(
                    Rule::GitCheckout,
                    Effect::MayChangeOutput,
                    true,
                    format!("Check out with Otto instead of “{}”", n.name),
                    "The prompt is the fixed checkout recipe (detach, merge-base, repos.json); Otto does it in seconds and writes the same repos.json fields. The step file becomes a table instead of the agent's prose.",
                    Edit::Replace {
                        node: n.id.clone(),
                        kind: "git_checkout",
                        params: json!({"mode": if branch { "branch" } else { "pr" }}),
                    },
                );
            }
            "agent_prompt" if is_gates(&sk, &p) => {
                match gates_command(n) {
                    Ok(params) => add(
                        Rule::GoGatesScript,
                        Effect::SameContract,
                        true,
                        format!("Run the Go gates script directly in “{}”", n.name),
                        "The prompt only runs pr_gates.sh once per changed Go repository and replies with its table; a script step does exactly that without an agent.",
                        Edit::Replace { node: n.id.clone(), kind: "script", params },
                    ),
                    Err(reason) => skipped.push(NotOffered {
                        rule: Rule::GoGatesScript,
                        nodes: vec![n.id.clone()],
                        reason,
                    }),
                }
            }
            "agent_prompt" if posts_review(g, n, &p) => {
                let review = nearest_review(g, &n.id).unwrap_or_default();
                let intro = asks_gotchas(&p).then(|| WorkflowNode {
                    id: next_id.clone(),
                    kind: "agent_prompt".into(),
                    name: "Draft PR summary".into(),
                    x: n.x,
                    y: n.y,
                    params: json!({
                        "provider": n.params.get("provider").cloned().unwrap_or(json!("claude")),
                        "prompt": DRAFT_SUMMARY_PROMPT,
                    }),
                    retry: None,
                });
                let files: Vec<String> = io(n)
                    .reads
                    .into_iter()
                    .filter(|f| g.nodes.iter().any(|w| w.id != n.id && io(w).writes.iter().any(|x| glob_eq(x, f))))
                    .filter(|f| !f.contains('*'))
                    // The run's registries are engine data, not summary text.
                    .filter(|f| !matches!(f.as_str(), "repos.json" | "prs.json"))
                    .collect();
                let mut params = json!({"review_from": review, "summary_files": files});
                if p.contains("no info") {
                    params["info"] = json!("skip");
                }
                if let Some(i) = &intro {
                    params["summary_intro_from"] = json!(i.id);
                }
                add(
                    Rule::PostReviewComments,
                    Effect::MayChangeOutput,
                    true,
                    format!("Post the review comments with Otto in “{}”", n.name),
                    "The prompt posts the review's findings as inline PR comments; Otto posts them from the review itself, at most once each. The wording of the comments can differ.",
                    Edit::Post { node: n.id.clone(), params, intro },
                );
            }
            "agent_prompt" if single_repo_only(&p) => {
                if incoming(g, &n.id).iter().any(|e| e.condition.is_some()) {
                    skipped.push(NotOffered {
                        rule: Rule::SingleRepoSkip,
                        nodes: vec![n.id.clone()],
                        reason: "an edge into this step already has a condition".into(),
                    });
                } else {
                    add(
                        Rule::SingleRepoSkip,
                        Effect::MayChangeOutput,
                        true,
                        format!("Skip “{}” when the run has one repository", n.name),
                        "Its prompt stops with a one-line note for a single repository; the step is skipped instead, so no agent starts and no one-line step file is written.",
                        Edit::GuardEdges { node: n.id.clone(), condition: "len(run.repos) > 1" },
                    );
                }
            }
            _ => {}
        }
        if n.kind == "agent_prompt" && !replaced && n.params.get("timeout_s").is_none() {
            let p90 = durations(history, &n.id).map(|d| d.1);
            let slow = p90.is_some_and(|ms| ms > 15 * 60_000);
            if sk.iter().any(|s| s == "otto-angular-gates") || slow {
                let secs = (2 * p90.unwrap_or(0) / 1000).clamp(1_800, 36_000);
                add(
                    Rule::StepTimeout,
                    Effect::MayChangeOutput,
                    false,
                    format!("Stop “{}” after {} minutes", n.name, secs / 60),
                    "A hung step now fails instead of holding the run.",
                    Edit::SetParam {
                        node: n.id.clone(),
                        key: "timeout_s",
                        value: json!(secs),
                    },
                );
            }
        }
        if n.kind == "review_run"
            && n.params.get("scope_assessment").is_none()
            && declares_lenses(n)
        {
            add(
                Rule::ReviewDeclaredLenses,
                Effect::ChangesFindings,
                false,
                format!("Hold “{}” to its declared lenses", n.name),
                "The scope check may still drop a lens but never adds one, so the review is shorter — and its findings can change.",
                Edit::SetParam { node: n.id.clone(), key: "scope_assessment", value: json!("configured") },
            );
        }
    }
}

fn incoming<'a>(g: &'a WorkflowGraph, id: &str) -> Vec<&'a WorkflowEdge> {
    g.edges.iter().filter(|e| e.target == id).collect()
}

fn is_checkout(p: &str) -> bool {
    ["--detach", "merge-base", "repos.json", "base_branch"]
        .iter()
        .all(|w| p.contains(w))
}

fn is_gates(skills: &[String], p: &str) -> bool {
    skills.iter().any(|s| s == "otto-go-gates") && p.contains("pr_gates.sh")
}

fn declares_lenses(n: &WorkflowNode) -> bool {
    let explicit = n
        .params
        .get("reviewers")
        .and_then(Value::as_array)
        .is_some_and(|r| !r.is_empty())
        && n.params.get("mode").and_then(Value::as_str) == Some("orchestrator");
    let name = n.name.to_lowercase();
    let counted = name.match_indices(" lens").any(|(i, _)| {
        name[..i]
            .trim_end()
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_digit())
    });
    explicit || counted
}

fn single_repo_only(p: &str) -> bool {
    let head: String = p.chars().take(400).collect();
    [
        "only one repo",
        "single repo",
        "one repository",
        "single-repo",
    ]
    .iter()
    .any(|w| head.contains(w))
        && ["stop", "skip", "one line", "one-line"]
            .iter()
            .any(|w| head.contains(w))
}

fn posts_review(g: &WorkflowGraph, n: &WorkflowNode, p: &str) -> bool {
    nearest_review(g, &n.id).is_some()
        && p.contains("comment_pr")
        && p.contains("inline")
        && (p.contains("finding") || p.contains("list_findings"))
        && ![
            "no pull request",
            "no pr ",
            "there is no pr",
            "without a pr",
        ]
        .iter()
        .any(|w| p.contains(w))
}

fn asks_gotchas(p: &str) -> bool {
    p.contains("gotchas")
        && ![
            "omit the gotchas",
            "no gotchas",
            "without gotchas",
            "skip the gotchas",
        ]
        .iter()
        .any(|w| p.contains(w))
}

/// The closest `review_run` ancestor (fewest edges back).
fn nearest_review(g: &WorkflowGraph, id: &str) -> Option<String> {
    let mut seen = HashSet::new();
    let mut frontier = vec![id.to_string()];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for t in frontier {
            for e in g.edges.iter().filter(|e| e.target == t) {
                if !seen.insert(e.source.clone()) {
                    continue;
                }
                if node(g, &e.source).is_some_and(|n| n.kind == "review_run") {
                    return Some(e.source.clone());
                }
                next.push(e.source.clone());
            }
        }
        frontier = next;
    }
    None
}

/// The Go gates step as a `script` (engine.md §7.2): the gates script path
/// and the `--out` name come from the prompt and must be plain; the name
/// reaches the command only as `$OTTO_OUT` (DR-113, DR-202).
fn gates_command(n: &WorkflowNode) -> Result<Value, String> {
    let raw = n
        .params
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    let script = tokens
        .iter()
        .map(|t| {
            t.trim_matches(|c| "`\"'(),;".contains(c))
                .trim_end_matches(['.', ':'])
        })
        .find(|t| t.ends_with("pr_gates.sh"))
        .unwrap_or_default();
    let script_ok = !script.contains("..")
        && script
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/~-".contains(c));
    if !script_ok {
        return Err("the gates script path in the prompt is not a plain path".into());
    }
    let script = match script.strip_prefix("~/") {
        Some(rest) => format!("\"$HOME/{rest}\""),
        None => format!("\"{script}\""),
    };
    let after = raw.split_once("--out").map_or("", |(_, r)| r.trim_start());
    let out = match after.strip_prefix(['"', '\'', '`']) {
        Some(rest) => rest.split(['"', '\'', '`']).next().unwrap_or_default(),
        None => after.split_whitespace().next().unwrap_or_default(),
    };
    let name = out.rsplit('/').next().unwrap_or_default();
    let plain = !out.contains("..")
        && !matches!(name, "" | "." | "..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !plain {
        return Err("output file name in the prompt is not a plain file name".into());
    }
    // A gate that fails is a row of the table (the prompt's contract: keep
    // every row), so the script's exit code alone does not fail the step;
    // no fresh report does (a crash, or a retry that would cat the last one).
    let command = format!(
        "if [ -z \"$(git -C \"$OTTO_WORKTREE\" diff --name-only \"$OTTO_BASE\" HEAD -- '*.go')\" ]; then echo \"$OTTO_REPO_NAME: no Go changes — gates NOT RUN\" >> \"$OTTO_CONTEXT_DIR/$OTTO_OUT\"; exit 0; fi\n\
         f=\"$OTTO_CONTEXT_DIR/$OTTO_REPO_NAME-$OTTO_OUT\"; rm -f \"$f\"\n\
         {script} \"$OTTO_WORKTREE\" \"$OTTO_BASE\" --main \"$OTTO_REPO\" --out \"$f\"; rc=$?\n\
         [ -f \"$f\" ] || {{ echo \"$OTTO_REPO_NAME: the gates script wrote no report (exit $rc)\" >&2; exit 1; }}\n\
         cat \"$f\" >> \"$OTTO_CONTEXT_DIR/$OTTO_OUT\""
    );
    Ok(json!({
        "command": command,
        "timeout_s": 900,
        "foreach": "repos",
        "cwd": "worktree",
        "output_file": name,
        "idempotent": true,
    }))
}

fn next_node_id(g: &WorkflowGraph) -> String {
    let max = g
        .nodes
        .iter()
        .filter_map(|n| n.id.strip_prefix('n')?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("n{}", max + 1)
}

fn node<'a>(g: &'a WorkflowGraph, id: &str) -> Option<&'a WorkflowNode> {
    g.nodes.iter().find(|n| n.id == id)
}

fn node_mut<'a>(g: &'a mut WorkflowGraph, id: &str) -> Option<&'a mut WorkflowNode> {
    g.nodes.iter_mut().find(|n| n.id == id)
}

fn new_edge_id(g: &WorkflowGraph, from: &str, to: &str) -> String {
    let base = format!("e-opt-{from}-{to}");
    let mut id = base.clone();
    let mut k = 2;
    while g.edges.iter().any(|e| e.id == id) {
        id = format!("{base}-{k}");
        k += 1;
    }
    id
}

fn apply_edit(g: &mut WorkflowGraph, edit: &Edit) {
    match edit {
        Edit::Replace { node, kind, params } => {
            if let Some(n) = node_mut(g, node) {
                n.kind = kind.to_string();
                n.params = params.clone();
            }
        }
        Edit::Post {
            node,
            params,
            intro,
        } => {
            if let Some(n) = node_mut(g, node) {
                n.kind = "post_review_comments".into();
                n.params = params.clone();
            }
            if let Some(intro) = intro {
                for e in g.edges.iter_mut().filter(|e| e.target == *node) {
                    e.target = intro.id.clone();
                }
                let id = new_edge_id(g, &intro.id, node);
                let at = g
                    .nodes
                    .iter()
                    .position(|n| n.id == *node)
                    .unwrap_or(g.nodes.len());
                g.nodes.insert(at, intro.clone());
                g.edges.push(WorkflowEdge {
                    id,
                    source: intro.id.clone(),
                    target: node.clone(),
                    condition: None,
                });
            }
        }
        Edit::SetParam { node, key, value } => {
            if let Some(Value::Object(m)) = node_mut(g, node).map(|n| &mut n.params) {
                m.insert(key.to_string(), value.clone());
            }
        }
        Edit::GuardEdges { node, condition } => {
            for e in g.edges.iter_mut().filter(|e| e.target == *node) {
                e.condition = Some(condition.to_string());
            }
        }
        Edit::Parallel { node } => rewire(g, node),
    }
}

/// Re-wire `id` to depend only on the steps whose files it reads (or the
/// trigger), then give every other step back any ancestor it lost, so later
/// steps still wait for everything they waited for.
fn rewire(g: &mut WorkflowGraph, id: &str) {
    let Some(deps) = parallel_deps(g, id) else {
        return;
    };
    let before: HashMap<String, HashSet<String>> = g
        .nodes
        .iter()
        .map(|n| (n.id.clone(), ancestors(g, &n.id)))
        .collect();
    let at = g
        .edges
        .iter()
        .position(|e| e.target == id)
        .unwrap_or(g.edges.len());
    g.edges.retain(|e| e.target != id);
    let at = at.min(g.edges.len());
    for (k, d) in deps.iter().enumerate() {
        let eid = new_edge_id(g, d, id);
        g.edges.insert(
            at + k,
            WorkflowEdge {
                id: eid,
                source: d.clone(),
                target: id.to_string(),
                condition: None,
            },
        );
    }
    let order = topo_order(g).unwrap_or_default();
    for x in order.iter().filter(|x| *x != id) {
        let now = ancestors(g, x);
        let mut lost: Vec<String> = before[x]
            .iter()
            .filter(|a| !now.contains(*a))
            .cloned()
            .collect();
        // A movable step keeps only what it reads from; any other keeps all.
        if node(g, x).is_some_and(|n| movable(g, n)) {
            let reads = node(g, x).map(|n| writers_of(g, n)).unwrap_or_default();
            lost.retain(|a| reads.contains(a));
        }
        // A lost node reached from another lost one by unconditional edges
        // needs no edge of its own.
        let keep: Vec<&String> = lost
            .iter()
            .filter(|a| {
                !lost
                    .iter()
                    .any(|b| b != *a && unconditional_ancestors(g, b).contains(*a))
            })
            .collect();
        for a in order.iter().filter(|o| keep.contains(o)) {
            let eid = new_edge_id(g, a, x);
            g.edges.push(WorkflowEdge {
                id: eid,
                source: a.clone(),
                target: x.clone(),
                condition: None,
            });
        }
    }
}

/// An agent, script, checkout or Jira step with no conditional edge in, no
/// outward effect and no "previous step" wording.
fn movable(g: &WorkflowGraph, n: &WorkflowNode) -> bool {
    matches!(
        n.kind.as_str(),
        "agent_prompt" | "script" | "git_checkout" | "prepare_context"
    ) && incoming(g, &n.id).iter().all(|e| e.condition.is_none())
        && !outward(n)
        && !says_previous_step(&prompt(n))
}

/// A movable step's new predecessors, or `None` when it is not movable, is
/// an agent that names no file or step it reads, or already depends on
/// exactly those.
fn parallel_deps(g: &WorkflowGraph, id: &str) -> Option<Vec<String>> {
    let n = node(g, id)?;
    let mine = io(n);
    if !movable(g, n)
        || (n.kind == "agent_prompt" && mine.reads.is_empty() && mine.steps.is_empty())
    {
        return None;
    }
    let anc = ancestors(g, id);
    let order = topo_order(g).ok()?;
    let numbers = numbering(g, &order, &HashSet::new());
    let mut deps: Vec<String> = order
        .iter()
        .filter(|a| anc.contains(*a))
        .filter(|a| {
            let w = node(g, a).map(io).unwrap_or_default();
            mine.reads
                .iter()
                .any(|f| w.writes.iter().any(|x| glob_eq(x, f)))
                || numbers.get(*a).is_some_and(|k| mine.steps.contains(k))
        })
        .cloned()
        .collect();
    // Only the nearest writers: one that is already an ancestor of another dep goes.
    let all = deps.clone();
    deps.retain(|d| !all.iter().any(|o| o != d && ancestors(g, o).contains(d)));
    if deps.is_empty() {
        deps = order
            .iter()
            .filter(|a| anc.contains(*a) && incoming(g, a).is_empty())
            .take(1)
            .cloned()
            .collect();
    }
    let current: Vec<String> = incoming(g, id).iter().map(|e| e.source.clone()).collect();
    let same = deps.len() == current.len() && deps.iter().all(|d| current.contains(d));
    (!same && !deps.is_empty()).then_some(deps)
}

fn parallel_candidates(g: &WorkflowGraph) -> Vec<Candidate> {
    let order = topo_order(g).unwrap_or_default();
    order
        .iter()
        .filter_map(|id| {
            let deps = parallel_deps(g, id)?;
            let n = node(g, id)?;
            let names = |ids: &[String]| {
                ids.iter()
                    .map(|i| format!("“{}”", node(g, i).map_or(i.as_str(), |n| n.name.as_str())))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let was: Vec<String> = incoming(g, id).iter().map(|e| e.source.clone()).collect();
            Some(Candidate {
                change: OptimizeChange {
                    id: change_id(Rule::Parallel, id),
                    rule: Rule::Parallel,
                    nodes: vec![id.clone()],
                    title: format!("Start “{}” right after {}", n.name, names(&deps)),
                    why: format!(
                        "It reads only what {} wrote, so it no longer waits for {}; every later step still waits for everything it waited for. Its prompt lists fewer prior step summaries.",
                        names(&deps),
                        names(&was)
                    ),
                    effect: Effect::MayChangeOutput,
                    preselected: true,
                    saving_ms: None,
                },
                edit: Edit::Parallel { node: id.clone() },
            })
        })
        .collect()
}

// ---------------------------------------------------------------- build --

struct Built {
    graph: WorkflowGraph,
    offered: Vec<OptimizeChange>,
    /// Each offered change's edit, for its own saving.
    offered_edits: Vec<Edit>,
    not_offered: Vec<NotOffered>,
    /// Ids applied to `graph`, in order, with their edits.
    applied: Vec<String>,
    edits: Vec<Edit>,
}

fn build(base: &WorkflowGraph, history: &[WorkflowRun], opts: &Options<'_>) -> Built {
    let mut cands = Vec::new();
    let mut not_offered = Vec::new();
    detect(base, history, &mut cands, &mut not_offered);
    let mut b = Built {
        graph: base.clone(),
        offered: vec![],
        offered_edits: vec![],
        not_offered,
        applied: vec![],
        edits: vec![],
    };
    let wanted = |c: &OptimizeChange| opts.selection.map_or(c.preselected, |s| s.contains(&c.id));
    let add = |b: &mut Built, c: Candidate| {
        let mut t = b.graph.clone();
        apply_edit(&mut t, &c.edit);
        let mut edits = b.edits.clone();
        edits.push(c.edit.clone());
        match check(base, &t, &edits, opts) {
            Ok(()) => {
                if wanted(&c.change) {
                    b.graph = t;
                    b.edits = edits;
                    b.applied.push(c.change.id.clone());
                }
                b.offered.push(c.change);
                b.offered_edits.push(c.edit);
            }
            Err(reason) => b.not_offered.push(NotOffered {
                rule: c.change.rule,
                nodes: c.change.nodes,
                reason,
            }),
        }
    };
    for c in cands {
        add(&mut b, c);
    }
    for c in parallel_candidates(&b.graph.clone()) {
        add(&mut b, c);
    }
    b
}

// --------------------------------------------------------------- checks --

fn triggers(g: &WorkflowGraph) -> BTreeSet<String> {
    g.nodes
        .iter()
        .filter(|n| incoming(g, &n.id).is_empty())
        .map(|n| n.id.clone())
        .collect()
}

fn check(
    base: &WorkflowGraph,
    draft: &WorkflowGraph,
    edits: &[Edit],
    opts: &Options<'_>,
) -> Result<(), String> {
    if let Some(m) = (opts.validate)(draft).into_iter().next() {
        return Err(format!("the draft does not validate: {m}"));
    }
    let Ok(order) = topo_order(draft) else {
        return Err("the draft has a cycle".into());
    };
    let new_roots: Vec<String> = triggers(draft)
        .difference(&triggers(base))
        .cloned()
        .collect();
    if !new_roots.is_empty() {
        return Err(format!(
            "{} would no longer follow a trigger",
            new_roots.join(", ")
        ));
    }
    numbering_check(base, draft, &order).map_err(|(n, id, by)| {
        format!("it would give step{n} (which “{by}” refers to) to another step than “{id}”")
    })?;
    // Reads: every writer a step read from is still before it.
    for x in &draft.nodes {
        let Some(_) = node(base, &x.id) else { continue };
        let base_anc = ancestors(base, &x.id);
        let now = ancestors(draft, &x.id);
        for w in writers_of(draft, x)
            .into_iter()
            .filter(|w| base_anc.contains(w))
        {
            if !now.contains(&w) {
                return Err(format!(
                    "“{}” reads what “{}” writes but would start before it",
                    x.name,
                    name_of(draft, &w)
                ));
            }
        }
    }
    // Outward steps keep their order.
    let out: Vec<&WorkflowNode> = base.nodes.iter().filter(|n| outward(n)).collect();
    for b in &out {
        let now = ancestors(draft, &b.id);
        for a in ancestors(base, &b.id)
            .iter()
            .filter(|a| out.iter().any(|o| o.id == **a))
        {
            if !now.contains(a) {
                return Err(format!(
                    "“{}” would no longer wait for “{}”",
                    b.name,
                    name_of(base, a)
                ));
            }
        }
    }
    never_changed(base, draft, edits)
}

fn name_of(g: &WorkflowGraph, id: &str) -> String {
    node(g, id).map_or_else(|| id.to_string(), |n| n.name.clone())
}

/// Ids and names of every base node stay; kept nodes keep kind and params,
/// except what a chosen change itself sets.
fn never_changed(
    base: &WorkflowGraph,
    draft: &WorkflowGraph,
    edits: &[Edit],
) -> Result<(), String> {
    for b in &base.nodes {
        let Some(d) = node(draft, &b.id) else {
            return Err(format!("“{}” would be removed", b.name));
        };
        if d.name != b.name {
            return Err(format!("“{}” would be renamed", b.name));
        }
        let replaced = edits.iter().any(
            |e| matches!(e, Edit::Replace { node, .. } | Edit::Post { node, .. } if *node == b.id),
        );
        if replaced {
            continue;
        }
        let mut allowed = b.params.clone();
        for e in edits {
            if let (Edit::SetParam { node, key, value }, Value::Object(m)) = (e, &mut allowed) {
                if *node == b.id {
                    m.insert(key.to_string(), value.clone());
                }
            }
        }
        if d.kind != b.kind || d.params != allowed {
            return Err(format!("“{}” would change beyond this change", b.name));
        }
    }
    Ok(())
}

/// Every condition outcome to dry-run: each conditional edge (of either
/// graph) on or off; at most 64, else the first 64 plus all-on and all-off.
fn combinations(base: &WorkflowGraph, draft: &WorkflowGraph) -> Vec<HashSet<String>> {
    let mut vars: Vec<String> = Vec::new();
    for e in base
        .edges
        .iter()
        .chain(&draft.edges)
        .filter(|e| e.condition.is_some())
    {
        if !vars.contains(&e.id) {
            vars.push(e.id.clone());
        }
    }
    let total = 1u64.checked_shl(vars.len() as u32).unwrap_or(u64::MAX);
    let mut masks: Vec<u64> = (0..total.min(64)).collect();
    if total > 64 {
        masks.push(total.wrapping_sub(1));
    }
    masks
        .into_iter()
        .map(|m| {
            vars.iter()
                .enumerate()
                .filter(|(i, _)| m >> i & 1 == 1)
                .map(|(_, v)| v.clone())
                .collect()
        })
        .collect()
}

/// The serial step numbers (engine.md §3.2) with `inactive` edges not taken
/// and every step succeeding.
fn numbering(
    g: &WorkflowGraph,
    order: &[String],
    inactive: &HashSet<String>,
) -> HashMap<String, u32> {
    let mut ran: HashMap<String, u32> = HashMap::new();
    for id in order {
        let inc = incoming(g, id);
        let runs = inc.is_empty()
            || inc
                .iter()
                .any(|e| ran.contains_key(&e.source) && !inactive.contains(&e.id));
        if runs {
            let k = ran.len() as u32 + 1;
            ran.insert(id.clone(), k);
        }
    }
    ran
}

/// Step numbers the draft's prompts refer to (`step4`, `step-1`, `step 3`).
fn referenced(g: &WorkflowGraph) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for n in &g.nodes {
        for k in io(n).steps {
            out.push((k, n.id.clone()));
        }
    }
    out
}

/// `Err((N, node it had, referring step's name))` when a referenced number
/// moves to another node in some combination.
fn numbering_check(
    base: &WorkflowGraph,
    draft: &WorkflowGraph,
    order: &[String],
) -> Result<(), (u32, String, String)> {
    let base_order = topo_order(base).unwrap_or_default();
    let refs = referenced(draft);
    for inactive in combinations(base, draft) {
        let b = numbering(base, &base_order, &inactive);
        let d = numbering(draft, order, &inactive);
        for (k, by) in &refs {
            let had = b.iter().find(|(_, n)| *n == k).map(|(id, _)| id);
            let has = d.iter().find(|(_, n)| *n == k).map(|(id, _)| id);
            if let Some(had) = had {
                if has != Some(had) {
                    return Err((*k, name_of(base, had), name_of(draft, by)));
                }
            }
        }
    }
    Ok(())
}

/// The step file names of a graph with every edge taken.
fn step_files(g: &WorkflowGraph) -> HashMap<String, (u32, String)> {
    let order = topo_order(g).unwrap_or_default();
    numbering(g, &order, &HashSet::new())
        .into_iter()
        .map(|(id, k)| {
            let file = format!("step{k}-{}.md", slug(&name_of(g, &id)));
            (id, (k, file))
        })
        .collect()
}

fn report(base: &WorkflowGraph, draft: &WorkflowGraph, opts: &Options<'_>) -> Checks {
    let (bf, df) = (step_files(base), step_files(draft));
    let refs = referenced(draft);
    let order = topo_order(draft).unwrap_or_default();
    let base_order = topo_order(base).unwrap_or_default();
    let mut numbering_rows: Vec<NumberingCheck> = Vec::new();
    for (k, _) in &refs {
        if numbering_rows.iter().any(|r| r.step_no == *k) {
            continue;
        }
        let all = combinations(base, draft).into_iter().all(|inactive| {
            let b = numbering(base, &base_order, &inactive);
            let d = numbering(draft, &order, &inactive);
            let had = b.iter().find(|(_, n)| *n == k).map(|(id, _)| id);
            had.is_none() || had == d.iter().find(|(_, n)| *n == k).map(|(id, _)| id)
        });
        let id = bf
            .iter()
            .find(|(_, (n, _))| n == k)
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
        numbering_rows.push(NumberingCheck {
            step_no: *k,
            node_id: id,
            ok: all,
        });
    }
    numbering_rows.sort_by_key(|r| r.step_no);
    let mut renumbered: Vec<Renumbered> = order
        .iter()
        .filter_map(|id| {
            let (from, to) = (bf.get(id)?, df.get(id)?);
            (from.0 != to.0).then(|| Renumbered {
                node_id: id.clone(),
                from: format!("step{}", from.0),
                to: format!("step{}", to.0),
            })
        })
        .collect();
    renumbered.retain(|r| {
        !refs
            .iter()
            .any(|(k, _)| bf.get(&r.node_id).is_some_and(|f| f.0 == *k))
    });
    let files = order
        .iter()
        .filter_map(|id| {
            let d = node(draft, id)?;
            let now = io(d);
            let was = node(base, id).map(io).unwrap_or_default();
            let mut writes: Vec<String> = now.writes.iter().cloned().collect();
            writes.extend(df.get(id).map(|f| f.1.clone()));
            let listed =
                |g: &WorkflowGraph, files: &HashMap<String, (u32, String)>| -> BTreeSet<String> {
                    ancestors(g, id)
                        .iter()
                        .filter_map(|a| files.get(a).map(|f| f.1.clone()))
                        .collect()
                };
            let (lb, ld) = (listed(base, &bf), listed(draft, &df));
            let mut removed_ids: Vec<String> = ancestors(base, id)
                .difference(&ancestors(draft, id))
                .cloned()
                .collect();
            removed_ids.sort_by_key(|a| bf.get(a).map(|f| f.0));
            Some(FileCheck {
                node_id: id.clone(),
                reads: now.reads.iter().cloned().collect(),
                writes,
                reads_added: now.reads.difference(&was.reads).cloned().collect(),
                reads_lost: was.reads.difference(&now.reads).cloned().collect(),
                listing_removed: removed_ids
                    .iter()
                    .filter_map(|a| bf.get(a).map(|f| f.1.clone()))
                    .filter(|f| lb.contains(f) && !ld.contains(f))
                    .collect(),
            })
        })
        .collect();
    let mut warnings = Vec::new();
    for n in &draft.nodes {
        let fork = draft.edges.iter().filter(|e| e.source == n.id).count();
        if fork > opts.max_parallel_steps {
            warnings.push(format!(
                "{fork} steps start after “{}”; at most {} run at once",
                n.name, opts.max_parallel_steps
            ));
        }
    }
    Checks {
        numbering: numbering_rows,
        renumbered,
        files,
        warnings,
    }
}

// ------------------------------------------------------- reads / writes --

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Io {
    /// Context files (globs with `*` for placeholders).
    reads: BTreeSet<String>,
    writes: BTreeSet<String>,
    /// Step numbers named (`step4`, the trigger's `step-1`).
    steps: BTreeSet<u32>,
}

const EXTENSIONS: [&str; 8] = [
    ".md", ".json", ".txt", ".log", ".csv", ".yaml", ".yml", ".html",
];

/// A file name in prompt text, its directory dropped and placeholders
/// (`<KEY>`, `$VAR`) as `*`; `None` for step files (counted as numbers).
fn file_token(t: &str) -> Option<String> {
    let t = t.trim_end_matches(['.', ':']);
    let name = t.rsplit('/').next()?;
    if !EXTENSIONS
        .iter()
        .any(|e| name.len() > e.len() && name.ends_with(e))
    {
        return None;
    }
    let mut out = String::new();
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                }
                out.push('*');
            }
            '$' => {
                while chars.peek().is_some_and(|c| {
                    c.is_ascii_alphanumeric() || *c == '_' || *c == '{' || *c == '}'
                }) {
                    chars.next();
                }
                out.push('*');
            }
            c => out.push(c),
        }
    }
    while out.contains("**") {
        out = out.replace("**", "*");
    }
    let lower = out.to_lowercase();
    let output_json = lower.starts_with('*') && lower.ends_with(".output.json");
    let step_file = lower.starts_with("step")
        && lower[4..].starts_with(|c: char| c.is_ascii_digit() || c == 'n');
    if output_json || step_file {
        return None;
    }
    Some(out)
}

/// `step4`, `step-1`, `step 3` (DR: the addendum's `step[\s-]?(\d+)`).
fn step_refs(p: &str) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for (i, _) in p.match_indices("step") {
        if p[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        let rest = &p[i + 4..];
        let rest = rest.strip_prefix(['-', ' ']).unwrap_or(rest);
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(k) = digits.parse() {
            out.insert(k);
        }
    }
    out
}

fn io(n: &WorkflowNode) -> Io {
    let mut io = Io::default();
    let set = |v: &mut BTreeSet<String>, f: &str| {
        v.insert(f.to_string());
    };
    match n.kind.as_str() {
        "agent_prompt" => {
            let p = prompt(n);
            io.steps = step_refs(&p);
            let words: Vec<&str> = p
                .split(|c: char| c.is_whitespace() || "()[]{},;|".contains(c))
                .filter(|w| !w.is_empty())
                .collect();
            for (i, w) in words.iter().enumerate() {
                let Some(f) = file_token(w) else { continue };
                let before = words[i.saturating_sub(8)..i].join(" ");
                let to_after_write = (before.contains("write") || before.contains("save"))
                    && i > 0
                    && matches!(words[i - 1], "to" | "into");
                if words[i.saturating_sub(4)..i].contains(&"--out") || to_after_write {
                    set(&mut io.writes, &f);
                } else {
                    set(&mut io.reads, &f);
                }
            }
            if [
                "update repos.json",
                "rewrite repos.json",
                "write repos.json",
                "edit repos.json",
                "updates repos.json",
                "rewrites repos.json",
            ]
            .iter()
            .any(|w| p.contains(w))
            {
                set(&mut io.writes, "repos.json");
            }
            if p.contains("step-1") || p.contains("step1") {
                io.steps.insert(1);
            }
        }
        "prepare_context" => set(&mut io.writes, "jira-*.md"),
        "git_checkout" => {
            for f in ["repos.json", "prs.json"] {
                set(&mut io.reads, f);
                set(&mut io.writes, f);
            }
        }
        "script" => {
            if let Some(f) = n.params.get("output_file").and_then(Value::as_str) {
                set(&mut io.writes, f);
            }
            if n.params.get("foreach").and_then(Value::as_str) == Some("repos") {
                set(&mut io.reads, "repos.json");
            }
        }
        "post_review_comments" => {
            for f in n
                .params
                .get("summary_files")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                set(&mut io.reads, f);
            }
            set(&mut io.reads, "prs.json");
        }
        _ => {}
    }
    io
}

/// `*`-glob match either way (a reader's `jira-*.md` meets a writer's).
fn glob_eq(a: &str, b: &str) -> bool {
    fn m(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => m(&p[1..], s) || (!s.is_empty() && m(p, &s[1..])),
            (Some(x), Some(y)) if x.eq_ignore_ascii_case(y) => m(&p[1..], &s[1..]),
            _ => false,
        }
    }
    m(a.as_bytes(), b.as_bytes()) || m(b.as_bytes(), a.as_bytes())
}

/// The nodes `x` reads from: writers of its files and the steps it names.
fn writers_of(g: &WorkflowGraph, x: &WorkflowNode) -> BTreeSet<String> {
    let mine = io(x);
    let files = step_files(g);
    g.nodes
        .iter()
        .filter(|w| w.id != x.id)
        .filter(|w| {
            let theirs = io(w);
            mine.reads
                .iter()
                .any(|f| theirs.writes.iter().any(|t| glob_eq(t, f)))
                || files.get(&w.id).is_some_and(|f| mine.steps.contains(&f.0))
        })
        .map(|w| w.id.clone())
        .collect()
}

// ---------------------------------------------------------------- graph --
// The run order and ancestors are the engine's (`crate::graph`).

fn unconditional_ancestors(g: &WorkflowGraph, id: &str) -> HashSet<String> {
    walk_back(g, id, |e| e.condition.is_none())
}

fn walk_back(
    g: &WorkflowGraph,
    id: &str,
    follow: impl Fn(&WorkflowEdge) -> bool,
) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut stack = vec![id.to_string()];
    while let Some(t) = stack.pop() {
        for e in g.edges.iter().filter(|e| e.target == t && follow(e)) {
            if seen.insert(e.source.clone()) {
                stack.push(e.source.clone());
            }
        }
    }
    seen.remove(id);
    seen
}

// ------------------------------------------------------------- estimate --

struct Estimated {
    estimate: Estimate,
    savings: HashMap<String, u64>,
}

/// A step's working time in one run: its duration minus time spent queued or
/// waiting for the machine (runs from before phases count whole).
fn working_ms(s: &otto_core::workflows::NodeRunState) -> Option<(u64, bool)> {
    let total = s.duration_ms.filter(|_| s.status == NodeStatus::Success)?;
    if s.phases.is_empty() {
        return Some((total, false));
    }
    let parked: i64 = s
        .phases
        .windows(2)
        .filter(|w| matches!(w[0].phase, StepPhase::Queued | StepPhase::Waiting))
        .map(|w| (w[1].at - w[0].at).num_milliseconds().max(0))
        .sum();
    Some((total.saturating_sub(parked as u64), true))
}

fn runs_used(history: &[WorkflowRun]) -> Vec<&WorkflowRun> {
    let mut runs: Vec<&WorkflowRun> = history
        .iter()
        .filter(|r| r.status == RunStatus::Success)
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    runs.truncate(50);
    runs
}

fn median_p90(mut v: Vec<u64>) -> Option<(u64, u64)> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let at = |q: usize| v[((v.len() * q).div_ceil(100)).clamp(1, v.len()) - 1];
    Some((at(50), at(90)))
}

/// Median and P90 working time of `id` over the last 50 successful runs.
fn durations(history: &[WorkflowRun], id: &str) -> Option<(u64, u64)> {
    median_p90(
        runs_used(history)
            .iter()
            .filter_map(|r| {
                r.nodes
                    .iter()
                    .find(|s| s.node_id == id)
                    .and_then(working_ms)
            })
            .map(|(ms, _)| ms)
            .collect(),
    )
}

/// Per node: (median, P90) in `g`, replaced and guarded steps by estimate.
fn node_times(
    g: &WorkflowGraph,
    edits: &[Edit],
    history: &[WorkflowRun],
) -> HashMap<String, (u64, u64)> {
    let runs = runs_used(history);
    g.nodes
        .iter()
        .map(|n| {
            let mut t = durations(history, &n.id).unwrap_or((0, 0));
            for e in edits {
                match e {
                    Edit::Replace { node, kind, .. } if *node == n.id => {
                        t = match *kind {
                            "git_checkout" => (15_000, 15_000),
                            _ => (
                                t.0.saturating_sub(30_000).max(10_000),
                                t.1.saturating_sub(30_000).max(10_000),
                            ),
                        }
                    }
                    Edit::Post { node, .. } if *node == n.id => t = (15_000, 15_000),
                    Edit::Post { intro: Some(i), .. } if i.id == n.id => t = (45_000, 45_000),
                    Edit::GuardEdges { node, .. } if *node == n.id => {
                        let per_run: Vec<u64> = runs
                            .iter()
                            .filter_map(|r| {
                                let ms = r
                                    .nodes
                                    .iter()
                                    .find(|s| s.node_id == n.id)
                                    .and_then(working_ms)?
                                    .0;
                                let repos = r
                                    .input
                                    .get("repos")
                                    .and_then(Value::as_array)
                                    .map_or(1, Vec::len);
                                Some(if repos > 1 { ms } else { 0 })
                            })
                            .collect();
                        t = median_p90(per_run).unwrap_or((0, 0));
                    }
                    _ => {}
                }
            }
            (n.id.clone(), t)
        })
        .collect()
}

/// Whether a kind holds one of the run's step slots (engine.md §3.4).
fn takes_slot(kind: &str) -> bool {
    matches!(
        kind,
        "agent_prompt"
            | "prepare_context"
            | "review_run"
            | "script"
            | "git_checkout"
            | "post_review_comments"
            | "loop"
            | "check"
            | "canvas"
            | "git_pr"
    ) || kind.starts_with("product_")
}

/// Wall clock of a run under the start rule (engine.md §3.2): walk the run
/// order, stop at the first step whose predecessors have not all settled or
/// that finds every step slot taken.
fn makespan(g: &WorkflowGraph, times: &HashMap<String, u64>, slots: usize) -> u64 {
    let order = topo_order(g).unwrap_or_default();
    let mut ended: HashMap<String, u64> = HashMap::new();
    let mut running: Vec<(String, u64)> = Vec::new();
    let (mut now, mut next) = (0u64, 0usize);
    loop {
        while next < order.len() {
            let id = &order[next];
            if !incoming(g, id)
                .iter()
                .all(|e| ended.get(&e.source).is_some_and(|t| *t <= now))
            {
                break;
            }
            let kind = node(g, id).map_or("", |n| n.kind.as_str());
            if takes_slot(kind)
                && running
                    .iter()
                    .filter(|(r, _)| takes_slot(node(g, r).map_or("", |n| n.kind.as_str())))
                    .count()
                    >= slots.max(1)
            {
                break;
            }
            running.push((id.clone(), now + times.get(id).copied().unwrap_or(0)));
            next += 1;
        }
        let Some(&(_, t)) = running.iter().min_by_key(|(_, t)| *t) else {
            return now;
        };
        now = t;
        running.retain(|(id, end)| {
            let done = *end <= now;
            if done {
                ended.insert(id.clone(), *end);
            }
            !done
        });
    }
}

fn estimate(
    base: &WorkflowGraph,
    built: &Built,
    history: &[WorkflowRun],
    slots: usize,
) -> Estimated {
    let runs = runs_used(history);
    let mut e = Estimate {
        runs: runs.len(),
        since: runs.iter().map(|r| r.started_at).min(),
        ..Estimate::default()
    };
    if runs.len() < 3 {
        e.note = Some("Not enough runs to estimate yet".into());
        return Estimated {
            estimate: e,
            savings: HashMap::new(),
        };
    }
    let span = |g: &WorkflowGraph, edits: &[Edit], slow: bool| {
        let t = node_times(g, edits, history);
        let pick: HashMap<String, u64> = t
            .into_iter()
            .map(|(k, (m, p))| (k, if slow { p } else { m }))
            .collect();
        makespan(g, &pick, slots)
    };
    e.typical_before_ms = Some(span(base, &[], false));
    e.slow_before_ms = Some(span(base, &[], true));
    e.typical_after_ms = Some(span(&built.graph, &built.edits, false));
    e.slow_after_ms = Some(span(&built.graph, &built.edits, true));
    let older = runs.iter().any(|r| {
        r.nodes
            .iter()
            .any(|s| s.duration_ms.is_some() && s.phases.is_empty())
    });
    let mut notes = vec!["replaced steps' times are estimates until they have run".to_string()];
    if older {
        notes.insert(
            0,
            "older runs include machine waits that the new engine no longer has".into(),
        );
    }
    e.note = Some(notes.join("; "));
    let before = e.typical_before_ms.unwrap_or(0);
    let mut savings = HashMap::new();
    for (c, edit) in built.offered.iter().zip(&built.offered_edits) {
        let mut g = base.clone();
        apply_edit(&mut g, edit);
        let saving = before.saturating_sub(span(&g, std::slice::from_ref(edit), false));
        savings.insert(c.id.clone(), saving);
    }
    Estimated {
        estimate: e,
        savings,
    }
}

#[cfg(test)]
#[path = "optimize_tests.rs"]
mod tests;
