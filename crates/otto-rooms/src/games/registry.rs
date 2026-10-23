//! Bounded memory-only relay. All membership and packet decisions share a room lock.
use super::types::*;
use crate::capability::{well_formed, Capability, Digest, TokenBucket};
use otto_core::{Error, Result};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex};

pub const GRACE: Duration = Duration::from_secs(20);
/// Member entries ever admitted to one room, including guests who left.
pub const MAX_MEMBER_ENTRIES: usize = 16;
pub type SharedRoom = Arc<Mutex<Room>>;
#[derive(Default)]
pub struct Registry {
    rooms: Mutex<HashMap<String, SharedRoom>>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    All,
    One(usize),
    /// Every member except the host (snapshot fan-out).
    Guests,
}
impl Target {
    pub fn includes(self, index: usize) -> bool {
        match self {
            Target::All => true,
            Target::One(member) => member == index,
            Target::Guests => index != 0,
        }
    }
}
#[derive(Clone)]
pub struct Notice {
    pub to: Target,
    pub event: Event,
}
pub struct Member {
    pub view: MemberView,
    hash: Digest,
    connection: u64,
    disconnected: Option<Instant>,
    seq: u64,
    stream_rate: TokenBucket,
    control_rate: TokenBucket,
}
pub struct Room {
    pub id: String,
    owner: String,
    pub config: Config,
    created: Instant,
    invite: Option<Digest>,
    pub members: Vec<Member>,
    pub round: u64,
    pub generation: u64,
    pub phase: Phase,
    pub tx: broadcast::Sender<Notice>,
    result: Option<serde_json::Value>,
}
/// The game relay's throttle message; `socket.rs` closes the socket on it.
pub const RATE_LIMITED: &str = "Too many game messages";
fn name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 40 || value.chars().any(char::is_control) {
        return Err(Error::Invalid("Enter a name of 1–40 characters".into()));
    }
    Ok(value.into())
}
fn conflict(message: &str) -> Error {
    Error::Conflict(message.into())
}
impl Member {
    fn new(name: String, role: Role, slot: usize, hash: Digest) -> Self {
        Self {
            view: MemberView {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                role,
                slot,
                connected: false,
                ready: false,
                left: false,
                rematch: false,
            },
            hash,
            connection: 0,
            disconnected: Some(Instant::now()),
            seq: 0,
            stream_rate: TokenBucket::new(90.0),
            control_rate: TokenBucket::new(15.0),
        }
    }
    fn active(&self) -> bool {
        !self.view.left
    }
}
impl Registry {
    pub async fn create(&self, owner: &str, display: &str, config: Config) -> Result<Credential> {
        let name = name(display)?;
        if !config.game.maps().contains(&config.map) {
            return Err(Error::Invalid("Choose a map for this game".into()));
        }
        let mut rooms = self.rooms.lock().await;
        let mut active = 0;
        // Close/evict expired entries before applying capacity, including when idle.
        let mut remove = vec![];
        for (id, shared) in rooms.iter() {
            let mut room = shared.lock().await;
            room.maintain(Instant::now());
            if room.phase == Phase::Closed {
                remove.push(id.clone());
            } else if room.owner == owner {
                active += 1;
            }
        }
        for id in remove {
            rooms.remove(&id);
        }
        if rooms.len() >= 32 || active >= 2 {
            return Err(conflict("Game room limit reached; close an existing game"));
        }
        let (credential, credential_hash) = Capability::issue();
        let (invite, invite_hash) = Capability::issue();
        let id = uuid::Uuid::new_v4().to_string();
        let host = Member::new(name, Role::Host, 0, credential_hash);
        let response = Credential {
            room_id: id.clone(),
            member_id: host.view.id.clone(),
            role: Role::Host,
            token: credential,
            config: config.clone(),
            invite: Some(invite.clone()),
            invite_url: None,
        };
        rooms.insert(
            id.clone(),
            Arc::new(Mutex::new(Room {
                id,
                owner: owner.into(),
                config,
                created: Instant::now(),
                invite: Some(invite_hash),
                members: vec![host],
                round: 1,
                generation: 0,
                phase: Phase::Lobby,
                tx: broadcast::channel(256).0,
                result: None,
            })),
        );
        Ok(response)
    }
    pub async fn get(&self, id: &str) -> Result<SharedRoom> {
        self.rooms
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound("Game room not found".into()))
    }
    pub async fn forget(&self, id: &str) {
        self.rooms.lock().await.remove(id);
    }
    /// The invite admits guests until the room is full or the match starts.
    pub async fn join(&self, id: &str, invite: &str, display: &str) -> Result<Credential> {
        let name = name(display)?;
        if !well_formed(invite) {
            return Err(Error::Unauthorized);
        }
        let shared = self.get(id).await?;
        let mut room = shared.lock().await;
        room.live()?;
        // `Digest::matches` is the shared constant-time compare (this was a
        // short-circuiting `==` on the array before the relays shared it).
        let invited = room.invite.is_some_and(|digest| digest.matches(invite));
        if !invited || room.created.elapsed() > Duration::from_secs(600) {
            return Err(Error::Unauthorized);
        }
        let active = room.members.iter().filter(|m| m.active()).count();
        if active >= room.config.game.capacity()
            || room.members.len() >= MAX_MEMBER_ENTRIES
            || room.phase != Phase::Lobby
        {
            return Err(conflict("Game room is full"));
        }
        let (credential, credential_hash) = Capability::issue();
        let guest = Member::new(name, Role::Guest, room.members.len(), credential_hash);
        let response = Credential {
            room_id: id.into(),
            member_id: guest.view.id.clone(),
            role: Role::Guest,
            token: credential,
            config: room.config.clone(),
            invite: None,
            invite_url: None,
        };
        room.members.push(guest);
        room.changed();
        Ok(response)
    }
    pub async fn authenticate(&self, id: &str, credential: &str) -> Result<usize> {
        if !well_formed(credential) {
            return Err(Error::Unauthorized);
        }
        let shared = self.get(id).await?;
        let mut room = shared.lock().await;
        room.live()?;
        room.members
            .iter()
            .position(|member| member.active() && member.hash.matches(credential))
            .ok_or(Error::Unauthorized)
    }
}
impl Room {
    fn live(&mut self) -> Result<()> {
        self.maintain(Instant::now());
        if self.phase == Phase::Closed {
            return Err(conflict("Game room closed"));
        }
        Ok(())
    }
    fn active(&self) -> impl Iterator<Item = &Member> {
        self.members.iter().filter(|m| m.active())
    }
    pub fn paused(&self) -> bool {
        self.phase == Phase::Playing && !self.all_connected()
    }
    fn all_connected(&self) -> bool {
        self.active().all(|m| m.view.connected)
    }
    pub fn view(&self) -> View {
        View {
            id: self.id.clone(),
            config: self.config.clone(),
            round: self.round,
            generation: self.generation,
            phase: self.phase,
            paused: self.paused(),
            members: self.members.iter().map(|m| m.view.clone()).collect(),
        }
    }
    pub fn emit(&self, to: Target, event: Event) {
        let _ = self.tx.send(Notice { to, event });
    }
    fn changed(&self) {
        self.emit(Target::All, Event::State { room: self.view() });
    }
    fn advance(&mut self) {
        self.generation += 1;
        for member in &mut self.members {
            member.seq = 0;
        }
    }
    pub fn connect(&mut self, index: usize) -> Result<u64> {
        self.live()?;
        let member = self.members.get_mut(index).ok_or(Error::Unauthorized)?;
        if member.view.left {
            return Err(Error::Unauthorized);
        }
        if member.view.connected {
            return Err(conflict("This player is already connected"));
        }
        member.connection += 1;
        member.view.connected = true;
        member.disconnected = None;
        let connection = member.connection;
        self.advance();
        self.changed();
        if let Some(result) = &self.result {
            self.emit(
                Target::One(index),
                Event::Finished {
                    round: self.round,
                    generation: self.generation,
                    result: result.clone(),
                },
            );
        }
        Ok(connection)
    }
    pub fn disconnect(&mut self, index: usize, connection: u64) {
        let Some(member) = self.members.get_mut(index) else {
            return;
        };
        if member.connection != connection || !member.view.connected {
            return;
        }
        member.view.connected = false;
        member.view.ready = false;
        member.disconnected = Some(Instant::now());
        self.advance();
        self.changed();
    }
    /// A guest leaves for good; the room carries on without them.
    fn depart(&mut self, index: usize) {
        let member = &mut self.members[index];
        member.view.left = true;
        member.view.connected = false;
        member.view.ready = false;
        member.view.rematch = false;
        member.disconnected = None;
        self.advance();
        self.changed();
    }
    pub fn maintain(&mut self, now: Instant) {
        if self.phase == Phase::Closed {
            return;
        }
        if now.duration_since(self.created) > Duration::from_secs(3600) {
            return self.close("Game room expired");
        }
        let expired = |m: &Member| {
            m.active()
                && m.disconnected
                    .is_some_and(|at| now.duration_since(at) > GRACE)
        };
        if self.members.first().is_some_and(expired) {
            return self.close("The host disconnected; create a new game");
        }
        let gone: Vec<usize> = (1..self.members.len())
            .filter(|&i| expired(&self.members[i]))
            .collect();
        for index in gone {
            self.depart(index);
        }
    }
    pub fn close(&mut self, reason: &str) {
        if self.phase == Phase::Closed {
            return;
        }
        self.phase = Phase::Closed;
        self.invite = None;
        self.emit(
            Target::All,
            Event::Closed {
                reason: reason.into(),
            },
        );
        self.changed();
    }
    fn epoch(&self, round: u64, generation: u64) -> Result<()> {
        if round != self.round || generation != self.generation {
            return Err(conflict("Stale game round or connection"));
        }
        Ok(())
    }
    fn playing(&self) -> Result<()> {
        if self.phase != Phase::Playing || !self.all_connected() {
            return Err(conflict("Game is not running"));
        }
        Ok(())
    }
    fn host(index: usize) -> Result<()> {
        if index != 0 {
            return Err(Error::Forbidden(
                "Only the host may control game state".into(),
            ));
        }
        Ok(())
    }
    pub fn apply(&mut self, index: usize, connection: u64, command: Command) -> Result<()> {
        self.live()?;
        let member = self.members.get_mut(index).ok_or(Error::Unauthorized)?;
        if member.view.left || !member.view.connected || member.connection != connection {
            return Err(Error::Unauthorized);
        }
        if matches!(command, Command::Snapshot { .. } | Command::Input { .. }) {
            member.stream_rate.take(60.0, 90.0, RATE_LIMITED)?;
        } else {
            member.control_rate.take(5.0, 15.0, RATE_LIMITED)?;
        }
        match command {
            Command::Ping => self.emit(Target::One(index), Event::Pong),
            Command::Leave if index == 0 => self.close("The host ended the game"),
            Command::Leave => {
                self.emit(
                    Target::One(index),
                    Event::Closed {
                        reason: "You left the game".into(),
                    },
                );
                self.depart(index);
            }
            Command::Ready { ready } => {
                if self.phase != Phase::Lobby {
                    return Err(conflict("Ready is only available in the lobby"));
                }
                self.members[index].view.ready = ready;
                self.changed();
            }
            Command::Start { round, generation } => {
                Self::host(index)?;
                self.epoch(round, generation)?;
                if self.phase != Phase::Lobby
                    || self.active().count() < 2
                    || !self.active().all(|m| m.view.connected && m.view.ready)
                {
                    return Err(conflict("Every player must be connected and ready"));
                }
                self.phase = Phase::Playing;
                self.changed();
            }
            Command::Snapshot {
                round,
                generation,
                seq,
                data,
            } => {
                Self::host(index)?;
                self.packet(index, round, generation, seq, &data, 16384)?;
                let event = Event::Snapshot {
                    round,
                    generation,
                    seq,
                    data,
                };
                self.emit(Target::Guests, event);
            }
            Command::Input {
                round,
                generation,
                seq,
                data,
            } => {
                if index == 0 {
                    return Err(Error::Forbidden("Only guests relay input".into()));
                }
                self.packet(index, round, generation, seq, &data, 1024)?;
                let event = Event::Input {
                    round,
                    generation,
                    seq,
                    slot: index,
                    data,
                };
                self.emit(Target::One(0), event);
            }
            Command::Finish {
                round,
                generation,
                result,
            } => {
                Self::host(index)?;
                self.epoch(round, generation)?;
                self.playing()?;
                bounded(&result, 4096)?;
                self.phase = Phase::Finished;
                self.result = Some(result.clone());
                self.changed();
                let event = Event::Finished {
                    round,
                    generation,
                    result,
                };
                self.emit(Target::All, event);
            }
            Command::Rematch => {
                if self.phase != Phase::Finished {
                    return Err(conflict("Finish the game before requesting a rematch"));
                }
                if index != 0 {
                    // Guests ask; the host decides when the next round starts.
                    self.members[index].view.rematch = true;
                    self.changed();
                    return Ok(());
                }
                self.round += 1;
                self.advance();
                self.phase = Phase::Lobby;
                self.result = None;
                for member in &mut self.members {
                    member.view.rematch = false;
                    member.view.ready = false;
                }
                self.changed();
            }
        }
        Ok(())
    }
    fn packet(
        &mut self,
        index: usize,
        round: u64,
        generation: u64,
        seq: u64,
        data: &serde_json::Value,
        max: usize,
    ) -> Result<()> {
        self.epoch(round, generation)?;
        self.playing()?;
        bounded(data, max)?;
        if !data.is_object() || seq <= self.members[index].seq || seq > 9_007_199_254_740_991 {
            return Err(Error::Invalid("Invalid game payload or sequence".into()));
        }
        self.members[index].seq = seq;
        Ok(())
    }
}
fn bounded(value: &serde_json::Value, max: usize) -> Result<()> {
    if serde_json::to_vec(value)
        .map_err(|e| Error::Invalid(e.to_string()))?
        .len()
        > max
    {
        return Err(Error::Invalid("Game payload is too large".into()));
    }
    Ok(())
}
