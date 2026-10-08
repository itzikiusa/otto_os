//! The central feature-policy guard (RBAC Task 1.4).
//!
//! One Axum middleware, layered **immediately after** [`crate::auth::auth_middleware`]
//! on the protected routes, that enforces the *feature* authorization axis for
//! every request the auth chokepoint admits. It reads the route **template** from
//! the [`axum::extract::MatchedPath`] (e.g. `/api/v1/connections/{id}/db/query`)
//! plus the method, looks the pair up in [`crate::policy::policy_for`], and:
//!
//! - [`PolicyDecision::Exempt`] → passes the request through (public / token /
//!   self-owned / workspace-axis / catalog routes the feature axis doesn't gate);
//! - [`PolicyDecision::Require`]`(feature, cap)` → **allows iff** the caller is
//!   root **or** [`GrantsRepo::capability_of`] for that feature is `>= cap`,
//!   otherwise `403`;
//! - [`PolicyDecision::Deny`] → `403` (fail closed: any protected route with no
//!   policy entry is denied).
//!
//! This is an *additional* axis on top of the unchanged workspace-role /
//! ownership gates that stay in the handlers (`require_ws_role`, owner checks).
//! The guard never weakens those; `effective = min(feature_grant, ws_role)`. Root
//! bypasses the feature axis here (and `capability_of` independently returns
//! `Admin` for root), never depending on grant rows.
//!
//! Denials are rendered through the standard [`ApiError`] → JSON `Problem` path
//! (`{"code":"forbidden","message":...}`), matching every other handler.

use axum::extract::{MatchedPath, Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use otto_core::auth::{AuthContext, AuthUser, SessionScope};
use otto_core::domain::{Capability, WorkspaceRole};
use otto_core::Error;
use otto_state::GrantsRepo;

use crate::error::ApiError;
use crate::policy::{policy_for, route_class, PolicyDecision, RouteClass};
use crate::state::ServerCtx;

/// App state that can hand the guard a [`GrantsRepo`].
///
/// Implemented for the production [`ServerCtx`] (constructs a repo from the
/// shared pool — a cheap handle clone) and, in tests, for a minimal state so the
/// guard can be exercised without assembling the full server context.
pub trait HasGrants {
    fn grants(&self) -> GrantsRepo;
    /// Optional resource-policy storage; minimal legacy test states omit it.
    fn resource_pool(&self) -> Option<otto_state::DbPool> {
        None
    }
}

impl HasGrants for ServerCtx {
    fn resource_pool(&self) -> Option<otto_state::DbPool> {
        Some(self.pool.clone())
    }
    fn grants(&self) -> GrantsRepo {
        // SG-11: the guard runs on every non-root request; answer it from the
        // shared per-user grant cache (flushed by the grants routes through
        // `AuthCache::invalidate_user`; 10 s TTL backstop).
        let repo = GrantsRepo::new(self.pool.clone());
        match self.auth_cache.grant_cache() {
            Some(cache) => repo.with_cache(cache),
            None => repo,
        }
    }
}

/// The feature-policy guard middleware.
///
/// Generic over the app state so the same code runs in production (`ServerCtx`)
/// and in the RBAC matrix tests (a minimal `HasGrants` state). Layer it with
/// `from_fn_with_state(state, feature_guard::<S>)` as a `route_layer` directly
/// after the auth middleware, so the [`AuthUser`] extension is present and the
/// [`MatchedPath`] is set for the matched route.
pub async fn feature_guard<S>(state: State<S>, req: Request, next: Next) -> Response
where
    S: HasGrants + Clone + Send + Sync + 'static,
{
    // REQUEST CREDENTIAL (S11 "flip the default", S6-304). Publish what this
    // request's credential is to every handler: a non-human credential on a
    // write outside the reviewed agent allow-list has its ROOT AUTHORITY
    // WITHHELD, so each `require_root` / `require_setup_authority` /
    // `root_authority` write gate refuses it — a new root-gated route is
    // closed to agents without anyone remembering to tag it.
    let cred = request_credential_for(&req);
    otto_core::auth::with_request_credential(cred, guard_inner(state, req, next)).await
}

/// The [`otto_core::auth::RequestCredential`] for `req` (its `AuthContext`,
/// method and matched template). No `AuthContext` ⇒ not a positively
/// identified person ([`otto_core::auth::OUTSIDE_REQUEST`]): consent signals
/// fail closed.
pub fn request_credential_for(req: &Request) -> otto_core::auth::RequestCredential {
    let Some(ctx) = req.extensions().get::<AuthContext>() else {
        return otto_core::auth::OUTSIDE_REQUEST;
    };
    let agent = !crate::ui_bridge::is_human(ctx);
    let method = req.method();
    let write = !(method == axum::http::Method::GET
        || method == axum::http::Method::HEAD
        || method == axum::http::Method::OPTIONS);
    let template = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str())
        .unwrap_or("");
    otto_core::auth::RequestCredential {
        agent,
        root_withheld: agent && write && !crate::policy::agent_root_write_allowed(method, template),
    }
}

async fn guard_inner<S>(State(state): State<S>, req: Request, next: Next) -> Response
where
    S: HasGrants + Clone + Send + Sync + 'static,
{
    // The matched route template, including the `/api/v1` nest prefix the policy
    // table keys on. A request with no `MatchedPath` never matched a route in
    // this router (it would 404 below) — but if one somehow reaches the guard, we
    // fail closed.
    let Some(matched) = req.extensions().get::<MatchedPath>() else {
        return forbidden("no matched route").into_response();
    };
    let template = matched.as_str().to_string();
    let method = req.method().clone();

    // SCOPE BRANCH (mobile plan Task 1.5). A *scoped* (guest / share-link) token
    // carries an [`AuthContext::scope`]; for it the feature policy is **skipped
    // entirely** and a strict deny-by-default scope policy applies instead: the
    // allow-list is exactly two routes (GET the one session, POST input to it iff
    // Editor) and *everything else* is `403`. An unscoped token (the normal/api/
    // impersonation case) falls through to the unchanged feature-policy path below.
    //
    // We read the full [`AuthContext`] from extensions (not just [`AuthUser`]) to
    // see the scope. Its absence here would mean the guard ran outside the auth
    // chokepoint; for a scoped decision that is unreachable in production, but if
    // an `AuthContext` is present and scoped we enforce the scope and never look
    // at grants. If no `AuthContext` is present we behave exactly as before.
    if let Some(ctx) = req.extensions().get::<AuthContext>() {
        // MCP BRANCH (design §14 F1). A `kind='mcp'` restricted token (the outward
        // "Otto as MCP server") may reach ONLY the governed invoke choke point and
        // the outward-server status read — every other route is denied, so a token
        // leaked from an external agent's `.mcp.json` cannot bypass the control
        // plane by calling feature endpoints directly. Deny-by-default.
        if ctx.mcp_only {
            // The Streamable-HTTP transport (`POST /mcp/http`, plus its `GET`
            // 405-probe) is the modern way an external client reaches the otto.*
            // tools over HTTP; the legacy invoke choke point + status read stay
            // permitted for the stdio bridge. EVERYTHING else is denied so a
            // leaked `kind='mcp'` token can never reach a feature endpoint
            // directly (design §14 F1).
            let allowed = mcp_route_allowed(&method, &template);
            return if allowed {
                next.run(req).await
            } else {
                forbidden(
                    "mcp-restricted token: only /mcp/http and /mcp/otto-tools/invoke are permitted",
                )
                .into_response()
            };
        }
        // CREDENTIAL CLASS (S11-01/03, S8-01, S1-02, S3-01). An agent session's
        // own token authorizes AS ITS OWNER (often root), so `require_root` and
        // the feature grant both pass it. Admin and Secret routes — identity /
        // policy / daemon administration, human approval gates, plaintext
        // credentials and credential minting — need a person's own credential.
        if let Some(class) = route_class(&method, &template) {
            if let Err(msg) = credential_class_gate(class, crate::ui_bridge::is_human(ctx)) {
                return forbidden(msg).into_response();
            }
        }
        // READ-ONLY AGENT SESSIONS (crate::personal_agent_policy). A session
        // confined read-only (a proactive personal-agent run, or a run of a
        // read-only schedule) may READ through its own token but never write:
        // every non-GET request is refused unless it is an allow-listed read
        // route (searches, introspection, the governed invoke — which applies
        // the same policy per tool). This is what makes the native stdio tools
        // (room posts, canvas/swarm writes, PR comments…) read-only too, not
        // just the governed catalog. An unreadable row (a DB error, or a row
        // already gone — its tokens are revoked with it) fails CLOSED, like the
        // root-route gate: a transient SQLITE_BUSY must never let a read-only
        // agent's write through (S8-08).
        if let Some(sid) = ctx
            .managed_session_id
            .clone()
            .or(ctx.mcp_session_id.clone())
        {
            if !crate::personal_agent_policy::read_only_route_allowed(&method, &template) {
                if let Some(pool) = state.resource_pool() {
                    let read_only = crate::personal_agent_policy::session_read_only(&pool, &sid)
                        .await
                        .unwrap_or(true);
                    if read_only {
                        return forbidden(
                            "this agent session is read-only: it may read but not change anything",
                        )
                        .into_response();
                    }
                }
            }
        }
        // HOST FILES (`/fs/*`). The daemon reads the host filesystem as the
        // owner, unconfined — so an agent session's own credential reaching
        // `/fs/read` would read exactly the files its Seatbelt profile hides
        // (`Otto/secrets.json`, WebKit storage, …). Agent credentials never
        // reach the Files surface; the owner's UI token still does.
        if agent_session_of(ctx).is_some() && is_host_files_route(&template) {
            return forbidden("an agent session credential cannot read host files").into_response();
        }
        if let Some(scope) = ctx.scope.clone() {
            // EMAIL-OTP GATE (mobile plan Task 7.3). A share locked to a recipient
            // email is OTP-pending until the guest redeems the emailed code via
            // `POST /api/v1/share/verify` (a public, Exempt route that never reaches
            // this guard). While pending, the scope reaches **nothing** here —
            // fail closed, deny every protected route (even GET the session). This
            // is what makes a leaked link alone useless without the mailbox code.
            if scope.otp_pending {
                return forbidden("share requires email-OTP verification").into_response();
            }
            // Concrete request path (with the real session-id segment), matched
            // against the `{id}`-templated route to extract & compare the id.
            let concrete = req.uri().path().to_string();
            return if scope_allows(&method, &template, &concrete, &scope) {
                next.run(req).await
            } else {
                forbidden("share token is scoped to a single session").into_response()
            };
        }
    }

    // PLUGIN BRANCH (runtime custom plugins). Requests to `/api/v1/plugins/<slug>/…`
    // (reverse-proxied to the sidecar) are gated by a string-keyed permission axis
    // keyed on the plugin **slug**. `GET` ⇒ `View`, any other method ⇒ `Edit`; root
    // bypasses. This sits AFTER the scope branch (a share token never reaches a
    // plugin) and BEFORE `policy_for` (whose default arm would `Deny` these). The
    // proxy route template is `/api/v1/plugins/{slug}/{*rest}`, so the literal slug
    // is read from the CONCRETE request path (which, inside the nest, may or may not
    // still carry the `/api/v1` prefix — handle both).
    if template.starts_with("/api/v1/plugins/") {
        let Some(AuthUser(user)) = req.extensions().get::<AuthUser>().cloned() else {
            return ApiError(Error::Unauthorized).into_response();
        };
        if user.is_root {
            return next.run(req).await;
        }
        let path = req.uri().path().to_string();
        let slug = path
            .strip_prefix("/api/v1/plugins/")
            .or_else(|| path.strip_prefix("/plugins/"))
            .and_then(|rest| rest.split('/').next())
            .unwrap_or("");
        if slug.is_empty() {
            return forbidden("malformed plugin route").into_response();
        }
        let needed = if method == Method::GET {
            Capability::View
        } else {
            Capability::Edit
        };
        return match state.grants().capability_of_plugin(&user, slug).await {
            Ok(have) if have >= needed => next.run(req).await,
            Ok(_) => {
                forbidden(&format!("requires plugin {slug}:{}", needed.as_str())).into_response()
            }
            Err(e) => ApiError(e).into_response(),
        };
    }

    match policy_for(&method, &template) {
        PolicyDecision::Exempt => next.run(req).await,
        PolicyDecision::Deny => forbidden("route not permitted").into_response(),
        PolicyDecision::Require(feature, needed) => {
            // The authenticated user, inserted by `auth_middleware`. Absent ⇒ the
            // guard was mounted outside the auth chokepoint; fail closed (401).
            let Some(AuthUser(user)) = req.extensions().get::<AuthUser>().cloned() else {
                return ApiError(Error::Unauthorized).into_response();
            };
            // Root bypasses the feature axis unconditionally (never depends on a
            // grant row). For everyone else, the granted capability must meet or
            // exceed the requirement.
            if user.is_root {
                return next.run(req).await;
            }
            let (feature, needed) = if let Some(pool) = state.resource_pool() {
                match bound_session_feature(&pool, &template, req.uri().path()).await {
                    Ok(Some(feature)) => (feature, Capability::View),
                    Ok(None) => (feature, needed),
                    Err(e) => return ApiError(e).into_response(),
                }
            } else {
                (feature, needed)
            };
            // An enforced resource uses the feature only as its page entry
            // gate. Its handler/service must authorize the exact operation.
            // Legacy resources keep their existing coarse tier unchanged.
            let resource = if let Some(pool) = state.resource_pool() {
                match indirect_resource_route(&pool, &template, req.uri().path()).await {
                    Ok(value) => value,
                    Err(e) => return ApiError(e).into_response(),
                }
            } else {
                resource_route(&template, req.uri().path())
            };
            let needed = match (state.resource_pool(), resource) {
                (Some(pool), Some((kind, id))) => {
                    match otto_state::resource_access::ResourceAccessRepo::new(pool)
                        .get_policy(kind, &id)
                        .await
                    {
                        Ok(policy) if policy.mode == otto_core::access::AccessMode::Enforced => {
                            Capability::View
                        }
                        Ok(_) => needed,
                        Err(e) => return ApiError(e).into_response(),
                    }
                }
                _ => needed,
            };
            match state.grants().capability_of(&user, feature).await {
                Ok(have) if have >= needed => next.run(req).await,
                Ok(_) => forbidden(&format!(
                    "requires {}:{}",
                    feature.as_str(),
                    needed.as_str()
                ))
                .into_response(),
                // A repo error here is an authorization failure → fail closed.
                Err(e) => ApiError(e).into_response(),
            }
        }
    }
}

/// Pure credential-class decision: `human` is [`crate::ui_bridge::is_human`]
/// for the caller. Admin and Secret routes refuse every non-human credential
/// (agent session, internal/external MCP, share link). Outward routes are
/// tagged but still allowed — the user has not decided yet whether an
/// agent's own token may merge/push/post directly (the `otto-pr` skill relies
/// on it). Enforcing them is adding `RouteClass::Outward` to the match below.
pub fn credential_class_gate(class: RouteClass, human: bool) -> Result<(), &'static str> {
    if human {
        return Ok(());
    }
    match class {
        RouteClass::Admin => Err(
            "an agent session's credential cannot administer Otto or decide a human approval — \
             a person signed in to Otto must do this",
        ),
        RouteClass::Secret => Err(
            "an agent session's credential cannot reveal or mint credentials — \
             a person signed in to Otto must do this",
        ),
        RouteClass::Outward => Ok(()),
    }
}

/// The agent session an Otto-minted credential is bound to: an author
/// session's API token (`managed_session_id`) or a session's internal MCP
/// credential (`mcp_session_id`). `None` for a person's own login/PAT token.
pub fn agent_session_of(ctx: &AuthContext) -> Option<&otto_core::Id> {
    ctx.managed_session_id
        .as_ref()
        .or(ctx.mcp_session_id.as_ref())
}

/// The daemon-side host filesystem routes (`/api/v1/fs/*`).
fn is_host_files_route(template: &str) -> bool {
    template.starts_with("/api/v1/fs/")
}

// ---------------------------------------------------------------------------
// Root-mounted routes (outside `/api/v1`).
// ---------------------------------------------------------------------------

/// Every root-mounted route that authenticates an Otto bearer token itself
/// (WebSockets and the browser proxy are not nested under `/api/v1`, so
/// [`feature_guard`] never sees them). Each one calls [`root_route_gate`]
/// before doing any work; `/ws/term` applies the same rules in
/// `otto_sessions::ws` (`agent_attach_rule`), and the room sockets use room
/// tokens (agent credentials are refused when a room is joined). The
/// `root_routes_apply_agent_rules` test keeps this list complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootRoute {
    /// `/ws/events` — the event stream (receive-only for agents).
    Events,
    /// `/ws/lsp` — a language server over a host directory (host file reads).
    Lsp,
    /// `/ws/api-client/stream` — sends real HTTP requests.
    ApiStream,
    /// `/ws/browser/{tab_id}/live` — drives a live browser tab.
    BrowserLive,
    /// `/browser/proxy` — fetches a page for the reader view (a read).
    BrowserProxy,
}

/// The credential facts the root-route rules depend on.
#[derive(Debug, Clone, Copy, Default)]
pub struct RootCred {
    /// A share-link (session-scoped) token.
    pub scoped: bool,
    /// A `kind='mcp'` restricted token (external or a session's internal one).
    pub mcp_only: bool,
    /// An Otto-minted agent-session credential.
    pub agent_session: bool,
    /// The agent session is confined read-only.
    pub read_only: bool,
}

/// Pure decision for a root-mounted route — the same three rules the
/// `/api/v1` guard applies: share and MCP-restricted tokens reach none of
/// these; an agent session never reaches host files (LSP); a read-only
/// session never reaches a route that acts (send HTTP, drive a browser).
pub fn root_route_decision(route: RootRoute, cred: RootCred) -> Result<(), &'static str> {
    if cred.scoped {
        return Err("a share-link token cannot use this endpoint");
    }
    if cred.mcp_only {
        return Err("an mcp-restricted token cannot use this endpoint");
    }
    if cred.agent_session && route == RootRoute::Lsp {
        return Err("an agent session credential cannot open a language server");
    }
    if cred.agent_session
        && cred.read_only
        && matches!(route, RootRoute::ApiStream | RootRoute::BrowserLive)
    {
        return Err("this agent session is read-only: it may read but not change anything");
    }
    Ok(())
}

/// [`root_route_decision`] for an authenticated context. `pool` resolves the
/// calling session's read-only flag (an unreadable row fails closed); pass
/// `None` only for a route no read-only rule covers.
pub async fn root_route_gate(
    route: RootRoute,
    auth: &AuthContext,
    pool: Option<&otto_state::DbPool>,
) -> Result<(), Error> {
    let agent = agent_session_of(auth);
    let read_only = match (agent, pool) {
        (Some(sid), Some(pool)) => crate::personal_agent_policy::session_read_only(pool, sid)
            .await
            .unwrap_or(true),
        // No pool: only the rules that do not need the row apply.
        _ => false,
    };
    let cred = RootCred {
        scoped: auth.is_scoped(),
        mcp_only: auth.mcp_only,
        agent_session: agent.is_some(),
        read_only,
    };
    root_route_decision(route, cred).map_err(|m| Error::Forbidden(m.into()))
}

/// Exact HTTP surface reachable by an MCP-restricted credential. Internal
/// reviewer credentials use the same deny-by-default boundary as outward MCP
/// tokens, so reusing one as a bearer token can never reach Vault writes.
fn mcp_route_allowed(method: &Method, template: &str) -> bool {
    (method == Method::POST && template == "/api/v1/mcp/otto-tools/invoke")
        || (method == Method::GET && template == "/api/v1/mcp/otto-server")
        // The light enabled-names read: a strict subset of the status above.
        || (method == Method::GET && template == "/api/v1/mcp/otto-server/enabled")
        // The stdio bridge's own tool-call audit append (R7).
        || (method == Method::POST && template == "/api/v1/mcp/tool-calls")
        || (template == "/api/v1/mcp/http" && (method == Method::POST || method == Method::GET))
}

/// Build a `403 Forbidden` `ApiError` (rendered as the JSON `Problem` body).
fn forbidden(reason: &str) -> ApiError {
    ApiError(Error::Forbidden(reason.to_string()))
}

/// The deny-by-default scope policy for a **scoped** (share-link) token.
///
/// Returns `true` iff `(method, template)` is on the *exact* two-route allow-list
/// AND the concrete request targets the token's pinned session:
///
/// - `GET  /api/v1/sessions/{id}`        — allowed iff `{id} == scope.session_id`.
/// - `POST /api/v1/sessions/{id}/input`  — allowed iff `{id} == scope.session_id`
///   **and** `scope.role == Editor` (a viewer share can never type).
///
/// **Everything else is denied** (returns `false`): session enumeration
/// (`GET /workspaces/{}/sessions`), any other session id, restart / archive /
/// delete / patch, and every non-session surface (connections, db, git, usage,
/// settings, users, admin, impersonate, …). This is an allow-list, not a
/// block-list, so a route added later is denied for scoped tokens until it is
/// explicitly allow-listed here. We also **fail closed** when the concrete
/// session-id segment cannot be extracted (a malformed/unexpected path).
///
/// `template` is the Axum [`MatchedPath`] (`/api/v1/sessions/{id}` …) and
/// `concrete` is the real request path (`/api/v1/sessions/<the-real-id>` …); the
/// id is extracted by aligning the two segment-for-segment.
fn scope_allows(method: &Method, template: &str, concrete: &str, scope: &SessionScope) -> bool {
    // GET /api/v1/sessions/{id} — read the one pinned session.
    if method == Method::GET && template == "/api/v1/sessions/{id}" {
        return match path_segment(template, concrete, "{id}") {
            Some(id) => id == scope.session_id.as_str(),
            None => false, // unparseable id ⇒ fail closed
        };
    }
    // POST /api/v1/sessions/{id}/input — drive the one pinned session, Editor only.
    if method == Method::POST && template == "/api/v1/sessions/{id}/input" {
        if scope.role != WorkspaceRole::Editor {
            return false; // a viewer share can never send input
        }
        return match path_segment(template, concrete, "{id}") {
            Some(id) => id == scope.session_id.as_str(),
            None => false,
        };
    }
    // GET /api/v1/share/whoami — the token's own session id + role (no id in
    // the path; it only ever describes this scope).
    if method == Method::GET && template == "/api/v1/share/whoami" {
        return true;
    }
    // Deny-by-default: anything not on the allow-list above.
    false
}

/// Extract the concrete value of a `{...}` placeholder from a request path by
/// aligning the templated `template` with the real `concrete` path **from the
/// right** (trailing-segment alignment). Returns the concrete segment lined up
/// with `placeholder`, or `None` if the placeholder isn't found or the concrete
/// path is too short to reach it (callers treat `None` as fail-closed).
///
/// Right-alignment is deliberate: the [`MatchedPath`] template carries the full
/// nest prefix (`/api/v1/sessions/{id}`) while the request URI seen inside a
/// nested router is the *prefix-stripped* path (`/sessions/S1`). The two share an
/// identical **suffix** — the part that contains the placeholder — so aligning
/// the last segments is correct regardless of how much prefix the nest strips.
///
/// e.g. `path_segment("/api/v1/sessions/{id}", "/sessions/S1", "{id}")`
/// → `Some("S1")`; also `("/api/v1/sessions/{id}", "/api/v1/sessions/S1", "{id}")`
/// → `Some("S1")`.
fn path_segment<'a>(template: &str, concrete: &'a str, placeholder: &str) -> Option<&'a str> {
    let t: Vec<&str> = template.split('/').collect();
    let c: Vec<&str> = concrete.split('/').collect();
    // Position of the placeholder counted from the END of the template.
    let from_end = t.iter().rev().position(|seg| *seg == placeholder)?;
    // The concrete path must be long enough to have a segment that far from its
    // own end; otherwise it does not correspond to this template — fail closed.
    if from_end >= c.len() {
        return None;
    }
    Some(c[c.len() - 1 - from_end])
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use otto_core::Id;

    fn scope(session: &str, role: WorkspaceRole) -> SessionScope {
        SessionScope {
            session_id: Id::from(session),
            role,
            otp_pending: false,
        }
    }

    #[test]
    fn internal_reviewer_mcp_token_cannot_reach_direct_vault_mutations() {
        assert!(!mcp_route_allowed(
            &Method::PUT,
            "/api/v1/workspaces/{ws}/vault/vaults/{id}/file",
        ));
        assert!(!mcp_route_allowed(
            &Method::PUT,
            "/api/v1/workspaces/{ws}/vault/vaults/{id}/note",
        ));
        assert!(mcp_route_allowed(
            &Method::POST,
            "/api/v1/mcp/otto-tools/invoke",
        ));
    }

    #[test]
    fn mcp_token_reaches_the_audit_append_but_only_by_post() {
        assert!(mcp_route_allowed(&Method::POST, "/api/v1/mcp/tool-calls"));
        assert!(!mcp_route_allowed(&Method::GET, "/api/v1/mcp/tool-calls"));
    }

    #[test]
    fn mcp_token_reaches_the_light_enabled_read_but_only_by_get() {
        assert!(mcp_route_allowed(
            &Method::GET,
            "/api/v1/mcp/otto-server/enabled",
        ));
        assert!(!mcp_route_allowed(
            &Method::POST,
            "/api/v1/mcp/otto-server/enabled",
        ));
        assert!(!mcp_route_allowed(
            &Method::PATCH,
            "/api/v1/mcp/otto-server",
        ));
    }

    #[test]
    fn extracts_concrete_session_id() {
        // Right-aligned: works whether or not the concrete path carries the
        // `/api/v1` nest prefix the template always has.
        assert_eq!(
            path_segment("/api/v1/sessions/{id}", "/api/v1/sessions/S1", "{id}"),
            Some("S1")
        );
        assert_eq!(
            path_segment("/api/v1/sessions/{id}", "/sessions/S1", "{id}"),
            Some("S1"),
            "nest-stripped concrete path still aligns by suffix"
        );
        assert_eq!(
            path_segment(
                "/api/v1/sessions/{id}/input",
                "/sessions/abc-123/input",
                "{id}"
            ),
            Some("abc-123")
        );
        // Placeholder absent ⇒ None.
        assert_eq!(
            path_segment("/api/v1/sessions/all", "/sessions/all", "{id}"),
            None
        );
        // Concrete too short to reach the placeholder ⇒ None (fail closed).
        assert_eq!(path_segment("/a/b/{id}/x", "x", "{id}"), None);
    }

    #[test]
    fn viewer_share_may_get_its_session_only() {
        let s = scope("S1", WorkspaceRole::Viewer);
        // GET its own session → allow.
        assert!(scope_allows(
            &Method::GET,
            "/api/v1/sessions/{id}",
            "/api/v1/sessions/S1",
            &s
        ));
        // GET another session → deny.
        assert!(!scope_allows(
            &Method::GET,
            "/api/v1/sessions/{id}",
            "/api/v1/sessions/S2",
            &s
        ));
        // POST input as a viewer → deny (role cap).
        assert!(!scope_allows(
            &Method::POST,
            "/api/v1/sessions/{id}/input",
            "/api/v1/sessions/S1/input",
            &s
        ));
    }

    #[test]
    fn editor_share_may_input_its_session_only() {
        let s = scope("S1", WorkspaceRole::Editor);
        assert!(scope_allows(
            &Method::POST,
            "/api/v1/sessions/{id}/input",
            "/api/v1/sessions/S1/input",
            &s
        ));
        // Input to a different session → deny.
        assert!(!scope_allows(
            &Method::POST,
            "/api/v1/sessions/{id}/input",
            "/api/v1/sessions/S2/input",
            &s
        ));
        // GET its own session is still allowed for an editor share.
        assert!(scope_allows(
            &Method::GET,
            "/api/v1/sessions/{id}",
            "/api/v1/sessions/S1",
            &s
        ));
    }

    #[test]
    fn any_share_may_ask_whoami_but_not_post_to_it() {
        for role in [WorkspaceRole::Viewer, WorkspaceRole::Editor] {
            let s = scope("S1", role);
            assert!(scope_allows(
                &Method::GET,
                "/api/v1/share/whoami",
                "/api/v1/share/whoami",
                &s
            ));
            assert!(!scope_allows(
                &Method::POST,
                "/api/v1/share/whoami",
                "/api/v1/share/whoami",
                &s
            ));
        }
    }

    #[test]
    fn everything_else_is_denied() {
        for role in [WorkspaceRole::Viewer, WorkspaceRole::Editor] {
            let s = scope("S1", role);
            // Enumeration.
            assert!(!scope_allows(
                &Method::GET,
                "/api/v1/workspaces/{id}/sessions",
                "/api/v1/workspaces/W1/sessions",
                &s
            ));
            // Session-control writes on its OWN session.
            assert!(!scope_allows(
                &Method::POST,
                "/api/v1/sessions/{id}/restart",
                "/api/v1/sessions/S1/restart",
                &s
            ));
            assert!(!scope_allows(
                &Method::DELETE,
                "/api/v1/sessions/{id}",
                "/api/v1/sessions/S1",
                &s
            ));
            assert!(!scope_allows(
                &Method::PATCH,
                "/api/v1/sessions/{id}",
                "/api/v1/sessions/S1",
                &s
            ));
            // Non-session surfaces.
            for (m, tmpl, uri) in [
                (Method::GET, "/api/v1/connections", "/api/v1/connections"),
                (
                    Method::GET,
                    "/api/v1/usage/summary",
                    "/api/v1/usage/summary",
                ),
                (Method::GET, "/api/v1/users", "/api/v1/users"),
                (Method::PUT, "/api/v1/settings", "/api/v1/settings"),
            ] {
                assert!(
                    !scope_allows(&m, tmpl, uri, &s),
                    "scoped token must be denied {m} {tmpl}"
                );
            }
        }
    }
}

async fn bound_session_feature(
    pool: &otto_state::DbPool,
    template: &str,
    path: &str,
) -> otto_core::Result<Option<otto_core::domain::Feature>> {
    use otto_core::access::{AccessMode, ResourceKind};
    use otto_core::domain::Feature;
    let t = template.strip_prefix("/api/v1").unwrap_or(template);
    if !matches!(
        t,
        "/sessions/{id}"
            | "/sessions/{id}/input"
            | "/sessions/{id}/restart"
            | "/sessions/{id}/archive"
            | "/sessions/{id}/unarchive"
            | "/sessions/{id}/kill"
    ) {
        return Ok(None);
    }
    let p = path.strip_prefix("/api/v1").unwrap_or(path);
    let id = p.split('/').nth(2).unwrap_or_default().to_string();
    let session = otto_state::SessionsRepo::new(pool.clone()).get(&id).await?;
    let Some((resource, op)) = crate::resource_sessions::binding(&session) else {
        return Ok(None);
    };
    let policy = otto_state::ResourceAccessRepo::new(pool.clone())
        .get_live_policy(resource.kind, &resource.id)
        .await?;
    if policy.mode != AccessMode::Enforced {
        return Ok(None);
    }
    Ok(Some(match resource.kind {
        ResourceKind::K8sCluster => Feature::Kubernetes,
        ResourceKind::AwsAccount => Feature::Aws,
        ResourceKind::Connection if op == "db_query" => Feature::Database,
        ResourceKind::Connection => Feature::Connections,
        ResourceKind::McpServer => Feature::Mcp,
    }))
}

async fn indirect_resource_route(
    pool: &otto_state::DbPool,
    template: &str,
    path: &str,
) -> otto_core::Result<Option<(otto_core::access::ResourceKind, String)>> {
    if let Some(resource) = resource_route(template, path) {
        return Ok(Some(resource));
    }
    let t = template.strip_prefix("/api/v1").unwrap_or(template);
    let p = path.strip_prefix("/api/v1").unwrap_or(path);
    let id = p.split('/').nth(3).unwrap_or_default().to_string();
    let server = match t {
        "/mcp/tools/{tool_id}" => Some(
            otto_state::McpToolsRepo::new(pool.clone())
                .get(&id)
                .await?
                .server_id,
        ),
        "/mcp/approvals/{id}/decide" => {
            otto_state::McpApprovalRepo::new(pool.clone())
                .get(&id)
                .await?
                .server_id
        }
        _ => None,
    };
    Ok(server.map(|id| (otto_core::access::ResourceKind::McpServer, id)))
}

/// Bind only explicitly resource-keyed templates. Collection/create routes must
/// never acquire a lower feature tier by looking like an ID in a raw URL.
fn resource_route(template: &str, path: &str) -> Option<(otto_core::access::ResourceKind, String)> {
    use otto_core::access::ResourceKind;
    let template = template.strip_prefix("/api/v1").unwrap_or(template);
    let path = path.strip_prefix("/api/v1").unwrap_or(path);
    for (prefix, kind) in [
        ("/connections/", ResourceKind::Connection),
        ("/mcp/servers/", ResourceKind::McpServer),
        ("/mcp-servers/", ResourceKind::McpServer),
        ("/aws/accounts/", ResourceKind::AwsAccount),
        ("/k8s/clusters/", ResourceKind::K8sCluster),
    ] {
        if template
            .strip_prefix(prefix)
            .is_some_and(|tail| tail == "{id}" || tail.starts_with("{id}/"))
        {
            let id = path.strip_prefix(prefix)?.split('/').next()?;
            if !id.is_empty() {
                return Some((kind, id.to_string()));
            }
        }
    }
    None
}

#[cfg(test)]
mod resource_route_tests {
    use super::*;
    #[test]
    fn creates_and_imports_do_not_lower_feature_tiers() {
        assert!(resource_route("/api/v1/aws/accounts", "/api/v1/aws/accounts").is_none());
        assert!(
            resource_route("/api/v1/k8s/clusters/import", "/api/v1/k8s/clusters/import").is_none()
        );
        assert!(resource_route(
            "/api/v1/connections/unsaved/db/test",
            "/api/v1/connections/unsaved/db/test"
        )
        .is_none());
        assert_eq!(
            resource_route(
                "/api/v1/k8s/clusters/{id}/exec",
                "/api/v1/k8s/clusters/cluster-y/exec"
            ),
            Some((
                otto_core::access::ResourceKind::K8sCluster,
                "cluster-y".into()
            ))
        );
    }
}

#[cfg(test)]
mod root_route_tests {
    use super::*;

    const ALL: [RootRoute; 5] = [
        RootRoute::Events,
        RootRoute::Lsp,
        RootRoute::ApiStream,
        RootRoute::BrowserLive,
        RootRoute::BrowserProxy,
    ];

    #[test]
    fn share_and_mcp_tokens_reach_no_root_route() {
        for route in ALL {
            for cred in [
                RootCred {
                    scoped: true,
                    ..Default::default()
                },
                RootCred {
                    mcp_only: true,
                    ..Default::default()
                },
                RootCred {
                    mcp_only: true,
                    agent_session: true,
                    ..Default::default()
                },
            ] {
                assert!(
                    root_route_decision(route, cred).is_err(),
                    "{route:?} must refuse {cred:?}"
                );
            }
        }
    }

    #[test]
    fn agent_sessions_never_reach_host_files_and_read_only_never_acts() {
        let agent = RootCred {
            agent_session: true,
            ..Default::default()
        };
        let read_only = RootCred {
            read_only: true,
            ..agent
        };
        assert!(root_route_decision(RootRoute::Lsp, agent).is_err());
        for route in [RootRoute::ApiStream, RootRoute::BrowserLive] {
            assert!(root_route_decision(route, agent).is_ok(), "{route:?}");
            assert!(root_route_decision(route, read_only).is_err(), "{route:?}");
        }
        // Reads stay open to a read-only session.
        for route in [RootRoute::Events, RootRoute::BrowserProxy] {
            assert!(root_route_decision(route, read_only).is_ok(), "{route:?}");
        }
        // A person's own token is unaffected everywhere.
        for route in ALL {
            assert!(root_route_decision(route, RootCred::default()).is_ok());
        }
    }

    #[test]
    fn host_files_routes_are_recognised() {
        assert!(is_host_files_route("/api/v1/fs/read"));
        assert!(is_host_files_route("/api/v1/fs/browse"));
        assert!(is_host_files_route("/api/v1/fs/stat"));
        assert!(!is_host_files_route("/api/v1/fsx"));
        assert!(!is_host_files_route("/api/v1/workspaces/{id}/files"));
    }

    /// Route coverage: every root-mounted WebSocket / proxy route in the
    /// workspace (they bypass [`feature_guard`]) must be listed here with the
    /// marker that proves its handler applies the agent-credential rules. A
    /// new root route fails this test until it is gated and listed.
    #[test]
    #[allow(clippy::disallowed_methods)] // sync source scan in a test
    fn root_routes_apply_agent_rules() {
        // (route path, file that handles it, marker that file must contain)
        const COVERED: &[(&str, &str, &str)] = &[
            (
                "/ws/events",
                "otto-server/src/ws_events.rs",
                "RootRoute::Events",
            ),
            ("/ws/lsp", "otto-server/src/lsp/mod.rs", "RootRoute::Lsp"),
            (
                "/ws/api-client/stream",
                "otto-server/src/routes/api_stream.rs",
                "RootRoute::ApiStream",
            ),
            (
                "/ws/browser/{tab_id}/live",
                "otto-server/src/routes/browser_live.rs",
                "RootRoute::BrowserLive",
            ),
            (
                "/browser/proxy",
                "otto-server/src/modules.rs",
                "RootRoute::BrowserProxy",
            ),
            (
                "/ws/term/{session_id}",
                "otto-sessions/src/ws.rs",
                "agent_attach_rule(&auth",
            ),
            (
                "/ws/game-rooms/{id}",
                "otto-server/src/game_rooms/socket.rs",
                "ctx.game_rooms.authenticate(",
            ),
            // Room sockets authenticate a ROOM token, never an Otto bearer
            // token; agent credentials are refused when a room is joined.
            (
                "/ws/rooms/{id}",
                "otto-server/src/rooms/socket.rs",
                "ctx.rooms.authenticate(",
            ),
            (
                "/ws/rooms/{id}/terminal",
                "otto-server/src/rooms/terminal.rs",
                "ctx.rooms.authenticate(",
            ),
        ];
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut found = std::collections::BTreeSet::new();
        let mut stack = vec![crates.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    if p.file_name().is_some_and(|n| n != "target" && n != "tests") {
                        stack.push(p);
                    }
                    continue;
                }
                if p.extension().is_none_or(|e| e != "rs")
                    || p.ends_with("otto-server/src/feature_guard.rs")
                {
                    continue;
                }
                let src = std::fs::read_to_string(&p).unwrap_or_default();
                for (i, _) in src.match_indices(".route(\"") {
                    let rest = &src[i + ".route(\"".len()..];
                    let path = &rest[..rest.find('"').unwrap_or(0)];
                    if path.starts_with("/ws/") || path == "/browser/proxy" {
                        found.insert(path.to_string());
                    }
                }
            }
        }
        assert!(
            found.contains("/ws/events"),
            "scan found nothing: {found:?}"
        );
        for path in &found {
            let Some((_, file, marker)) = COVERED.iter().find(|(p, ..)| p == path) else {
                panic!("root route {path} is not covered by the agent-credential rules");
            };
            let src = std::fs::read_to_string(crates.join(file)).unwrap();
            assert!(
                src.contains(marker),
                "{file} must apply `{marker}` for {path}"
            );
        }
    }
}

#[cfg(test)]
mod credential_class_tests {
    use super::*;

    #[test]
    fn non_human_credentials_are_refused_on_admin_and_secret_only() {
        for class in [RouteClass::Admin, RouteClass::Secret, RouteClass::Outward] {
            assert!(credential_class_gate(class, true).is_ok(), "{class:?}");
        }
        assert!(credential_class_gate(RouteClass::Admin, false).is_err());
        assert!(credential_class_gate(RouteClass::Secret, false).is_err());
        // Outward is a tag only until the user decides (see the gate's doc).
        assert!(credential_class_gate(RouteClass::Outward, false).is_ok());
    }
}
