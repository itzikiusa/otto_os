use super::{registry::*, types::*};
use serde_json::json;
use std::time::{Duration, Instant};

fn config() -> Config {
    Config {
        game: GameKind::Shooter,
        map: "station".into(),
    }
}
async fn pair() -> (Registry, Credential, Credential, SharedRoom) {
    let registry = Registry::default();
    let host = registry.create("owner", "Host", config()).await.unwrap();
    let guest = registry
        .join(&host.room_id, host.invite.as_deref().unwrap(), "Guest")
        .await
        .unwrap();
    let room = registry.get(&host.room_id).await.unwrap();
    (registry, host, guest, room)
}
#[tokio::test]
async fn invite_is_consumed_and_capabilities_are_room_scoped() {
    let (reg, host, guest, _) = pair().await;
    assert!(reg
        .join(&host.room_id, host.invite.as_deref().unwrap(), "Third")
        .await
        .is_err());
    assert_eq!(
        reg.authenticate(&host.room_id, &guest.token).await.unwrap(),
        1
    );
    assert!(reg
        .authenticate(&host.room_id, host.invite.as_deref().unwrap())
        .await
        .is_err());
    let other = reg.create("other", "Other", config()).await.unwrap();
    assert!(reg
        .authenticate(&other.room_id, &guest.token)
        .await
        .is_err());
}
#[tokio::test]
async fn wrong_role_and_stale_round_packets_never_relay() {
    let (_, _, _, room) = pair().await;
    let mut room = room.lock().await;
    let h = room.connect(0).unwrap();
    let g = room.connect(1).unwrap();
    room.apply(0, h, Command::Ready { ready: true }).unwrap();
    room.apply(1, g, Command::Ready { ready: true }).unwrap();
    let epoch = room.generation;
    assert!(room
        .apply(
            1,
            g,
            Command::Start {
                round: 1,
                generation: epoch
            }
        )
        .is_err());
    room.apply(
        0,
        h,
        Command::Start {
            round: 1,
            generation: epoch,
        },
    )
    .unwrap();
    assert!(room
        .apply(
            1,
            g,
            Command::Snapshot {
                round: 1,
                generation: epoch,
                seq: 1,
                data: json!({})
            }
        )
        .is_err());
    assert!(room
        .apply(
            0,
            h,
            Command::Input {
                round: 1,
                generation: epoch,
                seq: 1,
                data: json!({})
            }
        )
        .is_err());
    assert!(room
        .apply(
            1,
            g,
            Command::Input {
                round: 0,
                generation: epoch,
                seq: 1,
                data: json!({})
            }
        )
        .is_err());
    room.apply(
        1,
        g,
        Command::Input {
            round: 1,
            generation: epoch,
            seq: 1,
            data: json!({"forward":1}),
        },
    )
    .unwrap();
    assert!(room
        .apply(
            1,
            g,
            Command::Input {
                round: 1,
                generation: epoch,
                seq: 1,
                data: json!({})
            }
        )
        .is_err());
    room.apply(
        0,
        h,
        Command::Finish {
            round: 1,
            generation: epoch,
            result: json!({"winner":0}),
        },
    )
    .unwrap();
    room.apply(0, h, Command::Rematch).unwrap();
    assert_eq!(room.round, 1);
    room.apply(1, g, Command::Rematch).unwrap();
    assert_eq!(room.round, 2);
    assert_eq!(room.phase, Phase::Lobby);
    assert!(!room.members[0].view.ready);
}
#[tokio::test]
async fn reconnect_invalidates_packets_and_old_socket_cleanup() {
    let (_, _, _, room) = pair().await;
    let mut room = room.lock().await;
    let first = room.connect(0).unwrap();
    assert!(room.connect(0).is_err());
    room.disconnect(0, first);
    let second = room.connect(0).unwrap();
    room.disconnect(0, first);
    assert!(room.members[0].view.connected);
    assert!(room
        .apply(0, first, Command::Ready { ready: true })
        .is_err());
    room.disconnect(0, second);
    room.maintain(Instant::now() + Duration::from_secs(21));
    assert_eq!(room.phase, Phase::Closed);
}
#[tokio::test]
async fn map_payload_and_message_rate_are_bounded() {
    let reg = Registry::default();
    assert!(reg
        .create(
            "owner",
            "Host",
            Config {
                game: GameKind::Kart,
                map: "station".into()
            }
        )
        .await
        .is_err());
    let (_, _, _, room) = pair().await;
    let mut room = room.lock().await;
    let h = room.connect(0).unwrap();
    for _ in 0..20 {
        let _ = room.apply(0, h, Command::Ping);
    }
    assert!(room.apply(0, h, Command::Ping).is_err());
}

#[tokio::test]
async fn concurrent_invite_redemption_has_exactly_one_winner() {
    let registry = std::sync::Arc::new(Registry::default());
    let host = registry.create("owner", "Host", config()).await.unwrap();
    let (a, b) = tokio::join!(
        registry.join(&host.room_id, host.invite.as_deref().unwrap(), "Guest A"),
        registry.join(&host.room_id, host.invite.as_deref().unwrap(), "Guest B")
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        registry
            .get(&host.room_id)
            .await
            .unwrap()
            .lock()
            .await
            .members
            .len(),
        2
    );
}
#[tokio::test]
async fn capacity_and_abandoned_rooms_are_bounded() {
    let registry = Registry::default();
    let first = registry.create("owner", "Host", config()).await.unwrap();
    registry.create("owner", "Host", config()).await.unwrap();
    assert!(registry.create("owner", "Host", config()).await.is_err());
    let room = registry.get(&first.room_id).await.unwrap();
    room.lock()
        .await
        .maintain(Instant::now() + Duration::from_secs(21));
    assert!(registry.create("owner", "Host", config()).await.is_ok());
    assert!(registry.get(&first.room_id).await.is_err());
    for i in 0..30 {
        registry
            .create(&format!("owner{i}"), "Host", config())
            .await
            .unwrap();
    }
    assert!(registry.create("overflow", "Host", config()).await.is_err());
}
#[tokio::test]
async fn pause_reconnect_rejects_old_generation_and_oversized_data() {
    let (_, _, _, shared) = pair().await;
    let mut room = shared.lock().await;
    let mut notices = room.tx.subscribe();
    let host = room.connect(0).unwrap();
    let guest = room.connect(1).unwrap();
    room.apply(0, host, Command::Ready { ready: true }).unwrap();
    room.apply(1, guest, Command::Ready { ready: true })
        .unwrap();
    let old_generation = room.generation;
    room.apply(
        0,
        host,
        Command::Start {
            round: 1,
            generation: old_generation,
        },
    )
    .unwrap();
    assert!(room
        .apply(
            0,
            host,
            Command::Snapshot {
                round: 1,
                generation: old_generation,
                seq: 1,
                data: json!({"large":"a".repeat(16384)})
            }
        )
        .is_err());
    assert!(room
        .apply(
            1,
            guest,
            Command::Input {
                round: 1,
                generation: old_generation,
                seq: 1,
                data: json!({"large":"a".repeat(1024)})
            }
        )
        .is_err());
    assert!(room
        .apply(
            0,
            host,
            Command::Finish {
                round: 1,
                generation: old_generation,
                result: json!({"large":"a".repeat(4096)})
            }
        )
        .is_err());
    room.disconnect(1, guest);
    assert!(room.paused());
    assert!(room
        .apply(
            0,
            host,
            Command::Snapshot {
                round: 1,
                generation: old_generation,
                seq: 1,
                data: json!({})
            }
        )
        .is_err());
    let guest = room.connect(1).unwrap();
    assert!(!room.paused());
    assert!(room
        .apply(
            1,
            guest,
            Command::Input {
                round: 1,
                generation: old_generation,
                seq: 1,
                data: json!({})
            }
        )
        .is_err());
    let generation = room.generation;
    room.apply(
        0,
        host,
        Command::Snapshot {
            round: 1,
            generation,
            seq: 1,
            data: json!({"tick":1}),
        },
    )
    .unwrap();
    let mut seen_generation = 0;
    while let Ok(notice) = notices.try_recv() {
        match notice.event {
            Event::State { room } => seen_generation = room.generation,
            Event::Snapshot { generation, .. } => {
                assert_eq!(generation, seen_generation);
                assert_eq!(notice.to, Some(1));
            }
            _ => {}
        }
    }
    room.apply(0, host, Command::Leave).unwrap();
    assert!(room.connect(1).is_err());
}
