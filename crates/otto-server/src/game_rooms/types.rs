use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GameKind {
    Shooter,
    Kart,
}
impl GameKind {
    pub fn maps(self) -> &'static [&'static str] {
        match self {
            GameKind::Shooter => &["isle", "mesa", "deadcity"],
            GameKind::Kart => &[
                "meadow", "downtown", "canyon", "frost", "lagoon", "volcano", "skyway",
            ],
        }
    }
    /// Active members, host included (the host fills the rest with bots/CPUs).
    pub fn capacity(self) -> usize {
        match self {
            GameKind::Shooter => 4,
            GameKind::Kart => 8,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub game: GameKind,
    pub map: String,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Host,
    Guest,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Lobby,
    Playing,
    Finished,
    Closed,
}
#[derive(Serialize)]
pub struct Credential {
    pub room_id: String,
    pub member_id: String,
    pub role: Role,
    pub token: String,
    pub config: Config,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_url: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub role: Role,
    /// Index in the room's member list; stable for the room's lifetime.
    pub slot: usize,
    pub connected: bool,
    pub ready: bool,
    /// A guest who left or outlived the reconnect grace; never reconnects.
    pub left: bool,
    /// A guest asked the host for a rematch.
    pub rematch: bool,
}
#[derive(Clone, Serialize)]
pub struct View {
    pub id: String,
    pub config: Config,
    pub round: u64,
    pub generation: u64,
    pub phase: Phase,
    pub paused: bool,
    pub members: Vec<MemberView>,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Ready {
        ready: bool,
    },
    Start {
        round: u64,
        generation: u64,
    },
    Snapshot {
        round: u64,
        generation: u64,
        seq: u64,
        data: Value,
    },
    Input {
        round: u64,
        generation: u64,
        seq: u64,
        data: Value,
    },
    Finish {
        round: u64,
        generation: u64,
        result: Value,
    },
    Rematch,
    Leave,
    Ping,
}
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    State {
        room: View,
    },
    Snapshot {
        round: u64,
        generation: u64,
        seq: u64,
        data: Value,
    },
    Input {
        round: u64,
        generation: u64,
        seq: u64,
        slot: usize,
        data: Value,
    },
    Finished {
        round: u64,
        generation: u64,
        result: Value,
    },
    Error {
        message: String,
    },
    Closed {
        reason: String,
    },
    Pong,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub name: String,
    pub config: Config,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Join {
    pub room_id: String,
    pub invite: String,
    pub name: String,
}
