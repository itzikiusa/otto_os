//! Resource-bound terminals retain their resource restrictions on reconnect.
use crate::state::ServerCtx;
use otto_core::access::{ResourceKind, ResourceRef};
use otto_core::domain::{Session, User};
use otto_core::{Error, Result};

pub(crate) fn binding(session: &Session) -> Option<(ResourceRef, &'static str)> {
    if let Some(k8s) = session.meta.get("k8s") {
        let id = k8s.get("cluster_id")?.as_str()?.to_string();
        let op = if k8s.get("mode").and_then(|v| v.as_str()) == Some("k9s") {
            "k9s"
        } else {
            "exec"
        };
        let child = k8s
            .get("ns")
            .and_then(|v| v.as_str())
            .map(|ns| format!("namespace:{ns}"));
        return Some((
            ResourceRef {
                kind: ResourceKind::K8sCluster,
                id,
                child,
            },
            op,
        ));
    }
    if let Some(aws) = session.meta.get("aws") {
        return Some((
            ResourceRef {
                kind: ResourceKind::AwsAccount,
                id: aws.get("account_id")?.as_str()?.into(),
                child: None,
            },
            "configure",
        ));
    }
    if let Some(id) = &session.connection_id {
        return Some((
            ResourceRef {
                kind: ResourceKind::Connection,
                id: id.clone(),
                child: None,
            },
            "shell",
        ));
    }
    if session.meta.get("source").and_then(|v| v.as_str()) == Some("db_assist") {
        return Some((
            ResourceRef {
                kind: ResourceKind::Connection,
                id: session.meta.get("connection_id")?.as_str()?.into(),
                child: session
                    .meta
                    .get("resource_node")
                    .and_then(|v| v.as_str())
                    .filter(|n| !n.is_empty())
                    .map(|n| {
                        let path = otto_dbviewer::types::NodePath::parse(n);
                        path.get("db")
                            .or_else(|| path.get("kdb"))
                            .unwrap_or(n)
                            .to_string()
                    }),
            },
            "db_query",
        ));
    }
    None
}

pub async fn check(ctx: &ServerCtx,user:&User,session:&Session)->Result<()> {
    check_with_pool(&ctx.pool,user,session).await
}

pub(crate) async fn check_with_pool(pool:&otto_state::SqlitePool,user:&User,session:&Session)->Result<()> {
    let Some((resource, op)) = binding(session) else {
        if session.meta.get("k8s").is_some()
            || session.meta.get("aws").is_some()
            || session.meta.get("source").and_then(|v| v.as_str()) == Some("db_assist")
        {
            return Err(Error::Forbidden("invalid resource session binding".into()));
        }
        return Ok(());
    };
    let current = otto_state::UsersRepo::new(pool.clone())
        .get(&user.id)
        .await?;
    if current.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    check_page(pool, &current, &resource, op).await?;
    if op=="db_query" {
        otto_state::GrantsRepo::new(pool.clone()).check_global(&current,otto_core::domain::Feature::Agents,otto_core::domain::Capability::Edit,"DB Assistant host-agent access was revoked").await?;
    }

    match resource.kind {
        ResourceKind::K8sCluster if op == "k9s" => {
            otto_k8s::access::check_k9s(pool, &current, &resource.id).await
        }
        ResourceKind::K8sCluster => {
            otto_k8s::access::check(
                pool,
                &current,
                &resource.id,
                op,
                resource
                    .child
                    .as_deref()
                    .and_then(|c| c.strip_prefix("namespace:")),
            )
            .await
        }
        ResourceKind::AwsAccount => {
            otto_aws::access::check(pool, &current, &resource.id, op, None).await
        }
        ResourceKind::Connection => {
            let connection = otto_state::ConnectionsRepo::new(pool.clone())
                .get(&resource.id)
                .await?;
            let feature = if op == "db_query" {
                otto_core::domain::Feature::Database
            } else {
                otto_core::domain::Feature::Connections
            };
            otto_state::GrantsRepo::new(pool.clone())
                .check_global(
                    &current,
                    feature,
                    otto_core::domain::Capability::View,
                    "resource page access is disabled",
                )
                .await?;
            if let Some(ws) = &connection.workspace_id {
                if otto_state::WorkspacesRepo::new(pool.clone()).role_of(&current, ws).await?.is_none() {
                    return Err(Error::NotFound("connection".into()));
                }
            }
            otto_rbac::ResourceAccess::new(pool.clone())
                .check(&current, &resource, op)
                .await
        }
        ResourceKind::McpServer => unreachable!(),
    }
}

async fn check_page(
    pool: &otto_state::SqlitePool,
    user: &User,
    resource: &ResourceRef,
    op: &str,
) -> Result<()> {
    let policy = otto_state::ResourceAccessRepo::new(pool.clone())
        .get_live_policy(resource.kind, &resource.id)
        .await?;
    let feature = match resource.kind {
        ResourceKind::K8sCluster => otto_core::domain::Feature::Kubernetes,
        ResourceKind::AwsAccount => otto_core::domain::Feature::Aws,
        ResourceKind::Connection if op == "db_query" => otto_core::domain::Feature::Database,
        ResourceKind::Connection => otto_core::domain::Feature::Connections,
        ResourceKind::McpServer => otto_core::domain::Feature::Mcp,
    };
    let needed = if policy.mode == otto_core::access::AccessMode::Legacy {
        otto_core::domain::Capability::Edit
    } else {
        otto_core::domain::Capability::View
    };
    otto_state::GrantsRepo::new(pool.clone())
        .check_global(
            user,
            feature,
            needed,
            "resource page or legacy execution access is disabled",
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::access::{AccessActor, AccessMode};
    #[tokio::test]
    async fn resource_terminal_rechecks_page_revocation_and_legacy_execution_tier() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("reader", "hash", "Reader", false)
            .await
            .unwrap();
        let id = otto_core::new_id();
        sqlx::query("INSERT INTO connections (id,name,kind,params_json,created_by,created_at) VALUES (?,'Test','ssh','{}',?,'2026-09-05T00:00:00Z')").bind(&id).bind(&user.id).execute(&pool).await.unwrap();
        let resource = ResourceRef {
            kind: ResourceKind::Connection,
            id: id.clone(),
            child: None,
        };
        sqlx::query("INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?,'connections','view')").bind(&user.id).execute(&pool).await.unwrap();
        assert!(check_page(&pool, &user, &resource, "shell").await.is_ok());
        sqlx::query("DELETE FROM user_feature_grants WHERE user_id=?")
            .bind(&user.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(check_page(&pool, &user, &resource, "shell").await.is_err());
        sqlx::query("INSERT INTO user_feature_grants (feature,capability,user_id) VALUES ('connections','view',?)")
            .bind(&user.id)
            .execute(&pool)
            .await
            .unwrap();
        let repo = otto_state::ResourceAccessRepo::new(pool.clone());
        let mut p = repo
            .get_policy(ResourceKind::Connection, &id)
            .await
            .unwrap();
        p.mode = AccessMode::Legacy;
        repo.put_policy(
            &p,
            p.revision,
            &AccessActor {
                real_user_id: user.id.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
        assert!(check_page(&pool, &user, &resource, "shell").await.is_err());
        sqlx::query("UPDATE user_feature_grants SET capability='edit' WHERE user_id=?")
            .bind(&user.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(check_page(&pool, &user, &resource, "shell").await.is_ok());
    }
}
