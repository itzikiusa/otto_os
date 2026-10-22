//! Replay publication preserves acknowledged progress even if a later send fails.
//! The publisher seam keeps byte transformations and partial-result handling on
//! the same path for the Kafka adapter and deterministic failure tests.
//!
//! Intent is persisted BEFORE the first send (data-integrity-4-05): the
//! `broker_replays` row starts `running` and `on_start` (the `replay.start`
//! audit entry) runs, then the evidence is checkpointed every
//! [`CHECKPOINT_EVERY`] acknowledgements and the row ends `done` / `failed`.
//! A daemon stop mid-loop leaves a `running` row with the evidence up to the
//! last checkpoint, which the next start marks `interrupted`.

use crate::kafka::RawMessage;
use crate::types::{ProduceResp, ReplayEvidence, ReplayReq, ReplayResp};
use otto_core::{Id, Result};
use otto_state::BrokerOpsRepo;
use std::future::Future;

/// Acknowledgements between evidence checkpoints.
pub(crate) const CHECKPOINT_EVERY: usize = 100;

pub(crate) async fn publish<F, Fut, S, SFut>(
    ops: Option<&BrokerOpsRepo>,
    cluster_id: &Id,
    req: &ReplayReq,
    messages: Vec<RawMessage>,
    on_start: S,
    mut send: F,
) -> ReplayResp
where
    F: FnMut(RawMessage) -> Fut,
    Fut: Future<Output = Result<ProduceResp>>,
    S: FnOnce(Id) -> SFut,
    SFut: Future<Output = ()>,
{
    let mut evidence = Vec::with_capacity(messages.len());
    let mut error = None;
    let mut replay_id = otto_core::new_id();
    let mut tracked = false;
    if let Some(ops) = ops {
        match ops
            .start_replay(cluster_id, &req.source_topic, &req.target_topic)
            .await
        {
            Ok(row) => {
                replay_id = row.id;
                tracked = true;
            }
            // Without a record of intent, an interrupted run would leave no
            // trace of what it sent: refuse before anything is produced.
            Err(e) => {
                return ReplayResp {
                    replay_id,
                    source_topic: req.source_topic.clone(),
                    target_topic: req.target_topic.clone(),
                    count: 0,
                    evidence,
                    error: Some(format!(
                        "Replay not started: its evidence record could not be created ({e}). No messages were published."
                    )),
                    evidence_saved: false,
                };
            }
        }
    }
    on_start(replay_id.clone()).await;
    for mut m in messages {
        let key_preview = m.key.as_deref().and_then(|k| {
            std::str::from_utf8(k).ok().map(|s| {
                let mut chars = s.trim_end_matches('\0').chars();
                let mut preview: String = chars.by_ref().take(64).collect();
                if chars.next().is_some() {
                    preview.push('…');
                }
                preview
            })
        });
        if let Some(t) = &req.transform {
            if let Some(key) = &t.set_key {
                m.key = Some(key.as_bytes().to_vec());
            }
            if let Some((hk, hv)) = &t.add_header {
                if let Some(pos) = m.headers.iter().position(|(k, _)| k == hk) {
                    m.headers[pos] = (hk.clone(), Some(hv.as_bytes().to_vec()));
                } else {
                    m.headers.push((hk.clone(), Some(hv.as_bytes().to_vec())));
                }
            }
        }
        let (partition, offset) = (m.partition, m.offset);
        match send(m).await {
            Ok(resp) => evidence.push(ReplayEvidence {
                partition,
                offset,
                key_preview,
                target_partition: resp.partition,
                target_offset: resp.offset,
            }),
            Err(e) => {
                error = Some(format!(
                    "Replay stopped after {} acknowledged messages: {e}. The failed send may also have reached Kafka; retrying can duplicate messages.",
                    evidence.len()
                ));
                break;
            }
        }
        if tracked && evidence.len() % CHECKPOINT_EVERY == 0 {
            if let Some(ops) = ops {
                // A missed checkpoint only widens the gap an interruption
                // could lose; the final write below still reports failure.
                if let Err(e) = ops
                    .update_replay(
                        &replay_id,
                        evidence.len() as i64,
                        serde_json::to_value(&evidence).unwrap_or_default(),
                        "running",
                    )
                    .await
                {
                    tracing::warn!(replay = %replay_id, error = %e, "replay evidence checkpoint failed");
                }
            }
        }
    }

    let mut evidence_saved = false;
    if let (true, Some(ops)) = (tracked, ops) {
        let status = if error.is_some() { "failed" } else { "done" };
        match ops
            .update_replay(
                &replay_id,
                evidence.len() as i64,
                serde_json::to_value(&evidence).unwrap_or_default(),
                status,
            )
            .await
        {
            Ok(()) => evidence_saved = true,
            Err(e) => {
                let save_error = format!("Evidence could not be saved: {e}. Keep the returned evidence; retrying can duplicate messages.");
                error = Some(match error {
                    Some(prior) => format!("{prior} {save_error}"),
                    None => save_error,
                });
            }
        }
    }
    ReplayResp {
        replay_id,
        source_topic: req.source_topic.clone(),
        target_topic: req.target_topic.clone(),
        count: evidence.len(),
        evidence,
        error,
        evidence_saved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::Error;

    #[tokio::test]
    async fn nth_publish_failure_persists_acknowledged_prefix_and_stops() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        // Use the production repository against an isolated minimal evidence table.
        sqlx::query("CREATE TABLE broker_replays (id TEXT PRIMARY KEY, cluster_id TEXT, source_topic TEXT, target_topic TEXT, count INTEGER, evidence_json TEXT, status TEXT NOT NULL DEFAULT 'done', created_at TEXT)")
            .execute(&pool).await.unwrap();
        let ops = BrokerOpsRepo::new(pool);
        let req: ReplayReq = serde_json::from_value(serde_json::json!({
            "source_topic":"source", "target_topic":"target", "selector":{"type":"latest", "count":3}
        })).unwrap();
        let messages = (0..3)
            .map(|offset| RawMessage {
                partition: 0,
                offset,
                timestamp_ms: None,
                key: None,
                value: Some(vec![42]),
                headers: vec![],
                size: 1,
            })
            .collect();
        let sent = std::cell::RefCell::new(Vec::new());
        let started = std::cell::Cell::new(None);
        let response = publish(
            Some(&ops),
            &"isolated".into(),
            &req,
            messages,
            |id: Id| {
                // The row exists (running) before the first send.
                assert!(sent.borrow().is_empty(), "intent recorded before sending");
                started.set(Some(id));
                std::future::ready(())
            },
            |m| {
                sent.borrow_mut().push(m.offset);
                std::future::ready(if m.offset == 1 {
                    Err(Error::Upstream("injected send failure".into()))
                } else {
                    Ok(ProduceResp {
                        partition: 0,
                        offset: 100 + m.offset,
                    })
                })
            },
        )
        .await;
        assert_eq!(started.take().as_ref(), Some(&response.replay_id));
        assert_eq!(
            sent.into_inner(),
            vec![0, 1],
            "do not continue producing after the failed send"
        );
        assert_eq!(response.count, 1);
        assert!(response
            .error
            .as_deref()
            .unwrap()
            .contains("retrying can duplicate"));
        assert!(response.evidence_saved);
        let saved = ops.get_replay(&response.replay_id).await.unwrap();
        assert_eq!(saved.count, 1);
        assert_eq!(saved.status, "failed");
        let evidence: serde_json::Value = serde_json::from_str(&saved.evidence_json).unwrap();
        assert_eq!(evidence[0]["offset"], 0);
        assert_eq!(evidence[0]["target_offset"], 100);
    }
}
