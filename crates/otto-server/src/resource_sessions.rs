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

pub async fn check(ctx: &ServerCtx, user: &User, session: &Session) -> Result<()> {
    check_with_pool(&ctx.pool, user, session).await
}

pub(crate) async fn check_with_pool(
    pool: &otto_state::DbPool,
    user: &User,
    session: &Session,
) -> Result<()> {
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
    check_bound(pool, &current, &resource, op).await
}

/// Batched [`check`] for a session LIST: keeps the admitted rows in order.
/// The caller is loaded at most once (only when some row is resource-bound)
/// and each distinct (resource, op) is decided once — the list used to reload
/// the user and the page grants for every k8s/aws/db row (F9).
pub async fn check_many(ctx: &ServerCtx, user: &User, sessions: Vec<Session>) -> Vec<Session> {
    check_many_with_pool(&ctx.pool, user, sessions).await
}

/// [`check_many`] against a bare pool (what the unit tests drive).
pub(crate) async fn check_many_with_pool(
    pool: &otto_state::DbPool,
    user: &User,
    sessions: Vec<Session>,
) -> Vec<Session> {
    let mut current: Option<Option<User>> = None;
    let mut memo: std::collections::HashMap<
        (ResourceKind, String, Option<String>, &'static str),
        bool,
    > = std::collections::HashMap::new();
    let mut out = Vec::with_capacity(sessions.len());
    for session in sessions {
        let ok = match binding(&session) {
            None => check_with_pool(pool, user, &session).await.is_ok(),
            Some((resource, op)) => {
                if current.is_none() {
                    current = Some(
                        otto_state::UsersRepo::new(pool.clone())
                            .get(&user.id)
                            .await
                            .ok(),
                    );
                }
                match current.as_ref().and_then(|c| c.as_ref()) {
                    None => false,
                    Some(cur) => {
                        let key = (
                            resource.kind,
                            resource.id.clone(),
                            resource.child.clone(),
                            op,
                        );
                        match memo.get(&key) {
                            Some(&v) => v,
                            None => {
                                let v = check_bound(pool, cur, &resource, op).await.is_ok();
                                memo.insert(key, v);
                                v
                            }
                        }
                    }
                }
            }
        };
        if ok {
            out.push(session);
        }
    }
    out
}

/// The resource-bound half of [`check_with_pool`], for an already-loaded user.
async fn check_bound(
    pool: &otto_state::DbPool,
    current: &User,
    resource: &ResourceRef,
    op: &'static str,
) -> Result<()> {
    if current.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    let current = current.clone();
    let resource = resource.clone();
    check_page(pool, &current, &resource, op).await?;
    if op == "db_query" {
        otto_state::GrantsRepo::new(pool.clone())
            .check_global(
                &current,
                otto_core::domain::Feature::Agents,
                otto_core::domain::Capability::Edit,
                "DB Assistant host-agent access was revoked",
            )
            .await?;
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
                if otto_state::WorkspacesRepo::new(pool.clone())
                    .role_of(&current, ws)
                    .await?
                    .is_none()
                {
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
    pool: &otto_state::DbPool,
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

    async fn pool() -> otto_state::DbPool {
        let pool = otto_state::DbPool::from(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap(),
        );
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    fn session(id: &str, connection_id: Option<&str>) -> Session {
        serde_json::from_value(serde_json::json!({
            "id": id, "workspace_id": "ws", "kind": "connection", "provider": "ssh",
            "title": id, "status": "running", "cwd": "/tmp",
            "provider_session_id": null, "connection_id": connection_id,
            "created_by": "someone", "created_at": "2026-09-05T00:00:00Z",
            "last_active_at": "2026-09-05T00:00:00Z", "archived": false, "meta": {}
        }))
        .unwrap()
    }

    /// The batched list filter (`GET /sessions`, the workspace list): a row
    /// bound to a connection whose access the caller lost is dropped, each
    /// connection is decided on its own (the memo never leaks one id's allow
    /// to another), unbound rows always stay, the input order is kept, and a
    /// disabled caller loses every bound row.
    #[tokio::test]
    async fn check_many_drops_revoked_and_disabled_bound_rows_in_order() {
        let pool = pool().await;
        let users = otto_state::UsersRepo::new(pool.clone());
        let user = users
            .create("reader", "hash", "Reader", false)
            .await
            .unwrap();
        let (open, revoked) = (otto_core::new_id(), otto_core::new_id());
        for id in [&open, &revoked] {
            sqlx::query("INSERT INTO connections (id,name,kind,params_json,created_by,created_at) VALUES (?,'Test','ssh','{}',?,'2026-09-05T00:00:00Z')").bind(id).bind(&user.id).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?,'connections','edit')").bind(&user.id).execute(&pool).await.unwrap();
        // `open` keeps execution through the legacy tier; `revoked` has a
        // managed policy with no rule for this user (its grant is gone).
        let repo = otto_state::ResourceAccessRepo::new(pool.clone());
        let mut p = repo
            .get_policy(ResourceKind::Connection, &open)
            .await
            .unwrap();
        p.mode = AccessMode::Legacy;
        let actor = AccessActor {
            real_user_id: user.id.clone(),
            effective_user_id: None,
        };
        repo.put_policy(&p, p.revision, &actor).await.unwrap();

        let rows = || {
            vec![
                session("plain-1", None),
                session("open-1", Some(open.as_str())),
                session("revoked-1", Some(revoked.as_str())),
                session("open-2", Some(open.as_str())),
                session("plain-2", None),
                session("revoked-2", Some(revoked.as_str())),
            ]
        };
        let ids = |v: Vec<Session>| v.into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(
            ids(check_many_with_pool(&pool, &user, rows()).await),
            ["plain-1", "open-1", "open-2", "plain-2"]
        );

        // The caller is reloaded: a disabled account keeps only unbound rows,
        // even though the stale `user` it passed in is still enabled.
        sqlx::query("UPDATE users SET disabled = 1 WHERE id = ?")
            .bind(&user.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            ids(check_many_with_pool(&pool, &user, rows()).await),
            ["plain-1", "plain-2"]
        );
    }
    /// test-quality-3-05: the memo key carries the k8s namespace. Two exec
    /// terminals on ONE cluster, in namespaces `a` and `b`, under a policy that
    /// allows exec on `namespace:a` only: just the `a` row survives, whichever
    /// row is decided (and memoised) first.
    #[tokio::test]
    async fn check_many_decides_each_k8s_namespace_separately_in_both_orders() {
        let pool = pool().await;
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("ns-reader", "hash", "NS Reader", false)
            .await
            .unwrap();
        let cluster = otto_core::new_id();
        sqlx::query("INSERT INTO k8s_clusters (id,name,source,context_name,created_by,created_at,updated_at) VALUES (?,'c','kubeconfig','ctx',?,'2026-09-05T00:00:00Z','2026-09-05T00:00:00Z')").bind(&cluster).bind(&user.id).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?,'kubernetes','edit')").bind(&user.id).execute(&pool).await.unwrap();
        let repo = otto_state::ResourceAccessRepo::new(pool.clone());
        let mut p = repo
            .get_policy(ResourceKind::K8sCluster, &cluster)
            .await
            .unwrap();
        p.mode = AccessMode::Enforced;
        let rule = |ops: &[&str], children: Option<Vec<String>>| otto_core::access::AccessRule {
            id: otto_core::new_id(),
            subject_kind: otto_core::access::SubjectKind::User,
            subject_id: user.id.clone(),
            effect: otto_core::access::RuleEffect::Allow,
            operations: ops.iter().map(|o| o.to_string()).collect(),
            children,
            grantable_operations: vec![],
            credential_connection_id: None,
        };
        p.rules = vec![
            rule(&["discover"], None),
            rule(&["exec"], Some(vec!["namespace:a".into()])),
        ];
        let actor = AccessActor {
            real_user_id: user.id.clone(),
            effective_user_id: None,
        };
        repo.put_policy(&p, p.revision, &actor).await.unwrap();

        let k8s = |id: &str, ns: &str| -> Session {
            let mut s = session(id, None);
            s.meta = serde_json::json!({"k8s": {"cluster_id": cluster, "ns": ns}});
            s
        };
        let ids = |v: Vec<Session>| v.into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(
            ids(check_many_with_pool(&pool, &user, vec![k8s("in-a", "a"), k8s("in-b", "b")]).await),
            ["in-a"]
        );
        assert_eq!(
            ids(check_many_with_pool(&pool, &user, vec![k8s("in-b", "b"), k8s("in-a", "a")]).await),
            ["in-a"]
        );
    }

    #[tokio::test]
    async fn resource_terminal_rechecks_page_revocation_and_legacy_execution_tier() {
        let pool = otto_state::DbPool::from(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap(),
        );
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
