//! Nonblocking ingress bounds. Work and rejection notices have separate limits;
//! when both are full the transport must retain the event for provider retry.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const BUSY_REPLY: &str = "Otto is busy with earlier messages. This message was not started. Please send it again after the current work finishes.";
const TOTAL: usize = 256;
const PER_INTEGRATION: usize = 64;
const PER_CONVERSATION: usize = 8;
const NOTICES: usize = 8;
const CONTROLS: usize = 8;

type Key = (String, String, Option<String>);
#[derive(Default)]
struct Counts {
    total: usize,
    notices: usize,
    controls: usize,
    integrations: HashMap<String, usize>,
    conversations: HashMap<Key, usize>,
}

#[derive(Default, Clone)]
pub struct InboundAdmission(Arc<Mutex<Counts>>);
pub enum Admission {
    Work(WorkPermit),
    Busy(NoticePermit),
    Retry,
}
pub struct WorkPermit {
    counts: Arc<Mutex<Counts>>,
    key: Option<Key>,
}
pub struct NoticePermit(Arc<Mutex<Counts>>);

impl InboundAdmission {
    /// Existing quick commands have their own small reserve: a full turn
    /// queue must not prevent its owner from stopping or detaching that turn.
    pub(crate) fn admit_chat(
        &self,
        integration: &str,
        chat: &str,
        thread: Option<&str>,
        text: &str,
        edited: bool,
    ) -> Admission {
        let command = text.split_whitespace().next().unwrap_or("");
        if !edited
            && matches!(
                command,
                "/help" | "/sessions" | "/stop" | "/new" | "/restart" | "/who"
            )
        {
            let mut counts = self.0.lock().unwrap_or_else(|error| error.into_inner());
            if counts.controls < CONTROLS {
                counts.controls += 1;
                return Admission::Work(WorkPermit {
                    counts: self.0.clone(),
                    key: None,
                });
            }
            if counts.notices < NOTICES {
                counts.notices += 1;
                return Admission::Busy(NoticePermit(self.0.clone()));
            }
            return Admission::Retry;
        }
        self.admit(integration, chat, thread)
    }

    pub fn admit(&self, integration: &str, chat: &str, thread: Option<&str>) -> Admission {
        let mut counts = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let key = (
            integration.to_owned(),
            chat.to_owned(),
            thread.map(str::to_owned),
        );
        if counts.total < TOTAL
            && counts.integrations.get(integration).copied().unwrap_or(0) < PER_INTEGRATION
            && counts.conversations.get(&key).copied().unwrap_or(0) < PER_CONVERSATION
        {
            counts.total += 1;
            *counts
                .integrations
                .entry(integration.to_owned())
                .or_default() += 1;
            *counts.conversations.entry(key.clone()).or_default() += 1;
            Admission::Work(WorkPermit {
                counts: self.0.clone(),
                key: Some(key),
            })
        } else if counts.notices < NOTICES {
            counts.notices += 1;
            Admission::Busy(NoticePermit(self.0.clone()))
        } else {
            Admission::Retry
        }
    }
}

impl Drop for WorkPermit {
    fn drop(&mut self) {
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(key) = &self.key else {
            counts.controls -= 1;
            return;
        };
        counts.total -= 1;
        if let Some(count) = counts.integrations.get_mut(&key.0) {
            *count -= 1;
            if *count == 0 {
                counts.integrations.remove(&key.0);
            }
        }
        if let Some(count) = counts.conversations.get_mut(key) {
            *count -= 1;
            if *count == 0 {
                counts.conversations.remove(key);
            }
        }
    }
}
impl Drop for NoticePermit {
    fn drop(&mut self) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .notices -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injected_handles_share_limits_and_unrelated_fixtures_are_isolated() {
        let webhook = InboundAdmission::default();
        let chat = webhook.clone();
        let unrelated = InboundAdmission::default();
        let held: Vec<_> = (0..PER_CONVERSATION)
            .map(|_| webhook.admit("same-integration", "chat", None))
            .collect();
        assert!(matches!(
            chat.admit("same-integration", "chat", None),
            Admission::Busy(_)
        ));
        assert!(matches!(
            unrelated.admit("same-integration", "chat", None),
            Admission::Work(_)
        ));
        drop(held);
        assert!(matches!(
            chat.admit("same-integration", "chat", None),
            Admission::Work(_)
        ));
    }

    #[test]
    fn backlog_is_bounded_and_rejection_capacity_never_becomes_work() {
        let admission = InboundAdmission::default();
        let work: Vec<_> = (0..PER_CONVERSATION)
            .map(|_| match admission.admit("telegram:w", "chat", None) {
                Admission::Work(permit) => permit,
                _ => panic!("capacity available"),
            })
            .collect();
        let notices: Vec<_> = (0..NOTICES)
            .map(|_| match admission.admit("telegram:w", "chat", None) {
                Admission::Busy(permit) => permit,
                _ => panic!("full conversation must be rejected"),
            })
            .collect();
        assert!(matches!(
            admission.admit("telegram:w", "chat", None),
            Admission::Retry
        ));
        assert!(matches!(
            admission.admit("telegram:w", "other", None),
            Admission::Work(_)
        ));
        drop(notices);
        drop(work);
        assert_eq!(admission.0.lock().unwrap().conversations.len(), 0);
        assert!(matches!(
            admission.admit("telegram:w", "chat", None),
            Admission::Work(_)
        ));
    }

    #[test]
    fn stopping_a_full_conversation_uses_bounded_reserved_capacity() {
        let admission = InboundAdmission::default();
        let held: Vec<_> = (0..PER_CONVERSATION)
            .map(|_| admission.admit("slack:w", "chat", None))
            .collect();
        assert!(matches!(
            admission.admit_chat("slack:w", "chat", None, "ordinary text", false),
            Admission::Busy(_)
        ));
        assert!(
            matches!(
                admission.admit_chat("slack:w", "chat", None, "/stop", true),
                Admission::Busy(_)
            ),
            "an edit must not bypass turn limits"
        );
        assert!(matches!(
            admission.admit_chat("slack:w", "chat", None, "/unknown", false),
            Admission::Busy(_)
        ));
        let controls: Vec<_> = (0..CONTROLS)
            .map(
                |_| match admission.admit_chat("slack:w", "chat", None, "/stop", false) {
                    Admission::Work(permit) => permit,
                    _ => panic!("stop needs reserved capacity"),
                },
            )
            .collect();
        assert!(matches!(
            admission.admit_chat("slack:w", "chat", None, "/stop", false),
            Admission::Busy(_)
        ));
        drop(controls);
        drop(held);
        assert_eq!(admission.0.lock().unwrap().controls, 0);
    }

    #[tokio::test]
    async fn cancelled_work_releases_capacity_and_evicted_identity_counts() {
        let admission = InboundAdmission::default();
        let Admission::Work(permit) = admission.admit("slack:w", "chat", None) else {
            panic!("capacity");
        };
        let task = tokio::spawn(async move {
            let _permit = permit;
            std::future::pending::<()>().await;
        });
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let counts = admission.0.lock().unwrap();
        assert_eq!(counts.total, 0);
        assert!(counts.integrations.is_empty());
        assert!(counts.conversations.is_empty());
    }

    #[test]
    fn many_conversations_share_integration_and_global_limits() {
        let admission = InboundAdmission::default();
        let mut held = Vec::new();
        for integration in 0..4 {
            for chat in 0..PER_INTEGRATION {
                match admission.admit(&integration.to_string(), &chat.to_string(), None) {
                    Admission::Work(permit) => held.push(permit),
                    _ => panic!("within budget"),
                }
            }
            assert!(matches!(
                admission.admit(&integration.to_string(), "overflow", None),
                Admission::Busy(_)
            ));
        }
        assert_eq!(held.len(), TOTAL);
        // High-cardinality overflow cannot grow retained per-identity maps.
        for chat in 0..100_000 {
            assert!(matches!(
                admission.admit("overflow", &chat.to_string(), None),
                Admission::Busy(_)
            ));
        }
        assert_eq!(admission.0.lock().unwrap().conversations.len(), TOTAL);

        assert!(matches!(
            admission.admit("new", "chat", None),
            Admission::Busy(_)
        ));
        drop(held);
        let counts = admission.0.lock().unwrap();
        assert_eq!(counts.total, 0);
        assert!(counts.integrations.is_empty());
        assert!(counts.conversations.is_empty());
    }
}
