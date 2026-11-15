//! "Share with Friends" Live Collaborative IDE & Co-Debugging Hub (`CollabHub`).
//!
//! Enables a workspace host to open a real-time collaborative IDE room backed by a
//! 64-hex capability `invite_token`. Teammates or external friends join via the share
//! link (`#/ide?collab=<room_id>&token=<invite_token>`), receive a distinct presence
//! color and per-peer `peer_secret`, sync multi-cursors & selections, co-edit shared
//! buffers (when granted `IdeCollabRole::Editor`), follow each other's active file/line,
//! and view or co-drive live DAP debug sessions.

mod ops;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

use otto_core::domain::User;
use otto_core::{new_id, Error, Result};
use tokio::sync::broadcast;

use self::ops::{ct_eq_32, mint_hex_secret_64, now_rfc3339, sha256_bytes};
use crate::target::ResolvedTarget;
use crate::types::{
    IdeCollabEvent, IdeCollabJoinReq, IdeCollabJoinResp, IdeCollabPeer, IdeCollabRole,
    IdeCollabRoom, IdeCreateCollabRoomReq, IdeCreateCollabRoomResp, IdeDebugSessionSnapshot,
    IdeSharedBuffer, IdeTargetSpec,
};

const BROADCAST_CAPACITY: usize = 256;
pub(crate) const MAX_COLLAB_ROOMS: usize = 32;
pub(crate) const ROOM_IDLE_TTL: Duration = Duration::from_secs(2 * 3600);
pub(crate) const MAX_SHARED_BUFFERS_PER_ROOM: usize = 32;
pub(crate) const MAX_ROOM_BUFFER_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_SHARED_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const MAX_PEERS_PER_ROOM: usize = 32;
pub(super) const MAX_SELECTIONS_PER_PEER: usize = 64;

const PEER_COLORS: &[&str] = &[
    "hsl(212, 88%, 56%)",
    "hsl(152, 68%, 42%)",
    "hsl(28, 90%, 54%)",
    "hsl(275, 76%, 60%)",
    "hsl(342, 82%, 58%)",
    "hsl(186, 78%, 42%)",
    "hsl(46, 92%, 46%)",
    "hsl(12, 84%, 56%)",
];

pub(super) struct PeerRecord {
    pub(super) peer: IdeCollabPeer,
    pub(super) secret_hash: [u8; 32],
}

pub(super) struct SharedBufferRecord {
    pub(super) buf: IdeSharedBuffer,
    pub(super) clean: bool,
    pub(super) touched_seq: u64,
}

pub(super) struct RoomState {
    pub(super) id: String,
    pub(super) created_seq: u64,
    pub(super) title: String,
    pub(super) target: IdeTargetSpec,
    pub(super) resolved_target: Option<ResolvedTarget>,
    pub(super) host_user_id: String,
    pub(super) default_role: IdeCollabRole,
    pub(super) invite_token_hash: [u8; 32],
    pub(super) peers: Vec<PeerRecord>,
    pub(super) shared_buffers: BTreeMap<String, SharedBufferRecord>,
    pub(super) buffer_seq: u64,
    pub(super) active_debug: Option<IdeDebugSessionSnapshot>,
    pub(super) created_at: String,
    pub(super) last_activity_at: Instant,
    pub(super) events_tx: broadcast::Sender<IdeCollabEvent>,
}

impl RoomState {
    pub(super) fn snapshot(&self) -> IdeCollabRoom {
        IdeCollabRoom {
            id: self.id.clone(),
            title: self.title.clone(),
            target: self.target.clone(),
            host_user_id: self.host_user_id.clone(),
            default_role: self.default_role,
            peers: self.peers.iter().map(|p| p.peer.clone()).collect(),
            active_debug_id: self.active_debug.as_ref().map(|d| d.id.clone()),
            created_at: self.created_at.clone(),
        }
    }

    fn total_buffer_bytes(&self) -> usize {
        self.shared_buffers
            .values()
            .map(|r| r.buf.content.len())
            .sum()
    }

    pub(super) fn enforce_buffer_caps(&mut self, keep_path: &str) {
        while self.shared_buffers.len() > MAX_SHARED_BUFFERS_PER_ROOM
            || self.total_buffer_bytes() > MAX_ROOM_BUFFER_BYTES
        {
            let candidate = self
                .shared_buffers
                .iter()
                .filter(|(p, r)| p.as_str() != keep_path && r.clean)
                .min_by_key(|(_, r)| r.touched_seq)
                .or_else(|| {
                    self.shared_buffers
                        .iter()
                        .filter(|(p, _)| p.as_str() != keep_path)
                        .min_by_key(|(_, r)| r.touched_seq)
                })
                .map(|(p, _)| p.clone());
            match candidate {
                Some(evict_path) => {
                    self.shared_buffers.remove(&evict_path);
                }
                None => break,
            }
        }
    }

    pub(super) fn verify_invite_token(&self, token: &str) -> Result<()> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(Error::Unauthorized);
        }
        let candidate = sha256_bytes(trimmed);
        if !ct_eq_32(&candidate, &self.invite_token_hash) {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    pub(super) fn find_authenticated_peer_mut(
        &mut self,
        peer_id: &str,
        peer_secret: &str,
    ) -> Result<&mut PeerRecord> {
        let secret = peer_secret.trim();
        if secret.is_empty() {
            return Err(Error::Unauthorized);
        }
        let candidate = sha256_bytes(secret);
        let rec = self
            .peers
            .iter_mut()
            .find(|p| p.peer.peer_id == peer_id)
            .ok_or(Error::Unauthorized)?;
        if !ct_eq_32(&candidate, &rec.secret_hash) {
            return Err(Error::Unauthorized);
        }
        Ok(rec)
    }
}

/// Central hub managing live IDE collaboration rooms, cursors, buffers, and co-debugging.
pub struct CollabHub {
    pub(super) rooms: RwLock<HashMap<String, RoomState>>,
    room_seq: AtomicU64,
}

impl Default for CollabHub {
    fn default() -> Self {
        Self::new()
    }
}

impl CollabHub {
    pub fn new() -> Self {
        Self {
            rooms: RwLock::new(HashMap::new()),
            room_seq: AtomicU64::new(0),
        }
    }

    pub(super) fn evict_expired_locked(rooms: &mut HashMap<String, RoomState>, now: Instant) {
        let expired: Vec<String> = rooms
            .iter()
            .filter(|(_, r)| now.saturating_duration_since(r.last_activity_at) >= ROOM_IDLE_TTL)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some(removed) = rooms.remove(&id) {
                let _ = removed
                    .events_tx
                    .send(IdeCollabEvent::RoomClosed { room_id: id });
            }
        }
    }

    /// Host creates a collaborative room for `req.target` and receives a 64-hex `invite_token`.
    pub fn create_room(
        &self,
        host: &User,
        req: IdeCreateCollabRoomReq,
    ) -> Result<IdeCreateCollabRoomResp> {
        self.create_room_with_resolved(host, req, None, None)
    }

    /// Host creates a collaborative room with its pre-authorized [`ResolvedTarget`] (enabling
    /// remote SFTP writes for `Editor` guests on SSH targets) and optional initial debug session.
    pub fn create_room_with_resolved(
        &self,
        host: &User,
        req: IdeCreateCollabRoomReq,
        resolved_target: Option<ResolvedTarget>,
        initial_debug: Option<IdeDebugSessionSnapshot>,
    ) -> Result<IdeCreateCollabRoomResp> {
        req.target.validate()?;
        let room_id = format!("collab_{}", new_id().to_ascii_lowercase());
        let invite_token = mint_hex_secret_64();
        let invite_token_hash = sha256_bytes(&invite_token);
        let host_peer_secret = mint_hex_secret_64();
        let host_peer_secret_hash = sha256_bytes(&host_peer_secret);
        let now = now_rfc3339();
        let now_inst = Instant::now();
        let created_seq = self.room_seq.fetch_add(1, Ordering::Relaxed);

        let default_title = req
            .target
            .root
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("Workspace");
        let title = req
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{default_title} (Shared)"));

        let host_name = if host.username.trim().is_empty() {
            "Host".to_string()
        } else {
            host.username.clone()
        };

        let host_peer_id = format!("peer_host_{}", &new_id().to_ascii_lowercase()[..10]);
        let host_peer = PeerRecord {
            peer: IdeCollabPeer {
                peer_id: host_peer_id.clone(),
                display_name: host_name,
                color: PEER_COLORS[0].to_string(),
                role: IdeCollabRole::Editor,
                is_host: true,
                active_path: None,
                active_line: None,
                selections: Vec::new(),
                updated_at: now.clone(),
            },
            secret_hash: host_peer_secret_hash,
        };

        let (events_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let state = RoomState {
            id: room_id.clone(),
            created_seq,
            title,
            target: req.target,
            resolved_target,
            host_user_id: host.id.clone(),
            default_role: req.default_role,
            invite_token_hash,
            peers: vec![host_peer],
            shared_buffers: BTreeMap::new(),
            buffer_seq: 0,
            active_debug: initial_debug,
            created_at: now,
            last_activity_at: now_inst,
            events_tx,
        };

        let snapshot = state.snapshot();
        let share_hash = format!("#/ide?collab={room_id}&token={invite_token}");
        let mut guard = self
            .rooms
            .write()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        Self::evict_expired_locked(&mut guard, now_inst);
        while guard.len() >= MAX_COLLAB_ROOMS {
            let Some(oldest_id) = guard
                .iter()
                .min_by_key(|(_, r)| (r.last_activity_at, r.created_seq))
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            if let Some(removed) = guard.remove(&oldest_id) {
                let _ = removed
                    .events_tx
                    .send(IdeCollabEvent::RoomClosed { room_id: oldest_id });
            }
        }
        guard.insert(room_id, state);

        Ok(IdeCreateCollabRoomResp {
            room: snapshot,
            invite_token,
            share_hash,
            host_peer_id,
            host_peer_secret,
        })
    }

    /// List active collaboration rooms visible to `user` (host's own rooms, or all for root).
    pub fn list_rooms(&self, user: &User) -> Result<Vec<IdeCollabRoom>> {
        let now_inst = Instant::now();
        {
            let guard = self
                .rooms
                .read()
                .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
            if guard
                .values()
                .any(|r| now_inst.saturating_duration_since(r.last_activity_at) >= ROOM_IDLE_TTL)
            {
                drop(guard);
                let mut wguard = self
                    .rooms
                    .write()
                    .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
                Self::evict_expired_locked(&mut wguard, now_inst);
            }
        }
        let guard = self
            .rooms
            .read()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        let mut out: Vec<IdeCollabRoom> = guard
            .values()
            .filter(|r| user.is_root || r.host_user_id == user.id)
            .map(RoomState::snapshot)
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    /// Host updates a connected peer's live role (`viewer` ⇄ `editor`).
    pub fn set_peer_role(
        &self,
        user: &User,
        room_id: &str,
        peer_id: &str,
        role: IdeCollabRole,
    ) -> Result<IdeCollabRoom> {
        let mut guard = self
            .rooms
            .write()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        let room = guard
            .get_mut(room_id)
            .ok_or_else(|| Error::NotFound(format!("collab room '{room_id}' not found")))?;
        if !user.is_root && room.host_user_id != user.id {
            return Err(Error::Forbidden(
                "only the room host may change peer permissions".into(),
            ));
        }

        let peer_rec = room
            .peers
            .iter_mut()
            .find(|p| p.peer.peer_id == peer_id)
            .ok_or_else(|| Error::NotFound(format!("peer '{peer_id}' not found in room")))?;
        if peer_rec.peer.is_host && role != IdeCollabRole::Editor {
            return Err(Error::Invalid("cannot demote the room host".into()));
        }
        peer_rec.peer.role = role;
        peer_rec.peer.updated_at = now_rfc3339();
        room.last_activity_at = Instant::now();

        let snapshot = room.snapshot();
        let _ = room.events_tx.send(IdeCollabEvent::RoleChanged {
            peer_id: peer_id.to_string(),
            role,
            room: snapshot.clone(),
        });
        Ok(snapshot)
    }

    /// Host closes/revokes a collaboration room immediately.
    pub fn close_room(&self, user: &User, room_id: &str) -> Result<()> {
        let mut guard = self
            .rooms
            .write()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        let room = guard
            .get(room_id)
            .ok_or_else(|| Error::NotFound(format!("collab room '{room_id}' not found")))?;
        if !user.is_root && room.host_user_id != user.id {
            return Err(Error::Forbidden(
                "only the room host may close this collaboration room".into(),
            ));
        }
        if let Some(removed) = guard.remove(room_id) {
            let _ = removed.events_tx.send(IdeCollabEvent::RoomClosed {
                room_id: room_id.to_string(),
            });
        }
        Ok(())
    }

    /// Guest (or host browser tab) joins a room using its capability `token`.
    pub fn join_room(&self, room_id: &str, req: IdeCollabJoinReq) -> Result<IdeCollabJoinResp> {
        let display_name = req.display_name.trim();
        if display_name.is_empty() {
            return Err(Error::Invalid("display_name must not be empty".into()));
        }
        if display_name.chars().count() > 64 {
            return Err(Error::Invalid(
                "display_name must be 64 characters or fewer".into(),
            ));
        }

        let now_inst = Instant::now();
        let mut guard = self
            .rooms
            .write()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        Self::evict_expired_locked(&mut guard, now_inst);
        let room = guard
            .get_mut(room_id)
            .ok_or_else(|| Error::NotFound(format!("collab room '{room_id}' not found")))?;
        room.verify_invite_token(&req.token)?;
        room.last_activity_at = now_inst;

        if room.peers.len() >= MAX_PEERS_PER_ROOM {
            return Err(Error::Conflict(format!(
                "room has reached the maximum of {MAX_PEERS_PER_ROOM} peers"
            )));
        }

        let peer_secret = mint_hex_secret_64();
        let secret_hash = sha256_bytes(&peer_secret);
        let color = PEER_COLORS[room.peers.len() % PEER_COLORS.len()].to_string();
        let peer_id = format!("peer_{}", &new_id().to_ascii_lowercase()[..12]);
        let peer = IdeCollabPeer {
            peer_id: peer_id.clone(),
            display_name: display_name.to_string(),
            color: color.clone(),
            role: room.default_role,
            is_host: false,
            active_path: None,
            active_line: None,
            selections: Vec::new(),
            updated_at: now_rfc3339(),
        };
        room.peers.push(PeerRecord {
            peer: peer.clone(),
            secret_hash,
        });

        let snapshot = room.snapshot();
        let _ = room.events_tx.send(IdeCollabEvent::PeerJoined {
            peer,
            room: snapshot.clone(),
        });

        Ok(IdeCollabJoinResp {
            peer_id,
            peer_secret,
            color,
            room: snapshot,
        })
    }
}
