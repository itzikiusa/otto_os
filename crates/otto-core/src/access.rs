//! Shared resource-authorization contracts.
//!
//! Feature, workspace, and token ceilings are separate authorization axes. The
//! types here describe only the resource axis consumed by connection, MCP, AWS,
//! and Kubernetes services.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::BoxFuture;
use crate::domain::User;
use crate::{Error, Id, Result};

/// A top-level resource governed by an [`AccessPolicy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Connection,
    McpServer,
    AwsAccount,
    K8sCluster,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::McpServer => "mcp_server",
            Self::AwsAccount => "aws_account",
            Self::K8sCluster => "k8s_cluster",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "connection" => Some(Self::Connection),
            "mcp_server" => Some(Self::McpServer),
            "aws_account" => Some(Self::AwsAccount),
            "k8s_cluster" => Some(Self::K8sCluster),
            _ => None,
        }
    }
}

/// One resource and, optionally, one opaque stable child identity.
///
/// Examples include `bucket:reports`, `namespace:payments`, a database name, or
/// an MCP tool name. Callers own normalization; authorization uses exact string
/// matching so a named selection never absorbs newly discovered children.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRef {
    pub kind: ResourceKind,
    pub id: Id,
    pub child: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    User,
    Group,
}

impl SubjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Group => "group",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleEffect {
    Allow,
    Deny,
}

/// Rollout mode for one resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    /// Preserve the resource's existing feature/workspace behavior.
    Legacy,
    /// Require an explicit matching resource allow; every deny wins.
    Enforced,
}

/// A user or group rule within one top-level resource policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessRule {
    pub id: Id,
    pub subject_kind: SubjectKind,
    pub subject_id: Id,
    pub effect: RuleEffect,
    pub operations: Vec<String>,
    /// `None` means every current and future child. `Some` is an exact,
    /// non-empty selection and never includes newly discovered children.
    pub children: Option<Vec<String>>,
    /// Delegation ceiling carried by this rule. It does not itself grant the
    /// operation; management code intersects proposed changes with this set.
    pub grantable_operations: Vec<String>,
    /// Optional saved connection profile whose native credentials enforce this
    /// Connection Allow rule's operation/child ceiling. Secrets remain in the
    /// Keychain; this stores only the existing connection profile id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_connection_id: Option<Id>,
}

/// Current version of a resource's policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessPolicy {
    pub kind: ResourceKind,
    pub resource_id: Id,
    pub mode: AccessMode,
    /// Zero identifies the synthetic missing-policy legacy state. Persisted
    /// versions begin at one and increment through compare-and-swap updates.
    pub revision: i64,
    pub rules: Vec<AccessRule>,
}

impl AccessPolicy {
    pub fn legacy(kind: ResourceKind, resource_id: Id) -> Self {
        Self {
            kind,
            resource_id,
            mode: AccessMode::Legacy,
            revision: 0,
            rules: Vec::new(),
        }
    }
}

/// Resource-axis authorization result. Higher-level feature/workspace/token
/// checks may still narrow an allowed decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessDecision {
    pub allowed: bool,
    pub reason: String,
    pub matched_rule_ids: Vec<Id>,
    pub mode: AccessMode,
}

/// Dependency-inversion boundary consumed by feature services. Implementations
/// evaluate only the resource axis; callers retain their existing feature,
/// workspace, token, native privilege, and approval checks.
pub trait ResourceAccessChecker: Send + Sync {
    fn evaluate<'a>(
        &'a self,
        user: &'a User,
        resource: &'a ResourceRef,
        operation: &'a str,
    ) -> BoxFuture<'a, Result<AccessDecision>>;

    fn check<'a>(
        &'a self,
        user: &'a User,
        resource: &'a ResourceRef,
        operation: &'a str,
    ) -> BoxFuture<'a, Result<()>>;
}

/// Audit identity for policy and administration mutations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessActor {
    pub real_user_id: Id,
    /// Present when the request is acting as a different effective user.
    pub effective_user_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessGroup {
    pub id: Id,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Named operation preset. Rules copy these operations when assigned, so a
/// later role edit cannot silently widen an existing policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessRole {
    pub id: Id,
    pub name: String,
    pub description: Option<String>,
    pub kind: ResourceKind,
    pub operations: Vec<String>,
    pub grantable_operations: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const CONNECTION_OPERATIONS: &[&str] = &[
    "discover",
    "db_browse",
    "db_query",
    "db_export",
    "db_data",
    "db_schema",
    "configure",
    "manage_access",
    "change_submit",
    "change_approve",
    "change_execute",
    "shell",
    "sftp_read",
    "sftp_write",
];

const MCP_OPERATIONS: &[&str] = &[
    "discover",
    "invoke",
    "configure",
    "manage_access",
    "approve",
];

const AWS_OPERATIONS: &[&str] = &[
    "discover",
    "configure",
    "manage_access",
    "s3_list",
    "s3_read",
    "s3_write",
    "s3_delete",
    "s3_buckets",
    "ec2_view",
    "ec2_start",
    "ec2_stop",
    "ec2_reboot",
    "ec2_terminate",
    "sqs_view",
    "sqs_send",
    "sqs_receive",
    "sqs_delete",
    "sqs_purge",
    "sqs_redrive",
    "athena_view",
    "athena_query",
    "eks_view",
    "eks_import",
    "rds_view",
    "metrics",
];

const K8S_OPERATIONS: &[&str] = &[
    "discover",
    "configure",
    "manage_access",
    "workloads_view",
    "resources_view",
    "secrets_view",
    "logs",
    "metrics",
    "exec",
    "k9s",
    "apply",
    "scale",
    "restart",
    "delete",
];

/// Closed operation catalogue for a resource family.
pub fn operations_for(kind: ResourceKind) -> &'static [&'static str] {
    match kind {
        ResourceKind::Connection => CONNECTION_OPERATIONS,
        ResourceKind::McpServer => MCP_OPERATIONS,
        ResourceKind::AwsAccount => AWS_OPERATIONS,
        ResourceKind::K8sCluster => K8S_OPERATIONS,
    }
}

pub fn validate_operation(kind: ResourceKind, operation: &str) -> Result<()> {
    if operations_for(kind).contains(&operation) {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "unknown {} operation '{operation}'",
            kind.as_str()
        )))
    }
}

/// Validate the closed vocabulary and structural invariants of a policy.
pub fn validate_policy(policy: &AccessPolicy) -> Result<()> {
    use std::collections::HashSet;

    if policy.resource_id.trim().is_empty() {
        return Err(Error::Invalid("resource id cannot be empty".into()));
    }

    let mut rule_ids = HashSet::new();
    for rule in &policy.rules {
        if rule.id.trim().is_empty() || !rule_ids.insert(rule.id.as_str()) {
            return Err(Error::Invalid(format!(
                "duplicate or empty access rule id '{}'",
                rule.id
            )));
        }
        if rule.subject_id.trim().is_empty() {
            return Err(Error::Invalid(format!(
                "rule '{}' has an empty subject id",
                rule.id
            )));
        }
        if rule.operations.is_empty() {
            return Err(Error::Invalid(format!(
                "rule '{}' must select at least one operation",
                rule.id
            )));
        }
        for operation in rule
            .operations
            .iter()
            .chain(rule.grantable_operations.iter())
        {
            validate_operation(policy.kind, operation)?;
        }
        if rule.credential_connection_id.is_some()
            && (policy.kind != ResourceKind::Connection || rule.effect != RuleEffect::Allow)
        {
            return Err(Error::Invalid(format!(
                "rule '{}' may use credential_connection_id only for a connection allow",
                rule.id
            )));
        }
        if rule
            .grantable_operations
            .iter()
            .any(|operation| !rule.operations.contains(operation))
        {
            return Err(Error::Invalid(format!(
                "rule '{}' grants delegation beyond its allowed operations",
                rule.id
            )));
        }
        if rule.effect == RuleEffect::Deny && !rule.grantable_operations.is_empty() {
            return Err(Error::Invalid(format!(
                "deny rule '{}' cannot delegate operations",
                rule.id
            )));
        }
        if let Some(children) = &rule.children {
            if children.is_empty() || children.iter().any(|child| child.trim().is_empty()) {
                return Err(Error::Invalid(format!(
                    "rule '{}' must select at least one non-empty child",
                    rule.id
                )));
            }
            let mut unique = HashSet::new();
            if children.iter().any(|child| !unique.insert(child.as_str())) {
                return Err(Error::Invalid(format!(
                    "rule '{}' contains duplicate children",
                    rule.id
                )));
            }
        }
    }
    Ok(())
}
