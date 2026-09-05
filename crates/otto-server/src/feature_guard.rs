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
use crate::policy::{policy_for, PolicyDecision};
use crate::state::ServerCtx;

/// App state that can hand the guard a [`GrantsRepo`].
///
/// Implemented for the production [`ServerCtx`] (constructs a repo from the
/// shared pool — a cheap handle clone) and, in tests, for a minimal state so the
/// guard can be exercised without assembling the full server context.
pub trait HasGrants {
    fn grants(&self) -> GrantsRepo;
    /// Optional resource-policy storage; minimal legacy test states omit it.
    fn resource_pool(&self) -> Option<otto_state::SqlitePool> { None }
}

impl HasGrants for ServerCtx {
    fn resource_pool(&self) -> Option<otto_state::SqlitePool> { Some(self.pool.clone()) }
    fn grants(&self) -> GrantsRepo {
        GrantsRepo::new(self.pool.clone())
    }
}

/// The feature-policy guard middleware.
///
/// Generic over the app state so the same code runs in production (`ServerCtx`)
/// and in the RBAC matrix tests (a minimal `HasGrants` state). Layer it with
/// `from_fn_with_state(state, feature_guard::<S>)` as a `route_layer` directly
/// after the auth middleware, so the [`AuthUser`] extension is present and the
/// [`MatchedPath`] is set for the matched route.
pub async fn feature_guard<S>(State(state): State<S>, req: Request, next: Next) -> Response
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
            let (feature,needed)=if let Some(pool)=state.resource_pool() {
                match bound_session_feature(&pool,&template,req.uri().path()).await {
                    Ok(Some(feature))=>(feature,Capability::View),
                    Ok(None)=>(feature,needed),
                    Err(e)=>return ApiError(e).into_response(),
                }
            } else {(feature,needed)};
            // An enforced resource uses the feature only as its page entry
            // gate. Its handler/service must authorize the exact operation.
            // Legacy resources keep their existing coarse tier unchanged.
            let resource = if let Some(pool) = state.resource_pool() {
                match indirect_resource_route(&pool,&template,req.uri().path()).await {
                    Ok(value) => value,
                    Err(e) => return ApiError(e).into_response(),
                }
            } else { resource_route(&template,req.uri().path()) };
            let needed = match (state.resource_pool(), resource) {
                (Some(pool), Some((kind,id))) => match otto_state::resource_access::ResourceAccessRepo::new(pool).get_policy(kind,&id).await {
                    Ok(policy) if policy.mode == otto_core::access::AccessMode::Enforced => Capability::View,
                    Ok(_) => needed,
                    Err(e) => return ApiError(e).into_response(),
                },
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

/// Exact HTTP surface reachable by an MCP-restricted credential. Internal
/// reviewer credentials use the same deny-by-default boundary as outward MCP
/// tokens, so reusing one as a bearer token can never reach Vault writes.
fn mcp_route_allowed(method: &Method, template: &str) -> bool {
    (method == Method::POST && template == "/api/v1/mcp/otto-tools/invoke")
        || (method == Method::GET && template == "/api/v1/mcp/otto-server")
        || (template == "/api/v1/mcp/http"
            && (method == Method::POST || method == Method::GET))
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
                (Method::GET, "/api/v1/usage/summary", "/api/v1/usage/summary"),
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

async fn bound_session_feature(pool:&otto_state::SqlitePool,template:&str,path:&str)->otto_core::Result<Option<otto_core::domain::Feature>> {
    use otto_core::domain::Feature;
    use otto_core::access::{AccessMode,ResourceKind};
    let t=template.strip_prefix("/api/v1").unwrap_or(template);
    if !matches!(t,"/sessions/{id}"|"/sessions/{id}/input"|"/sessions/{id}/restart"|"/sessions/{id}/archive"|"/sessions/{id}/unarchive"|"/sessions/{id}/kill") {return Ok(None);}
    let p=path.strip_prefix("/api/v1").unwrap_or(path);
    let id=p.split('/').nth(2).unwrap_or_default().to_string();
    let session=otto_state::SessionsRepo::new(pool.clone()).get(&id).await?;
    let Some((resource,op))=crate::resource_sessions::binding(&session) else {return Ok(None);};
    let policy=otto_state::ResourceAccessRepo::new(pool.clone()).get_live_policy(resource.kind,&resource.id).await?;
    if policy.mode!=AccessMode::Enforced {return Ok(None);}
    Ok(Some(match resource.kind {ResourceKind::K8sCluster=>Feature::Kubernetes,ResourceKind::AwsAccount=>Feature::Aws,ResourceKind::Connection if op=="db_query"=>Feature::Database,ResourceKind::Connection=>Feature::Connections,ResourceKind::McpServer=>Feature::Mcp}))
}

async fn indirect_resource_route(pool: &otto_state::SqlitePool, template: &str, path: &str) -> otto_core::Result<Option<(otto_core::access::ResourceKind,String)>> {
    if let Some(resource) = resource_route(template,path) { return Ok(Some(resource)); }
    let t = template.strip_prefix("/api/v1").unwrap_or(template);
    let p = path.strip_prefix("/api/v1").unwrap_or(path);
    let id = p.split('/').nth(3).unwrap_or_default().to_string();
    let server = match t {
        "/mcp/tools/{tool_id}" => Some(otto_state::McpToolsRepo::new(pool.clone()).get(&id).await?.server_id),
        "/mcp/approvals/{id}/decide" => otto_state::McpApprovalRepo::new(pool.clone()).get(&id).await?.server_id,
        _ => None,
    };
    Ok(server.map(|id| (otto_core::access::ResourceKind::McpServer,id)))
}

/// Bind only explicitly resource-keyed templates. Collection/create routes must
/// never acquire a lower feature tier by looking like an ID in a raw URL.
fn resource_route(template: &str, path: &str) -> Option<(otto_core::access::ResourceKind, String)> {
    use otto_core::access::ResourceKind;
    let template = template.strip_prefix("/api/v1").unwrap_or(template);
    let path = path.strip_prefix("/api/v1").unwrap_or(path);
    for (prefix, kind) in [("/connections/",ResourceKind::Connection), ("/mcp/servers/",ResourceKind::McpServer),
        ("/mcp-servers/",ResourceKind::McpServer), ("/aws/accounts/",ResourceKind::AwsAccount),
        ("/k8s/clusters/",ResourceKind::K8sCluster)] {
        if template.strip_prefix(prefix).is_some_and(|tail| tail == "{id}" || tail.starts_with("{id}/")) {
            let id = path.strip_prefix(prefix)?.split('/').next()?;
            if !id.is_empty() { return Some((kind,id.to_string())); }
        }
    }
    None
}

#[cfg(test)]
mod resource_route_tests {
    use super::*;
    #[test]
    fn creates_and_imports_do_not_lower_feature_tiers() {
        assert!(resource_route("/api/v1/aws/accounts","/api/v1/aws/accounts").is_none());
        assert!(resource_route("/api/v1/k8s/clusters/import","/api/v1/k8s/clusters/import").is_none());
        assert!(resource_route("/api/v1/connections/unsaved/db/test","/api/v1/connections/unsaved/db/test").is_none());
        assert_eq!(resource_route("/api/v1/k8s/clusters/{id}/exec","/api/v1/k8s/clusters/cluster-y/exec"),Some((otto_core::access::ResourceKind::K8sCluster,"cluster-y".into())));
    }
}
