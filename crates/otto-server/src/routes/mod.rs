//! Core REST routes (contract endpoints #1-16, #57-58).

pub mod activity;
pub mod admin_sessions;
pub mod api_client;
pub mod backup;
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
pub mod handover;
pub mod impersonate;
pub mod logs;
pub mod mcp_servers;
pub mod meta;
pub mod mission;
pub mod name_themes;
pub mod notifications;
pub mod onboarding;
pub mod product_memory;
pub mod settings;
pub mod share;
pub mod swarm_ingest;
pub mod usage;
pub mod users;
pub mod workflows;
pub mod search;
pub mod workspaces;

use axum::routing::{delete, get, patch, post, put};
use axum::Router;

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
        .route(
            "/workspaces/{id}",
            patch(workspaces::update).delete(workspaces::archive),
        )
        .route(
            "/workspaces/{id}/members",
            get(workspaces::members).put(workspaces::set_members),
        )
        .route("/settings", get(settings::get_all).put(settings::put_all))
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
        .route("/api-client/import-curl", post(api_client::import_curl))
        // --- Share-link: session-level mint + list (mobile plan Task 1.9) --
        .route("/sessions/{id}/share", post(share::mint_share))
        .route("/sessions/{id}/shares", get(share::list_shares))
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
        // Human-approval resume: requires bearer auth, Editor in the run's workspace.
        .route("/workflow-runs/{id}/approve", post(workflows::approve_run))
}
