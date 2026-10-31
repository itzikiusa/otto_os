//! The `RoomsCtx` seam, driven through a fake host: invite origins, the
//! authority sync, the audit trail, ICE credentials and the maintenance pass.
use crate::{authority, ice, lifecycle, Authority, RoomAudit, RoomRegistry, RoomsCtx};
use otto_core::{api::*, secrets::SecretStore, Error, Result};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct MemSecrets(Mutex<HashMap<String, String>>);
impl SecretStore for MemSecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[derive(Clone)]
struct Fake {
    rooms: Arc<RoomRegistry>,
    dir: PathBuf,
    secrets: Arc<dyn SecretStore>,
    settings: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    audits: Arc<Mutex<Vec<RoomAudit>>>,
    authority: Arc<Mutex<HashMap<String, Authority>>>,
    ended: Arc<Mutex<Vec<(String, String)>>>,
    lan: Arc<Mutex<Option<String>>>,
}
impl Fake {
    fn new() -> Self {
        Self {
            rooms: Arc::default(),
            dir: std::env::temp_dir(),
            secrets: Arc::new(MemSecrets::default()),
            settings: Arc::default(),
            audits: Arc::default(),
            authority: Arc::default(),
            ended: Arc::default(),
            lan: Arc::default(),
        }
    }
    fn actions(&self) -> Vec<String> {
        self.audits
            .lock()
            .unwrap()
            .iter()
            .map(|a| a.action.clone())
            .collect()
    }
}
impl RoomsCtx for Fake {
    fn rooms(&self) -> &RoomRegistry {
        &self.rooms
    }
    fn data_dir(&self) -> &Path {
        &self.dir
    }
    fn base_url(&self) -> &str {
        "http://127.0.0.1:7700"
    }
    fn lan_origin(&self) -> Option<String> {
        self.lan.lock().unwrap().clone()
    }
    fn lan_certificate_sha256(&self) -> Option<String> {
        Some("ab".repeat(32))
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
    async fn setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        Ok(self.settings.lock().unwrap().get(key).cloned())
    }
    async fn put_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        self.settings
            .lock()
            .unwrap()
            .insert(key.into(), value.clone());
        Ok(())
    }
    async fn audit(&self, entry: RoomAudit) {
        self.audits.lock().unwrap().push(entry);
    }
    fn live_handle(&self, _session: &str) -> Option<Arc<otto_pty::PtyHandle>> {
        None
    }
    async fn begin_authority(&self, _: &str, _: &str, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    async fn authority(&self, session: &str) -> Option<Authority> {
        self.authority.lock().unwrap().get(session).cloned()
    }
    async fn set_driver(&self, session: &str, room: &str, member: Option<&str>) -> Result<u64> {
        let mut map = self.authority.lock().unwrap();
        let a = map.get_mut(session).ok_or(Error::Unauthorized)?;
        if a.room_id != room {
            return Err(Error::Unauthorized);
        }
        a.driver = member.map(str::to_owned);
        a.epoch += 1;
        Ok(a.epoch)
    }
    async fn fence_connection(&self, session: &str, room: &str, _: &str) -> Result<u64> {
        self.set_driver(session, room, None).await
    }
    async fn end_authority(&self, session: &str, room: &str) {
        self.ended
            .lock()
            .unwrap()
            .push((session.into(), room.into()));
    }
}

#[tokio::test]
async fn invites_use_the_public_origin_else_the_lan_listener_else_the_daemon_and_are_audited() {
    let ctx = Fake::new();
    let host = ctx
        .rooms
        .create("s1", "owner", "Host", "Session", "shell")
        .await
        .unwrap();
    let invite = lifecycle::invite(&ctx, &host.room_id, "owner", "real", RoomRole::Viewer)
        .await
        .unwrap();
    assert!(invite
        .url
        .starts_with(&format!("http://127.0.0.1:7700/#/room/{}/", host.room_id)));
    // A serving LAN listener beats loopback: another computer can reach it.
    *ctx.lan.lock().unwrap() = Some("https://192.168.1.20:7701".into());
    let invite = lifecycle::invite(&ctx, &host.room_id, "owner", "real", RoomRole::Viewer)
        .await
        .unwrap();
    // …and pins its self-signed certificate for the joining app.
    assert!(invite.url.ends_with(&format!("/{}", "ab".repeat(32))));
    assert!(invite.url.starts_with(&format!(
        "https://192.168.1.20:7701/#/room/{}/",
        host.room_id
    )));
    ctx.put_setting(
        "room_public_origin",
        &serde_json::json!("https://rooms.example.test/"),
    )
    .await
    .unwrap();
    let invite = lifecycle::invite(&ctx, &host.room_id, "owner", "real", RoomRole::Editor)
        .await
        .unwrap();
    assert!(invite.url.starts_with("https://rooms.example.test/#/room/"));
    // Only the session owner may invite; the refusal is not audited.
    assert!(lifecycle::invite(
        &ctx,
        &host.room_id,
        "intruder",
        "intruder",
        RoomRole::Viewer
    )
    .await
    .is_err());
    let audits = ctx.audits.lock().unwrap().clone();
    assert_eq!(audits.len(), 3);
    assert_eq!(audits[0].action, "room.invite");
    assert_eq!(audits[0].user_id.as_deref(), Some("real"));
    assert_eq!(audits[0].target, host.room_id);
}

#[tokio::test]
async fn authority_sync_follows_the_manager_and_refuses_another_room() {
    let ctx = Fake::new();
    let host = ctx
        .rooms
        .create("s1", "owner", "Host", "Session", "shell")
        .await
        .unwrap();
    let shared = ctx.rooms.get(&host.room_id).await.unwrap();
    let mut room = shared.lock().await;
    // No authority, or authority held by another room: refused.
    assert!(authority::sync_authority(&ctx, &mut room).await.is_err());
    ctx.authority.lock().unwrap().insert(
        "s1".into(),
        Authority {
            room_id: "other".into(),
            driver: None,
            epoch: 1,
        },
    );
    assert!(authority::sync_authority(&ctx, &mut room).await.is_err());
    ctx.authority.lock().unwrap().insert(
        "s1".into(),
        Authority {
            room_id: host.room_id.clone(),
            driver: Some("guest".into()),
            epoch: 7,
        },
    );
    authority::sync_authority(&ctx, &mut room).await.unwrap();
    assert_eq!((room.driver.as_str(), room.grant_epoch), ("guest", 7));
    // A driver effect goes through the manager, then resyncs (None = host).
    authority::effect(&ctx, &mut room, crate::Effect::Driver(None))
        .await
        .unwrap();
    assert_eq!(
        (room.driver.clone(), room.grant_epoch),
        (host.member_id.clone(), 8)
    );
    // End releases the manager's authority, ends the room and audits it.
    authority::effect(&ctx, &mut room, crate::Effect::End)
        .await
        .unwrap();
    assert!(room.ended);
    assert_eq!(
        *ctx.ended.lock().unwrap(),
        vec![("s1".to_string(), host.room_id.clone())]
    );
    assert_eq!(ctx.actions(), vec!["room.closed"]);
}

#[tokio::test]
async fn ice_configuration_signs_turn_only_with_a_secret() {
    let ctx = Fake::new();
    let req = RoomSettingsReq {
        public_origin: "https://rooms.example.test".into(),
        stun_urls: vec!["stun:relay.example.test:3478".into()],
        turn_urls: vec!["turn:relay.example.test:3478".into()],
        relay_only: true,
        turn_secret: None,
    };
    let saved = ice::put_settings(&ctx, req.clone()).await.unwrap();
    assert!(!saved.turn_secret_configured);
    let RoomEvent::Ice {
        ice_servers,
        relay_configured,
        relay_only,
        ..
    } = ice::configuration(&ctx, "member-1").await.unwrap()
    else {
        panic!("expected an ICE answer")
    };
    assert_eq!(ice_servers.len(), 1, "no TURN without a secret");
    assert!(!relay_configured && relay_only);
    let saved = ice::put_settings(
        &ctx,
        RoomSettingsReq {
            turn_secret: Some("fixture-turn-secret".into()),
            ..req
        },
    )
    .await
    .unwrap();
    assert!(saved.turn_secret_configured);
    // The secret itself stays in the secret store, never in settings.
    let stored = ctx.settings.lock().unwrap()["room_ice"].to_string();
    assert!(!stored.contains("fixture-turn-secret"));
    let RoomEvent::Ice {
        ice_servers,
        relay_configured,
        ..
    } = ice::configuration(&ctx, "member-1").await.unwrap()
    else {
        panic!("expected an ICE answer")
    };
    assert!(relay_configured);
    let turn = &ice_servers[1];
    assert!(turn
        .username
        .as_deref()
        .is_some_and(|u| u.ends_with(":member-1")));
    assert!(turn
        .credential
        .as_deref()
        .is_some_and(|c| !c.is_empty() && c != "fixture-turn-secret"));
}

#[tokio::test]
async fn maintenance_sweep_times_out_members_and_settles_unconfirmed_audio() {
    let ctx = Fake::new();
    let host = ctx
        .rooms
        .create("s1", "owner", "Host", "Session", "shell")
        .await
        .unwrap();
    let shared = ctx.rooms.get(&host.room_id).await.unwrap();
    let mut room = shared.lock().await;
    room.connect(&host.member_id).unwrap();
    assert_eq!(
        room.sweep(Instant::now()),
        (false, false),
        "fresh heartbeat"
    );
    // A silent socket is disconnected and the host reclaims the driver.
    room.members.get_mut(&host.member_id).unwrap().heartbeat =
        Instant::now() - Duration::from_secs(11);
    assert_eq!(room.sweep(Instant::now()), (true, true));
    assert!(!room.members[&host.member_id].view.connected);
    // Unconfirmed audio with someone joined: everyone is muted, epoch bumps.
    room.members
        .get_mut(&host.member_id)
        .unwrap()
        .view
        .audio_joined = true;
    room.audio_pending_since = Some(Instant::now() - Duration::from_secs(4));
    let epoch = room.audio_epoch;
    assert!(room.settle_audio());
    assert_eq!(room.audio_epoch, epoch + 1);
    assert!(!room.members[&host.member_id].view.audio_joined);
    // Nobody joined: the pending epoch is simply enforced.
    room.audio_pending_since = Some(Instant::now() - Duration::from_secs(4));
    assert!(!room.settle_audio());
    assert_eq!(room.audio_enforced_epoch, room.audio_epoch);
    // Not yet due: nothing happens.
    room.audio_pending_since = Some(Instant::now());
    assert!(!room.settle_audio());
}
