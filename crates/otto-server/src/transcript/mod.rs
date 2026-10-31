//! Transcript services shared by the conversation routes and the daemon's
//! background hosts (architecture-3-03): [`resolve`] (session → transcript
//! path) and [`fold`] (the fold knobs).

pub mod fold;
pub mod resolve;
