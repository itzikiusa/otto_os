//! Re-export of [`otto_core::cancel_signal`] — the primitive moved down so the
//! swarm runtime (`otto-swarm`) can share it; server code keeps this path.

pub use otto_core::cancel_signal::*;
