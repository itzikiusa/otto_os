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
pub(crate) const BEARER_SUBPROTOCOL: &str = "otto-bearer";

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
            // Agent-credential rules for root routes (MCP-restricted tokens
            // never subscribe; an agent session's token only receives).
            if let Err(e) = crate::feature_guard::root_route_gate(
                crate::feature_guard::RootRoute::Events,
                &auth,
                Some(&ctx.pool),
            )
            .await
            {
                return ApiError(e).into_response();
            }
            // Only a person's own credential may act as an Otto window for
            // agent UI control (`hello`); an agent session's token — which
            // can open this socket too — only ever receives events.
            let ui_capable = crate::ui_bridge::is_human(&auth);
            // Authorize against the effective user (== real for a normal token).
            let user = auth.effective_user;
            // Client frames are small JSON (hello / presence): cap them.
            let ws = ws
                .max_message_size(crate::ui_bridge::MAX_CLIENT_FRAME)
                .max_frame_size(crate::ui_bridge::MAX_CLIENT_FRAME);
            // Echo `otto-bearer` only when the client used the subprotocol path,
            // otherwise the browser would reject an unsolicited subprotocol.
            if used_subprotocol {
                ws.protocols([BEARER_SUBPROTOCOL])
                    .on_upgrade(move |socket| handle_events(socket, ctx, user, ui_capable, token))
            } else {
                ws.on_upgrade(move |socket| handle_events(socket, ctx, user, ui_capable, token))
            }
        }
        Err(_) => ApiError(Error::Unauthorized).into_response(),
    }
}

/// Extract the bearer token from a `Sec-WebSocket-Protocol: otto-bearer, <token>`
/// request header. Returns `None` when the header is absent or not in that form.
pub(crate) fn token_from_subprotocol(headers: &HeaderMap) -> Option<String> {
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

/// How often an open `/ws/events` socket re-validates its credential (S8-03).
/// A revocation (`otto_rbac::tokens::revocation_signal`) re-checks at once;
/// this cadence catches the rest — user disable, an impersonation token's TTL.
const EVENTS_REAUTH_INTERVAL: Duration = Duration::from_secs(60);

/// WebSocket close code sent when the socket's credential stopped verifying
/// (docs/contracts/ws.md): the client must not silently reconnect with it.
pub(crate) const CLOSE_AUTH_REVOKED: u16 = 4401;

/// Whether the token that opened this socket still verifies AS THE SAME user.
/// A transient store error keeps the socket (retry next tick): only a definite
/// verdict — revoked, expired, disabled, or now another identity — closes it.
async fn still_authorized(ctx: &ServerCtx, token: &str, user: &User) -> bool {
    match ctx.authenticator.authenticate(token).await {
        Ok(auth) => auth.effective_user.id == user.id && !scope_denied(&auth),
        Err(Error::Internal(_)) => true,
        Err(_) => false,
    }
}

async fn handle_events(
    socket: WebSocket,
    ctx: ServerCtx,
    user: User,
    ui_capable: bool,
    token: String,
) {
    // Shared serialize-once fan-out (ws_fanout.rs): a recv is an Arc clone and
    // the JSON text is built at most once per event across every socket.
    let mut events = crate::ws_fanout::subscribe(&ctx.events);
    // Agent UI control: a human's socket is registered as a (not yet
    // addressable) Otto document; its `hello` makes it a command target and
    // `ui_frames` carries the per-connection frames (hello_ack, ui_command,
    // ui_command_cancel) that never touch the broadcast bus.
    let (ui_conn, mut ui_frames) = if ui_capable {
        let (id, rx) = ctx.ui_bridge.register(&user.id);
        (Some(id), Some(rx))
    } else {
        (None, None)
    };
    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.tick().await; // consume the immediate first tick
                       // Optional per-connection topic filter (`subscribe` frame): a socket that
                       // only needs a few event types (the menu-bar tray) never pays for the
                       // rest — they are dropped before the authorization check and serializing.
    let mut topics: Option<std::collections::HashSet<String>> = None;

    // Per-connection authorization caches (role / session owner / admin);
    // see `AuthCaches` for their freshness + bounds.
    let mut caches = AuthCaches::new(std::time::Instant::now());
    // Hands the worker back while a burst of big frames drains.
    let mut pacer = crate::ws_fanout::Pacer::new();
    // Credential re-validation (S8-03): the token was checked once at upgrade;
    // logout, "revoke all", a revoked API token or an expired impersonation
    // must not leave this socket streaming (or a `ui_command` target) forever.
    let mut reauth = tokio::time::interval(EVENTS_REAUTH_INTERVAL);
    reauth.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    reauth.tick().await; // the upgrade just authenticated
    let mut revocations = otto_rbac::tokens::revocation_signal();
    revocations.mark_unchanged();

    loop {
        tokio::select! {
            recheck = async {
                tokio::select! {
                    _ = reauth.tick() => true,
                    changed = revocations.changed() => changed.is_ok(),
                }
            } => {
                if recheck && !still_authorized(&ctx, &token, &user).await {
                    let _ = sink
                        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: CLOSE_AUTH_REVOKED,
                            reason: "credential revoked".into(),
                        })))
                        .await;
                    break;
                }
            }
            event = events.recv() => match event {
                Ok(crate::ws_fanout::FanItem::Event(frame)) => {
                    let event = &frame.event;
                    if topics.as_ref().is_some_and(|t| !t.contains(event.type_name())) {
                        continue;
                    }
                    caches.expire(std::time::Instant::now());
                    let ok = allowed(&ctx, &user, event, &mut caches.role, &mut caches.owner, &mut caches.admin).await;
                    // After the check: the owner must still receive its own
                    // `session_removed` before the entry goes.
                    caches.forget(event);
                    if !ok {
                        continue;
                    }
                    let Some(text) = frame.text() else { continue };
                    let len = text.len();
                    if sink.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                    pacer.sent(len).await;
                }
                // The shared pump fell behind the bus (every socket missed
                // the same events), or this socket fell behind the pump:
                // either way this client refetches (ws.md "Lag resync frame").
                Ok(crate::ws_fanout::FanItem::Lagged(skipped))
                | Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    // The bounded bus dropped events this socket hadn't read
                    // yet. Logging alone left the client silently stale until
                    // its next reconnect; tell it to refetch instead (it reuses
                    // its reconnect resync). See docs/contracts/ws.md.
                    tracing::warn!("events ws lagged, skipped {skipped} events — sent resync");
                    if sink.send(Message::Text(resync_frame(skipped).into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                if sink.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
            frame = async {
                match ui_frames.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => match frame {
                Some(text) => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            incoming = stream.next() => match incoming {
                // Client→server: only the UI-control `hello` / `presence`
                // frames (parsed strictly, ≤ 64 KiB); everything else, and
                // anything from a non-human socket, is ignored.
                Some(Ok(Message::Text(text))) => {
                    // `subscribe` (any socket, see ws.md): set/clear the topic
                    // filter and acknowledge with this daemon's boot id.
                    if let Some(filter) = parse_subscribe(text.as_str()) {
                        let ack = subscribe_ack(filter.as_ref());
                        topics = filter;
                        if sink.send(Message::Text(ack.into())).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    if let Some(conn) = &ui_conn {
                        ctx.ui_bridge.on_client_frame(conn, text.as_str());
                    }
                }
                Some(Ok(_)) => continue,
                _ => break,
            },
        }
    }
    if let Some(conn) = &ui_conn {
        ctx.ui_bridge.unregister(conn);
    }
}

/// Most topics one `subscribe` frame may name, and the longest topic accepted.
const MAX_TOPICS: usize = 64;
const MAX_TOPIC_LEN: usize = 64;

/// Parse a client `{"type":"subscribe","topics":[…]}` frame. `None` = not a
/// (valid) subscribe frame; `Some(None)` = clear the filter (an empty list);
/// `Some(Some(set))` = deliver only these event types. Over-long lists or
/// topics are rejected whole (the frame is then ignored).
fn parse_subscribe(text: &str) -> Option<Option<std::collections::HashSet<String>>> {
    #[derive(Deserialize)]
    struct Subscribe {
        #[serde(rename = "type")]
        kind: String,
        topics: Vec<String>,
    }
    let f: Subscribe = serde_json::from_str(text).ok()?;
    if f.kind != "subscribe"
        || f.topics.len() > MAX_TOPICS
        || f.topics
            .iter()
            .any(|t| t.is_empty() || t.len() > MAX_TOPIC_LEN)
    {
        return None;
    }
    Some((!f.topics.is_empty()).then(|| f.topics.into_iter().collect()))
}

/// `subscribe_ack`: the active filter (sorted; `null` = everything) and the
/// daemon's boot id (a changed id means the daemon restarted).
fn subscribe_ack(filter: Option<&std::collections::HashSet<String>>) -> String {
    let topics = filter.map(|t| {
        let mut v: Vec<&String> = t.iter().collect();
        v.sort();
        v
    });
    serde_json::json!({
        "type": "subscribe_ack",
        "topics": topics,
        "boot_id": crate::transport::boot_id(),
    })
    .to_string()
}

/// Per-connection `resync` frame sent when the broadcast receiver lagged and
/// `skipped` events were dropped for this socket.
fn resync_frame(skipped: u64) -> String {
    serde_json::json!({ "type": "resync", "skipped": skipped }).to_string()
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
    /// Strictly one user's: delivered ONLY to that user's connections — not to
    /// workspace members and not to root (the Otto Assistant is personal).
    Owner(&'a Id),
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
        }
        // Conversation view: transcript prose / tool output and produced
        // artifacts are session-private, exactly like the trail.
        | Event::TranscriptAppended {
            session_id,
            workspace_id,
            ..
        }
        | Event::TranscriptLive {
            session_id,
            workspace_id,
            ..
        }
        | Event::ArtifactAdded {
            session_id,
            workspace_id,
            ..
        } => Scope::Session {
            workspace_id,
            session_id,
            owner: None,
        },
        Event::SessionRenamed {
            session_id,
            workspace_id,
            ..
        }
        | Event::SessionRemoved {
            session_id,
            workspace_id,
        } => Scope::Session {
            workspace_id,
            session_id,
            owner: None,
        },
        Event::SessionCreated { session } | Event::SessionArchiveChanged { session } => Scope::Session {
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
        | Event::SwarmProjectCleared { workspace_id, .. }
        | Event::SwarmMessagePosted { workspace_id, .. }
        | Event::SwarmStatus { workspace_id, .. }
        | Event::SwarmGoalUpdated { workspace_id, .. }
        // Product events are workspace-scoped.
        | Event::ProductChanged { workspace_id, .. }
        | Event::PlanRun { workspace_id, .. }
        // Review, workflow, and skill-eval events are workspace-scoped too.
        | Event::ReviewChanged { workspace_id, .. }
        | Event::GoalLoopUpdated { workspace_id, .. }
        | Event::WorkflowRunUpdated { workspace_id, .. }
        | Event::SkillEvalUpdated { workspace_id, .. }
        | Event::SkillReviewUpdated { workspace_id, .. }
        // Live canvas-document edits + the agent-session-started signal go to the
        // scene's workspace members.
        | Event::CanvasUpdated { workspace_id, .. }
        // API-client history writes go to every member of the request's workspace.
        | Event::ApiHistoryAppended { workspace_id, .. }
        | Event::ApiRunProgress { workspace_id, .. }
        | Event::ApiClientChanged { workspace_id, .. }
        | Event::CanvasSessionStarted { workspace_id, .. }
        // A session's referenced-scenes set changed — workspace-member scoped like
        // the other canvas events (Canvas is a workspace-shared tool).
        | Event::CanvasRefsChanged { workspace_id, .. }
        // A watched repo's working tree changed: its workspace's members (the
        // REST status read applies the per-repo role check).
        | Event::RepoStatusChanged { workspace_id, .. }
        // Browser tab/annotation updates go to the workspace's members, like
        // the other canvas-family live-edit events.
        | Event::BrowserTabUpdated { workspace_id, .. }
        | Event::BrowserAnnotationAdded { workspace_id, .. }
        // A remote live session's lifecycle tick (no URL/title — owner-private
        // details stay on the per-tab REST/WS surface).
        | Event::BrowserLiveSessionUpdated { workspace_id, .. }
        // Live mockup-source edits + the mockup-agent-started signal go to the
        // story's workspace members (same delivery as canvas).
        | Event::MockupUpdated { workspace_id, .. }
        | Event::MockupSessionStarted { workspace_id, .. }
        // Design Hall graph events (artifact content/meta, link moves, captured
        // learning signals) go to the artifact's workspace members, exactly like
        // the canvas/mockup live-edit events they supersede.
        | Event::DesignArtifactUpdated { workspace_id, .. }
        | Event::DesignLinkUpdated { workspace_id, .. }
        | Event::DesignLearningUpdate { workspace_id, .. }
        // Design-assist turn states + variant-run completion: workspace
        // members, like the canvas/mockup agent-session-started signals.
        | Event::DesignAssistUpdated { workspace_id, .. }
        | Event::DesignVariantsReady { workspace_id, .. }
        // Live DB-Assistant answer edits + the assist-agent-started signal go to the
        // connection's workspace members (same delivery as canvas/mockup).
        | Event::DbAssistUpdated { workspace_id, .. }
        | Event::DbAssistSessionStarted { workspace_id, .. }
        // Findings-workflow events (status changes, agent-action started, proof-pack
        // exported) are workspace-scoped, like reviews.
        | Event::FindingUpdated { workspace_id, .. }
        | Event::FindingActionStarted { workspace_id, .. }
        | Event::ProofPackExported { workspace_id, .. }
        // Budget-exceeded alerts are scoped to the workspace that crossed the cap
        // (it carries `workspace_id`), so they go to that workspace's members.
        | Event::BudgetExceeded { workspace_id, .. }
        // Work-graph (Mission Control) updates go to the item's workspace members.
        | Event::WorkGraphUpdated { workspace_id, .. }
        // Proof-pack updates are workspace-scoped (gated on viewer access).
        | Event::ProofPackUpdated { workspace_id, .. }
        // Scheduled-task run updates go to the task's workspace members.
        | Event::ScheduledTaskRunUpdated { workspace_id, .. }
        // Personal-agent run updates + room messages go to the workspace members
        // (rooms are always fully user-visible by design).
        | Event::PersonalAgentRunUpdated { workspace_id, .. }
        | Event::PersonalAgentActivity { workspace_id, .. }
        | Event::AgentRoomMessage { workspace_id, .. }
        // Run with Otto stage updates go to the run's workspace members.
        | Event::OttoRunUpdated { workspace_id, .. }
        // History index rescan progress goes to the requesting workspace.
        | Event::HistoryIndexProgress { workspace_id, .. } => Scope::Workspace(workspace_id),
        // Usage tick, self-improvement updates, and insight-ready are global
        // (no workspace axis — insights are a cross-workspace cadence report).
        // Deliver them to every authenticated client, matching the `Notice` pattern.
        Event::UsageMetricsTick { .. }
        | Event::ImprovementUpdated { .. }
        | Event::InsightReady { .. } => Scope::Everyone,
        // AWS / Kubernetes console registries are global libraries (like
        // connections); the install-job ticks are machine-wide.
        Event::AwsAccountUpdated { .. }
        | Event::AwsInstallUpdated { .. }
        | Event::K8sClusterUpdated { .. }
        | Event::K8sInstallUpdated { .. }
        // The Chromium download job is machine-wide, like the k8s installer.
        | Event::BrowserEngineInstallUpdated { .. }
        | Event::K8sMonitorCycle { .. } => Scope::Everyone,
        // MCP approval invalidations go to the approval's workspace members.
        // Workspace-less ones (and the row-less consume/expiry signals) carry
        // only an opaque id + status — no title/tool/args — so every
        // authenticated client may use them as a "refetch your list" cue; the
        // REST list applies visibility. Same for the access-changed cue.
        Event::McpApprovalChanged {
            workspace_id: Some(workspace_id),
            ..
        } => Scope::Workspace(workspace_id),
        Event::McpApprovalChanged {
            workspace_id: None, ..
        }
        | Event::ResourceAccessChanged { .. } => Scope::Everyone,
        // Otto Assistant events are personal: the owner only (no root fan-out).
        Event::AssistantTurn { user_id, .. }
        | Event::AssistantTaskUpdate { user_id, .. }
        | Event::AssistantNeedsYou { user_id, .. }
        | Event::AssistantLimit { user_id, .. }
        // Agent UI control asks ONLY the session's owner (the one person who
        // can grant it) — not workspace admins, not root.
        | Event::UiControlRequested { user_id, .. }
        // A notice-list change (read/dismiss) cues only the actor's windows.
        | Event::NotificationsChanged { user_id }
        // Workbench docs are per-user: only the owner's windows hear of them.
        | Event::WorkbenchDocChanged { user_id, .. } => Scope::Owner(user_id),
    }
}

/// Notice → everyone; workspace events → members (viewer+); session-family
/// events → the session's owner, a workspace Admin, or root (after the same
/// viewer-membership gate).
/// How long a cached workspace role / Admin answer is trusted. There is no
/// membership-change event, so a demoted (or removed) member kept receiving a
/// workspace's events for the socket's whole lifetime; now within this window.
const AUTH_CACHE_TTL: Duration = Duration::from_secs(60);
/// Session owners cached per socket before the map is reset (a long-lived
/// socket otherwise remembered every session it ever saw an event for).
const OWNER_CACHE_CAP: usize = 4096;

/// The per-connection authorization caches `allowed` reads.
struct AuthCaches {
    /// Workspace viewer-membership results.
    role: HashMap<Id, bool>,
    /// Session owner (`created_by`, immutable) per session_id — keeps the
    /// high-frequency `TrailAppended` path off the DB.
    owner: HashMap<Id, Option<Id>>,
    /// Workspace-Admin results — a non-owner, non-root recipient used to pay
    /// an Admin role query per `session_status` / `trail_appended` (F8).
    admin: HashMap<Id, bool>,
    refreshed: std::time::Instant,
}

impl AuthCaches {
    fn new(now: std::time::Instant) -> Self {
        Self {
            role: HashMap::new(),
            owner: HashMap::new(),
            admin: HashMap::new(),
            refreshed: now,
        }
    }

    /// Drop role/Admin answers older than [`AUTH_CACHE_TTL`] so a membership
    /// change takes effect on open sockets, and bound the owner map.
    fn expire(&mut self, now: std::time::Instant) {
        if now.saturating_duration_since(self.refreshed) >= AUTH_CACHE_TTL {
            self.role.clear();
            self.admin.clear();
            self.refreshed = now;
        }
        if self.owner.len() >= OWNER_CACHE_CAP {
            self.owner.clear();
        }
    }

    /// Forget a removed session's owner (it can emit nothing further).
    fn forget(&mut self, event: &Event) {
        if let Event::SessionRemoved { session_id, .. } = event {
            self.owner.remove(session_id);
        }
    }
}

async fn allowed(
    ctx: &ServerCtx,
    user: &User,
    event: &Event,
    role_cache: &mut HashMap<Id, bool>,
    owner_cache: &mut HashMap<Id, Option<Id>>,
    admin_cache: &mut HashMap<Id, bool>,
) -> bool {
    let (workspace_id, session) = match scope_of(event) {
        Scope::Everyone => return true,
        Scope::User(target) => {
            return match target {
                None => true,
                Some(target) => user.is_root || &user.id == target,
            };
        }
        Scope::Owner(target) => return &user.id == target,
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
    if user.is_root || owner == user.id {
        return true;
    }
    if let Some(&admin) = admin_cache.get(workspace_id) {
        return admin;
    }
    let admin = session_owner_admin_or_root(ctx.roles.as_ref(), user, workspace_id, &owner).await;
    admin_cache.insert(workspace_id.clone(), admin);
    admin
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

    #[test]
    fn subscribe_frame_parses_caps_and_clears() {
        let set =
            parse_subscribe(r#"{"type":"subscribe","topics":["session_status","notification"]}"#)
                .expect("valid frame")
                .expect("a filter");
        assert!(set.contains("session_status") && set.contains("notification"));
        assert_eq!(set.len(), 2);
        // An empty list clears the filter (all events again).
        assert_eq!(
            parse_subscribe(r#"{"type":"subscribe","topics":[]}"#),
            Some(None)
        );
        // Not a subscribe frame / malformed / over the caps → ignored.
        assert_eq!(parse_subscribe(r#"{"type":"hello","topics":["x"]}"#), None);
        assert_eq!(parse_subscribe("not json"), None);
        let many: Vec<String> = (0..=MAX_TOPICS).map(|i| format!("t{i}")).collect();
        let frame = serde_json::json!({"type":"subscribe","topics":many}).to_string();
        assert_eq!(parse_subscribe(&frame), None);
        let long = "x".repeat(MAX_TOPIC_LEN + 1);
        let frame = serde_json::json!({"type":"subscribe","topics":[long]}).to_string();
        assert_eq!(parse_subscribe(&frame), None);
    }

    #[test]
    fn subscribe_ack_carries_sorted_topics_and_boot_id() {
        let set: std::collections::HashSet<String> = ["notification", "mcp_approval_changed"]
            .map(String::from)
            .into();
        let v: serde_json::Value = serde_json::from_str(&subscribe_ack(Some(&set))).unwrap();
        assert_eq!(v["type"], "subscribe_ack");
        assert_eq!(
            v["topics"],
            serde_json::json!(["mcp_approval_changed", "notification"])
        );
        assert_eq!(v["boot_id"], crate::transport::boot_id());
        let v: serde_json::Value = serde_json::from_str(&subscribe_ack(None)).unwrap();
        assert!(v["topics"].is_null());
    }

    #[test]
    fn transport_invalidation_events_are_scoped() {
        let ws: Id = "ws1".into();
        let scoped = Event::McpApprovalChanged {
            approval_id: Some("a".into()),
            workspace_id: Some(ws.clone()),
            status: "pending".into(),
        };
        assert!(matches!(scope_of(&scoped), Scope::Workspace(w) if w == &ws));
        let global = Event::McpApprovalChanged {
            approval_id: None,
            workspace_id: None,
            status: "expired".into(),
        };
        assert!(matches!(scope_of(&global), Scope::Everyone));
        let access = Event::ResourceAccessChanged {
            kind: None,
            resource_id: None,
        };
        assert!(matches!(scope_of(&access), Scope::Everyone));
        let uid: Id = "u1".into();
        let notes = Event::NotificationsChanged {
            user_id: uid.clone(),
        };
        assert!(matches!(scope_of(&notes), Scope::Owner(u) if u == &uid));
    }

    #[test]
    fn resync_frame_shape() {
        let v: serde_json::Value = serde_json::from_str(&resync_frame(42)).unwrap();
        assert_eq!(v, serde_json::json!({"type": "resync", "skipped": 42}));
    }

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
            session_owner_admin_or_root(
                &roles,
                &user("alice", false),
                &"ws1".into(),
                &"alice".into()
            )
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
            !session_owner_admin_or_root(
                &roles,
                &user("bob", false),
                &"ws1".into(),
                &"alice".into()
            )
            .await
        );
    }

    #[tokio::test]
    async fn workspace_admin_non_owner_receives_session_events() {
        // carol is a workspace Admin (not the owner) -> allowed.
        let roles = StubRoles::new("carol", "ws1", WorkspaceRole::Admin);
        assert!(
            session_owner_admin_or_root(
                &roles,
                &user("carol", false),
                &"ws1".into(),
                &"alice".into()
            )
            .await
        );
    }

    #[tokio::test]
    async fn root_receives_session_events_without_role_rows() {
        // The stub grants nothing to root; the helper's own root branch wins.
        let roles = StubRoles::new("nobody", "nowhere", WorkspaceRole::Viewer);
        assert!(
            session_owner_admin_or_root(
                &roles,
                &user("root", true),
                &"ws1".into(),
                &"alice".into()
            )
            .await
        );
    }

    #[tokio::test]
    async fn owner_and_root_short_circuit_before_admin_db_check() {
        // Neither owner nor root should hit the RoleChecker (mirrors
        // session_owner_or_admin's short-circuit and keeps the hot path cheap).
        let roles = StubRoles::new("nobody", "nowhere", WorkspaceRole::Admin);
        assert!(
            session_owner_admin_or_root(
                &roles,
                &user("alice", false),
                &"ws1".into(),
                &"alice".into()
            )
            .await
        );
        assert!(
            session_owner_admin_or_root(
                &roles,
                &user("root", true),
                &"ws1".into(),
                &"alice".into()
            )
            .await
        );
        assert_eq!(
            roles.calls.load(Ordering::SeqCst),
            0,
            "no admin check for owner/root"
        );
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
            Event::TranscriptAppended {
                session_id: "s1".into(),
                workspace_id: "ws1".into(),
                cursor: "0".into(),
                turns: Vec::new().into(),
            },
            Event::TranscriptLive {
                session_id: "s1".into(),
                workspace_id: "ws1".into(),
                text: "".into(),
                input: "".into(),
                status: "".into(),
                branch: None,
            },
            Event::ArtifactAdded {
                session_id: "s1".into(),
                workspace_id: "ws1".into(),
                artifact: serde_json::Value::Null,
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
            mcp_only: false,
            mcp_scope: None,
            mcp_internal: false,
            mcp_session_id: None,
            managed_session_id: None,
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
        assert!(
            scope_denied(&denied),
            "a viewer share must be denied /ws/events"
        );
        let denied = ctx_with_scope(Some(otto_core::auth::SessionScope {
            session_id: "S1".into(),
            role: WorkspaceRole::Editor,
            otp_pending: false,
        }));
        assert!(
            scope_denied(&denied),
            "an editor share must be denied /ws/events too"
        );
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

    /// Otto Assistant events are owner-only — never workspace-wide or global.
    #[test]
    fn assistant_events_are_owner_scoped() {
        let evs = [
            Event::AssistantTurn {
                user_id: "alice".into(),
                thread_id: "t1".into(),
                turn: serde_json::json!({}),
                thread: None,
            },
            Event::AssistantTaskUpdate {
                user_id: "alice".into(),
                task: serde_json::json!({}),
            },
            Event::AssistantNeedsYou {
                user_id: "alice".into(),
                task: serde_json::json!({}),
                open_count: 1,
            },
            Event::AssistantLimit {
                user_id: "alice".into(),
                thread_id: None,
                limit: serde_json::json!({}),
                suggestion: None,
                task_id: None,
                auto_switched: false,
            },
            // Agent UI control asks only the session owner.
            Event::UiControlRequested {
                user_id: "alice".into(),
                workspace_id: "ws1".into(),
                session_id: "s1".into(),
                session_title: "t".into(),
                module: "connections".into(),
                command: "db_run_query".into(),
            },
            // Workbench docs are per-user.
            Event::WorkbenchDocChanged {
                workspace_id: "ws1".into(),
                user_id: "alice".into(),
                doc_id: "d1".into(),
                action: "updated".into(),
                rev: 1,
                updated_at: "t".into(),
                client_id: None,
            },
        ];
        for ev in &evs {
            assert!(
                matches!(scope_of(ev), Scope::Owner(u) if u == "alice"),
                "expected Scope::Owner(alice) for {ev:?}"
            );
        }
    }

    /// Design Hall graph events are workspace-member scoped (like the canvas /
    /// mockup live-edit events they supersede), never global.
    #[test]
    fn design_events_are_workspace_scoped() {
        for ev in [
            Event::DesignArtifactUpdated {
                workspace_id: "ws1".into(),
                artifact_id: "a1".into(),
                format: "html".into(),
                change: "content".into(),
                version_id: Some("v1".into()),
                content: Some("<p>x</p>".into()),
            },
            Event::DesignLinkUpdated {
                workspace_id: "ws1".into(),
                artifact_id: "a1".into(),
                link_id: None,
                target_artifact_id: None,
                target_version_id: None,
                reason: "extracted".into(),
            },
            Event::DesignLearningUpdate {
                workspace_id: "ws1".into(),
                kind: "shipped".into(),
                signal_id: None,
                artifact_id: None,
            },
            Event::DesignAssistUpdated {
                workspace_id: "ws1".into(),
                artifact_id: "a1".into(),
                turn_id: "t1".into(),
                status: "done".into(),
                mode: "refine".into(),
                branch: "main".into(),
                session_id: None,
                version_id: Some("v2".into()),
                error: None,
            },
            Event::DesignVariantsReady {
                workspace_id: "ws1".into(),
                artifact_id: "a1".into(),
                run_id: "r1".into(),
                base_version_id: None,
                version_ids: vec![],
                failed: 0,
            },
        ] {
            assert!(
                matches!(scope_of(&ev), Scope::Workspace(w) if w == "ws1"),
                "{ev:?}"
            );
        }
    }

    /// Perf P3 / security: role + Admin answers expire after the TTL (a
    /// demoted member stops receiving events), a removed session's owner is
    /// evicted, and the owner map stays bounded.
    #[test]
    fn auth_caches_expire_roles_and_evict_removed_sessions() {
        let t0 = std::time::Instant::now();
        let mut c = AuthCaches::new(t0);
        c.role.insert("w".into(), true);
        c.admin.insert("w".into(), true);
        c.owner.insert("s1".into(), Some("u".into()));
        c.owner.insert("s2".into(), Some("u".into()));
        c.expire(t0 + Duration::from_secs(5));
        assert!(c.role.contains_key("w") && c.admin.contains_key("w"));
        c.expire(t0 + AUTH_CACHE_TTL);
        assert!(c.role.is_empty() && c.admin.is_empty());
        c.forget(&Event::SessionRemoved {
            session_id: "s1".into(),
            workspace_id: "w".into(),
        });
        assert!(!c.owner.contains_key("s1") && c.owner.contains_key("s2"));
        for i in 0..OWNER_CACHE_CAP {
            c.owner.insert(format!("x{i}"), None);
        }
        c.expire(t0 + AUTH_CACHE_TTL);
        assert!(c.owner.is_empty(), "owner cache is bounded");
    }
}
