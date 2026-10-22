//! The `channel_notify` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall {
        ctx,
        ws,
        node,
        input,
        env,
        ..
    } = call;
    let run_id: &Id = &env.run_id;
    let p = &node.params;
    let raw_msg = p
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Workflow notification")
        .to_string();

    // The RUN's original input (trigger metadata: originating chat +
    // thread, jira_ticket, working_directory, …). Node-input threading
    // loses these hop by hop — a node that emits a fresh object drops
    // every run-level key — so by the time a notify node runs at the end
    // of a graph, `input` holds only the previous node's output. Read the
    // run row instead of trusting the hop chain.
    // One column (perf N5) — not the whole run with its nodes_json.
    let run_input = WorkflowsRepo::new(ctx.pool.clone())
        .run_input(run_id)
        .await
        .unwrap_or(Value::Null);

    // `{key}` substitution draws from the run input FIRST, then the node
    // input overlays it — so `{jira_ticket}` / `{working_directory}`
    // resolve from the trigger while `{score}` / `{total}` / `{base}`
    // come from the step that just ran (and win on any name collision).
    let mut subs = serde_json::Map::new();
    if let Some(obj) = run_input.as_object() {
        for (k, v) in obj {
            subs.insert(k.clone(), v.clone());
        }
    }
    if let Some(obj) = input.as_object() {
        for (k, v) in obj {
            subs.insert(k.clone(), v.clone());
        }
    }
    let message = subs.iter().fold(raw_msg, |acc, (k, v)| {
        let placeholder = format!("{{{k}}}");
        let replacement = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        acc.replace(&placeholder, &replacement)
    });

    // Where the run was triggered from. A chat-triggered run carries the
    // originating chat + thread; every agent step already replies inside
    // that thread, so a notify node posting to the integration's DEFAULT
    // channel with no thread was the one message that escaped the
    // conversation — arriving with no indication of which PR it belonged
    // to. Reply in the originating thread when we have one.
    let origin_channel = run_input
        .get("channel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let origin_chat = run_input
        .get("chat")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let origin_thread = run_input
        .get("thread")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let preferred_channel: Option<Channel> =
        p.get("channel")
            .and_then(Value::as_str)
            .and_then(|s| match s {
                "slack" => Some(Channel::Slack),
                "telegram" => Some(Channel::Telegram),
                _ => None,
            });

    let integrations = ctx
        .integrations_store
        .list_all_enabled()
        .await
        .map_err(|e| {
            otto_core::Error::Upstream(format!("channel_notify: load integrations: {e}"))
        })?;

    // Filter to the workspace's enabled integrations, optionally by channel.
    // Webhooks are inbound-only (not a proactive-push target), so they're
    // excluded here.
    let targets: Vec<_> = integrations
        .into_iter()
        .filter(|i| i.workspace_id == ws.id)
        .filter(|i| i.channel != Channel::Webhook)
        .filter(|i| preferred_channel.is_none() || Some(i.channel) == preferred_channel)
        .filter(|i| !i.channel_id.trim().is_empty())
        .collect();

    if targets.is_empty() {
        return Err(otto_core::Error::Invalid(
            "channel_notify: no enabled integration with a default chat configured".into(),
        ));
    }

    let secrets = &ctx.secrets;
    let mut sent = 0usize;
    for integ in &targets {
        let ws_id = &integ.workspace_id;
        // Prefer the originating chat/thread when this integration is the
        // channel the run was triggered from; otherwise fall back to the
        // integration's configured default (a scheduled/manual run has no
        // originating conversation to reply into).
        let same_channel = origin_channel
            .as_deref()
            .is_some_and(|c| c.eq_ignore_ascii_case(integ.channel.as_str()));
        let (chat, thread) = match (same_channel, origin_chat.as_deref()) {
            (true, Some(c)) => (c, origin_thread.as_deref()),
            _ => (integ.channel_id.trim(), None),
        };
        // Build an outbound adapter reusing the same logic as
        // improve_notify (avoids a public API surface on ChannelManager).
        let send_result = match integ.channel {
            Channel::Telegram => {
                let key = format!("chan-bot-{ws_id}-telegram");
                let token = otto_core::secrets::get_async(secrets, &key).await;
                match token.ok().flatten().filter(|t| !t.is_empty()) {
                    Some(token) => {
                        let adapter = otto_channels::telegram::TelegramAdapter::new(token);
                        adapter.send(chat, thread, &message).await.map(|_| ())
                    }
                    None => {
                        tracing::debug!(workspace = %ws_id, "channel_notify: telegram token missing");
                        continue;
                    }
                }
            }
            Channel::Slack => {
                let key = format!("chan-bot-{ws_id}-slack");
                let token = otto_core::secrets::get_async(secrets, &key).await;
                match token.ok().flatten().filter(|t| !t.is_empty()) {
                    Some(token) => {
                        let adapter = otto_channels::slack::SlackAdapter::new(token);
                        adapter.send(chat, thread, &message).await.map(|_| ())
                    }
                    None => {
                        tracing::debug!(workspace = %ws_id, "channel_notify: slack token missing");
                        continue;
                    }
                }
            }
            // Webhooks are inbound-only; excluded from `targets` above.
            Channel::Webhook => continue,
        };
        match send_result {
            Ok(_) => sent += 1,
            Err(e) => {
                tracing::warn!("channel_notify: send failed: {e}");
            }
        }
    }

    if sent == 0 {
        return Err(otto_core::Error::Upstream(
            "channel_notify: all sends failed".into(),
        ));
    }
    Ok((
        json!({ "sent": sent, "message": message }),
        vec![format!("channel_notify: sent to {sent} integration(s)")],
    ))
}
