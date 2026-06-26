//! Persistence for the **work graph** — Mission Control's unified, traceable
//! model of every agentic activity.
//!
//! A [`WorkItem`] is a PROJECTION of an authoritative module row (a session,
//! swarm, goal loop, workflow run, review, product story, PR, or channel
//! trigger), upserted by the natural key `(workspace_id, kind, source_id)`.
//! [`WorkEdge`]s link items, [`WorkEvent`]s are the append-only audit trail,
//! [`WorkArtifact`]s are evidence/trace, and [`WorkApproval`]s are the human
//! gate. The projection is driven by the daemon event bus + a backfill sweep
//! (see `otto-server`); this module is purely storage + queries.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

// ---------------------------------------------------------------------------
// Enums — string-backed, snake_case on the wire and in the DB.
// ---------------------------------------------------------------------------

/// Generate a string-backed enum with snake_case serde + `as_str`/`parse` that
/// agree exactly with the stored column value.
macro_rules! str_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
        impl $name {
            /// The canonical lowercase string stored in SQLite / JSON.
            pub fn as_str(&self) -> &'static str { match self { $(Self::$variant => $s),+ } }
            /// Parse from the stored string form.
            pub fn parse(s: &str) -> Option<Self> { match s { $($s => Some(Self::$variant),)+ _ => None } }
        }
    };
}

str_enum!(
    /// The eight kinds of work the graph unifies (one per Otto activity type).
    WorkKind {
        Session => "session",
        Swarm => "swarm",
        GoalLoop => "goal_loop",
        Workflow => "workflow",
        Review => "review",
        ProductStory => "product_story",
        Pr => "pr",
        ExternalTrigger => "external_trigger",
        OttoRun => "otto_run",
    }
);

str_enum!(
    /// The normalized lifecycle every source status maps onto.
    WorkStatus {
        Pending => "pending",
        Running => "running",
        Waiting => "waiting",
        Blocked => "blocked",
        Succeeded => "succeeded",
        Failed => "failed",
        Cancelled => "cancelled",
        Done => "done",
    }
);

str_enum!(
    /// How two work items relate (the `work_edges.relation` axis).
    EdgeRelation {
        Spawned => "spawned",
        DependsOn => "depends_on",
        Fixes => "fixes",
        Reviews => "reviews",
        Verifies => "verifies",
        Blocks => "blocks",
        BelongsTo => "belongs_to",
    }
);

str_enum!(
    /// Who/what acted (the `work_events.actor` + `work_items.owner_kind` axis).
    WorkActor {
        User => "user",
        Agent => "agent",
        System => "system",
        Integration => "integration",
    }
);

str_enum!(
    /// The "policy" axis: how risky/sensitive a unit of work is.
    RiskLevel {
        Low => "low",
        Medium => "medium",
        High => "high",
        Critical => "critical",
    }
);

str_enum!(
    /// Kind of evidence/trace artifact attached to a work item.
    ArtifactKind {
        Diff => "diff",
        Commit => "commit",
        Pr => "pr",
        TestRun => "test_run",
        Report => "report",
        File => "file",
        Link => "link",
        Finding => "finding",
        Session => "session",
    }
);

str_enum!(
    /// Lifecycle of a human-approval gate.
    ApprovalStatus {
        Pending => "pending",
        Approved => "approved",
        Rejected => "rejected",
    }
);

impl WorkStatus {
    /// Map a source module's raw status string onto the normalized lifecycle.
    /// Unknown values fall back to [`WorkStatus::Running`] so an item never
    /// disappears from the active view because of an unseen status.
    pub fn from_source(kind: WorkKind, raw: &str) -> WorkStatus {
        use WorkKind::*;
        use WorkStatus::*;
        let r = raw.to_ascii_lowercase();
        match kind {
            Session | ExternalTrigger => match r.as_str() {
                "running" | "working" => Running,
                "idle" | "reconnectable" | "reconnecting" | "waiting" => Waiting,
                "exited" | "done" | "closed" | "archived" => Done,
                _ => Running,
            },
            Swarm => match r.as_str() {
                "queued" | "draft" | "paused" => Pending,
                "running" | "active" => Running,
                "waiting" => Waiting,
                "done" | "succeeded" => Succeeded,
                "error" | "failed" => Failed,
                "stopped" | "aborted" | "cancelled" | "canceled" => Cancelled,
                _ => Running,
            },
            GoalLoop => match r.as_str() {
                "draft" => Pending,
                "running" => Running,
                "paused" | "waiting" => Waiting,
                "blocked" => Blocked,
                "succeeded" => Succeeded,
                "exhausted" | "failed" => Failed,
                "stopped" | "cancelled" | "canceled" => Cancelled,
                _ => Running,
            },
            Workflow => match r.as_str() {
                "pending" | "queued" => Pending,
                "running" => Running,
                "completed" | "success" | "succeeded" => Succeeded,
                "error" | "failed" => Failed,
                "canceled" | "cancelled" => Cancelled,
                _ => Running,
            },
            Review => match r.as_str() {
                "queued" | "pending" => Pending,
                "running" => Running,
                "done" | "succeeded" => Succeeded,
                "error" | "failed" => Failed,
                "cancelled" | "canceled" => Cancelled,
                _ => Running,
            },
            Pr => match r.as_str() {
                "open" | "running" => Running,
                "merged" | "closed" | "done" => Done,
                _ => Running,
            },
            ProductStory => match r.as_str() {
                "done" | "closed" | "shipped" | "released" => Done,
                "blocked" => Blocked,
                _ => Running,
            },
            OttoRun => match r.as_str() {
                "queued" => Pending,
                "awaiting_approval" => Waiting,
                "completed" => Succeeded,
                "failed" => Failed,
                "rejected" | "cancelled" => Cancelled,
                // resolving_source/building_context/provisioning/executing/
                // proving/reviewing/drafting_pr are all "actively working".
                _ => Running,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Row structs
// ---------------------------------------------------------------------------

/// A single unit of agentic work — the spine of Mission Control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: Id,
    pub workspace_id: Id,
    pub kind: WorkKind,
    pub source_id: String,
    pub title: String,
    pub goal: Option<String>,
    pub status: WorkStatus,
    pub owner: Option<String>,
    pub owner_kind: WorkActor,
    pub repo_id: Option<String>,
    pub branch: Option<String>,
    pub cost_so_far: f64,
    pub risk_level: RiskLevel,
    pub result_summary: Option<String>,
    pub context_summary: Option<String>,
    pub started_by_id: Option<String>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A directed relationship between two work items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkEdge {
    pub id: Id,
    pub workspace_id: Id,
    pub from_item_id: Id,
    pub to_item_id: Id,
    pub relation: EdgeRelation,
    pub created_at: DateTime<Utc>,
}

/// An edge joined to its peer item, for the detail view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeView {
    pub relation: EdgeRelation,
    /// `"out"` = this item → peer; `"in"` = peer → this item.
    pub direction: String,
    pub peer_id: Id,
    pub peer_kind: WorkKind,
    pub peer_title: String,
    pub peer_status: WorkStatus,
}

/// One entry in a work item's append-only audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkEvent {
    pub id: Id,
    pub work_item_id: Id,
    pub workspace_id: Id,
    pub ts: DateTime<Utc>,
    pub actor: WorkActor,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A piece of evidence / trace attached to a work item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkArtifact {
    pub id: Id,
    pub work_item_id: Id,
    pub workspace_id: Id,
    pub kind: ArtifactKind,
    pub title: String,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// A human-approval gate on a work item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkApproval {
    pub id: Id,
    pub work_item_id: Id,
    pub workspace_id: Id,
    pub status: ApprovalStatus,
    pub reason: Option<String>,
    pub requested_by: String,
    pub requested_at: DateTime<Utc>,
    pub decided_by: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_note: Option<String>,
}

// ---------------------------------------------------------------------------
// Input / output DTOs
// ---------------------------------------------------------------------------

/// The data a projector hands to [`WorkGraphRepo::upsert_item`]. `goal` and
/// `risk_level` are honored on CREATE only (they're human-governable after
/// that); `cost_so_far`/`result_summary`/`context_summary`/`started_by_id` use
/// COALESCE so `None` leaves an existing value untouched.
#[derive(Debug, Clone)]
pub struct WorkItemUpsert {
    pub workspace_id: Id,
    pub kind: WorkKind,
    pub source_id: String,
    pub title: String,
    pub goal: Option<String>,
    pub status: WorkStatus,
    pub owner: Option<String>,
    pub owner_kind: WorkActor,
    pub repo_id: Option<String>,
    pub branch: Option<String>,
    pub cost_so_far: Option<f64>,
    pub risk_level: RiskLevel,
    pub result_summary: Option<String>,
    pub context_summary: Option<String>,
    pub started_by_id: Option<String>,
}

/// Outcome of an upsert, so the caller knows which audit events to append.
#[derive(Debug, Clone)]
pub struct UpsertResult {
    pub item: WorkItem,
    pub created: bool,
    pub prev_status: Option<WorkStatus>,
}

/// Input for [`WorkGraphRepo::append_event`].
#[derive(Debug, Clone)]
pub struct NewWorkEvent {
    pub work_item_id: Id,
    pub workspace_id: Id,
    pub actor: WorkActor,
    pub event_type: String,
    pub payload: serde_json::Value,
}

/// Input for [`WorkGraphRepo::add_artifact`].
#[derive(Debug, Clone)]
pub struct NewArtifact {
    pub work_item_id: Id,
    pub workspace_id: Id,
    pub kind: ArtifactKind,
    pub title: String,
    pub reference: Option<String>,
    pub payload: serde_json::Value,
}

/// Filter for the Mission Control list / graph.
#[derive(Debug, Clone, Default)]
pub struct MissionFilter {
    pub kind: Option<WorkKind>,
    pub status: Option<WorkStatus>,
    pub risk: Option<RiskLevel>,
    pub q: Option<String>,
    pub limit: Option<i64>,
}

/// The full detail of a work item (the "one traceable unit").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItemDetail {
    #[serde(flatten)]
    pub item: WorkItem,
    pub edges: Vec<EdgeView>,
    pub events: Vec<WorkEvent>,
    pub artifacts: Vec<WorkArtifact>,
    pub approvals: Vec<WorkApproval>,
    pub pending_approvals: i64,
    pub needs_approval: bool,
}

/// `{key, count}` bucket used by the summary breakdowns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountBucket {
    pub key: String,
    pub count: i64,
}

/// Mission Control header summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionSummary {
    pub total: i64,
    pub active: i64,
    pub needs_approval: i64,
    pub total_cost: f64,
    pub by_kind: Vec<CountBucket>,
    pub by_status: Vec<CountBucket>,
    pub by_risk: Vec<CountBucket>,
}

/// A trimmed work item for the graph view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: Id,
    pub kind: WorkKind,
    pub title: String,
    pub status: WorkStatus,
    pub risk_level: RiskLevel,
    pub cost_so_far: f64,
    pub owner_kind: WorkActor,
    pub needs_approval: bool,
}

/// An edge for the graph view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from_item_id: Id,
    pub to_item_id: Id,
    pub relation: EdgeRelation,
}

/// Nodes + edges for the Mission Control graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphView {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

// ---------------------------------------------------------------------------
// Repo
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct WorkGraphRepo {
    pool: SqlitePool,
}

const ITEM_COLS: &str = "id, workspace_id, kind, source_id, title, goal, status, owner, owner_kind, \
    repo_id, branch, cost_so_far, risk_level, result_summary, context_summary, started_by_id, \
    last_event_at, created_at, updated_at";

fn parse_enum<T>(opt: Option<T>, raw: &str, what: &str) -> Result<T> {
    opt.ok_or_else(|| Error::Internal(format!("bad {what} value '{raw}'")))
}

fn opt_ts(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    match s {
        Some(v) => Ok(Some(ts(&v)?)),
        None => Ok(None),
    }
}

fn row_to_item(r: &sqlx::sqlite::SqliteRow) -> Result<WorkItem> {
    let kind_s: String = r.get("kind");
    let status_s: String = r.get("status");
    let owner_kind_s: String = r.get("owner_kind");
    let risk_s: String = r.get("risk_level");
    Ok(WorkItem {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        kind: parse_enum(WorkKind::parse(&kind_s), &kind_s, "kind")?,
        source_id: r.get("source_id"),
        title: r.get("title"),
        goal: r.get("goal"),
        status: parse_enum(WorkStatus::parse(&status_s), &status_s, "status")?,
        owner: r.get("owner"),
        owner_kind: parse_enum(WorkActor::parse(&owner_kind_s), &owner_kind_s, "owner_kind")?,
        repo_id: r.get("repo_id"),
        branch: r.get("branch"),
        cost_so_far: r.get("cost_so_far"),
        risk_level: parse_enum(RiskLevel::parse(&risk_s), &risk_s, "risk_level")?,
        result_summary: r.get("result_summary"),
        context_summary: r.get("context_summary"),
        started_by_id: r.get("started_by_id"),
        last_event_at: opt_ts(r.get("last_event_at"))?,
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_event(r: &sqlx::sqlite::SqliteRow) -> Result<WorkEvent> {
    let actor_s: String = r.get("actor");
    Ok(WorkEvent {
        id: r.get("id"),
        work_item_id: r.get("work_item_id"),
        workspace_id: r.get("workspace_id"),
        ts: ts(&r.get::<String, _>("ts"))?,
        actor: parse_enum(WorkActor::parse(&actor_s), &actor_s, "actor")?,
        event_type: r.get("event_type"),
        payload: json(&r.get::<String, _>("payload_json"))?,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_artifact(r: &sqlx::sqlite::SqliteRow) -> Result<WorkArtifact> {
    let kind_s: String = r.get("kind");
    Ok(WorkArtifact {
        id: r.get("id"),
        work_item_id: r.get("work_item_id"),
        workspace_id: r.get("workspace_id"),
        kind: parse_enum(ArtifactKind::parse(&kind_s), &kind_s, "artifact kind")?,
        title: r.get("title"),
        reference: r.get("ref"),
        payload: json(&r.get::<String, _>("payload_json"))?,
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

fn row_to_approval(r: &sqlx::sqlite::SqliteRow) -> Result<WorkApproval> {
    let status_s: String = r.get("status");
    Ok(WorkApproval {
        id: r.get("id"),
        work_item_id: r.get("work_item_id"),
        workspace_id: r.get("workspace_id"),
        status: parse_enum(ApprovalStatus::parse(&status_s), &status_s, "approval status")?,
        reason: r.get("reason"),
        requested_by: r.get("requested_by"),
        requested_at: ts(&r.get::<String, _>("requested_at"))?,
        decided_by: r.get("decided_by"),
        decided_at: opt_ts(r.get("decided_at"))?,
        decision_note: r.get("decision_note"),
    })
}

impl WorkGraphRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // --- items ----------------------------------------------------------

    /// Find a work item by its projection key.
    pub async fn find_by_source(
        &self,
        workspace_id: &Id,
        kind: WorkKind,
        source_id: &str,
    ) -> Result<Option<WorkItem>> {
        let q = format!(
            "SELECT {ITEM_COLS} FROM work_items WHERE workspace_id = ? AND kind = ? AND source_id = ?"
        );
        let row = sqlx::query(&q)
            .bind(workspace_id)
            .bind(kind.as_str())
            .bind(source_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("find work item by source"))?;
        row.as_ref().map(row_to_item).transpose()
    }

    async fn get_unscoped(&self, id: &Id) -> Result<WorkItem> {
        let q = format!("SELECT {ITEM_COLS} FROM work_items WHERE id = ?");
        let row = sqlx::query(&q)
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("get work item"))?;
        row_to_item(&row)
    }

    /// Fetch one item, scoped to a workspace (404 if absent or other workspace).
    pub async fn get_item(&self, workspace_id: &Id, id: &Id) -> Result<WorkItem> {
        let q = format!("SELECT {ITEM_COLS} FROM work_items WHERE id = ? AND workspace_id = ?");
        let row = sqlx::query(&q)
            .bind(id)
            .bind(workspace_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("work item"))?;
        row_to_item(&row)
    }

    /// Upsert a work item by `(workspace_id, kind, source_id)`. On update,
    /// `goal`/`risk_level` are preserved (human-governable); COALESCE-guarded
    /// fields keep their value when the upsert passes `None`.
    pub async fn upsert_item(&self, up: &WorkItemUpsert) -> Result<UpsertResult> {
        let now = fmt(Utc::now());
        if let Some(prev) = self
            .find_by_source(&up.workspace_id, up.kind, &up.source_id)
            .await?
        {
            sqlx::query(
                "UPDATE work_items SET title = ?, status = ?, owner = ?, owner_kind = ?, \
                 repo_id = COALESCE(?, repo_id), branch = COALESCE(?, branch), \
                 result_summary = COALESCE(?, result_summary), \
                 context_summary = COALESCE(?, context_summary), \
                 cost_so_far = COALESCE(?, cost_so_far), \
                 started_by_id = COALESCE(?, started_by_id), updated_at = ? WHERE id = ?",
            )
            .bind(&up.title)
            .bind(up.status.as_str())
            .bind(&up.owner)
            .bind(up.owner_kind.as_str())
            .bind(&up.repo_id)
            .bind(&up.branch)
            .bind(&up.result_summary)
            .bind(&up.context_summary)
            .bind(up.cost_so_far)
            .bind(&up.started_by_id)
            .bind(&now)
            .bind(&prev.id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update work item"))?;
            let item = self.get_unscoped(&prev.id).await?;
            Ok(UpsertResult {
                item,
                created: false,
                prev_status: Some(prev.status),
            })
        } else {
            let id = new_id();
            sqlx::query(
                "INSERT INTO work_items (id, workspace_id, kind, source_id, title, goal, status, \
                 owner, owner_kind, repo_id, branch, cost_so_far, risk_level, result_summary, \
                 context_summary, started_by_id, created_at, updated_at) \
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&id)
            .bind(&up.workspace_id)
            .bind(up.kind.as_str())
            .bind(&up.source_id)
            .bind(&up.title)
            .bind(&up.goal)
            .bind(up.status.as_str())
            .bind(&up.owner)
            .bind(up.owner_kind.as_str())
            .bind(&up.repo_id)
            .bind(&up.branch)
            .bind(up.cost_so_far.unwrap_or(0.0))
            .bind(up.risk_level.as_str())
            .bind(&up.result_summary)
            .bind(&up.context_summary)
            .bind(&up.started_by_id)
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(dberr("insert work item"))?;
            let item = self.get_unscoped(&id).await?;
            Ok(UpsertResult {
                item,
                created: true,
                prev_status: None,
            })
        }
    }

    /// List items in a workspace, filtered. Newest-updated first.
    pub async fn list_items(&self, workspace_id: &Id, f: &MissionFilter) -> Result<Vec<WorkItem>> {
        let limit = f.limit.unwrap_or(200).clamp(1, 1000);
        let kind = f.kind.map(|k| k.as_str().to_string());
        let status = f.status.map(|s| s.as_str().to_string());
        let risk = f.risk.map(|r| r.as_str().to_string());
        let like = f.q.as_ref().map(|s| format!("%{s}%"));
        let q = format!(
            "SELECT {ITEM_COLS} FROM work_items WHERE workspace_id = ? \
             AND (? IS NULL OR kind = ?) \
             AND (? IS NULL OR status = ?) \
             AND (? IS NULL OR risk_level = ?) \
             AND (? IS NULL OR title LIKE ?) \
             ORDER BY updated_at DESC LIMIT ?"
        );
        let rows = sqlx::query(&q)
            .bind(workspace_id)
            .bind(&kind)
            .bind(&kind)
            .bind(&status)
            .bind(&status)
            .bind(&risk)
            .bind(&risk)
            .bind(&like)
            .bind(&like)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list work items"))?;
        rows.iter().map(row_to_item).collect()
    }

    /// Update the human-governable fields. `None` leaves a field unchanged.
    pub async fn patch_item(
        &self,
        workspace_id: &Id,
        id: &Id,
        risk: Option<RiskLevel>,
        goal: Option<String>,
        result: Option<String>,
    ) -> Result<WorkItem> {
        let now = fmt(Utc::now());
        let res = sqlx::query(
            "UPDATE work_items SET risk_level = COALESCE(?, risk_level), \
             goal = COALESCE(?, goal), result_summary = COALESCE(?, result_summary), \
             updated_at = ? WHERE id = ? AND workspace_id = ?",
        )
        .bind(risk.map(|r| r.as_str()))
        .bind(&goal)
        .bind(&result)
        .bind(&now)
        .bind(id)
        .bind(workspace_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("patch work item"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("work item {id}")));
        }
        self.get_item(workspace_id, id).await
    }

    /// Find the work item that projects a given session, regardless of whether
    /// it was classified as a plain `session` or an `external_trigger`.
    pub async fn find_session_item(
        &self,
        workspace_id: &Id,
        source_id: &str,
    ) -> Result<Option<WorkItem>> {
        let q = format!(
            "SELECT {ITEM_COLS} FROM work_items WHERE workspace_id = ? AND source_id = ? \
             AND kind IN ('session','external_trigger') LIMIT 1"
        );
        let row = sqlx::query(&q)
            .bind(workspace_id)
            .bind(source_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("find session work item"))?;
        row.as_ref().map(row_to_item).transpose()
    }

    /// Set an item's normalized status directly (cheap; no event/broadcast).
    pub async fn set_status(&self, workspace_id: &Id, id: &Id, status: WorkStatus) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE work_items SET status = ?, updated_at = ? WHERE id = ? AND workspace_id = ?",
        )
        .bind(status.as_str())
        .bind(&now)
        .bind(id)
        .bind(workspace_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set work item status"))?;
        Ok(())
    }

    /// Snapshot the running cost (USD) for an item.
    pub async fn set_cost(&self, workspace_id: &Id, id: &Id, cost: f64) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE work_items SET cost_so_far = ?, updated_at = ? WHERE id = ? AND workspace_id = ?",
        )
        .bind(cost)
        .bind(&now)
        .bind(id)
        .bind(workspace_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set work item cost"))?;
        Ok(())
    }

    // --- events ---------------------------------------------------------

    /// Append an audit event and bump the item's `last_event_at`/`updated_at`.
    pub async fn append_event(&self, ev: &NewWorkEvent) -> Result<WorkEvent> {
        let id = new_id();
        let now = fmt(Utc::now());
        let payload = serde_json::to_string(&ev.payload).unwrap_or_else(|_| "{}".to_string());
        sqlx::query(
            "INSERT INTO work_events (id, work_item_id, workspace_id, ts, actor, event_type, \
             payload_json, created_at) VALUES (?,?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(&ev.work_item_id)
        .bind(&ev.workspace_id)
        .bind(&now)
        .bind(ev.actor.as_str())
        .bind(&ev.event_type)
        .bind(&payload)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("append work event"))?;
        sqlx::query("UPDATE work_items SET last_event_at = ?, updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(&now)
            .bind(&ev.work_item_id)
            .execute(&self.pool)
            .await
            .map_err(dberr("touch work item"))?;
        let row = sqlx::query(
            "SELECT id, work_item_id, workspace_id, ts, actor, event_type, payload_json, created_at \
             FROM work_events WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("get work event"))?;
        row_to_event(&row)
    }

    /// Newest-first events for an item (capped).
    pub async fn events_for(&self, work_item_id: &Id, limit: i64) -> Result<Vec<WorkEvent>> {
        let rows = sqlx::query(
            "SELECT id, work_item_id, workspace_id, ts, actor, event_type, payload_json, created_at \
             FROM work_events WHERE work_item_id = ? ORDER BY ts DESC LIMIT ?",
        )
        .bind(work_item_id)
        .bind(limit.clamp(1, 1000))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list work events"))?;
        rows.iter().map(row_to_event).collect()
    }

    // --- edges ----------------------------------------------------------

    /// Add a directed edge (idempotent on the `(from,to,relation)` unique key).
    pub async fn add_edge(
        &self,
        workspace_id: &Id,
        from_item_id: &Id,
        to_item_id: &Id,
        relation: EdgeRelation,
    ) -> Result<WorkEdge> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT OR IGNORE INTO work_edges (id, workspace_id, from_item_id, to_item_id, \
             relation, created_at) VALUES (?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(workspace_id)
        .bind(from_item_id)
        .bind(to_item_id)
        .bind(relation.as_str())
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add work edge"))?;
        let row = sqlx::query(
            "SELECT id, workspace_id, from_item_id, to_item_id, relation, created_at \
             FROM work_edges WHERE from_item_id = ? AND to_item_id = ? AND relation = ?",
        )
        .bind(from_item_id)
        .bind(to_item_id)
        .bind(relation.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("get work edge"))?;
        let rel_s: String = row.get("relation");
        Ok(WorkEdge {
            id: row.get("id"),
            workspace_id: row.get("workspace_id"),
            from_item_id: row.get("from_item_id"),
            to_item_id: row.get("to_item_id"),
            relation: parse_enum(EdgeRelation::parse(&rel_s), &rel_s, "relation")?,
            created_at: ts(&row.get::<String, _>("created_at"))?,
        })
    }

    /// Edges touching an item, joined to the peer item, for the detail view.
    pub async fn edges_for_detail(&self, work_item_id: &Id) -> Result<Vec<EdgeView>> {
        let rows = sqlx::query(
            "SELECT e.relation AS relation, 'out' AS direction, i.id AS peer_id, \
                    i.kind AS peer_kind, i.title AS peer_title, i.status AS peer_status \
             FROM work_edges e JOIN work_items i ON i.id = e.to_item_id WHERE e.from_item_id = ? \
             UNION ALL \
             SELECT e.relation AS relation, 'in' AS direction, i.id AS peer_id, \
                    i.kind AS peer_kind, i.title AS peer_title, i.status AS peer_status \
             FROM work_edges e JOIN work_items i ON i.id = e.from_item_id WHERE e.to_item_id = ?",
        )
        .bind(work_item_id)
        .bind(work_item_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list edges for item"))?;
        rows.iter()
            .map(|r| {
                let rel_s: String = r.get("relation");
                let kind_s: String = r.get("peer_kind");
                let status_s: String = r.get("peer_status");
                Ok(EdgeView {
                    relation: parse_enum(EdgeRelation::parse(&rel_s), &rel_s, "relation")?,
                    direction: r.get("direction"),
                    peer_id: r.get("peer_id"),
                    peer_kind: parse_enum(WorkKind::parse(&kind_s), &kind_s, "kind")?,
                    peer_title: r.get("peer_title"),
                    peer_status: parse_enum(WorkStatus::parse(&status_s), &status_s, "status")?,
                })
            })
            .collect()
    }

    // --- artifacts ------------------------------------------------------

    /// Attach an artifact (idempotent on `(work_item_id, kind, ref)`).
    pub async fn add_artifact(&self, a: &NewArtifact) -> Result<WorkArtifact> {
        let id = new_id();
        let now = fmt(Utc::now());
        let payload = serde_json::to_string(&a.payload).unwrap_or_else(|_| "{}".to_string());
        sqlx::query(
            "INSERT OR IGNORE INTO work_artifacts (id, work_item_id, workspace_id, kind, title, \
             \"ref\", payload_json, created_at) VALUES (?,?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(&a.work_item_id)
        .bind(&a.workspace_id)
        .bind(a.kind.as_str())
        .bind(&a.title)
        .bind(&a.reference)
        .bind(&payload)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add work artifact"))?;
        // Re-select the (new or pre-existing) row for this unique key.
        let row = sqlx::query(
            "SELECT id, work_item_id, workspace_id, kind, title, \"ref\", payload_json, created_at \
             FROM work_artifacts WHERE work_item_id = ? AND kind = ? \
             AND (\"ref\" = ? OR (\"ref\" IS NULL AND ? IS NULL)) ORDER BY created_at ASC LIMIT 1",
        )
        .bind(&a.work_item_id)
        .bind(a.kind.as_str())
        .bind(&a.reference)
        .bind(&a.reference)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("get work artifact"))?;
        row_to_artifact(&row)
    }

    /// Artifacts for an item, newest-first.
    pub async fn artifacts_for(&self, work_item_id: &Id) -> Result<Vec<WorkArtifact>> {
        let rows = sqlx::query(
            "SELECT id, work_item_id, workspace_id, kind, title, \"ref\", payload_json, created_at \
             FROM work_artifacts WHERE work_item_id = ? ORDER BY created_at DESC",
        )
        .bind(work_item_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list work artifacts"))?;
        rows.iter().map(row_to_artifact).collect()
    }

    // --- approvals ------------------------------------------------------

    /// Open a pending approval gate on an item.
    pub async fn request_approval(
        &self,
        workspace_id: &Id,
        work_item_id: &Id,
        reason: Option<String>,
        requested_by: &str,
    ) -> Result<WorkApproval> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO work_approvals (id, work_item_id, workspace_id, status, reason, \
             requested_by, requested_at) VALUES (?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(work_item_id)
        .bind(workspace_id)
        .bind(ApprovalStatus::Pending.as_str())
        .bind(&reason)
        .bind(requested_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("request approval"))?;
        self.get_approval(&id).await
    }

    async fn get_approval(&self, id: &Id) -> Result<WorkApproval> {
        let row = sqlx::query(
            "SELECT id, work_item_id, workspace_id, status, reason, requested_by, requested_at, \
             decided_by, decided_at, decision_note FROM work_approvals WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("approval"))?;
        row_to_approval(&row)
    }

    /// Decide a pending approval. Errors `Conflict` if already decided / absent.
    pub async fn decide_approval(
        &self,
        workspace_id: &Id,
        approval_id: &Id,
        status: ApprovalStatus,
        decided_by: &str,
        note: Option<String>,
    ) -> Result<WorkApproval> {
        let now = fmt(Utc::now());
        let res = sqlx::query(
            "UPDATE work_approvals SET status = ?, decided_by = ?, decided_at = ?, \
             decision_note = ? WHERE id = ? AND workspace_id = ? AND status = 'pending'",
        )
        .bind(status.as_str())
        .bind(decided_by)
        .bind(&now)
        .bind(&note)
        .bind(approval_id)
        .bind(workspace_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("decide approval"))?;
        if res.rows_affected() == 0 {
            return Err(Error::Conflict(format!(
                "approval {approval_id} not pending or not found"
            )));
        }
        self.get_approval(approval_id).await
    }

    /// All approvals for an item, newest-first.
    pub async fn approvals_for(&self, work_item_id: &Id) -> Result<Vec<WorkApproval>> {
        let rows = sqlx::query(
            "SELECT id, work_item_id, workspace_id, status, reason, requested_by, requested_at, \
             decided_by, decided_at, decision_note FROM work_approvals WHERE work_item_id = ? \
             ORDER BY requested_at DESC",
        )
        .bind(work_item_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list approvals"))?;
        rows.iter().map(row_to_approval).collect()
    }

    async fn pending_approval_count(&self, work_item_id: &Id) -> Result<i64> {
        let row =
            sqlx::query("SELECT COUNT(*) AS c FROM work_approvals WHERE work_item_id = ? AND status = 'pending'")
                .bind(work_item_id)
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("count pending approvals"))?;
        Ok(row.get::<i64, _>("c"))
    }

    // --- aggregate views ------------------------------------------------

    /// Assemble the full detail of a work item.
    pub async fn item_detail(&self, workspace_id: &Id, id: &Id) -> Result<WorkItemDetail> {
        let item = self.get_item(workspace_id, id).await?;
        let edges = self.edges_for_detail(id).await?;
        let events = self.events_for(id, 200).await?;
        let artifacts = self.artifacts_for(id).await?;
        let approvals = self.approvals_for(id).await?;
        let pending = self.pending_approval_count(id).await?;
        Ok(WorkItemDetail {
            item,
            edges,
            events,
            artifacts,
            approvals,
            pending_approvals: pending,
            needs_approval: pending > 0,
        })
    }

    async fn count_buckets(&self, workspace_id: &Id, col: &str) -> Result<Vec<CountBucket>> {
        // `col` is a fixed internal identifier (never user input) — safe to format.
        let q = format!(
            "SELECT {col} AS key, COUNT(*) AS count FROM work_items WHERE workspace_id = ? \
             GROUP BY {col} ORDER BY count DESC"
        );
        let rows = sqlx::query(&q)
            .bind(workspace_id)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("count buckets"))?;
        Ok(rows
            .iter()
            .map(|r| CountBucket {
                key: r.get("key"),
                count: r.get("count"),
            })
            .collect())
    }

    /// Mission Control header summary for a workspace.
    pub async fn summary(&self, workspace_id: &Id) -> Result<MissionSummary> {
        let agg = sqlx::query(
            "SELECT COUNT(*) AS total, \
             COALESCE(SUM(cost_so_far), 0.0) AS total_cost, \
             SUM(CASE WHEN status IN ('pending','running','waiting','blocked') THEN 1 ELSE 0 END) AS active \
             FROM work_items WHERE workspace_id = ?",
        )
        .bind(workspace_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("work summary"))?;
        let needs = sqlx::query(
            "SELECT COUNT(DISTINCT a.work_item_id) AS c FROM work_approvals a \
             JOIN work_items i ON i.id = a.work_item_id \
             WHERE a.workspace_id = ? AND a.status = 'pending'",
        )
        .bind(workspace_id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("needs-approval count"))?;
        Ok(MissionSummary {
            total: agg.get::<i64, _>("total"),
            active: agg.get::<Option<i64>, _>("active").unwrap_or(0),
            needs_approval: needs.get::<i64, _>("c"),
            total_cost: agg.get::<f64, _>("total_cost"),
            by_kind: self.count_buckets(workspace_id, "kind").await?,
            by_status: self.count_buckets(workspace_id, "status").await?,
            by_risk: self.count_buckets(workspace_id, "risk_level").await?,
        })
    }

    /// Nodes + edges for the graph view. Edges are included only when BOTH
    /// endpoints are in the returned node set.
    pub async fn graph(&self, workspace_id: &Id, f: &MissionFilter) -> Result<GraphView> {
        let items = self.list_items(workspace_id, f).await?;
        let mut ids = std::collections::HashSet::new();
        let mut nodes = Vec::with_capacity(items.len());
        for it in &items {
            ids.insert(it.id.clone());
            let pending = self.pending_approval_count(&it.id).await?;
            nodes.push(GraphNode {
                id: it.id.clone(),
                kind: it.kind,
                title: it.title.clone(),
                status: it.status,
                risk_level: it.risk_level,
                cost_so_far: it.cost_so_far,
                owner_kind: it.owner_kind,
                needs_approval: pending > 0,
            });
        }
        let rows = sqlx::query(
            "SELECT from_item_id, to_item_id, relation FROM work_edges WHERE workspace_id = ?",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list edges"))?;
        let mut edges = Vec::new();
        for r in &rows {
            let from: Id = r.get("from_item_id");
            let to: Id = r.get("to_item_id");
            if ids.contains(&from) && ids.contains(&to) {
                let rel_s: String = r.get("relation");
                edges.push(GraphEdge {
                    from_item_id: from,
                    to_item_id: to,
                    relation: parse_enum(EdgeRelation::parse(&rel_s), &rel_s, "relation")?,
                });
            }
        }
        Ok(GraphView { nodes, edges })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    fn upsert(ws: &str, kind: WorkKind, src: &str, title: &str, status: WorkStatus) -> WorkItemUpsert {
        WorkItemUpsert {
            workspace_id: ws.into(),
            kind,
            source_id: src.into(),
            title: title.into(),
            goal: Some(format!("goal of {title}")),
            status,
            owner: Some("u1".into()),
            owner_kind: WorkActor::User,
            repo_id: Some("/repo".into()),
            branch: Some("main".into()),
            cost_so_far: Some(1.5),
            risk_level: RiskLevel::Medium,
            result_summary: None,
            context_summary: Some("ctx".into()),
            started_by_id: None,
        }
    }

    #[test]
    fn enum_roundtrip() {
        for s in ["session", "swarm", "goal_loop", "workflow", "review", "product_story", "pr", "external_trigger"] {
            assert_eq!(WorkKind::parse(s).unwrap().as_str(), s);
        }
        assert_eq!(EdgeRelation::parse("belongs_to").unwrap().as_str(), "belongs_to");
        assert_eq!(WorkActor::parse("integration").unwrap().as_str(), "integration");
        assert_eq!(ArtifactKind::parse("test_run").unwrap().as_str(), "test_run");
    }

    #[test]
    fn status_normalization() {
        assert_eq!(WorkStatus::from_source(WorkKind::Session, "Idle"), WorkStatus::Waiting);
        assert_eq!(WorkStatus::from_source(WorkKind::Session, "exited"), WorkStatus::Done);
        assert_eq!(WorkStatus::from_source(WorkKind::Swarm, "error"), WorkStatus::Failed);
        assert_eq!(WorkStatus::from_source(WorkKind::GoalLoop, "exhausted"), WorkStatus::Failed);
        assert_eq!(WorkStatus::from_source(WorkKind::Workflow, "completed"), WorkStatus::Succeeded);
        assert_eq!(WorkStatus::from_source(WorkKind::Review, "done"), WorkStatus::Succeeded);
        // Unknown → Running (never silently drop from the active view).
        assert_eq!(WorkStatus::from_source(WorkKind::Swarm, "weird"), WorkStatus::Running);
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_tracks_status() {
        let repo = WorkGraphRepo::new(mem_pool().await);
        let r1 = repo.upsert_item(&upsert("w1", WorkKind::Session, "s1", "fix login", WorkStatus::Running)).await.unwrap();
        assert!(r1.created);
        assert_eq!(r1.prev_status, None);

        // Second upsert with a new status updates in place (same id), tracks prev.
        let mut up = upsert("w1", WorkKind::Session, "s1", "fix login", WorkStatus::Done);
        up.cost_so_far = None; // COALESCE keeps prior 1.5
        let r2 = repo.upsert_item(&up).await.unwrap();
        assert!(!r2.created);
        assert_eq!(r2.item.id, r1.item.id);
        assert_eq!(r2.prev_status, Some(WorkStatus::Running));
        assert_eq!(r2.item.status, WorkStatus::Done);
        assert_eq!(r2.item.cost_so_far, 1.5);

        let list = repo.list_items(&"w1".into(), &MissionFilter::default()).await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn approval_flow_and_summary() {
        let repo = WorkGraphRepo::new(mem_pool().await);
        let it = repo.upsert_item(&upsert("w1", WorkKind::Review, "r1", "security review", WorkStatus::Running)).await.unwrap().item;
        repo.upsert_item(&upsert("w1", WorkKind::Pr, "repo:42", "PR #42", WorkStatus::Running)).await.unwrap();

        let ap = repo.request_approval(&"w1".into(), &it.id, Some("ship?".into()), "u1").await.unwrap();
        assert_eq!(ap.status, ApprovalStatus::Pending);

        let detail = repo.item_detail(&"w1".into(), &it.id).await.unwrap();
        assert!(detail.needs_approval);
        assert_eq!(detail.pending_approvals, 1);

        let s = repo.summary(&"w1".into()).await.unwrap();
        assert_eq!(s.total, 2);
        assert_eq!(s.needs_approval, 1);
        assert_eq!(s.active, 2);

        let decided = repo.decide_approval(&"w1".into(), &ap.id, ApprovalStatus::Approved, "u2", Some("ok".into())).await.unwrap();
        assert_eq!(decided.status, ApprovalStatus::Approved);
        // Deciding again conflicts.
        assert!(matches!(
            repo.decide_approval(&"w1".into(), &ap.id, ApprovalStatus::Rejected, "u2", None).await,
            Err(Error::Conflict(_))
        ));
        let s2 = repo.summary(&"w1".into()).await.unwrap();
        assert_eq!(s2.needs_approval, 0);
    }

    #[tokio::test]
    async fn edges_events_artifacts_and_graph() {
        let repo = WorkGraphRepo::new(mem_pool().await);
        let review = repo.upsert_item(&upsert("w1", WorkKind::Review, "r1", "review", WorkStatus::Running)).await.unwrap().item;
        let pr = repo.upsert_item(&upsert("w1", WorkKind::Pr, "repo:42", "PR #42", WorkStatus::Running)).await.unwrap().item;

        repo.add_edge(&"w1".into(), &review.id, &pr.id, EdgeRelation::Reviews).await.unwrap();
        // Idempotent.
        repo.add_edge(&"w1".into(), &review.id, &pr.id, EdgeRelation::Reviews).await.unwrap();

        let ev = repo.append_event(&NewWorkEvent {
            work_item_id: review.id.clone(),
            workspace_id: "w1".into(),
            actor: WorkActor::Agent,
            event_type: "tool_call".into(),
            payload: serde_json::json!({"tool": "grep"}),
        }).await.unwrap();
        assert_eq!(ev.event_type, "tool_call");

        repo.add_artifact(&NewArtifact {
            work_item_id: review.id.clone(),
            workspace_id: "w1".into(),
            kind: ArtifactKind::Report,
            title: "verdict".into(),
            reference: None,
            payload: serde_json::json!({"verdict": "block"}),
        }).await.unwrap();

        let detail = repo.item_detail(&"w1".into(), &review.id).await.unwrap();
        assert_eq!(detail.edges.len(), 1);
        assert_eq!(detail.edges[0].peer_id, pr.id);
        assert_eq!(detail.edges[0].direction, "out");
        assert_eq!(detail.events.len(), 1);
        assert_eq!(detail.artifacts.len(), 1);

        let g = repo.graph(&"w1".into(), &MissionFilter::default()).await.unwrap();
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.edges.len(), 1);
    }
}
