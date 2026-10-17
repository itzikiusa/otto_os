use super::{registry::*, types::*};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn config(game: GameKind) -> Config {
    let map = game.maps()[0].into();
    Config { game, map }
}
/// A room with `guests` joined guests (not yet connected).
async fn room_with(
    game: GameKind,
    guests: usize,
) -> (Registry, Credential, Vec<Credential>, SharedRoom) {
    let registry = Registry::default();
    let host = registry
        .create("owner", "Host", config(game))
        .await
        .unwrap();
    let invite = host.invite.clone().unwrap();
    let mut joined = vec![];
    for i in 0..guests {
        joined.push(
            registry
                .join(&host.room_id, &invite, &format!("Guest {i}"))
                .await
                .unwrap(),
        );
    }
    let room = registry.get(&host.room_id).await.unwrap();
    (registry, host, joined, room)
}
/// Connects and readies every member, starts the match; returns connection ids.
fn start_all(room: &mut Room) -> Vec<u64> {
    let ids: Vec<u64> = (0..room.members.len())
        .map(|i| room.connect(i).unwrap())
        .collect();
    for (i, id) in ids.iter().enumerate() {
        room.apply(i, *id, Command::Ready { ready: true }).unwrap();
    }
    let (round, generation) = (room.round, room.generation);
    room.apply(0, ids[0], Command::Start { round, generation })
        .unwrap();
    ids
}
fn snapshot(room: &Room, seq: u64, data: Value) -> Command {
    let (round, generation) = (room.round, room.generation);
    Command::Snapshot {
        round,
        generation,
        seq,
        data,
    }
}
fn input(room: &Room, seq: u64, data: Value) -> Command {
    let (round, generation) = (room.round, room.generation);
    Command::Input {
        round,
        generation,
        seq,
        data,
    }
}
fn finish(room: &Room) -> Command {
    let (round, generation) = (room.round, room.generation);
    Command::Finish {
        round,
        generation,
        result: json!({"winner":0}),
    }
}

#[tokio::test]
async fn invite_admits_guests_up_to_each_game_capacity() {
    for (game, capacity) in [(GameKind::Shooter, 4), (GameKind::Kart, 8)] {
        let (reg, host, guests, room) = room_with(game, capacity - 1).await;
        let invite = host.invite.as_deref().unwrap();
        assert!(reg.join(&host.room_id, invite, "Extra").await.is_err());
        let slots: Vec<usize> = room
            .lock()
            .await
            .view()
            .members
            .iter()
            .map(|m| m.slot)
            .collect();
        assert_eq!(slots, (0..capacity).collect::<Vec<_>>());
        let last = guests.last().unwrap();
        assert_eq!(
            reg.authenticate(&host.room_id, &last.token).await.unwrap(),
            capacity - 1
        );
        assert!(reg.authenticate(&host.room_id, invite).await.is_err());
        let other = reg.create("other", "Other", config(game)).await.unwrap();
        assert!(reg.authenticate(&other.room_id, &last.token).await.is_err());
        assert!(reg
            .join(&host.room_id, &"x".repeat(43), "Bad")
            .await
            .is_err());
    }
}
#[tokio::test]
async fn joins_close_once_the_match_starts_and_entries_are_bounded() {
    let (reg, host, _, shared) = room_with(GameKind::Kart, 1).await;
    let invite = host.invite.clone().unwrap();
    start_all(&mut *shared.lock().await);
    assert!(reg.join(&host.room_id, &invite, "Late").await.is_err());
    // Guests leaving in the lobby free capacity, but the entry list stays bounded.
    let (reg, host, _, shared) = room_with(GameKind::Shooter, 0).await;
    let invite = host.invite.clone().unwrap();
    for i in 1..MAX_MEMBER_ENTRIES {
        reg.join(&host.room_id, &invite, "Guest").await.unwrap();
        let mut room = shared.lock().await;
        let id = room.connect(i).unwrap();
        room.apply(i, id, Command::Leave).unwrap();
    }
    assert!(reg.join(&host.room_id, &invite, "Overflow").await.is_err());
}
#[tokio::test]
async fn guest_leave_keeps_the_room_and_host_leave_closes_it() {
    let (reg, host, guests, shared) = room_with(GameKind::Kart, 2).await;
    let mut room = shared.lock().await;
    let ids = start_all(&mut room);
    room.apply(1, ids[1], Command::Leave).unwrap();
    assert!(room.members[1].view.left && room.phase == Phase::Playing && !room.paused());
    assert!(room.connect(1).is_err());
    drop(room);
    assert!(reg
        .authenticate(&host.room_id, &guests[0].token)
        .await
        .is_err());
    let mut room = shared.lock().await;
    let seq = snapshot(&room, 1, json!({"tick":1}));
    room.apply(0, ids[0], seq).unwrap();
    room.apply(0, ids[0], Command::Leave).unwrap();
    assert_eq!(room.phase, Phase::Closed);
}
#[tokio::test]
async fn grace_expiry_drops_guests_but_closes_for_the_host() {
    let (_, _, _, shared) = room_with(GameKind::Kart, 2).await;
    let mut room = shared.lock().await;
    let ids = start_all(&mut room);
    room.disconnect(2, ids[2]);
    assert!(room.paused());
    room.maintain(Instant::now() + Duration::from_secs(21));
    assert!(room.members[2].view.left && !room.paused() && room.phase == Phase::Playing);
    room.disconnect(0, ids[0]);
    room.maintain(Instant::now() + Duration::from_secs(21));
    assert_eq!(room.phase, Phase::Closed);
}
#[tokio::test]
async fn snapshots_fan_out_to_guests_and_inputs_reach_the_host_with_slot() {
    let (_, _, _, shared) = room_with(GameKind::Kart, 3).await;
    let mut room = shared.lock().await;
    let mut notices = room.tx.subscribe();
    let ids = start_all(&mut room);
    let packet = snapshot(&room, 1, json!({"tick":1}));
    room.apply(0, ids[0], packet).unwrap();
    let packet = input(&room, 1, json!({"steer":1}));
    room.apply(2, ids[2], packet).unwrap();
    let packet = input(&room, 1, json!({}));
    assert!(
        room.apply(0, ids[0], packet).is_err(),
        "host never relays input"
    );
    let packet = snapshot(&room, 1, json!({}));
    assert!(
        room.apply(1, ids[1], packet).is_err(),
        "guests never relay snapshots"
    );
    let (mut snapshots, mut inputs) = (0, 0);
    while let Ok(notice) = notices.try_recv() {
        match notice.event {
            Event::Snapshot { .. } => {
                snapshots += 1;
                assert!(!notice.to.includes(0) && (1..4).all(|i| notice.to.includes(i)));
            }
            Event::Input { slot, .. } => {
                inputs += 1;
                assert_eq!((slot, notice.to), (2, Target::One(0)));
            }
            _ => {}
        }
    }
    assert_eq!((snapshots, inputs), (1, 1));
}
#[tokio::test]
async fn start_requires_two_ready_members_and_the_host() {
    let (_, _, _, shared) = room_with(GameKind::Shooter, 0).await;
    let mut room = shared.lock().await;
    let h = room.connect(0).unwrap();
    room.apply(0, h, Command::Ready { ready: true }).unwrap();
    let (round, generation) = (room.round, room.generation);
    assert!(room
        .apply(0, h, Command::Start { round, generation })
        .is_err());
    drop(room);
    let (_, _, _, shared) = room_with(GameKind::Shooter, 2).await;
    let mut room = shared.lock().await;
    let ids: Vec<u64> = (0..3).map(|i| room.connect(i).unwrap()).collect();
    room.apply(0, ids[0], Command::Ready { ready: true })
        .unwrap();
    room.apply(1, ids[1], Command::Ready { ready: true })
        .unwrap();
    let (round, generation) = (room.round, room.generation);
    assert!(room
        .apply(0, ids[0], Command::Start { round, generation })
        .is_err());
    room.apply(2, ids[2], Command::Ready { ready: true })
        .unwrap();
    assert!(room
        .apply(1, ids[1], Command::Start { round, generation })
        .is_err());
    room.apply(0, ids[0], Command::Start { round, generation })
        .unwrap();
    let stale = Command::Input {
        round: 0,
        generation,
        seq: 1,
        data: json!({}),
    };
    assert!(room.apply(1, ids[1], stale).is_err());
    let packet = input(&room, 1, json!({"f":1}));
    room.apply(1, ids[1], packet).unwrap();
    let replay = input(&room, 1, json!({}));
    assert!(room.apply(1, ids[1], replay).is_err());
}
#[tokio::test]
async fn host_rematch_resets_the_round_and_guest_rematch_only_flags() {
    let (_, _, _, shared) = room_with(GameKind::Kart, 1).await;
    let mut room = shared.lock().await;
    let ids = start_all(&mut room);
    assert!(room.apply(1, ids[1], Command::Rematch).is_err());
    let done = finish(&room);
    assert!(room.apply(1, ids[1], done).is_err());
    let done = finish(&room);
    room.apply(0, ids[0], done).unwrap();
    room.apply(1, ids[1], Command::Rematch).unwrap();
    assert!(room.members[1].view.rematch && room.round == 1);
    room.apply(0, ids[0], Command::Rematch).unwrap();
    assert_eq!((room.round, room.phase), (2, Phase::Lobby));
    assert!(room
        .members
        .iter()
        .all(|m| !m.view.ready && !m.view.rematch));
}
#[tokio::test]
async fn reconnect_invalidates_packets_and_old_socket_cleanup() {
    let (_, _, _, room) = room_with(GameKind::Shooter, 1).await;
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
    let wrong = Config {
        game: GameKind::Kart,
        map: "isle".into(),
    };
    assert!(reg.create("owner", "Host", wrong).await.is_err());
    let (_, _, _, shared) = room_with(GameKind::Shooter, 1).await;
    let mut room = shared.lock().await;
    let ids = start_all(&mut room);
    let large = snapshot(&room, 1, json!({"large":"a".repeat(16384)}));
    assert!(room.apply(0, ids[0], large).is_err());
    let large = input(&room, 1, json!({"large":"a".repeat(1024)}));
    assert!(room.apply(1, ids[1], large).is_err());
    for _ in 0..20 {
        let _ = room.apply(0, ids[0], Command::Ping);
    }
    assert!(room.apply(0, ids[0], Command::Ping).is_err());
}
#[tokio::test]
async fn concurrent_redemption_never_exceeds_capacity() {
    let registry = std::sync::Arc::new(Registry::default());
    let host = registry
        .create("owner", "Host", config(GameKind::Shooter))
        .await
        .unwrap();
    let invite = host.invite.clone().unwrap();
    let joins = (0..6).map(|i| {
        let (registry, id, invite) = (registry.clone(), host.room_id.clone(), invite.clone());
        tokio::spawn(async move { registry.join(&id, &invite, &format!("G{i}")).await.is_ok() })
    });
    let mut admitted = 0;
    for join in joins {
        admitted += join.await.unwrap() as usize;
    }
    assert_eq!(admitted, 3);
    let room = registry.get(&host.room_id).await.unwrap();
    assert_eq!(room.lock().await.members.len(), 4);
}
#[tokio::test]
async fn capacity_and_abandoned_rooms_are_bounded() {
    let registry = Registry::default();
    let cfg = || config(GameKind::Shooter);
    let first = registry.create("owner", "Host", cfg()).await.unwrap();
    registry.create("owner", "Host", cfg()).await.unwrap();
    assert!(registry.create("owner", "Host", cfg()).await.is_err());
    let room = registry.get(&first.room_id).await.unwrap();
    room.lock()
        .await
        .maintain(Instant::now() + Duration::from_secs(21));
    assert!(registry.create("owner", "Host", cfg()).await.is_ok());
    assert!(registry.get(&first.room_id).await.is_err());
    for i in 0..30 {
        registry
            .create(&format!("owner{i}"), "Host", cfg())
            .await
            .unwrap();
    }
    assert!(registry.create("overflow", "Host", cfg()).await.is_err());
}
#[tokio::test]
async fn pause_and_reconnect_reject_old_generation_packets() {
    let (_, _, _, shared) = room_with(GameKind::Shooter, 1).await;
    let mut room = shared.lock().await;
    let mut notices = room.tx.subscribe();
    let ids = start_all(&mut room);
    let old = room.generation;
    room.disconnect(1, ids[1]);
    assert!(room.paused());
    let paused = snapshot(&room, 1, json!({}));
    assert!(room.apply(0, ids[0], paused).is_err());
    let guest = room.connect(1).unwrap();
    assert!(!room.paused());
    let stale = Command::Input {
        round: 1,
        generation: old,
        seq: 1,
        data: json!({}),
    };
    assert!(room.apply(1, guest, stale).is_err());
    let fresh = snapshot(&room, 1, json!({"tick":1}));
    room.apply(0, ids[0], fresh).unwrap();
    let mut seen = 0;
    while let Ok(notice) = notices.try_recv() {
        match notice.event {
            Event::State { room } => seen = room.generation,
            Event::Snapshot { generation, .. } => assert_eq!(generation, seen),
            _ => {}
        }
    }
}
