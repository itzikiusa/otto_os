//! Auth abstractions shared by router-builder crates.
//!
//! otto-server implements these traits (backed by otto-rbac) and injects them
//! into the sessions/connections/git routers, which keeps the dependency DAG
//! flat: every crate depends only on otto-core.

use std::future::Future;
use std::pin::Pin;

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

/// The resolved identity of an authenticated request: the **real** token owner
/// (used for audit) and the **effective** user (used for every authorization
/// decision). For a normal token these are the same user; they diverge only
/// when an admin is impersonating another user (Part D, Task 5.2), where
/// `real_user` is the admin and `effective_user` is the impersonation target.
///
/// Invariant (today): `real_user == effective_user` for every token. Task 5.2
/// is the only place that may make them differ.
///
/// NOTE (mobile spec): a `scope: Option<SessionScope>` field will be added here
/// later for guest/share-link tokens (mutually exclusive with impersonation).
/// It is intentionally **not** added yet — adding it now would pull in unused
/// types. The struct is shaped so that field can land without churning callers.
#[derive(Debug, Clone)]
pub struct AuthContext {
    /// The token owner — the identity audit always records.
    pub real_user: User,
    /// The identity every authorization check runs against (== `real_user`
    /// unless impersonating).
    pub effective_user: User,
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
        };
        assert_eq!(ctx.real_user.id, ctx.effective_user.id);
        assert_eq!(ctx.real_user.username, ctx.effective_user.username);
        assert_eq!(ctx.real_user.is_root, ctx.effective_user.is_root);
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
