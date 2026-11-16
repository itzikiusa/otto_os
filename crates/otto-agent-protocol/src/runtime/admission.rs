//! Bounds on getting a message to the agent: how long a starting CLI may
//! take to answer its handshake, and how much may wait in a thread's queue.
use super::*;

/// How long a spawned CLI may take to answer its handshake before the thread
/// says so instead of showing "Starting" forever. Generous: a cold CLI
/// (first run after an update, a slow MCP server, a big repo) can take tens
/// of seconds.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(90);

/// At most this many messages wait in one thread's queue …
pub const MAX_QUEUED_PROMPTS: usize = 20;
/// … holding at most this much attachment data (base64) together.
pub const MAX_QUEUED_BYTES: usize = 64 * 1024 * 1024;

fn attachment_bytes(req: &RuntimePromptReq) -> usize {
    req.images
        .iter()
        .map(|i| i.data_base64.len())
        .sum::<usize>()
        + req.files.iter().map(|f| f.data_base64.len()).sum::<usize>()
}

/// Refuse queueing `req` behind what already waits (a clear 4xx, never a
/// silently dropped message).
pub(super) fn check_queue_room(queue: &[QueuedRecord], req: &RuntimePromptReq) -> Result<()> {
    if queue.len() >= MAX_QUEUED_PROMPTS {
        return Err(Error::Conflict(format!(
            "{MAX_QUEUED_PROMPTS} messages are already queued — wait for the agent, or remove one"
        )));
    }
    let held: usize = queue.iter().map(|q| attachment_bytes(&q.req)).sum();
    if held + attachment_bytes(req) > MAX_QUEUED_BYTES {
        return Err(Error::PayloadTooLarge(format!(
            "the queued messages' attachments would exceed {} MiB — wait for the agent, or remove one",
            MAX_QUEUED_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

/// A queued prompt shared, not copied: publishing and persisting the queue
/// clone the `Arc`, and the row is serialized on the persister task, off the
/// session lock (attachments can be tens of MiB).
pub(super) mod shared_req {
    use std::sync::Arc;

    use otto_core::agent_runtime::RuntimePromptReq;

    pub fn serialize<S: serde::Serializer>(
        v: &Arc<RuntimePromptReq>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&**v, s)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<Arc<RuntimePromptReq>, D::Error> {
        <RuntimePromptReq as serde::Deserialize>::deserialize(d).map(Arc::new)
    }
}

/// Fail the spawn of `live`'s current process if it has not answered its
/// handshake within `config.handshake_timeout` (a CLI waiting on a login or
/// a browser step, a hung language server). The `Fatal` stops it and fails
/// a turn already waiting on it; the next message respawns.
pub(super) fn watch_handshake(inner: &Arc<Inner>, live: &Arc<Live>) {
    let after = inner.config.handshake_timeout;
    if after.is_zero() {
        return;
    }
    let Some(epoch) = live.lock().proc.as_ref().map(|p| p.epoch) else {
        return;
    };
    // Weak: a watchdog never keeps an evictable session loaded.
    let (inner, live) = (Arc::downgrade(inner), Arc::downgrade(live));
    tokio::spawn(async move {
        tokio::time::sleep(after).await;
        let (Some(inner), Some(live)) = (inner.upgrade(), live.upgrade()) else {
            return;
        };
        let mut st = live.lock();
        let starting = st.proc.as_ref().is_some_and(|p| p.epoch == epoch)
            && st
                .proj
                .session
                .as_ref()
                .is_some_and(|p| p.status == ProviderSessionStatus::Starting);
        if !starting {
            return;
        }
        let failure = handshake_timeout_failure(st.session.provider_slug(), after);
        tracing::warn!(session = %st.session.id, "{}", failure.message);
        let step = DriverStep {
            events: vec![DriverEvent::Fatal(failure)],
            ..DriverStep::default()
        };
        apply_step(&inner, &live, &mut st, step);
    });
}

pub(super) fn handshake_timeout_failure(cli: &str, after: Duration) -> ProviderFailure {
    ProviderFailure {
        class: "handshake_timeout".into(),
        message: format!(
            "{cli} did not answer within {} s — is it installed and signed in?",
            after.as_secs()
        ),
        code: None,
        retryable: Some(true),
    }
}
