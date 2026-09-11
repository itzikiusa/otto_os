//! Core REST routes (contract endpoints #1-16, #57-58).

pub mod database_changes;
pub mod resource_access;
pub mod access_groups;

pub mod activity;
pub mod admin_sessions;
pub mod api_client;
pub mod backup;
pub mod browser;
pub mod capabilities;
pub mod channel_webhook;
pub mod swarm_webhook;
pub mod api_stream;
pub mod audit;
pub mod auth_routes;
pub mod email_sender;
pub mod findings;
pub mod goal_loops;
pub mod grants;
pub mod grpc;
pub mod fs;
pub mod proof_pack;
pub mod repo_rules;
pub mod runs;
pub mod personal_agents;
pub mod scheduled_tasks;
pub mod handover;
pub mod impersonate;
pub mod logs;
pub mod mcp_servers;
pub mod meta;
pub mod mission;
pub mod workgraph;
pub mod name_themes;
pub mod notifications;
pub mod onboarding;
pub mod proof;
pub mod product_memory;
pub mod settings;
pub mod share;
pub mod snips;
pub mod swarm_ingest;
pub mod slash_commands;
pub mod transcript;
pub mod usage;
pub mod users;
pub mod workflows;
pub mod search;
pub mod workspaces;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::Router;

use otto_core::domain::SCRATCH_WORKSPACE_ID;

use crate::state::ServerCtx;

/// Routes reachable without authentication.
pub fn public_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/health", get(meta::health))
        .route("/meta", get(meta::meta))
        .route("/onboarding/root", post(onboarding::onboard_root))
        .route("/auth/login", post(auth_routes::login))
        // Provider activity ingest: gated by the per-session token Otto sets on
        // the agent PTY, not by a user bearer token (the agent's hooks have no
        // user session). Verified inside the handler.
        .route("/ingest/claude", post(activity::claude_ingest))
        .route("/ingest/codex", post(activity::codex_ingest))
        // Provider token-usage ingest (same per-session token gate as above).
        .route("/ingest/usage", post(usage::ingest))
        // Swarm agents post to the shared board via their per-session token.
        .route("/ingest/swarm/board", post(swarm_ingest::board_ingest))
        // Swarm (PO) agents publish a feature draft to the Product page.
        .route("/ingest/swarm/product", post(swarm_ingest::product_ingest))
        // Swarm discovery/design agents publish a generated mockup (HTML/Mermaid)
        // for the story under discovery.
        .route("/ingest/swarm/mockup", post(swarm_ingest::ingest_mockup))
        // Swarm discovery agents publish the consolidated discovery report.
        .route(
            "/ingest/swarm/discovery-report",
            post(swarm_ingest::ingest_discovery_report),
        )
        // Runtime-plugin host API: sidecars call back here with their own
        // OTTO_PLUGIN_TOKEN (validated in each handler) — NOT a user bearer, so it
        // lives in public_routes (outside the user auth chokepoint), like /ingest/*.
        .merge(crate::plugins::host_routes())
        // Email-OTP share gate (mobile plan Task 7.3): redeem an emailed code for
        // a share token. PUBLIC by design — the share token in the body IS the
        // auth, so this must be reachable BEFORE the (still OTP-pending) scoped
        // token can attach. Rate-limited per peer IP inside the handler.
        .route("/share/verify", post(share::verify_share))
        // Email-OTP share EXTENSION (mobile plan Task 7.4): re-issue a fresh OTP
        // for an existing OTP share, emailed to the LOCKED original recipient ONLY
        // (the body carries no email — the destination is read from the share row).
        // PUBLIC by design (the share token IS the auth) and reachable after the
        // window elapses. Rate-limited per peer IP inside the handler.
        .route("/share/extend", post(share::extend_share))
        // Workflow webhook trigger: PUBLIC-by-token. The 64-char hex token in the
        // URL path is the credential; no bearer auth required. The handler validates
        // the token against the `workflow_triggers` table before starting any run.
        // POLICY EXEMPTION: the orchestrator must add this path prefix to the
        // allowlist in policy.rs (same treatment as /share/* endpoints).
        .route(
            "/workflows/{id}/webhook/{token}",
            post(workflows::webhook_trigger),
        )
        // Inbound channel webhook: PUBLIC-by-key. The per-webhook secret in the
        // `X-Otto-Webhook-Key` header (or `Authorization: Bearer`) is the
        // credential; no user bearer auth. The handler validates the key against
        // the keychain before triggering any agent session.
        // POLICY EXEMPTION: same allowlist treatment as /workflows/*/webhook/*.
        .route("/webhooks/{workspace_id}", post(channel_webhook::inbound))
        // Run with Otto webhook entry: launch a source→PR-draft run. Same
        // per-workspace webhook key as the channel webhook. POLICY EXEMPTION:
        // classified `Exempt` in policy.rs (key-guarded, no bearer).
        .route("/webhooks/{workspace_id}/run", post(channel_webhook::run_inbound))
        // External trigger that auto-plans + starts a specific swarm (worktree
        // isolation). Same per-workspace webhook key as the channel webhook.
        .route("/webhooks/swarm/{workspace_id}/{swarm_id}", post(swarm_webhook::trigger))
        // DB Assistant `q` tool: the file-backed agent runs `./q '<read-only SQL>'`,
        // which POSTs here with its per-assist `x-assist-key` (NOT a user bearer —
        // the agent's PTY has no user session). Lives in public_routes (outside the
        // bearer chokepoint), like `/ingest/*`; the handler validates the key and
        // refuses writes. POLICY EXEMPTION: classified `Exempt` in policy.rs.
        .route("/db-assist/{aid}/query", post(crate::db_assist::query_tool))
}

/// Routes that require a bearer token (the auth middleware is layered on top
/// of this router, together with any api_extras, by `build_router`).
pub fn protected_routes() -> Router<ServerCtx> {
    Router::new()
        .merge(resource_access::api_router::<ServerCtx>())
        .merge(database_changes::api_router())
        .route("/auth/logout", post(auth_routes::logout))
        .route("/auth/me", get(auth_routes::me))
        // --- API tokens (personal access tokens) ------------------------
        .route(
            "/auth/tokens",
            get(auth_routes::list_tokens).post(auth_routes::create_token),
        )
        .route("/auth/tokens/{id}", delete(auth_routes::revoke_token))
        // --- Share-link management (mobile plan Task 1.9) ----------------
        .route(
            "/auth/shares/{share_id}",
            delete(share::revoke_share),
        )
        .route("/auth/shares/revoke-all", post(share::revoke_all_shares))
        // --- Per-user email sender (Gmail App Password → Keychain; mobile
        // plan Task 7.1). Self-owned (any authed user sets their OWN sender):
        // Exempt in policy, like `/auth/tokens`.
        .route(
            "/email-sender",
            get(email_sender::get_email_sender).put(email_sender::set_email_sender),
        )
        // --- Agent activity (live trail + task tracker) ------------------
        .route(
            "/workspaces/{wid}/sessions/{sid}/trail",
            get(activity::list_trail).post(activity::append_trail),
        )
        .route(
            "/workspaces/{wid}/sessions/{sid}/tasks",
            get(activity::list_tasks).put(activity::put_tasks),
        )
        .route(
            "/workspaces/{wid}/activity/summary",
            get(activity::workspace_summary),
        )
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", patch(users::update).delete(users::remove))
        // --- Grants (per-user feature grants, RBAC Task 2.1) -------------
        .route(
            "/users/{id}/grants",
            get(grants::get_grants::<ServerCtx>).put(grants::put_grants::<ServerCtx>),
        )
        // Custom-plugin grants (string-keyed by slug). Covered by the `/users/`
        // prefix rule in policy.rs (Users:Admin); handlers additionally require root.
        .route(
            "/users/{id}/plugin-grants",
            get(grants::get_plugin_grants::<ServerCtx>)
                .put(grants::put_plugin_grants::<ServerCtx>),
        )
        // Caller's effective capability map (any authed user; Exempt in policy).
        .route("/auth/capabilities", get(grants::capabilities::<ServerCtx>))
        // --- Admin active-sessions overview + terminate (RBAC Task 4.2) ---
        // The sanctioned cross-user view; gated Users:Admin/root via policy.rs.
        .route(
            "/admin/sessions",
            get(admin_sessions::list_sessions::<ServerCtx>),
        )
        .route(
            "/admin/sessions/{id}/terminate",
            post(admin_sessions::terminate::<ServerCtx>),
        )
        .route(
            "/admin/sessions/{id}/remove",
            post(admin_sessions::remove::<ServerCtx>),
        )
        // --- Admin impersonation (act-as, audited; RBAC Task 5.2) ---------
        // Gated Users:Admin/root via policy.rs; the handlers enforce the
        // anti-escalation guardrails (never up/sideways, no nesting, no self).
        .route(
            "/admin/impersonate/{user_id}",
            post(impersonate::start::<ServerCtx>),
        )
        .route(
            "/admin/impersonate/stop",
            post(impersonate::stop::<ServerCtx>),
        )
        .route(
            "/workspaces",
            get(workspaces::list).post(workspaces::create),
        )
        // The system-owned scratch workspace (workspace-less sessions). A
        // static segment, so it ranks above `/workspaces/{id}` regardless of
        // order; kept adjacent for readability. Read-only — the `{id}` PATCH /
        // DELETE / members handlers answer 409 for it.
        .route("/workspaces/scratch", get(workspaces::get_scratch))
        .route(
            "/workspaces/{id}",
            patch(workspaces::update).delete(workspaces::archive),
        )
        .route(
            "/workspaces/{id}/members",
            get(workspaces::members).put(workspaces::set_members),
        )
        .route("/settings", get(settings::get_all).put(settings::put_all))
        // --- Dynamic model catalog (discovered per-provider model ids) ----
        .route("/providers/models", get(crate::model_catalog::list))
        // --- Walkthrough video redirect resolver (WebKit can't follow a
        // redirecting <video src>; see routes/meta.rs) --------------------
        .route("/walkthroughs/resolve", get(meta::resolve_walkthrough))
        .route(
            "/providers/models/refresh",
            post(crate::model_catalog::refresh),
        )
        // --- Session name themes (auto-naming new sessions) --------------
        .route(
            "/name-themes",
            get(name_themes::list).post(name_themes::create),
        )
        .route("/name-themes/active", put(name_themes::set_active))
        .route(
            "/name-themes/{id}",
            put(name_themes::update).delete(name_themes::delete),
        )
        // --- Trust & Safety Center (root only) ---------------------------
        .route("/audit-log", get(audit::list))
        .route("/security-posture", get(audit::posture))
        // --- Workspace MCP servers (user-managed `.mcp.json` entries) -----
        .route(
            "/workspaces/{id}/mcp-servers",
            get(mcp_servers::list).post(mcp_servers::create),
        )
        .route(
            "/mcp-servers/{id}",
            patch(mcp_servers::update).delete(mcp_servers::delete),
        )
        // --- MCP Control Plane: outward "Otto as MCP server" + gateway + the
        //     capability endpoints behind the otto.* tools. (The registry /
        //     governance routes live in the otto-mcp module router.) ----------
        .route(
            "/mcp/otto-tools/invoke",
            post(crate::mcp_outward::otto_tools_invoke),
        )
        .route(
            "/mcp/otto-server",
            get(crate::mcp_outward::otto_server_status).patch(crate::mcp_outward::otto_server_config),
        )
        // Streamable-HTTP MCP transport: external clients reach the otto.* tools
        // over HTTP here (no local stdio subprocess). Confined for kind='mcp'
        // tokens by the feature guard; per-token scope enforced in the handler.
        .route(
            "/mcp/http",
            post(crate::mcp_http::mcp_http_post).get(crate::mcp_http::mcp_http_get),
        )
        // Multiple scoped MCP tokens (the per-user / per-token access layer).
        .route(
            "/mcp/tokens",
            get(crate::mcp_outward::list_mcp_tokens).post(crate::mcp_outward::create_mcp_token),
        )
        .route(
            "/mcp/tokens/{id}",
            delete(crate::mcp_outward::revoke_mcp_token),
        )
        .route("/mcp/gateway/tools", get(crate::mcp_outward::gateway_tools))
        .route("/mcp/gateway/invoke", post(crate::mcp_outward::gateway_invoke))
        .route(
            "/workspaces/{wid}/mcp/code-search",
            get(crate::mcp_capabilities::code_search),
        )
        .route(
            "/workspaces/{wid}/mcp/context-packet",
            post(crate::mcp_capabilities::context_packet),
        )
        .route(
            "/workspaces/{wid}/mcp/proof-pack",
            get(crate::mcp_capabilities::proof_pack),
        )
        // --- Usage tracking & system metrics (embedded ClickHouse) -------
        .route("/usage/status", get(usage::status))
        .route("/usage/summary", get(usage::summary))
        .route("/usage/by-kind", get(usage::by_kind))
        .route("/usage/metrics", get(usage::metrics))
        .route("/usage/config", put(usage::put_config))
        .route("/usage/install", post(usage::install))
        // Usage budgets (opt-in spend caps; enforcement default off).
        .route(
            "/usage/budgets",
            get(usage::budgets).put(usage::put_budgets),
        )
        // Work-graph attribution drilldown + pre-launch cost forecast (B1).
        .route("/usage/attribution", get(usage::attribution))
        .route("/usage/forecast", post(usage::forecast))
        .route(
            "/notifications",
            get(notifications::list).delete(notifications::clear),
        )
        .route(
            "/notifications/settings",
            get(notifications::get_settings).put(notifications::put_settings),
        )
        .route(
            "/notifications/read-all",
            post(notifications::mark_all_read),
        )
        .route(
            "/notifications/{id}/read",
            post(notifications::mark_read),
        )
        .route("/notifications/{id}", delete(notifications::dismiss))
        .route(
            "/workspaces/{ws}/product/stories/{sid}/memory/ingest",
            post(product_memory::ingest),
        )
        .route("/fs/browse", get(fs::browse))
        .route("/fs/read", get(fs::read_file))
        .route("/logs/daemon", get(logs::daemon_logs))
        // UI last-resort error report (self-heal hook in ui/src/main.ts).
        .route("/client/errors", post(logs::client_error))
        // --- API client ("Postman") -------------------------------------
        .route(
            "/workspaces/{wid}/api-client/collections",
            get(api_client::list_collections).post(api_client::create_collection),
        )
        .route(
            "/workspaces/{wid}/api-client/collections/{id}",
            patch(api_client::update_collection).delete(api_client::delete_collection),
        )
        .route(
            "/workspaces/{wid}/api-client/collections/{id}/openapi",
            get(api_client::export_openapi),
        )
        .route(
            "/workspaces/{wid}/api-client/requests",
            get(api_client::list_requests).post(api_client::create_request),
        )
        .route(
            "/workspaces/{wid}/api-client/requests/{id}",
            get(api_client::get_request)
                .patch(api_client::update_request)
                .delete(api_client::delete_request),
        )
        .route(
            "/workspaces/{wid}/api-client/environments",
            get(api_client::list_environments).post(api_client::create_environment),
        )
        .route(
            "/workspaces/{wid}/api-client/environments/{id}",
            patch(api_client::update_environment).delete(api_client::delete_environment),
        )
        .route(
            "/workspaces/{wid}/api-client/environments/{id}/activate",
            post(api_client::activate_environment),
        )
        .route(
            "/workspaces/{wid}/api-client/history",
            get(api_client::list_history).delete(api_client::clear_history),
        )
        .route(
            "/workspaces/{wid}/api-client/execute",
            post(api_client::execute),
        )
        .route(
            "/workspaces/{wid}/api-client/secure-all",
            post(api_client::secure_all),
        )
        .route(
            "/workspaces/{wid}/api-client/grpc/describe",
            post(grpc::describe),
        )
        .route(
            "/workspaces/{wid}/api-client/grpc/invoke",
            post(grpc::invoke),
        )
        .route(
            "/workspaces/{wid}/api-client/grpc/reflect",
            post(grpc::reflect),
        )
        .route(
            "/workspaces/{wid}/api-client/oauth2/token",
            post(api_client::oauth2_token),
        )
        .route(
            "/workspaces/{wid}/api-client/cookies",
            get(api_client::list_cookies).delete(api_client::clear_cookies),
        )
        .route(
            "/workspaces/{wid}/api-client/automations",
            get(api_client::list_automations).post(api_client::create_automation),
        )
        .route(
            "/workspaces/{wid}/api-client/automations/{id}",
            patch(api_client::update_automation).delete(api_client::delete_automation),
        )
        .route(
            "/workspaces/{wid}/api-client/automations/{id}/run",
            post(api_client::run_automation),
        )
        .route(
            "/workspaces/{wid}/api-client/postman/sync",
            post(api_client::postman_sync),
        )
        .route("/api-client/import-curl", post(api_client::import_curl))
        // --- Share-link: session-level mint + list (mobile plan Task 1.9) --
        .route("/sessions/{id}/share", post(share::mint_share))
        .route("/sessions/{id}/shares", get(share::list_shares))
        // --- Conversation view (design docs/design/conversation-view.md §4.3):
        // transcript rebuilt from the provider's JSONL, extracted images,
        // produced artifacts (served by opaque id), board→agent tasks, the
        // composer's image inbox, and the History surface.
        .route("/sessions/{id}/transcript", get(transcript::get_transcript))
        .route(
            "/sessions/{id}/transcript/touch",
            post(transcript::touch_transcript),
        )
        .route(
            "/sessions/{id}/slash-commands",
            get(slash_commands::slash_commands),
        )
        .route(
            "/sessions/{id}/transcript/images/{img_id}",
            get(transcript::transcript_image),
        )
        .route("/sessions/{id}/artifacts", get(transcript::list_artifacts))
        .route(
            "/sessions/{id}/artifacts/{artifact_id}",
            get(transcript::get_artifact),
        )
        .route("/sessions/{id}/tasks", post(transcript::create_task))
        .route("/sessions/{id}/inbox", post(transcript::inbox_upload))
        .route("/workspaces/{wid}/history", get(transcript::history))
        .route(
            "/workspaces/{wid}/history/transcript",
            get(transcript::history_transcript),
        )
        .route(
            "/workspaces/{wid}/history/transcript/images/{img_id}",
            get(transcript::history_transcript_image),
        )
        .route(
            "/workspaces/{wid}/history/import",
            post(transcript::history_import),
        )
        .route(
            "/workspaces/{wid}/transcript/touch",
            post(transcript::touch_workspace_transcripts),
        )
        .route(
            "/workspaces/{wid}/history/rescan",
            post(transcript::history_rescan),
        )
        .route(
            "/sessions/{id}/handover",
            post(handover::handover_session),
        )
        .route(
            "/sessions/{id}/handover/brief",
            post(handover::handover_brief),
        )
        // --- Workflow engine --------------------------------------------
        .route("/workflows/node-types", get(workflows::node_types))
        .route("/workflows/templates", get(workflows::list_templates))
        .route(
            "/workspaces/{wid}/workflows",
            get(workflows::list_workflows).post(workflows::create_workflow),
        )
        .route(
            "/workspaces/{wid}/workflows/from-template",
            post(workflows::create_from_template),
        )
        .route(
            "/workspaces/{wid}/workflows/generate",
            post(workflows::generate_workflow),
        )
        .route(
            "/workflows/{id}",
            get(workflows::get_workflow)
                .patch(workflows::update_workflow)
                .delete(workflows::delete_workflow),
        )
        .route("/workflows/{id}/run", post(workflows::run_workflow))
        .route("/workflows/{id}/runs", get(workflows::list_runs))
        // In-flight runs across a workspace (the "Running" sidebar list).
        .route(
            "/workspaces/{wid}/workflow-runs/active",
            get(workflows::list_active_runs),
        )
        // Workflow versioning (history + restore).
        .route("/workflows/{id}/versions", get(workflows::list_versions))
        .route(
            "/workflows/{id}/versions/{v}",
            get(workflows::get_version),
        )
        .route(
            "/workflows/{id}/versions/{v}/restore",
            post(workflows::restore_version),
        )
        // Workflow triggers CRUD (schedule / event kinds; webhook is in public_routes).
        .route(
            "/workflows/{id}/triggers",
            get(workflows::list_triggers).post(workflows::create_trigger),
        )
        .route(
            "/workflow-triggers/{id}",
            patch(workflows::update_trigger).delete(workflows::delete_trigger),
        )
        .route("/workflow-runs/{id}", get(workflows::get_run))
        .route("/workflow-runs/{id}/cancel", post(workflows::cancel_run))
        .route("/workflow-runs/{id}/retry-node", post(workflows::retry_run_node))
        // Human-approval resume: requires bearer auth, Editor in the run's workspace.
        .route("/workflow-runs/{id}/approve", post(workflows::approve_run))
        // --- Snips (screenshot → annotate → clipboard) -------------------
        .merge(snips::snips_routes())
        // --- Browser (reader/annotate tabs + on-demand page fetch) -------
        .merge(browser::routes())
}

// ── The `scratch` workspace is a SESSION home, not a workspace API ──────────
// `WorkspacesRepo::role_of` hands every authenticated user an implicit Editor
// role on `SCRATCH_WORKSPACE_ID` so workspace-less sessions need no membership
// rows. `require_ws_role` is the only gate on ~40 other `/workspaces/{wid}/…`
// route families (api-client collections/environments/cookies, workflows,
// mcp-servers, connections, vault, memory, …), so without a second gate that
// implicit Editor would make every one of those families a shared, world-
// writable store keyed on a workspace whose root is the daemon's `$HOME`.
//
// The gate is a path check, not a role change: only the routes contract #16a
// actually promises under `scratch` — the session family, the broadcast relay
// and the activity summary — exist there; everything else answers `404`, as if
// the route had never been mounted for that id.

/// Whether `path` may reach a handler. True for every path that is not under
/// `/workspaces/scratch/…` (this guard's only business) and for the session
/// family + `broadcast` + `activity/summary` inside it.
///
/// Accepts the path with or without the `/api/v1` prefix: the layer runs inside
/// the `nest("/api/v1", …)`, where axum has already stripped it, while callers
/// and tests speak full URLs.
fn scratch_path_allowed(path: &str) -> bool {
    let p = path.strip_prefix("/api/v1").unwrap_or(path);
    let Some(rest) = p.strip_prefix("/workspaces/") else {
        return true;
    };
    // `/workspaces/scratch` itself (contract #16a) has no tail — untouched.
    let Some((wid, tail)) = rest.split_once('/') else {
        return true;
    };
    // axum routes on the RAW path but hands `{wid}` to handlers percent-DECODED,
    // so `/workspaces/scr%61tch/workflows` would reach the scratch workspace
    // through a raw-string comparison. Decode the id before comparing; the tail
    // stays raw, which only ever refuses more (an encoded `sessions` 404s).
    if pct_decode(wid) != SCRATCH_WORKSPACE_ID {
        return true;
    }
    tail == "sessions"
        || tail.starts_with("sessions/")
        || tail == "broadcast"
        || tail == "activity/summary"
        // Not a session route, but the one the contract promises answers `409`
        // ("member edits → 409", `reject_system_workspace`); a 404 here would
        // silently change that documented answer. `GET` is admin-gated and the
        // row has no members, so nothing leaks.
        || tail == "members"
}

/// Minimal percent-decode for one path segment (ASCII comparison only — an
/// undecodable byte run is left as-is, which can only make the segment differ
/// from `scratch` and so never opens a path up).
fn pct_decode(seg: &str) -> std::borrow::Cow<'_, str> {
    if !seg.contains('%') {
        return std::borrow::Cow::Borrowed(seg);
    }
    let b = seg.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    std::borrow::Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

/// `404` for any `/workspaces/scratch/…` route outside the session family.
/// Layered over the whole protected router (core routes + every module extra)
/// in `build_router`, so a family added later is covered by default.
pub async fn scratch_guard(req: Request, next: Next) -> Response {
    if !scratch_path_allowed(req.uri().path()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod scratch_guard_tests {
    use super::scratch_path_allowed;

    #[test]
    fn session_family_broadcast_and_activity_summary_are_allowed() {
        for p in [
            "/api/v1/workspaces/scratch/sessions",
            "/api/v1/workspaces/scratch/sessions/01X/transcript",
            "/api/v1/workspaces/scratch/broadcast",
            "/api/v1/workspaces/scratch/activity/summary",
            // Kept reachable so the system-workspace guard still answers 409.
            "/api/v1/workspaces/scratch/members",
            // The same paths as the nested router sees them.
            "/workspaces/scratch/sessions",
            "/workspaces/scratch/sessions/01X/tasks",
        ] {
            assert!(scratch_path_allowed(p), "{p} must stay reachable");
        }
    }

    #[test]
    fn every_other_workspace_family_is_refused_under_scratch() {
        for p in [
            "/api/v1/workspaces/scratch/api-client/environments",
            "/api/v1/workspaces/scratch/workflows",
            "/api/v1/workspaces/scratch/mcp-servers",
            "/api/v1/workspaces/scratch/connections",
            "/api/v1/workspaces/scratch/history/transcript",
            "/workspaces/scratch/api-client/cookies",
            // A near-miss that must not slip through the prefix test.
            "/api/v1/workspaces/scratch/sessions-export",
            "/api/v1/workspaces/scratch/",
            // Percent-encoded id: axum would still route it to `wid = scratch`.
            "/api/v1/workspaces/scr%61tch/workflows",
        ] {
            assert!(!scratch_path_allowed(p), "{p} must 404");
        }
    }

    #[test]
    fn paths_outside_the_scratch_subtree_are_untouched() {
        for p in [
            "/api/v1/workspaces/scratch",
            "/workspaces/scratch",
            "/api/v1/workspaces/01X/workflows",
            "/api/v1/workspaces",
            "/api/v1/sessions/01X",
            "/health",
        ] {
            assert!(scratch_path_allowed(p), "{p} must be left alone");
        }
    }
}
