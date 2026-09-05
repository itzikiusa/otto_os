//! Connection resource gates used by service entry points and HTTP adapters.
use otto_core::access::{AccessMode, ResourceKind, ResourceRef};
use otto_core::domain::Connection;
use otto_core::domain::{Capability, Feature};
use otto_core::{Error, Id, Result};
use otto_rbac::resource_access::ResourceAccess;
use otto_state::{GrantsRepo, SqlitePool, UsersRepo, WorkspacesRepo};

pub(crate) async fn enforced(pool: &SqlitePool, id: &Id) -> Result<bool> {
    Ok(
        otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
            .get_policy(ResourceKind::Connection, id)
            .await?
            .mode
            == AccessMode::Enforced,
    )
}

pub(crate) async fn check(
    pool: &SqlitePool,
    conn: &Connection,
    user_id: &Id,
    operation: &str,
) -> Result<()> {
    if !enforced(pool, &conn.id).await? {
        return Ok(());
    }
    let user = UsersRepo::new(pool.clone()).get(user_id).await?;
    if user.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    GrantsRepo::new(pool.clone())
        .check_global(
            &user,
            Feature::Connections,
            Capability::View,
            "Connections feature access required",
        )
        .await?;
    if let Some(ws) = &conn.workspace_id {
        if WorkspacesRepo::new(pool.clone())
            .role_of(&user, ws)
            .await?
            .is_none()
        {
            return Err(Error::NotFound("connection".into()));
        }
    }
    let resource = ResourceRef {
        kind: ResourceKind::Connection,
        id: conn.id.clone(),
        child: None,
    };
    let access = ResourceAccess::new(pool.clone());
    if !access.evaluate(&user, &resource, "discover").await?.allowed {
        return Err(Error::NotFound("connection".into()));
    }
    access.check(&user, &resource, operation).await
}

pub(crate) fn redact(mut conn: Connection) -> Connection {
    conn.params = serde_json::json!({});
    conn.secret_ref = None;
    conn.first_command = None;
    conn
}
