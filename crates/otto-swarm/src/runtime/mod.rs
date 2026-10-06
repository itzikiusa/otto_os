//! The swarm orchestration runtime (moved out of otto-server): the per-swarm
//! Coordinator + lifecycle/recruit/plan/goals/triggers HTTP surface
//! ([`engine`]), the one-turn runner ([`run`]) and standalone agent runner
//! ([`agent_run`]), the agent scheduler ([`scheduler`]) and its event bell
//! ([`wake`]), the goal verifier ([`verify`]), branch merge ([`merge`]),
//! channel triggers ([`channels`]) and per-agent workspaces ([`workspace`]).
//!
//! Everything it needs from the daemon comes through [`host::SwarmHost`].

pub mod agent_run;
pub mod channels;
pub mod engine;
pub mod host;
pub mod merge;
pub mod run;
pub mod scheduler;
pub mod verify;
pub mod wake;
pub mod workspace;

pub use host::{SwarmHost, SwarmRt};
