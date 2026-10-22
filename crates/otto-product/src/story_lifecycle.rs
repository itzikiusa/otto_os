//! Admission and cancellation through the final write of a studio turn.
use otto_core::cancel_signal::CancelSignal;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

#[derive(Default)]
struct Story {
    gate: Arc<RwLock<()>>,
    cancel: CancelSignal,
}
fn story(id: &str) -> Arc<Story> {
    static STORIES: OnceLock<Mutex<HashMap<String, Weak<Story>>>> = OnceLock::new();
    let mut stories = STORIES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    stories.retain(|_, s| s.strong_count() > 0);
    if let Some(s) = stories.get(id).and_then(Weak::upgrade) {
        return s;
    }
    let state = Arc::new(Story::default());
    stories.insert(id.to_owned(), Arc::downgrade(&state));
    state
}

pub struct Turn {
    _state: Arc<Story>,
    _read: OwnedRwLockReadGuard<()>,
}
/// Caller must reload the story/thread after claiming, before starting work.
pub async fn claim(id: &str) -> otto_core::Result<Turn> {
    let state = story(id);
    let read = state.gate.clone().read_owned().await;
    if state.cancel.is_cancelled() {
        return Err(otto_core::Error::Conflict("story is being deleted".into()));
    }
    Ok(Turn {
        _state: state,
        _read: read,
    })
}

pub struct Deleting {
    _state: Arc<Story>,
    _write: OwnedRwLockWriteGuard<()>,
}
/// Cancel before waiting: every live host turn observes the signal and stops.
/// Keep this guard until the story deletion/cleanup finishes.
pub async fn begin_delete(id: &str) -> Deleting {
    let state = story(id);
    state.cancel.cancel();
    let write = state.gate.clone().write_owned().await;
    Deleting {
        _state: state,
        _write: write,
    }
}

/// The handler's Turn owns this state until all publication writes complete.
pub fn cancellation(id: &str) -> CancelSignal {
    story(id).cancel.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn deletion_cancels_then_drains_and_blocks_late_admission() {
        let id = otto_core::new_id();
        let turn = claim(&id).await.unwrap();
        let cancel = cancellation(&id);
        let mut deleting = Box::pin(begin_delete(&id));
        tokio::select! {
            _ = &mut deleting => panic!("must wait for final publication"),
            _ = cancel.cancelled() => {}
        }
        drop(turn);
        let deletion = deleting.await;
        let mut late = Box::pin(claim(&id));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut late)
                .await
                .is_err()
        );
        drop(deletion);
        assert!(late.await.is_err());
        // Other stories remain usable throughout; dropped registry entries expire.
        assert!(claim(&otto_core::new_id()).await.is_ok());
    }
    #[tokio::test]
    async fn cancelled_startup_cannot_proceed_as_a_new_turn() {
        let id = otto_core::new_id();
        let turn = claim(&id).await.unwrap();
        let signal = cancellation(&id);
        let task_id = id.clone();
        let deletion = tokio::spawn(async move { begin_delete(&task_id).await });
        signal.cancelled().await;
        assert!(signal.is_cancelled());
        drop(turn);
        drop(deletion.await.unwrap());
    }
}
