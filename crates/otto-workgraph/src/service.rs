//! [`WorkGraphService`] — persist a work-graph mutation, append its audit event,
//! and broadcast the live signal. Cheap to construct (clones a pool handle + a
//! broadcast sender), mirroring `otto_server`'s `ActivityService`.

use otto_core::event::Event;
use otto_core::{Id, Result};
use serde_json::json;
use tokio::sync::broadcast;

use otto_state::{
    ApprovalStatus, EdgeRelation, NewArtifact, NewWorkEvent, RiskLevel, UpsertResult, WorkActor,
    WorkApproval, WorkArtifact, WorkEdge, WorkEvent, WorkGraphRepo, WorkItem, WorkItemUpsert,
};

#[derive(Clone)]
pub struct WorkGraphService {
    repo: WorkGraphRepo,
    events: broadcast::Sender<Event>,
}

impl WorkGraphService {
    pub fn new(repo: WorkGraphRepo, events: broadcast::Sender<Event>) -> Self {
        Self { repo, events }
    }

    /// Direct repo access for read-only queries (list/detail/summary/graph).
    pub fn repo(&self) -> &WorkGraphRepo {
        &self.repo
    }

    fn emit(&self, item: &WorkItem) {
        let _ = self.events.send(Event::WorkGraphUpdated {
            workspace_id: item.workspace_id.clone(),
            item_id: item.id.clone(),
            kind: item.kind.as_str().to_string(),
            status: item.status.as_str().to_string(),
        });
    }

    /// Upsert a projected item. Appends a `created` event on first sight and a
    /// `status_changed` event on a real transition; broadcasts `WorkGraphUpdated`
    /// ONLY on create or status change (cost/title-only refreshes stay quiet, so
    /// the 2 s session-status churn doesn't spam the UI or bloat the audit trail).
    pub async fn record(&self, up: WorkItemUpsert) -> Result<WorkItem> {
        let UpsertResult {
            item,
            created,
            prev_status,
        } = self.repo.upsert_item(&up).await?;
        let status_changed = prev_status.map(|s| s != item.status).unwrap_or(false);
        if created {
            let _ = self
                .repo
                .append_event(&NewWorkEvent {
                    work_item_id: item.id.clone(),
                    workspace_id: item.workspace_id.clone(),
                    actor: up.owner_kind,
                    event_type: "created".into(),
                    payload: json!({ "kind": item.kind.as_str(), "title": item.title }),
                })
                .await;
        } else if status_changed {
            let _ = self
                .repo
                .append_event(&NewWorkEvent {
                    work_item_id: item.id.clone(),
                    workspace_id: item.workspace_id.clone(),
                    actor: WorkActor::System,
                    event_type: "status_changed".into(),
                    payload: json!({
                        "from": prev_status.map(|s| s.as_str()),
                        "to": item.status.as_str(),
                    }),
                })
                .await;
        }
        if created || status_changed {
            self.emit(&item);
        }
        Ok(item)
    }

    /// Append an audit event WITHOUT broadcasting (used for high-volume signals
    /// like tool calls). The detail view picks them up on its next refresh.
    pub async fn ingest_event(&self, ev: NewWorkEvent) -> Result<WorkEvent> {
        self.repo.append_event(&ev).await
    }

    /// Update the status of the work item projecting `session_id` (plain session
    /// or external_trigger), normalizing the raw source status. No-op if the item
    /// doesn't exist yet (a missed create self-heals on the next reconcile) or the
    /// status is unchanged. Emits + records a `status_changed` event on a real
    /// transition. Cheap (no ClickHouse) — safe to call from the event loop.
    pub async fn set_status_by_session(
        &self,
        workspace_id: &Id,
        session_id: &Id,
        raw_status: &str,
    ) -> Result<()> {
        let Some(item) = self
            .repo
            .find_session_item(workspace_id, session_id)
            .await?
        else {
            return Ok(());
        };
        let next = otto_state::WorkStatus::from_source(item.kind, raw_status);
        if next == item.status {
            return Ok(());
        }
        self.repo.set_status(workspace_id, &item.id, next).await?;
        let _ = self
            .repo
            .append_event(&NewWorkEvent {
                work_item_id: item.id.clone(),
                workspace_id: workspace_id.clone(),
                actor: WorkActor::System,
                event_type: "status_changed".into(),
                payload: json!({ "from": item.status.as_str(), "to": next.as_str() }),
            })
            .await;
        let _ = self.events.send(Event::WorkGraphUpdated {
            workspace_id: workspace_id.clone(),
            item_id: item.id.clone(),
            kind: item.kind.as_str().to_string(),
            status: next.as_str().to_string(),
        });
        Ok(())
    }

    /// Link two items (idempotent). Records an `edge_added` event on the source.
    pub async fn add_edge(
        &self,
        workspace_id: &Id,
        from_item_id: &Id,
        to_item_id: &Id,
        relation: EdgeRelation,
    ) -> Result<WorkEdge> {
        let edge = self
            .repo
            .add_edge(workspace_id, from_item_id, to_item_id, relation)
            .await?;
        Ok(edge)
    }

    /// Link two items and emit (used by API). Records an `edge_added` event.
    pub async fn add_edge_emit(
        &self,
        workspace_id: &Id,
        from_item_id: &Id,
        to_item_id: &Id,
        relation: EdgeRelation,
        actor: WorkActor,
    ) -> Result<WorkEdge> {
        let edge = self
            .repo
            .add_edge(workspace_id, from_item_id, to_item_id, relation)
            .await?;
        let _ = self
            .repo
            .append_event(&NewWorkEvent {
                work_item_id: from_item_id.clone(),
                workspace_id: workspace_id.clone(),
                actor,
                event_type: "edge_added".into(),
                payload: json!({ "to": to_item_id, "relation": relation.as_str() }),
            })
            .await;
        let item = self.repo.get_item(workspace_id, from_item_id).await?;
        self.emit(&item);
        Ok(edge)
    }

    /// Attach an artifact (idempotent). Records an `artifact_added` event.
    pub async fn add_artifact(&self, a: NewArtifact) -> Result<WorkArtifact> {
        let art = self.repo.add_artifact(&a).await?;
        let _ = self
            .repo
            .append_event(&NewWorkEvent {
                work_item_id: a.work_item_id.clone(),
                workspace_id: a.workspace_id.clone(),
                actor: WorkActor::System,
                event_type: "artifact_added".into(),
                payload: json!({ "kind": a.kind.as_str(), "title": a.title, "ref": a.reference }),
            })
            .await;
        Ok(art)
    }

    /// Snapshot a cost; no event/broadcast (called from the reconcile sweep).
    pub async fn set_cost(&self, workspace_id: &Id, id: &Id, cost: f64) -> Result<()> {
        self.repo.set_cost(workspace_id, id, cost).await
    }

    /// Open a human-approval gate. Records `approval_requested` + broadcasts.
    pub async fn request_approval(
        &self,
        workspace_id: &Id,
        work_item_id: &Id,
        reason: Option<String>,
        requested_by: &str,
    ) -> Result<WorkApproval> {
        let ap = self
            .repo
            .request_approval(workspace_id, work_item_id, reason.clone(), requested_by)
            .await?;
        let _ = self
            .repo
            .append_event(&NewWorkEvent {
                work_item_id: work_item_id.clone(),
                workspace_id: workspace_id.clone(),
                actor: WorkActor::User,
                event_type: "approval_requested".into(),
                payload: json!({ "approval_id": ap.id, "reason": reason, "by": requested_by }),
            })
            .await;
        if let Ok(item) = self.repo.get_item(workspace_id, work_item_id).await {
            self.emit(&item);
        }
        Ok(ap)
    }

    /// Decide a pending approval. Records `approval_decided` + broadcasts.
    pub async fn decide_approval(
        &self,
        workspace_id: &Id,
        approval_id: &Id,
        status: ApprovalStatus,
        decided_by: &str,
        note: Option<String>,
    ) -> Result<WorkApproval> {
        let ap = self
            .repo
            .decide_approval(workspace_id, approval_id, status, decided_by, note.clone())
            .await?;
        let _ = self
            .repo
            .append_event(&NewWorkEvent {
                work_item_id: ap.work_item_id.clone(),
                workspace_id: workspace_id.clone(),
                actor: WorkActor::User,
                event_type: "approval_decided".into(),
                payload: json!({ "approval_id": ap.id, "decision": status.as_str(), "by": decided_by, "note": note }),
            })
            .await;
        if let Ok(item) = self.repo.get_item(workspace_id, &ap.work_item_id).await {
            self.emit(&item);
        }
        Ok(ap)
    }

    /// Annotate human-governable fields (risk/goal/result). Records `note` + emits.
    pub async fn patch_item(
        &self,
        workspace_id: &Id,
        id: &Id,
        risk: Option<RiskLevel>,
        goal: Option<String>,
        result: Option<String>,
    ) -> Result<WorkItem> {
        let item = self
            .repo
            .patch_item(workspace_id, id, risk, goal.clone(), result.clone())
            .await?;
        let _ = self
            .repo
            .append_event(&NewWorkEvent {
                work_item_id: id.clone(),
                workspace_id: workspace_id.clone(),
                actor: WorkActor::User,
                event_type: "note".into(),
                payload: json!({
                    "risk": risk.map(|r| r.as_str()),
                    "goal_set": goal.is_some(),
                    "result_set": result.is_some(),
                }),
            })
            .await;
        self.emit(&item);
        Ok(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_state::{MissionFilter, WorkKind, WorkStatus};

    async fn pool() -> otto_state::SqlitePool {
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
        pool
    }

    fn up(status: WorkStatus) -> WorkItemUpsert {
        WorkItemUpsert {
            workspace_id: "w1".into(),
            kind: WorkKind::Session,
            source_id: "s1".into(),
            title: "fix login".into(),
            goal: Some("make it green".into()),
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

    #[tokio::test]
    async fn record_emits_on_create_and_status_change_only() {
        let (tx, mut rx) = broadcast::channel(16);
        let svc = WorkGraphService::new(WorkGraphRepo::new(pool().await), tx);

        svc.record(up(WorkStatus::Running)).await.unwrap();
        // create → one broadcast
        assert!(matches!(rx.try_recv(), Ok(Event::WorkGraphUpdated { .. })));

        // same status again → no broadcast
        svc.record(up(WorkStatus::Running)).await.unwrap();
        assert!(rx.try_recv().is_err());

        // status change → broadcast
        svc.record(up(WorkStatus::Done)).await.unwrap();
        assert!(matches!(rx.try_recv(), Ok(Event::WorkGraphUpdated { .. })));

        let items = svc
            .repo()
            .list_items(&"w1".into(), &MissionFilter::default())
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, WorkStatus::Done);
        // created + status_changed events recorded (no event for the no-op upsert)
        let events = svc.repo().events_for(&items[0].id, 50).await.unwrap();
        assert_eq!(events.len(), 2);
    }
}
