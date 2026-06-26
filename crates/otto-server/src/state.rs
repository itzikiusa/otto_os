//! Shared server context (axum state).

use std::sync::Arc;

use otto_connections::{ConnectionsService, Spawner};
use otto_dbviewer::DbViewerService;
use otto_improve::ImprovementEngine;
use otto_core::auth::{RoleChecker, TokenAuthenticator};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_orchestrator::Orchestrator;
use otto_sessions::SessionManager;
use otto_core::domain::{AgentTask, Notice, TrailEvent};
use otto_state::{
    ActivityRepo, AuditRepo, GitStore, IntegrationsRepo, IssuesRepo, NewAuditEntry, NewNotice,
    NewTask, NewTrail, NotificationsRepo, ReviewFindingsRepo, ReviewsRepo, SkillEvalsRepo,
    WorkspacesRepo,
};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

/// Composition-root context cloned into every handler. Implements the ctx
/// traits of the sessions/connections/git routers (see `modules.rs`).
#[derive(Clone)]
pub struct ServerCtx {
    pub pool: SqlitePool,
    pub secrets: Arc<dyn SecretStore>,
    pub events: broadcast::Sender<Event>,
    pub authenticator: Arc<dyn TokenAuthenticator>,
    pub roles: Arc<dyn RoleChecker>,
    /// Shared short-TTL auth-lookup cache (login/api tokens only; share +
    /// impersonation tokens are never cached). Wired into `authenticator`; the
    /// grants route uses it as a `GrantsInvalidator` to flush on `set_grants`.
    pub auth_cache: otto_rbac::AuthCache,
    /// Daemon version reported by `/meta` (the ottod CARGO_PKG_VERSION).
    pub version: String,
    /// Loopback base URL of this daemon, e.g. `http://127.0.0.1:7700`. Used by the
    /// outward MCP-server executor to self-call governed capability endpoints with
    /// a short-lived ephemeral token (so each tool reuses its endpoint's RBAC).
    pub base_url: String,
    /// The daemon data directory (library, swarm worktrees/scratch, …).
    pub data_dir: std::path::PathBuf,
    /// Runtime custom-plugin supervisor (spawns/proxies sidecar processes).
    pub plugins: Arc<crate::plugins::PluginManager>,
    // -- module handles (wired by ottod at boot) ---------------------------
    pub manager: Arc<SessionManager>,
    pub workspaces: WorkspacesRepo,
    pub connections: Arc<ConnectionsService>,
    /// Native data-access layer for the DB Explorer (browse/query/schema).
    pub db_explorer: Arc<DbViewerService>,
    /// In-memory registry of live DB Assistant sessions (`assist_id → entry`).
    /// Ephemeral by design — discarded on close/restart; backs `db_assist.rs`.
    pub db_assist: crate::db_assist::DbAssistRegistry,
    /// Message Brokers (Kafka) viewer engine — cluster CRUD + rdkafka client pool.
    pub brokers: Arc<otto_brokers::BrokersService>,
    /// MCP Control Plane engine — outbound MCP client + governance pipeline
    /// (registry/health/discovery/policy/approval/audit). See `otto-mcp`.
    pub mcp: Arc<otto_mcp::McpService>,
    pub spawner: Arc<dyn Spawner>,
    pub git_store: GitStore,
    pub issues_store: IssuesRepo,
    pub integrations_store: IntegrationsRepo,
    /// Webhook-channel bridge: turns an inbound `POST /webhooks/{ws}` into an
    /// agent session (reuses the channel `Bridge`). Its own instance, separate
    /// from the live Slack/Telegram supervisor. `None` until a root user exists
    /// (onboarding); the inbound handler then returns 503.
    pub channel_bridge: Option<Arc<otto_channels::Bridge>>,
    pub reviews_store: ReviewsRepo,
    /// Persistent review finding identity + lifecycle (A1 verified-review loop).
    pub findings_store: ReviewFindingsRepo,
    pub skill_evals_store: SkillEvalsRepo,
    /// Per-run cancellation flags for in-flight skill evaluations.
    pub skill_eval_cancels: crate::skill_eval::CancelRegistry,
    pub orchestrator: Arc<Orchestrator>,
    pub improve_engine: Arc<ImprovementEngine>,
    pub context_library: otto_context::Library,
    /// Embedded ClickHouse usage + metrics store (no-op when unavailable).
    pub usage: Arc<otto_usage::UsageEngine>,
    pub product: std::sync::Arc<otto_product::ProductService>,
    pub product_repo: otto_state::ProductRepo,
    /// Story attachments (files/images) repo — backs `product_media.rs`.
    pub attachment_repo: otto_state::ProductAttachmentRepo,
    /// Discovery-run repo — repeatable discovery-swarm launches per story.
    pub discovery_repo: otto_state::ProductDiscoveryRepo,
    /// Refinement-thread repo — conversational agent refinement threads per story.
    pub refinement_repo: otto_state::ProductRefinementRepo,
    /// Mockup pinned-annotation repo.
    pub mockup_repo: otto_state::ProductMockupRepo,
    /// Discovery-chat repo — lightweight conversational discovery threads per story.
    pub discovery_chat_repo: otto_state::DiscoveryChatRepo,
    /// Canvas Studio scene repo — visual JSON-node scenes (workspace-scoped).
    pub canvas_repo: otto_state::CanvasRepo,
    /// Per-run cancellation flags for in-flight product analysis agents (manual
    /// Stop). Mirrors `skill_eval_cancels`.
    pub product_agent_cancels: crate::product_run::CancelRegistry,
    /// Memory layer — workspace-scoped keyword+vector knowledge store.
    pub memory: std::sync::Arc<otto_memory::MemoryService>,
    // -- Agent Swarm -------------------------------------------------------
    pub swarm: Arc<otto_swarm::SwarmService>,
    pub swarm_repo: otto_state::SwarmRepo,
    /// Per-swarm Coordinator runtime handles (start/pause/abort/resume).
    pub swarm_coords: crate::swarm_runtime::CoordinatorRegistry,
    /// Per-run cancellation flags for in-flight swarm runs (manual Stop / abort).
    pub swarm_run_cancels: crate::swarm_run::CancelRegistry,
    // -- Goal Loops --------------------------------------------------------
    pub goal_loops_repo: otto_state::GoalLoopsRepo,
    /// Per-loop controller runtime handles (start/pause/resume/stop).
    pub goal_loops: crate::goal_loop::GoalLoopRegistry,
    // -- Mission Control / work graph --------------------------------------
    /// Unified work-graph service (persist + emit). The projector
    /// (`workgraph_projector`) feeds it from the event bus; the routes read it.
    pub workgraph: std::sync::Arc<otto_workgraph::WorkGraphService>,
    // -- Proof Packs -------------------------------------------------------
    pub proof_repo: otto_state::ProofRepo,
    /// Per-pack async locks serializing status/risk recompute so concurrent
    /// artifact adds / gates can't interleave a stale read→write (no lost-update).
    pub proof_locks: crate::proof::ProofLocks,
}

impl ServerCtx {
    /// Notification service bound to this context's DB pool and event bus.
    ///
    /// Wave-2 producers (the credential monitor, session-event hooks, …) call
    /// `ctx.notifications().create(..)` to persist a notice *and* push it live
    /// over `Event::Notification`.
    pub fn notifications(&self) -> NotificationService {
        NotificationService {
            repo: NotificationsRepo::new(self.pool.clone()),
            events: self.events.clone(),
        }
    }

    /// Agent-activity service (live trail + task tracker) bound to this
    /// context's DB pool and event bus. Persists an entry and pushes the
    /// matching live event in one call.
    pub fn activity(&self) -> ActivityService {
        ActivityService {
            repo: ActivityRepo::new(self.pool.clone()),
            events: self.events.clone(),
        }
    }

    /// Append a security-audit entry, BEST-EFFORT: a failure is logged and
    /// swallowed so it can never fail the request being audited. Call it at
    /// sensitive sites (login, token mint/revoke, settings & network-listener
    /// changes, confirmed DB writes); the Trust & Safety Center reads the
    /// resulting ledger via `GET /audit-log`. The audit table is append-only —
    /// [`AuditRepo`] exposes no update/delete.
    pub async fn audit(&self, entry: NewAuditEntry) {
        let action = entry.action.clone();
        if let Err(e) = AuditRepo::new(self.pool.clone()).insert(entry).await {
            tracing::warn!(%action, "audit insert failed (best-effort): {e}");
        }
    }
}

/// Persists agent-activity rows and broadcasts the matching live event. Cheap
/// to construct (clones a pool handle + broadcast sender); created on demand via
/// [`ServerCtx::activity`].
#[derive(Clone)]
pub struct ActivityService {
    repo: ActivityRepo,
    events: broadcast::Sender<Event>,
}

impl ActivityService {
    /// Append a trail entry and broadcast `Event::TrailAppended`.
    pub async fn append_trail(&self, new: NewTrail) -> otto_core::Result<TrailEvent> {
        let workspace_id = new.workspace_id.clone();
        let session_id = new.session_id.clone();
        let event = self.repo.append_trail(new).await?;
        let _ = self.events.send(Event::TrailAppended {
            workspace_id,
            session_id,
            event: event.clone(),
        });
        Ok(event)
    }

    /// Replace a session's task list and broadcast `Event::TasksUpdated`.
    pub async fn put_tasks(
        &self,
        session_id: &otto_core::Id,
        workspace_id: &otto_core::Id,
        tasks: &[NewTask],
    ) -> otto_core::Result<Vec<AgentTask>> {
        let tasks = self.repo.replace_tasks(session_id, workspace_id, tasks).await?;
        let _ = self.events.send(Event::TasksUpdated {
            workspace_id: workspace_id.clone(),
            session_id: session_id.clone(),
            tasks: tasks.clone(),
        });
        Ok(tasks)
    }

    /// Direct repo access (list operations).
    pub fn repo(&self) -> &ActivityRepo {
        &self.repo
    }
}

/// Persists a notice and broadcasts it on the event bus in one call. Cheap to
/// construct (clones a pool handle + a broadcast sender), so it is created
/// on demand via [`ServerCtx::notifications`].
#[derive(Clone)]
pub struct NotificationService {
    repo: NotificationsRepo,
    events: broadcast::Sender<Event>,
}

impl NotificationService {
    /// Build a service from a DB pool + event bus directly. Lets producers that
    /// exist *before* the full [`ServerCtx`] is assembled (e.g. the session
    /// output scanner attached to the `SessionManager`) emit notices.
    pub fn new(pool: SqlitePool, events: broadcast::Sender<Event>) -> Self {
        Self {
            repo: NotificationsRepo::new(pool),
            events,
        }
    }

    /// Create (de-duping on `source_key`) and broadcast `Event::Notification`.
    /// Returns the persisted notice. A broadcast with no live subscribers is
    /// not an error.
    pub async fn create(&self, new: NewNotice) -> otto_core::Result<Notice> {
        // The notice owner drives per-user WS delivery (None = global notice).
        let user_id = new.user_id.clone();
        let notice = self.repo.create(new).await?;
        let _ = self.events.send(Event::Notification {
            notice: notice.clone(),
            user_id,
        });
        Ok(notice)
    }

    /// Direct access to the underlying repository (list/read/dismiss/settings).
    pub fn repo(&self) -> &NotificationsRepo {
        &self.repo
    }
}
