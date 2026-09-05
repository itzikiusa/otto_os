//! Pre-activation native readiness checks. Runtime repeats the checks; this is
//! setup feedback, not a cached assertion of future privileges.
use super::*;
use otto_core::access::{AccessMode, AccessPolicy, RuleEffect, SubjectKind};

impl DbViewerService {
    pub async fn validate_access_policy(
        &self,
        candidate: &AccessPolicy,
        actor_user_id: &Id,
    ) -> Result<()> {
        if candidate.mode != AccessMode::Enforced {
            return Ok(());
        }
        otto_core::access::validate_policy(candidate)?;
        let actor = otto_state::UsersRepo::new(self.connections.pool())
            .get(actor_user_id)
            .await?;
        if !actor.is_root || actor.disabled {
            return Err(Error::Forbidden(
                "root must verify native credential setup before enforcing connection access"
                    .into(),
            ));
        }
        let logical = self.connections.get(&candidate.resource_id).await?;
        let Some(engine) = Engine::from_kind(logical.kind) else {
            return Ok(()); // SSH operation gates do not offer arbitrary DB scripts.
        };
        let repo = otto_state::resource_access::ResourceAccessRepo::new(self.connections.pool());
        let checker = otto_rbac::resource_access::ResourceAccess::new(self.connections.pool());
        let mut checked = std::collections::HashSet::new();
        for rule in &candidate.rules {
            if rule.effect != RuleEffect::Allow
                || !rule.operations.iter().any(|op| {
                    matches!(
                        op.as_str(),
                        "db_query" | "db_data" | "db_schema" | "change_execute"
                    )
                })
            {
                continue;
            }
            let subjects = match rule.subject_kind {
                SubjectKind::User => vec![rule.subject_id.clone()],
                SubjectKind::Group => repo.group_members(&rule.subject_id).await?,
            };
            let source = rule
                .credential_connection_id
                .as_ref()
                .unwrap_or(&logical.id);
            let profile = self.connections.get(source).await?;
            let mut left = logical.params.clone();
            let mut right = profile.params.clone();
            for params in [&mut left, &mut right] {
                if let Some(map) = params.as_object_mut() {
                    for key in ["user", "username", "password"] {
                        map.remove(key);
                    }
                }
            }
            if profile.kind != logical.kind || left != right {
                return Err(crate::native_access::setup_error("credential profile differs from the logical endpoint, database, TLS or SSH configuration"));
            }
            for subject in subjects {
                if !checked.insert((subject.clone(), source.clone())) {
                    continue;
                }
                let user = otto_state::UsersRepo::new(self.connections.pool())
                    .get(&subject)
                    .await?;
                if user.is_root || user.disabled {
                    continue;
                }
                if !matches!(engine, Engine::Mysql | Engine::Postgres) {
                    return Err(crate::native_access::setup_error(
                        "restricted script execution is unsupported for this engine",
                    ));
                }
                let resolved = self
                    .resolve(source, actor_user_id, None, "configure")
                    .await?;
                for grant in resolved.driver.native_grants(&resolved.config).await? {
                    let resource = crate::access::target(&logical.id, Some(&grant.child));
                    let permission = checker
                        .preview(&user, candidate, &resource, grant.operation)
                        .await?;
                    let profiles: std::collections::HashSet<_> = candidate
                        .rules
                        .iter()
                        .filter(|r| {
                            r.effect == RuleEffect::Allow
                                && permission.matched_rule_ids.contains(&r.id)
                        })
                        .map(|r| r.credential_connection_id.as_ref().unwrap_or(&logical.id))
                        .collect();
                    if profiles.len() > 1 {
                        return Err(crate::native_access::setup_error(
                            "candidate rules select conflicting credential profiles",
                        ));
                    }
                    if !permission.allowed {
                        let change = checker
                            .preview(&user, candidate, &resource, "change_execute")
                            .await?;
                        if permission.reason == "explicit_deny" || !change.allowed {
                            return Err(crate::native_access::setup_error("native credential privileges exceed a candidate user's effective scope"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
