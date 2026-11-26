//! The run's chat thread in the `live` style (engine.md §9): the view the
//! renderers take, the needs-you post and notice, the status card of a run
//! still queued for a slot (DR-015, DR-113.4) and the stored status message
//! (`chat_status_json`) the progress pump keeps editing.

use super::super::*;
use super::schedule::RunCx;
use otto_core::agent_runtime::Attention;
use otto_workflows::chat_status::{
    render_needs_you, render_status, ChatThread, ChatUpdates, ChatView, StatusMessage,
};

/// The renderers' view of `workflow`: its name, the clock, the Mac's offset.
pub(in crate::workflow_engine) fn view(workflow: &Workflow) -> ChatView<'_> {
    ChatView {
        workflow: &workflow.name,
        now: chrono::Utc::now(),
        tz: *chrono::Local::now().offset(),
    }
}

/// The run's chat style (§9.5); the offline e2e daemon keeps today's
/// per-step thread unless the run asks for one, like it keeps its PTY agent.
pub(in crate::workflow_engine) fn style(workflow: &Workflow, input: &Value) -> ChatUpdates {
    let start_sets =
        |n: &WorkflowNode| n.kind == "manual_trigger" && n.params.get(ChatUpdates::KEY).is_some();
    let explicit =
        input.get(ChatUpdates::KEY).is_some() || workflow.graph.nodes.iter().any(start_sets);
    if otto_e2e() && !explicit {
        return ChatUpdates::PerStep;
    }
    ChatUpdates::resolve(&workflow.graph, input)
}

/// A step started waiting on a person: one notice and, in the live style, one
/// new post in the run's thread (edits do not notify) — once per episode.
pub(in crate::workflow_engine) async fn needs_you(
    cx: RunCx<'_>,
    node: &WorkflowNode,
    a: &Attention,
) {
    let step = node_display_name(node);
    let slack = cx.input.get("channel").and_then(Value::as_str) == Some("slack");
    let mention = cx
        .input
        .get("user")
        .and_then(Value::as_str)
        .filter(|_| slack);
    cx.progress
        .announce(render_needs_you(&view(cx.workflow), step, a, mention, None));
    crate::run_notices::on_workflow_needs_you(cx.ctx, cx.workflow, cx.run_id, step, a).await;
}

/// The run's chat thread: the stored status message (a restart, resume or
/// retry edits the same one), else a new one in the trigger's thread.
pub(in crate::workflow_engine) async fn thread(
    ctx: &ServerCtx,
    run_id: &Id,
    target: &ChatTarget,
    style: ChatUpdates,
) -> Option<ChatThread> {
    let integ = otto_state::IntegrationsRepo::new(ctx.pool.clone())
        .get(&target.ws, target.channel)
        .await
        .ok()??;
    let adapter = ctx.outbound.adapter(&integ).await?;
    let stored = WorkflowsRepo::new(ctx.pool.clone())
        .chat_status(run_id)
        .await;
    let msg = stored
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_else(|| StatusMessage {
            channel: target.channel.as_str().to_string(),
            chat: target.chat.clone(),
            thread: target.thread.clone(),
            message_id: None,
        });
    Some(ChatThread::new(style, adapter, msg))
}

/// Store the status message's id once it is posted (or posted again).
pub(in crate::workflow_engine) async fn persist(ctx: &ServerCtx, run_id: &Id, t: &ChatThread) {
    let msg = serde_json::to_value(&t.live.msg).ok();
    let _ = WorkflowsRepo::new(ctx.pool.clone())
        .set_chat_status(run_id, msg.as_ref())
        .await;
}

/// The live card of a run that waits for a run slot (queued), or of one
/// canceled while it waited (`canceled`): a chat run says so at once.
pub(in crate::workflow_engine) async fn queued_card(
    ctx: &ServerCtx,
    workflow: &Workflow,
    run_id: &Id,
    input: &Value,
    canceled: bool,
) {
    let Some(target) = resolve_chat_target(workflow, input) else {
        return;
    };
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    if canceled && !repo.is_canceled(run_id).await {
        return; // the gate closed: the run waits for the next boot
    }
    let style = style(workflow, input);
    if style != ChatUpdates::Live {
        return;
    }
    let Some(mut t) = thread(ctx, run_id, &target, style).await else {
        return;
    };
    let text = if canceled {
        format!("⏹ *{}* — canceled while queued", workflow.name)
    } else {
        let ov = otto_core::workflows::RunOverview {
            state: otto_core::workflows::RunState::Queued,
            ..Default::default()
        };
        render_status(&view(workflow), &ov, &[], &workflow.graph)
    };
    if t.status(text).await | t.flush().await {
        persist(ctx, run_id, &t).await;
    }
}
