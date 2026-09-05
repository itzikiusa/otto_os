//! Deny-wins resource authorization.
//!
//! Evaluation reads group membership for every decision so removals take effect
//! immediately in existing sessions. Feature, workspace, token, native
//! credential, and approval ceilings remain caller responsibilities.

use otto_core::access::{
    validate_operation, validate_policy, AccessDecision, AccessMode, AccessPolicy, AccessRule,
    ResourceAccessChecker, ResourceRef, RuleEffect, SubjectKind,
};
use otto_core::domain::User;
use otto_core::{Error, Id, Result};
use otto_state::ResourceAccessRepo;
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct ResourceAccess {
    repo: ResourceAccessRepo,
}

impl ResourceAccess {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            repo: ResourceAccessRepo::new(pool),
        }
    }

    pub async fn evaluate(
        &self,
        user: &User,
        resource: &ResourceRef,
        operation: &str,
    ) -> Result<AccessDecision> {
        validate_operation(resource.kind, operation)?;
        let policy = self
            .repo
            .get_live_policy(resource.kind, &resource.id)
            .await?;
        let groups = self.repo.groups_for_user(&user.id).await?;
        Self::evaluate_policy(
            &user.id,
            user.is_root,
            user.disabled,
            &groups,
            &policy,
            resource,
            operation,
        )
    }

    pub async fn check(&self, user: &User, resource: &ResourceRef, operation: &str) -> Result<()> {
        let decision = self.evaluate(user, resource, operation).await?;
        if decision.allowed {
            Ok(())
        } else {
            Err(Error::Forbidden(decision.reason))
        }
    }

    /// Evaluate an unsaved policy using the target user's current memberships.
    pub async fn preview(
        &self,
        user: &User,
        policy: &AccessPolicy,
        resource: &ResourceRef,
        operation: &str,
    ) -> Result<AccessDecision> {
        let groups = self.repo.groups_for_user(&user.id).await?;
        Self::evaluate_policy(
            &user.id,
            user.is_root,
            user.disabled,
            &groups,
            policy,
            resource,
            operation,
        )
    }

    /// Pure deny-wins evaluation for management previews and focused tests.
    pub fn evaluate_policy(
        user_id: &Id,
        is_root: bool,
        disabled: bool,
        group_ids: &[Id],
        policy: &AccessPolicy,
        resource: &ResourceRef,
        operation: &str,
    ) -> Result<AccessDecision> {
        validate_operation(resource.kind, operation)?;
        validate_policy(policy)?;
        if policy.kind != resource.kind || policy.resource_id != resource.id {
            return Err(Error::Invalid(
                "policy and evaluated resource do not match".into(),
            ));
        }

        if disabled {
            return Ok(decision(false, "user_disabled", Vec::new(), policy.mode));
        }
        if is_root {
            return Ok(decision(true, "root", Vec::new(), policy.mode));
        }
        if policy.mode == AccessMode::Legacy {
            return Ok(decision(
                true,
                "legacy_access",
                Vec::new(),
                AccessMode::Legacy,
            ));
        }

        let requested = if operation == "discover" && resource.child.is_none() {
            parent_discovery(user_id, group_ids, policy, resource)
        } else {
            operation_decision(user_id, group_ids, policy, resource, operation)
        };
        if operation == "discover" || !requested.allowed {
            return Ok(requested);
        }

        let discover = if resource.child.is_some() {
            operation_decision(user_id, group_ids, policy, resource, "discover")
        } else {
            unscoped_decision(user_id, group_ids, policy, "discover")
        };
        if discover.allowed {
            Ok(requested)
        } else {
            Ok(decision(
                false,
                "discover_required",
                discover.matched_rule_ids,
                policy.mode,
            ))
        }
    }
}

impl ResourceAccessChecker for ResourceAccess {
    fn evaluate<'a>(
        &'a self,
        user: &'a User,
        resource: &'a ResourceRef,
        operation: &'a str,
    ) -> otto_core::auth::BoxFuture<'a, Result<AccessDecision>> {
        Box::pin(ResourceAccess::evaluate(self, user, resource, operation))
    }

    fn check<'a>(
        &'a self,
        user: &'a User,
        resource: &'a ResourceRef,
        operation: &'a str,
    ) -> otto_core::auth::BoxFuture<'a, Result<()>> {
        Box::pin(ResourceAccess::check(self, user, resource, operation))
    }
}

fn operation_decision(
    user_id: &Id,
    group_ids: &[Id],
    policy: &AccessPolicy,
    resource: &ResourceRef,
    operation: &str,
) -> AccessDecision {
    collect_decision(policy, |rule| {
        subject_matches(rule, user_id, group_ids)
            && rule
                .operations
                .iter()
                .any(|candidate| candidate == operation)
            && operation_scope_applies(rule, resource)
    })
}

fn unscoped_decision(
    user_id: &Id,
    group_ids: &[Id],
    policy: &AccessPolicy,
    operation: &str,
) -> AccessDecision {
    collect_decision(policy, |rule| {
        subject_matches(rule, user_id, group_ids)
            && rule
                .operations
                .iter()
                .any(|candidate| candidate == operation)
            && rule.children.is_none()
    })
}

fn parent_discovery(
    user_id: &Id,
    group_ids: &[Id],
    policy: &AccessPolicy,
    resource: &ResourceRef,
) -> AccessDecision {
    let unrestricted = unscoped_decision(user_id, group_ids, policy, "discover");
    if unrestricted.reason == "explicit_deny" || unrestricted.allowed {
        return unrestricted;
    }

    let mut candidates = Vec::new();
    for rule in &policy.rules {
        if rule.effect == RuleEffect::Allow
            && subject_matches(rule, user_id, group_ids)
            && rule
                .operations
                .iter()
                .any(|operation| operation == "discover")
        {
            if let Some(children) = &rule.children {
                for child in children {
                    if !candidates.contains(child) {
                        candidates.push(child.clone());
                    }
                }
            }
        }
    }

    let mut surviving_allows = Vec::new();
    let mut child_denies = Vec::new();
    for child in candidates {
        let child_resource = ResourceRef {
            kind: resource.kind,
            id: resource.id.clone(),
            child: Some(child),
        };
        let child_decision =
            operation_decision(user_id, group_ids, policy, &child_resource, "discover");
        if child_decision.allowed {
            extend_unique(&mut surviving_allows, child_decision.matched_rule_ids);
        } else if child_decision.reason == "explicit_deny" {
            extend_unique(&mut child_denies, child_decision.matched_rule_ids);
        }
    }
    if !surviving_allows.is_empty() {
        decision(true, "explicit_allow", surviving_allows, policy.mode)
    } else if !child_denies.is_empty() {
        decision(false, "explicit_deny", child_denies, policy.mode)
    } else {
        decision(false, "no_matching_allow", Vec::new(), policy.mode)
    }
}

fn collect_decision<F>(policy: &AccessPolicy, mut applies: F) -> AccessDecision
where
    F: FnMut(&AccessRule) -> bool,
{
    let mut allows = Vec::new();
    let mut denies = Vec::new();
    for rule in &policy.rules {
        if applies(rule) {
            match rule.effect {
                RuleEffect::Allow => allows.push(rule.id.clone()),
                RuleEffect::Deny => denies.push(rule.id.clone()),
            }
        }
    }
    if !denies.is_empty() {
        decision(false, "explicit_deny", denies, policy.mode)
    } else if !allows.is_empty() {
        decision(true, "explicit_allow", allows, policy.mode)
    } else {
        decision(false, "no_matching_allow", Vec::new(), policy.mode)
    }
}

fn subject_matches(rule: &AccessRule, user_id: &Id, group_ids: &[Id]) -> bool {
    match rule.subject_kind {
        SubjectKind::User => rule.subject_id == *user_id,
        SubjectKind::Group => group_ids.contains(&rule.subject_id),
    }
}

fn operation_scope_applies(rule: &AccessRule, resource: &ResourceRef) -> bool {
    match (&resource.child, &rule.children) {
        (Some(_), None) => true,
        (Some(child), Some(children)) => children.contains(child),
        (None, None) => true,
        // Broad operations require an all-children allow. Any applicable child
        // deny still blocks a broad call, preventing child=None from bypassing
        // a namespace/database/tool restriction.
        (None, Some(_)) => rule.effect == RuleEffect::Deny,
    }
}

fn extend_unique(target: &mut Vec<Id>, values: Vec<Id>) {
    for value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn decision(
    allowed: bool,
    reason: &str,
    matched_rule_ids: Vec<Id>,
    mode: AccessMode,
) -> AccessDecision {
    AccessDecision {
        allowed,
        reason: reason.into(),
        matched_rule_ids,
        mode,
    }
}
