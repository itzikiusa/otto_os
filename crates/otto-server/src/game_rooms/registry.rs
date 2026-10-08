//! Bounded memory-only relay. All membership and packet decisions share a room lock.
use super::types::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use otto_core::{Error, Result};
use rand::{rand_core::UnwrapErr, rngs::SysRng, Rng};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex};

pub(super) const GRACE: Duration = Duration::from_secs(20);
pub(super) type SharedRoom = Arc<Mutex<Room>>;
#[derive(Default)]
pub struct Registry {
    rooms: Mutex<HashMap<String, SharedRoom>>,
}
#[derive(Clone)]
pub(super) struct Notice {
    pub to: Option<usize>,
    pub event: Event,
}
pub(super) struct Member {
    pub view: MemberView,
    hash: [u8; 32],
    connection: u64,
    disconnected: Option<Instant>,
    seq: u64,
    rematch: bool,
    stream_rate: Rate,
    control_rate: Rate,
}
pub(super) struct Room {
    pub id: String,
    owner: String,
    pub config: Config,
    created: Instant,
    invite: Option<[u8; 32]>,
    pub members: Vec<Member>,
    pub round: u64,
    pub generation: u64,
    pub phase: Phase,
    pub tx: broadcast::Sender<Notice>,
    result: Option<serde_json::Value>,
}
struct Rate {
    tokens: f64,
    at: Instant,
}
impl Rate {
    fn new(tokens: f64) -> Self {
        Self {
            tokens,
            at: Instant::now(),
        }
    }
    fn take(&mut self, rate: f64, burst: f64) -> Result<()> {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.at).as_secs_f64() * rate).min(burst);
        self.at = now;
        if self.tokens < 1.0 {
            return Err(Error::Conflict("Too many game messages".into()));
        }
        self.tokens -= 1.0;
        Ok(())
    }
}
fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}
fn token() -> String {
    let mut bytes = [0u8; 32];
    UnwrapErr(SysRng).fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
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
    fn new(name: String, role: Role, token: &str) -> Self {
        Self {
            view: MemberView {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                role,
                connected: false,
                ready: false,
            },
            hash: hash(token),
            connection: 0,
            disconnected: Some(Instant::now()),
            seq: 0,
            rematch: false,
            stream_rate: Rate::new(90.0),
            control_rate: Rate::new(15.0),
        }
    }
}
impl Registry {
    pub(super) async fn create(
        &self,
        owner: &str,
        display: &str,
        config: Config,
    ) -> Result<Credential> {
        let name = name(display)?;
        let valid = match config.game {
            GameKind::Shooter => ["station", "foundry", "dunes"],
            GameKind::Kart => ["coast", "forest", "neon"],
        };
        if !valid.contains(&config.map.as_str()) {
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
        let credential = token();
        let invite = token();
        let id = uuid::Uuid::new_v4().to_string();
        let host = Member::new(name, Role::Host, &credential);
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
                invite: Some(hash(&invite)),
                members: vec![host],
                round: 1,
                generation: 0,
                phase: Phase::Lobby,
                tx: broadcast::channel(64).0,
                result: None,
            })),
        );
        Ok(response)
    }
    pub(super) async fn get(&self, id: &str) -> Result<SharedRoom> {
        self.rooms
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound("Game room not found".into()))
    }
    pub(super) async fn forget(&self, id: &str) {
        self.rooms.lock().await.remove(id);
    }
    pub(super) async fn join(&self, id: &str, invite: &str, display: &str) -> Result<Credential> {
        let name = name(display)?;
        if invite.len() != 43 {
            return Err(Error::Unauthorized);
        }
        let shared = self.get(id).await?;
        let mut room = shared.lock().await;
        room.live()?;
        if room.invite != Some(hash(invite)) || room.created.elapsed() > Duration::from_secs(600) {
            return Err(Error::Unauthorized);
        }
        if room.members.len() != 1 || room.phase != Phase::Lobby {
            return Err(conflict("Game room is full"));
        }
        let credential = token();
        let guest = Member::new(name, Role::Guest, &credential);
        let response = Credential {
            room_id: id.into(),
            member_id: guest.view.id.clone(),
            role: Role::Guest,
            token: credential,
            config: room.config.clone(),
            invite: None,
            invite_url: None,
        };
        room.invite = None;
        room.members.push(guest);
        room.changed();
        Ok(response)
    }
    pub(super) async fn authenticate(&self, id: &str, credential: &str) -> Result<usize> {
        if credential.len() != 43 {
            return Err(Error::Unauthorized);
        }
        let shared = self.get(id).await?;
        let mut room = shared.lock().await;
        room.live()?;
        room.members
            .iter()
            .position(|member| member.hash == hash(credential))
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
    pub(super) fn paused(&self) -> bool {
        self.phase == Phase::Playing && !self.both_connected()
    }
    fn both_connected(&self) -> bool {
        self.members.len() == 2 && self.members.iter().all(|m| m.view.connected)
    }
    pub(super) fn view(&self) -> View {
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
    pub(super) fn emit(&self, to: Option<usize>, event: Event) {
        let _ = self.tx.send(Notice { to, event });
    }
    fn changed(&self) {
        self.emit(None, Event::State { room: self.view() });
    }
    fn advance(&mut self) {
        self.generation += 1;
        for member in &mut self.members {
            member.seq = 0;
        }
    }
    pub(super) fn connect(&mut self, index: usize) -> Result<u64> {
        self.live()?;
        let member = self.members.get_mut(index).ok_or(Error::Unauthorized)?;
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
                Some(index),
                Event::Finished {
                    round: self.round,
                    generation: self.generation,
                    result: result.clone(),
                },
            );
        }
        Ok(connection)
    }
    pub(super) fn disconnect(&mut self, index: usize, connection: u64) {
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
    pub(super) fn maintain(&mut self, now: Instant) {
        if self.phase == Phase::Closed {
            return;
        }
        if now.duration_since(self.created) > Duration::from_secs(3600) {
            self.close("Game room expired");
        } else if self.members.iter().any(|m| {
            m.disconnected
                .is_some_and(|at| now.duration_since(at) > GRACE)
        }) {
            self.close("A player disconnected; create a new game");
        }
    }
    pub(super) fn close(&mut self, reason: &str) {
        if self.phase == Phase::Closed {
            return;
        }
        self.phase = Phase::Closed;
        self.invite = None;
        self.emit(
            None,
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
        if self.phase != Phase::Playing || !self.both_connected() {
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
    pub(super) fn apply(&mut self, index: usize, connection: u64, command: Command) -> Result<()> {
        self.live()?;
        let member = self.members.get_mut(index).ok_or(Error::Unauthorized)?;
        if !member.view.connected || member.connection != connection {
            return Err(Error::Unauthorized);
        }
        if matches!(command, Command::Snapshot { .. } | Command::Input { .. }) {
            member.stream_rate.take(60.0, 90.0)?;
        } else {
            member.control_rate.take(5.0, 15.0)?;
        }
        match command {
            Command::Ping => self.emit(Some(index), Event::Pong),
            Command::Leave => self.close("A player left the game"),
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
                    || !self.both_connected()
                    || !self.members.iter().all(|m| m.view.ready)
                {
                    return Err(conflict("Both players must be connected and ready"));
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
                self.emit(
                    Some(1),
                    Event::Snapshot {
                        round,
                        generation,
                        seq,
                        data,
                    },
                );
            }
            Command::Input {
                round,
                generation,
                seq,
                data,
            } => {
                if index != 1 {
                    return Err(Error::Forbidden("Only the guest may relay input".into()));
                }
                self.packet(index, round, generation, seq, &data, 1024)?;
                self.emit(
                    Some(0),
                    Event::Input {
                        round,
                        generation,
                        seq,
                        data,
                    },
                );
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
                self.emit(
                    None,
                    Event::Finished {
                        round,
                        generation,
                        result,
                    },
                );
            }
            Command::Rematch => {
                if self.phase != Phase::Finished || !self.both_connected() {
                    return Err(conflict("Finish the game before requesting a rematch"));
                }
                self.members[index].rematch = true;
                if self.members.iter().all(|m| m.rematch) {
                    self.round += 1;
                    self.advance();
                    self.phase = Phase::Lobby;
                    self.result = None;
                    for member in &mut self.members {
                        member.rematch = false;
                        member.view.ready = false;
                    }
                    self.changed();
                }
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
