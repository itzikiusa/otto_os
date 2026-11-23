//! Owner-side room lifecycle: open a room on a live session, list, end and
//! invite. The handlers authenticate the owner (and check session ownership
//! and the workspace role) before calling in; `owner` is the effective user
//! and `actor` the real user recorded in the audit trail.
use crate::{auth::eligible, authority, ice, RoomsCtx};
use otto_core::{api::*, domain::Session, Error, Result};

/// Open a room on the owner's live local session: create the registry entry,
/// take the manager's room authority (pinned to this process), start the
/// maintenance worker.
pub async fn open<C: RoomsCtx>(
    ctx: &C,
    session: &Session,
    actor: &str,
    name: &str,
) -> Result<RoomCredential> {
    eligible(session)?;
    let sid = session.id.as_str();
    let owner = session.created_by.as_str();
    let handle = ctx
        .live_handle(sid)
        .filter(|h| !h.has_exited())
        .ok_or_else(|| Error::Conflict("Start the local session before opening a room".into()))?;
    let credentials = ctx
        .rooms()
        .create(sid, owner, name, &session.title, &session.provider)
        .await?;
    if let Err(error) = ctx
        .begin_authority(sid, &credentials.room_id, owner, &credentials.member_id)
        .await
    {
        ctx.rooms().forget(&credentials.room_id).await;
        return Err(error);
    }
    let room = match ctx.rooms().get(&credentials.room_id).await {
        Ok(room) => room,
        Err(error) => {
            ctx.end_authority(sid, &credentials.room_id).await;
            ctx.rooms().forget(&credentials.room_id).await;
            return Err(error);
        }
    };
    let sync_result = {
        let mut room = room.lock().await;
        room.spawn_seq = handle.spawn_seq();
        authority::sync_authority(ctx, &mut room).await
    };
    if let Err(error) = sync_result {
        ctx.end_authority(sid, &credentials.room_id).await;
        ctx.rooms().forget(&credentials.room_id).await;
        return Err(error);
    }
    authority::audit(
        ctx,
        &credentials.room_id,
        Some(actor.into()),
        Some(credentials.member_id.clone()),
        "create",
    )
    .await;
    authority::maintain(ctx.clone(), room);
    Ok(credentials)
}
/// The owner's live rooms, each as the host sees it.
pub async fn list<C: RoomsCtx>(ctx: &C, owner: &str) -> Result<Vec<RoomSnapshot>> {
    let mut result = vec![];
    for shared in ctx.rooms().all().await {
        let mut room = shared.lock().await;
        if room.owner == owner && room.live().is_ok() {
            authority::sync_authority(ctx, &mut room).await?;
            result.push(room.snapshot(&room.host)?);
        }
    }
    Ok(result)
}
pub async fn end<C: RoomsCtx>(ctx: &C, id: &str, owner: &str) -> Result<()> {
    let shared = ctx.rooms().get(id).await?;
    let mut room = shared.lock().await;
    if room.owner != owner {
        return Err(Error::Forbidden(
            "Only the room owner can end sharing".into(),
        ));
    }
    authority::end_room(ctx, &mut room, "The host ended the room").await;
    let owner = room.owner.clone();
    drop(room);
    authority::audit(ctx, id, Some(owner), None, "end").await;
    Ok(())
}
pub async fn invite<C: RoomsCtx>(
    ctx: &C,
    id: &str,
    owner: &str,
    actor: &str,
    role: RoomRole,
) -> Result<RoomInvite> {
    let (origin, pin) = ice::invite_origin(ctx).await?;
    let invite = ctx.rooms().invite(id, owner, role).await?;
    let url = match pin {
        Some(sha256) => format!("{origin}/#/room/{id}/{invite}/{sha256}"),
        None => format!("{origin}/#/room/{id}/{invite}"),
    };
    authority::audit(ctx, id, Some(actor.into()), None, "invite").await;
    Ok(RoomInvite {
        invite,
        url,
        expires_at: (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Authority, RoomAudit, RoomRegistry};
    use otto_core::domain::{SessionKind, SessionStatus};
    use otto_core::secrets::SecretStore;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    struct NoopSecrets;
    impl SecretStore for NoopSecrets {
        fn put(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn get(&self, _: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn delete(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct SyncFailCtx {
        rooms: Arc<RoomRegistry>,
        handle: Arc<otto_pty::PtyHandle>,
        ended: Arc<Mutex<Vec<(String, String)>>>,
        secrets: Arc<dyn SecretStore>,
    }

    impl RoomsCtx for SyncFailCtx {
        fn rooms(&self) -> &RoomRegistry {
            &self.rooms
        }
        fn data_dir(&self) -> &Path {
            Path::new("/tmp")
        }
        fn base_url(&self) -> &str {
            "http://127.0.0.1:7700"
        }
        fn secrets(&self) -> &Arc<dyn SecretStore> {
            &self.secrets
        }
        async fn setting(&self, _: &str) -> Result<Option<serde_json::Value>> {
            Ok(None)
        }
        async fn put_setting(&self, _: &str, _: &serde_json::Value) -> Result<()> {
            Ok(())
        }
        async fn audit(&self, _: RoomAudit) {}
        fn live_handle(&self, _: &str) -> Option<Arc<otto_pty::PtyHandle>> {
            Some(self.handle.clone())
        }
        async fn begin_authority(&self, _: &str, _: &str, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        async fn authority(&self, _: &str) -> Option<Authority> {
            // Return None so `sync_authority` fails with Unauthorized.
            None
        }
        async fn set_driver(&self, _: &str, _: &str, _: Option<&str>) -> Result<u64> {
            Ok(1)
        }
        async fn fence_connection(&self, _: &str, _: &str, _: &str) -> Result<u64> {
            Ok(1)
        }
        async fn end_authority(&self, session: &str, room: &str) {
            self.ended
                .lock()
                .unwrap()
                .push((session.to_string(), room.to_string()));
        }
    }

    #[tokio::test]
    async fn open_cleans_up_authority_and_room_when_sync_authority_fails() {
        let handle = Arc::new(
            otto_pty::PtyHandle::spawn(&otto_pty::CommandSpec {
                program: "/bin/sleep".into(),
                args: vec!["5".into()],
                cwd: None,
                env: vec![],
            })
            .unwrap(),
        );
        let ctx = SyncFailCtx {
            rooms: Arc::default(),
            handle: handle.clone(),
            ended: Arc::default(),
            secrets: Arc::new(NoopSecrets),
        };
        let now = chrono::Utc::now();
        let session = Session {
            id: "sess-1".into(),
            workspace_id: "ws-1".into(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "Test".into(),
            cwd: "/tmp".into(),
            connection_id: None,
            provider_session_id: None,
            status: SessionStatus::Running,
            created_by: "alice".into(),
            archived: false,
            meta: serde_json::json!({}),
            created_at: now,
            last_active_at: now,
        };

        let err = open(&ctx, &session, "alice", "Alice").await.unwrap_err();
        assert!(matches!(err, Error::Unauthorized), "{err:?}");
        assert!(
            ctx.rooms.all().await.is_empty(),
            "failed open must forget the room from registry"
        );
        let ended = ctx.ended.lock().unwrap();
        assert_eq!(ended.len(), 1, "failed open must call end_authority");
        assert_eq!(ended[0].0, "sess-1");
        let _ = handle.kill();
    }
}
