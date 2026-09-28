use super::*;
use otto_core::api::*;
#[tokio::test]
async fn invitation_is_single_use_and_pending_projection_is_private() {
    let reg = RoomRegistry::new();
    let host = reg
        .create("s", "owner", "Host", "Secret session", "shell")
        .await
        .unwrap();
    let invite = reg
        .invite(&host.room_id, "owner", RoomRole::Editor)
        .await
        .unwrap();
    let guest = reg.join(&host.room_id, &invite, "Guest").await.unwrap();
    assert!(reg.join(&host.room_id, &invite, "Replay").await.is_err());
    assert_eq!(
        reg.authenticate(&host.room_id, &guest.token).await.unwrap(),
        guest.member_id
    );
    assert!(reg
        .authenticate("different-room", &guest.token)
        .await
        .is_err());
    let pending =
        serde_json::to_value(reg.snapshot(&host.room_id, &guest.member_id).await.unwrap()).unwrap();
    assert_eq!(pending.as_object().unwrap().len(), 3);
    assert_eq!(pending["admission"], "pending");
    assert!(pending.get("session_id").is_none());
}
#[tokio::test]
async fn concurrent_admissions_never_exceed_four_people() {
    let reg = std::sync::Arc::new(RoomRegistry::new());
    let host = reg
        .create("s", "owner", "Host", "Session", "shell")
        .await
        .unwrap();
    let mut tasks = vec![];
    let mut guests = vec![];
    for _ in 0..8 {
        let invite = reg
            .invite(&host.room_id, "owner", RoomRole::Viewer)
            .await
            .unwrap();
        let guest = reg.join(&host.room_id, &invite, "Guest").await.unwrap();
        guests.push(guest);
    }
    for guest in guests {
        let (r, h) = (reg.clone(), host.clone());
        tasks.push(tokio::spawn(async move {
            r.admit(&h.room_id, &h.member_id, &guest.member_id, RoomRole::Viewer)
                .await
        }));
    }
    let mut admitted = 0;
    for task in tasks {
        admitted += usize::from(task.await.unwrap().is_ok());
    }
    assert_eq!(admitted, 3);
    let snap = reg.snapshot(&host.room_id, &host.member_id).await.unwrap();
    assert_eq!(
        snap.members
            .unwrap()
            .iter()
            .filter(|m| m.admission == RoomAdmission::Admitted)
            .count(),
        4
    );
}
#[tokio::test]
async fn guest_cannot_admit_or_upgrade_invitation() {
    let reg = RoomRegistry::new();
    let host = reg
        .create("s", "owner", "Host", "Session", "shell")
        .await
        .unwrap();
    let invite = reg
        .invite(&host.room_id, "owner", RoomRole::Viewer)
        .await
        .unwrap();
    let guest = reg.join(&host.room_id, &invite, "Guest").await.unwrap();
    assert!(reg
        .admit(
            &host.room_id,
            &guest.member_id,
            &guest.member_id,
            RoomRole::Editor
        )
        .await
        .is_err());
    assert!(reg
        .admit(
            &host.room_id,
            &host.member_id,
            &guest.member_id,
            RoomRole::Editor
        )
        .await
        .is_err());
    assert!(reg
        .invite(&host.room_id, &guest.member_id, RoomRole::Viewer)
        .await
        .is_err());
}
#[test]
fn room_actions_reject_spoofed_identity_and_unknown_commands() {
    assert!(serde_json::from_value::<RoomAction>(
        serde_json::json!({"type":"chat","text":"hello","nonce":"1","member_id":"host"})
    )
    .is_err());
    assert!(serde_json::from_value::<RoomAction>(
        serde_json::json!({"type":"native_input","key":"enter"})
    )
    .is_err());
}

#[tokio::test]
async fn room_limits_and_session_uniqueness_are_enforced() {
    let reg = RoomRegistry::new();
    let host = reg
        .create("s", "owner", "Host", "Session", "shell")
        .await
        .unwrap();
    assert!(reg
        .create("s", "owner", "Other", "Session", "shell")
        .await
        .is_err());
    for _ in 0..8 {
        reg.invite(&host.room_id, "owner", RoomRole::Viewer)
            .await
            .unwrap();
    }
    assert!(reg
        .invite(&host.room_id, "owner", RoomRole::Viewer)
        .await
        .is_err());
    for n in 1..32 {
        reg.create(&format!("s{n}"), "owner", "Host", "Session", "shell")
            .await
            .unwrap();
    }
    assert!(reg
        .create("too-many", "owner", "Host", "Session", "shell")
        .await
        .is_err());
}

async fn admitted_room() -> (RoomRegistry, RoomCredential, RoomCredential) {
    let reg = RoomRegistry::new();
    let host = reg
        .create("s", "owner", "Host", "Session", "shell")
        .await
        .unwrap();
    let invite = reg
        .invite(&host.room_id, "owner", RoomRole::Editor)
        .await
        .unwrap();
    let guest = reg.join(&host.room_id, &invite, "Guest").await.unwrap();
    reg.admit(
        &host.room_id,
        &host.member_id,
        &guest.member_id,
        RoomRole::Editor,
    )
    .await
    .unwrap();
    {
        let room = reg.get(&host.room_id).await.unwrap();
        let mut room = room.lock().await;
        room.connect(&host.member_id).unwrap();
        room.connect(&guest.member_id).unwrap();
    }
    (reg, host, guest)
}
#[tokio::test]
async fn chat_is_attributed_bounded_and_retries_deduplicate() {
    let (reg, host, guest) = admitted_room().await;
    let room = reg.get(&host.room_id).await.unwrap();
    let mut room = room.lock().await;
    room.apply(
        &guest.member_id,
        RoomAction::Chat {
            text: "hello".into(),
            nonce: "retry".into(),
        },
    )
    .unwrap();
    room.apply(
        &guest.member_id,
        RoomAction::Chat {
            text: "hello".into(),
            nonce: "retry".into(),
        },
    )
    .unwrap();
    assert_eq!(room.messages.len(), 1);
    assert_eq!(room.messages[0].member_id, guest.member_id);
    assert_eq!(room.messages[0].name, "Guest");
    assert!(room
        .apply(
            &guest.member_id,
            RoomAction::Chat {
                text: "🔥".repeat(1025),
                nonce: "big".into()
            }
        )
        .is_err());
    for n in 0..9 {
        room.apply(
            &guest.member_id,
            RoomAction::Chat {
                text: "x".into(),
                nonce: n.to_string(),
            },
        )
        .unwrap();
    }
    assert!(room
        .apply(
            &guest.member_id,
            RoomAction::Chat {
                text: "x".into(),
                nonce: "too-fast".into()
            }
        )
        .is_err());
}
#[tokio::test]
async fn disconnect_fences_old_generation_and_withdraws_media() {
    let (reg, host, guest) = admitted_room().await;
    let room = reg.get(&host.room_id).await.unwrap();
    let mut room = room.lock().await;
    room.apply(
        &host.member_id,
        RoomAction::GrantControl {
            member_id: guest.member_id.clone(),
        },
    )
    .unwrap();
    assert!(room.disconnect(&guest.member_id, 1));
    assert_eq!(room.driver, host.member_id);
    let generation = room.connect(&guest.member_id).unwrap();
    assert_eq!(generation, 2);
    assert!(room.connected(&guest.member_id, 1).is_err());
    assert!(!room.disconnect(&guest.member_id, 1));
    assert!(room.members[&guest.member_id].view.connected);
    assert!(!room.members[&guest.member_id].view.presenter_allowed);
}
#[tokio::test]
async fn presenter_permission_and_annotation_revoke_epochs_are_independent() {
    let (reg, host, guest) = admitted_room().await;
    let room = reg.get(&host.room_id).await.unwrap();
    let mut room = room.lock().await;
    assert!(room
        .apply(
            &guest.member_id,
            RoomAction::StartPresent {
                title: "IDE".into(),
                width: 800,
                height: 600
            }
        )
        .is_err());
    room.apply(
        &host.member_id,
        RoomAction::GrantPresent {
            member_id: guest.member_id.clone(),
            allowed: true,
        },
    )
    .unwrap();
    room.apply(
        &guest.member_id,
        RoomAction::StartPresent {
            title: "IDE".into(),
            width: 800,
            height: 600,
        },
    )
    .unwrap();
    assert!(room
        .apply(
            &guest.member_id,
            RoomAction::StartPresent {
                title: "second".into(),
                width: 800,
                height: 600
            }
        )
        .is_err());
    let source = room.presentations[0].clone();
    room.apply(
        &host.member_id,
        RoomAction::RequestAnnotation {
            source_id: source.id.clone(),
        },
    )
    .unwrap();
    room.apply(
        &guest.member_id,
        RoomAction::GrantAnnotation {
            source_id: source.id.clone(),
            member_id: host.member_id.clone(),
            allowed: true,
        },
    )
    .unwrap();
    let old = room
        .grants
        .iter()
        .find(|g| g.member_id == host.member_id)
        .unwrap()
        .epoch;
    room.apply(
        &host.member_id,
        RoomAction::RevokeAnnotation {
            source_id: source.id.clone(),
            member_id: host.member_id.clone(),
            blocked: true,
        },
    )
    .unwrap();
    assert!(room
        .apply(
            &guest.member_id,
            RoomAction::GrantAnnotation {
                source_id: source.id.clone(),
                member_id: host.member_id.clone(),
                allowed: true
            }
        )
        .is_err());
    room.apply(
        &host.member_id,
        RoomAction::RevokeAnnotation {
            source_id: source.id.clone(),
            member_id: host.member_id.clone(),
            blocked: false,
        },
    )
    .unwrap();
    room.apply(
        &guest.member_id,
        RoomAction::GrantAnnotation {
            source_id: source.id.clone(),
            member_id: host.member_id.clone(),
            allowed: true,
        },
    )
    .unwrap();
    assert!(room
        .apply(
            &host.member_id,
            RoomAction::Annotation {
                source_id: source.id.clone(),
                source_generation: 1,
                clear_epoch: 0,
                grant_epoch: old,
                tool: RoomAnnotationTool::Pen,
                points: vec![RoomPoint { x: 0.5, y: 0.5 }]
            }
        )
        .is_err());
    assert_eq!(room.driver, host.member_id);
}

#[tokio::test]
async fn expiry_and_host_disconnect_cannot_restore_media() {
    let (reg, host, guest) = admitted_room().await;
    let shared = reg.get(&host.room_id).await.unwrap();
    let mut room = shared.lock().await;
    room.apply(
        &host.member_id,
        RoomAction::GrantPresent {
            member_id: guest.member_id.clone(),
            allowed: true,
        },
    )
    .unwrap();
    room.apply(
        &guest.member_id,
        RoomAction::StartPresent {
            title: "Guest IDE".into(),
            width: 800,
            height: 600,
        },
    )
    .unwrap();
    room.disconnect(&host.member_id, 1);
    assert!(room.presentations.is_empty());
    assert!(room.grants.is_empty());
    assert!(!room.members[&guest.member_id].view.presenter_allowed);
    room.members.get_mut(&guest.member_id).unwrap().disconnected =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(31));
    drop(room);
    assert!(reg.authenticate(&host.room_id, &guest.token).await.is_err());
}
#[tokio::test]
async fn chat_retention_nonce_window_and_annotation_clear_are_bounded() {
    let (reg, host, guest) = admitted_room().await;
    let shared = reg.get(&host.room_id).await.unwrap();
    let mut room = shared.lock().await;
    for n in 0..205 {
        let m = room.members.get_mut(&guest.member_id).unwrap();
        m.chat_rate.tokens = 10.0;
        m.action_rate.tokens = 80.0;
        room.apply(
            &guest.member_id,
            RoomAction::Chat {
                text: "message".into(),
                nonce: n.to_string(),
            },
        )
        .unwrap();
    }
    assert_eq!(room.messages.len(), 200);
    assert_eq!(room.messages.front().unwrap().seq, 6);
    room.apply(
        &host.member_id,
        RoomAction::StartPresent {
            title: "Host IDE".into(),
            width: 800,
            height: 600,
        },
    )
    .unwrap();
    let source = room.presentations[0].clone();
    let mark = RoomAction::Annotation {
        source_id: source.id.clone(),
        source_generation: 1,
        clear_epoch: 0,
        grant_epoch: 1,
        tool: RoomAnnotationTool::Pen,
        points: vec![RoomPoint { x: 0.5, y: 0.5 }],
    };
    room.apply(&host.member_id, mark.clone()).unwrap();
    room.apply(
        &host.member_id,
        RoomAction::ClearAnnotations {
            source_id: source.id.clone(),
        },
    )
    .unwrap();
    assert!(room.marks.is_empty());
    assert!(room.apply(&host.member_id, mark).is_err());
    let invalid = RoomAction::Annotation {
        source_id: source.id,
        source_generation: 1,
        clear_epoch: 1,
        grant_epoch: 1,
        tool: RoomAnnotationTool::Pen,
        points: vec![RoomPoint { x: 1.1, y: 0.5 }],
    };
    assert!(room.apply(&host.member_id, invalid).is_err());
}

#[tokio::test]
async fn geometry_updates_fence_annotations_without_lifting_host_blocks() {
    let (reg, host, guest) = admitted_room().await;
    let shared = reg.get(&host.room_id).await.unwrap();
    let mut room = shared.lock().await;
    room.apply(
        &host.member_id,
        RoomAction::StartPresent {
            title: "Host IDE".into(),
            width: 800,
            height: 600,
        },
    )
    .unwrap();
    let source = room.presentations[0].id.clone();
    room.apply(
        &host.member_id,
        RoomAction::RevokeAnnotation {
            source_id: source.clone(),
            member_id: guest.member_id.clone(),
            blocked: true,
        },
    )
    .unwrap();
    assert!(room
        .apply(
            &guest.member_id,
            RoomAction::UpdatePresent {
                source_id: source.clone(),
                width: 900,
                height: 600
            }
        )
        .is_err());
    room.apply(
        &host.member_id,
        RoomAction::UpdatePresent {
            source_id: source.clone(),
            width: 900,
            height: 600,
        },
    )
    .unwrap();
    assert_eq!(room.presentations[0].generation, 2);
    assert_eq!(room.presentations[0].clear_epoch, 1);
    assert!(
        room.grants
            .iter()
            .find(|g| g.member_id == guest.member_id)
            .unwrap()
            .blocked
    );
    assert!(
        !room
            .grants
            .iter()
            .find(|g| g.member_id == guest.member_id)
            .unwrap()
            .allowed
    );
}
