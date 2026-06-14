//! Auth abstractions shared by router-builder crates.
//!
//! otto-server implements these traits (backed by otto-rbac) and injects them
//! into the sessions/connections/git routers, which keeps the dependency DAG
//! flat: every crate depends only on otto-core.

use std::future::Future;
use std::pin::Pin;

use crate::domain::{User, WorkspaceRole};
use crate::{Id, Result};

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
