//! What the rooms domain needs from its host. otto-server implements this for
//! the daemon context (`otto-server/src/rooms/ctx.rs`); tests use fakes.
use crate::{games, RoomRegistry};
use otto_core::{secrets::SecretStore, Result};
use std::{future::Future, path::Path, sync::Arc};

/// The session manager's room authority for one session: which room owns it,
/// who drives the terminal (`None` = the host) and the grant epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    pub room_id: String,
    pub driver: Option<String>,
    pub epoch: u64,
}

/// One audit row. Membership/permission decisions only — never chat, SDP,
/// media or credentials.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomAudit {
    pub user_id: Option<String>,
    pub action: String,
    pub target: String,
    pub detail: Option<serde_json::Value>,
}

pub trait RoomsCtx: Clone + Send + Sync + 'static {
    fn rooms(&self) -> &RoomRegistry;
    fn game_rooms(&self) -> &games::Registry;
    /// The daemon data directory; recap archives live under `room-recaps/`.
    fn data_dir(&self) -> &Path;
    /// This daemon's loopback base URL: the invite origin when no public
    /// room origin is configured.
    fn base_url(&self) -> &str;
    /// Keychain-backed secrets (the TURN shared secret).
    fn secrets(&self) -> &Arc<dyn SecretStore>;
    /// A global setting (`room_ice`, `room_public_origin`, `room_recap_engine`).
    fn setting(&self, key: &str) -> impl Future<Output = Result<Option<serde_json::Value>>> + Send;
    fn put_setting(
        &self,
        key: &str,
        value: &serde_json::Value,
    ) -> impl Future<Output = Result<()>> + Send;
    /// Best-effort audit insert (a failure is logged by the host, not raised).
    fn audit(&self, entry: RoomAudit) -> impl Future<Output = ()> + Send;

    // -- the session manager's room authority ------------------------------
    /// The session's live process, if any.
    fn live_handle(&self, session: &str) -> Option<Arc<otto_pty::PtyHandle>>;
    /// Begin exclusive room authority, pinned to the current process.
    fn begin_authority(
        &self,
        session: &str,
        room: &str,
        owner: &str,
        host_member: &str,
    ) -> impl Future<Output = Result<()>> + Send;
    fn authority(&self, session: &str) -> impl Future<Output = Option<Authority>> + Send;
    /// Grant (or with `None` reclaim) terminal control; returns the new epoch.
    fn set_driver(
        &self,
        session: &str,
        room: &str,
        member: Option<&str>,
    ) -> impl Future<Output = Result<u64>> + Send;
    /// Fence a reconnecting member's old frames; returns the new epoch.
    fn fence_connection(
        &self,
        session: &str,
        room: &str,
        member: &str,
    ) -> impl Future<Output = Result<u64>> + Send;
    fn end_authority(&self, session: &str, room: &str) -> impl Future<Output = ()> + Send;
}
