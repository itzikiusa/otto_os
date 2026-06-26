//! `GET /ws/events` — event stream WebSocket (see docs/contracts/ws.md).
//!
//! Auth: the bearer token is read from the `Sec-WebSocket-Protocol` header
//! (the browser sends `["otto-bearer", "<token>"]`; we validate the token and
//! echo back the `otto-bearer` subprotocol). This keeps the token out of the
//! request URL, which is logged everywhere. A legacy `?token=` query parameter
//! is still accepted as a fallback. Validation happens BEFORE the upgrade (401
//! otherwise). Session-scoped events are delivered only to the session's
//! **owner**, a workspace **Admin** of the session's workspace, or root — and
//! only after the workspace-Viewer membership gate passes (so a non-member
//! never reaches the owner check). `Notice` events go to every authenticated
//! client; `Notification` events are delivered per the notice's owner. A Ping
//! frame is sent every 30 seconds.

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use otto_core::auth::{AuthContext, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::{Error, Id};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::error::ApiError;
use crate::state::ServerCtx;

/// Fixed first subprotocol the browser offers alongside the token; echoed back
/// on a successful upgrade so the handshake completes.
const BEARER_SUBPROTOCOL: &str = "otto-bearer";

#[derive(Debug, Deserialize)]
pub struct TokenQuery {
    token: Option<String>,
}

pub async fn events_ws(
    ws: WebSocketUpgrade,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    State(ctx): State<ServerCtx>,
) -> Response {
    // Prefer the bearer subprotocol; fall back to the legacy `?token=` query.
    let subprotocol_token = token_from_subprotocol(&headers);
    let used_subprotocol = subprotocol_token.is_some();
    let Some(token) = subprotocol_token.or(query.token) else {
        return ApiError(Error::Unauthorized).into_response();
    };
    match ctx.authenticator.authenticate(&token).await {
        Ok(auth) => {
            // Scoped (share-link) tokens get ZERO event-stream access: `/ws/events`
            // is workspace-scoped and would leak every sibling session's status,
            // trail, and tasks. A share token only ever sees its one session's live
            // terminal via `/ws/term`. Refuse the upgrade with 403 (deny-by-default).
            if scope_denied(&auth) {
                return ApiError(Error::Forbidden(
                    "share-scoped tokens cannot subscribe to the event stream".into(),
                ))
                .into_response();
            }
            // Authorize against the effective user (== real for a normal token).
            let user = auth.effective_user;
            // Echo `otto-bearer` only when the client used the subprotocol path,
            // otherwise the browser would reject an unsolicited subprotocol.
            if used_subprotocol {
                ws.protocols([BEARER_SUBPROTOCOL])
                    .on_upgrade(move |socket| handle_events(socket, ctx, user))
            } else {
                ws.on_upgrade(move |socket| handle_events(socket, ctx, user))
            }
        }
        Err(_) => ApiError(Error::Unauthorized).into_response(),
    }
}

/// Extract the bearer token from a `Sec-WebSocket-Protocol: otto-bearer, <token>`
/// request header. Returns `None` when the header is absent or not in that form.
fn token_from_subprotocol(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)?
        .to_str()
        .ok()?;
    // The header is a comma-separated list; the browser sends the fixed marker
    // first and the token second.
    let mut parts = raw.split(',').map(str::trim);
    if parts.next()? != BEARER_SUBPROTOCOL {
        return None;
    }
    let token = parts.next()?;
    (!token.is_empty()).then(|| token.to_string())
}

/// True iff this authenticated context must be denied `/ws/events` outright: a
/// scoped (`kind='share'`) token is pinned to a single session's terminal and is
/// never allowed onto the workspace-wide event stream. Pure (no I/O) so the
/// pre-upgrade denial is exercised directly by the unit test below — building a
/// full `ServerCtx` to drive `events_ws` end-to-end is infeasible in tests.
fn scope_denied(auth: &AuthContext) -> bool {
    auth.is_scoped()
}

async fn handle_events(socket: WebSocket, ctx: ServerCtx, user: User) {
    let mut events = ctx.events.subscribe();
    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.tick().await; // consume the immediate first tick

    // Role-check results cached per workspace for this connection's lifetime.
    let mut role_cache: HashMap<Id, bool> = HashMap::new();
    // Session owner (`created_by`) cached per session_id for this connection's
    // lifetime. `created_by` is immutable, so one lookup per session_id is
    // enough — this keeps the high-frequency `TrailAppended` path off the DB.
    let mut owner_cache: HashMap<Id, Option<Id>> = HashMap::new();

    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => {
                    if !allowed(&ctx, &user, &event, &mut role_cache, &mut owner_cache).await {
                        continue;
                    }
                    let Ok(text) = serde_json::to_string(&event) else { continue };
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!("events ws lagged, skipped {skipped} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                if sink.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => match incoming {
                // Client→server messages on this socket are ignored.
                Some(Ok(_)) => continue,
                _ => break,
            },
        }
    }
}

/// How a single event is scoped on the wire.
enum Scope<'a> {
    /// Every authenticated client receives it (free-form `Notice` toasts).
    Everyone,
    /// Per-user: a global notice (`None`) → everyone; an owned notice → that
    /// user (root sees all), mirroring the REST `NoticeAccess` policy.
    User(&'a Option<Id>),
    /// Workspace-scoped, delivered to any member with viewer+ on `workspace_id`
    /// (Improvement + Swarm events). No owner axis.
    Workspace(&'a Id),
    /// Session-family event: viewer membership on `workspace_id` is required AND
    /// the recipient must be the session's `owner`, a workspace Admin, or root.
    /// `owner` is `SessionCreated`'s `session.created_by` when known up front;
    /// `None` means it must be resolved from `session_id` (with the per-conn
    /// cache) inside `allowed`.
    Session {
        workspace_id: &'a Id,
        session_id: &'a Id,
        owner: Option<&'a Id>,
    },
}

/// Classify an event into its delivery [`Scope`]. Pure (no I/O), so the routing
/// table is exercised directly by the unit tests below.
fn scope_of(event: &Event) -> Scope<'_> {
    match event {
        Event::Notice { .. } => Scope::Everyone,
        Event::Notification { user_id, .. } => Scope::User(user_id),
        // Session-family events carry `session_id` + `workspace_id`; the owner is
        // resolved lazily. `SessionCreated` carries the full `Session`, so its
        // owner (`created_by`) is known without a lookup.
        Event::SessionStatus {
            session_id,
            workspace_id,
            ..
        }
        | Event::SessionMetaUpdated {
            session_id,
            workspace_id,
            ..
        }
        | Event::TrailAppended {
            session_id,
            workspace_id,
            ..
        }
        | Event::TasksUpdated {
            session_id,
            workspace_id,
            ..
        } => Scope::Session {
            workspace_id,
            session_id,
            owner: None,
        },
        Event::SessionRemoved {
            session_id,
            workspace_id,
        } => Scope::Session {
            workspace_id,
            session_id,
            owner: None,
        },
        Event::SessionCreated { session } => Scope::Session {
            workspace_id: &session.workspace_id,
            session_id: &session.id,
            owner: Some(&session.created_by),
        },
        // Improvement (self-reflection) events are workspace-scoped — gate them
        // on viewer access to that workspace, but with no owner axis.
        Event::ImprovementRunStarted { workspace_id, .. }
        | Event::ImprovementRunFinished { workspace_id, .. }
        | Event::ImprovementEditApplied { workspace_id, .. }
        | Event::ImprovementApprovalPending { workspace_id, .. }
        // Agent-swarm events are workspace-scoped, like the improvement events.
        | Event::SwarmRunUpdated { workspace_id, .. }
        | Event::SwarmTaskUpdated { workspace_id, .. }
        | Event::SwarmMessagePosted { workspace_id, .. }
        | Event::SwarmStatus { workspace_id, .. }
        // Product events are workspace-scoped.
        | Event::ProductChanged { workspace_id, .. }
        | Event::PlanRun { workspace_id, .. }
        // Review, workflow, and skill-eval events are workspace-scoped too.
        | Event::ReviewChanged { workspace_id, .. }
        | Event::GoalLoopUpdated { workspace_id, .. }
        | Event::WorkflowRunUpdated { workspace_id, .. }
        | Event::SkillEvalUpdated { workspace_id, .. }
        // Live canvas-document edits + the agent-session-started signal go to the
        // scene's workspace members.
        | Event::CanvasUpdated { workspace_id, .. }
        | Event::CanvasSessionStarted { workspace_id, .. }
        // Live mockup-source edits + the mockup-agent-started signal go to the
        // story's workspace members (same delivery as canvas).
        | Event::MockupUpdated { workspace_id, .. }
        | Event::MockupSessionStarted { workspace_id, .. }
        // Live DB-Assistant answer edits + the assist-agent-started signal go to the
        // connection's workspace members (same delivery as canvas/mockup).
        | Event::DbAssistUpdated { workspace_id, .. }
        | Event::DbAssistSessionStarted { workspace_id, .. }
        // Budget-exceeded alerts are scoped to the workspace that crossed the cap
        // (it carries `workspace_id`), so they go to that workspace's members.
        | Event::BudgetExceeded { workspace_id, .. }
        // Work-graph (Mission Control) updates go to the item's workspace members.
        | Event::WorkGraphUpdated { workspace_id, .. } => Scope::Workspace(workspace_id),
        // Usage tick, self-improvement updates, and insight-ready are global
        // (no workspace axis — insights are a cross-workspace cadence report).
        // Deliver them to every authenticated client, matching the `Notice` pattern.
        Event::UsageMetricsTick { .. }
        | Event::ImprovementUpdated { .. }
        | Event::InsightReady { .. } => Scope::Everyone,
    }
}

/// Notice → everyone; workspace events → members (viewer+); session-family
/// events → the session's owner, a workspace Admin, or root (after the same
/// viewer-membership gate).
async fn allowed(
    ctx: &ServerCtx,
    user: &User,
    event: &Event,
    role_cache: &mut HashMap<Id, bool>,
    owner_cache: &mut HashMap<Id, Option<Id>>,
) -> bool {
    let (workspace_id, session) = match scope_of(event) {
        Scope::Everyone => return true,
        Scope::User(target) => {
            return match target {
                None => true,
                Some(target) => user.is_root || &user.id == target,
            };
        }
        Scope::Workspace(workspace_id) => (workspace_id, None),
        Scope::Session {
            workspace_id,
            session_id,
            owner,
        } => (workspace_id, Some((session_id, owner))),
    };

    // Workspace membership (viewer+) gate — required for both workspace and
    // session events; cached per workspace for this connection's lifetime.
    if !workspace_viewer(ctx.roles.as_ref(), user, workspace_id, role_cache).await {
        return false;
    }

    // Session-family events add the owner/admin/root axis on top of membership.
    let Some((session_id, owner)) = session else {
        return true;
    };
    let owner = match owner {
        // `SessionCreated` ships the owner; no lookup needed.
        Some(owner) => owner.clone(),
        // Otherwise resolve `created_by` once per session_id and cache it —
        // `created_by` is immutable, so the high-frequency `TrailAppended`
        // path stays off the DB after the first event for that session.
        None => match resolve_owner(ctx, session_id, owner_cache).await {
            Some(owner) => owner,
            // Session vanished / lookup failed: fail closed for non-root.
            None => return user.is_root,
        },
    };
    session_owner_admin_or_root(ctx.roles.as_ref(), user, workspace_id, &owner).await
}

/// Cached workspace viewer-membership check for this connection's lifetime.
async fn workspace_viewer(
    roles: &dyn RoleChecker,
    user: &User,
    workspace_id: &Id,
    cache: &mut HashMap<Id, bool>,
) -> bool {
    if let Some(&ok) = cache.get(workspace_id) {
        return ok;
    }
    let ok = roles
        .check(user, workspace_id, WorkspaceRole::Viewer)
        .await
        .is_ok();
    cache.insert(workspace_id.clone(), ok);
    ok
}

/// Resolve and cache a session's immutable `created_by` owner. At most one DB
/// lookup per `session_id` per connection; `None` is cached on a missing session
/// so a vanished session is never re-queried on the hot path.
async fn resolve_owner(
    ctx: &ServerCtx,
    session_id: &Id,
    cache: &mut HashMap<Id, Option<Id>>,
) -> Option<Id> {
    if let Some(owner) = cache.get(session_id) {
        return owner.clone();
    }
    let owner = ctx.manager.get(session_id).await.ok().map(|s| s.created_by);
    cache.insert(session_id.clone(), owner.clone());
    owner
}

/// The session-event recipient rule: root, the session's owner, or a workspace
/// Admin. This mirrors [`otto_core::auth::session_owner_or_admin`] (root /
/// owner short-circuit before the Admin DB check) but takes a bare `owner` id
/// rather than a `Session`, since `allowed` only ever holds the owner id.
async fn session_owner_admin_or_root(
    roles: &dyn RoleChecker,
    user: &User,
    workspace_id: &Id,
    owner: &Id,
) -> bool {
    user.is_root
        || owner == &user.id
        || roles
            .check(user, workspace_id, WorkspaceRole::Admin)
            .await
            .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::auth::{BoxFuture, RoleChecker};
    use otto_core::domain::{
        AgentTask, Session, SessionKind, SessionStatus as DomainSessionStatus, TrailEvent,
        TrailKind, TrailLevel, TrailSource, User, WorkspaceRole,
    };
    use otto_core::{Error, Result};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn user(id: &str, is_root: bool) -> User {
        User {
            id: id.into(),
            username: id.into(),
            display_name: id.into(),
            is_root,
            disabled: false,
            created_at: chrono::Utc::now(),
        }
    }

    fn session(id: &str, ws: &str, created_by: &str) -> Session {
        Session {
            id: id.into(),
            workspace_id: ws.into(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "t".into(),
            status: DomainSessionStatus::Running,
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: created_by.into(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            archived: false,
            meta: serde_json::Value::Null,
        }
    }

    /// A stub [`RoleChecker`] granting exactly one `(user, ws)` a role, counting
    /// every call so cache behavior can be asserted. Root is *not* special-cased
    /// here — the decision helper owns the root branch.
    struct StubRoles {
        ok_user: &'static str,
        ok_ws: &'static str,
        granted: WorkspaceRole,
        calls: Arc<AtomicUsize>,
    }

    impl StubRoles {
        fn new(ok_user: &'static str, ok_ws: &'static str, granted: WorkspaceRole) -> Self {
            Self {
                ok_user,
                ok_ws,
                granted,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl RoleChecker for StubRoles {
        fn check<'a>(
            &'a self,
            u: &'a User,
            workspace_id: &'a Id,
            min: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if u.id == self.ok_user && workspace_id == self.ok_ws && self.granted >= min {
                    Ok(())
                } else {
                    Err(Error::Forbidden("stub: insufficient role".into()))
                }
            })
        }
    }

    // ---- session_owner_admin_or_root (the #L10 policy core) ----------------

    #[tokio::test]
    async fn owner_receives_own_session_events() {
        // alice owns the session but has no role at all in the workspace.
        let roles = StubRoles::new("nobody", "ws1", WorkspaceRole::Viewer);
        assert!(
            session_owner_admin_or_root(&roles, &user("alice", false), &"ws1".into(), &"alice".into())
                .await
        );
    }

    #[tokio::test]
    async fn non_owner_editor_is_denied_session_events() {
        // bob (user B) is a workspace Editor but NOT the session owner -> denied.
        // This is the leak (#L10): a workspace viewer/editor must NOT see another
        // user's session events.
        let roles = StubRoles::new("bob", "ws1", WorkspaceRole::Editor);
        assert!(
            !session_owner_admin_or_root(&roles, &user("bob", false), &"ws1".into(), &"alice".into())
                .await
        );
    }

    #[tokio::test]
    async fn workspace_admin_non_owner_receives_session_events() {
        // carol is a workspace Admin (not the owner) -> allowed.
        let roles = StubRoles::new("carol", "ws1", WorkspaceRole::Admin);
        assert!(
            session_owner_admin_or_root(&roles, &user("carol", false), &"ws1".into(), &"alice".into())
                .await
        );
    }

    #[tokio::test]
    async fn root_receives_session_events_without_role_rows() {
        // The stub grants nothing to root; the helper's own root branch wins.
        let roles = StubRoles::new("nobody", "nowhere", WorkspaceRole::Viewer);
        assert!(
            session_owner_admin_or_root(&roles, &user("root", true), &"ws1".into(), &"alice".into())
                .await
        );
    }

    #[tokio::test]
    async fn owner_and_root_short_circuit_before_admin_db_check() {
        // Neither owner nor root should hit the RoleChecker (mirrors
        // session_owner_or_admin's short-circuit and keeps the hot path cheap).
        let roles = StubRoles::new("nobody", "nowhere", WorkspaceRole::Admin);
        assert!(
            session_owner_admin_or_root(&roles, &user("alice", false), &"ws1".into(), &"alice".into())
                .await
        );
        assert!(
            session_owner_admin_or_root(&roles, &user("root", true), &"ws1".into(), &"alice".into())
                .await
        );
        assert_eq!(roles.calls.load(Ordering::SeqCst), 0, "no admin check for owner/root");
    }

    // ---- workspace_viewer cache -------------------------------------------

    #[tokio::test]
    async fn workspace_viewer_caches_one_check_per_workspace() {
        let roles = StubRoles::new("bob", "ws1", WorkspaceRole::Viewer);
        let bob = user("bob", false);
        let mut cache = HashMap::new();
        assert!(workspace_viewer(&roles, &bob, &"ws1".into(), &mut cache).await);
        assert!(workspace_viewer(&roles, &bob, &"ws1".into(), &mut cache).await);
        assert!(workspace_viewer(&roles, &bob, &"ws1".into(), &mut cache).await);
        assert_eq!(
            roles.calls.load(Ordering::SeqCst),
            1,
            "viewer membership resolved once and cached for the connection"
        );
    }

    // ---- scope_of routing -------------------------------------------------

    fn trail() -> TrailEvent {
        TrailEvent {
            id: "tr1".into(),
            session_id: "s1".into(),
            workspace_id: "ws1".into(),
            ts: chrono::Utc::now(),
            source: TrailSource::Agent,
            kind: TrailKind::Note,
            level: TrailLevel::Info,
            summary: "hi".into(),
            detail: None,
        }
    }

    /// Every session-family variant must classify as `Scope::Session` carrying
    /// `session_id` + `workspace_id`; the lookup ones leave `owner == None`,
    /// `SessionCreated` carries the owner inline.
    #[test]
    fn session_family_events_route_to_session_scope() {
        let lookups = [
            Event::SessionStatus {
                session_id: "s1".into(),
                workspace_id: "ws1".into(),
                status: DomainSessionStatus::Running,
            },
            Event::SessionMetaUpdated {
                session_id: "s1".into(),
                workspace_id: "ws1".into(),
                meta: serde_json::Value::Null,
            },
            Event::SessionRemoved {
                session_id: "s1".into(),
                workspace_id: "ws1".into(),
            },
            Event::TrailAppended {
                session_id: "s1".into(),
                workspace_id: "ws1".into(),
                event: trail(),
            },
            Event::TasksUpdated {
                session_id: "s1".into(),
                workspace_id: "ws1".into(),
                tasks: Vec::<AgentTask>::new(),
            },
        ];
        for ev in &lookups {
            match scope_of(ev) {
                Scope::Session {
                    workspace_id,
                    session_id,
                    owner,
                } => {
                    assert_eq!(workspace_id, "ws1");
                    assert_eq!(session_id, "s1");
                    assert!(owner.is_none(), "lookup events carry no inline owner");
                }
                _ => panic!("expected Scope::Session for {ev:?}"),
            }
        }

        // SessionCreated carries the owner inline (no lookup needed).
        let created = Event::SessionCreated {
            session: session("s1", "ws1", "alice"),
        };
        match scope_of(&created) {
            Scope::Session { owner, .. } => assert_eq!(owner, Some(&"alice".to_string())),
            _ => panic!("expected Scope::Session for SessionCreated"),
        }
    }

    // ---- scope_denied: share tokens get NO event stream (Task 1.7) --------

    fn ctx_with_scope(scope: Option<otto_core::auth::SessionScope>) -> AuthContext {
        let u = user("guest", false);
        AuthContext {
            real_user: u.clone(),
            effective_user: u,
            scope,
        }
    }

    /// A scoped (share-link) token is refused on `/ws/events` (pre-upgrade): the
    /// stream is workspace-wide and would leak every sibling session.
    #[test]
    fn scoped_token_denied_on_events_stream() {
        let denied = ctx_with_scope(Some(otto_core::auth::SessionScope {
            session_id: "S1".into(),
            role: WorkspaceRole::Viewer,
            otp_pending: false,
        }));
        assert!(scope_denied(&denied), "a viewer share must be denied /ws/events");
        let denied = ctx_with_scope(Some(otto_core::auth::SessionScope {
            session_id: "S1".into(),
            role: WorkspaceRole::Editor,
            otp_pending: false,
        }));
        assert!(scope_denied(&denied), "an editor share must be denied /ws/events too");
    }

    /// A normal (unscoped) token is unaffected — it proceeds to the per-event
    /// `allowed()` filter as before.
    #[test]
    fn unscoped_token_allowed_onto_events_stream() {
        assert!(
            !scope_denied(&ctx_with_scope(None)),
            "a normal/impersonation token must NOT be denied at the scope gate"
        );
    }

    /// Non-session events keep their prior scope (regression guard for the
    /// "leave these alone" constraint).
    #[test]
    fn non_session_events_keep_prior_scope() {
        assert!(matches!(
            scope_of(&Event::Notice {
                level: "info".into(),
                title: "t".into(),
                body: "b".into()
            }),
            Scope::Everyone
        ));
        assert!(matches!(
            scope_of(&Event::ImprovementRunStarted {
                workspace_id: "ws1".into(),
                run_id: "r1".into(),
            }),
            Scope::Workspace(_)
        ));
        assert!(matches!(
            scope_of(&Event::SwarmStatus {
                workspace_id: "ws1".into(),
                swarm_id: "sw1".into(),
                status: "active".into(),
            }),
            Scope::Workspace(_)
        ));
    }
}
