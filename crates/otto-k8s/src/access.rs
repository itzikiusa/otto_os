//! Cluster permissions, namespace isolation and action operation mapping.
use crate::resources::Kind;
use axum::body::Body;
use futures_util::StreamExt;
use otto_core::access::{AccessActor, AccessPolicy, ResourceKind, ResourceRef};
use otto_core::domain::{Capability, Feature, User};
use otto_core::{Error, Id, Result};
use otto_rbac::resource_access::ResourceAccess;
use otto_state::{DbPool, GrantsRepo};

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
    pool: &DbPool,
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
    pool: &DbPool,
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
pub async fn check_k9s(pool: &DbPool, user: &User, id: &Id) -> Result<()> {
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

pub async fn initialize(pool: &DbPool, user: &User, id: &Id) -> Result<()> {
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
pub fn guard_body(body: Body, pool: DbPool, user: User, id: Id, ns: String) -> Body {
    let stream = futures_util::stream::unfold(
        (
            body.into_data_stream(),
            pool,
            user,
            id,
            ns,
            None::<std::time::Instant>,
        ),
        |(mut stream, pool, user, id, ns, mut last_checked)| async move {
            loop {
                // Re-authorize at most once per [`GUARD_RECHECK`] (S6-08): a
                // check is 4+ SQLite queries, and a fast stream (64 KiB S3
                // chunks, a followed multi-pod log) otherwise ran one per
                // chunk. The idle tick below still rechecks a stalled stream.
                if recheck_due(last_checked, std::time::Instant::now()) {
                    let current = match otto_state::UsersRepo::new(pool.clone()).get(&user.id).await
                    {
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
                    last_checked = Some(std::time::Instant::now());
                }
                tokio::select! {
                    data = stream.next() => return data.map(|data| (data, (stream, pool, user, id, ns, last_checked))),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
                }
            }
        },
    );
    Body::from_stream(stream)
}

/// Longest a streamed body runs on a stale authorization (see `guard_body`).
pub const GUARD_RECHECK: std::time::Duration = std::time::Duration::from_secs(1);

/// True when a streamed body's grant is due for a recheck.
fn recheck_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    last.is_none_or(|t| now.saturating_duration_since(t) >= GUARD_RECHECK)
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
pub async fn can_configure(pool: &DbPool, user: &User, id: &Id) -> Result<bool> {
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

#[cfg(test)]
mod guard_recheck_tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// S6-08: a streamed body re-authorizes once per `GUARD_RECHECK`, not
    /// once per chunk — the first chunk always checks.
    #[test]
    fn recheck_runs_at_most_once_per_interval() {
        let t0 = Instant::now();
        assert!(recheck_due(None, t0));
        assert!(!recheck_due(Some(t0), t0));
        assert!(!recheck_due(Some(t0), t0 + Duration::from_millis(999)));
        assert!(recheck_due(Some(t0), t0 + GUARD_RECHECK));
        // 1,000 chunks inside one second cost exactly one check.
        let mut last = None;
        let mut checks = 0;
        for i in 0..1000u64 {
            let now = t0 + Duration::from_micros(i * 900);
            if recheck_due(last, now) {
                checks += 1;
                last = Some(now);
            }
        }
        assert_eq!(checks, 1);
    }
}
