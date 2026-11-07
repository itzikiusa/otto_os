//! The node kinds `execute_node` dispatches to, one file per kind. Each
//! `execute` takes the whole [`NodeCall`]; the arm bodies were moved here
//! verbatim, so the kinds keep the parent module's helpers via
//! `use super::super::*`. [`dispatch`] is `execute_node`'s per-kind match,
//! moved here with the few-line kinds it still runs inline.

use super::{api_request, product_publication, truncate};
use super::{AgentAssociation, ProgressSink, RunEnv, StepScope};
use otto_core::domain::{User, Workspace};
use otto_core::workflows::{NodeActivity, WorkflowNode};
use otto_core::{Id, Result};
use serde_json::{json, Value};
use std::time::Duration;

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

/// Execute one node by kind (`execute_node`'s match). Returns
/// `(output, logs)`. The large kinds live in `nodes/<kind>.rs` and take the
/// whole call; the few-line kinds stay inline below.
pub(in crate::workflow_engine) async fn dispatch(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let (ctx, ws, user, node) = (call.ctx, call.ws, call.user, call.node);
    let env = call.env;
    let run_id: &Id = &env.run_id;
    let p = &node.params;
    match node.kind.as_str() {
        "manual_trigger" => Ok((call.input, vec![])),

        "log" => {
            let input = call.input;
            let line = format!("{}", input);
            Ok((input, vec![format!("log: {}", truncate(&line, 500))]))
        }

        "delay" => {
            let ms = p.get("ms").and_then(Value::as_u64).unwrap_or(0).min(10_000);
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Ok((call.input, vec![format!("waited {ms}ms")]))
        }

        "transform" => {
            // Merge params.json (object) onto the incoming object.
            let mut base = match call.input {
                Value::Object(m) => m,
                other => {
                    let mut m = serde_json::Map::new();
                    m.insert("input".into(), other);
                    m
                }
            };
            if let Some(Value::Object(patch)) = p.get("json") {
                for (k, v) in patch {
                    base.insert(k.clone(), v.clone());
                }
            }
            Ok((Value::Object(base), vec![]))
        }

        "agent_prompt" => agent_prompt::execute(call).await,

        "prepare_context" => prepare_context::execute(call).await,

        "http_request" => http_request::execute(call).await,

        // --- Game pipeline scaffolds (need an external engine to be real) ----
        "game_engine" => game_engine::execute(call).await,

        "verifier" => verifier::execute(call).await,

        // --- DB Query -------------------------------------------------------
        // Runs a read-only SQL/NoSQL statement against a saved DB-Explorer
        // connection. `params.connection_id` is the otto-dbviewer Connection id;
        // `params.statement` is the query text; `params.max_rows` (optional,
        // default 100) caps the result set.  Mutating statements (INSERT/UPDATE/
        // DELETE/DROP/…) are blocked by the engine's existing write-gate unless
        // `params.confirm_write = true` is explicitly set (not the default).
        "db_query" => db_query::execute(call).await,

        // --- Broker Peek ----------------------------------------------------
        // Consumes up to `params.limit` recent messages from a Kafka topic on
        // a saved broker cluster.  Read-only (consume, not produce).
        // `params.cluster_id` — the otto-brokers BrokerCluster id.
        // `params.topic`       — topic name.
        // `params.limit`       — max messages to return (default 20, capped 50).
        "broker_peek" => broker_peek::execute(call).await,

        // --- Channel Notify -------------------------------------------------
        // Sends a text message to a Slack or Telegram integration configured
        // for the workflow's workspace.
        // `params.message`  — the text to send (supports {input.*} references
        //                     as a simple placeholder substitution: not a full
        //                     templating engine, just the top-level input object
        //                     keys).
        // `params.channel`  — "slack" | "telegram" (default: first enabled)
        // The `channel_id` (Slack channel / Telegram chat id) is taken from
        // `Integration.channel_id` (the default chat set when the integration
        // was configured). To override, the params may contain `chat_id`.
        "channel_notify" => channel_notify::execute(call).await,

        // --- Budget Gate ----------------------------------------------------
        // Calls `check_budget` (same function the monitor uses) for the given
        // workspace + provider.  If the budget is blocked, the node errors,
        // causing downstream nodes to be skipped.  If exceeded but not blocked
        // (warn-only mode), it continues and sets `exceeded: true` in the output
        // so downstream nodes can branch on it.
        // `params.provider`      — provider name; empty ⇒ the run's resolved default agent
        // `params.workspace_id`  — override the run workspace (optional; default ws.id)
        "budget_gate" => budget_gate::execute(call).await,

        // --- Human Approval -------------------------------------------------
        // Pauses the run until an operator calls
        // `POST /workflow-runs/{id}/approve` with `{"node_id": ..., "approved": true}`.
        // The engine sets `waiting_approval = 1` on the run row and then waits
        // — woken by the decision's announcement, re-reading at least every
        // [`APPROVAL_SAFETY_RECHECK`] — up to `params.timeout_s` (default 24h,
        // min 60s), for the row to be cleared. If the operator rejects (`approved: false`) the
        // node errors. If the deadline expires the node errors with
        // "approval timed out".
        "human_approval" => human_approval::execute(call).await,

        // --- API Run --------------------------------------------------------
        // Share the API client's environment, auth, transport and history path.
        "api_run" => api_request::execute(ctx, ws, user, p).await,

        // --- Condition (branching primitive) --------------------------------
        "condition" => {
            let expr = p.get("expr").and_then(Value::as_str).unwrap_or("true");
            let input = call.input;
            let value = otto_core::expr::eval(expr, &input)
                .map_err(|e| otto_core::Error::Invalid(format!("condition: {e}")))?;
            let result = otto_core::expr::truthy(&value);
            let mut out = match &input {
                Value::Object(m) => m.clone(),
                other => {
                    let mut m = serde_json::Map::new();
                    m.insert("input".into(), other.clone());
                    m
                }
            };
            out.insert("result".into(), json!(result));
            out.insert("value".into(), value);
            Ok((
                Value::Object(out),
                vec![format!("condition `{expr}` → {result}")],
            ))
        }

        // --- Loop (bounded iterate-until) -----------------------------------
        "loop" => loop_node::execute(call).await,

        // --- Review Run (wired: local-review engine + 0–100 score + goals) ---
        "review_run" => review_run::execute(call).await,

        // --- Product nodes (wired: real single-agent turn over story context) -
        "product_analyze" | "product_rewrite" | "product_plan" => product::execute(call).await,

        // --- Product Publish (RFC / Jira; dry-run by default) ----------------
        "product_publish" => {
            product_publication::execute(ctx, ws, user, node, call.input, run_id).await
        }

        // --- Canvas (generate a mermaid/excalidraw diagram artifact) ---------
        "canvas" => canvas::execute(call).await,

        // --- Git PR (draft, or open with open=true) --------------------------
        // Multi-repo aware: opens ONE PR per changed repo. Targets come from the
        // reference the implementer/reviewer used — explicit `params.repos[]`, a
        // loop's published `repos[]`, a single upstream review reference, or
        // fan-in of several review branches — so a run that touched multiple
        // repositories opens them all, each on its own base branch. The PR never
        // needs the repo/base re-typed; it inherits them.
        "git_pr" => git_pr::execute(call).await,

        // --- Self-improvement (offer-only) ----------------------------------
        // Reflect on the workspace's recent sessions and OFFER skill/memory
        // improvements. Runs the self-improvement engine with Autonomy::Propose
        // so every proposed edit is QUEUED for approval — never auto-applied —
        // then posts the offered list to the trigger's chat thread (design §I).
        "self_improve" => self_improve::execute(call).await,

        other => Err(otto_core::Error::Invalid(format!(
            "unknown node kind '{other}'"
        ))),
    }
}
