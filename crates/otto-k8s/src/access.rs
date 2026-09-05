//! Cluster permissions, namespace isolation and action operation mapping.
use crate::resources::Kind;
use axum::body::Body;
use futures_util::StreamExt;
use otto_core::access::{AccessActor, AccessPolicy, ResourceKind, ResourceRef};
use otto_core::domain::{Capability, Feature, User};
use otto_core::{Error, Id, Result};
use otto_rbac::resource_access::ResourceAccess;
use otto_state::{GrantsRepo, SqlitePool};

pub fn namespace(ns: Option<&str>) -> Result<Option<String>> {
    let Some(ns) = ns.map(str::trim).filter(|ns| !ns.is_empty()) else {
        return Ok(None);
    };
    if ns.len() > 63
        || !ns
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || ns.starts_with('-')
        || ns.ends_with('-')
    {
        return Err(Error::Invalid("invalid Kubernetes namespace".into()));
    }
    Ok(Some(format!("namespace:{ns}")))
}

pub fn validate_policy(policy: &AccessPolicy) -> Result<()> {
    otto_core::access::validate_policy(policy)?;
    for rule in &policy.rules {
        if let Some(children) = &rule.children {
            for child in children {
                let ns = child.strip_prefix("namespace:").ok_or_else(|| {
                    Error::Invalid("Kubernetes child scopes must be namespace:<name>".into())
                })?;
                if namespace(Some(ns))?.as_deref() != Some(child) {
                    return Err(Error::Invalid("invalid namespace scope".into()));
                }
            }
            if rule
                .operations
                .iter()
                .chain(&rule.grantable_operations)
                .any(|op| matches!(op.as_str(), "configure" | "manage_access" | "k9s"))
            {
                return Err(Error::Invalid(
                    "cluster configuration and k9s require unrestricted cluster scope".into(),
                ));
            }
        }
    }
    Ok(())
}

pub fn read_operation(kind: Kind) -> &'static str {
    match kind {
        Kind::Pods
        | Kind::Deployments
        | Kind::Statefulsets
        | Kind::Daemonsets
        | Kind::Replicasets
        | Kind::Jobs
        | Kind::Cronjobs
        | Kind::Rollouts
        | Kind::Applications => "workloads_view",
        Kind::Secrets => "secrets_view",
        _ => "resources_view",
    }
}

pub fn action_operation(action: &str) -> Result<&'static str> {
    Ok(match action.trim() {
        "restart" | "argocd_app_restart" => "restart",
        "scale" => "scale",
        "delete_pod" => "delete",
        "rollout_status" => "workloads_view",
        "rollout_undo"
        | "rollout_pause"
        | "rollout_resume"
        | "rollout_promote"
        | "rollout_abort"
        | "rollout_retry"
        | "argocd_sync"
        | "argocd_refresh"
        | "argocd_terminate_op"
        | "cronjob_trigger"
        | "cronjob_suspend"
        | "cronjob_resume" => "apply",
        _ => return Err(Error::Invalid("unknown Kubernetes action".into())),
    })
}

pub async fn allowed(
    pool: &SqlitePool,
    user: &User,
    id: &Id,
    operation: &str,
    ns: Option<&str>,
) -> Result<bool> {
    let evaluator = ResourceAccess::new(pool.clone());
    if GrantsRepo::new(pool.clone())
        .capability_of(user, Feature::Kubernetes)
        .await?
        < Capability::View
    {
        return Ok(false);
    }
    let resource = ResourceRef {
        kind: ResourceKind::K8sCluster,
        id: id.clone(),
        child: namespace(ns)?,
    };
    match evaluator.evaluate(user, &resource, operation).await {
        Ok(decision) => Ok(decision.allowed),
        Err(Error::NotFound(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

pub async fn check(
    pool: &SqlitePool,
    user: &User,
    id: &Id,
    operation: &str,
    ns: Option<&str>,
) -> Result<()> {
    if !allowed(pool, user, id, "discover", None).await? {
        return Err(Error::NotFound("Kubernetes cluster".into()));
    }
    if allowed(pool, user, id, operation, ns).await? {
        Ok(())
    } else {
        Err(Error::Forbidden(format!(
            "cluster does not allow {operation} for this namespace"
        )))
    }
}

/// k9s may change namespace and invoke every supported action. Its dedicated
/// grant therefore does not override a narrower denial of logs/exec/mutation.
pub async fn check_k9s(pool: &SqlitePool, user: &User, id: &Id) -> Result<()> {
    for op in [
        "k9s",
        "workloads_view",
        "resources_view",
        "secrets_view",
        "logs",
        "metrics",
        "exec",
        "apply",
        "scale",
        "restart",
        "delete",
    ] {
        check(pool, user, id, op, None).await?;
    }
    Ok(())
}

pub async fn initialize(pool: &SqlitePool, user: &User, id: &Id) -> Result<()> {
    let capability = GrantsRepo::new(pool.clone())
        .capability_of(user, Feature::Kubernetes)
        .await?;
    let operations: Vec<String> = otto_core::access::operations_for(ResourceKind::K8sCluster)
        .iter()
        .filter(|op| {
            let need = match **op {
                "configure" | "manage_access" | "k9s" => Capability::Admin,
                "exec" | "apply" | "scale" | "restart" | "delete" => Capability::Edit,
                _ => Capability::View,
            };
            capability >= need
        })
        .map(|op| (*op).to_string())
        .collect();
    let actor = AccessActor {
        real_user_id: user.id.clone(),
        effective_user_id: None,
    };
    otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
        .initialize_owner_policy(
            ResourceKind::K8sCluster,
            id,
            &user.id,
            &operations,
            &operations,
            &actor,
        )
        .await?;
    Ok(())
}

/// Recheck the grant while following logs, including when no data arrives.
pub fn guard_body(body: Body, pool: SqlitePool, user: User, id: Id, ns: String) -> Body {
    let stream = futures_util::stream::unfold(
        (body.into_data_stream(), pool, user, id, ns),
        |(mut stream, pool, user, id, ns)| async move {
            loop {
                let current = match otto_state::UsersRepo::new(pool.clone()).get(&user.id).await {
                    Ok(user) => user,
                    Err(_) => return None,
                };
                if !matches!(GrantsRepo::new(pool.clone()).capability_of(&current, Feature::Kubernetes).await, Ok(cap) if cap >= Capability::View)
                {
                    return None;
                }
                if !matches!(
                    allowed(&pool, &current, &id, "logs", Some(&ns)).await,
                    Ok(true)
                ) {
                    return None;
                }
                tokio::select! {
                    data = stream.next() => return data.map(|data| (data, (stream, pool, user, id, ns))),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                }
            }
        },
    );
    Body::from_stream(stream)
}

/// Native credential attachment is host authority, not delegated resource configuration.
pub fn require_setup_authority(user: &User) -> Result<()> {
    if user.is_root && !user.disabled {
        Ok(())
    } else {
        Err(Error::Forbidden(
            "only root can attach or change native cloud credentials".into(),
        ))
    }
}

/// Legacy resources still require the original Admin feature tier to expose configuration.
pub async fn can_configure(pool: &SqlitePool, user: &User, id: &Id) -> Result<bool> {
    if !allowed(pool, user, id, "configure", None).await? {
        return Ok(false);
    }
    let policy = otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
        .get_policy(ResourceKind::K8sCluster, id)
        .await?;
    Ok(policy.mode != otto_core::access::AccessMode::Legacy
        || GrantsRepo::new(pool.clone())
            .capability_of(user, Feature::Kubernetes)
            .await?
            >= Capability::Admin)
}
