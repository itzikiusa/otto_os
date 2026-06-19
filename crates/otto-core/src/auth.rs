//! Auth abstractions shared by router-builder crates.
//!
//! otto-server implements these traits (backed by otto-rbac) and injects them
//! into the sessions/connections/git routers, which keeps the dependency DAG
//! flat: every crate depends only on otto-core.

use std::future::Future;
use std::pin::Pin;

use crate::domain::{GitAccount, IssueAccount, User, WorkspaceRole};
use crate::{Error, Id, Result};

/// Boxed future alias used by the auth traits (no external deps).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The authenticated user, inserted into request extensions by the server's
/// auth middleware and read by downstream handlers.
#[derive(Debug, Clone)]
pub struct AuthUser(pub User);

/// Validates a bearer token (HTTP header or WS `?token=`) into a user.
pub trait TokenAuthenticator: Send + Sync {
    fn authenticate<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<User>>;
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
    use crate::domain::{GitAccount, GitProviderKind, IssueAccount, IssueProviderKind};
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
