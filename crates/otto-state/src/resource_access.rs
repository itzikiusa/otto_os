//! Persistence for group/user resource authorization.
//!
//! Policy heads use compare-and-swap revisions. Every successful policy write
//! stores an immutable snapshot and appends the security audit row in the same
//! SQLite transaction.

use chrono::Utc;
use otto_core::access::{
    validate_operation, validate_policy, AccessActor, AccessGroup, AccessMode, AccessPolicy,
    AccessRole, AccessRule, ResourceKind, RuleEffect, SubjectKind,
};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use crate::convert::{dberr, dberr_unique, fmt, ts};

#[derive(Clone)]
pub struct ResourceAccessRepo {
    pool: SqlitePool,
}

impl ResourceAccessRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_group(
        &self,
        name: &str,
        description: Option<&str>,
        actor: &AccessActor,
    ) -> Result<AccessGroup> {
        let name = required_name(name, "group")?;
        let id = new_id();
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("create group begin"))?;
        sqlx::query(
            "INSERT INTO access_groups (id, name, description, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(description)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(dberr_unique(
            "create access group",
            "access group name already exists",
        ))?;
        append_audit(
            &mut tx,
            actor,
            "resource_access.group_create",
            &id,
            serde_json::json!({ "name": name }),
        )
        .await?;
        tx.commit().await.map_err(dberr("create group commit"))?;
        self.get_group(&id).await
    }

    pub async fn get_group(&self, id: &Id) -> Result<AccessGroup> {
        let row = sqlx::query("SELECT * FROM access_groups WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("access group"))?;
        group_from_row(&row)
    }

    pub async fn list_groups(&self) -> Result<Vec<AccessGroup>> {
        let rows = sqlx::query("SELECT * FROM access_groups ORDER BY name, id")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list access groups"))?;
        rows.iter().map(group_from_row).collect()
    }

    pub async fn update_group(
        &self,
        id: &Id,
        name: &str,
        description: Option<&str>,
        actor: &AccessActor,
    ) -> Result<AccessGroup> {
        let name = required_name(name, "group")?;
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("update group begin"))?;
        let changed = sqlx::query(
            "UPDATE access_groups SET name = ?, description = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(description)
        .bind(&now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr_unique(
            "update access group",
            "access group name already exists",
        ))?
        .rows_affected();
        if changed == 0 {
            return Err(Error::NotFound("access group".into()));
        }
        append_audit(
            &mut tx,
            actor,
            "resource_access.group_update",
            id,
            serde_json::json!({ "name": name }),
        )
        .await?;
        tx.commit().await.map_err(dberr("update group commit"))?;
        self.get_group(id).await
    }

    pub async fn delete_group(&self, id: &Id, actor: &AccessActor) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("delete group begin"))?;
        let changed = sqlx::query("DELETE FROM access_groups WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete access group"))?
            .rows_affected();
        if changed == 0 {
            return Err(Error::NotFound("access group".into()));
        }
        append_audit(
            &mut tx,
            actor,
            "resource_access.group_delete",
            id,
            serde_json::json!({}),
        )
        .await?;
        tx.commit().await.map_err(dberr("delete group commit"))?;
        Ok(())
    }

    pub async fn add_group_member(
        &self,
        group_id: &Id,
        user_id: &Id,
        actor: &AccessActor,
    ) -> Result<()> {
        self.set_group_member(group_id, user_id, true, actor).await
    }

    pub async fn remove_group_member(
        &self,
        group_id: &Id,
        user_id: &Id,
        actor: &AccessActor,
    ) -> Result<()> {
        self.set_group_member(group_id, user_id, false, actor).await
    }

    async fn set_group_member(
        &self,
        group_id: &Id,
        user_id: &Id,
        present: bool,
        actor: &AccessActor,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(dberr("membership begin"))?;
        ensure_exists(&mut tx, "access_groups", group_id, "access group").await?;
        ensure_exists(&mut tx, "users", user_id, "user").await?;
        let changed = if present {
            sqlx::query(
                "INSERT OR IGNORE INTO access_group_members (group_id, user_id, created_at)
                 VALUES (?, ?, ?)",
            )
            .bind(group_id)
            .bind(user_id)
            .bind(fmt(Utc::now()))
            .execute(&mut *tx)
            .await
            .map_err(dberr("add access group member"))?
            .rows_affected()
        } else {
            sqlx::query("DELETE FROM access_group_members WHERE group_id = ? AND user_id = ?")
                .bind(group_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(dberr("remove access group member"))?
                .rows_affected()
        };
        if changed > 0 {
            append_audit(
                &mut tx,
                actor,
                if present {
                    "resource_access.member_add"
                } else {
                    "resource_access.member_remove"
                },
                group_id,
                serde_json::json!({ "user_id": user_id }),
            )
            .await?;
        }
        tx.commit().await.map_err(dberr("membership commit"))?;
        Ok(())
    }

    pub async fn group_members(&self, group_id: &Id) -> Result<Vec<Id>> {
        ensure_exists_pool(&self.pool, "access_groups", group_id, "access group").await?;
        sqlx::query_scalar(
            "SELECT user_id FROM access_group_members WHERE group_id = ? ORDER BY user_id",
        )
        .bind(group_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list access group members"))
    }

    pub async fn groups_for_user(&self, user_id: &Id) -> Result<Vec<Id>> {
        sqlx::query_scalar(
            "SELECT group_id FROM access_group_members WHERE user_id = ? ORDER BY group_id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list user access groups"))
    }

    pub async fn create_role(
        &self,
        name: &str,
        description: Option<&str>,
        kind: ResourceKind,
        operations: &[String],
        grantable_operations: &[String],
        actor: &AccessActor,
    ) -> Result<AccessRole> {
        validate_role(name, kind, operations, grantable_operations)?;
        let id = new_id();
        let now = fmt(Utc::now());
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("create role begin"))?;
        sqlx::query(
            "INSERT INTO access_roles
             (id, name, description, resource_kind, operations_json,
              grantable_operations_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name.trim())
        .bind(description)
        .bind(kind.as_str())
        .bind(json_strings(operations, "role operations")?)
        .bind(json_strings(
            grantable_operations,
            "role grantable operations",
        )?)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(dberr_unique(
            "create access role",
            "access role name already exists",
        ))?;
        append_audit(
            &mut tx,
            actor,
            "resource_access.role_create",
            &id,
            serde_json::json!({ "name": name.trim(), "resource_kind": kind }),
        )
        .await?;
        tx.commit().await.map_err(dberr("create role commit"))?;
        self.get_role(&id).await
    }

    pub async fn get_role(&self, id: &Id) -> Result<AccessRole> {
        let row = sqlx::query("SELECT * FROM access_roles WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("access role"))?;
        role_from_row(&row)
    }

    pub async fn list_roles(&self) -> Result<Vec<AccessRole>> {
        let rows = sqlx::query("SELECT * FROM access_roles ORDER BY resource_kind, name, id")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list access roles"))?;
        rows.iter().map(role_from_row).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_role(
        &self,
        id: &Id,
        name: &str,
        description: Option<&str>,
        kind: ResourceKind,
        operations: &[String],
        grantable_operations: &[String],
        actor: &AccessActor,
    ) -> Result<AccessRole> {
        validate_role(name, kind, operations, grantable_operations)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("update role begin"))?;
        let changed = sqlx::query(
            "UPDATE access_roles SET name = ?, description = ?, resource_kind = ?,
             operations_json = ?, grantable_operations_json = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name.trim())
        .bind(description)
        .bind(kind.as_str())
        .bind(json_strings(operations, "role operations")?)
        .bind(json_strings(
            grantable_operations,
            "role grantable operations",
        )?)
        .bind(fmt(Utc::now()))
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(dberr_unique(
            "update access role",
            "access role name already exists",
        ))?
        .rows_affected();
        if changed == 0 {
            return Err(Error::NotFound("access role".into()));
        }
        append_audit(
            &mut tx,
            actor,
            "resource_access.role_update",
            id,
            serde_json::json!({ "name": name.trim(), "resource_kind": kind }),
        )
        .await?;
        tx.commit().await.map_err(dberr("update role commit"))?;
        self.get_role(id).await
    }

    pub async fn delete_role(&self, id: &Id, actor: &AccessActor) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("delete role begin"))?;
        let changed = sqlx::query("DELETE FROM access_roles WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(dberr("delete access role"))?
            .rows_affected();
        if changed == 0 {
            return Err(Error::NotFound("access role".into()));
        }
        append_audit(
            &mut tx,
            actor,
            "resource_access.role_delete",
            id,
            serde_json::json!({}),
        )
        .await?;
        tx.commit().await.map_err(dberr("delete role commit"))?;
        Ok(())
    }

    /// Return the current policy, or the explicit synthetic Legacy revision 0
    /// state when the resource has never been initialized.
    pub async fn get_policy(&self, kind: ResourceKind, resource_id: &Id) -> Result<AccessPolicy> {
        let revision: Option<i64> = sqlx::query_scalar(
            "SELECT revision FROM resource_access_policies
             WHERE resource_kind = ? AND resource_id = ?",
        )
        .bind(kind.as_str())
        .bind(resource_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("resource access policy"))?;
        let Some(revision) = revision else {
            return Ok(AccessPolicy::legacy(kind, resource_id.clone()));
        };
        self.get_policy_version(kind, resource_id, revision).await
    }

    /// Load a policy for live authorization while proving the governed resource
    /// still exists in the same SQLite read transaction. An existing resource
    /// without a policy is rollout-compatible Legacy; a missing/deleted resource
    /// is NotFound and can never inherit that fallback.
    pub async fn get_live_policy(
        &self,
        kind: ResourceKind,
        resource_id: &Id,
    ) -> Result<AccessPolicy> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("live resource policy begin"))?;
        let table = match kind {
            ResourceKind::Connection => "connections",
            ResourceKind::McpServer => "mcp_servers",
            ResourceKind::AwsAccount => "aws_accounts",
            ResourceKind::K8sCluster => "k8s_clusters",
        };
        let exists_sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = ?)");
        let exists: bool = sqlx::query_scalar(&exists_sql)
            .bind(resource_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("live resource existence"))?;
        if !exists {
            return Err(Error::NotFound(format!("{} resource", kind.as_str())));
        }

        let revision: Option<i64> = sqlx::query_scalar(
            "SELECT revision FROM resource_access_policies
             WHERE resource_kind = ? AND resource_id = ?",
        )
        .bind(kind.as_str())
        .bind(resource_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(dberr("live resource access policy"))?;
        let policy = match revision {
            None => AccessPolicy::legacy(kind, resource_id.clone()),
            Some(revision) => {
                let json: String = sqlx::query_scalar(
                    "SELECT policy_json FROM resource_access_policy_versions
                     WHERE resource_kind = ? AND resource_id = ? AND revision = ?",
                )
                .bind(kind.as_str())
                .bind(resource_id)
                .bind(revision)
                .fetch_one(&mut *tx)
                .await
                .map_err(dberr("live resource access policy version"))?;
                serde_json::from_str(&json).map_err(|error| {
                    Error::Internal(format!("decode live resource access policy: {error}"))
                })?
            }
        };
        tx.commit()
            .await
            .map_err(dberr("live resource policy commit"))?;
        Ok(policy)
    }

    pub async fn get_policy_version(
        &self,
        kind: ResourceKind,
        resource_id: &Id,
        revision: i64,
    ) -> Result<AccessPolicy> {
        let json: String = sqlx::query_scalar(
            "SELECT policy_json FROM resource_access_policy_versions
             WHERE resource_kind = ? AND resource_id = ? AND revision = ?",
        )
        .bind(kind.as_str())
        .bind(resource_id)
        .bind(revision)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("resource access policy version"))?;
        serde_json::from_str(&json)
            .map_err(|error| Error::Internal(format!("decode resource access policy: {error}")))
    }

    /// Compare-and-swap the policy head, preserving an immutable version and
    /// audit entry atomically. `expected_revision = 0` creates the first row.
    pub async fn put_policy(
        &self,
        policy: &AccessPolicy,
        expected_revision: i64,
        actor: &AccessActor,
    ) -> Result<AccessPolicy> {
        if expected_revision < 0 {
            return Err(Error::Invalid(
                "expected revision cannot be negative".into(),
            ));
        }
        validate_policy(policy)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("policy update begin"))?;
        validate_policy_references(&mut tx, policy).await?;

        let next_revision = expected_revision + 1;
        let now = fmt(Utc::now());
        let changed = if expected_revision == 0 {
            sqlx::query(
                "INSERT INTO resource_access_policies
                 (resource_kind, resource_id, mode, revision, updated_at)
                 VALUES (?, ?, ?, 1, ?) ON CONFLICT(resource_kind, resource_id) DO NOTHING",
            )
            .bind(policy.kind.as_str())
            .bind(&policy.resource_id)
            .bind(mode_str(policy.mode))
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(dberr("create resource access policy"))?
            .rows_affected()
        } else {
            sqlx::query(
                "UPDATE resource_access_policies SET mode = ?, revision = ?, updated_at = ?
                 WHERE resource_kind = ? AND resource_id = ? AND revision = ?",
            )
            .bind(mode_str(policy.mode))
            .bind(next_revision)
            .bind(&now)
            .bind(policy.kind.as_str())
            .bind(&policy.resource_id)
            .bind(expected_revision)
            .execute(&mut *tx)
            .await
            .map_err(dberr("update resource access policy"))?
            .rows_affected()
        };
        if changed == 0 {
            return Err(Error::Conflict(format!(
                "resource access policy revision {expected_revision} is stale"
            )));
        }

        let mut stored = policy.clone();
        stored.revision = next_revision;
        let policy_json = serde_json::to_string(&stored)
            .map_err(|error| Error::Internal(format!("encode resource access policy: {error}")))?;
        sqlx::query(
            "INSERT INTO resource_access_policy_versions
             (resource_kind, resource_id, revision, policy_json, actor_user_id,
              effective_user_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(stored.kind.as_str())
        .bind(&stored.resource_id)
        .bind(stored.revision)
        .bind(&policy_json)
        .bind(&actor.real_user_id)
        .bind(&actor.effective_user_id)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(dberr("insert resource access policy version"))?;
        append_audit(
            &mut tx,
            actor,
            "resource_access.policy_update",
            &format!("{}:{}", stored.kind.as_str(), stored.resource_id),
            serde_json::json!({
                "previous_revision": expected_revision,
                "revision": stored.revision,
                "mode": stored.mode,
                "policy": stored,
            }),
        )
        .await?;
        tx.commit().await.map_err(dberr("policy update commit"))?;
        Ok(stored)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn initialize_owner_policy(
        &self,
        kind: ResourceKind,
        resource_id: &Id,
        owner_user_id: &Id,
        operations: &[String],
        grantable_operations: &[String],
        actor: &AccessActor,
    ) -> Result<AccessPolicy> {
        let current = self.get_policy(kind, resource_id).await?;
        let expected_revision = match (current.mode, current.revision, current.rules.is_empty()) {
            (AccessMode::Legacy, 0, true) => 0,
            (AccessMode::Enforced, 1, true) => 1,
            _ => {
                return Err(Error::Conflict(
                    "resource access policy is already initialized".into(),
                ))
            }
        };
        let policy = AccessPolicy {
            kind,
            resource_id: resource_id.clone(),
            mode: AccessMode::Enforced,
            revision: expected_revision,
            rules: vec![AccessRule {
                id: new_id(),
                subject_kind: SubjectKind::User,
                subject_id: owner_user_id.clone(),
                effect: RuleEffect::Allow,
                operations: operations.to_vec(),
                children: None,
                grantable_operations: grantable_operations.to_vec(),
                credential_connection_id: None,
            }],
        };
        self.put_policy(&policy, expected_revision, actor).await
    }
}

fn required_name<'a>(name: &'a str, entity: &str) -> Result<&'a str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Err(Error::Invalid(format!("{entity} name cannot be empty")))
    } else {
        Ok(trimmed)
    }
}

fn group_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AccessGroup> {
    Ok(AccessGroup {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        created_at: ts(&row.get::<String, _>("created_at"))?,
        updated_at: ts(&row.get::<String, _>("updated_at"))?,
    })
}

fn role_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AccessRole> {
    let kind_text: String = row.get("resource_kind");
    let kind = ResourceKind::parse(&kind_text)
        .ok_or_else(|| Error::Internal(format!("invalid resource kind '{kind_text}'")))?;
    Ok(AccessRole {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        kind,
        operations: parse_strings(&row.get::<String, _>("operations_json"), "role operations")?,
        grantable_operations: parse_strings(
            &row.get::<String, _>("grantable_operations_json"),
            "role grantable operations",
        )?,
        created_at: ts(&row.get::<String, _>("created_at"))?,
        updated_at: ts(&row.get::<String, _>("updated_at"))?,
    })
}

fn validate_role(
    name: &str,
    kind: ResourceKind,
    operations: &[String],
    grantable_operations: &[String],
) -> Result<()> {
    required_name(name, "role")?;
    if operations.is_empty() {
        return Err(Error::Invalid(
            "role must select at least one operation".into(),
        ));
    }
    for operation in operations.iter().chain(grantable_operations.iter()) {
        validate_operation(kind, operation)?;
    }
    if grantable_operations
        .iter()
        .any(|operation| !operations.contains(operation))
    {
        return Err(Error::Invalid(
            "role grants delegation beyond its allowed operations".into(),
        ));
    }
    Ok(())
}

fn json_strings(values: &[String], context: &str) -> Result<String> {
    serde_json::to_string(values)
        .map_err(|error| Error::Internal(format!("encode {context}: {error}")))
}

fn parse_strings(value: &str, context: &str) -> Result<Vec<String>> {
    serde_json::from_str(value)
        .map_err(|error| Error::Internal(format!("decode {context}: {error}")))
}

fn mode_str(mode: AccessMode) -> &'static str {
    match mode {
        AccessMode::Legacy => "legacy",
        AccessMode::Enforced => "enforced",
    }
}

async fn validate_policy_references(
    tx: &mut Transaction<'_, Sqlite>,
    policy: &AccessPolicy,
) -> Result<()> {
    for rule in &policy.rules {
        let (table, context) = match rule.subject_kind {
            SubjectKind::User => ("users", "user"),
            SubjectKind::Group => ("access_groups", "access group"),
        };
        ensure_exists(tx, table, &rule.subject_id, context).await?;
        if let Some(connection_id) = &rule.credential_connection_id {
            ensure_exists(tx, "connections", connection_id, "credential connection").await?;
        }
    }
    Ok(())
}

async fn ensure_exists(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
    id: &Id,
    context: &str,
) -> Result<()> {
    let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = ?)");
    let exists: bool = sqlx::query_scalar(&sql)
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map_err(dberr("validate access reference"))?;
    if exists {
        Ok(())
    } else {
        Err(Error::Invalid(format!("{context} '{id}' does not exist")))
    }
}

async fn ensure_exists_pool(pool: &SqlitePool, table: &str, id: &Id, context: &str) -> Result<()> {
    let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = ?)");
    let exists: bool = sqlx::query_scalar(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(dberr("validate access reference"))?;
    if exists {
        Ok(())
    } else {
        Err(Error::NotFound(context.into()))
    }
}

async fn append_audit(
    tx: &mut Transaction<'_, Sqlite>,
    actor: &AccessActor,
    action: &str,
    target: &str,
    mut detail: serde_json::Value,
) -> Result<()> {
    if let serde_json::Value::Object(object) = &mut detail {
        object.insert(
            "effective_user_id".into(),
            serde_json::to_value(&actor.effective_user_id)
                .map_err(|error| Error::Internal(format!("encode audit identity: {error}")))?,
        );
    }
    let detail = serde_json::to_string(&detail)
        .map_err(|error| Error::Internal(format!("encode resource access audit: {error}")))?;
    sqlx::query(
        "INSERT INTO audit_log (id, ts, user_id, action, target, detail, ip)
         VALUES (?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(new_id())
    .bind(fmt(Utc::now()))
    .bind(&actor.real_user_id)
    .bind(action)
    .bind(target)
    .bind(detail)
    .execute(&mut **tx)
    .await
    .map_err(dberr("insert resource access audit"))?;
    Ok(())
}
