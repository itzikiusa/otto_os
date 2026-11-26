//! The run's mutable bookkeeping (`RunState`) and the node loop's control
//! signal (`NodeFlow`). Admission builds the state, the scheduler mutates it
//! node by node, and finalize consumes it — one owner, threaded through.

use super::super::*;

/// Everything a run accumulates while its nodes execute. Moved out of
/// `run_workflow_with_budget`'s locals so admission, the scheduler and
/// finalize share one value instead of a dozen loose `let mut`s.
pub(in crate::workflow_engine) struct RunState {
    /// node_id -> output once it has run.
    pub(in crate::workflow_engine) outputs: HashMap<String, Value>,
    /// node_id -> resolved state for persistence (graph order).
    pub(in crate::workflow_engine) states: Vec<NodeRunState>,
    /// 1-based number of the NEXT executed step — drives step-file naming.
    /// Skipped nodes don't consume a number; adopted ones do (their files are
    /// re-linked so the trail stays complete).
    pub(in crate::workflow_engine) step_counter: usize,
    /// Nodes that errored (or were poisoned by an errored upstream) — these
    /// propagate failure. A node pruned by an edge condition (or downstream of
    /// a pruned one) is only marked Skipped: it does NOT fail the run, and
    /// nothing downstream needs a set of them.
    pub(in crate::workflow_engine) errored: HashSet<String>,
    /// Edge ids whose condition evaluated false (the branch was not taken).
    pub(in crate::workflow_engine) inactive_edges: HashSet<String>,
    /// The run was canceled (before it started or mid-node).
    pub(in crate::workflow_engine) canceled: bool,
    /// The run's active time budget ran out.
    pub(in crate::workflow_engine) timed_out: bool,
}

/// What the node loop does after one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workflow_engine) enum NodeFlow {
    /// Go on to the next node in topological order.
    Next,
    /// Leave the loop for finalize (canceled or out of time budget).
    Stop,
}

impl RunState {
    /// Every node Pending, nothing run yet.
    pub(in crate::workflow_engine) fn new(graph: &WorkflowGraph) -> Self {
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
            step_counter: 0,
            errored: HashSet::new(),
            inactive_edges: HashSet::new(),
            canceled: false,
            timed_out: false,
        }
    }

    /// Retry re-entry: adopt the prior run's states/outputs for everything the
    /// run scope excludes. In-scope nodes start fresh (Pending). Adopted Error
    /// states seed `errored` so downstream decisions still see the poison.
    pub(in crate::workflow_engine) fn adopt_prior(
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
            if !(out_of_scope || continue_unfinished && settled) {
                continue;
            }
            if let Some(idx) = self.states.iter().position(|s| s.node_id == ps.node_id) {
                if let Some(o) = &ps.output {
                    self.outputs.insert(ps.node_id.clone(), o.clone());
                }
                if ps.status == NodeStatus::Error {
                    self.errored.insert(ps.node_id.clone());
                }
                self.states[idx] = ps.clone();
            }
        }
    }

    /// Reconstruct adopted branch decisions without rerunning their nodes
    /// (`otto_workflows::graph::reconstruct_branches`, with today's run data).
    pub(in crate::workflow_engine) fn reconstruct_branches(
        &mut self,
        order: &[String],
        graph: &WorkflowGraph,
        files: &crate::workflow_context::RunContextFiles,
        input: &Value,
    ) {
        otto_workflows::graph::reconstruct_branches(
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
pub(in crate::workflow_engine) fn run_data(
    files: &crate::workflow_context::RunContextFiles,
    input: &Value,
) -> Value {
    let prs: Vec<otto_workflows::run_refs::RunPr> = files
        .read_named(otto_workflows::run_refs::PRS_FILE)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    otto_workflows::graph::run_data(input, &files.repos(), &prs)
}
