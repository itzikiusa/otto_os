use otto_core::{api::RecapState, Error, Result};
use std::collections::BTreeSet;
#[derive(Debug)]
pub(super) struct Consent {
    pub epoch: u64,
    pub state: RecapState,
    pub consented: BTreeSet<String>,
    pub required: BTreeSet<String>,
}
impl Consent {
    pub fn new(required: BTreeSet<String>) -> Self {
        Self {
            epoch: 1,
            state: RecapState::AwaitingConsent,
            consented: BTreeSet::new(),
            required,
        }
    }
    pub fn consent(&mut self, id: &str, epoch: u64, allow: bool) -> Result<()> {
        if epoch != self.epoch || self.state == RecapState::Stopped || !self.required.contains(id) {
            return Err(Error::Forbidden("Capture consent epoch expired".into()));
        }
        if !allow {
            self.interrupt(self.required.clone(), false);
            return Ok(());
        }
        self.consented.insert(id.into());
        if !matches!(self.state, RecapState::Capturing | RecapState::Finalizing) {
            self.state = if self.consented == self.required {
                RecapState::Ready
            } else {
                RecapState::AwaitingConsent
            };
        }
        Ok(())
    }
    pub fn start(&mut self, epoch: u64) -> Result<()> {
        if epoch != self.epoch
            || self.state != RecapState::Ready
            || self.required.is_empty()
            || self.consented != self.required
        {
            return Err(Error::Forbidden(
                "Every connected participant must opt in before the host starts capture".into(),
            ));
        }
        self.state = RecapState::Capturing;
        Ok(())
    }
    pub fn interrupt(&mut self, required: BTreeSet<String>, stop: bool) {
        if self.state == RecapState::Stopped {
            return;
        }
        self.epoch += 1;
        self.required = required;
        self.consented.clear();
        self.state = if stop {
            RecapState::Stopped
        } else {
            RecapState::Paused
        };
    }
    pub fn accepts(&self, epoch: u64) -> bool {
        self.state == RecapState::Capturing && self.epoch == epoch
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_needs_every_current_member_and_explicit_start() {
        let mut c = Consent::new(BTreeSet::from(["host".into(), "guest".into()]));
        assert!(c.start(1).is_err());
        c.consent("host", 1, true).unwrap();
        assert!(!c.accepts(1));
        c.consent("guest", 1, true).unwrap();
        assert_eq!(c.state, RecapState::Ready);
        assert!(!c.accepts(1));
        c.start(1).unwrap();
        assert!(c.accepts(1));
    }
    #[test]
    fn withdrawal_and_new_member_fence_old_uploads_and_never_auto_resume() {
        let mut c = Consent::new(BTreeSet::from(["host".into()]));
        c.consent("host", 1, true).unwrap();
        c.start(1).unwrap();
        c.interrupt(BTreeSet::from(["host".into(), "guest".into()]), false);
        assert!(!c.accepts(1));
        assert_eq!(c.epoch, 2);
        assert!(c.consented.is_empty());
        assert!(c.consent("guest", 1, true).is_err());
        c.consent("host", 2, true).unwrap();
        c.consent("guest", 2, true).unwrap();
        assert!(!c.accepts(2));
        c.start(2).unwrap();
        c.consent("guest", 2, false).unwrap();
        assert_eq!(c.epoch, 3);
        assert!(!c.accepts(2));
    }
    #[test]
    fn stopped_recaps_cannot_restart() {
        let mut c = Consent::new(BTreeSet::from(["host".into()]));
        c.interrupt(c.required.clone(), true);
        assert!(c.consent("host", 2, true).is_err());
        assert!(c.start(2).is_err());
    }
}
