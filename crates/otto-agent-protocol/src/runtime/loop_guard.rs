//! A brake on agent-to-agent ping-pong between linked threads.
//!
//! Every message one thread's agent sends another arrives with "answer it
//! with otto_thread_message" (`AgentRuntime::peer_header`, the child wake
//! text), and neither `otto.thread_message` nor `otto.thread_open` needs an
//! approval — so two linked agents could keep each other busy without end.
//! The guard counts, per linked group (the root and every thread opened
//! under it), the prompts an AGENT originated since a person last prompted
//! any member: `message_peer` from an agent, `open_peer` with an opening
//! prompt, and a child's reply queued for the agent that asked (the wake).
//! After [`AGENT_HOPS_MAX`] the next one is refused (`409`, [`PAUSED_ERROR`])
//! and the root's timeline gets ONE `system_notice` ([`PAUSED_NOTICE`]); a
//! held wake waits in the queue with [`PAUSED_QUEUED`] as its `error` (sent
//! when the person retries it). A person's prompt in any member — a typed
//! message, a queued message sent, an "Ask another agent", a person's
//! `…/peers/{peer}/message` — starts the count again. In memory only: a
//! daemon restart also starts it again.

use std::collections::HashMap;
use std::sync::Mutex;

use otto_core::agent_runtime::RuntimeSession;
use otto_core::{Error, Id, Result};

use super::AgentRuntime;

/// Agent-originated prompts a linked group may exchange without a person.
pub const AGENT_HOPS_MAX: u32 = 16;
/// The one notice the root thread's timeline gets when the group pauses.
pub const PAUSED_NOTICE: &str = "Linked threads paused after 16 agent-to-agent messages. \
     Send a message in any of them to continue.";
/// The refusal an agent reads (`409`).
pub const PAUSED_ERROR: &str = "these linked threads have exchanged 16 agent-to-agent messages \
     without the person — stop here and ask the person to continue (their next message in any \
     linked thread resumes)";
/// The `error` of a child's reply held in its parent's queue meanwhile.
pub const PAUSED_QUEUED: &str = "Held: these linked threads exchanged 16 agent-to-agent \
     messages without you. Send it to continue.";

#[derive(Default)]
struct Hops {
    count: u32,
    /// The pause notice is written (once per pause).
    noticed: bool,
}

/// Counts per group root.
#[derive(Default)]
pub(super) struct LoopGuard {
    groups: Mutex<HashMap<Id, Hops>>,
}

/// [`LoopGuard::admit`]'s answer.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Admit {
    Yes,
    /// Refused; `first` = the first refusal of this pause (write the notice).
    No {
        first: bool,
    },
}

impl LoopGuard {
    fn groups(&self) -> std::sync::MutexGuard<'_, HashMap<Id, Hops>> {
        self.groups.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Count one agent-originated prompt in group `root`, or refuse it.
    pub(super) fn admit(&self, root: &Id) -> Admit {
        let mut groups = self.groups();
        let h = groups.entry(root.clone()).or_default();
        if h.count >= AGENT_HOPS_MAX {
            let first = !h.noticed;
            h.noticed = true;
            return Admit::No { first };
        }
        h.count += 1;
        Admit::Yes
    }

    /// Give back a counted prompt that was not delivered (the peer was busy).
    pub(super) fn refund(&self, root: &Id) {
        if let Some(h) = self.groups().get_mut(root) {
            h.count = h.count.saturating_sub(1);
        }
    }

    /// A person prompted a member: the group starts again.
    pub(super) fn reset(&self, root: &Id) {
        self.groups().remove(root);
    }

    #[cfg(test)]
    pub(super) fn count(&self, root: &Id) -> u32 {
        self.groups().get(root).map_or(0, |h| h.count)
    }
}

/// The group key: the root of `s`'s linked group.
pub(super) fn group_root(s: &RuntimeSession) -> Id {
    s.parent_id.clone().unwrap_or_else(|| s.id.clone())
}

impl AgentRuntime {
    /// A person prompted thread `id` (the HTTP routes that require a
    /// person): its linked group's agents may message each other again.
    pub async fn person_prompted(&self, id: &Id) {
        if let Ok(s) = self.get(id).await {
            self.inner.loop_guard.reset(&group_root(&s));
        }
    }

    /// Count one agent-originated prompt sent from or to `s`'s group;
    /// refused (`409`) once the group paused. The first refusal writes the
    /// pause notice in the root's timeline (in `s`'s when the root is closed).
    pub(super) async fn admit_agent_prompt(&self, s: &RuntimeSession) -> Result<()> {
        let root = group_root(s);
        match self.inner.loop_guard.admit(&root) {
            Admit::Yes => Ok(()),
            Admit::No { first } => {
                if first && self.notice(&root, PAUSED_NOTICE).await.is_err() {
                    if let Err(e) = self.notice(&s.id, PAUSED_NOTICE).await {
                        tracing::debug!(session = %s.id, "loop-guard notice skipped: {e}");
                    }
                }
                Err(Error::Conflict(PAUSED_ERROR.into()))
            }
        }
    }

    /// [`Self::admit_agent_prompt`] undone: the prompt was not delivered.
    pub(super) fn refund_agent_prompt(&self, s: &RuntimeSession) {
        self.inner.loop_guard.refund(&group_root(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_pauses_after_the_cap_and_a_person_resumes_it() {
        let g = LoopGuard::default();
        let root: Id = "root".into();
        for _ in 0..AGENT_HOPS_MAX {
            assert_eq!(g.admit(&root), Admit::Yes);
        }
        assert_eq!(g.count(&root), AGENT_HOPS_MAX);
        assert_eq!(g.admit(&root), Admit::No { first: true });
        assert_eq!(g.admit(&root), Admit::No { first: false }, "one notice");
        // Another group is not affected.
        assert_eq!(g.admit(&"other".into()), Admit::Yes);
        g.reset(&root);
        assert_eq!(g.count(&root), 0);
        assert_eq!(g.admit(&root), Admit::Yes);
    }

    #[test]
    fn a_refunded_prompt_does_not_count() {
        let g = LoopGuard::default();
        let root: Id = "root".into();
        assert_eq!(g.admit(&root), Admit::Yes);
        g.refund(&root);
        assert_eq!(g.count(&root), 0);
        g.refund(&root);
        assert_eq!(g.count(&root), 0, "never below zero");
    }
}
