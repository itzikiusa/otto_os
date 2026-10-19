//! Admission, exact-turn cancellation and cleanup for a studio conversation.
use otto_core::cancel_signal::CancelSignal;
use otto_core::{Error, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

type Key = (String, String, String);
#[derive(Default)]
struct Running {
    id: String,
    cancel: CancelSignal,
    finished: CancelSignal,
}
fn turns() -> &'static Mutex<HashMap<Key, Arc<Running>>> {
    static TURNS: OnceLock<Mutex<HashMap<Key, Arc<Running>>>> = OnceLock::new();
    TURNS.get_or_init(Default::default)
}
pub(crate) struct Turn {
    key: Key,
    running: Arc<Running>,
}
impl Turn {
    pub(crate) fn cancel(&self) -> CancelSignal {
        self.running.cancel.clone()
    }
    pub(crate) fn check_running(&self) -> Result<()> {
        if self.running.cancel.is_cancelled() {
            return Err(Error::Conflict("product turn stopped".into()));
        }
        Ok(())
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        let mut active = turns().lock().unwrap_or_else(|e| e.into_inner());
        if active
            .get(&self.key)
            .is_some_and(|run| Arc::ptr_eq(run, &self.running))
        {
            active.remove(&self.key);
        }
        self.running.finished.cancel();
    }
}
#[cfg(test)]
pub(crate) fn claim(kind: &str, workspace: &str, conversation: &str) -> Result<Turn> {
    claim_with_id(kind, workspace, conversation, None)
}
pub(crate) fn claim_with_id(
    kind: &str,
    workspace: &str,
    conversation: &str,
    id: Option<&str>,
) -> Result<Turn> {
    let id = match id {
        Some(id) if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) => {
            return Err(Error::Invalid(
                "turn_id must contain 1–128 bytes without control characters".into(),
            ));
        }
        Some(id) => id.to_owned(),
        None => otto_core::new_id(),
    };
    let key = (kind.into(), workspace.into(), conversation.into());
    let mut active = turns().lock().unwrap_or_else(|e| e.into_inner());
    if active.contains_key(&key) {
        return Err(Error::Conflict(
            "a turn is already running in this conversation".into(),
        ));
    }
    let running = Arc::new(Running {
        id,
        ..Default::default()
    });
    active.insert(key.clone(), running.clone());
    Ok(Turn { key, running })
}
/// Drain only the matching generation. An old Stop never affects its successor.
/// False means no matching turn was admitted, including a send still in transit.
pub(crate) async fn stop(kind: &str, workspace: &str, conversation: &str, id: &str) -> bool {
    let running = {
        let active = turns().lock().unwrap_or_else(|e| e.into_inner());
        active
            .get(&(kind.into(), workspace.into(), conversation.into()))
            .filter(|run| run.id == id)
            .cloned()
    };
    let Some(running) = running else {
        return false;
    };
    running.cancel.cancel();
    running.finished.cancelled().await;
    true
}

/// Complete an accepted turn independently of its HTTP response lifetime.
pub(crate) async fn complete<T: Send + 'static>(
    task: impl std::future::Future<Output = crate::http::ApiResult<T>> + Send + 'static,
) -> crate::http::ApiResult<T> {
    tokio::spawn(task).await.map_err(|error| {
        crate::http::ApiError(Error::Upstream(format!(
            "product turn task failed: {error}"
        )))
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_turns_are_rejected_until_the_owner_finishes() {
        let id = otto_core::new_id();
        let first = claim("discovery", "ws", &id).unwrap();
        assert!(matches!(
            claim("discovery", "ws", &id),
            Err(Error::Conflict(_))
        ));
        drop(first);
        assert!(claim("discovery", "ws", &id).is_ok());
    }

    #[test]
    fn independent_workspaces_and_conversation_types_do_not_block_each_other() {
        let id = otto_core::new_id();
        let _first = claim("discovery", "wa", &id).unwrap();
        let _other_ws = claim("discovery", "wb", &id).unwrap();
        let _refine = claim("refinement", "wa", &id).unwrap();
        assert!(claim("refinement", "wa", &id).is_err());
    }

    #[tokio::test]
    async fn cancelling_a_turn_releases_its_conversation() {
        let id = otto_core::new_id();
        let child_id = id.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = claim("refinement", "ws", &child_id).unwrap();
            tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        rx.await.unwrap();
        assert!(claim("refinement", "ws", &id).is_err());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(claim("refinement", "ws", &id).is_ok());
    }
    #[tokio::test]
    async fn stop_targets_exact_turn_and_waits_for_owner_cleanup() {
        let id = otto_core::new_id();
        let owner = claim_with_id("discovery", "ws", &id, Some("turn-a")).unwrap();
        let signal = owner.cancel();
        assert!(!stop("discovery", "other-ws", &id, "turn-a").await);
        assert!(!stop("discovery", "ws", &id, "stale-turn").await);
        assert!(!signal.is_cancelled());
        let mut stopping = Box::pin(stop("discovery", "ws", &id, "turn-a"));
        tokio::select! {
            _ = &mut stopping => panic!("must drain the owner before responding"),
            _ = signal.cancelled() => {}
        }
        assert!(claim("discovery", "ws", &id).is_err());
        drop(owner);
        assert!(stopping.await);
        let replacement = claim_with_id("discovery", "ws", &id, Some("turn-b")).unwrap();
        assert!(!stop("discovery", "ws", &id, "turn-a").await);
        assert!(!replacement.cancel().is_cancelled());
    }

    #[test]
    fn malformed_turn_identity_is_rejected_before_admission() {
        let id = otto_core::new_id();
        assert!(claim_with_id("refinement", "ws", &id, Some("")).is_err());
        assert!(claim_with_id("refinement", "ws", &id, Some(&"x".repeat(129))).is_err());
        assert!(claim("refinement", "ws", &id).is_ok());
    }
    #[tokio::test]
    async fn disconnected_send_keeps_stop_ownership_until_cleanup_finishes() {
        let id = otto_core::new_id();
        let work_id = id.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (cleanup_tx, cleanup_rx) = tokio::sync::oneshot::channel();
        let caller = tokio::spawn(complete(async move {
            let owner = claim_with_id("discovery", "ws", &work_id, Some("request")).unwrap();
            started_tx.send(()).unwrap();
            owner.cancel().cancelled().await;
            cleanup_rx.await.unwrap();
            drop(owner);
            Ok(())
        }));
        started_rx.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        let mut stopping = Box::pin(stop("discovery", "ws", &id, "request"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut stopping)
                .await
                .is_err(),
            "disconnect must not release admission before cleanup"
        );
        assert!(claim("discovery", "ws", &id).is_err());
        cleanup_tx.send(()).unwrap();
        assert!(stopping.await);
        assert!(claim("discovery", "ws", &id).is_ok());
    }
}
