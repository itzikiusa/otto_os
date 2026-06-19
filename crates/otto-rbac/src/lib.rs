//! otto-rbac — password hashing, token auth sessions, and role checking.
//!
//! Implements `otto_core::auth::{TokenAuthenticator, RoleChecker}` so the
//! server (and router-builder crates) only depend on the otto-core traits.

pub mod passwords;
pub mod tokens;

use otto_core::auth::{AuthContext, BoxFuture, RoleChecker, TokenAuthenticator};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id, Result};
use otto_state::WorkspacesRepo;
use sqlx::SqlitePool;

pub use passwords::{hash_password, validate_password, verify_password, MIN_PASSWORD_LEN};
pub use tokens::{AuthRepo, IMPERSONATION_TOKEN_TTL_MINS};

/// `TokenAuthenticator` backed by [`AuthRepo`].
#[derive(Clone)]
pub struct RbacAuthenticator {
    repo: AuthRepo,
}

impl RbacAuthenticator {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            repo: AuthRepo::new(pool),
        }
    }
}

impl TokenAuthenticator for RbacAuthenticator {
    fn authenticate<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<AuthContext>> {
        Box::pin(self.repo.authenticate(token))
    }
}

/// `RoleChecker` backed by `WorkspacesRepo::role_of` (root passes everywhere;
/// missing membership → `Error::Forbidden`).
#[derive(Clone)]
pub struct RbacRoleChecker {
    workspaces: WorkspacesRepo,
}

impl RbacRoleChecker {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            workspaces: WorkspacesRepo::new(pool),
        }
    }
}

impl RoleChecker for RbacRoleChecker {
    fn check<'a>(
        &'a self,
        user: &'a User,
        workspace_id: &'a Id,
        min: WorkspaceRole,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            match self.workspaces.role_of(user, workspace_id).await? {
                Some(role) if role >= min => Ok(()),
                Some(_) => Err(Error::Forbidden(format!(
                    "requires at least {} role in this workspace",
                    min.as_str()
                ))),
                None => Err(Error::Forbidden("not a member of this workspace".into())),
            }
        })
    }
}
