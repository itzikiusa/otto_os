//! The scheduler's decision core: which nodes of a run start, in which
//! order, with which step number — as a pure state machine the engine drives
//! (engine.md §3). The engine owns outputs, files and node futures; this owns
//! the walk over run order.
//!
//! **The start rule.** The walk goes over `order` (`graph::topo_order`) from
//! the first undecided node. It stops at a node whose relevant predecessors
//! have not all settled; adopted, out-of-scope and skipped nodes are decided
//! at once and the walk goes on; a node that runs takes the next step number
//! now. A node that needs a step slot when all slots are taken is numbered,
//! held ([`Start::Queue`]) and the walk stops until a slot frees.
//!
//! Because a node is decided only after its predecessors settled and nodes
//! are decided strictly in run order, every node gets the decision and the
//! step number the serial engine gives it, whatever the finish order — the
//! property test below checks it on random graphs, and the clone test on the
//! five exported graphs against the Phase-0 goldens.
//!
//! **Adoption (DR-013).** A settled node adopted on a retry or a restart
//! consumes its persisted `step_no`; an old row without one consumes a
//! number when it is Success or Error or a Skipped node with `started_at`
//! (executed, then skipped through chat). Branch- and upstream-skipped nodes
//! never started, so they take none.

use std::collections::HashSet;

use otto_core::workflows::{NodeRunState, NodeStatus, WorkflowGraph, WorkflowNode};

use crate::graph::{decide_node, dependency_is_relevant, outgoing_edges, EdgeView, NodeDecision};

/// Default per-run step slots (`wf_max_parallel_steps`).
pub const DEFAULT_MAX_PARALLEL_STEPS: usize = 4;
/// Range of `wf_max_parallel_steps`; 1 runs one slotted step at a time.
pub const MAX_PARALLEL_STEPS_RANGE: std::ops::RangeInclusive<usize> = 1..=8;

/// Whether a node holds a process or an agent while it runs and therefore
/// takes one of the run's step slots (engine.md §3.4). Pure kinds run without.
pub fn takes_step_slot(node: &WorkflowNode) -> bool {
    match node.kind.as_str() {
        "agent_prompt"
        | "review_run"
        | "script"
        | "git_checkout"
        | "post_review_comments"
        | "loop"
        | "check"
        | "canvas"
        | "git_pr" => true,
        "prepare_context" => node
            .params
            .get("prompt")
            .and_then(|v| v.as_str())
            .is_some_and(|p| !p.trim().is_empty()),
        kind => kind.starts_with("product_"),
    }
}

/// Why a node was skipped without running; [`SkipWhy::log_line`] is the
/// node's one log line (today's wording).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipWhy {
    /// Outside the run scope (start-from-here / retry one step).
    OutsideScope,
    /// An active-path predecessor errored; the node is poisoned too.
    Upstream,
    /// No incoming edge was taken.
    Branch,
}

impl SkipWhy {
    pub fn log_line(self) -> &'static str {
        match self {
            SkipWhy::OutsideScope => "outside run scope",
            SkipWhy::Upstream => "skipped (upstream did not succeed)",
            SkipWhy::Branch => "skipped (branch not taken)",
        }
    }
}

/// One decision of a walk, in run order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// An adopted prior state (retry / restart): keep it as it is. `step_no`
    /// is the number it consumed (re-link `step<N>-…` when it succeeded).
    Adopt { idx: usize, step_no: Option<u32> },
    /// Mark the node Skipped with `why.log_line()`.
    Skip { idx: usize, why: SkipWhy },
    /// Numbered, but every step slot is taken: the node is `running` in
    /// phase `queued` ("waiting for a step slot (4 of 4 in use)"); a later
    /// [`Start::Run`] with the same `idx` starts it.
    Queue {
        idx: usize,
        step_no: u32,
        sources: Vec<String>,
    },
    /// Start the node now with input from `sources` (`graph::assemble_input`).
    Run {
        idx: usize,
        step_no: u32,
        sources: Vec<String>,
    },
}

/// How a node that ran ended, as far as the decisions of later nodes care.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Failed (an error, or an output carrying an outcome error).
    pub errored: bool,
    /// Produced an output (incl. a chat skip's marker output).
    pub has_output: bool,
    /// Out-edges not taken (`graph::eval_outgoing`; a chat skip: its
    /// conditional out-edges).
    pub inactive_edges: Vec<String>,
}

/// What the run starts from: the node states after adoption
/// (`adopt_prior`), what of them produced an output or is poisoned, the
/// untaken edges (`graph::reconstruct_branches`) and the run scope.
pub struct Init<'a> {
    /// Indexed like `graph.nodes`.
    pub states: &'a [NodeRunState],
    pub has_output: &'a HashSet<String>,
    pub errored: &'a HashSet<String>,
    pub inactive_edges: &'a HashSet<String>,
    pub run_set: Option<&'a HashSet<String>>,
    /// The prior run's node states (retry / restart), for edge relevance.
    pub prior: Option<&'a [NodeRunState]>,
    pub continue_unfinished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Undecided,
    Running,
    Settled,
}

#[derive(Debug, Clone)]
struct Held {
    idx: usize,
    step_no: u32,
    sources: Vec<String>,
}

/// The decision core of one run. Indices are positions in `graph.nodes`.
#[derive(Debug, Clone)]
pub struct Scheduler {
    ids: Vec<String>,
    /// Graph indices in run order.
    order: Vec<usize>,
    /// Relevant incoming edges per node: (source idx, edge id).
    preds: Vec<Vec<(usize, String)>>,
    slotted: Vec<bool>,
    slot: Vec<Slot>,
    errored: Vec<bool>,
    has_output: Vec<bool>,
    inactive: HashSet<String>,
    /// Adoption per node (decided before the walk): `Some` = adopted.
    adopt: Vec<Option<Consume>>,
    outside: Vec<bool>,
    cursor: usize,
    step_counter: u32,
    cap: usize,
    in_use: usize,
    held: Option<Held>,
}

impl Scheduler {
    /// `order` is `graph::topo_order(graph)`; `max_parallel_steps` is
    /// clamped to [`MAX_PARALLEL_STEPS_RANGE`].
    pub fn new(
        graph: &WorkflowGraph,
        order: &[String],
        init: Init<'_>,
        max_parallel_steps: usize,
    ) -> Self {
        let ids: Vec<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
        let pos = |id: &str| ids.iter().position(|x| x == id);
        let preds = ids
            .iter()
            .map(|id| {
                graph
                    .edges
                    .iter()
                    .filter(|e| &e.target == id)
                    .filter(|e| dependency_is_relevant(&e.source, init.run_set, init.prior))
                    .filter_map(|e| pos(&e.source).map(|s| (s, e.id.clone())))
                    .collect()
            })
            .collect();
        let state = |id: &str| init.states.iter().find(|s| s.node_id == id);
        let outside: Vec<bool> = ids
            .iter()
            .map(|id| init.run_set.is_some_and(|set| !set.contains(id)))
            .collect();
        let adopt = ids
            .iter()
            .zip(&outside)
            .map(|(id, out)| {
                state(id)
                    .filter(|st| {
                        st.status != NodeStatus::Pending && (init.continue_unfinished || *out)
                    })
                    .map(consumes_number)
            })
            .collect();
        Self {
            order: order.iter().filter_map(|id| pos(id)).collect(),
            preds,
            slotted: graph.nodes.iter().map(takes_step_slot).collect(),
            slot: vec![Slot::Undecided; ids.len()],
            errored: ids.iter().map(|id| init.errored.contains(id)).collect(),
            has_output: ids.iter().map(|id| init.has_output.contains(id)).collect(),
            inactive: init.inactive_edges.clone(),
            adopt,
            outside,
            ids,
            cursor: 0,
            step_counter: 0,
            cap: max_parallel_steps.clamp(
                *MAX_PARALLEL_STEPS_RANGE.start(),
                *MAX_PARALLEL_STEPS_RANGE.end(),
            ),
            in_use: 0,
            held: None,
        }
    }

    /// Continue the walk: every decision that can be taken now, in run order.
    /// Call it at the start and after every [`Scheduler::settle`].
    pub fn startable(&mut self) -> Vec<Start> {
        let mut out = Vec::new();
        if let Some(held) = self.held.take() {
            if self.in_use >= self.cap {
                self.held = Some(held);
                return out;
            }
            self.in_use += 1;
            self.slot[held.idx] = Slot::Running;
            out.push(Start::Run {
                idx: held.idx,
                step_no: held.step_no,
                sources: held.sources,
            });
        }
        while let Some(&idx) = self.order.get(self.cursor) {
            if let Some(consume) = self.adopt[idx] {
                let step_no = match consume {
                    Consume::Nothing => None,
                    Consume::Next => {
                        self.step_counter += 1;
                        Some(self.step_counter)
                    }
                    Consume::Persisted(n) => {
                        self.step_counter = n;
                        Some(n)
                    }
                };
                self.decide(idx, Slot::Settled);
                out.push(Start::Adopt { idx, step_no });
                continue;
            }
            if self.outside[idx] {
                self.decide(idx, Slot::Settled);
                out.push(Start::Skip {
                    idx,
                    why: SkipWhy::OutsideScope,
                });
                continue;
            }
            if self.preds[idx]
                .iter()
                .any(|(s, _)| self.slot[*s] != Slot::Settled)
            {
                break;
            }
            let views: Vec<EdgeView> = self.preds[idx]
                .iter()
                .map(|(s, e)| EdgeView {
                    source: self.ids[*s].clone(),
                    errored: self.errored[*s],
                    has_output: self.has_output[*s],
                    edge_active: !self.inactive.contains(e),
                })
                .collect();
            match decide_node(&views) {
                NodeDecision::ErrorSkip => {
                    self.errored[idx] = true;
                    self.decide(idx, Slot::Settled);
                    out.push(Start::Skip {
                        idx,
                        why: SkipWhy::Upstream,
                    });
                }
                NodeDecision::BranchSkip => {
                    self.decide(idx, Slot::Settled);
                    out.push(Start::Skip {
                        idx,
                        why: SkipWhy::Branch,
                    });
                }
                NodeDecision::Run(sources) => {
                    self.step_counter += 1;
                    let step_no = self.step_counter;
                    if self.slotted[idx] && self.in_use >= self.cap {
                        self.cursor += 1;
                        out.push(Start::Queue {
                            idx,
                            step_no,
                            sources: sources.clone(),
                        });
                        self.held = Some(Held {
                            idx,
                            step_no,
                            sources,
                        });
                        break;
                    }
                    self.in_use += usize::from(self.slotted[idx]);
                    self.decide(idx, Slot::Running);
                    out.push(Start::Run {
                        idx,
                        step_no,
                        sources,
                    });
                }
            }
        }
        out
    }

    /// A node that ran has settled (success, error or a chat skip).
    pub fn settle(&mut self, idx: usize, outcome: Outcome) {
        if self.slot[idx] == Slot::Running && self.slotted[idx] {
            self.in_use -= 1;
        }
        self.slot[idx] = Slot::Settled;
        self.errored[idx] |= outcome.errored;
        self.has_output[idx] |= outcome.has_output;
        self.inactive.extend(outcome.inactive_edges);
    }

    /// Nothing runs, nothing is held and every node is decided: the run is over.
    pub fn is_finished(&self) -> bool {
        self.held.is_none()
            && self.cursor == self.order.len()
            && !self.slot.contains(&Slot::Running)
    }

    /// Step slots: (in use, cap) — for "waiting for a step slot (4 of 4 in use)".
    pub fn slots(&self) -> (usize, usize) {
        (self.in_use, self.cap)
    }

    /// The node the walk stopped at, when later nodes are ready but wait for
    /// it to start (run order): `(blocking idx, ready idxs)` — for the note
    /// "waiting for <step> to start (run order)". `None` when nobody waits.
    pub fn run_order_wait(&self) -> Option<(usize, Vec<usize>)> {
        let blocking = self
            .held
            .as_ref()
            .map(|h| h.idx)
            .or_else(|| self.order.get(self.cursor).copied())?;
        let ready: Vec<usize> = self.order[self.cursor..]
            .iter()
            .copied()
            .filter(|&i| i != blocking && self.slot[i] == Slot::Undecided)
            .filter(|&i| {
                self.preds[i]
                    .iter()
                    .all(|(s, _)| self.slot[*s] == Slot::Settled)
            })
            .collect();
        (!ready.is_empty()).then_some((blocking, ready))
    }

    fn decide(&mut self, idx: usize, slot: Slot) {
        self.slot[idx] = slot;
        self.cursor += 1;
    }
}

/// The number an adopted state consumes (DR-013).
#[derive(Debug, Clone, Copy)]
enum Consume {
    /// Never executed (branch- or upstream-skipped).
    Nothing,
    /// An old row that executed: the next number.
    Next,
    /// Its persisted `step_no`.
    Persisted(u32),
}

fn consumes_number(st: &NodeRunState) -> Consume {
    if let Some(n) = st.step_no {
        return Consume::Persisted(n);
    }
    let executed = matches!(st.status, NodeStatus::Success | NodeStatus::Error)
        || st.status == NodeStatus::Skipped && st.started_at.is_some();
    if executed {
        Consume::Next
    } else {
        Consume::Nothing
    }
}

/// The out-edges a chat-skipped node does not take: its conditional ones (it
/// decided nothing); the unconditional ones stay live for its marker output.
pub fn chat_skip_inactive_edges(graph: &WorkflowGraph, node_id: &str) -> Vec<String> {
    outgoing_edges(graph, node_id)
        .into_iter()
        .filter(|e| e.condition.is_some())
        .map(|e| e.id.clone())
        .collect()
}

#[cfg(test)]
mod tests;
