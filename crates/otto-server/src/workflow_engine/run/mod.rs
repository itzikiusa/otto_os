//! Phases of `run_workflow_with_budget`, moved out of the driver verbatim.
//! `finalize` is the post-loop phase (stop, drain, terminal write, delivery,
//! cleanup).

pub(super) mod finalize;
