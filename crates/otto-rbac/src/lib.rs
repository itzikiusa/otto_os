//! otto-rbac — password hashing, token auth sessions, and role checking.
//!
//! Implements `otto_core::auth::{TokenAuthenticator, RoleChecker}` so the
//! server (and router-builder crates) only depend on the otto-core traits.

pub mod cache;
pub mod passwords;
pub mod tokens;

use otto_core::auth::{AuthContext, BoxFuture, RoleChecker, TokenAuthenticator};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id, Result};
use otto_state::WorkspacesRepo;
use sqlx::SqlitePool;

pub use cache::{AuthCache, NoopGrantsInvalidator};
pub use passwords::{hash_password, validate_password, verify_password, MIN_PASSWORD_LEN};
pub use tokens::{AuthRepo, IMPERSONATION_TOKEN_TTL_MINS};

/// `TokenAuthenticator` backed by [`AuthRepo`].
///
/// The `new_with_cache` constructor wires an [`AuthCache`] so repeated requests
/// with the same `login`/`api` token avoid redundant SQLite reads. The cache
/// (`Arc`-backed) is returned alongside the authenticator so the caller can
/// hand an `Arc<dyn GrantsInvalidator>` to [`otto_state::GrantsRepo`].
#[derive(Clone)]
pub struct RbacAuthenticator {
    repo: AuthRepo,
}

impl RbacAuthenticator {
    /// Construct without a cache (every authenticate hits the DB). Suitable for
    /// contexts where caching is not desired (standalone components, some tests).
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            repo: AuthRepo::new(pool),
        }
    }

    /// Construct with a shared [`AuthCache`]. The caller retains a clone of
    /// `cache` to pass to [`otto_state::GrantsRepo::new_with_invalidator`] so
    /// grant changes flush the user's cached auth entries.
    pub fn new_with_cache(pool: SqlitePool, cache: AuthCache) -> Self {
        Self {
            repo: AuthRepo::with_cache(pool, cache),
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
