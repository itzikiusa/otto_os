//! The take-over / hand-back lock between a human viewer and an agent driving
//! one live session. Pure state (no I/O) — the session wraps it in a mutex and
//! broadcasts a `state` frame whenever a transition reports a change.
//!
//! Rules (contract: api.md "Control lock"):
//! - a human's first input while nobody drives makes them the driver;
//! - `take_over` always succeeds for a human (an agent that was driving is
//!   preempted and remembered as *waiting*);
//! - `hand_back` returns control to a waiting agent, else to nobody;
//! - human input while the agent drives is refused (`NotDriver`);
//! - an agent asking to act while a human drives is paused (`AgentPaused`) and
//!   marked waiting, so the human's hand-back resumes it.

use super::types::ControllerKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Driver {
    Human(String),
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlError {
    /// Human input while the agent drives — take over first.
    NotDriver,
    /// The agent may not act while a human drives.
    AgentPaused,
}

#[derive(Debug, Clone, Default)]
pub struct ControlLock {
    holder: Option<Driver>,
    agent_waiting: bool,
}

impl ControlLock {
    pub fn kind(&self) -> ControllerKind {
        match self.holder {
            None => ControllerKind::None,
            Some(Driver::Human(_)) => ControllerKind::Human,
            Some(Driver::Agent) => ControllerKind::Agent,
        }
    }

    pub fn human_user(&self) -> Option<&str> {
        match &self.holder {
            Some(Driver::Human(u)) => Some(u.as_str()),
            _ => None,
        }
    }

    pub fn agent_waiting(&self) -> bool {
        self.agent_waiting
    }

    /// A human input frame. `Ok(true)` when it changed who drives.
    pub fn human_input(&mut self, user: &str) -> Result<bool, ControlError> {
        match &self.holder {
            Some(Driver::Agent) => Err(ControlError::NotDriver),
            Some(Driver::Human(u)) if u == user => Ok(false),
            _ => {
                self.holder = Some(Driver::Human(user.to_string()));
                Ok(true)
            }
        }
    }

    /// Always succeeds; returns whether anything changed.
    pub fn take_over(&mut self, user: &str) -> bool {
        match &self.holder {
            Some(Driver::Human(u)) if u == user => false,
            Some(Driver::Agent) => {
                self.agent_waiting = true;
                self.holder = Some(Driver::Human(user.to_string()));
                true
            }
            _ => {
                self.holder = Some(Driver::Human(user.to_string()));
                true
            }
        }
    }

    /// A human releases control: to the waiting agent, else to nobody.
    /// No-op (false) unless a human holds it.
    pub fn hand_back(&mut self) -> bool {
        if !matches!(self.holder, Some(Driver::Human(_))) {
            return false;
        }
        if self.agent_waiting {
            self.agent_waiting = false;
            self.holder = Some(Driver::Agent);
        } else {
            self.holder = None;
        }
        true
    }

    /// The last viewer of `user` left: release their control like a hand-back.
    pub fn human_left(&mut self, user: &str) -> bool {
        if self.human_user() == Some(user) {
            self.hand_back()
        } else {
            false
        }
    }

    /// The agent wants to act. `Ok(true)` when it just became the driver.
    pub fn agent_acquire(&mut self) -> Result<bool, ControlError> {
        match self.holder {
            Some(Driver::Human(_)) => {
                self.agent_waiting = true;
                Err(ControlError::AgentPaused)
            }
            Some(Driver::Agent) => Ok(false),
            None => {
                self.holder = Some(Driver::Agent);
                Ok(true)
            }
        }
    }

    /// The agent is done. Clears a pending wait too.
    pub fn agent_release(&mut self) -> bool {
        self.agent_waiting = false;
        if matches!(self.holder, Some(Driver::Agent)) {
            self.holder = None;
            true
        } else {
            false
        }
    }

    pub fn agent_drives(&self) -> bool {
        matches!(self.holder, Some(Driver::Agent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_human_input_claims_an_idle_session() {
        let mut l = ControlLock::default();
        assert_eq!(l.kind(), ControllerKind::None);
        assert_eq!(l.human_input("u1"), Ok(true));
        assert_eq!(l.human_input("u1"), Ok(false));
        assert_eq!(l.kind(), ControllerKind::Human);
        // A second human (ws admin) typing simply becomes the driver.
        assert_eq!(l.human_input("u2"), Ok(true));
        assert_eq!(l.human_user(), Some("u2"));
    }

    #[test]
    fn human_input_is_refused_while_the_agent_drives_until_take_over() {
        let mut l = ControlLock::default();
        assert_eq!(l.agent_acquire(), Ok(true));
        assert_eq!(l.human_input("u1"), Err(ControlError::NotDriver));
        assert!(l.take_over("u1"));
        assert!(l.agent_waiting());
        assert_eq!(l.human_input("u1"), Ok(false));
        // The agent is paused while the human drives.
        assert_eq!(l.agent_acquire(), Err(ControlError::AgentPaused));
        assert!(!l.agent_drives());
        // Hand back resumes the waiting agent.
        assert!(l.hand_back());
        assert!(l.agent_drives());
        assert!(!l.agent_waiting());
    }

    #[test]
    fn hand_back_without_a_waiting_agent_frees_the_session() {
        let mut l = ControlLock::default();
        l.take_over("u1");
        assert!(l.hand_back());
        assert_eq!(l.kind(), ControllerKind::None);
        assert!(!l.hand_back(), "nothing to hand back");
    }

    #[test]
    fn a_departing_driver_releases_only_their_own_control() {
        let mut l = ControlLock::default();
        l.take_over("u1");
        assert!(!l.human_left("u2"));
        assert!(l.human_left("u1"));
        assert_eq!(l.kind(), ControllerKind::None);
    }

    #[test]
    fn agent_release_clears_the_wait() {
        let mut l = ControlLock::default();
        l.take_over("u1");
        let _ = l.agent_acquire();
        assert!(l.agent_waiting());
        assert!(!l.agent_release());
        assert!(!l.agent_waiting());
        assert!(l.hand_back());
        assert_eq!(l.kind(), ControllerKind::None);
    }
}
