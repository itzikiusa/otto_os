//! `RoomsCtx` for the daemon: settings and audit through the state DB,
//! secrets through the Keychain store, room authority through the session
//! manager.
use crate::ServerCtx;
use otto_core::{secrets::SecretStore, Id, Result};
use otto_rooms::{games, Authority, RoomAudit, RoomRegistry, RoomsCtx};
use std::{path::Path, sync::Arc};

impl RoomsCtx for ServerCtx {
    fn rooms(&self) -> &RoomRegistry {
        &self.rooms
    }
    fn game_rooms(&self) -> &games::Registry {
        &self.game_rooms
    }
    fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    fn base_url(&self) -> &str {
        &self.base_url
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
    async fn setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        otto_state::SettingsRepo::new(self.pool.clone())
            .get(key)
            .await
    }
    async fn put_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        otto_state::SettingsRepo::new(self.pool.clone())
            .put(key, value)
            .await
    }
    async fn audit(&self, entry: RoomAudit) {
        ServerCtx::audit(
            self,
            otto_state::NewAuditEntry {
                user_id: entry.user_id,
                action: entry.action,
                target: Some(entry.target),
                detail: entry.detail,
                ip: None,
            },
        )
        .await;
    }
    fn live_handle(&self, session: &str) -> Option<Arc<otto_pty::PtyHandle>> {
        self.manager.live_handle(&Id::from(session))
    }
    async fn begin_authority(
        &self,
        session: &str,
        room: &str,
        owner: &str,
        host_member: &str,
    ) -> Result<()> {
        self.manager
            .begin_room_authority(&Id::from(session), room, &Id::from(owner), host_member)
            .await
    }
    async fn authority(&self, session: &str) -> Option<Authority> {
        self.manager
            .room_authority_snapshot(&Id::from(session))
            .await
            .map(|a| Authority {
                room_id: a.room_id,
                driver: a.driver,
                epoch: a.epoch,
            })
    }
    async fn set_driver(&self, session: &str, room: &str, member: Option<&str>) -> Result<u64> {
        self.manager
            .set_room_driver(&Id::from(session), room, member)
            .await
    }
    async fn fence_connection(&self, session: &str, room: &str, member: &str) -> Result<u64> {
        self.manager
            .fence_room_connection(&Id::from(session), room, member)
            .await
    }
    async fn end_authority(&self, session: &str, room: &str) {
        self.manager
            .end_room_authority(&Id::from(session), room)
            .await;
    }
}
