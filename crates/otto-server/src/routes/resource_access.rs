//! Per-resource access management. Resource checks supplement feature/workspace
//! membership; group/role administration is restricted to the instance owner.
use otto_core::access::{AccessPolicy, AccessRule, RuleEffect, SubjectKind};
use otto_core::domain::User;
use otto_core::{Error, Id, Result};

/// Validate a delegated edit against the caller's existing grantable ceiling.
/// The persisted old policy, never the submitted policy, determines authority.
fn applies(rule: &AccessRule, user: &User, groups: &[Id]) -> bool {
    match rule.subject_kind {
        SubjectKind::User => rule.subject_id == user.id,
        SubjectKind::Group => groups.contains(&rule.subject_id),
    }
}
fn covers(rule: &AccessRule, child: Option<&str>) -> bool {
    match (&rule.children, child) {
        (None, _) => true,
        (Some(children), Some(child)) => children.iter().any(|c| c == child),
        _ => false,
    }
}
fn validate_delegation(
    user: &User,
    groups: &[Id],
    old: &AccessPolicy,
    new: &AccessPolicy,
) -> Result<()> {
    if user.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    if user.is_root {
        return Ok(());
    }
    let deny = || Error::Forbidden("change exceeds your delegated access authority".into());
    if old.mode != new.mode || old.kind != new.kind || old.resource_id != new.resource_id {
        return Err(deny());
    }
    let changed: Vec<&AccessRule> = old
        .rules
        .iter()
        .filter(|r| !new.rules.contains(r))
        .chain(new.rules.iter().filter(|r| !old.rules.contains(r)))
        .collect();
    for rule in changed {
        // A caller cannot grant itself privileges through a direct rule or
        // through any of its groups, nor manufacture further delegation rights.
        if applies(rule, user, groups) || !rule.grantable_operations.is_empty() {
            return Err(deny());
        }
        if old.rules.contains(rule) && rule.effect == RuleEffect::Deny {
            return Err(deny());
        }
        if let Some(profile) = &rule.credential_connection_id {
            if !old
                .rules
                .iter()
                .any(|r| r.id == rule.id && r.credential_connection_id.as_ref() == Some(profile))
            {
                return Err(deny());
            }
        }
        let scopes: Vec<Option<&str>> = match &rule.children {
            None => vec![None],
            Some(children) => children.iter().map(|s| Some(s.as_str())).collect(),
        };
        for child in scopes {
            for op in &rule.operations {
                let authority = old.rules.iter().any(|r| {
                    applies(r, user, groups)
                        && covers(r, child)
                        && r.effect == RuleEffect::Allow
                        && r.grantable_operations.contains(op)
                });
                let restricted = old.rules.iter().any(|r| {
                    applies(r, user, groups)
                        && r.effect == RuleEffect::Deny
                        && (covers(r, child) || child.is_none())
                        && (r.operations.contains(op)
                            || r.operations
                                .iter()
                                .any(|o| o == "manage_access" || o == "discover"))
                });
                if !authority || restricted {
                    return Err(deny());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "resource_access_tests.rs"]
mod tests;

use super::access_groups;
use crate::auth::{require_root, CurrentAuthContext};
use crate::error::ApiResult;
use crate::state::ServerCtx;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use otto_core::access::{
    operations_for, AccessActor, AccessDecision, AccessMode, ResourceKind, ResourceRef,
};
use otto_core::auth::AuthContext;
use otto_core::domain::{Capability, Feature, WorkspaceRole};
use otto_rbac::resource_access::ResourceAccess;
use otto_state::resource_access::ResourceAccessRepo;
use otto_state::{
    AwsAccountsRepo, ConnectionsRepo, GrantsRepo, K8sClustersRepo, McpRegistryRepo, SqlitePool,
    UsersRepo, WorkspacesRepo,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub trait AccessCtx: Clone + Send + Sync + 'static {
    fn access_pool(&self) -> SqlitePool;
    fn access_db(&self) -> Option<std::sync::Arc<otto_dbviewer::DbViewerService>> {
        None
    }
    fn retire_mcp<'a>(
        &'a self,
        _id: &'a Id,
        _apply: bool,
    ) -> otto_core::auth::BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
}
impl AccessCtx for ServerCtx {
    fn access_pool(&self) -> SqlitePool {
        self.pool.clone()
    }
    fn access_db(&self) -> Option<std::sync::Arc<otto_dbviewer::DbViewerService>> {
        Some(self.db_explorer.clone())
    }
    fn retire_mcp<'a>(
        &'a self,
        id: &'a Id,
        apply: bool,
    ) -> otto_core::auth::BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let server = self.mcp.registry().get(id).await?;
            let sessions = self.manager.list_by_workspace(&server.workspace_id).await?;
            let mut roots =
                BTreeSet::from([self.workspaces.get(&server.workspace_id).await?.root_path]);
            for session in &sessions {
                roots.insert(session.cwd.clone());
                if self.manager.is_live(&session.id) {
                    if !apply {
                        return Err(Error::Conflict("stop active workspace sessions before switching a direct MCP server to governed access".into()));
                    }
                    // A session raced the preview/save. Stop the direct client
                    // before completing activation, then retire its launcher.
                    self.manager.kill_session(&session.id).await?;
                }
            }
            for root in roots {
                otto_sessions::mcp::retire_user_server(&root, &server.name, apply)
                    .map_err(Error::Conflict)?;
            }
            Ok(())
        })
    }
}

pub(super) fn actor(auth: &AuthContext) -> AccessActor {
    AccessActor {
        real_user_id: auth.real_user.id.clone(),
        effective_user_id: (auth.real_user.id != auth.effective_user.id)
            .then(|| auth.effective_user.id.clone()),
    }
}

pub fn api_router<S: AccessCtx>() -> Router<S> {
    Router::new()
        .route(
            "/access/groups",
            get(access_groups::list_groups::<S>).post(access_groups::create_group::<S>),
        )
        .route(
            "/access/groups/{id}",
            axum::routing::put(access_groups::update_group::<S>)
                .delete(access_groups::delete_group::<S>),
        )
        .route(
            "/access/groups/{id}/members",
            get(access_groups::members::<S>),
        )
        .route(
            "/access/groups/{id}/members/{uid}",
            axum::routing::put(access_groups::add_member::<S>)
                .delete(access_groups::remove_member::<S>),
        )
        .route(
            "/access/roles",
            get(access_groups::list_roles::<S>).post(access_groups::create_role::<S>),
        )
        .route(
            "/access/roles/{id}",
            axum::routing::put(access_groups::update_role::<S>)
                .delete(access_groups::delete_role::<S>),
        )
        .route(
            "/access/{kind}/{id}",
            get(get_policy::<S>).put(put_policy::<S>),
        )
        .route("/access/{kind}/{id}/subjects", get(subjects::<S>))
        .route("/access/{kind}/{id}/capabilities", get(capabilities::<S>))
        .route("/access/{kind}/{id}/effective", get(effective::<S>))
        .route("/access/{kind}/{id}/preview", post(preview::<S>))
}

/// Resolve the resource before authorizing so IDs cannot cross resource families.
async fn workspace(pool: &SqlitePool, kind: ResourceKind, id: &Id) -> Result<Option<Id>> {
    match kind {
        ResourceKind::Connection => Ok(ConnectionsRepo::new(pool.clone())
            .get(id)
            .await?
            .workspace_id),
        ResourceKind::McpServer => Ok(Some(
            McpRegistryRepo::new(pool.clone())
                .get(id)
                .await?
                .workspace_id,
        )),
        ResourceKind::AwsAccount => {
            AwsAccountsRepo::new(pool.clone()).get(id).await?;
            Ok(None)
        }
        ResourceKind::K8sCluster => {
            K8sClustersRepo::new(pool.clone()).get(id).await?;
            Ok(None)
        }
    }
}

fn feature_for(kind: ResourceKind, operation: &str) -> Feature {
    match kind {
        ResourceKind::Connection
            if operation.starts_with("db_") || operation.starts_with("change_") =>
        {
            Feature::Database
        }
        ResourceKind::Connection => Feature::Connections,
        ResourceKind::McpServer => Feature::Mcp,
        ResourceKind::K8sCluster => Feature::Kubernetes,
        ResourceKind::AwsAccount => match operation.split('_').next().unwrap_or("") {
            "s3" => Feature::AwsS3,
            "sqs" => Feature::AwsSqs,
            "ec2" => Feature::AwsEc2,
            "athena" => Feature::AwsAthena,
            "eks" => Feature::AwsEks,
            "rds" => Feature::AwsRds,
            _ => Feature::Aws,
        },
    }
}

async fn page_access(
    pool: &SqlitePool,
    user: &User,
    kind: ResourceKind,
    id: &Id,
    operation: &str,
) -> Result<()> {
    let ws = workspace(pool, kind, id).await?;
    if user.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    let grants = GrantsRepo::new(pool.clone());
    grants
        .check_global(
            user,
            feature_for(kind, operation),
            Capability::View,
            "page access is disabled",
        )
        .await?;
    if kind == ResourceKind::AwsAccount {
        grants
            .check_global(
                user,
                Feature::Aws,
                Capability::View,
                "AWS page access is disabled",
            )
            .await?;
    }
    if let Some(ws) = ws {
        if !WorkspacesRepo::new(pool.clone())
            .role_of(user, &ws)
            .await?
            .is_some_and(|r| r >= WorkspaceRole::Viewer)
        {
            return Err(Error::NotFound("resource".into()));
        }
    }
    Ok(())
}

async fn management(
    pool: &SqlitePool,
    user: &User,
    kind: ResourceKind,
    id: &Id,
) -> Result<AccessPolicy> {
    page_access(pool, user, kind, id, "manage_access").await?;
    let policy = ResourceAccessRepo::new(pool.clone())
        .get_policy(kind, id)
        .await?;
    if !user.is_root {
        if policy.mode == AccessMode::Legacy {
            return Err(Error::Forbidden("root must configure legacy access".into()));
        }
        let access = ResourceAccess::new(pool.clone());
        let resource = ResourceRef {
            kind,
            id: id.clone(),
            child: None,
        };
        if !access.evaluate(user, &resource, "discover").await?.allowed {
            return Err(Error::NotFound("resource".into()));
        }
        access.check(user, &resource, "manage_access").await?;
    }
    Ok(policy)
}

async fn get_policy<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((kind, id)): Path<(ResourceKind, Id)>,
) -> ApiResult<Json<AccessPolicy>> {
    Ok(Json(
        management(&ctx.access_pool(), &auth.effective_user, kind, &id).await?,
    ))
}

#[derive(Deserialize)]
pub struct PolicyInput {
    pub policy: AccessPolicy,
    pub preview_token: Option<String>,
}

async fn put_policy<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((kind, id)): Path<(ResourceKind, Id)>,
    Json(req): Json<PolicyInput>,
) -> ApiResult<Json<AccessPolicy>> {
    let _mcp_activation = if kind == ResourceKind::McpServer {
        Some(otto_sessions::mcp::activation_gate().write().await)
    } else {
        None
    };
    let pool = ctx.access_pool();
    let old = management(&pool, &auth.effective_user, kind, &id).await?;
    if req.policy.kind != kind
        || req.policy.resource_id != id
        || req.policy.revision != old.revision
    {
        return Err(Error::Conflict("access changed; reload before saving".into()).into());
    }
    otto_core::access::validate_policy(&req.policy)?;
    validate_resource_policy(&req.policy)?;
    let repo = ResourceAccessRepo::new(pool.clone());
    let groups = repo.groups_for_user(&auth.effective_user.id).await?;
    validate_delegation(&auth.effective_user, &groups, &old, &req.policy)?;
    if old.mode != req.policy.mode {
        require_root(&auth.effective_user)?;
        let preview = build_preview(&ctx, &auth.effective_user, &old, &req.policy).await?;
        if req.preview_token.as_deref() != Some(preview.token.as_str()) {
            return Err(Error::Conflict(
                "review the current access preview before changing enforcement".into(),
            )
            .into());
        }
        if !preview.issues.is_empty() {
            return Err(Error::Invalid(preview.issues.join("; ")).into());
        }
    }
    // Validate profiles even for already-enforced policies: delegation cannot
    // silently swap to a stronger credential through an ordinary rule edit.
    for rule in &req.policy.rules {
        if let Some(profile) = &rule.credential_connection_id {
            if !auth.effective_user.is_root
                && old
                    .rules
                    .iter()
                    .find(|r| r.id == rule.id)
                    .and_then(|r| r.credential_connection_id.as_ref())
                    != Some(profile)
            {
                return Err(
                    Error::Forbidden("only root can assign execution credentials".into()).into(),
                );
            }
            ConnectionsRepo::new(pool.clone()).get(profile).await?;
        }
    }
    if kind == ResourceKind::Connection
        && req.policy.mode == AccessMode::Enforced
        && auth.effective_user.is_root
    {
        if let Some(db) = ctx.access_db() {
            db.validate_access_policy(&req.policy, &auth.effective_user.id)
                .await?;
        }
    }
    if kind == ResourceKind::McpServer
        && old.mode == AccessMode::Legacy
        && req.policy.mode == AccessMode::Enforced
    {
        // Cleanup failure leaves the policy in legacy mode. The exclusive
        // activation guard prevents stale pending launches on every provider.
        ctx.retire_mcp(&id, true).await?;
    }
    let policy = repo
        .put_policy(&req.policy, old.revision, &actor(&auth))
        .await?;
    Ok(Json(policy))
}

#[derive(Default, Deserialize)]
pub struct EffectiveQuery {
    pub user_id: Option<Id>,
    pub child: Option<String>,
}
#[derive(Serialize)]
pub struct EffectiveAccess {
    pub kind: ResourceKind,
    pub resource_id: Id,
    pub user_id: Id,
    pub child: Option<String>,
    pub mode: AccessMode,
    pub operations: BTreeMap<String, AccessDecision>,
}

async fn decisions(
    pool: &SqlitePool,
    user: &User,
    policy: &AccessPolicy,
    child: Option<String>,
) -> Result<EffectiveAccess> {
    let resource = ResourceRef {
        kind: policy.kind,
        id: policy.resource_id.clone(),
        child: child.clone(),
    };
    let access = ResourceAccess::new(pool.clone());
    let mut operations = BTreeMap::new();
    for op in operations_for(policy.kind) {
        let mut decision = access.preview(user, policy, &resource, op).await?;
        if let Err(e) = page_access(pool, user, policy.kind, &policy.resource_id, op).await {
            decision.allowed = false;
            decision.reason = e.to_string();
        }
        // Legacy authorization still uses feature tiers at the execution route.
        if policy.mode == AccessMode::Legacy && !user.is_root {
            let cap = GrantsRepo::new(pool.clone())
                .capability_of(user, feature_for(policy.kind, op))
                .await?;
            let need = legacy_capability(op);
            if cap < need {
                decision.allowed = false;
                decision.reason = format!("legacy access requires {}", need.as_str());
            }
        }
        operations.insert((*op).to_string(), decision);
    }
    Ok(EffectiveAccess {
        kind: policy.kind,
        resource_id: policy.resource_id.clone(),
        user_id: user.id.clone(),
        child,
        mode: policy.mode,
        operations,
    })
}
fn legacy_capability(op: &str) -> Capability {
    if matches!(
        op,
        "manage_access" | "configure" | "approve" | "change_approve"
    ) {
        Capability::Admin
    } else if matches!(
        op,
        "discover"
            | "db_browse"
            | "logs"
            | "metrics"
            | "resources_view"
            | "workloads_view"
            | "secrets_view"
            | "s3_buckets"
            | "s3_list"
            | "s3_read"
            | "sftp_read"
    ) || op.ends_with("_view")
    {
        Capability::View
    } else {
        Capability::Edit
    }
}

async fn capabilities<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((kind, id)): Path<(ResourceKind, Id)>,
    Query(q): Query<EffectiveQuery>,
) -> ApiResult<Json<EffectiveAccess>> {
    let pool = ctx.access_pool();
    let user = &auth.effective_user;
    page_access(&pool, user, kind, &id, "discover").await?;
    let policy = ResourceAccessRepo::new(pool.clone())
        .get_policy(kind, &id)
        .await?;
    let resource = ResourceRef {
        kind,
        id: id.clone(),
        child: None,
    };
    if !ResourceAccess::new(pool.clone())
        .evaluate(user, &resource, "discover")
        .await?
        .allowed
    {
        return Err(Error::NotFound("resource".into()).into());
    }
    let mut response = decisions(&pool, user, &policy, q.child).await?;
    // Self view does not disclose identities or IDs of administrative rules.
    for decision in response.operations.values_mut() {
        decision.matched_rule_ids.clear();
    }
    Ok(Json(response))
}
async fn effective<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((kind, id)): Path<(ResourceKind, Id)>,
    Query(q): Query<EffectiveQuery>,
) -> ApiResult<Json<EffectiveAccess>> {
    let pool = ctx.access_pool();
    let policy = management(&pool, &auth.effective_user, kind, &id).await?;
    let target = UsersRepo::new(pool.clone())
        .get(&q.user_id.unwrap_or_else(|| auth.effective_user.id.clone()))
        .await?;
    Ok(Json(decisions(&pool, &target, &policy, q.child).await?))
}
async fn subjects<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((kind, id)): Path<(ResourceKind, Id)>,
) -> ApiResult<Json<Value>> {
    let pool = ctx.access_pool();
    management(&pool, &auth.effective_user, kind, &id).await?;
    let users: Vec<Value> = UsersRepo::new(pool.clone())
        .list()
        .await?
        .into_iter()
        .filter(|u| !u.disabled)
        .map(|u| json!({"id":u.id,"display_name":u.display_name,"username":u.username}))
        .collect();
    let repo = ResourceAccessRepo::new(pool);
    Ok(Json(
        json!({"users":users,"groups":repo.list_groups().await?,"roles":repo.list_roles().await?}),
    ))
}
#[derive(Serialize)]
pub struct AccessPreview {
    pub token: String,
    pub revision: i64,
    pub changes: Vec<Value>,
    pub issues: Vec<String>,
}
fn validate_resource_policy(policy: &AccessPolicy) -> Result<()> {
    match policy.kind {
        ResourceKind::AwsAccount => otto_aws::access::validate_policy(policy),
        ResourceKind::K8sCluster => otto_k8s::access::validate_policy(policy),
        _ => Ok(()),
    }
}
async fn build_preview<S: AccessCtx>(
    ctx: &S,
    actor: &User,
    old: &AccessPolicy,
    new: &AccessPolicy,
) -> Result<AccessPreview> {
    let pool = &ctx.access_pool();
    otto_core::access::validate_policy(new)?;
    let mut issues = validate_resource_policy(new)
        .err()
        .map(|e| vec![e.to_string()])
        .unwrap_or_default();
    if new.kind == ResourceKind::Connection && new.mode == AccessMode::Enforced && actor.is_root {
        if let Some(db) = ctx.access_db() {
            if let Err(e) = db.validate_access_policy(new, &actor.id).await {
                issues.push(e.to_string());
            }
        }
    }
    if new.kind == ResourceKind::McpServer
        && old.mode == AccessMode::Legacy
        && new.mode == AccessMode::Enforced
    {
        if let Err(e) = ctx.retire_mcp(&new.resource_id, false).await {
            issues.push(e.to_string());
        }
    }
    let children: BTreeSet<String> = old
        .rules
        .iter()
        .chain(&new.rules)
        .filter_map(|r| r.children.as_ref())
        .flatten()
        .cloned()
        .collect();
    let mut changes = Vec::new();
    for user in UsersRepo::new(pool.clone()).list().await? {
        let before = decisions(pool, &user, old, None).await?;
        let after = decisions(pool, &user, new, None).await?;
        let mut child_changes = Vec::new();
        for child in &children {
            let before = decisions(pool, &user, old, Some(child.clone())).await?;
            let after = decisions(pool, &user, new, Some(child.clone())).await?;
            child_changes
                .push(json!({"child":child,"before":before.operations,"after":after.operations}));
        }
        changes.push(json!({"user_id":user.id,"display_name":user.display_name,"before":before.operations,"after":after.operations,"children":child_changes}));
    }
    let token = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(old, new, &changes))
                .map_err(|e| Error::Internal(e.to_string()))?
        )
    );
    Ok(AccessPreview {
        token,
        revision: old.revision,
        changes,
        issues,
    })
}
async fn preview<S: AccessCtx>(
    State(ctx): State<S>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((kind, id)): Path<(ResourceKind, Id)>,
    Json(req): Json<PolicyInput>,
) -> ApiResult<Json<AccessPreview>> {
    let pool = ctx.access_pool();
    let old = management(&pool, &auth.effective_user, kind, &id).await?;
    if req.policy.kind != kind
        || req.policy.resource_id != id
        || req.policy.revision != old.revision
    {
        return Err(Error::Conflict("access changed; reload before preview".into()).into());
    }
    let groups = ResourceAccessRepo::new(pool.clone())
        .groups_for_user(&auth.effective_user.id)
        .await?;
    validate_delegation(&auth.effective_user, &groups, &old, &req.policy)?;
    Ok(Json(
        build_preview(&ctx, &auth.effective_user, &old, &req.policy).await?,
    ))
}
