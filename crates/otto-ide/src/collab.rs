//! "Share with Friends" Live Collaborative IDE & Co-Debugging Hub (`CollabHub`).
//!
//! Enables a workspace host to open a real-time collaborative IDE room backed by a
//! 64-hex capability `invite_token`. Teammates or external friends join via the share
//! link (`#/ide?collab=<room_id>&token=<invite_token>`), receive a distinct presence
//! color and per-peer `peer_secret`, sync multi-cursors & selections, co-edit shared
//! buffers (when granted `IdeCollabRole::Editor`), follow each other's active file/line,
//! and view or co-drive live DAP debug sessions.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::RwLock;

use chrono::Utc;
use otto_core::domain::User;
use otto_core::{new_id, Error, Result};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::types::{
    IdeCollabCursorReq, IdeCollabDebugActionReq, IdeCollabEditReq, IdeCollabEvent,
    IdeCollabJoinReq, IdeCollabJoinResp, IdeCollabPeer, IdeCollabRole, IdeCollabRoom,
    IdeCollabStateResp, IdeCreateCollabRoomReq, IdeCreateCollabRoomResp, IdeDebugSessionSnapshot,
    IdeDebugState, IdeSharedBuffer, IdeTargetSpec,
};

const BROADCAST_CAPACITY: usize = 256;
const MAX_SHARED_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const MAX_PEERS_PER_ROOM: usize = 32;

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

struct PeerRecord {
    peer: IdeCollabPeer,
    secret_hash: [u8; 32],
    host_bound: bool,
}

struct RoomState {
    id: String,
    title: String,
    target: IdeTargetSpec,
    host_user_id: String,
    default_role: IdeCollabRole,
    invite_token_hash: [u8; 32],
    peers: Vec<PeerRecord>,
    shared_buffers: BTreeMap<String, IdeSharedBuffer>,
    active_debug: Option<IdeDebugSessionSnapshot>,
    created_at: String,
    events_tx: broadcast::Sender<IdeCollabEvent>,
}

impl RoomState {
    fn snapshot(&self) -> IdeCollabRoom {
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

    fn verify_invite_token(&self, token: &str) -> Result<()> {
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

    fn find_authenticated_peer_mut(
        &mut self,
        peer_id: &str,
        peer_secret: &str,
    ) -> Result<&mut PeerRecord> {
        let secret = peer_secret.trim();
        if secret.is_empty() {
            return Err(Error::Unauthorized);
        }
        let candidate = sha256_bytes(secret);
        let invite_hash = self.invite_token_hash;
        let rec = self
            .peers
            .iter_mut()
            .find(|p| p.peer.peer_id == peer_id)
            .ok_or(Error::Unauthorized)?;
        let secret_matches = ct_eq_32(&candidate, &rec.secret_hash)
            || (rec.peer.is_host && ct_eq_32(&candidate, &invite_hash));
        if !secret_matches {
            return Err(Error::Unauthorized);
        }
        Ok(rec)
    }
}

/// Central hub managing live IDE collaboration rooms, cursors, buffers, and co-debugging.
pub struct CollabHub {
    rooms: RwLock<HashMap<String, RoomState>>,
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
        }
    }

    pub fn with_data_dir(_data_dir: PathBuf) -> Self {
        Self::new()
    }

    /// Host creates a collaborative room for `req.target` and receives a 64-hex `invite_token`.
    pub fn create_room(
        &self,
        host: &User,
        req: IdeCreateCollabRoomReq,
    ) -> Result<IdeCreateCollabRoomResp> {
        req.target.validate()?;
        let room_id = format!("collab_{}", new_id().to_ascii_lowercase());
        let invite_token = mint_hex_secret_64();
        let invite_token_hash = sha256_bytes(&invite_token);
        let now = now_rfc3339();

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
            .unwrap_or_else(|| format!("{} (Shared)", default_title));

        let host_name = if host.username.trim().is_empty() {
            "Host".to_string()
        } else {
            host.username.clone()
        };

        let host_peer = PeerRecord {
            peer: IdeCollabPeer {
                peer_id: format!("peer_host_{}", &new_id().to_ascii_lowercase()[..10]),
                display_name: host_name.clone(),
                color: PEER_COLORS[0].to_string(),
                role: IdeCollabRole::Editor,
                is_host: true,
                active_path: None,
                active_line: None,
                selections: Vec::new(),
                updated_at: now.clone(),
            },
            secret_hash: invite_token_hash,
            host_bound: false,
        };

        let (events_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let state = RoomState {
            id: room_id.clone(),
            title,
            target: req.target,
            host_user_id: host.id.clone(),
            default_role: req.default_role,
            invite_token_hash,
            peers: vec![host_peer],
            shared_buffers: BTreeMap::new(),
            active_debug: None,
            created_at: now,
            events_tx,
        };

        let snapshot = state.snapshot();
        let share_hash = format!("#/ide?collab={room_id}&token={invite_token}");
        let mut guard = self
            .rooms
            .write()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        guard.insert(room_id, state);

        Ok(IdeCreateCollabRoomResp {
            room: snapshot,
            invite_token,
            share_hash,
        })
    }

    /// List active collaboration rooms visible to `user` (host's own rooms, or all for root).
    pub fn list_rooms(&self, user: &User) -> Result<Vec<IdeCollabRoom>> {
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

        let mut guard = self
            .rooms
            .write()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        let room = guard
            .get_mut(room_id)
            .ok_or_else(|| Error::NotFound(format!("collab room '{room_id}' not found")))?;
        room.verify_invite_token(&req.token)?;

        let peer_secret = mint_hex_secret_64();
        let secret_hash = sha256_bytes(&peer_secret);
        let now = now_rfc3339();

        // If the host's own IDE tab joins with the host's display name for the first time,
        // bind to the seeded host peer so the host stays Editor without duplicating presence.
        if let Some(host_rec) = room.peers.iter_mut().find(|p| {
            p.peer.is_host
                && !p.host_bound
                && (p.peer.display_name.eq_ignore_ascii_case(display_name)
                    || display_name.eq_ignore_ascii_case("host"))
        }) {
            host_rec.secret_hash = secret_hash;
            host_rec.host_bound = true;
            if !display_name.eq_ignore_ascii_case("host") {
                host_rec.peer.display_name = display_name.to_string();
            }
            host_rec.peer.updated_at = now;
            let peer_clone = host_rec.peer.clone();
            let snapshot = room.snapshot();
            let _ = room.events_tx.send(IdeCollabEvent::PeerJoined {
                peer: peer_clone.clone(),
                room: snapshot.clone(),
            });
            return Ok(IdeCollabJoinResp {
                peer_id: peer_clone.peer_id,
                peer_secret,
                color: peer_clone.color,
                room: snapshot,
            });
        }

        if room.peers.len() >= MAX_PEERS_PER_ROOM {
            return Err(Error::Conflict(format!(
                "room has reached the maximum of {MAX_PEERS_PER_ROOM} peers"
            )));
        }

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
            updated_at: now,
        };
        room.peers.push(PeerRecord {
            peer: peer.clone(),
            secret_hash,
            host_bound: false,
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

    /// Fetch current room state, shared buffers, and active debug snapshot for an invited peer.
    pub fn get_state(&self, room_id: &str, token: &str) -> Result<IdeCollabStateResp> {
        let guard = self
            .rooms
            .read()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        let room = guard
            .get(room_id)
            .ok_or_else(|| Error::NotFound(format!("collab room '{room_id}' not found")))?;
        room.verify_invite_token(token)?;
        Ok(IdeCollabStateResp {
            room: room.snapshot(),
            shared_buffers: room.shared_buffers.clone(),
            active_debug: room.active_debug.clone(),
        })
    }

    /// Subscribe to real-time `IdeCollabEvent`s for an invited peer (used by SSE route).
    pub fn subscribe(
        &self,
        room_id: &str,
        token: &str,
    ) -> Result<(IdeCollabRoom, broadcast::Receiver<IdeCollabEvent>)> {
        let guard = self
            .rooms
            .read()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        let room = guard
            .get(room_id)
            .ok_or_else(|| Error::NotFound(format!("collab room '{room_id}' not found")))?;
        room.verify_invite_token(token)?;
        Ok((room.snapshot(), room.events_tx.subscribe()))
    }

    /// Broadcast a peer's live multi-cursor & multi-selection ranges.
    pub fn update_cursor(&self, room_id: &str, req: IdeCollabCursorReq) -> Result<()> {
        let mut guard = self
            .rooms
            .write()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        let room = guard
            .get_mut(room_id)
            .ok_or_else(|| Error::NotFound(format!("collab room '{room_id}' not found")))?;

        let (peer_id, display_name, color) = {
            let rec = room.find_authenticated_peer_mut(&req.peer_id, &req.peer_secret)?;
            rec.peer.active_path = Some(req.path.clone());
            rec.peer.active_line = Some(req.active_line);
            rec.peer.selections = req.selections.clone();
            rec.peer.updated_at = now_rfc3339();
            (
                rec.peer.peer_id.clone(),
                rec.peer.display_name.clone(),
                rec.peer.color.clone(),
            )
        };

        let _ = room.events_tx.send(IdeCollabEvent::CursorMoved {
            peer_id,
            display_name,
            color,
            path: req.path,
            selections: req.selections,
            active_line: req.active_line,
        });
        Ok(())
    }

    /// Verify that `(peer_id, peer_secret)` belongs to `room_id` AND holds `IdeCollabRole::Editor`,
    /// returning the room's `IdeTargetSpec` + `host_user_id` for target saves or co-debugging.
    pub fn authorize_editor(
        &self,
        room_id: &str,
        peer_id: &str,
        peer_secret: &str,
    ) -> Result<(IdeTargetSpec, String)> {
        let mut guard = self
            .rooms
            .write()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        let room = guard
            .get_mut(room_id)
            .ok_or_else(|| Error::NotFound(format!("collab room '{room_id}' not found")))?;
        let target = room.target.clone();
        let host_user_id = room.host_user_id.clone();
        let rec = room.find_authenticated_peer_mut(peer_id, peer_secret)?;
        if rec.peer.role != IdeCollabRole::Editor {
            return Err(Error::Forbidden(
                "viewer peers do not have permission to edit or control debugging".into(),
            ));
        }
        Ok((target, host_user_id))
    }

    /// Apply a buffer edit from an `Editor` peer, updating the room's shared buffer version
    /// and broadcasting `IdeCollabEvent::BufferUpdated`.
    pub fn apply_edit(
        &self,
        room_id: &str,
        req: &IdeCollabEditReq,
        saved_to_target: bool,
    ) -> Result<u64> {
        if req.content.len() > MAX_SHARED_BUFFER_BYTES {
            return Err(Error::PayloadTooLarge(
                "shared buffer exceeds 8 MiB limit".into(),
            ));
        }
        if req.path.trim().is_empty() {
            return Err(Error::Invalid("buffer path must not be empty".into()));
        }

        let mut guard = self
            .rooms
            .write()
            .map_err(|_| Error::Internal("collab lock poisoned".into()))?;
        let room = guard
            .get_mut(room_id)
            .ok_or_else(|| Error::NotFound(format!("collab room '{room_id}' not found")))?;

        {
            let rec = room.find_authenticated_peer_mut(&req.peer_id, &req.peer_secret)?;
            if rec.peer.role != IdeCollabRole::Editor {
                return Err(Error::Forbidden(
                    "viewer peers cannot edit shared buffers".into(),
                ));
            }
            rec.peer.active_path = Some(req.path.clone());
            rec.peer.updated_at = now_rfc3339();
        }

        let next_version = match room.shared_buffers.get(&req.path) {
            Some(existing) => {
                if let Some(expected) = req.base_version {
                    if expected > existing.version {
                        return Err(Error::Conflict(format!(
                            "base_version {expected} is ahead of current version {}",
                            existing.version
                        )));
                    }
                }
                existing.version + 1
            }
            None => 1,
        };

        room.shared_buffers.insert(
            req.path.clone(),
            IdeSharedBuffer {
                content: req.content.clone(),
                version: next_version,
            },
        );

        let _ = room.events_tx.send(IdeCollabEvent::BufferUpdated {
            peer_id: req.peer_id.clone(),
            path: req.path.clone(),
            content: req.content.clone(),
            version: next_version,
            saved: saved_to_target,
        });

        Ok(next_version)
    }

    /// Authorize a co-debugger guest action (`IdeCollabDebugActionReq`), verifying `Editor` role.
    pub fn authorize_debug_action(
        &self,
        room_id: &str,
        req: &IdeCollabDebugActionReq,
    ) -> Result<(IdeTargetSpec, String)> {
        self.authorize_editor(room_id, &req.peer_id, &req.peer_secret)
    }

    /// Publish an updated `IdeDebugSessionSnapshot` to all rooms sharing `target` (or already
    /// tracking `snapshot.id`).
    pub fn publish_debug_snapshot(
        &self,
        target: Option<&IdeTargetSpec>,
        snapshot: &IdeDebugSessionSnapshot,
    ) {
        let Ok(mut guard) = self.rooms.write() else {
            return;
        };
        let target_key = target.map(IdeTargetSpec::project_key);
        for room in guard.values_mut() {
            let matches_target = target_key
                .as_ref()
                .is_some_and(|tk| room.target.project_key() == *tk);
            let matches_session = room
                .active_debug
                .as_ref()
                .is_some_and(|d| d.id == snapshot.id);
            if matches_target || matches_session {
                if snapshot.state == IdeDebugState::Terminated {
                    room.active_debug = None;
                } else {
                    room.active_debug = Some(snapshot.clone());
                }
                let _ = room.events_tx.send(IdeCollabEvent::DebugUpdated {
                    session: snapshot.clone(),
                });
            }
        }
    }
}

fn mint_hex_secret_64() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    hex::encode(bytes)
}

fn sha256_bytes(input: &str) -> [u8; 32] {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn ct_eq_32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (&x, &y)| acc | (x ^ y))
        == 0
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{IdeDebugAction, IdeSelectionRange};
    use chrono::Utc;

    fn test_user(id: &str, username: &str, is_root: bool) -> User {
        User {
            id: id.into(),
            username: username.into(),
            display_name: username.into(),
            is_root,
            disabled: false,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn full_room_lifecycle_roles_cursors_buffers_and_shared_debug() {
        let hub = CollabHub::new();
        let host = test_user("u_host", "itzik", false);
        let stranger = test_user("u_other", "mallory", false);

        let created = hub
            .create_room(
                &host,
                IdeCreateCollabRoomReq {
                    target: IdeTargetSpec::local("/tmp/shared-repo"),
                    title: Some("Pair Debug Go".into()),
                    default_role: IdeCollabRole::Viewer,
                },
            )
            .expect("create room");

        assert_eq!(created.invite_token.len(), 64);
        assert!(created.share_hash.contains(&created.room.id));
        assert!(created.share_hash.contains(&created.invite_token));
        assert_eq!(created.room.peers.len(), 1);
        assert!(created.room.peers[0].is_host);

        // Wrong token rejected on join & state.
        assert!(hub
            .join_room(
                &created.room.id,
                IdeCollabJoinReq {
                    display_name: "Alice".into(),
                    token: "0".repeat(64),
                },
            )
            .is_err());

        let (_, mut rx) = hub
            .subscribe(&created.room.id, &created.invite_token)
            .expect("subscribe");

        // Friend joins as default Viewer.
        let joined = hub
            .join_room(
                &created.room.id,
                IdeCollabJoinReq {
                    display_name: "Alice".into(),
                    token: created.invite_token.clone(),
                },
            )
            .expect("alice joins");
        assert_eq!(joined.peer_secret.len(), 64);
        assert_eq!(joined.room.peers.len(), 2);

        let ev = rx.recv().await.expect("peer joined event");
        assert!(matches!(ev, IdeCollabEvent::PeerJoined { .. }));

        // Multi-cursor sync works for Viewers too!
        hub.update_cursor(
            &created.room.id,
            IdeCollabCursorReq {
                peer_id: joined.peer_id.clone(),
                peer_secret: joined.peer_secret.clone(),
                path: "main.go".into(),
                selections: vec![
                    IdeSelectionRange {
                        anchor: 10,
                        head: 18,
                    },
                    IdeSelectionRange {
                        anchor: 32,
                        head: 40,
                    },
                ],
                active_line: 12,
            },
        )
        .expect("cursor update");
        let ev = rx.recv().await.expect("cursor event");
        assert!(matches!(
            ev,
            IdeCollabEvent::CursorMoved {
                active_line: 12,
                ..
            }
        ));

        // Viewer CANNOT edit buffers or control debugger!
        let edit_req = IdeCollabEditReq {
            peer_id: joined.peer_id.clone(),
            peer_secret: joined.peer_secret.clone(),
            path: "main.go".into(),
            content: "package main\n".into(),
            base_version: None,
            save: false,
        };
        assert!(matches!(
            hub.apply_edit(&created.room.id, &edit_req, false),
            Err(Error::Forbidden(_))
        ));
        assert!(matches!(
            hub.authorize_debug_action(
                &created.room.id,
                &IdeCollabDebugActionReq {
                    peer_id: joined.peer_id.clone(),
                    peer_secret: joined.peer_secret.clone(),
                    debug_id: "dbg_1".into(),
                    action: IdeDebugAction::StepOver,
                    thread_id: None,
                    frame_id: None,
                    path: None,
                    line: None,
                    breakpoints: None,
                }
            ),
            Err(Error::Forbidden(_))
        ));

        // Stranger cannot elevate Alice; Host elevates Alice to Editor.
        assert!(hub
            .set_peer_role(
                &stranger,
                &created.room.id,
                &joined.peer_id,
                IdeCollabRole::Editor
            )
            .is_err());
        let updated_room = hub
            .set_peer_role(
                &host,
                &created.room.id,
                &joined.peer_id,
                IdeCollabRole::Editor,
            )
            .expect("promote alice");
        assert_eq!(
            updated_room
                .peers
                .iter()
                .find(|p| p.peer_id == joined.peer_id)
                .unwrap()
                .role,
            IdeCollabRole::Editor
        );
        let _ = rx.recv().await.expect("role changed event");

        // Now Alice can edit shared buffers and co-debug!
        let v1 = hub
            .apply_edit(&created.room.id, &edit_req, false)
            .expect("editor edit");
        assert_eq!(v1, 1);
        let _ = rx.recv().await.expect("buffer updated event");

        // Shared debug snapshot broadcast updates room state + SSE stream.
        let dbg_snap = IdeDebugSessionSnapshot {
            id: "dbg_99".into(),
            title: "Debug main.go".into(),
            language: "go".into(),
            adapter: "dlv".into(),
            state: IdeDebugState::Paused,
            stopped_reason: Some("breakpoint".into()),
            active_thread_id: Some(1),
            active_frame_id: Some(10),
            paused_file: Some("/tmp/shared-repo/main.go".into()),
            paused_line: Some(8),
            threads: vec![],
            stack_frames: vec![],
            scopes: vec![],
            inline_values: vec![],
            breakpoints: vec![],
            output: vec![],
            error: None,
        };
        hub.publish_debug_snapshot(Some(&IdeTargetSpec::local("/tmp/shared-repo")), &dbg_snap);
        let ev = rx.recv().await.expect("debug updated event");
        assert!(matches!(ev, IdeCollabEvent::DebugUpdated { .. }));

        let state = hub
            .get_state(&created.room.id, &created.invite_token)
            .expect("get state");
        assert_eq!(state.shared_buffers.get("main.go").unwrap().version, 1);
        assert_eq!(state.active_debug.unwrap().id, "dbg_99");

        // Close room broadcasts RoomClosed and revokes access immediately.
        hub.close_room(&host, &created.room.id).expect("close room");
        let ev = rx.recv().await.expect("room closed event");
        assert!(matches!(ev, IdeCollabEvent::RoomClosed { .. }));
        assert!(hub
            .get_state(&created.room.id, &created.invite_token)
            .is_err());
    }
}
