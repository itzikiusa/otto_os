//! Per-session room authority, serialized with final PTY writes.
use dashmap::DashMap;
use otto_core::{Error, Id, Result};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomAuthoritySnapshot {
    pub room_id: String,
    /// None means the host, including takeover from an ordinary local pane.
    pub driver: Option<String>,
    pub epoch: u64,
}

#[derive(Default)]
pub(crate) struct RoomAuthorityMap(DashMap<Id, Arc<Mutex<AuthorityState>>>);
impl RoomAuthorityMap {
    pub fn entry(&self, session: &Id) -> Arc<Mutex<AuthorityState>> {
        self.0.entry(session.clone()).or_default().clone()
    }
}

#[derive(Default)]
pub(crate) struct AuthorityState {
    epoch: u64,
    writer_epoch: Arc<AtomicU64>,
    room: Option<RoomLease>,
}
struct RoomLease {
    room_id: String,
    owner: Id,
    host_member: String,
    spawn_seq: u64,
    driver: Option<String>,
}
impl AuthorityState {
    fn advance(&mut self) -> Result<u64> {
        self.epoch = self
            .epoch
            .checked_add(1)
            .ok_or_else(|| Error::Conflict("terminal control epoch exhausted".into()))?;
        self.writer_epoch.store(self.epoch, Ordering::Release);
        Ok(self.epoch)
    }
    pub fn authorization(&self) -> otto_pty::InputAuthorization {
        otto_pty::InputAuthorization {
            epoch: self.writer_epoch.clone(),
            expected: self.epoch,
        }
    }
    pub fn begin(&mut self, room: &str, owner: &Id, host: &str, spawn: u64) -> Result<()> {
        if self.room.is_some() {
            return Err(Error::Conflict("session already has an active room".into()));
        }
        self.advance()?;
        self.room = Some(RoomLease {
            room_id: room.into(),
            owner: owner.clone(),
            host_member: host.into(),
            spawn_seq: spawn,
            driver: None,
        });
        Ok(())
    }
    pub fn grant(&mut self, room: &str, member: Option<&str>) -> Result<u64> {
        let lease = self
            .room
            .as_ref()
            .filter(|r| r.room_id == room)
            .ok_or_else(|| Error::Forbidden("room authority has ended".into()))?;
        let driver = member
            .filter(|m| *m != lease.host_member)
            .map(str::to_owned);
        self.advance()?;
        if let Some(lease) = self.room.as_mut() {
            lease.driver = driver;
        }
        Ok(self.epoch)
    }
    /// Fence a connection using the authoritative driver, never a stale room snapshot.
    pub fn connect(&mut self, room: &str, member: &str) -> Result<u64> {
        let lease = self
            .room
            .as_ref()
            .filter(|r| r.room_id == room)
            .ok_or_else(|| Error::Forbidden("room authority has ended".into()))?;
        let driver = if member == lease.host_member || lease.driver.as_deref() == Some(member) {
            None
        } else {
            lease.driver.clone()
        };
        self.grant(room, driver.as_deref())
    }

    pub fn check_room(&self, room: &str, member: &str, epoch: u64, spawn: u64) -> Result<()> {
        let lease = self
            .room
            .as_ref()
            .filter(|r| r.room_id == room)
            .ok_or_else(|| Error::Forbidden("room authority has ended".into()))?;
        if lease.spawn_seq != spawn {
            return Err(Error::Conflict("shared process has changed".into()));
        }
        if epoch != self.epoch || member != lease.driver.as_deref().unwrap_or(&lease.host_member) {
            return Err(Error::Forbidden(
                "terminal control changed; request control again".into(),
            ));
        }
        Ok(())
    }
    /// Existing caller authorization is checked before reaching this boundary.
    /// A share principal may carry the owner's ID, so `scoped` cannot be inferred
    /// from user identity. Passive terminal responses never reclaim input.
    pub fn human(&mut self, user: &Id, scoped: bool, passive: bool, spawn: u64) -> Result<()> {
        let Some(lease) = &self.room else {
            return Ok(());
        };
        if scoped || &lease.owner != user {
            return Err(Error::Forbidden(
                "session input is controlled by its room host".into(),
            ));
        }
        if lease.spawn_seq != spawn {
            return Err(Error::Conflict("shared process has changed".into()));
        }
        if lease.driver.is_some() {
            if passive {
                return Err(Error::Forbidden(
                    "another room participant controls input".into(),
                ));
            }
            self.advance()?;
            if let Some(lease) = self.room.as_mut() {
                lease.driver = None;
            }
        }
        Ok(())
    }
    /// Internal trusted automation reclaims input before its write. New process
    /// generations terminate old room authority, never carry guest grants over.
    pub fn automation(&mut self, spawn: u64) -> Result<()> {
        if let Some(lease) = &self.room {
            if lease.spawn_seq != spawn {
                self.advance()?;
                self.room = None;
            } else if lease.driver.is_some() {
                self.advance()?;
                if let Some(lease) = self.room.as_mut() {
                    lease.driver = None;
                }
            }
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Option<RoomAuthoritySnapshot> {
        self.room.as_ref().map(|r| RoomAuthoritySnapshot {
            room_id: r.room_id.clone(),
            driver: r.driver.clone(),
            epoch: self.epoch,
        })
    }
    pub fn end(&mut self, room: &str) {
        if self.room.as_ref().is_some_and(|r| r.room_id == room) {
            self.room = None;
            self.epoch = self.epoch.saturating_add(1);
            self.writer_epoch.store(self.epoch, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn active() -> AuthorityState {
        let mut state = AuthorityState::default();
        state.begin("room", &Id::from("owner"), "host", 7).unwrap();
        state
    }
    #[test]
    fn room_authority_only_current_driver_and_process_can_write() {
        let mut state = active();
        let epoch = state.grant("room", Some("guest")).unwrap();
        assert!(state.check_room("room", "guest", epoch, 7).is_ok());
        assert!(state.check_room("room", "host", epoch, 7).is_err());
        assert!(state.check_room("other", "guest", epoch, 7).is_err());
        assert!(state.check_room("room", "guest", epoch, 8).is_err());
    }
    #[test]
    fn room_authority_regrant_same_member_rejects_old_input() {
        let mut state = active();
        let old = state.grant("room", Some("guest")).unwrap();
        state.grant("room", None).unwrap();
        let new = state.grant("room", Some("guest")).unwrap();
        assert!(new > old);
        assert!(state.check_room("room", "guest", old, 7).is_err());
        assert!(state.check_room("room", "guest", new, 7).is_ok());
    }
    #[test]
    fn room_authority_passive_frames_cannot_reclaim_and_scoped_owner_cannot_write() {
        let mut state = active();
        let epoch = state.grant("room", Some("guest")).unwrap();
        assert!(state.human(&Id::from("owner"), false, true, 7).is_err());
        assert!(state.human(&Id::from("owner"), true, false, 7).is_err());
        assert!(state.human(&Id::from("admin"), false, false, 7).is_err());
        assert_eq!(state.snapshot().unwrap().epoch, epoch);
        state.human(&Id::from("owner"), false, false, 7).unwrap();
        let snapshot = state.snapshot().unwrap();
        assert!(snapshot.epoch > epoch);
        assert_eq!(snapshot.driver, None);
        assert!(state.check_room("room", "host", snapshot.epoch, 7).is_ok());
    }
    #[test]
    fn room_authority_end_revokes_old_members_and_new_room_has_new_epoch() {
        let mut state = active();
        let epoch = state.grant("room", Some("guest")).unwrap();
        state.end("unrelated");
        assert!(state.check_room("room", "guest", epoch, 7).is_ok());
        state.end("room");
        assert!(state.check_room("room", "guest", epoch, 7).is_err());
        state.begin("next", &Id::from("owner"), "host", 7).unwrap();
        assert!(state.snapshot().unwrap().epoch > epoch);
    }
    #[test]
    fn unrelated_reconnect_never_restores_a_guest_after_host_takeover() {
        let mut state = active();
        state.grant("room", Some("guest")).unwrap();
        state.human(&Id::from("owner"), false, false, 7).unwrap();
        let reclaimed = state.snapshot().unwrap().epoch;
        let epoch = state.connect("room", "viewer").unwrap();
        assert!(epoch > reclaimed);
        assert_eq!(state.snapshot().unwrap().driver, None);
        assert!(state.check_room("room", "guest", epoch, 7).is_err());
        state.grant("room", Some("guest")).unwrap();
        state.connect("room", "viewer").unwrap();
        assert_eq!(state.snapshot().unwrap().driver.as_deref(), Some("guest"));
        state.connect("room", "guest").unwrap();
        assert_eq!(state.snapshot().unwrap().driver, None);
    }

    #[tokio::test]
    async fn room_authority_revoke_waits_for_final_enqueue_permit() {
        let map = RoomAuthorityMap::default();
        let lock = map.entry(&Id::from("session"));
        let mut held = lock.clone().lock_owned().await;
        held.begin("room", &Id::from("owner"), "host", 7).unwrap();
        let epoch = held.grant("room", Some("guest")).unwrap();
        held.check_room("room", "guest", epoch, 7).unwrap();
        let task = tokio::spawn(async move { lock.lock().await.grant("room", None) });
        tokio::task::yield_now().await;
        assert!(
            !task.is_finished(),
            "revoke acknowledged before final enqueue authorization"
        );
        drop(held);
        assert!(task.await.unwrap().unwrap() > epoch);
    }
}
