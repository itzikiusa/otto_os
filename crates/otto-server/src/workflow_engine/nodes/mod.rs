//! The node kinds `execute_node` dispatches to, one file per kind. Each
//! `execute` takes the whole [`NodeCall`]; the arm bodies were moved here
//! verbatim, so the kinds keep the parent module's helpers via
//! `use super::super::*`.

use super::{AgentAssociation, ProgressSink, RunEnv, StepScope};
use otto_core::domain::{User, Workspace};
use otto_core::workflows::{NodeActivity, WorkflowNode};
use serde_json::Value;

use crate::state::ServerCtx;

pub(super) mod agent_prompt;
pub(super) mod broker_peek;
pub(super) mod budget_gate;
pub(super) mod canvas;
pub(super) mod channel_notify;
pub(super) mod db_query;
pub(super) mod game_engine;
pub(super) mod git_pr;
pub(super) mod http_request;
pub(super) mod human_approval;
pub(super) mod loop_node;
pub(super) mod prepare_context;
pub(super) mod product;
pub(super) mod review_run;
pub(super) mod self_improve;
pub(super) mod swarm_task;
pub(super) mod verifier;

/// The arguments of one `execute_node` call, bundled so every node kind takes
/// the same single parameter. Field meanings are documented on
/// `execute_node`.
pub(in crate::workflow_engine) struct NodeCall<'a> {
    pub(in crate::workflow_engine) ctx: &'a ServerCtx,
    pub(in crate::workflow_engine) ws: &'a Workspace,
    pub(in crate::workflow_engine) user: &'a User,
    pub(in crate::workflow_engine) node: &'a WorkflowNode,
    pub(in crate::workflow_engine) input: Value,
    pub(in crate::workflow_engine) env: &'a RunEnv,
    pub(in crate::workflow_engine) scope: &'a StepScope,
    pub(in crate::workflow_engine) session_tx:
        &'a tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    pub(in crate::workflow_engine) log_tx: &'a tokio::sync::mpsc::UnboundedSender<String>,
    pub(in crate::workflow_engine) activity_tx:
        &'a tokio::sync::mpsc::UnboundedSender<NodeActivity>,
    pub(in crate::workflow_engine) progress: &'a ProgressSink,
}
