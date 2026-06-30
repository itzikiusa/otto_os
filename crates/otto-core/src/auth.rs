//! Auth abstractions shared by router-builder crates.
//!
//! otto-server implements these traits (backed by otto-rbac) and injects them
//! into the sessions/connections/git routers, which keeps the dependency DAG
//! flat: every crate depends only on otto-core.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::domain::{GitAccount, IssueAccount, Session, User, WorkspaceRole};
use crate::{Error, Id, Result};

/// Boxed future alias used by the auth traits (no external deps).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The authenticated user, inserted into request extensions by the server's
/// auth middleware and read by downstream handlers.
///
/// This always carries the **effective** user (== the real token owner for a
/// normal token; the impersonation *target* once Part D lands). Keeping it as
/// the effective user means every existing consumer — the feature guard,
/// `require_session_owner_or_admin`, `CurrentUser` — transparently authorizes
/// against the effective identity without any signature change.
#[derive(Debug, Clone)]
pub struct AuthUser(pub User);

/// The capability a scoped (guest / share-link) token is pinned to: exactly one
/// session, accessible at a capped [`WorkspaceRole`] (`Viewer` = read-only,
/// `Editor` = may also send input). A scoped token can touch *only* this one
/// session; the server's scope guard rejects everything else (deny-by-default).
///
/// This is intentionally distinct from a workspace membership: the `role` here
/// is the *ceiling* the share grants, independent of any role the synthetic
/// guest user holds (it holds none).
///
/// `otp_pending` is the **email-OTP gate** (mobile plan Tasks 7.2/7.3): when the
/// share was minted with a locked `recipient_email`, the guest must first redeem
/// an emailed one-time code via `POST /api/v1/share/verify`. While
/// `otp_pending == true` the scope reaches **nothing** — both the feature-guard
/// scope branch and the terminal-WS gate deny everything (only `/share/verify`,
/// which is Exempt, stays reachable). It is computed by [`TokenAuthenticator`]
/// as `recipient_email.is_some() && (verified_at.is_none() || window_expired)`,
/// so a plain share (no recipient email) always has `otp_pending == false` and
/// behaves exactly as before the gate existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionScope {
    /// The single session id this token may reach.
    pub session_id: Id,
    /// The capped role on that session: `Viewer` (read-only) or `Editor`
    /// (read + input). Never `Admin` — shares can never escalate.
    pub role: WorkspaceRole,
    /// `true` iff this share is gated by an emailed OTP the guest has not yet
    /// passed (or whose verified window has elapsed). While `true`, the scope
    /// is confined to `/share/verify` and reaches nothing else (fail closed).
    /// Always `false` for a plain (no-recipient-email) share.
    pub otp_pending: bool,
}

/// The per-token permission scope of a `kind='mcp'` token (the outward "Otto as
/// an MCP server"). It is the additional narrowing that makes **multiple MCP
/// tokens with different accesses** possible: every token is owned by a user
/// (identity + that user's base RBAC) AND carries one of these, applied at the
/// single governed choke point so it constrains BOTH the HTTP transport and the
/// legacy stdio path identically.
///
/// Stored as JSON in `auth_sessions.mcp_scope` (nullable). A NULL column ⇒
/// [`McpScope::unrestricted`] ⇒ the legacy single-token behaviour (every
/// globally-enabled tool, writes allowed) — so pre-existing tokens are unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpScope {
    /// The bare tool names (e.g. `"list_workflows"`, no `otto.` prefix) this
    /// token may call. `None` ⇒ all globally-enabled tools. `Some(list)` ⇒ only
    /// those in the list (an empty list reaches nothing). Always intersected with
    /// the server's global enabled set — a scope never *widens* access.
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    /// Whether this token may call **mutating** tools. When `false` (default) every
    /// mutating/DANGEROUS tool is denied by the scope even if it is in `tools` —
    /// the natural "read-only token" axis.
    #[serde(default)]
    pub allow_writes: bool,
    /// Optional workspace pin. When set, a tool call whose `workspace_id` argument
    /// differs is denied. Best-effort: only enforced for tools that take a
    /// `workspace_id` (tools keyed on a sub-id still defer to the owner's ws role).
    #[serde(default)]
    pub workspace_id: Option<Id>,
}

impl McpScope {
    /// The legacy/unrestricted scope: every globally-enabled tool, writes allowed,
    /// no workspace pin. A NULL `mcp_scope` column deserializes to this.
    pub fn unrestricted() -> Self {
        Self {
            tools: None,
            allow_writes: true,
            workspace_id: None,
        }
    }

    /// Returns `Some(reason)` if this scope **denies** calling `bare_tool` (a bare
    /// name like `"run_workflow"`), else `None` (the scope permits it — the global
    /// enable/approval gates still apply downstream). `mutating` is the tool's
    /// catalog classification; `ws` is the call's `workspace_id` argument if any.
    ///
    /// This is the single source of truth for per-token MCP authorization, kept in
    /// otto-core so it is dependency-free and unit-testable in isolation.
    pub fn deny_reason(&self, bare_tool: &str, mutating: bool, ws: Option<&str>) -> Option<String> {
        if let Some(list) = &self.tools {
            if !list.iter().any(|t| t == bare_tool) {
                return Some(format!(
                    "tool '{bare_tool}' is not in this token's allowed set"
                ));
            }
        }
        if mutating && !self.allow_writes {
            return Some(format!(
                "this token is read-only; '{bare_tool}' is a mutating tool"
            ));
        }
        if let (Some(pin), Some(req)) = (self.workspace_id.as_deref(), ws) {
            if pin != req {
                return Some(format!("this token is scoped to workspace '{pin}'"));
            }
        }
        None
    }
}

/// The resolved identity of an authenticated request: the **real** token owner
/// (used for audit) and the **effective** user (used for every authorization
/// decision). For a normal token these are the same user; they diverge only
/// when an admin is impersonating another user (Part D, Task 5.2), where
/// `real_user` is the admin and `effective_user` is the impersonation target.
///
/// Invariant (today): `real_user == effective_user` for every token. Task 5.2
/// is the only place that may make them differ.
///
/// `scope` is `None` for every normal/api/impersonation token (the whole
/// authorized surface is reachable). It is `Some` only for a guest/share-link
/// (`kind='share'`) token, which is then pinned to a single session — the scope
/// guard (mobile plan Task 1.5) enforces that pin. Share-token population of
/// this field lands in mobile plan Task 1.3; until then nothing sets `Some`, so
/// behavior is unchanged. Scope and impersonation are mutually exclusive.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// The token owner — the identity audit always records.
    pub real_user: User,
    /// The identity every authorization check runs against (== `real_user`
    /// unless impersonating).
    pub effective_user: User,
    /// The single-session capability a share-link token is pinned to; `None`
    /// for every unscoped (normal/api/impersonation) token.
    pub scope: Option<SessionScope>,
    /// True for a `kind='mcp'` restricted token (the outward "Otto as MCP
    /// server"). It authorizes ONLY `POST /mcp/otto-tools/invoke` (+ `GET
    /// /mcp/otto-server`); the feature guard 403s every other route, so the
    /// control plane stays in the path even if the token leaks from a
    /// `.mcp.json` (design §14 F1). `false` for every other token kind.
    pub mcp_only: bool,
    /// The per-token permission scope, present only for a `kind='mcp'` token
    /// (`None` for every other kind). A `kind='mcp'` token with a NULL
    /// `mcp_scope` column resolves to [`McpScope::unrestricted`] here, so legacy
    /// single-token behaviour is preserved. The governed invoke choke point reads
    /// this to constrain which `otto.*` tools the token may call (and whether it
    /// may call mutating ones) — the mechanism behind multiple MCP tokens with
    /// different accesses.
    pub mcp_scope: Option<McpScope>,
}

impl AuthContext {
    /// True iff this is a scoped (guest / share-link) token — i.e. it carries a
    /// [`SessionScope`] and must be confined to that one session.
    pub fn is_scoped(&self) -> bool {
        self.scope.is_some()
    }
}

/// Validates a bearer token (HTTP header or WS `?token=`) into an [`AuthContext`].
///
/// For a normal token the returned context has `real_user == effective_user`.
pub trait TokenAuthenticator: Send + Sync {
    fn authenticate<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<AuthContext>>;
}

/// Checks that a user holds at least `min` role in a workspace (root passes).
pub trait RoleChecker: Send + Sync {
    fn check<'a>(
        &'a self,
        user: &'a User,
        workspace_id: &'a Id,
        min: WorkspaceRole,
    ) -> BoxFuture<'a, Result<()>>;
}

/// Called by [`otto_state::GrantsRepo::set_grants`] after writing new grants so
/// any auth-lookup cache can evict stale entries for `user_id`.
///
/// The no-op implementation is used wherever no cache is wired in (tests,
/// components that opt out). The real implementation is `AuthCache` in
/// `otto-rbac`.
pub trait GrantsInvalidator: Send + Sync {
    fn invalidate_user(&self, user_id: &str);
}

/// True iff `user` may access `session`: root, the session's creator, or a
/// workspace **Admin** of the session's workspace.
///
/// This is the single source of truth for per-session ownership. Both the
/// lower `otto-sessions` HTTP handlers and the higher `otto-server`
/// `require_session_owner_or_admin` wrapper call it, so the owner-or-admin axis
/// is defined in exactly one place. Root and the owner are decided without a DB
/// round-trip; the admin branch defers to the injected [`RoleChecker`] (which
/// also short-circuits root inside `role_of`).
pub async fn session_owner_or_admin(
    roles: &dyn RoleChecker,
    user: &User,
    session: &Session,
) -> bool {
    user.is_root
        || session.created_by == user.id
        || roles
            .check(user, &session.workspace_id, WorkspaceRole::Admin)
            .await
            .is_ok()
}

// ---------------------------------------------------------------------------
// Credential ownership
// ---------------------------------------------------------------------------

/// A stored credential that belongs to exactly one user.
///
/// Implemented by every account type whose stored token can be *used* to act as
/// that user against a third-party service (Jira/Confluence, GitHub/GitLab/…).
/// The [`authorize_owner`] guard is the single chokepoint that keeps one user
/// from acting through another user's credential.
pub trait OwnedCredential {
    /// The id of the user that owns this credential.
    fn owner_id(&self) -> &Id;
}

impl OwnedCredential for GitAccount {
    fn owner_id(&self) -> &Id {
        &self.user_id
    }
}

impl OwnedCredential for IssueAccount {
    fn owner_id(&self) -> &Id {
        &self.user_id
    }
}

/// Authorize a caller to *use* a stored credential.
///
/// A credential may be used only by its owner or by root. This is the canonical
/// guard for every code path that resolves an account id and then acts with that
/// account's third-party token — without it, any authenticated user could read
/// or write through another user's identity (the S4 cross-user credential leak).
///
/// Returns `Forbidden` when `user` is neither the owner nor root.
pub fn authorize_owner<C: OwnedCredential>(account: &C, user: &User) -> Result<()> {
    if account.owner_id() != &user.id && !user.is_root {
        return Err(Error::Forbidden("not the account owner".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        GitAccount, GitProviderKind, IssueAccount, IssueProviderKind, Session, SessionKind,
        SessionStatus,
    };
    use chrono::Utc;

    /// A trivial credential to exercise the generic guard directly.
    struct Cred(Id);
    impl OwnedCredential for Cred {
        fn owner_id(&self) -> &Id {
            &self.0
        }
    }

    fn user(id: &str, is_root: bool) -> User {
        User {
            id: id.into(),
            username: id.into(),
            display_name: id.into(),
            is_root,
            disabled: false,
            created_at: Utc::now(),
        }
    }

    // ---- session_owner_or_admin -------------------------------------------

    /// A stub [`RoleChecker`] that grants `granted_role` to exactly one
    /// `(user_id, workspace_id)` pair and denies everyone else. Root is *not*
    /// special-cased here so the test proves the helper's own root branch (not
    /// the checker's) handles root.
    struct StubRoles {
        ok_user: &'static str,
        ok_ws: &'static str,
        granted: WorkspaceRole,
    }

    impl RoleChecker for StubRoles {
        fn check<'a>(
            &'a self,
            user: &'a User,
            workspace_id: &'a Id,
            min: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move {
                if user.id == self.ok_user && workspace_id == self.ok_ws && self.granted >= min {
                    Ok(())
                } else {
                    Err(Error::Forbidden("stub: insufficient role".into()))
                }
            })
        }
    }

    fn session(id: &str, ws: &str, created_by: &str) -> Session {
        Session {
            id: id.into(),
            workspace_id: ws.into(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "t".into(),
            status: SessionStatus::Running,
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: created_by.into(),
            created_at: Utc::now(),
            last_active_at: Utc::now(),
            archived: false,
            meta: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn owner_may_access_own_session() {
        // alice has no role at all in the workspace, yet owns the session.
        let roles = StubRoles {
            ok_user: "nobody",
            ok_ws: "ws1",
            granted: WorkspaceRole::Viewer,
        };
        let s = session("s1", "ws1", "alice");
        assert!(session_owner_or_admin(&roles, &user("alice", false), &s).await);
    }

    #[tokio::test]
    async fn non_owner_editor_is_denied() {
        // bob is a workspace Editor but not the owner -> denied (Editor < Admin).
        let roles = StubRoles {
            ok_user: "bob",
            ok_ws: "ws1",
            granted: WorkspaceRole::Editor,
        };
        let s = session("s1", "ws1", "alice");
        assert!(!session_owner_or_admin(&roles, &user("bob", false), &s).await);
    }

    #[tokio::test]
    async fn workspace_admin_non_owner_is_allowed() {
        // carol is a workspace Admin (but not the owner) -> allowed.
        let roles = StubRoles {
            ok_user: "carol",
            ok_ws: "ws1",
            granted: WorkspaceRole::Admin,
        };
        let s = session("s1", "ws1", "alice");
        assert!(session_owner_or_admin(&roles, &user("carol", false), &s).await);
    }

    #[tokio::test]
    async fn root_is_always_allowed_without_role_rows() {
        // The stub grants nothing to root; the helper's own root branch wins.
        let roles = StubRoles {
            ok_user: "nobody",
            ok_ws: "nowhere",
            granted: WorkspaceRole::Viewer,
        };
        let s = session("s1", "ws1", "alice");
        assert!(session_owner_or_admin(&roles, &user("root", true), &s).await);
    }

    fn git_account_owned_by(owner: &str) -> GitAccount {
        GitAccount {
            id: "g1".into(),
            user_id: owner.into(),
            provider: GitProviderKind::Github,
            label: "gh".into(),
            username: "octocat".into(),
            token_ref: "gitacct-1".into(),
            api_base_url: None,
            namespace: None,
            token_expires_at: None,
            created_at: Utc::now(),
        }
    }

    fn issue_account_owned_by(owner: &str) -> IssueAccount {
        IssueAccount {
            id: "i1".into(),
            user_id: owner.into(),
            provider: IssueProviderKind::Jira,
            label: "work".into(),
            email: "owner@example.com".into(),
            token_ref: "issueacct-1".into(),
            base_url: "https://example.atlassian.net".into(),
            token_expires_at: None,
            created_at: Utc::now(),
        }
    }

    /// A normal-token [`AuthContext`] resolves `real_user == effective_user`:
    /// the plumbing introduced for impersonation must be a no-op until Task 5.2
    /// actually diverges the two identities.
    #[test]
    fn auth_context_normal_token_real_equals_effective() {
        let u = user("alice", false);
        let ctx = AuthContext {
            real_user: u.clone(),
            effective_user: u,
            scope: None,
            mcp_only: false,
            mcp_scope: None,
        };
        assert_eq!(ctx.real_user.id, ctx.effective_user.id);
        assert_eq!(ctx.real_user.username, ctx.effective_user.username);
        assert_eq!(ctx.real_user.is_root, ctx.effective_user.is_root);
        // A normal token is unscoped: it can reach the whole authorized surface.
        assert!(ctx.scope.is_none());
        assert!(!ctx.is_scoped());
    }

    /// A share-link [`AuthContext`] carries an optional [`SessionScope`] that pins
    /// it to exactly one session with a capped role. (Nothing mints `Some` yet —
    /// that lands in plan Task 1.3 — but the shape must exist for it to.)
    #[test]
    fn auth_context_carries_optional_session_scope() {
        let u = user("guest", false);
        let ctx = AuthContext {
            real_user: u.clone(),
            effective_user: u,
            scope: Some(SessionScope {
                session_id: Id::from("S1"),
                role: WorkspaceRole::Viewer,
                otp_pending: false,
            }),
            mcp_only: false,
            mcp_scope: None,
        };
        assert!(ctx.is_scoped());
        let scope = ctx.scope.expect("scoped token must carry a SessionScope");
        assert_eq!(scope.session_id, Id::from("S1"));
        assert_eq!(scope.role, WorkspaceRole::Viewer);
        assert!(!scope.otp_pending, "a plain share is not OTP-pending");
    }

    // ---- McpScope -----------------------------------------------------------

    #[test]
    fn unrestricted_scope_permits_everything() {
        let s = McpScope::unrestricted();
        assert!(s.deny_reason("run_workflow", true, Some("ws1")).is_none());
        assert!(s.deny_reason("list_workflows", false, None).is_none());
    }

    #[test]
    fn read_only_scope_denies_mutating_tools() {
        let s = McpScope {
            tools: None,
            allow_writes: false,
            workspace_id: None,
        };
        // A read tool is fine.
        assert!(s.deny_reason("list_workflows", false, None).is_none());
        // A mutating tool is denied even though the tool list is unrestricted.
        let reason = s
            .deny_reason("run_workflow", true, None)
            .expect("mutating tool must be denied for a read-only token");
        assert!(reason.contains("read-only"), "got {reason}");
    }

    #[test]
    fn tool_allowlist_denies_out_of_scope_tools() {
        let s = McpScope {
            tools: Some(vec!["list_workflows".into(), "get_workflow".into()]),
            allow_writes: true,
            workspace_id: None,
        };
        assert!(s.deny_reason("list_workflows", false, None).is_none());
        // Not in the list → denied even though writes are allowed.
        let reason = s
            .deny_reason("list_repos", false, None)
            .expect("out-of-scope tool must be denied");
        assert!(reason.contains("allowed set"), "got {reason}");
    }

    #[test]
    fn workspace_pin_denies_other_workspaces() {
        let s = McpScope {
            tools: None,
            allow_writes: true,
            workspace_id: Some("ws-allowed".into()),
        };
        // Matching workspace → allowed.
        assert!(s
            .deny_reason("list_sessions", false, Some("ws-allowed"))
            .is_none());
        // Different workspace → denied.
        let reason = s
            .deny_reason("list_sessions", false, Some("ws-other"))
            .expect("a different workspace must be denied");
        assert!(reason.contains("ws-allowed"), "got {reason}");
        // A call with no workspace arg is not blocked by the pin (best-effort).
        assert!(s.deny_reason("list_bundled_skills", false, None).is_none());
    }

    #[test]
    fn mcp_scope_json_round_trips_and_defaults() {
        let s = McpScope {
            tools: Some(vec!["list_workflows".into()]),
            allow_writes: true,
            workspace_id: Some("ws1".into()),
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: McpScope = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
        // Absent fields default (forward-compatible with a minimal stored blob).
        let minimal: McpScope = serde_json::from_str("{}").unwrap();
        assert_eq!(minimal.tools, None);
        assert!(!minimal.allow_writes);
        assert_eq!(minimal.workspace_id, None);
    }

    #[test]
    fn owner_is_authorized() {
        assert!(authorize_owner(&Cred("alice".into()), &user("alice", false)).is_ok());
    }

    #[test]
    fn non_owner_is_forbidden() {
        let err = authorize_owner(&Cred("alice".into()), &user("mallory", false)).unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
    }

    #[test]
    fn root_can_use_any_credential() {
        assert!(authorize_owner(&Cred("alice".into()), &user("root", true)).is_ok());
    }

    #[test]
    fn git_account_non_owner_forbidden_owner_ok() {
        let acct = git_account_owned_by("alice");
        assert!(authorize_owner(&acct, &user("alice", false)).is_ok());
        assert!(matches!(
            authorize_owner(&acct, &user("bob", false)).unwrap_err(),
            Error::Forbidden(_)
        ));
        assert!(authorize_owner(&acct, &user("root", true)).is_ok());
    }

    #[test]
    fn issue_account_non_owner_forbidden_owner_ok() {
        let acct = issue_account_owned_by("alice");
        assert!(authorize_owner(&acct, &user("alice", false)).is_ok());
        assert!(matches!(
            authorize_owner(&acct, &user("bob", false)).unwrap_err(),
            Error::Forbidden(_)
        ));
        assert!(authorize_owner(&acct, &user("root", true)).is_ok());
    }
}
