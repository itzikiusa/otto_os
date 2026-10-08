//! Admission ownership for one conversation's complete provider turn.
use otto_core::cancel_signal::{InFlightGuard, InFlightSet};
use otto_core::{Error, Result};
use std::sync::OnceLock;

pub(crate) fn claim(kind: &str, workspace: &str, conversation: &str) -> Result<InFlightGuard> {
    static TURNS: OnceLock<InFlightSet> = OnceLock::new();
    let key = format!("{kind}:{workspace}:{conversation}");
    TURNS
        .get_or_init(Default::default)
        .claim(&key)
        .ok_or_else(|| Error::Conflict("a turn is already running in this conversation".into()))
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
}
