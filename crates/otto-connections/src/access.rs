//! Connection resource gates used by service entry points and HTTP adapters.
use otto_core::access::{AccessMode, ResourceKind, ResourceRef};
use otto_core::domain::Connection;
use otto_core::domain::{Capability, ConnectionKind, Environment, Feature};
use otto_core::{Error, Id, Result};
use otto_rbac::resource_access::ResourceAccess;
use otto_state::{DbPool, GrantsRepo, UsersRepo, WorkspacesRepo};

pub(crate) async fn enforced(pool: &DbPool, id: &Id) -> Result<bool> {
    Ok(
        otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
            .get_policy(ResourceKind::Connection, id)
            .await?
            .mode
            == AccessMode::Enforced,
    )
}

pub(crate) async fn check(
    pool: &DbPool,
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

/// Read-only and Production are the guards an agent's writes run into (the DB
/// Explorer's read-only refusal says the one way to write is to turn the
/// setting off), so an agent session's own credential may not loosen them —
/// on a Legacy or an Enforced profile, whoever owns the token. Tightening
/// (lock it, mark it Production) and Dev ↔ Staging remove no guard and stay
/// open. `None` keeps the stored value (PATCH semantics), so it is no change.
pub(crate) fn refuse_agent_unprotect(
    existing: &Connection,
    environment: Option<Environment>,
    read_only: Option<bool>,
) -> Result<()> {
    if !otto_core::auth::request_is_agent() {
        return Ok(());
    }
    let unlocks = existing.read_only && read_only == Some(false);
    let leaves_prod = existing.environment == Environment::Prod
        && environment.is_some_and(|e| e != Environment::Prod);
    if unlocks || leaves_prod {
        return Err(Error::Forbidden(format!(
            "an agent session's credential cannot turn off Read-only or move '{}' out of \
             Production — a person must change it in Connections",
            existing.name
        )));
    }
    Ok(())
}

/// A terminal on a database/custom profile runs the native client with the
/// stored credentials — every statement typed there skips the DB Explorer's
/// read-only refusal and its production confirmation. A person may open one
/// deliberately; an agent's own credential may not open one on a Read-only or
/// Production profile (SSH profiles carry no such guard).
pub(crate) fn refuse_agent_guarded_terminal(conn: &Connection) -> Result<()> {
    let guarded = conn.read_only || conn.environment == Environment::Prod;
    if guarded && conn.kind != ConnectionKind::Ssh && otto_core::auth::request_is_agent() {
        return Err(Error::Forbidden(format!(
            "an agent session's credential cannot open a terminal on the Read-only or \
             Production connection '{}' — use the DB Explorer, or a person opens it",
            conn.name
        )));
    }
    Ok(())
}

pub(crate) fn redact(mut conn: Connection) -> Connection {
    conn.params = serde_json::json!({});
    conn.secret_ref = None;
    conn.first_command = None;
    conn
}
