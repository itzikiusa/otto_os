//! Transcript services shared by the conversation routes, History and the
//! daemon's background hosts (architecture-3-03): [`resolve`] (session →
//! transcript path), [`fold`] (the fold knobs) and [`artifacts`] (the
//! work-graph artifact mirror).

pub mod artifacts;
pub mod fold;
pub mod resolve;
