use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GameKind {
    Shooter,
    Kart,
}
/// The one game catalog, shared with the UI chooser (`games/catalog.ts` reads
/// the same file). Adding a map or changing a capacity is one JSON edit.
const CATALOG_JSON: &str = include_str!("../../../../ui/src/modules/rooms/games/catalog.json");
#[derive(Deserialize)]
struct Catalog {
    games: HashMap<String, CatalogEntry>,
}
#[derive(Deserialize)]
struct CatalogEntry {
    capacity: usize,
    maps: Vec<String>,
}
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    // Compiled in and parsed by `catalog_lists_every_game_kind`; a malformed
    // file fails that test, never a request.
    CATALOG.get_or_init(|| serde_json::from_str(CATALOG_JSON).expect("game catalog parses"))
}
impl GameKind {
    /// The catalog key: the same snake_case name the wire uses.
    fn key(self) -> &'static str {
        match self {
            GameKind::Shooter => "shooter",
            GameKind::Kart => "kart",
        }
    }
    fn entry(self) -> &'static CatalogEntry {
        &catalog().games[self.key()]
    }
    pub fn maps(self) -> &'static [String] {
        &self.entry().maps
    }
    /// Active members, host included (the host fills the rest with bots/CPUs).
    pub fn capacity(self) -> usize {
        self.entry().capacity
    }
}
#[cfg(test)]
mod catalog_tests {
    use super::*;
    #[test]
    fn catalog_lists_every_game_kind() {
        let parsed: Catalog = serde_json::from_str(CATALOG_JSON).unwrap();
        assert_eq!(parsed.games.len(), 2, "catalog has a kind the enum lacks");
        for kind in [GameKind::Shooter, GameKind::Kart] {
            // The key is the wire name, so the UI's `config.game` indexes it too.
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.key());
            assert!(!kind.maps().is_empty());
            assert!(kind.capacity() >= 2);
        }
        assert_eq!(GameKind::Shooter.capacity(), 4);
        assert_eq!(GameKind::Kart.capacity(), 8);
        assert!(GameKind::Kart.maps().iter().any(|m| m == "meadow"));
        assert!(GameKind::Shooter.maps().iter().any(|m| m == "deadcity"));
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
