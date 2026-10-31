//! The registry stores hashes only. A per-room mutex serializes admission,
//! revocation and terminal writes; no database operation runs on keystrokes.
use crate::capability::{well_formed, Capability, Digest, TokenBucket};
use chrono::{DateTime, Utc};
use otto_core::{api::*, Error, Result};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex};

pub const ROOM_TTL: Duration = Duration::from_secs(8 * 3600);
pub const INVITE_TTL: Duration = Duration::from_secs(600);
pub const PENDING_TTL: Duration = Duration::from_secs(120);
pub const GRACE: Duration = Duration::from_secs(30);
pub type SharedRoom = Arc<Mutex<Room>>;
#[derive(Default)]
pub struct RoomRegistry {
    pub(crate) recaps: tokio::sync::OnceCell<Arc<crate::recap::archive::Store>>,
    rooms: Mutex<HashMap<String, SharedRoom>>,
}
#[derive(Clone)]
pub enum Notice {
    State,
    Event {
        to: Option<String>,
        event: RoomEvent,
    },
}
/// The rooms relay's throttle message (the shared bucket takes it per call).
pub const RATE_LIMITED: &str = "Too many events; retry shortly";
pub struct Member {
    pub view: RoomMember,
    pub hash: Digest,
    pub joined: Instant,
    pub disconnected: Option<Instant>,
    pub heartbeat: Instant,
    pub present_until: Option<Instant>,
    pub chat_rate: TokenBucket,
    pub action_rate: TokenBucket,
    pub annotation_rate: TokenBucket,
    pub pointer_at: Option<Instant>,
    pub terminal_active: bool,
    pub ice_rate: TokenBucket,
}
impl Member {
    fn new(name: String, role: RoomRole, hash: Digest, admission: RoomAdmission) -> Self {
        Self {
            view: RoomMember {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                role,
                admission,
                connected: false,
                generation: 0,
                control_requested: false,
                audio_joined: false,
                muted: true,
                room_muted: false,
                presenter_requested: false,
                presenter_allowed: role == RoomRole::Host,
            },
            hash,
            joined: Instant::now(),
            disconnected: Some(Instant::now()),
            heartbeat: Instant::now(),
            present_until: None,
            chat_rate: TokenBucket::new(10.0),
            action_rate: TokenBucket::new(80.0),
            annotation_rate: TokenBucket::new(48.0),
            pointer_at: None,
            terminal_active: false,
            ice_rate: TokenBucket::new(2.0),
        }
    }
    pub fn valid(&self) -> bool {
        if self.view.admission == RoomAdmission::Pending {
            self.joined.elapsed() < PENDING_TTL
        } else {
            self.disconnected.is_none_or(|time| time.elapsed() < GRACE)
        }
    }
}
struct Invite {
    role: RoomRole,
    created: Instant,
}
pub struct Room {
    pub recap: Option<crate::recap::Capture>,
    pub id: String,
    pub session: String,
    pub owner: String,
    pub title: String,
    pub provider: String,
    pub spawn_seq: u64,
    pub host: String,
    pub driver: String,
    pub grant_epoch: u64,
    pub created: Instant,
    pub expires: DateTime<Utc>,
    pub members: HashMap<String, Member>,
    invites: HashMap<Digest, Invite>,
    pub messages: VecDeque<RoomMessage>,
    pub next_message: u64,
    pub presentations: Vec<RoomPresentation>,
    pub grants: Vec<RoomAnnotationGrant>,
    pub marks: Vec<RoomAnnotation>,
    pub annotations_enabled: bool,
    pub audio_epoch: u64,
    pub audio_enforced_epoch: u64,
    pub audio_pending_since: Option<Instant>,
    pub signal: broadcast::Sender<Notice>,
    pub ended: bool,
}
pub(crate) fn invalid(s: &str) -> Error {
    Error::Invalid(s.into())
}
pub(crate) fn forbidden(s: &str) -> Error {
    Error::Forbidden(s.into())
}
fn name(s: &str) -> Result<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > 120 || s.chars().any(char::is_control) {
        return Err(invalid(
            "Name must contain 1–120 bytes without control characters",
        ));
    }
    Ok(s.into())
}
impl RoomRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn create(
        &self,
        session: &str,
        owner: &str,
        display_name: &str,
        title: &str,
        provider: &str,
    ) -> Result<RoomCredential> {
        let display_name = name(display_name)?;
        let mut rooms = self.rooms.lock().await;
        // Ended entries remain reachable by existing sockets but no longer own
        // registry capacity. The maintenance worker ends TTL-expired entries.
        rooms.retain(|_, room| room.try_lock().map(|r| !r.ended).unwrap_or(true));
        if rooms.len() >= 32 {
            return Err(Error::Conflict("Maximum active rooms reached".into()));
        }
        for room in rooms.values() {
            if room.lock().await.session == session {
                return Err(Error::Conflict("Session already has a room".into()));
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (token, hash) = Capability::issue();
        let host = Member::new(display_name, RoomRole::Host, hash, RoomAdmission::Admitted);
        let member_id = host.view.id.clone();
        let room = Room {
            recap: None,
            id: id.clone(),
            session: session.into(),
            owner: owner.into(),
            title: title.into(),
            provider: provider.into(),
            spawn_seq: 0,
            host: member_id.clone(),
            driver: member_id.clone(),
            grant_epoch: 0,
            created: Instant::now(),
            expires: Utc::now() + chrono::Duration::hours(8),
            members: HashMap::from([(member_id.clone(), host)]),
            invites: HashMap::new(),
            messages: VecDeque::new(),
            next_message: 0,
            presentations: vec![],
            grants: vec![],
            marks: vec![],
            annotations_enabled: true,
            audio_epoch: 0,
            audio_enforced_epoch: 0,
            audio_pending_since: None,
            signal: broadcast::channel(128).0,
            ended: false,
        };
        rooms.insert(id.clone(), Arc::new(Mutex::new(room)));
        Ok(RoomCredential {
            room_id: id,
            member_id,
            token,
        })
    }
    pub async fn get(&self, id: &str) -> Result<SharedRoom> {
        self.rooms
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or(Error::Unauthorized)
    }
    pub async fn all(&self) -> Vec<SharedRoom> {
        self.rooms.lock().await.values().cloned().collect()
    }
    pub async fn forget(&self, id: &str) {
        self.rooms.lock().await.remove(id);
    }
    pub async fn invite(&self, id: &str, owner: &str, role: RoomRole) -> Result<String> {
        let room = self.get(id).await?;
        let mut room = room.lock().await;
        room.live()?;
        if room.owner != owner {
            return Err(forbidden("Only the session owner can invite"));
        }
        if role == RoomRole::Host {
            return Err(invalid("Invitations cannot grant host"));
        }
        room.invites.retain(|_, i| i.created.elapsed() < INVITE_TTL);
        if room.invites.len() >= 8 {
            return Err(Error::Conflict("Too many outstanding invitations".into()));
        }
        let (token, hash) = Capability::issue();
        room.invites.insert(
            hash,
            Invite {
                role,
                created: Instant::now(),
            },
        );
        Ok(token)
    }
    pub async fn join(&self, id: &str, token: &str, display_name: &str) -> Result<RoomCredential> {
        let display_name = name(display_name)?;
        // Every invite is a 43-char capability; refuse anything else before
        // taking the room lock (the game relay always did).
        if !well_formed(token) {
            return Err(Error::Unauthorized);
        }
        let room = self.get(id).await?;
        let mut room = room.lock().await;
        room.live()?;
        room.members.retain(|_, m| m.valid());
        if room.admitted_count() >= 4 {
            return Err(Error::Conflict("Room is full".into()));
        }
        if room
            .members
            .values()
            .filter(|m| m.view.admission == RoomAdmission::Pending)
            .count()
            >= 8
        {
            return Err(Error::Conflict("Too many pending joins".into()));
        }
        let digest = Digest::of(token);
        let invite = room
            .invites
            .get(&digest)
            .filter(|i| i.created.elapsed() < INVITE_TTL)
            .ok_or(Error::Unauthorized)?;
        let role = invite.role;
        room.invites.remove(&digest);
        let (token, hash) = Capability::issue();
        let member = Member::new(display_name, role, hash, RoomAdmission::Pending);
        let member_id = member.view.id.clone();
        room.members.insert(member_id.clone(), member);
        room.changed();
        Ok(RoomCredential {
            room_id: id.into(),
            member_id,
            token,
        })
    }
    pub async fn authenticate(&self, id: &str, token: &str) -> Result<String> {
        if !well_formed(token) {
            return Err(Error::Unauthorized);
        }
        let room = self.get(id).await?;
        let room = room.lock().await;
        room.live()?;
        // `Digest::matches` is the shared constant-time compare.
        room.members
            .values()
            .find(|m| m.valid() && m.hash.matches(token))
            .map(|m| m.view.id.clone())
            .ok_or(Error::Unauthorized)
    }
    /// An admitted member's shared session id (any role) — the room-capability
    /// HTTP reads (`/room-transcript/…`).
    pub async fn member_session(&self, id: &str, token: &str) -> Result<String> {
        let member = self.authenticate(id, token).await?;
        let room = self.get(id).await?;
        let room = room.lock().await;
        room.admitted(&member)?;
        Ok(room.session.clone())
    }
    /// `(room id, session, member)` for the member who drives the terminal
    /// under grant `epoch` — the room-capability HTTP writes. `room_input`
    /// re-checks the grant and the live process at the actual write.
    pub async fn driver_session(
        &self,
        id: &str,
        token: &str,
        epoch: u64,
    ) -> Result<(String, String, String)> {
        let member = self.authenticate(id, token).await?;
        let room = self.get(id).await?;
        let room = room.lock().await;
        room.admitted(&member)?;
        if room.driver != member || room.grant_epoch != epoch {
            return Err(Error::Forbidden("You don't control the terminal".into()));
        }
        Ok((room.id.clone(), room.session.clone(), member))
    }
    pub async fn admit(&self, id: &str, host: &str, member: &str, role: RoomRole) -> Result<()> {
        let room = self.get(id).await?;
        let mut room = room.lock().await;
        room.admit(host, member, role)?;
        room.changed();
        Ok(())
    }
    pub async fn snapshot(&self, id: &str, member: &str) -> Result<RoomSnapshot> {
        let room = self.get(id).await?;
        let room = room.lock().await;
        room.snapshot(member)
    }
}
impl Room {
    pub fn live(&self) -> Result<()> {
        if self.ended || self.created.elapsed() >= ROOM_TTL {
            Err(Error::Unauthorized)
        } else {
            Ok(())
        }
    }
    pub fn member(&self, id: &str) -> Result<&Member> {
        self.live()?;
        self.members
            .get(id)
            .filter(|m| m.valid())
            .ok_or(Error::Unauthorized)
    }
    pub fn admitted(&self, id: &str) -> Result<&Member> {
        let m = self.member(id)?;
        if m.view.admission != RoomAdmission::Admitted {
            return Err(forbidden("Waiting for admission"));
        }
        Ok(m)
    }
    pub fn host(&self, id: &str) -> Result<()> {
        self.admitted(id)?;
        if self.host != id {
            return Err(forbidden("Only the host can do that"));
        }
        Ok(())
    }
    pub fn connected(&self, id: &str, generation: u64) -> Result<()> {
        let m = self.member(id)?;
        if !m.view.connected || m.view.generation != generation {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
    fn admitted_count(&self) -> usize {
        self.members
            .values()
            .filter(|m| m.view.admission == RoomAdmission::Admitted)
            .count()
    }
    pub fn admit(&mut self, host: &str, id: &str, role: RoomRole) -> Result<()> {
        self.host(host)?;
        let m = self.member(id)?;
        if m.view.admission != RoomAdmission::Pending {
            return Err(invalid("Member is not waiting"));
        }
        if role == RoomRole::Host || (m.view.role == RoomRole::Viewer && role == RoomRole::Editor) {
            return Err(forbidden("Admission cannot exceed invitation access"));
        }
        if self.admitted_count() >= 4 {
            return Err(Error::Conflict("Room is full".into()));
        }
        let m = self.members.get_mut(id).unwrap();
        m.view.admission = RoomAdmission::Admitted;
        m.view.role = role;
        if !m.view.connected {
            m.disconnected = Some(Instant::now());
        }
        self.recap_interrupt("A participant was admitted", false);
        Ok(())
    }
    pub fn changed(&self) {
        let _ = self.signal.send(Notice::State);
    }
    pub fn event(&mut self, to: Option<String>, event: RoomEvent) {
        self.record_event(&event);
        let _ = self.signal.send(Notice::Event { to, event });
    }
    pub fn snapshot(&self, id: &str) -> Result<RoomSnapshot> {
        let member = self.member(id)?;
        let admitted = member.view.admission == RoomAdmission::Admitted;
        Ok(RoomSnapshot {
            recap: if admitted {
                self.recap.as_ref().map(|r| r.view.clone())
            } else {
                None
            },
            room_id: self.id.clone(),
            member_id: id.into(),
            admission: member.view.admission,
            session_id: admitted.then(|| self.session.clone()),
            session_title: admitted.then(|| self.title.clone()),
            provider: admitted.then(|| self.provider.clone()),
            host_member_id: admitted.then(|| self.host.clone()),
            driver_member_id: admitted.then(|| self.driver.clone()),
            grant_epoch: admitted.then_some(self.grant_epoch),
            expires_at: admitted.then(|| self.expires.to_rfc3339()),
            members: admitted.then(|| {
                let mut members: Vec<_> = self
                    .members
                    .values()
                    .filter(|m| {
                        m.valid()
                            && (id == self.host || m.view.admission == RoomAdmission::Admitted)
                    })
                    .map(|m| m.view.clone())
                    .collect();
                members.sort_by(|a, b| a.id.cmp(&b.id));
                members
            }),
            messages: admitted.then(|| self.messages.iter().cloned().collect()),
            presentations: admitted.then(|| self.presentations.clone()),
            annotation_grants: admitted.then(|| self.grants.clone()),
            annotations: admitted.then(|| {
                self.marks
                    .iter()
                    .filter(|m| {
                        DateTime::parse_from_rfc3339(&m.expires_at)
                            .map(|d| d > Utc::now())
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect()
            }),
            annotations_enabled: admitted.then_some(self.annotations_enabled),
            audio_epoch: admitted.then_some(self.audio_epoch),
            audio_enforced_epoch: admitted.then_some(self.audio_enforced_epoch),
        })
    }
    pub fn stop_source(&mut self, id: &str) {
        if let Some(presentation) = self.presentations.iter().find(|p| p.id == id).cloned() {
            self.record(RecapEventData::Presentation {
                operation: "stop".into(),
                presentation,
            });
        }
        self.presentations.retain(|p| p.id != id);
        self.grants.retain(|g| g.source_id != id);
        self.marks.retain(|m| m.source_id != id);
    }
    pub fn audio_changed(&mut self) {
        self.audio_epoch += 1;
        self.audio_pending_since = Some(Instant::now());
    }
    pub fn withdraw_media(&mut self, id: &str) {
        self.audio_changed();
        let sources: Vec<_> = self
            .presentations
            .iter()
            .filter(|p| p.member_id == id)
            .map(|p| p.id.clone())
            .collect();
        for source in sources {
            self.stop_source(&source);
        }
        self.marks.retain(|m| m.member_id != id);
        for grant in &mut self.grants {
            if grant.member_id == id {
                grant.allowed = false;
                grant.requested = false;
                grant.epoch += 1;
            }
        }
        if let Some(member) = self.members.get_mut(id) {
            member.view.control_requested = false;
            member.view.audio_joined = false;
            member.view.muted = true;
            member.view.presenter_allowed = false;
            member.view.presenter_requested = false;
            member.present_until = None;
        }
        if id == self.host {
            self.presentations.clear();
            self.grants.clear();
            self.marks.clear();
            for m in self.members.values_mut() {
                m.view.audio_joined = false;
                m.view.muted = true;
                m.view.presenter_allowed = false;
                m.view.presenter_requested = false;
                m.present_until = None;
            }
        }
    }
    /// Returns whether the manager's driver must be reclaimed before notifying.
    pub fn disconnect(&mut self, id: &str, generation: u64) -> bool {
        if self.connected(id, generation).is_err() {
            return false;
        }
        self.withdraw_media(id);
        let member = self.members.get_mut(id).unwrap();
        member.view.connected = false;
        member.disconnected = Some(Instant::now());
        let reclaim = self.driver == id || id == self.host;
        if reclaim {
            self.driver = self.host.clone();
        }
        if self.members[id].view.admission == RoomAdmission::Admitted {
            self.recap_interrupt("A participant disconnected", false);
        }
        reclaim
    }
    pub fn connect(&mut self, id: &str) -> Result<u64> {
        self.member(id)?;
        self.withdraw_media(id);
        let member = self.members.get_mut(id).unwrap();
        member.view.generation += 1;
        member.terminal_active = false;
        member.view.connected = true;
        member.heartbeat = Instant::now();
        member.disconnected = None;
        let generation = member.view.generation;
        if self.members[id].view.admission == RoomAdmission::Admitted {
            self.recap_interrupt("A participant connected", false);
        }
        Ok(generation)
    }
    pub fn end(&mut self, reason: &str) {
        self.recap_interrupt(reason, true);
        self.ended = true;
        self.invites.clear();
        self.members.clear();
        self.presentations.clear();
        self.grants.clear();
        self.marks.clear();
        self.messages.clear();
        self.event(
            None,
            RoomEvent::Ended {
                reason: reason.into(),
            },
        );
    }
}
