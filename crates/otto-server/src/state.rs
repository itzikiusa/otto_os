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
    ActivityRepo, GitStore, IntegrationsRepo, IssuesRepo, NewNotice, NewTask, NewTrail,
    NotificationsRepo, ReviewsRepo, SkillEvalsRepo, WorkspacesRepo,
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
    /// Daemon version reported by `/meta` (the ottod CARGO_PKG_VERSION).
    pub version: String,
    // -- module handles (wired by ottod at boot) ---------------------------
    pub manager: Arc<SessionManager>,
    pub workspaces: WorkspacesRepo,
    pub connections: Arc<ConnectionsService>,
    /// Native data-access layer for the DB Explorer (browse/query/schema).
    pub db_explorer: Arc<DbViewerService>,
    pub spawner: Arc<dyn Spawner>,
    pub git_store: GitStore,
    pub issues_store: IssuesRepo,
    pub integrations_store: IntegrationsRepo,
    pub reviews_store: ReviewsRepo,
    pub skill_evals_store: SkillEvalsRepo,
    /// Per-run cancellation flags for in-flight skill evaluations.
    pub skill_eval_cancels: crate::skill_eval::CancelRegistry,
    pub orchestrator: Arc<Orchestrator>,
    pub improve_engine: Arc<ImprovementEngine>,
    pub context_library: otto_context::Library,
    /// Embedded ClickHouse usage + metrics store (no-op when unavailable).
    pub usage: Arc<otto_usage::UsageEngine>,
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
        let notice = self.repo.create(new).await?;
        let _ = self.events.send(Event::Notification {
            notice: notice.clone(),
        });
        Ok(notice)
    }

    /// Direct access to the underlying repository (list/read/dismiss/settings).
    pub fn repo(&self) -> &NotificationsRepo {
        &self.repo
    }
}
