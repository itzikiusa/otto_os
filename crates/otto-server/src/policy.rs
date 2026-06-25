//! RBAC route → `(Feature, Capability)` policy table (design spec §3.2/§3.4).
//!
//! The central feature-policy guard (Task 1.4) reads the Axum `MatchedPath` (the
//! `{id}`-templated route, **including** the `/api/v1` nest prefix) and the HTTP
//! method, then looks the pair up here to decide whether the *feature* axis
//! permits the request. This module is just that lookup — pure, allocation-free,
//! and exhaustively unit-tested. The guard layers the verdict on top of root
//! bypass + the orthogonal workspace-role / ownership gates (which stay in the
//! handlers); see the spec's three-gate model.
//!
//! ## Verdicts
//! - [`PolicyDecision::Exempt`] — the request is **not** feature-gated. This is an
//!   explicit allow-list: public routes, per-session-`?token=` ingest routes, and
//!   the cross-cutting *self-owned* routes (Auth/PAT self-management, `/auth/me`,
//!   Notifications, the FS-browse sandbox) plus the pure workspace-axis routes
//!   (workspace CRUD / members / MCP-server config) whose authorization is the
//!   `require_ws_role` gate the central guard can't replace. Static catalogs
//!   (`/swarm/presets`, `/workflows/node-types`) are exempt too.
//! - [`PolicyDecision::Require`]`(feature, capability)` — allow iff the caller is
//!   root or `capability_of(user, feature) >= capability`.
//! - [`PolicyDecision::Deny`] — **fail closed**. The default arm: any protected
//!   route with no matching rule is denied (403). The policy table is the
//!   allow-list; a newly-added route without an entry fails closed until someone
//!   maps it (backstopped by the Task 1.5 coverage test).
//!
//! ## Capability ladder (per feature, from §3.2)
//! Reads ⇒ `View`, writes/runs ⇒ `Edit`, management ⇒ `Admin`. The flagged
//! mismatches the spec calls out are encoded *exactly*:
//! - self-improvement **config** PUT = `SelfImprovement:Admin`, **run** = `:Edit`;
//! - workspace **context** PUT = `Context:Admin` (materialize = `:Edit`, read/preview = `:View`);
//! - **library** GET = `Skills:View`, library PUT/DELETE = `Skills:Admin`;
//! - skill-eval **promote** = `SkillEval:Admin`;
//! - usage/insights/settings/users/daemon-diagnostics = their feature at `Admin`
//!   (root-only today; now grantable through the feature grant).

use axum::http::Method;
use otto_core::domain::{Capability, Feature};

/// The feature-axis verdict for a `(method, matched-path)` pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PolicyDecision {
    /// Not feature-gated (public / token / self-owned / workspace-axis / catalog).
    Exempt,
    /// Allow iff root or `capability_of(user, feature) >= capability`.
    Require(Feature, Capability),
    /// Fail closed (unknown protected route, or an explicitly-denied one).
    Deny,
}

use Capability::{Admin, Edit, View};
use Feature::*;
use PolicyDecision::{Deny, Exempt, Require};

/// Map an `(method, matched_path)` pair to its feature-axis [`PolicyDecision`].
///
/// `matched_path` is the Axum route **template** with `{id}`-style placeholders,
/// including the `/api/v1` nest prefix the daemon mounts the API under — e.g.
/// `/api/v1/connections/{id}/db/query`. Root-mounted WebSocket / proxy routers
/// (`/ws/...`, `/browser/proxy`) self-authenticate via `?token=` and never reach
/// the central guard, so they are not represented here.
pub fn policy_for(method: &Method, matched_path: &str) -> PolicyDecision {
    // Strip the `/api/v1` nest prefix so the rules read against the
    // handler-relative templates (the form the route files declare). A path that
    // somehow lacks the prefix is matched as-is (defensive; in practice every
    // routed request carries it).
    let p = matched_path.strip_prefix("/api/v1").unwrap_or(matched_path);

    let get = method == Method::GET;
    let put = method == Method::PUT;
    // A "write" is anything that mutates: POST/PUT/PATCH/DELETE. (We only need to
    // distinguish reads from writes, plus PUT for the config-vs-read mismatches.)
    let write = !get && method != Method::HEAD && method != Method::OPTIONS;

    // ----------------------------------------------------------------------
    // 1. Exempt — public, per-session-token ingest, and self-owned / workspace-
    //    axis / catalog routes. This is an explicit allow-list (order matters:
    //    it precedes the feature families below).
    // ----------------------------------------------------------------------

    // Public (also served outside the bearer middleware, but the guard may still
    // see them — keep them exempt regardless).
    if matches!(p, "/health" | "/meta" | "/onboarding/root" | "/auth/login") {
        return Exempt;
    }
    // Provider activity / usage / swarm-board ingest — gated by the per-session
    // token Otto sets on the agent PTY, not a user bearer token.
    if p.starts_with("/ingest/") {
        return Exempt;
    }
    // Runtime custom plugins. The host API (`/plugin-host/*`) is sidecar-token
    // authed (like `/ingest/*`); the enabled-plugins list (`/plugins`) is a
    // self-scoped read any authed user may make for their sidebar. The reverse-
    // proxy routes (`/plugins/{slug}/…`) are NOT here — they are feature-gated by
    // the dedicated plugin branch in `feature_guard` before this table is reached
    // (and excluded from the policy-coverage test for that reason).
    if p.starts_with("/plugin-host/") || p == "/plugins" {
        return Exempt;
    }
    // Plugin management (install / enable / disable / remove) — admin only.
    if p == "/plugin-admin" || p.starts_with("/plugin-admin/") {
        return Require(Users, Admin);
    }
    // Auth / personal-access-token self-management + identity. Any authed user
    // manages their own session and tokens; never feature-gated.
    // `/auth/capabilities` is self-scoped (returns the caller's own grants) —
    // any authenticated user may call it; no feature gate needed.
    // Share revocation (`/auth/shares/*`) is self-owned, like `/auth/tokens/*`:
    // a user revokes their own shares; no feature grant needed.
    if matches!(p, "/auth/me" | "/auth/logout" | "/auth/tokens" | "/auth/capabilities")
        || p.starts_with("/auth/tokens/")
        || p.starts_with("/auth/shares")
    {
        return Exempt;
    }
    // Per-user notifications (self-owned).
    if p == "/notifications" || p.starts_with("/notifications/") {
        return Exempt;
    }
    // Per-user email sender (Gmail App Password → Keychain; mobile plan Task
    // 7.1). Self-owned — any authed user configures/reads their OWN sender, like
    // `/auth/tokens`; no feature grant needed. The app password lives in the
    // Keychain, never the DB.
    if p == "/email-sender" {
        return Exempt;
    }
    // Email-OTP share redemption (mobile plan Task 7.3). PUBLIC by design: the
    // share token in the request body is the auth, so it must be reachable while
    // the scoped token is still OTP-pending (the feature guard otherwise denies a
    // pending share everything). It is mounted as a public route and never reaches
    // this guard, but the coverage test still requires an explicit Exempt entry.
    if p == "/share/verify" {
        return Exempt;
    }
    // Email-OTP share EXTENSION (mobile plan Task 7.4). PUBLIC by design, same as
    // `/share/verify`: the share token in the request body is the auth, and extend
    // must be reachable even after the window elapses (when the scoped token is
    // OTP-pending and the feature guard denies everything else). The fresh OTP is
    // emailed to the LOCKED original recipient read from the row — never the
    // request — so there is no email field to validate here. Mounted as a public
    // route; the coverage test still requires this explicit Exempt entry.
    if p == "/share/extend" {
        return Exempt;
    }
    // FS-browse sandbox (cross-cutting; sandboxed in the handler).
    if matches!(p, "/fs/browse" | "/fs/read") {
        return Exempt;
    }
    // Static catalogs (no per-user data; safe to read for any authed user).
    if matches!(
        p,
        "/swarm/presets" | "/workflows/node-types" | "/lsp/capabilities"
    ) {
        return Exempt;
    }
    // Pure workspace-axis routes: workspace CRUD, membership, and per-workspace
    // MCP-server config are authorized solely by the `require_ws_role` gate the
    // central guard cannot replace (entity→workspace lookups live in the
    // handlers). They are not in the §3.1 feature set, so the feature axis is a
    // no-op for them. (Workspace creation is open to any authed user; the creator
    // becomes owner — preserved.)
    if p == "/workspaces" || p == "/workspaces/{id}" {
        return Exempt;
    }
    if p == "/workspaces/{id}/members" {
        return Exempt;
    }
    if p == "/mcp-servers/{id}" || p == "/workspaces/{id}/mcp-servers" {
        return Exempt;
    }

    // ----------------------------------------------------------------------
    // 2. Flagged mismatches — encode these *before* the generic prefix families
    //    so the special-cased capability wins.
    // ----------------------------------------------------------------------

    // Self-improvement: config GET=View / PUT=Admin; run=Edit; runs/edits read=View;
    // approve/reject/rollback=Edit. (§3.2: config=Admin, run=Edit.)
    if p.ends_with("/self-improvement") {
        return Require(SelfImprovement, if put { Admin } else { View });
    }
    if p.ends_with("/self-improvement/run") {
        return Require(SelfImprovement, Edit);
    }
    if p.contains("/improvement/runs") {
        return Require(SelfImprovement, View);
    }
    if p.ends_with("/improvement/edits") {
        return Require(SelfImprovement, View);
    }
    if p.starts_with("/improvement/edits/") {
        // approve / reject / rollback
        return Require(SelfImprovement, Edit);
    }

    // Workspace context: read GET=View, materialize POST=Edit, preview POST=View,
    // config PUT=Admin (§3.2 flagged: context PUT=Admin).
    if p == "/workspaces/{id}/context" {
        return Require(Context, if put { Admin } else { View });
    }
    if p == "/workspaces/{id}/context/materialize" {
        return Require(Context, Edit);
    }
    if p == "/workspaces/{id}/context/preview" {
        return Require(Context, View); // dry-run read
    }

    // Library (Skills feature): GET=View, PUT/DELETE=Admin (§3.2 flagged).
    // Covers /library/{skills,souls,context,default-soul} and the bundled-skills
    // install endpoints (install bundled→local = Edit, per the ladder).
    if p.starts_with("/library/bundled") {
        // GET /library/bundled lists bundled skills (read = View); the POST
        // install / install-all endpoints install bundled→local skills (Edit).
        return Require(Skills, if get { View } else { Edit });
    }
    if p.starts_with("/library/") || p == "/library" {
        return Require(Skills, if get { View } else { Admin });
    }

    // Skill-eval: reads=View, run/cancel=Edit, promote=Admin (§3.2 flagged),
    // config GET=View / PUT=Admin.
    if p == "/settings/skill-eval" {
        return Require(SkillEval, if put { Admin } else { View });
    }
    if p.ends_with("/skill-evaluations/{id}/promote") || p.ends_with("/promote") {
        return Require(SkillEval, Admin);
    }
    if p.starts_with("/workspaces/{id}/skill-evaluations")
        || p.starts_with("/workspaces/{id}/skill-sources")
        || p.starts_with("/skill-evaluations/")
        || p == "/skill-evaluations"
    {
        // start eval (POST) / cancel / retry = Edit; list / get / diff = View.
        return Require(SkillEval, if write { Edit } else { View });
    }

    // ----------------------------------------------------------------------
    // 3. Per-feature prefix families. Reads=View, writes=Edit, management=Admin.
    // ----------------------------------------------------------------------

    // ---- Database (DB Explorer over a connection + saved queries/dashboards) --
    // Note: `/connections/{id}/db/...` is the *Database* feature, distinct from
    // *Connections* profile management below. Check the db sub-prefix first.
    if p.starts_with("/connections/{id}/db/") {
        // Reads: capabilities / schema / history (GET). Schema browse via POST
        // (schema/children, object, schema-graph) is still a read. Mutating SQL,
        // DDL, query execution, cancel, completion, explain-with-agent = Edit.
        // Export (`db/export`) executes a full-result query — Edit tier.
        let read = get
            || p.ends_with("/db/schema/children")
            || p.ends_with("/db/object")
            || p.ends_with("/db/schema-graph")
            || p.ends_with("/db/test"); // connectivity probe — non-mutating
        return Require(Database, if read { View } else { Edit });
    }
    if p.starts_with("/workspaces/{wid}/db/")
        || p.starts_with("/db/saved-queries")
        || p.starts_with("/db/dashboards")
        || p.starts_with("/db/widgets")
    {
        // Saved queries / dashboards / widgets: list/get=View, CRUD/run=Edit.
        return Require(Database, if get { View } else { Edit });
    }

    // ---- Connections (profile management: list/open/create/edit/delete/sections)
    // Per §3.2: list=View, open/create/edit/sections=Edit, manage-all/global=Admin.
    // `POST /connections` (workspace-less create) and section management are
    // treated as Admin (global / management); `/workspaces/{id}/connections`
    // create is the per-workspace create = Edit.
    if p.starts_with("/workspaces/{id}/connections/import/") {
        // Import connection profiles from other DB tools (MySQL Workbench /
        // DBeaver / DataGrip / NoSQLBooster): detect/preview = View; scan reads
        // the local tool config + create creates connections = Edit. Matches the
        // per-workspace connection-create tier. (Must precede the exact-match
        // `/workspaces/{id}/connections` rule below.)
        return Require(Connections, if get { View } else { Edit });
    }
    if p == "/workspaces/{id}/connections" {
        // GET list = View, POST create = Edit.
        return Require(Connections, if get { View } else { Edit });
    }
    if p == "/connections/{id}/open" || p == "/connections/{id}/test" {
        return Require(Connections, Edit);
    }
    if p == "/connections/{id}/pin" {
        // Toggle pin/recency-order — Edit tier (personal preference, not admin).
        return Require(Connections, Edit);
    }
    if p.starts_with("/connections/{id}/sftp/") {
        // SFTP file browser over an SSH connection: browse/read (GET) = View;
        // download/upload/mkdir/remove/rename (POST, mutate the user's disk or
        // the remote) = Edit. Distinct from the `/db/` (Database) prefix above.
        return Require(Connections, if get { View } else { Edit });
    }
    if p == "/connections" {
        // Workspace-less (global) connection create — management tier.
        return Require(Connections, Admin);
    }
    if p == "/connections/{id}" {
        // PATCH / DELETE a connection profile — management tier (manage-all).
        return Require(Connections, Admin);
    }
    if p.starts_with("/workspaces/{id}/connection-sections")
        || p.starts_with("/connection-sections/")
    {
        // Section CRUD / reorder / move — organizing connection profiles.
        return Require(Connections, if get { View } else { Edit });
    }

    // ---- Agents (sessions, orchestration, broadcast, handover, input, LSP) ----
    // §3.2: list/inspect own = View; create/restart/archive/input/broadcast/
    // orchestrate = Edit. No Admin tier.
    // Send-to-agent context packets (preview + send) inject into a session = Edit
    // (the handler additionally enforces session owner/admin).
    if p.starts_with("/workspaces/{wid}/agents/") {
        return Require(Agents, Edit);
    }
    // Mission Control (B4): work-queue read aggregation = View; saved-view
    // create/delete = Edit (the handler adds the per-view owner guard).
    if p.starts_with("/workspaces/{id}/mission") {
        return Require(Agents, if get { View } else { Edit });
    }
    if p.starts_with("/mission-views/") {
        return Require(Agents, Edit);
    }
    // Cross-module workspace search (B5): broad read-only fan-out.
    if p == "/workspaces/{id}/search" {
        return Require(Agents, View);
    }
    if p == "/workspaces/{id}/sessions" {
        return Require(Agents, if get { View } else { Edit });
    }
    if p == "/sessions/{id}" {
        // GET inspect = View; PATCH/DELETE = Edit.
        return Require(Agents, if get { View } else { Edit });
    }
    // Share-link management (mobile plan Task 1.9): mint = Edit (owner/editor
    // action); list = View. Both require the caller to own/admin the session
    // (enforced in the handler by `require_session_owner_or_admin`). These
    // specific patterns are tested BEFORE the generic `/sessions/` prefix below.
    if p == "/sessions/{id}/shares" {
        return Require(Agents, if get { View } else { Edit });
    }
    if p == "/sessions/{id}/share" {
        // POST only (mint). A GET on this path would be a typo; map it Edit too.
        return Require(Agents, Edit);
    }
    if p == "/sessions/{id}/evolve" {
        // Manual trigger for the per-session live-evolve pass.
        // Requires SelfImprovement:Edit so it obeys the same gate as a workspace run.
        return Require(SelfImprovement, Edit);
    }
    if p.starts_with("/sessions/") {
        // restart / archive / unarchive / input / handover / handover-brief /
        // attach-product — all session-control writes.
        return Require(Agents, Edit);
    }
    if p == "/app/kill-sessions" {
        // Kill-all is an Agents write here (Task 3.3 additionally root-gates it).
        return Require(Agents, Edit);
    }
    if matches!(
        p,
        "/workspaces/{id}/orchestrate"
            | "/workspaces/{id}/orchestrate/execute"
            | "/workspaces/{id}/broadcast"
            | "/workspaces/{id}/relay"
            | "/workspaces/{id}/providers/update"
            | "/workspaces/{id}/lsp/install"
    ) {
        return Require(Agents, Edit);
    }
    // Session name themes (auto-naming new sessions). Per-user library: reading
    // the theme list = Agents View; creating/editing/selecting a theme = Agents
    // Edit. The handlers add the per-theme owner guard.
    if p == "/name-themes" || p.starts_with("/name-themes/") {
        return Require(Agents, if get { View } else { Edit });
    }

    // ---- Git (repos, status/diff/log/PRs, commit/push/pull, accounts) ---------
    // §3.2: view repos/status/diff/log/PRs=View; commit/push/pull/PR ops/manage
    // own accounts=Edit. No Admin tier.
    if p == "/git/accounts" {
        return Require(Git, if get { View } else { Edit });
    }
    if p.starts_with("/git/accounts/") {
        // remote-repos (GET) = View; account update/delete = Edit.
        return Require(Git, if get { View } else { Edit });
    }
    if p == "/git/repos" {
        // Workspace-independent global repo list (GET only) — read = View.
        // The handler additionally filters to the workspaces the caller may see.
        return Require(Git, View);
    }
    if p == "/workspaces/{id}/repos" || p == "/workspaces/{id}/repos/detect" {
        // list workspace repos (GET=View); add/detect repo (POST=Edit).
        return Require(Git, if get { View } else { Edit });
    }
    if p.starts_with("/repos/") {
        // status/branches/refs/log/diff/prs (+ comments/commits) reads=View; the
        // PR review machinery, commit/push/pull/merge/stage etc. writes=Edit.
        return Require(Git, if get { View } else { Edit });
    }
    if p.starts_with("/pr-review-comments/") {
        // approve / decline a review comment: these are always writes.
        return Require(Git, Edit);
    }
    if p.starts_with("/reviews/") {
        // review handoff / agent retry / state mutations = Edit;
        // findings list + merge-readiness are read-only = View.
        // Specifically: GET .../findings and GET .../merge-readiness are View;
        // POST .../findings/{fp}/state is Edit; all other POSTs are Edit.
        return Require(Git, if get { View } else { Edit });
    }
    if p == "/settings/pr-review" {
        // PR-review config — read=View, write=Edit (it's a Git-feature setting,
        // not a daemon Settings concern).
        return Require(Git, if get { View } else { Edit });
    }

    // ---- Issues (Jira / Confluence) -------------------------------------------
    // §3.2: read issues/search=View; comment/transition/assign/manage accounts=Edit.
    if p.starts_with("/issue/") || p == "/issue" {
        // accounts list/create/update/delete: list=View, mutate=Edit.
        // reads (projects/search/confluence/get/full/transitions GET/assignable/
        // issue-types/attachment)=View; comment/transition POST/assignee PUT=Edit.
        return Require(Issues, if get { View } else { Edit });
    }

    // ---- Product (stories / analyses / learnings / testcases) -----------------
    // §3.2: read stories/analyses=View; create/edit/run/publish=Edit.
    if p.starts_with("/product/")
        || p.starts_with("/workspaces/{ws}/product/")
        || p.starts_with("/workspaces/{id}/product/")
    {
        return Require(Product, if get { View } else { Edit });
    }

    // ---- Canvas (visual scenes: CRUD + agent assist) --------------------------
    // §3.2 analogue: read scenes=View; create/edit/delete/assist=Edit. Item
    // routes (`/canvas/scenes/{id}`, `/canvas/scenes/{id}/assist`, `/canvas/assist/
    // preview`) are covered by the `/canvas/` prefix; the collection route uses
    // the `{ws}` placeholder (see otto-canvas router) so it matches here. Root
    // bypasses.
    if p.starts_with("/canvas/") || p.starts_with("/workspaces/{ws}/canvas/") {
        return Require(Canvas, if get { View } else { Edit });
    }

    // ---- Memory (knowledge vault: memories / graph / recall / ingest) ---------
    // The memory layer has no dedicated Feature key; it is workspace knowledge
    // produced and consumed by the Product workflows (the per-story ingest route
    // is mounted under `/product/…` and already covered above). We gate the
    // standalone memory API on Product: read=View, mutate=Edit. Root bypasses.
    if p.starts_with("/workspaces/{ws}/memories")
        || p.starts_with("/workspaces/{ws}/memory/")
    {
        return Require(Product, if get { View } else { Edit });
    }

    // ---- Message Brokers (Kafka viewer: clusters / topics / groups / schema / --
    // sidebar cluster-sections) --
    // The brokers layer has no dedicated Feature key; it is data-infrastructure
    // browsing, a sibling of the DB Explorer (connect to a system, inspect/peek/
    // produce). We gate it on the *Database* feature: read=View, mutate=Edit.
    // Connectivity test and message consume/peek are non-mutating reads (View),
    // mirroring the `/db/test` precedent above. The `/brokers/cluster` prefix
    // covers both `/brokers/clusters*` and `/brokers/cluster-sections*`. Root
    // bypasses.
    if p.starts_with("/brokers/cluster")
        || p.starts_with("/workspaces/{wid}/brokers/cluster")
    {
        let read = get
            || p.ends_with("/test") // connectivity probe — non-mutating
            || p.ends_with("/consume"); // peek messages — does not mutate the topic
        return Require(Database, if read { View } else { Edit });
    }

    // ---- Swarm ----------------------------------------------------------------
    // §3.2: view swarms/board/runs=View; create/edit/run/start/pause/abort/
    // recruit/plan=Edit.
    if p.starts_with("/swarm/") || p.starts_with("/workspaces/{id}/swarm/") {
        return Require(Swarm, if get { View } else { Edit });
    }

    // ---- Goal Loops -----------------------------------------------------------
    // Workspace-axis feature: every handler enforces ws Viewer/Editor via the
    // role check (`require_ws_role`), so the feature-capability axis is not gated
    // here. (A dedicated GoalLoops capability is a possible future enhancement.)
    if p.starts_with("/goal-loops/")
        || p == "/goal-loops"
        || p.starts_with("/workspaces/{id}/goal-loops")
    {
        return Exempt;
    }

    // ---- ApiClient ("Postman") ------------------------------------------------
    // §3.2: read collections/history=View; create/edit/execute/grpc/oauth=Edit.
    if p.starts_with("/workspaces/{wid}/api-client/") || p == "/api-client/import-curl" {
        return Require(ApiClient, if get { View } else { Edit });
    }

    // ---- Workflows ------------------------------------------------------------
    // §3.2: view=View; create/edit/run=Edit. (node-types catalog already exempt.)
    // Webhook trigger is PUBLIC-by-token (registered in public_routes); the handler
    // validates the path token against workflow_triggers — exempt from feature gate.
    if p == "/workflows/{id}/webhook/{token}" {
        return Exempt;
    }
    if p == "/workflows/templates" {
        return Require(Workflows, View);
    }
    if p.starts_with("/workflows/")
        || p == "/workflows"
        || p.starts_with("/workflow-runs/")
        || p.starts_with("/workflow-triggers/")
        || p.starts_with("/workspaces/{wid}/workflows")
    {
        return Require(Workflows, if get { View } else { Edit });
    }

    // ---- Channels (Slack / Telegram / Webhook integrations) -------------------
    // §3.2: view integrations=View; configure=Edit.
    // Inbound channel webhook is PUBLIC-by-key (registered in public_routes); the
    // handler validates the `X-Otto-Webhook-Key` against the keychain — exempt
    // from the feature gate, like the workflow webhook trigger.
    if p == "/webhooks/{workspace_id}" {
        return Exempt;
    }
    // Inbound swarm-trigger webhook is PUBLIC-by-key too (registered in
    // public_routes); the handler validates the same per-workspace webhook key
    // via `X-Otto-Webhook-Key` / `Authorization: Bearer` — exempt from the
    // feature gate, like the channel webhook above.
    if p == "/webhooks/swarm/{workspace_id}/{swarm_id}" {
        return Exempt;
    }
    if p.starts_with("/workspaces/{id}/integrations") {
        return Require(Channels, if get { View } else { Edit });
    }

    // ---- Insights -------------------------------------------------------------
    // §3.2: read reports=View; trigger runs=Edit; configure scheduler=Admin.
    if p == "/insights/config" {
        return Require(Insights, if put { Admin } else { View });
    }
    if p == "/insights/run" {
        return Require(Insights, Edit);
    }
    if p == "/insights/reports" || p == "/insights/report" {
        return Require(Insights, View);
    }

    // ---- Usage ----------------------------------------------------------------
    // §3.2: read summary/metrics=View; configure/install engine=Admin. Budgets are
    // spend-cap configuration ⇒ Admin (write) / View (read).
    if p == "/usage/config" || p == "/usage/install" {
        return Require(Usage, Admin);
    }
    if p == "/usage/budgets" {
        return Require(Usage, if get { View } else { Admin });
    }
    if p.starts_with("/usage/") {
        // status / summary / by-kind / metrics — reads.
        return Require(Usage, View);
    }

    // ---- Settings + daemon diagnostics (root-only today; now grantable) -------
    // §3.2: read/write daemon settings=Settings:Admin. The Trust & Safety Center
    // (audit-log, security-posture) and daemon logs are daemon-wide diagnostics —
    // map them to Settings:Admin (their `require_root` handler gate stays).
    if p == "/settings" {
        return Require(Settings, Admin);
    }
    // Capability/health registry + support bundle — daemon diagnostics (root),
    // same tier as the audit log / security posture below.
    if matches!(p, "/capabilities" | "/support-bundle") {
        return Require(Settings, Admin);
    }
    // Settings export/import + state backup/restore (C3) — root diagnostics, same
    // tier as the audit log; handlers also enforce require_root.
    if matches!(
        p,
        "/settings/export" | "/settings/import" | "/state/backup" | "/state/restore"
    ) {
        return Require(Settings, Admin);
    }
    if matches!(p, "/audit-log" | "/security-posture" | "/logs/daemon") {
        return Require(Settings, Admin);
    }

    // ---- Users (user CRUD + grants + impersonation + session overview) --------
    // §3.2: Users:Admin only.
    if p == "/users" || p.starts_with("/users/") {
        return Require(Users, Admin);
    }
    // Admin active-sessions overview + terminate (Task 4.2) — the sanctioned
    // cross-user view. `Users:Admin` (or root) governs it, so a non-root admin
    // can also use it; the handlers deliberately do NOT add a root check.
    if p == "/admin/sessions" || p.starts_with("/admin/sessions/") {
        return Require(Users, Admin);
    }
    // Admin impersonation (Task 5.2). START (`/admin/impersonate/{user_id}`) is
    // gated `Users:Admin` (or root) so a granted admin — not only root — can
    // begin acting-as another user; the handler then enforces the anti-escalation
    // guardrails (never impersonate root or a fellow Users-admin, no nesting, no
    // self, …) on top of this feature gate.
    //
    // STOP (`/admin/impersonate/stop`) is **self-scoped** — it revokes the
    // *presented* token to end the overlay. Authorization here runs against the
    // EFFECTIVE user (the impersonation target, typically a plain non-admin), so
    // it cannot be `Users:Admin`-gated or "Exit" would be impossible. Treat it
    // like `/auth/logout`: Exempt (any authed session may end its own overlay).
    if p == "/admin/impersonate/stop" {
        return Exempt;
    }
    if p.starts_with("/admin/impersonate") {
        return Require(Users, Admin);
    }
    // Activity trail / tasks / summary are Agents reads/writes (per-session data),
    // owner-scoped in Phase 3. trail/tasks GET=View, append/put=Edit; summary=View.
    if p == "/workspaces/{wid}/sessions/{sid}/trail"
        || p == "/workspaces/{wid}/sessions/{sid}/tasks"
    {
        return Require(Agents, if get { View } else { Edit });
    }
    if p == "/workspaces/{wid}/activity/summary" {
        return Require(Agents, View);
    }

    // ----------------------------------------------------------------------
    // 4. Default — fail closed.
    // ----------------------------------------------------------------------
    Deny
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;

    // Helper: every test path carries the `/api/v1` nest prefix the guard sees.
    fn pol(m: Method, path: &str) -> PolicyDecision {
        policy_for(&m, path)
    }

    // ---- Security-critical mappings from the plan (Task 1.3 Step 1) ----------

    #[test]
    fn db_read_is_view_write_is_edit() {
        assert_eq!(
            pol(Method::GET, "/api/v1/connections/{id}/db/tables"),
            Require(Database, View),
            "db schema/table reads are View"
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/connections/{id}/db/history"),
            Require(Database, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/connections/{id}/db/query"),
            Require(Database, Edit),
            "running SQL is Edit"
        );
    }

    #[test]
    fn connection_mgmt_is_admin() {
        assert_eq!(
            pol(Method::POST, "/api/v1/connections"),
            Require(Connections, Admin)
        );
        assert_eq!(
            pol(Method::DELETE, "/api/v1/connections/{id}"),
            Require(Connections, Admin)
        );
    }

    #[test]
    fn connection_open_is_edit() {
        assert_eq!(
            pol(Method::POST, "/api/v1/connections/{id}/open"),
            Require(Connections, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{id}/connections"),
            Require(Connections, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/connections"),
            Require(Connections, Edit)
        );
        // Import connections from other DB tools: detect=View, scan/create=Edit.
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{id}/connections/import/sources"),
            Require(Connections, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/connections/import/scan"),
            Require(Connections, Edit)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/connections/import/create"),
            Require(Connections, Edit)
        );
    }

    #[test]
    fn sftp_browse_is_view_transfer_is_edit() {
        // Browse/read the remote tree = Connections:View.
        assert_eq!(
            pol(Method::GET, "/api/v1/connections/{id}/sftp/list"),
            Require(Connections, View)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/connections/{id}/sftp/read"),
            Require(Connections, View)
        );
        // Transfers / mutations = Connections:Edit.
        assert_eq!(
            pol(Method::POST, "/api/v1/connections/{id}/sftp/download"),
            Require(Connections, Edit)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/connections/{id}/sftp/upload"),
            Require(Connections, Edit)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/connections/{id}/sftp/remove"),
            Require(Connections, Edit)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/connections/{id}/sftp/rename"),
            Require(Connections, Edit)
        );
    }

    #[test]
    fn users_and_settings_admin() {
        assert_eq!(pol(Method::POST, "/api/v1/users"), Require(Users, Admin));
        assert_eq!(
            pol(Method::PATCH, "/api/v1/users/{id}"),
            Require(Users, Admin)
        );
        // Task 2.1: grants endpoints fall under the /users/ prefix rule.
        assert_eq!(
            pol(Method::GET, "/api/v1/users/{id}/grants"),
            Require(Users, Admin)
        );
        assert_eq!(
            pol(Method::PUT, "/api/v1/users/{id}/grants"),
            Require(Users, Admin)
        );
        assert_eq!(
            pol(Method::PUT, "/api/v1/settings"),
            Require(Settings, Admin)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/settings"),
            Require(Settings, Admin)
        );
    }

    #[test]
    fn admin_sessions_overview_is_users_admin() {
        // Task 4.2: the cross-user overview + terminate are gated by Users:Admin
        // (root passes via the guard), NOT a separate root-only path.
        assert_eq!(
            pol(Method::GET, "/api/v1/admin/sessions"),
            Require(Users, Admin)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/admin/sessions/{id}/terminate"),
            Require(Users, Admin)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/admin/sessions/{id}/remove"),
            Require(Users, Admin)
        );
    }

    #[test]
    fn admin_impersonate_start_is_users_admin_stop_is_exempt() {
        // Task 5.2: START is gated by Users:Admin (root passes via the guard);
        // the handler enforces the anti-escalation guardrails on top.
        assert_eq!(
            pol(Method::POST, "/api/v1/admin/impersonate/{user_id}"),
            Require(Users, Admin)
        );
        // STOP is self-scoped (revokes the presented token) and must NOT require
        // Users:Admin — the effective user mid-impersonation is a plain user, so
        // an Admin gate would make "Exit" impossible.
        assert_eq!(
            pol(Method::POST, "/api/v1/admin/impersonate/stop"),
            Exempt
        );
    }

    #[test]
    fn public_routes_exempt() {
        assert_eq!(pol(Method::POST, "/api/v1/auth/login"), Exempt);
        assert_eq!(pol(Method::GET, "/health"), Exempt);
        assert_eq!(pol(Method::GET, "/meta"), Exempt);
        assert_eq!(pol(Method::POST, "/api/v1/onboarding/root"), Exempt);
        assert_eq!(pol(Method::POST, "/api/v1/ingest/claude"), Exempt);
        assert_eq!(pol(Method::POST, "/api/v1/ingest/usage"), Exempt);
        assert_eq!(pol(Method::POST, "/api/v1/ingest/swarm/board"), Exempt);
    }

    #[test]
    fn unknown_protected_path_denies() {
        assert_eq!(pol(Method::GET, "/api/v1/some/new/route"), Deny);
        assert_eq!(pol(Method::POST, "/api/v1/foo"), Deny);
    }

    // ---- Self-owned / cross-cutting exemptions -------------------------------

    #[test]
    fn self_owned_routes_exempt() {
        assert_eq!(pol(Method::GET, "/api/v1/auth/me"), Exempt);
        assert_eq!(pol(Method::POST, "/api/v1/auth/logout"), Exempt);
        assert_eq!(pol(Method::GET, "/api/v1/auth/tokens"), Exempt);
        assert_eq!(pol(Method::POST, "/api/v1/auth/tokens"), Exempt);
        assert_eq!(pol(Method::DELETE, "/api/v1/auth/tokens/{id}"), Exempt);
        // Effective-capabilities map: self-scoped, any authed user.
        assert_eq!(pol(Method::GET, "/api/v1/auth/capabilities"), Exempt);
        assert_eq!(pol(Method::GET, "/api/v1/notifications"), Exempt);
        assert_eq!(pol(Method::POST, "/api/v1/notifications/{id}/read"), Exempt);
        assert_eq!(pol(Method::GET, "/api/v1/fs/browse"), Exempt);
        assert_eq!(pol(Method::GET, "/api/v1/fs/read"), Exempt);
        // Per-user email sender (Gmail App Password → Keychain): self-owned.
        assert_eq!(pol(Method::GET, "/api/v1/email-sender"), Exempt);
        assert_eq!(pol(Method::PUT, "/api/v1/email-sender"), Exempt);
    }

    #[test]
    fn workspace_axis_routes_exempt() {
        // Workspace CRUD / members / MCP config are pure workspace-role-axis.
        assert_eq!(pol(Method::GET, "/api/v1/workspaces"), Exempt);
        assert_eq!(pol(Method::POST, "/api/v1/workspaces"), Exempt);
        assert_eq!(pol(Method::PATCH, "/api/v1/workspaces/{id}"), Exempt);
        assert_eq!(pol(Method::PUT, "/api/v1/workspaces/{id}/members"), Exempt);
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/mcp-servers"),
            Exempt
        );
        assert_eq!(pol(Method::DELETE, "/api/v1/mcp-servers/{id}"), Exempt);
    }

    #[test]
    fn static_catalogs_exempt() {
        assert_eq!(pol(Method::GET, "/api/v1/swarm/presets"), Exempt);
        assert_eq!(pol(Method::GET, "/api/v1/workflows/node-types"), Exempt);
        assert_eq!(pol(Method::GET, "/api/v1/lsp/capabilities"), Exempt);
    }

    // ---- Flagged mismatches (encode exactly) ---------------------------------

    #[test]
    fn self_improvement_config_admin_run_edit() {
        assert_eq!(
            pol(Method::PUT, "/api/v1/workspaces/{id}/self-improvement"),
            Require(SelfImprovement, Admin),
            "config write = Admin"
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{id}/self-improvement"),
            Require(SelfImprovement, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/self-improvement/run"),
            Require(SelfImprovement, Edit),
            "run = Edit"
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/improvement/edits/{eid}/approve"),
            Require(SelfImprovement, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{id}/improvement/runs"),
            Require(SelfImprovement, View)
        );
    }

    #[test]
    fn context_put_is_admin() {
        assert_eq!(
            pol(Method::PUT, "/api/v1/workspaces/{id}/context"),
            Require(Context, Admin),
            "context config write = Admin"
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{id}/context"),
            Require(Context, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/context/materialize"),
            Require(Context, Edit)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/context/preview"),
            Require(Context, View)
        );
    }

    #[test]
    fn library_get_view_write_admin() {
        assert_eq!(
            pol(Method::GET, "/api/v1/library/skills"),
            Require(Skills, View)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/library/skills/{name}"),
            Require(Skills, View)
        );
        assert_eq!(
            pol(Method::PUT, "/api/v1/library/skills/{name}"),
            Require(Skills, Admin)
        );
        assert_eq!(
            pol(Method::DELETE, "/api/v1/library/context/{name}"),
            Require(Skills, Admin)
        );
        // Bundled-skill install (bundled→local) = Edit.
        assert_eq!(
            pol(Method::POST, "/api/v1/library/bundled/{name}/install"),
            Require(Skills, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/library/bundled"),
            Require(Skills, View)
        );
    }

    #[test]
    fn skill_eval_promote_is_admin() {
        assert_eq!(
            pol(Method::POST, "/api/v1/skill-evaluations/{id}/promote"),
            Require(SkillEval, Admin),
            "promote = Admin"
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/skill-evaluations"),
            Require(SkillEval, Edit),
            "start eval = Edit"
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/skill-evaluations/{id}/cancel"),
            Require(SkillEval, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/skill-evaluations/{id}"),
            Require(SkillEval, View)
        );
        assert_eq!(
            pol(Method::PUT, "/api/v1/settings/skill-eval"),
            Require(SkillEval, Admin)
        );
    }

    #[test]
    fn usage_insights_settings_admin_or_view() {
        assert_eq!(
            pol(Method::GET, "/api/v1/usage/summary"),
            Require(Usage, View)
        );
        assert_eq!(
            pol(Method::PUT, "/api/v1/usage/config"),
            Require(Usage, Admin)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/usage/install"),
            Require(Usage, Admin)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/insights/reports"),
            Require(Insights, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/insights/run"),
            Require(Insights, Edit)
        );
        assert_eq!(
            pol(Method::PUT, "/api/v1/insights/config"),
            Require(Insights, Admin)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/audit-log"),
            Require(Settings, Admin)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/logs/daemon"),
            Require(Settings, Admin)
        );
    }

    // ---- Per-feature family smoke checks -------------------------------------

    #[test]
    fn agents_family() {
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{id}/sessions"),
            Require(Agents, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/sessions"),
            Require(Agents, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/sessions/{id}"),
            Require(Agents, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/sessions/{id}/restart"),
            Require(Agents, Edit)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/sessions/{id}/input"),
            Require(Agents, Edit)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{id}/broadcast"),
            Require(Agents, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{wid}/sessions/{sid}/trail"),
            Require(Agents, View)
        );
        assert_eq!(
            pol(
                Method::POST,
                "/api/v1/workspaces/{wid}/sessions/{sid}/trail"
            ),
            Require(Agents, Edit)
        );
    }

    #[test]
    fn git_family() {
        assert_eq!(
            pol(Method::GET, "/api/v1/repos/{id}/status"),
            Require(Git, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/repos/{id}/commit"),
            Require(Git, Edit)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/repos/{id}/push"),
            Require(Git, Edit)
        );
        assert_eq!(pol(Method::GET, "/api/v1/git/accounts"), Require(Git, View));
        assert_eq!(
            pol(Method::POST, "/api/v1/git/accounts"),
            Require(Git, Edit)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/pr-review-comments/{cid}/approve"),
            Require(Git, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/settings/pr-review"),
            Require(Git, View)
        );
        // A1: new findings / merge-readiness routes — GET=View, POST=Edit.
        assert_eq!(
            pol(Method::GET, "/api/v1/reviews/{rid}/findings"),
            Require(Git, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/reviews/{rid}/findings/{fp}/state"),
            Require(Git, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/reviews/{rid}/merge-readiness"),
            Require(Git, View)
        );
    }

    #[test]
    fn issues_product_swarm_apiclient_families() {
        assert_eq!(
            pol(Method::GET, "/api/v1/issue/search"),
            Require(Issues, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/issue/{account_id}/{key}/comment"),
            Require(Issues, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/product/stories/{sid}"),
            Require(Product, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{ws}/product/stories"),
            Require(Product, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{id}/swarm/swarms"),
            Require(Swarm, View)
        );
        assert_eq!(
            pol(
                Method::POST,
                "/api/v1/workspaces/{id}/swarm/swarms/{sid}/start"
            ),
            Require(Swarm, Edit)
        );
        assert_eq!(
            pol(
                Method::GET,
                "/api/v1/workspaces/{wid}/api-client/collections"
            ),
            Require(ApiClient, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workspaces/{wid}/api-client/execute"),
            Require(ApiClient, Edit)
        );
    }

    #[test]
    fn workflows_and_channels_families() {
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{wid}/workflows"),
            Require(Workflows, View)
        );
        assert_eq!(
            pol(Method::POST, "/api/v1/workflows/{id}/run"),
            Require(Workflows, Edit)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/workflows/templates"),
            Require(Workflows, View)
        );
        assert_eq!(
            pol(Method::GET, "/api/v1/workspaces/{id}/integrations"),
            Require(Channels, View)
        );
        assert_eq!(
            pol(
                Method::PUT,
                "/api/v1/workspaces/{id}/integrations/{channel}"
            ),
            Require(Channels, Edit)
        );
    }

    #[test]
    fn kill_sessions_is_agents() {
        // Mapped under Agents:Edit here; Task 3.3 additionally root-gates it.
        assert_eq!(
            pol(Method::POST, "/api/v1/app/kill-sessions"),
            Require(Agents, Edit)
        );
    }
}
