//! Phases of `run_workflow_with_budget`, moved out of the driver verbatim.
//! `admission` is everything before the first node (graph, scope, identity,
//! input, repo isolation, context files, the Running CAS and the chat
//! progress pump); `state` is the run's mutable bookkeeping and the loop's
//! control signal; `board` holds the node states concurrent nodes publish;
//! `schedule` drives the scheduler and runs each started node (attempts,
//! outcome); `finalize` is the post-loop phase (stop, drain, terminal write,
//! delivery, cleanup). `identity` resolves whom a run acts as; `refs` syncs and resolves the
//! branches a run names.

pub(super) mod admission;
pub(super) mod board;
pub(super) mod chat;
pub(super) mod finalize;
pub(crate) mod hold;
pub(super) mod identity;
pub(super) mod refs;
pub(super) mod schedule;
pub(super) mod state;
