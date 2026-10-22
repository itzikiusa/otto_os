//! The `broker_peek` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall { ctx, node, .. } = call;
    let p = &node.params;
    let cluster_id: Id = p
        .get("cluster_id")
        .and_then(Value::as_str)
        .ok_or_else(|| otto_core::Error::Invalid("broker_peek: missing cluster_id".into()))?
        .to_string();
    let topic = p
        .get("topic")
        .and_then(Value::as_str)
        .ok_or_else(|| otto_core::Error::Invalid("broker_peek: missing topic".into()))?
        .to_string();
    let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(20).min(50) as usize;
    let req = ConsumeReq {
        partition: None,
        start: otto_brokers::types::StartPosition::default(),
        limit,
        max_wait_ms: Some(5_000),
        key_filter: None,
        value_filter: None,
        find_from_beginning: false,
        decode: ValueFormat::Auto,
        mask: None,
    };
    let resp = ctx
        .brokers
        .consume(&cluster_id, &topic, &req)
        .await
        .map_err(|e| otto_core::Error::Upstream(format!("broker_peek: {e}")))?;
    let count = resp.messages.len();
    // Serialize each message to a plain JSON object so downstream nodes
    // (e.g. agent_prompt or transform) can pattern-match on message content.
    let messages: Vec<Value> = resp
        .messages
        .into_iter()
        .map(|m| {
            json!({
                "partition": m.partition,
                "offset": m.offset,
                "timestamp_ms": m.timestamp_ms,
                "key": m.key.as_ref().map(|d| d.text.as_str()),
                "value": m.value.as_ref().map(|d| d.text.as_str()),
            })
        })
        .collect();
    Ok((
        json!({ "topic": topic, "messages": messages, "count": count }),
        vec![format!("broker_peek: {count} messages from '{topic}'")],
    ))
}
