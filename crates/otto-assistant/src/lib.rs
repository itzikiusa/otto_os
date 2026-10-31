//! Otto's **Personal Agents** engine + scheduler, extracted from `otto-server`.
//!
//! Leaf crate: everything it needs from the daemon goes through
//! [`AssistantCtx`], which `otto-server` implements for `ServerCtx`. The HTTP
//! routes (`routes::personal_agents`), the read-only agent policy
//! (`personal_agent_policy`, used by `feature_guard`) and the live activity
//! feed stay in `otto-server`.

pub mod ctx;
pub mod personal_agent_documents;
pub mod personal_agent_memory;
pub mod personal_agents_engine;
pub mod personal_agents_scheduler;

pub use ctx::{AgentSessionOutcome, AgentSessionRun, AssistantCtx, BoxFut, RunFailureNotice};
