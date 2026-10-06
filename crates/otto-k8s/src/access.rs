//! Cluster permissions, namespace isolation and action operation mapping.
use crate::resources::Kind;
use axum::body::Body;
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

/// For each namespace, whether ANY of `operations` is allowed there — the
/// namespace picker's question. [`allowed`] reloads the grant tier, policy and
/// group memberships on every call (~3 queries); a scoped user on a cluster
/// with 500 namespaces × 10 operations cost ~15k queries per page load
/// (S6-18). This loads them ONCE and evaluates in memory with the same
/// deny-wins [`ResourceAccess::evaluate_policy`].
pub async fn namespaces_allowing_any(
    pool: &DbPool,
    user: &User,
    id: &Id,
    operations: &[&str],
    namespaces: &[String],
) -> Result<Vec<bool>> {
    if GrantsRepo::new(pool.clone())
        .capability_of(user, Feature::Kubernetes)
        .await?
        < Capability::View
    {
        return Ok(vec![false; namespaces.len()]);
    }
    let repo = otto_state::resource_access::ResourceAccessRepo::new(pool.clone());
    let policy = match repo.get_live_policy(ResourceKind::K8sCluster, id).await {
        Ok(policy) => policy,
        Err(Error::NotFound(_)) => return Ok(vec![false; namespaces.len()]),
        Err(error) => return Err(error),
    };
    let groups = repo.groups_for_user(&user.id).await?;
    Ok(namespaces_allowing_any_in(
        user, &groups, &policy, id, operations, namespaces,
    ))
}

/// Pure half of [`namespaces_allowing_any`]: an invalid namespace name or an
/// evaluation error denies that namespace.
fn namespaces_allowing_any_in(
    user: &User,
    groups: &[Id],
    policy: &AccessPolicy,
    id: &Id,
    operations: &[&str],
    namespaces: &[String],
) -> Vec<bool> {
    namespaces
        .iter()
        .map(|ns| {
            let Ok(child) = namespace(Some(ns)) else {
                return false;
            };
            let resource = ResourceRef {
                kind: ResourceKind::K8sCluster,
                id: id.clone(),
                child,
            };
            operations.iter().any(|op| {
                ResourceAccess::evaluate_policy(
                    &user.id,
                    user.is_root,
                    user.disabled,
                    groups,
                    policy,
                    &resource,
                    op,
                )
                .is_ok_and(|d| d.allowed)
            })
        })
        .collect()
}

/// A per-request snapshot of the caller's Kubernetes tier and group
/// memberships, so a handler that asks about MANY clusters or namespaces
/// loads them once and each cluster's live policy once, then evaluates in
/// memory with the same deny-wins [`ResourceAccess::evaluate_policy`] as
/// [`allowed`] (S6-309: the fleet scope used to cost ~3 queries per
/// `allowed()` × (2 + granted namespaces) per cluster, on every request).
pub struct Evaluator {
    user: User,
    groups: Vec<Id>,
    has_tier: bool,
}

impl Evaluator {
    pub async fn load(pool: &DbPool, user: &User) -> Result<Self> {
        let has_tier = GrantsRepo::new(pool.clone())
            .capability_of(user, Feature::Kubernetes)
            .await?
            >= Capability::View;
        let groups = if has_tier {
            otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
                .groups_for_user(&user.id)
                .await?
        } else {
            Vec::new()
        };
        Ok(Self {
            user: user.clone(),
            groups,
            has_tier,
        })
    }

    /// Cluster `id`'s live policy; `None` when the caller lacks the
    /// Kubernetes tier or the cluster is gone (both deny, as in [`allowed`]).
    pub async fn policy(&self, pool: &DbPool, id: &Id) -> Result<Option<AccessPolicy>> {
        if !self.has_tier {
            return Ok(None);
        }
        match otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
            .get_live_policy(ResourceKind::K8sCluster, id)
            .await
        {
            Ok(policy) => Ok(Some(policy)),
            Err(Error::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// [`allowed`] against an already-loaded `policy`; an invalid namespace
    /// or an evaluation error denies.
    pub fn allows(&self, policy: &AccessPolicy, id: &Id, op: &str, ns: Option<&str>) -> bool {
        let Ok(child) = namespace(ns) else {
            return false;
        };
        let resource = ResourceRef {
            kind: ResourceKind::K8sCluster,
            id: id.clone(),
            child,
        };
        ResourceAccess::evaluate_policy(
            &self.user.id,
            self.user.is_root,
            self.user.disabled,
            &self.groups,
            policy,
            &resource,
            op,
        )
        .is_ok_and(|d| d.allowed)
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
    // Rechecked at most once per second and on idle ticks
    // ([`otto_connections::stream_guard`], shared with the other crate).
    otto_connections::stream_guard::guard_body(body, move || {
        let (pool, user, id, ns) = (pool.clone(), user.clone(), id.clone(), ns.clone());
        async move {
            let Ok(current) = otto_state::UsersRepo::new(pool.clone()).get(&user.id).await else {
                return false;
            };
            if !matches!(GrantsRepo::new(pool.clone()).capability_of(&current, Feature::Kubernetes).await, Ok(cap) if cap >= Capability::View)
            {
                return false;
            }
            matches!(
                allowed(&pool, &current, &id, "logs", Some(&ns)).await,
                Ok(true)
            )
        }
    })
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
mod namespace_scope_tests {
    use super::*;
    use otto_core::access::{AccessMode, AccessRule, RuleEffect, SubjectKind};

    /// S6-18: the in-memory namespace filter matches the per-call evaluator —
    /// a user scoped to `shop` sees only `shop`; an invalid name is denied.
    #[test]
    fn namespace_filter_evaluates_the_loaded_policy_in_memory() {
        let id: Id = "cluster-1".into();
        let user = User {
            id: "u1".into(),
            username: "u1".into(),
            display_name: "u1".into(),
            is_root: false,
            disabled: false,
            created_at: chrono::Utc::now(),
        };
        let policy = AccessPolicy {
            kind: ResourceKind::K8sCluster,
            resource_id: id.clone(),
            mode: AccessMode::Enforced,
            revision: 1,
            rules: vec![AccessRule {
                id: "r1".into(),
                subject_kind: SubjectKind::User,
                subject_id: user.id.clone(),
                effect: RuleEffect::Allow,
                operations: vec!["discover".into(), "logs".into()],
                children: Some(vec!["namespace:shop".into()]),
                grantable_operations: vec![],
                credential_connection_id: None,
            }],
        };
        let names = vec!["shop".to_string(), "payments".into(), "Bad_Name".into()];
        let got = namespaces_allowing_any_in(&user, &[], &policy, &id, &["logs", "exec"], &names);
        assert_eq!(got, vec![true, false, false]);
        let root = User {
            is_root: true,
            ..user.clone()
        };
        let got = namespaces_allowing_any_in(&root, &[], &policy, &id, &["exec"], &names);
        assert_eq!(got, vec![true, true, false]);
    }
}
