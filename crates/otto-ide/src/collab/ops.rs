//! Cursor broadcasting, shared buffer co-editing, target write resolution,
//! and owner-scoped co-debugging operations for [`super::CollabHub`].

use std::time::Instant;

use chrono::Utc;
use otto_core::{Error, Result};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::{CollabHub, SharedBufferRecord, MAX_SELECTIONS_PER_PEER, MAX_SHARED_BUFFER_BYTES};
use crate::target::ResolvedTarget;
use crate::types::{
    IdeCollabCursorReq, IdeCollabDebugActionReq, IdeCollabEditReq, IdeCollabEvent, IdeCollabRole,
    IdeCollabRoom, IdeCollabStateResp, IdeDebugSessionSnapshot, IdeDebugState, IdeSharedBuffer,
    IdeTargetSpec,
};

impl CollabHub {
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

    /// Return the host-authorized [`ResolvedTarget`] bound to `room_id`, if available.
    pub fn room_resolved_target(&self, room_id: &str) -> Option<ResolvedTarget> {
        let guard = self.rooms.read().ok()?;
        guard.get(room_id)?.resolved_target.clone()
    }

    /// Broadcast a peer's live multi-cursor & multi-selection ranges without touching `shared_buffers`.
    pub fn update_cursor(&self, room_id: &str, mut req: IdeCollabCursorReq) -> Result<()> {
        if req.selections.len() > MAX_SELECTIONS_PER_PEER {
            req.selections.truncate(MAX_SELECTIONS_PER_PEER);
        }
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
        room.last_activity_at = Instant::now();

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

    /// Pre-validate buffer size, path, `Editor` role, and `base_version` equality before
    /// touching the host or remote filesystem so rejected edits never overwrite target files.
    pub fn validate_edit_precheck(
        &self,
        room_id: &str,
        req: &IdeCollabEditReq,
    ) -> Result<(IdeTargetSpec, String)> {
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
        let target = room.target.clone();
        let host_user_id = room.host_user_id.clone();
        {
            let rec = room.find_authenticated_peer_mut(&req.peer_id, &req.peer_secret)?;
            if rec.peer.role != IdeCollabRole::Editor {
                return Err(Error::Forbidden(
                    "viewer peers cannot edit shared buffers".into(),
                ));
            }
        }
        Self::check_base_version(room.shared_buffers.get(&req.path), req.base_version)?;
        room.last_activity_at = Instant::now();
        Ok((target, host_user_id))
    }

    fn check_base_version(
        existing: Option<&SharedBufferRecord>,
        base_version: Option<u64>,
    ) -> Result<u64> {
        match existing {
            Some(rec) => {
                if let Some(expected) = base_version {
                    if expected != rec.buf.version {
                        return Err(Error::Conflict(format!(
                            "base_version {expected} does not match current version {}",
                            rec.buf.version
                        )));
                    }
                }
                Ok(rec.buf.version + 1)
            }
            None => {
                if let Some(expected) = base_version.filter(|&v| v != 0) {
                    return Err(Error::Conflict(format!(
                        "base_version {expected} does not match uninitialized buffer"
                    )));
                }
                Ok(1)
            }
        }
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
        room.last_activity_at = Instant::now();

        let next_version =
            Self::check_base_version(room.shared_buffers.get(&req.path), req.base_version)?;

        room.buffer_seq = room.buffer_seq.saturating_add(1);
        let touched_seq = room.buffer_seq;
        room.shared_buffers.insert(
            req.path.clone(),
            SharedBufferRecord {
                buf: IdeSharedBuffer {
                    content: req.content.clone(),
                    version: next_version,
                },
                clean: saved_to_target,
                touched_seq,
            },
        );
        room.enforce_buffer_caps(&req.path);

        let _ = room.events_tx.send(IdeCollabEvent::BufferUpdated {
            peer_id: req.peer_id.clone(),
            path: req.path.clone(),
            content: req.content.clone(),
            version: next_version,
            saved: saved_to_target,
        });

        Ok(next_version)
    }

    /// Authorize a co-debugger guest action (`IdeCollabDebugActionReq`), verifying `Editor` role
    /// and that `req.debug_id` matches the room's active debug session.
    pub fn authorize_debug_action(
        &self,
        room_id: &str,
        req: &IdeCollabDebugActionReq,
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
        let matches_room_debug = room
            .active_debug
            .as_ref()
            .is_some_and(|d| d.id == req.debug_id);
        let rec = room.find_authenticated_peer_mut(&req.peer_id, &req.peer_secret)?;
        if rec.peer.role != IdeCollabRole::Editor {
            return Err(Error::Forbidden(
                "viewer peers do not have permission to edit or control debugging".into(),
            ));
        }
        if !matches_room_debug {
            return Err(Error::Forbidden(
                "debug session is not active in this collaboration room".into(),
            ));
        }
        room.last_activity_at = Instant::now();
        Ok((target, host_user_id))
    }

    /// Publish an updated `IdeDebugSessionSnapshot` ONLY to collaboration rooms owned by
    /// `owner_user_id` on `target` (or already bound to `snapshot.id`), preventing another
    /// user on the same repository path from injecting or clobbering debug sessions.
    pub fn publish_debug_snapshot(
        &self,
        target: Option<&IdeTargetSpec>,
        owner_user_id: Option<&str>,
        snapshot: &IdeDebugSessionSnapshot,
    ) {
        let Ok(mut guard) = self.rooms.write() else {
            return;
        };
        let target_key = target.map(IdeTargetSpec::project_key);
        for room in guard.values_mut() {
            let matches_session = room
                .active_debug
                .as_ref()
                .is_some_and(|d| d.id == snapshot.id);
            let matches_owner_and_target = target_key
                .as_ref()
                .is_some_and(|tk| room.target.project_key() == *tk)
                && owner_user_id.is_some_and(|uid| room.host_user_id.as_str() == uid);
            if matches_session || matches_owner_and_target {
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
        let shared_buffers = room
            .shared_buffers
            .iter()
            .map(|(k, v)| (k.clone(), v.buf.clone()))
            .collect();
        Ok(IdeCollabStateResp {
            room: room.snapshot(),
            shared_buffers,
            active_debug: room.active_debug.clone(),
        })
    }
}

pub(super) fn mint_hex_secret_64() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    hex::encode(bytes)
}

pub(super) fn sha256_bytes(input: &str) -> [u8; 32] {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(super) fn ct_eq_32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (&x, &y)| acc | (x ^ y))
        == 0
}

pub(super) fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}
