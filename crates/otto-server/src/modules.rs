//! Integration seam (plan Task A9): assembles the module routers for
//! `build_router` and hosts the orchestrator, provider-update, DB-explain and
//! product-inject routes. The ctx-trait impls ([`ctx_impls`]), code review
//! ([`review`]), PR/commit drafting, session input, the browser proxy and
//! skill staging live in child modules.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use otto_connections::{ConnectionsService, Spawner};
use otto_core::api::{
    CreateSessionReq, ExecutePlanReq, HandoffReq, LocalReviewReq, NewPrCommentReq, OrchestrateReq,
    OrchestrateResp, RepoReviewBinding, RepoReviewConfigResp, ReviewConfig, ReviewConfigPreset,
    StartReviewReq, UpdateProvidersReq,
};
use otto_core::auth::{BoxFuture, RoleChecker};
use otto_core::domain::{
    CommentSeverity, CommentState, Connection, Review, ReviewAgentCfg, ReviewAgentState,
    ReviewComment, ReviewStatus, Session, SessionKind, User, Workspace, WorkspaceRole,
};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_orchestrator::{execute, ExecuteResp, OrchestratorContext, PlanIo, PlanSpawner};
use otto_pty::CommandSpec;
use otto_sessions::SessionManager;
use otto_state::{GitStore, IntegrationsRepo, IssuesRepo, WorkspacesRepo};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::{CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
use otto_core::cancel::{CancelGuard, CancelRegistryExt};
// The review engine's pure core lives in otto-review. The four items other
// engines reach through `crate::modules::…` are re-exported by name; the rest
// are the review code's own imports (children see them through `use super::*`).
use otto_review::engine::{
    anchored_line_text, append_to_review_file, compose_orchestrator_prompt, default_review_config,
    diff_file_paths, expand_agent_runs, general_comment_body, inline_anchor_rejected,
    is_blocking_severity, label_findings_with_lens, mode_source, orchestrator_budget,
    parse_draft_comments, render_diff, repo_review_binding_key, resolve_blocker,
    review_agent_cancel_key, review_agent_timeout, review_run_complete, reviewer_slots,
    same_draft_comment, severity_rank, ReviewBranches, REVIEW_SPAWN_STAGGER,
};
pub(crate) use otto_review::engine::{
    resolve_skill_inline, workflow_review_config, workflow_review_config_from_json,
    CHECKS_REVIEWER_NAME,
};
use otto_review::worktree::{teardown_pr_worktree, PrWorktree};

// The split-out concerns. Each child sees this module's imports and items
// through `use super::*`; this module wires their routes in `module_routers`
// and re-exports, by name, what the rest of the crate imports from here.
mod browser_proxy;
mod ctx_impls;
mod pr_draft;
pub(crate) mod review;
mod session_input;
mod skill_staging;

use self::browser_proxy::*;
use self::pr_draft::*;
use self::review::*;
use self::session_input::*;
use self::skill_staging::*;

pub use self::ctx_impls::PtySpawner;
pub(crate) use self::pr_draft::{
    compose_draft_prompt, ensure_jira_in_subject, jira_key_from_branch, parse_pr_draft,
    pr_draft_prompt, resolve_provider_remote,
};
pub(crate) use self::review::service::{
    cancel_running_review, default_review_provider, effective_review_mode, review_finding_briefs,
    review_findings_counts, review_open_counts_by_severity,
};
pub(crate) use self::skill_staging::{stage_skill_packages_at, StagedSkillPackages};

/// Routes: POST /workspaces/{id}/orchestrate, .../orchestrate/execute, and the
/// dedicated AI-free .../broadcast.
pub fn orchestrator_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{id}/orchestrate", post(orchestrate))
        .route(
            "/workspaces/{id}/orchestrate/execute",
            post(orchestrate_execute),
        )
        .route("/workspaces/{id}/broadcast", post(workspace_broadcast))
        .route("/workspaces/{id}/relay", post(workspace_relay))
        .route("/workspaces/{id}/sessions/open", post(open_agent_session))
        .route(
            "/workspaces/{id}/product/stories/{sid}/analyze",
            post(otto_product::analysis::analyze::<ServerCtx>),
        )
        // Curated analysis-lens catalog the Analysis tab renders as configurable
        // checks. Read-only (Viewer); the prefix policy gates it as Product/View.
        .route(
            "/workspaces/{id}/product/lenses",
            get(otto_product::analysis::product_lenses::<ServerCtx>),
        )
        .route(
            "/workspaces/{id}/product/stories/{sid}/rewrite",
            post(otto_product::analysis::rewrite::<ServerCtx>),
        )
        .route(
            "/workspaces/{id}/product/stories/{sid}/testcases/generate",
            post(otto_product::analysis::generate_tests::<ServerCtx>),
        )
        .route(
            "/workspaces/{id}/product/stories/{sid}/plan/generate",
            post(otto_product::analysis::generate_plan::<ServerCtx>),
        )
        .route(
            "/workspaces/{id}/product/stories/{sid}/plan",
            post(otto_product::analysis::save_plan::<ServerCtx>),
        )
        // Product → Canvas: list the Canvas scenes linked to a story (Viewer).
        .route(
            "/product/stories/{sid}/linked-canvases",
            get(otto_product::http::list_linked_canvases::<ServerCtx>),
        )
        // Talk-to-agent refinement: a conversational thread on a story. Create +
        // list its threads (story-scoped), then read/converse/archive a thread
        // (resolves the workspace from the thread → its story). The converse turn
        // runs the agent inline and may write a new `suggested` story version.
        .route(
            "/product/stories/{sid}/refinement-threads",
            post(otto_product::refine::create_thread::<ServerCtx>)
                .get(otto_product::refine::list_threads::<ServerCtx>),
        )
        .route(
            "/product/refinement-threads/{tid}",
            get(otto_product::refine::get_thread::<ServerCtx>),
        )
        .route(
            "/product/refinement-threads/{tid}/messages",
            post(otto_product::refine::send_message::<ServerCtx>),
        )
        .route(
            "/product/refinement-threads/{tid}/stop",
            post(otto_product::refine::stop_turn::<ServerCtx>),
        )
        .route(
            "/product/refinement-threads/{tid}/archive",
            post(otto_product::refine::archive_thread::<ServerCtx>),
        )
        // Discovery Chat: a lightweight conversational agent on a story (works
        // from an empty draft) for early discovery/research. Each turn assembles
        // a relevance-bounded context bundle and may propose actions the user
        // applies explicitly. Covered by the `/product/` policy prefix.
        .route(
            "/product/stories/{sid}/discovery-chats",
            post(otto_product::chat::create_chat::<ServerCtx>)
                .get(otto_product::chat::list_chats::<ServerCtx>),
        )
        .route(
            "/product/discovery-chats/{cid}",
            get(otto_product::chat::get_chat::<ServerCtx>),
        )
        .route(
            "/product/discovery-chats/{cid}/messages",
            post(otto_product::chat::send_message::<ServerCtx>),
        )
        .route(
            "/product/discovery-chats/{cid}/stop",
            post(otto_product::chat::stop_turn::<ServerCtx>),
        )
        .route(
            "/product/discovery-chats/{cid}/archive",
            post(otto_product::chat::archive_chat::<ServerCtx>),
        )
        .route(
            "/product/discovery-chats/{cid}/apply",
            post(otto_product::chat::apply_action::<ServerCtx>),
        )
        // Canvas agent-assist: turn a prompt into diagram blocks. The engine is
        // otto-canvas's; the agent turn runs through `CanvasAssistCtx` (above).
        .route(
            "/canvas/scenes/{id}/assist",
            post(otto_canvas::assist::assist_scene::<ServerCtx>),
        )
        .route(
            "/canvas/scenes/{id}/assist/stop",
            post(otto_canvas::assist::stop_assist::<ServerCtx>),
        )
        .route(
            "/canvas/assist/preview",
            post(otto_canvas::assist::assist_preview::<ServerCtx>),
        )
        // Session ↔ Canvas scene references — needs the SessionManager to resolve
        // a session's workspace, so it lives here rather than in otto-canvas.
        .merge(crate::canvas_refs::canvas_refs_routes())
        // Story attachments — upload route gets its own 40 MB body cap to bound
        // the ~33 % base64 inflation (raw content cap is enforced at 25 MB).
        .route(
            "/product/stories/{sid}/attachments",
            post(otto_product::media::upload_attachment::<ServerCtx>)
                .layer(DefaultBodyLimit::max(40 * 1024 * 1024))
                .get(otto_product::media::list_attachments::<ServerCtx>),
        )
        .route(
            "/product/attachments/{aid}",
            get(otto_product::media::serve_attachment::<ServerCtx>)
                .patch(otto_product::media::patch_attachment::<ServerCtx>)
                .delete(otto_product::media::delete_attachment::<ServerCtx>),
        )
        // Design arena: save an edited artifact from the UI editor. Same 40 MB
        // body cap as the upload (base64 inflation over the 25 MB raw cap).
        .route(
            "/product/attachments/{aid}/content",
            put(otto_product::media::put_attachment_content::<ServerCtx>)
                .layer(DefaultBodyLimit::max(40 * 1024 * 1024)),
        )
        // In-place design agent: generate / refine an artifact (html | mermaid |
        // excalidraw | scene3d) with a live, file-backed agent session (hidden
        // from the Agents list).
        .route(
            "/product/stories/{sid}/mockups/assist",
            post(crate::mockup_assist::assist_mockup),
        )
        // Blender bridge (optional, detected): status, headless render job,
        // job poll, generated-script download.
        .route(
            "/product/design/blender",
            get(crate::design_blender::blender_status),
        )
        .route(
            "/product/design/jobs/{id}",
            get(crate::design_blender::get_job),
        )
        .route(
            "/product/stories/{sid}/design/{aid}/blender-render",
            post(crate::design_blender::blender_render),
        )
        .route(
            "/product/stories/{sid}/design/{aid}/blender-script",
            get(crate::design_blender::blender_script),
        )
        .route(
            "/product/attachments/{aid}/annotations",
            get(otto_product::media::list_annotations::<ServerCtx>)
                .post(otto_product::media::create_annotation::<ServerCtx>),
        )
        .route(
            "/product/annotations/{id}",
            patch(otto_product::media::patch_annotation::<ServerCtx>)
                .delete(otto_product::media::delete_annotation::<ServerCtx>),
        )
        // Approve lives here (not in otto-product) so it can trigger self-improvement.
        .route(
            "/product/testcase-runs/{rid}/approve",
            post(otto_product::analysis::approve_testcase_run::<ServerCtx>),
        )
        // Per-agent retry: re-run a single failed/stuck analysis lens agent.
        .route(
            "/product/analyses/{aid}/agents/{agent_id}/retry",
            post(otto_product::analysis::retry_analysis_agent::<ServerCtx>),
        )
        // Per-agent stop: kill a running/waiting analysis agent on demand.
        .route(
            "/product/analyses/{aid}/agents/{agent_id}/stop",
            post(otto_product::analysis::stop_analysis_agent::<ServerCtx>),
        )
}

async fn orchestrate(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<OrchestrateReq>,
) -> ApiResult<Json<OrchestrateResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;
    // The planner spawns a real claude session in the workspace root —
    // pre-trust the folder so the PTY never stalls on the trust dialog.
    otto_sessions::trust::ensure_trusted("claude", &ws.root_path);
    let octx = orchestrator_context(&ctx, Typist::new(&user.id, &auth), &ws_id, ws).await?;
    let resp = ctx
        .orchestrator
        .orchestrate(req, octx)
        .await
        .map_err(ApiError)?;
    Ok(Json(resp))
}

/// The workspace facts a plan is planned AND validated against: the caller's
/// live sessions + connections and the live provider registry.
async fn orchestrator_context(
    ctx: &ServerCtx,
    who: Typist<'_>,
    ws_id: &Id,
    ws: Workspace,
) -> ApiResult<OrchestratorContext> {
    let sessions = input_agents(ctx, who, ws_id).await.map_err(ApiError)?;
    let connections = ctx
        .connections
        .list_for(ws_id, who.user_id)
        .await
        .map_err(ApiError)?;
    // Effective default agent for this workspace: per-workspace setting, else
    // the global default, else "claude". Steers spawn_sessions in the planner.
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    Ok(OrchestratorContext {
        sessions,
        connections,
        cwd: ws.root_path,
        default_provider,
        // Live registry (builtins + custom providers) so the planner can
        // spawn any configured provider by name, e.g. "open grok session".
        available_providers: ctx.manager.providers().names(),
    })
}

async fn orchestrate_execute(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<ExecutePlanReq>,
) -> ApiResult<Json<ExecuteResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    // The plan comes back from the client, which may have edited (or forged)
    // it since `/orchestrate` validated it — validate again against the live
    // workspace before anything is spawned or typed.
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;
    let octx = orchestrator_context(&ctx, Typist::new(&user.id, &auth), &ws_id, ws).await?;
    otto_orchestrator::parse::validate_plan(&req.plan, &octx, &octx.allowed_providers())
        .map_err(ApiError)?;
    let helper = ExecHelper {
        ctx: ctx.clone(),
        ws_id: ws_id.clone(),
        agent: crate::feature_guard::agent_session_of(&auth).cloned(),
        user,
    };
    Ok(Json(execute(&req.plan, &helper, &helper).await))
}

/// `POST /workspaces/{id}/broadcast` — relay a literal message to live agent
/// sessions. Dedicated, AI-free path: no parsing, no orchestrator, no fallback.
/// `session_ids` (when present) targets a subset; absent/empty hits all live
/// agents. Returns the sessions that actually received it.
async fn workspace_broadcast(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::BroadcastReq>,
) -> ApiResult<Json<otto_core::api::BroadcastResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let text = req.text.trim();
    if text.is_empty() {
        return Err(ApiError(Error::Invalid("broadcast text is empty".into())));
    }
    // Treat an empty target list the same as "no targets" → broadcast to all.
    let targets = req.session_ids.filter(|ids| !ids.is_empty());
    let who = Typist::resolve(&ctx, &user.id, &auth, &ws_id, req.from_session_id.as_ref())
        .await
        .map_err(ApiError)?;
    let session_ids = broadcast_sessions(&ctx, who, &ws_id, text, targets.as_deref())
        .await
        .map_err(ApiError)?;
    Ok(Json(otto_core::api::BroadcastResp { session_ids }))
}

/// `POST /workspaces/{id}/relay` — deliver a name-addressed message ("ronaldo:
/// do X", "ronaldo, messi: ship it", "all: stand down"). Resolves the leading
/// address against the workspace's live agent sessions; when nothing matches,
/// returns `unaddressed = true` (delivers nothing) so the caller can fall back.
/// Editor role.
async fn workspace_relay(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::RelayReq>,
) -> ApiResult<Json<otto_core::api::RelayResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let text = req.text.trim();
    if text.is_empty() {
        return Err(ApiError(Error::Invalid("relay text is empty".into())));
    }
    let who = Typist::new(&user.id, &auth);
    let candidates = input_agents(&ctx, who, &ws_id).await.map_err(ApiError)?;
    let addressable: Vec<_> = candidates
        .iter()
        .map(|s| otto_sessions::names::Addressable {
            id: s.id.clone(),
            handle: s
                .meta
                .get("name_handle")
                .and_then(Value::as_str)
                .unwrap_or(&s.title)
                .to_string(),
            full: s
                .meta
                .get("name_full")
                .and_then(Value::as_str)
                .unwrap_or(&s.title)
                .to_string(),
            title: s.title.clone(),
        })
        .collect();
    let resolved = otto_sessions::names::resolve_address(text, &addressable);
    let mut session_ids = Vec::new();
    for id in &resolved.targets {
        // Targets resolve only against the already-authorized candidates.
        let Some(session) = candidates.iter().find(|s| s.id == *id) else {
            continue;
        };
        if submit_session_text(&ctx, who, session, resolved.text.trim())
            .await
            .is_ok()
        {
            session_ids.push(id.clone());
        }
    }
    Ok(Json(otto_core::api::RelayResp {
        session_ids,
        broadcast: resolved.broadcast,
        unaddressed: resolved.targets.is_empty(),
        text: if resolved.targets.is_empty() {
            text.to_string()
        } else {
            resolved.text
        },
    }))
}

/// Per-request plan executor scoped to one workspace and acting user.
struct ExecHelper {
    ctx: ServerCtx,
    ws_id: Id,
    user: User,
    /// The caller's own agent session when the plan arrived on an agent
    /// credential: its broadcast/command steps reach that session, its workers
    /// and sibling agent sessions — never a shell sibling (S11-305).
    agent: Option<Id>,
}

impl ExecHelper {
    fn typist(&self) -> Typist<'_> {
        let agent = self.agent.as_ref();
        Typist {
            user_id: &self.user.id,
            agent,
            attributed_from: agent,
        }
    }
}

impl PlanSpawner for ExecHelper {
    fn spawn_agent<'a>(&'a self, provider: &'a str) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            let ws = self.ctx.workspaces.get(&self.ws_id).await?;
            let req = CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some(provider.to_string()),
                title: None,
                cwd: None,
                connection_id: None,
                model: None,
                meta: None,
            };
            self.ctx.manager.create(&ws, &self.user.id, req, None).await
        })
    }

    fn open_connection<'a>(&'a self, connection_id: &'a Id) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            let conn = self.ctx.connections.get(connection_id).await?;
            let visible = match &conn.workspace_id {
                None => true,
                Some(ws) => *ws == self.ws_id,
            };
            if !visible {
                return Err(Error::Forbidden(
                    "connection belongs to another workspace".into(),
                ));
            }
            self.ctx
                .connections
                .open(
                    &conn,
                    &self.ws_id,
                    &self.user.id,
                    None,
                    self.ctx.spawner.as_ref(),
                )
                .await
        })
    }
}

impl PlanIo for ExecHelper {
    fn broadcast<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<Vec<Id>>> {
        // Funnel through the one shared implementation so the AI/palette path and
        // the dedicated /broadcast endpoint can't drift. `None` = all live agents.
        Box::pin(async move {
            broadcast_sessions(&self.ctx, self.typist(), &self.ws_id, text, None).await
        })
    }

    fn run_command<'a>(&'a self, session_id: &'a Id, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let session = input_session(&self.ctx, &self.user.id, session_id).await?;
            if session.workspace_id != self.ws_id {
                return Err(Error::Forbidden(
                    "session belongs to another workspace".into(),
                ));
            }
            // Submit as a real keypress (paste + Enter), not "{text}\n" in one
            // burst — otherwise bracketed-paste TUIs paste the text but never send.
            submit_session_text(&self.ctx, self.typist(), &session, text).await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// PR review agent routes
// ---------------------------------------------------------------------------

#[path = "review_launch.rs"]
mod review_launch;
pub(crate) use review_launch::run_review_for_branch;

/// Routes: POST /workspaces/{id}/providers/update
pub fn provider_routes() -> Router<ServerCtx> {
    Router::new().route("/workspaces/{id}/providers/update", post(update_providers))
}

async fn update_providers(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(req): Json<UpdateProvidersReq>,
) -> ApiResult<Json<Session>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;

    // Collect (name, command) pairs — either the single requested provider or all.
    let all_cmds = ctx.manager.provider_update_commands();
    let pairs: Vec<(String, String)> = if let Some(ref name) = req.provider {
        let found = all_cmds.into_iter().find(|(n, _)| n == name);
        match found {
            Some(pair) => vec![pair],
            None => {
                return Err(ApiError(Error::Invalid(format!(
                    "provider '{name}' has no update command"
                ))));
            }
        }
    } else {
        all_cmds
    };

    if pairs.is_empty() {
        return Err(ApiError(Error::Invalid(
            "no providers have an update command configured".into(),
        )));
    }

    // Build the compound shell command: join with "; echo; " so each step's
    // output is separated by a blank line.
    let compound = pairs
        .iter()
        .map(|(_, cmd)| cmd.as_str())
        .collect::<Vec<_>>()
        .join("; echo; ");

    let session_req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some("shell".to_string()),
        title: Some("Update CLIs".to_string()),
        cwd: None,
        connection_id: None,
        model: None,
        meta: None,
    };

    let session = ctx
        .manager
        .create(&ws, &user.id, session_req, None)
        .await
        .map_err(ApiError)?;

    // Write the compound command into the PTY shortly after spawn, mirroring
    // the PtySpawner pattern used for connection first_command.
    delay_session_input(
        &ctx,
        &user,
        &session.id,
        compound,
        Duration::from_millis(800),
        token,
    );

    Ok(Json(session))
}

// ---------------------------------------------------------------------------
// Session input route  (POST /sessions/{id}/input)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ProxyTicketReq {
    url: String,
}

#[derive(serde::Serialize)]
struct ProxyTicketResp {
    ticket: String,
    expires_in_secs: u64,
}

/// `POST /browser/proxy-ticket` — authed (bearer + `Browser`/Edit policy);
/// returns a single-use ticket the take-over iframe passes as `?ticket=`.
/// Share-link and MCP-restricted tokens never get one
/// (`RootRoute::BrowserProxy`: the proxy itself only sees the ticket).
async fn browser_proxy_ticket(
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<ProxyTicketReq>,
) -> ApiResult<Json<ProxyTicketResp>> {
    crate::feature_guard::root_route_gate(
        crate::feature_guard::RootRoute::BrowserProxy,
        &auth,
        None,
    )
    .await
    .map_err(ApiError)?;
    let url = req.url.trim();
    let ok_scheme = reqwest::Url::parse(url)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false);
    if !ok_scheme {
        return Err(ApiError(Error::Invalid(
            "url must be an absolute http(s) URL".into(),
        )));
    }
    Ok(Json(ProxyTicketResp {
        ticket: mint_proxy_ticket(url, PROXY_TICKET_TTL),
        expires_in_secs: PROXY_TICKET_TTL.as_secs(),
    }))
}

/// Authed API route that mints proxy tickets (mounted under `/api/v1`).
pub fn browser_proxy_ticket_routes() -> Router<ServerCtx> {
    Router::new().route("/browser/proxy-ticket", post(browser_proxy_ticket))
}

/// Body of `POST /connections/{id}/db/explain-with-agent`. The UI sends the
/// schema text or result JSON it already has plus an optional question; we spawn
/// an agent session seeded with a prompt built from them.
#[derive(Debug, Deserialize)]
struct DbExplainReq {
    /// Pre-rendered context (schema DDL, structure, or result rows) to analyze.
    content: String,
    /// Optional user question; defaults to a general "explain this" prompt.
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    title: Option<String>,
    /// Needed only for global connections (which have no workspace of their own).
    #[serde(default)]
    workspace_id: Option<Id>,
}

/// Route: spawn an agent to explain/analyze a database schema or query result,
/// plus the file-backed **DB Assistant** turns (the connection-scoped half — the
/// `q`-tool query route is a PUBLIC route in `routes/mod.rs`, assist-key authed).
pub fn db_explorer_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/connections/{id}/db/explain-with-agent",
            post(db_explain_with_agent),
        )
        .route(
            "/connections/{id}/db/assist",
            post(crate::db_assist::assist),
        )
        .route(
            "/connections/{id}/db/assist/{aid}/summary",
            post(crate::db_assist::summary),
        )
        .route(
            "/connections/{id}/db/assist/{aid}",
            axum::routing::delete(crate::db_assist::close),
        )
}

async fn db_explain_with_agent(
    Path(conn_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(body): Json<DbExplainReq>,
) -> ApiResult<Json<Session>> {
    otto_state::GrantsRepo::new(ctx.pool.clone())
        .check_global(
            &user,
            otto_core::domain::Feature::Agents,
            otto_core::domain::Capability::Edit,
            "Database explanations require permission to run host agents",
        )
        .await?;
    let conn = ctx
        .db_explorer
        .get_connection(&conn_id)
        .await
        .map_err(ApiError)?;
    let ws_id = conn
        .workspace_id
        .clone()
        .or(body.workspace_id)
        .ok_or_else(|| ApiError(Error::Invalid("a workspace_id is required".into())))?;
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;

    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    otto_sessions::trust::ensure_trusted(&default_provider, &ws.root_path);

    let question = body
        .question
        .as_deref()
        .filter(|q| !q.trim().is_empty())
        .unwrap_or(
            "Explain this database schema/result: describe each table/field, the relationships, \
             and anything notable (indexing, normalization, possible issues). Then suggest a few \
             useful queries.",
        );
    let prompt = format!(
        "You are a database expert. A user is exploring the `{}` ({}) connection in Otto.\n\n\
         {}\n\n--- DATABASE CONTEXT ---\n{}\n",
        conn.name,
        conn.kind.as_str(),
        question,
        body.content
    );

    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(default_provider),
        title: Some(
            body.title
                .unwrap_or_else(|| format!("Explain {}", conn.name)),
        ),
        cwd: Some(ws.root_path.clone()),
        connection_id: None,
        model: None,
        meta: Some(serde_json::json!({"source":"db_assist", "connection_id":conn_id})),
    };
    let resource = otto_core::access::ResourceRef {
        kind: otto_core::access::ResourceKind::Connection,
        id: conn_id.clone(),
        child: None,
    };
    otto_rbac::ResourceAccess::new(ctx.pool.clone())
        .check(&user, &resource, "db_query")
        .await
        .map_err(ApiError)?;
    let session = ctx
        .manager
        .create(&ws, &user.id, req, None)
        .await
        .map_err(ApiError)?;

    delay_session_input(
        &ctx,
        &user,
        &session.id,
        prompt,
        Duration::from_millis(1500),
        token,
    );

    Ok(Json(session))
}

// ---------------------------------------------------------------------------
// Inject-session route
// ---------------------------------------------------------------------------

/// `POST /workspaces/{id}/product/stories/{sid}/inject-session`
///
/// Builds the inject bundle for the story and spawns an Agent session that
/// receives the bundle as its initial prompt (after a short settle delay).
async fn inject_session(
    Path((ws_id, sid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(req): Json<otto_product::types::InjectSessionReq>,
) -> ApiResult<Json<Session>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;

    // Load story and verify it belongs to the requested workspace.
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    if story.workspace_id != ws_id {
        return Err(ApiError(Error::NotFound(
            "story not found in workspace".into(),
        )));
    }

    // Resolve default provider (workspace → global → "claude"), mirroring analyze.
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    let provider = req.provider.clone().unwrap_or(default_provider);

    // Resolve cwd: req → story.cwd → temp dir.
    let cwd = req
        .cwd
        .or_else(|| story.cwd.clone())
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());

    // Build the inject bundle.
    let bundle = ctx
        .product
        .build_inject_bundle(&sid)
        .await
        .map_err(ApiError)?;

    // Spawn the agent session. A chosen model reaches the CLI only via
    // `meta.model` (→ `--model`); omit when empty so the provider default holds.
    let meta = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|m| serde_json::json!({ "model": m }));
    let create_req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider),
        title: Some(format!("Implement {}", story.source_key)),
        cwd: Some(cwd),
        connection_id: None,
        model: None,
        meta,
    };
    let session = ctx
        .manager
        .create(&ws, &user.id, create_req, None)
        .await
        .map_err(ApiError)?;

    // Write the bundle into the session after a short settle delay.
    let payload = bundle.markdown.clone();
    delay_session_input(
        &ctx,
        &user,
        &session.id,
        payload,
        Duration::from_secs(6),
        token,
    );

    // Record an inject event.
    ctx.product_repo
        .add_event(otto_state::NewEvent {
            story_id: sid,
            section: "inject".into(),
            kind: "inject_session".into(),
            summary: format!("Injected bundle into session {}", session.id),
            actor_id: Some(user.id),
            meta_json: None,
        })
        .await
        .ok();

    Ok(Json(session))
}

/// Routes for the inject-session endpoint.
pub fn inject_session_routes() -> Router<ServerCtx> {
    Router::new().route(
        "/workspaces/{id}/product/stories/{sid}/inject-session",
        post(inject_session),
    )
}

// ---------------------------------------------------------------------------
// Attach-product-story route (injects bundle into a running session)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct AttachProductReq {
    story_id: Id,
}

/// `POST /sessions/{session_id}/attach-product`
///
/// Loads the story (verifying it belongs to the session's workspace), builds
/// the inject bundle, pushes it into the live PTY, and tags the session meta
/// with `product_story: { id, title }` so the UI can badge the attachment.
async fn attach_product_story(
    Path(session_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(req): Json<AttachProductReq>,
) -> ApiResult<Json<Session>> {
    // Resolve the session and its workspace.
    let session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &session.workspace_id, WorkspaceRole::Editor).await?;

    crate::auth::require_session_owner_or_admin(&ctx, &user, &session).await?;

    // Load story and verify it belongs to the same workspace.
    let story = ctx
        .product_repo
        .get_story(&req.story_id)
        .await
        .map_err(ApiError)?;
    if story.workspace_id != session.workspace_id {
        return Err(ApiError(Error::NotFound(
            "story not found in workspace".into(),
        )));
    }

    // Build the inject bundle.
    let bundle = ctx
        .product
        .build_inject_bundle(&req.story_id)
        .await
        .map_err(ApiError)?;

    // Push the bundle into the live PTY after a short settle delay.
    let payload = bundle.markdown.clone();
    delay_session_input(
        &ctx,
        &user,
        &session.id,
        payload,
        Duration::from_secs(2),
        token,
    );

    // Tag the session meta with the attached story.
    let updated = ctx
        .manager
        .update_meta(
            &session_id,
            serde_json::json!({ "product_story": { "id": story.id, "title": story.title } }),
        )
        .await
        .map_err(ApiError)?;

    Ok(Json(updated))
}

/// Routes for the attach-product-story endpoint.
pub fn attach_product_routes() -> Router<ServerCtx> {
    Router::new().route(
        "/sessions/{session_id}/attach-product",
        post(attach_product_story),
    )
}

/// All module routers: `(api_extras, root_extras)` for [`crate::build_router`].
pub fn module_routers(ctx: &ServerCtx) -> (Vec<Router<ServerCtx>>, Vec<Router>) {
    let api = vec![
        otto_sessions::api_router::<ServerCtx>(),
        otto_connections::api_router::<ServerCtx>(),
        otto_aws::api_router::<ServerCtx>(),
        otto_k8s::api_router::<ServerCtx>(),
        otto_dbviewer::api_router::<ServerCtx>(),
        otto_brokers::api_router::<ServerCtx>(),
        otto_mcp::api_router::<ServerCtx>(),
        otto_product::router::<ServerCtx>(),
        otto_canvas::router::<ServerCtx>(),
        // Design Hall — the artifact graph (`/design/*`, Feature::Design).
        otto_design::router::<ServerCtx>(),
        // The unified design-assist pipeline (agent turns, variants, learned
        // rules) — otto-design-assist's; the turn runs via `DesignAssistCtx`.
        otto_design_assist::routes::<ServerCtx>(),
        otto_memory::router::<ServerCtx>(),
        otto_vault::router::<ServerCtx>(),
        crate::vault_docs_agent::routes(),
        crate::memory_gov::memory_gov_routes(),
        otto_git::router::<ServerCtx>(),
        otto_issues::router::<ServerCtx>(),
        otto_channels::router::<ServerCtx>(),
        otto_improve::router::<ServerCtx>(),
        otto_context::router::<ServerCtx>(),
        otto_skills::http::router::<ServerCtx>(),
        crate::routes::goal_loops::routes(),
        otto_insights::routes::<ServerCtx>(),
        orchestrator_routes(),
        db_explorer_routes(),
        pr_review_routes(),
        crate::routes::findings::routes(),
        crate::routes::repo_rules::routes(),
        crate::routes::proof_pack::routes(),
        crate::routes::scheduled_tasks::routes(),
        crate::routes::personal_agents::routes(),
        review_config_routes(),
        crate::skill_eval::routes(),
        crate::eval_lab_routes::routes(),
        crate::skill_review::routes(),
        provider_routes(),
        session_input_routes(),
        inject_session_routes(),
        attach_product_routes(),
        crate::context_packet::context_packet_routes(),
        crate::lsp::api_router(),
        crate::routes::capabilities::capabilities_routes(),
        crate::routes::search::search_routes(),
        crate::routes::backup::backup_routes(),
        crate::routes::connection_export::routes(),
        crate::routes::backup_git::routes(),
        // Runtime custom plugins: management + scoped host-API + reverse-proxy to
        // sidecar processes. (Asset/iframe routes are root-mounted; see root vec.)
        crate::plugins::api_routes(),
        // Single-use tickets for the root-level `/browser/proxy` take-over frame.
        browser_proxy_ticket_routes(),
    ];
    let root = vec![
        otto_sessions::ws_router(ctx.authenticator.clone(), ctx.clone()),
        crate::lsp::ws_router(ctx.authenticator.clone(), ctx.clone()),
        crate::routes::api_stream::ws_router(ctx.clone()),
        // Remote live browser viewer socket (`/ws/browser/{tab_id}/live`).
        crate::routes::browser_live::ws_router(ctx.clone()),
        browser_proxy_router(),
        // Runtime-plugin iframe assets: /plugins/{slug}/ui/* served as public
        // static files (root-mounted, outside /api/v1; the iframe's API calls are
        // the gated part). Listed last so it doesn't shadow other root routes.
        crate::plugins::asset_router(ctx.clone()),
    ];
    (api, root)
}

#[cfg(test)]
#[path = "../tests/unit/review_launch_recovery.rs"]
mod review_launch_recovery_tests;
