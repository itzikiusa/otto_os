//! `WorkGraphService` beyond `record`: the session status feed, the human
//! approval gate, annotations and edges — each must persist, leave exactly the
//! audit event it promises, and broadcast only when it says it does.

use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::{
    ApprovalStatus, EdgeRelation, NewWorkEvent, RiskLevel, WorkActor, WorkGraphRepo, WorkItem,
    WorkItemUpsert, WorkKind, WorkStatus,
};
use otto_workgraph::WorkGraphService;
use serde_json::json;
use tokio::sync::broadcast;

async fn service() -> (WorkGraphService, broadcast::Receiver<Event>) {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(false);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let (tx, rx) = broadcast::channel(64);
    (WorkGraphService::new(WorkGraphRepo::new(pool), tx), rx)
}

fn up(ws: &str, source: &str, kind: WorkKind, status: WorkStatus) -> WorkItemUpsert {
    WorkItemUpsert {
        workspace_id: ws.into(),
        kind,
        source_id: source.into(),
        title: format!("{source} title"),
        goal: None,
        status,
        owner: Some("u1".into()),
        owner_kind: WorkActor::User,
        repo_id: None,
        branch: None,
        cost_so_far: None,
        risk_level: RiskLevel::Medium,
        result_summary: None,
        context_summary: None,
        started_by_id: None,
    }
}

/// Every broadcast waiting in `rx`, as `(item_id, status)`.
fn drain(rx: &mut broadcast::Receiver<Event>) -> Vec<(Id, String)> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let Event::WorkGraphUpdated {
            item_id, status, ..
        } = ev
        {
            out.push((item_id, status));
        }
    }
    out
}

async fn event_types(svc: &WorkGraphService, item: &WorkItem) -> Vec<String> {
    let mut ev = svc.repo().events_for(&item.id, 100).await.unwrap();
    ev.reverse(); // oldest first
    ev.into_iter().map(|e| e.event_type).collect()
}

#[tokio::test]
async fn session_status_feed_moves_the_item_and_audits_real_transitions_only() {
    let (svc, mut rx) = service().await;
    let ws: Id = "w1".into();
    // No item for the session yet: a quiet no-op (the reconcile creates it).
    svc.set_status_by_session(&ws, &"ghost".into(), "idle")
        .await
        .unwrap();
    assert!(drain(&mut rx).is_empty());

    let item = svc
        .record(up("w1", "s1", WorkKind::Session, WorkStatus::Running))
        .await
        .unwrap();
    drain(&mut rx);

    // "working" normalises to Running = unchanged → nothing.
    svc.set_status_by_session(&ws, &"s1".into(), "working")
        .await
        .unwrap();
    assert!(drain(&mut rx).is_empty());

    // idle → Waiting: persisted, audited from→to, broadcast.
    svc.set_status_by_session(&ws, &"s1".into(), "IDLE")
        .await
        .unwrap();
    assert_eq!(drain(&mut rx), [(item.id.clone(), "waiting".to_string())]);
    let now = svc.repo().get_item(&ws, &item.id).await.unwrap();
    assert_eq!(now.status, WorkStatus::Waiting);
    let ev = svc.repo().events_for(&item.id, 10).await.unwrap();
    assert_eq!(ev[0].event_type, "status_changed");
    assert_eq!(ev[0].payload, json!({"from": "running", "to": "waiting"}));
    assert_eq!(ev[0].actor, WorkActor::System);

    // Another workspace's feed never touches this item.
    svc.set_status_by_session(&"w2".into(), &"s1".into(), "exited")
        .await
        .unwrap();
    assert_eq!(
        svc.repo().get_item(&ws, &item.id).await.unwrap().status,
        WorkStatus::Waiting
    );
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn external_trigger_items_follow_their_session_too() {
    let (svc, mut rx) = service().await;
    let item = svc
        .record(up(
            "w1",
            "s9",
            WorkKind::ExternalTrigger,
            WorkStatus::Running,
        ))
        .await
        .unwrap();
    drain(&mut rx);
    svc.set_status_by_session(&"w1".into(), &"s9".into(), "exited")
        .await
        .unwrap();
    assert_eq!(drain(&mut rx), [(item.id, "done".to_string())]);
}

#[tokio::test]
async fn an_approval_is_decided_once_and_only_inside_its_workspace() {
    let (svc, mut rx) = service().await;
    let ws: Id = "w1".into();
    let item = svc
        .record(up("w1", "s1", WorkKind::Session, WorkStatus::Running))
        .await
        .unwrap();
    drain(&mut rx);

    let ap = svc
        .request_approval(&ws, &item.id, Some("prod deploy".into()), "dana")
        .await
        .unwrap();
    assert_eq!(ap.status, ApprovalStatus::Pending);
    assert_eq!(ap.requested_by, "dana");
    assert_eq!(drain(&mut rx).len(), 1, "requesting broadcasts");

    // A caller in another workspace cannot decide it.
    let e = svc
        .decide_approval(&"w2".into(), &ap.id, ApprovalStatus::Approved, "eve", None)
        .await
        .unwrap_err();
    assert!(matches!(e, Error::Conflict(_)), "{e}");
    assert!(drain(&mut rx).is_empty());
    let still = svc.repo().approvals_for(&item.id).await.unwrap();
    assert_eq!(still[0].status, ApprovalStatus::Pending);

    let decided = svc
        .decide_approval(
            &ws,
            &ap.id,
            ApprovalStatus::Rejected,
            "lee",
            Some("not today".into()),
        )
        .await
        .unwrap();
    assert_eq!(decided.status, ApprovalStatus::Rejected);
    assert_eq!(decided.decided_by.as_deref(), Some("lee"));
    assert_eq!(decided.decision_note.as_deref(), Some("not today"));
    assert!(decided.decided_at.is_some());
    assert_eq!(drain(&mut rx).len(), 1);

    // A second decision (e.g. a double-click "Approve") is refused, and the
    // first decision stands.
    assert!(matches!(
        svc.decide_approval(&ws, &ap.id, ApprovalStatus::Approved, "lee", None)
            .await,
        Err(Error::Conflict(_))
    ));
    assert!(drain(&mut rx).is_empty());
    let all = svc.repo().approvals_for(&item.id).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].status, ApprovalStatus::Rejected);

    assert_eq!(
        event_types(&svc, &item).await,
        ["created", "approval_requested", "approval_decided"],
        "the refused decisions left no audit trail"
    );
    let ev = svc.repo().events_for(&item.id, 1).await.unwrap();
    assert_eq!(ev[0].payload["decision"], "rejected");
    assert_eq!(ev[0].payload["by"], "lee");
}

#[tokio::test]
async fn deciding_an_unknown_approval_is_a_conflict_not_a_panic() {
    let (svc, _rx) = service().await;
    assert!(matches!(
        svc.decide_approval(
            &"w1".into(),
            &"nope".into(),
            ApprovalStatus::Approved,
            "x",
            None
        )
        .await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn patching_annotates_without_clobbering_and_unknown_items_are_404() {
    let (svc, mut rx) = service().await;
    let ws: Id = "w1".into();
    let item = svc
        .record(up("w1", "s1", WorkKind::Session, WorkStatus::Running))
        .await
        .unwrap();
    drain(&mut rx);

    let p = svc
        .patch_item(
            &ws,
            &item.id,
            Some(RiskLevel::High),
            Some("ship it".into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(p.risk_level, RiskLevel::High);
    assert_eq!(p.goal.as_deref(), Some("ship it"));
    // `None` leaves a field as it was.
    let p = svc
        .patch_item(&ws, &item.id, None, None, Some("done".into()))
        .await
        .unwrap();
    assert_eq!(p.risk_level, RiskLevel::High);
    assert_eq!(p.goal.as_deref(), Some("ship it"));
    assert_eq!(p.result_summary.as_deref(), Some("done"));
    assert_eq!(drain(&mut rx).len(), 2);
    let ev = svc.repo().events_for(&item.id, 1).await.unwrap();
    assert_eq!(
        ev[0].payload,
        json!({"risk": null, "goal_set": false, "result_set": true})
    );

    // Unknown id, or the right id in the wrong workspace: 404, no side effects.
    for (w, id) in [("w1", "nope"), ("w2", item.id.as_str())] {
        assert!(matches!(
            svc.patch_item(&w.into(), &id.into(), Some(RiskLevel::Low), None, None)
                .await,
            Err(Error::NotFound(_))
        ));
    }
    assert!(drain(&mut rx).is_empty());
    assert_eq!(
        svc.repo().get_item(&ws, &item.id).await.unwrap().risk_level,
        RiskLevel::High
    );
}

#[tokio::test]
async fn edges_are_idempotent_and_only_the_api_variant_broadcasts() {
    let (svc, mut rx) = service().await;
    let ws: Id = "w1".into();
    let a = svc
        .record(up("w1", "a", WorkKind::Session, WorkStatus::Running))
        .await
        .unwrap();
    let b = svc
        .record(up("w1", "b", WorkKind::Review, WorkStatus::Running))
        .await
        .unwrap();
    drain(&mut rx);

    let e1 = svc
        .add_edge(&ws, &a.id, &b.id, EdgeRelation::Spawned)
        .await
        .unwrap();
    assert!(drain(&mut rx).is_empty(), "projector edges stay quiet");
    let e2 = svc
        .add_edge_emit(&ws, &a.id, &b.id, EdgeRelation::Spawned, WorkActor::User)
        .await
        .unwrap();
    assert_eq!(e1.id, e2.id, "same (from, to, relation) → same edge");
    assert_eq!(drain(&mut rx), [(a.id.clone(), "running".to_string())]);
    let ev = svc.repo().events_for(&a.id, 1).await.unwrap();
    assert_eq!(ev[0].event_type, "edge_added");
    assert_eq!(ev[0].payload, json!({"to": b.id, "relation": "spawned"}));

    // A different relation between the same pair is its own edge.
    let e3 = svc
        .add_edge(&ws, &a.id, &b.id, EdgeRelation::Reviews)
        .await
        .unwrap();
    assert_ne!(e3.id, e1.id);
}

#[tokio::test]
async fn high_volume_signals_are_recorded_without_broadcasting() {
    let (svc, mut rx) = service().await;
    let ws: Id = "w1".into();
    let item = svc
        .record(up("w1", "s1", WorkKind::Session, WorkStatus::Running))
        .await
        .unwrap();
    drain(&mut rx);
    svc.ingest_event(NewWorkEvent {
        work_item_id: item.id.clone(),
        workspace_id: ws.clone(),
        actor: WorkActor::Agent,
        event_type: "tool_call".into(),
        payload: json!({"tool": "Bash"}),
    })
    .await
    .unwrap();
    svc.set_cost(&ws, &item.id, 1.25).await.unwrap();
    // A title-only refresh of the same status is quiet as well.
    let mut refresh = up("w1", "s1", WorkKind::Session, WorkStatus::Running);
    refresh.title = "renamed".into();
    let (again, created) = svc.record_created(refresh).await.unwrap();
    assert!(!created);
    assert_eq!(again.id, item.id);
    assert!(drain(&mut rx).is_empty());
    assert_eq!(
        svc.repo()
            .get_item(&ws, &item.id)
            .await
            .unwrap()
            .cost_so_far,
        1.25
    );
    assert_eq!(event_types(&svc, &item).await, ["created", "tool_call"]);
}
