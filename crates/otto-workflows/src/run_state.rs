//! The run's mutable bookkeeping (`RunState`) and the pure halves of run
//! admission (the run scope, the seeded input, the run brief), moved out of
//! otto-server's `workflow_engine/run/{state,admission}.rs`. Admission builds
//! the state, the scheduler mutates it as nodes settle, and finalize consumes
//! it — one owner, threaded through.

use crate::context::{render_brief, RunContextFiles};
use otto_core::workflows::{NodeRunState, NodeStatus, Workflow, WorkflowGraph};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Everything a run accumulates while its nodes execute. Moved out of
/// `run_workflow_with_budget`'s locals so admission, the scheduler and
/// finalize share one value instead of a dozen loose `let mut`s.
pub struct RunState {
    /// node_id -> output once it has run.
    pub outputs: HashMap<String, Value>,
    /// node_id -> resolved state for persistence (graph order).
    pub states: Vec<NodeRunState>,
    /// Nodes that errored (or were poisoned by an errored upstream) — these
    /// propagate failure. A node pruned by an edge condition (or downstream of
    /// a pruned one) is only marked Skipped: it does NOT fail the run, and
    /// nothing downstream needs a set of them.
    pub errored: HashSet<String>,
    /// Edge ids whose condition evaluated false (the branch was not taken).
    pub inactive_edges: HashSet<String>,
    /// The run was canceled (before it started or mid-node).
    pub canceled: bool,
    /// The run's active time budget ran out.
    pub timed_out: bool,
}

impl RunState {
    /// Every node Pending, nothing run yet.
    pub fn new(graph: &WorkflowGraph) -> Self {
        let states = graph
            .nodes
            .iter()
            .map(|node| NodeRunState {
                node_id: node.id.clone(),
                ..Default::default() // every node Pending
            })
            .collect();
        Self {
            outputs: HashMap::new(),
            states,
            errored: HashSet::new(),
            inactive_edges: HashSet::new(),
            canceled: false,
            timed_out: false,
        }
    }

    /// Retry re-entry: adopt the prior run's states/outputs for everything the
    /// run scope excludes. In-scope nodes start fresh (Pending). Adopted Error
    /// states seed `errored` so downstream decisions still see the poison.
    pub fn adopt_prior(
        &mut self,
        prior_nodes: Option<&[NodeRunState]>,
        run_set: Option<&HashSet<String>>,
        continue_unfinished: bool,
    ) {
        let Some(prior) = prior_nodes else {
            return;
        };
        for ps in prior {
            let out_of_scope = run_set.is_some_and(|s| !s.contains(&ps.node_id));
            let settled = matches!(
                ps.status,
                NodeStatus::Success | NodeStatus::Skipped | NodeStatus::Error
            );
            // A step a restart caught mid-flight went back to pending with its
            // thread, sessions and number kept (DR-006): it re-enters from them.
            let interrupted = ps.status == NodeStatus::Pending && ps.started_at.is_some();
            if !(out_of_scope || continue_unfinished && (settled || interrupted)) {
                continue;
            }
            if let Some(idx) = self.states.iter().position(|s| s.node_id == ps.node_id) {
                // A step that succeeded with a JSON null output (a Start of a
                // run with no input) is stored without one; it still produced
                // it — without it the next step is "skipped (branch not taken)".
                if ps.output.is_some() || ps.status == NodeStatus::Success {
                    let output = ps.output.clone().unwrap_or(Value::Null);
                    self.outputs.insert(ps.node_id.clone(), output);
                }
                if ps.status == NodeStatus::Error {
                    self.errored.insert(ps.node_id.clone());
                }
                self.states[idx] = ps.clone();
            }
        }
    }

    /// Reconstruct adopted branch decisions without rerunning their nodes
    /// (`crate::graph::reconstruct_branches`, with today's run data).
    pub fn reconstruct_branches(
        &mut self,
        order: &[String],
        graph: &WorkflowGraph,
        files: &RunContextFiles,
        input: &Value,
    ) {
        crate::graph::reconstruct_branches(
            order,
            graph,
            &run_data(files, input),
            &self.states,
            &self.outputs,
            &mut self.inactive_edges,
            &mut self.errored,
        );
    }
}

/// The `run` object edge conditions and `condition` nodes see (DR-005):
/// the run input, the current `repos.json` registry and `prs.json`. Read
/// when the condition is evaluated (its source settles).
pub fn run_data(files: &RunContextFiles, input: &Value) -> Value {
    let prs: Vec<crate::run_refs::RunPr> = files
        .read_named(crate::run_refs::PRS_FILE)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    crate::graph::run_data(input, &files.repos(), &prs)
}

/// The set of nodes to actually execute (start-from-here / run-only); `None`
/// means the whole graph. Nodes outside the set are marked skipped. An
/// `adopt_start` scope (restart resume whose entry step provably finished)
/// treats the entry itself as done — it drops OUT of the set so the adoption
/// pass keeps its prior Success state/output while its descendants run.
pub fn scope_set(
    graph: &WorkflowGraph,
    start_node: Option<&str>,
    only_node: bool,
    adopt_start: bool,
) -> Option<HashSet<String>> {
    let s = start_node?;
    let mut set = if only_node {
        std::iter::once(s.to_string()).collect()
    } else {
        crate::graph::descendants_inclusive(graph, s)
    };
    if adopt_start {
        // The entry step is already proven done: only its descendants run
        // (a single-step scope runs nothing and finalizes over adopted states).
        set.remove(s);
    }
    Some(set)
}

/// Seed the run input from the entry manual_trigger node's configured fields
/// (its inspector — prompt/working_directory/repo_id/goals/…), letting the
/// actual /run body or chat-trigger input override per key. This is what makes
/// the Start node the place to set the input for a manual run.
pub fn seed_from_manual_trigger(workflow: &Workflow, input: Value) -> Value {
    let mut seeded = serde_json::Map::new();
    if let Some(mt) = workflow
        .graph
        .nodes
        .iter()
        .find(|n| n.kind == "manual_trigger")
    {
        // Strip reserved trigger/routing keys from manual_trigger params so a
        // saved graph cannot forge `origin_workspace_id`, `channel`, `chat`,
        // `thread`, etc.
        let mut params = mt.params.clone();
        crate::triggers::strip_reserved_input(&mut params, crate::triggers::InputSource::Manual);
        if let Some(o) = params.as_object() {
            for (k, v) in o {
                if !v.is_null() {
                    seeded.insert(k.clone(), v.clone());
                }
            }
        }
    }
    if let Some(o) = input.as_object() {
        for (k, v) in o {
            seeded.insert(k.clone(), v.clone());
        }
    }
    if seeded.is_empty() {
        input
    } else {
        Value::Object(seeded)
    }
}

/// Write the run's mission brief (`run-brief.md`): mission, repos table,
/// planned steps (IN-SCOPE only, so numbering matches what actually executes
/// on a start-from-here run) and the step-file protocol.
#[allow(clippy::too_many_arguments)]
pub fn write_brief(
    workflow: &Workflow,
    run_id: &str,
    order: &[String],
    run_set: Option<&HashSet<String>>,
    input: &Value,
    has_instructions: bool,
    has_prompt: bool,
    files: &RunContextFiles,
) {
    let planned: Vec<(String, String)> = order
        .iter()
        .filter(|nid| run_set.is_none_or(|s| s.contains(*nid)))
        .filter_map(|nid| workflow.graph.nodes.iter().find(|n| &n.id == nid))
        .map(|n| {
            let name = if n.name.is_empty() { &n.kind } else { &n.name };
            (name.clone(), n.kind.clone())
        })
        .collect();
    let mut brief = render_brief(
        &workflow.name,
        &workflow.description,
        run_id,
        input,
        &files.repos(),
        &planned,
        has_instructions,
        has_prompt,
    );
    // The requester's files (DR-114), copied in before the run starts.
    let refs = files
        .dir_str()
        .map(|d| crate::attachments::read(std::path::Path::new(&d)))
        .unwrap_or_default();
    if !refs.is_empty() {
        brief.push_str("\n## Attachments\n\n");
        for line in crate::attachments::brief_lines(&refs) {
            brief.push_str(&format!("- {line}\n"));
        }
    }
    files.write_brief(&brief);
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::workflows::{WorkflowEdge, WorkflowNode};
    use serde_json::json;

    fn node(id: &str, kind: &str, params: Value) -> WorkflowNode {
        WorkflowNode {
            id: id.into(),
            kind: kind.into(),
            name: String::new(),
            x: 0.0,
            y: 0.0,
            params,
            retry: None,
        }
    }

    fn workflow(nodes: Vec<WorkflowNode>, edges: Vec<WorkflowEdge>) -> Workflow {
        Workflow {
            id: "wf-1".into(),
            workspace_id: "ws-1".into(),
            name: "test".into(),
            description: String::new(),
            instructions: String::new(),
            graph: WorkflowGraph { nodes, edges },
            created_by: "u-1".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            version: 1,
            on_restart: "resume".into(),
            origin: None,
            origin_ref: None,
        }
    }

    #[test]
    fn seed_from_manual_trigger_strips_reserved_routing_keys() {
        let wf = workflow(
            vec![node(
                "start",
                "manual_trigger",
                json!({
                    "prompt": "hello",
                    "origin_workspace_id": "ws-victim",
                    "channel": "slack",
                    "chat": "C-VICTIM",
                    "thread": "123.45",
                }),
            )],
            vec![],
        );
        let merged = seed_from_manual_trigger(&wf, json!({ "prompt": "override" }));
        assert_eq!(merged["prompt"], "override");
        assert!(merged.get("origin_workspace_id").is_none());
        assert!(merged.get("channel").is_none());
        assert!(merged.get("chat").is_none());
        assert!(merged.get("thread").is_none());
    }

    #[test]
    fn the_scope_is_the_start_and_its_descendants_minus_an_adopted_start() {
        let edge = |id: &str, s: &str, t: &str| WorkflowEdge {
            id: id.into(),
            source: s.into(),
            target: t.into(),
            condition: None,
        };
        let wf = workflow(
            vec![
                node("a", "x", json!({})),
                node("b", "x", json!({})),
                node("c", "x", json!({})),
            ],
            vec![edge("e1", "a", "b"), edge("e2", "b", "c")],
        );
        let g = &wf.graph;
        let names = |s: Option<HashSet<String>>| {
            let mut v: Vec<String> = s.unwrap().into_iter().collect();
            v.sort();
            v
        };
        assert!(scope_set(g, None, false, false).is_none());
        assert_eq!(names(scope_set(g, Some("b"), false, false)), ["b", "c"]);
        assert_eq!(names(scope_set(g, Some("b"), false, true)), ["c"]);
        assert_eq!(names(scope_set(g, Some("b"), true, false)), ["b"]);
        assert!(names(scope_set(g, Some("b"), true, true)).is_empty());
    }

    /// CR1-014: the run brief names the requester's files for the steps.
    #[test]
    fn the_brief_names_the_attachments() {
        use otto_core::workflows::{AttachmentRef, AttachmentSource};
        let dir = tempfile::tempdir().unwrap();
        let files = RunContextFiles::create(dir.path(), "run-1");
        let ctx_dir = dir.path().join("workflow-context/run-1");
        let wf = workflow(vec![node("a", "log", json!({}))], vec![]);
        let brief = || {
            write_brief(
                &wf,
                "run-1",
                &["a".into()],
                None,
                &json!({}),
                false,
                false,
                &files,
            );
            std::fs::read_to_string(ctx_dir.join(crate::context::RUN_BRIEF_FILE)).unwrap()
        };
        assert!(!brief().contains("## Attachments"), "none, no section");
        let refs = [
            AttachmentRef {
                name: "spec.pdf".into(),
                mime: "application/pdf".into(),
                size_bytes: Some(2048),
                path: Some("attachments/spec.pdf".into()),
                error: None,
                source: AttachmentSource::Start,
            },
            AttachmentRef {
                name: "big.zip".into(),
                mime: "application/zip".into(),
                size_bytes: None,
                path: None,
                error: Some("too large".into()),
                source: AttachmentSource::Start,
            },
        ];
        let json = serde_json::to_vec(&refs).unwrap();
        std::fs::write(ctx_dir.join(crate::attachments::ATTACHMENTS_FILE), json).unwrap();
        let md = brief();
        assert!(md.contains("## Attachments"), "{md}");
        assert!(
            md.contains("- Attachment from the requester: attachments/spec.pdf"),
            "{md}"
        );
        assert!(
            md.contains("- Attachment not received: big.zip — too large"),
            "{md}"
        );
    }

    #[test]
    fn a_retry_adopts_out_of_scope_states_and_their_poison() {
        let wf = workflow(
            vec![node("a", "x", json!({})), node("b", "x", json!({}))],
            vec![],
        );
        let prior = vec![
            NodeRunState {
                node_id: "a".into(),
                status: NodeStatus::Error,
                output: Some(json!({"x": 1})),
                ..Default::default()
            },
            NodeRunState {
                node_id: "b".into(),
                status: NodeStatus::Success,
                ..Default::default()
            },
        ];
        let mut rs = RunState::new(&wf.graph);
        let scope: HashSet<String> = std::iter::once("b".to_string()).collect();
        rs.adopt_prior(Some(&prior), Some(&scope), false);
        assert_eq!(rs.states[0].status, NodeStatus::Error);
        assert_eq!(rs.outputs["a"], json!({"x": 1}));
        assert!(rs.errored.contains("a"));
        assert_eq!(
            rs.states[1].status,
            NodeStatus::Pending,
            "in scope starts fresh"
        );
    }
}
