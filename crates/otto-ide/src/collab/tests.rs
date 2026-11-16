use super::*;
use crate::types::{
    IdeCollabCursorReq, IdeCollabDebugActionReq, IdeCollabEditReq, IdeDebugAction, IdeDebugState,
    IdeSelectionRange,
};
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
    assert_eq!(created.host_peer_secret.len(), 64);
    assert_ne!(created.invite_token, created.host_peer_secret);
    assert!(created.share_hash.contains(&created.room.id));
    assert!(created.share_hash.contains(&created.invite_token));
    assert_eq!(created.room.peers.len(), 1);
    assert!(created.room.peers[0].is_host);
    assert_eq!(created.room.peers[0].peer_id, created.host_peer_id);

    assert!(matches!(
        hub.apply_edit(
            &created.room.id,
            &IdeCollabEditReq {
                peer_id: created.host_peer_id.clone(),
                peer_secret: created.invite_token.clone(),
                path: "main.go".into(),
                content: "hijacked\n".into(),
                base_version: None,
                save: false,
            },
            false,
        ),
        Err(Error::Unauthorized)
    ));

    let impostor = hub
        .join_room(
            &created.room.id,
            IdeCollabJoinReq {
                display_name: "Host".into(),
                token: created.invite_token.clone(),
            },
        )
        .expect("impostor joins as viewer");
    let imp_peer = impostor
        .room
        .peers
        .iter()
        .find(|p| p.peer_id == impostor.peer_id)
        .unwrap();
    assert!(!imp_peer.is_host);
    assert_eq!(imp_peer.role, IdeCollabRole::Viewer);

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
    assert_eq!(joined.room.peers.len(), 3);

    let ev = rx.recv().await.expect("peer joined event");
    assert!(matches!(ev, IdeCollabEvent::PeerJoined { .. }));

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

    let v1 = hub
        .apply_edit(&created.room.id, &edit_req, false)
        .expect("editor edit");
    assert_eq!(v1, 1);
    let _ = rx.recv().await.expect("buffer updated event");

    let v2 = hub
        .apply_edit(
            &created.room.id,
            &IdeCollabEditReq {
                base_version: Some(1),
                content: "package main\nfunc main() {}\n".into(),
                ..edit_req.clone()
            },
            false,
        )
        .expect("editor v2 edit");
    assert_eq!(v2, 2);
    let _ = rx.recv().await.expect("buffer v2 updated event");

    // A guest save is announced as saved only after its disk write, and only for
    // the latest version: a late write of v1 must not mark v2 clean.
    hub.mark_buffer_saved(&created.room.id, &edit_req.peer_id, &edit_req.path, 1);
    assert!(rx.try_recv().is_err(), "stale version must not broadcast");
    hub.mark_buffer_saved(&created.room.id, &edit_req.peer_id, &edit_req.path, 2);
    match rx.recv().await.expect("saved event") {
        IdeCollabEvent::BufferUpdated {
            version,
            saved,
            sha256,
            ..
        } => assert!(version == 2 && saved && sha256.is_some()),
        other => panic!("unexpected event {other:?}"),
    }

    let stale_req = IdeCollabEditReq {
        base_version: Some(1),
        content: "stale\n".into(),
        save: true,
        ..edit_req.clone()
    };
    assert!(matches!(
        hub.validate_edit_precheck(&created.room.id, &stale_req),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        hub.apply_edit(&created.room.id, &stale_req, false),
        Err(Error::Conflict(_))
    ));

    // Another user debugging the same repository path must NOT inject into host's room.
    let foreign_snap = IdeDebugSessionSnapshot {
        id: "dbg_stranger".into(),
        revision: 1,
        title: "Stranger Debug".into(),
        language: "go".into(),
        adapter: "dlv".into(),
        config_id: None,
        launch: None,
        state: IdeDebugState::Paused,
        stopped_reason: Some("breakpoint".into()),
        active_thread_id: Some(1),
        active_frame_id: Some(1),
        paused_file: Some("/tmp/shared-repo/main.go".into()),
        paused_line: Some(1),
        threads: vec![],
        stack_frames: vec![],
        scopes: vec![],
        inline_values: vec![],
        breakpoints: vec![],
        output: vec![],
        error: None,
    };
    hub.publish_debug_snapshot(
        Some(&IdeTargetSpec::local("/tmp/shared-repo")),
        Some("u_other"),
        &foreign_snap,
    );
    assert!(
        hub.get_state(&created.room.id, &created.invite_token)
            .unwrap()
            .active_debug
            .is_none(),
        "foreign user's debug snapshot on same target must not bind to host room"
    );

    let dbg_snap = IdeDebugSessionSnapshot {
        id: "dbg_99".into(),
        revision: 1,
        title: "Debug main.go".into(),
        language: "go".into(),
        adapter: "dlv".into(),
        config_id: None,
        launch: None,
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
    hub.publish_debug_snapshot(
        Some(&IdeTargetSpec::local("/tmp/shared-repo")),
        Some("u_host"),
        &dbg_snap,
    );
    let ev = rx.recv().await.expect("debug updated event");
    assert!(matches!(ev, IdeCollabEvent::DebugUpdated { .. }));

    assert!(matches!(
        hub.authorize_debug_action(
            &created.room.id,
            &IdeCollabDebugActionReq {
                peer_id: joined.peer_id.clone(),
                peer_secret: joined.peer_secret.clone(),
                debug_id: "dbg_foreign".into(),
                action: IdeDebugAction::StepOver,
                thread_id: None,
                frame_id: None,
                path: None,
                line: None,
                breakpoints: None,
            },
        ),
        Err(Error::Forbidden(_))
    ));
    assert!(hub
        .authorize_debug_action(
            &created.room.id,
            &IdeCollabDebugActionReq {
                peer_id: joined.peer_id.clone(),
                peer_secret: joined.peer_secret.clone(),
                debug_id: "dbg_99".into(),
                action: IdeDebugAction::StepOver,
                thread_id: None,
                frame_id: None,
                path: None,
                line: None,
                breakpoints: None,
            },
        )
        .is_ok());

    let state = hub
        .get_state(&created.room.id, &created.invite_token)
        .expect("get state");
    assert_eq!(state.shared_buffers.get("main.go").unwrap().version, 2);
    assert_eq!(state.active_debug.unwrap().id, "dbg_99");

    // Once the debugged target finishes (`Terminated`), `active_debug` clears to `None`,
    // non-restart actions are rejected, and an `Editor` peer can still trigger `Restart`.
    let mut term_snap = dbg_snap.clone();
    term_snap.revision = 2;
    term_snap.state = IdeDebugState::Terminated;
    hub.publish_debug_snapshot(
        Some(&IdeTargetSpec::local("/tmp/shared-repo")),
        Some("u_host"),
        &term_snap,
    );
    let _ = rx.recv().await.expect("terminated debug event");
    assert!(hub
        .get_state(&created.room.id, &created.invite_token)
        .unwrap()
        .active_debug
        .is_none());
    assert!(matches!(
        hub.authorize_debug_action(
            &created.room.id,
            &IdeCollabDebugActionReq {
                peer_id: joined.peer_id.clone(),
                peer_secret: joined.peer_secret.clone(),
                debug_id: "dbg_99".into(),
                action: IdeDebugAction::StepOver,
                thread_id: None,
                frame_id: None,
                path: None,
                line: None,
                breakpoints: None,
            },
        ),
        Err(Error::Forbidden(_))
    ));
    assert!(hub
        .authorize_debug_action(
            &created.room.id,
            &IdeCollabDebugActionReq {
                peer_id: joined.peer_id.clone(),
                peer_secret: joined.peer_secret.clone(),
                debug_id: "dbg_99".into(),
                action: IdeDebugAction::Restart,
                thread_id: None,
                frame_id: None,
                path: None,
                line: None,
                breakpoints: None,
            },
        )
        .is_ok());

    hub.close_room(&host, &created.room.id).expect("close room");
    let ev = rx.recv().await.expect("room closed event");
    assert!(matches!(ev, IdeCollabEvent::RoomClosed { .. }));
    assert!(hub
        .get_state(&created.room.id, &created.invite_token)
        .is_err());
}

#[test]
fn room_caps_idle_ttl_and_buffer_eviction_invariants_hold() {
    let hub = CollabHub::new();
    let host = test_user("u_host", "itzik", false);

    let mut first_room_id = String::new();
    let mut latest = None;
    for i in 0..(MAX_COLLAB_ROOMS + 5) {
        let resp = hub
            .create_room(
                &host,
                IdeCreateCollabRoomReq {
                    target: IdeTargetSpec::local(format!("/tmp/repo-{i}")),
                    title: Some(format!("Room {i}")),
                    default_role: IdeCollabRole::Editor,
                },
            )
            .expect("create room");
        if i == 0 {
            first_room_id = resp.room.id.clone();
        }
        latest = Some(resp);
    }
    assert_eq!(hub.list_rooms(&host).unwrap().len(), MAX_COLLAB_ROOMS);
    assert!(!hub
        .list_rooms(&host)
        .unwrap()
        .iter()
        .any(|r| r.id == first_room_id));

    let active = latest.unwrap();
    let host_peer_id = active.host_peer_id.clone();
    let host_peer_secret = active.host_peer_secret.clone();

    let five_mib = "a".repeat(5 * 1024 * 1024);
    for (p, save) in [
        ("dirty_older.rs", false),
        ("clean_oldest.rs", true),
        ("clean_newer.rs", true),
        ("fourth.rs", true),
    ] {
        hub.apply_edit(
            &active.room.id,
            &IdeCollabEditReq {
                peer_id: host_peer_id.clone(),
                peer_secret: host_peer_secret.clone(),
                path: p.into(),
                content: five_mib.clone(),
                base_version: None,
                save,
            },
            save,
        )
        .unwrap();
    }

    let st = hub
        .get_state(&active.room.id, &active.invite_token)
        .unwrap();
    assert!(!st.shared_buffers.contains_key("clean_oldest.rs"));
    assert!(st.shared_buffers.contains_key("dirty_older.rs"));
    assert!(st.shared_buffers.contains_key("fourth.rs"));
    let total_bytes: usize = st.shared_buffers.values().map(|b| b.content.len()).sum();
    assert!(total_bytes <= MAX_ROOM_BUFFER_BYTES);

    for i in 0..40 {
        hub.apply_edit(
            &active.room.id,
            &IdeCollabEditReq {
                peer_id: host_peer_id.clone(),
                peer_secret: host_peer_secret.clone(),
                path: format!("file_{i:02}.rs"),
                content: "fn main() {}\n".into(),
                base_version: None,
                save: true,
            },
            true,
        )
        .unwrap();
    }
    let st = hub
        .get_state(&active.room.id, &active.invite_token)
        .unwrap();
    assert_eq!(st.shared_buffers.len(), MAX_SHARED_BUFFERS_PER_ROOM);

    {
        let mut guard = hub.rooms.write().unwrap();
        let rm = guard.get_mut(&active.room.id).unwrap();
        rm.last_activity_at = Instant::now() - ROOM_IDLE_TTL - Duration::from_secs(5);
    }
    let remaining = hub.list_rooms(&host).unwrap();
    assert!(!remaining.iter().any(|r| r.id == active.room.id));
}
