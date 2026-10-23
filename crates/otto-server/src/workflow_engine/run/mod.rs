//! Phases of `run_workflow_with_budget`, moved out of the driver verbatim.
//! `admission` is everything before the first node (graph, scope, identity,
//! input, repo isolation, context files, the Running CAS and the chat
//! progress pump); `state` is the run's mutable bookkeeping and the loop's
//! control signal; `schedule` runs one node per step (decision, attempts,
//! outcome); `finalize` is the post-loop phase (stop, drain, terminal write,
//! delivery, cleanup). `identity` resolves whom a run acts as.

pub(super) mod admission;
pub(super) mod finalize;
pub(super) mod identity;
pub(super) mod schedule;
pub(super) mod state;
