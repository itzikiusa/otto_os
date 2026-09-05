//! Reviewed database changes: authorization and preflight around the durable
//! state machine. Only the persisted claimed artifact reaches the DB adapter.
use crate::state::ServerCtx;
use otto_core::access::{AccessMode, ResourceKind, ResourceRef};
use otto_core::auth::AuthContext;
use otto_core::domain::{Capability, ConnectionKind, Feature, User, WorkspaceRole};
use otto_core::{Error, Result};
use otto_rbac::resource_access::ResourceAccess;
use otto_state::database_changes::{
    ChangeInput, ChangeTarget, DatabaseChange, DatabaseChangesRepo, TargetSnapshot,
};
use otto_state::{ConnectionsRepo, GrantsRepo, ResourceAccessRepo, UsersRepo, WorkspacesRepo};

pub fn token_gate(auth: &AuthContext) -> Result<()> {
    // MCP tools do not expose this workflow yet. Do not let a scoped MCP or
    // session token use the REST route as an alternate approval channel.
    if auth.scope.is_some() || auth.mcp_only || auth.mcp_scope.is_some() || auth.mcp_internal {
        return Err(Error::Forbidden(
            "database review requires an unrestricted interactive or API identity".into(),
        ));
    }
    Ok(())
}
pub fn normalize(mut input: ChangeInput) -> Result<ChangeInput> {
    input.title = input.title.trim().to_owned();
    if input.title.is_empty()
        || input.title.len() > 200
        || input.description.len() > 16_384
        || input.script.trim().is_empty()
        || input.script.len() > 262_144
    {
        return Err(Error::Invalid("provide a title (1–200 characters), script (up to 256 KiB), and description (up to 16 KiB)".into()));
    }
    if input.targets.is_empty() || input.targets.len() > 20 {
        return Err(Error::Invalid(
            "select between 1 and 20 database targets".into(),
        ));
    }
    for target in &mut input.targets {
        let node = target
            .node
            .trim()
            .strip_prefix("db:")
            .unwrap_or(target.node.trim());
        if target.connection_id.is_empty()
            || node.is_empty()
            || node.len() > 128
            || node.contains('/')
            || node.contains(':')
            || node.chars().any(char::is_control)
        {
            return Err(Error::Invalid(
                "each target must identify one database, without table or schema paths".into(),
            ));
        }
        target.node = format!("db:{node}");
    }
    input
        .targets
        .sort_by(|a, b| (&a.connection_id, &a.node).cmp(&(&b.connection_id, &b.node)));
    if input.targets.windows(2).any(|p| p[0] == p[1]) {
        return Err(Error::Invalid("duplicate database target".into()));
    }
    Ok(input)
}
fn child(target: &ChangeTarget) -> &str {
    target.node.strip_prefix("db:").unwrap_or(&target.node)
}
/// Live feature, membership and deny-wins checks for every target.
pub async fn authorize(
    ctx: &ServerCtx,
    user: &User,
    targets: &[ChangeTarget],
    op: &str,
) -> Result<()> {
    authorize_targets(&ctx.pool, user, targets, op).await
}
async fn authorize_targets(
    pool: &otto_state::SqlitePool,
    user: &User,
    targets: &[ChangeTarget],
    op: &str,
) -> Result<()> {
    let user = UsersRepo::new(pool.clone()).get(&user.id).await?;
    if user.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    let grants = GrantsRepo::new(pool.clone());
    grants
        .check_global(
            &user,
            Feature::Database,
            Capability::View,
            "Database page access required",
        )
        .await?;
    let access = ResourceAccess::new(pool.clone());
    for target in targets {
        let conn = ConnectionsRepo::new(pool.clone())
            .get(&target.connection_id)
            .await?;
        if !matches!(conn.kind, ConnectionKind::Mysql | ConnectionKind::Postgres) {
            return Err(Error::Invalid(
                "reviewed changes support MySQL and PostgreSQL connections".into(),
            ));
        }
        let mode = ResourceAccessRepo::new(pool.clone())
            .get_policy(ResourceKind::Connection, &conn.id)
            .await?
            .mode;
        let mut min_role = WorkspaceRole::Viewer;
        if mode == AccessMode::Legacy {
            let min = match op {
                "change_submit" => Capability::Edit,
                "change_approve" | "change_execute" => Capability::Admin,
                _ => Capability::View,
            };
            min_role = match min {
                Capability::Admin => WorkspaceRole::Admin,
                Capability::Edit => WorkspaceRole::Editor,
                _ => WorkspaceRole::Viewer,
            };
            grants
                .check_global(
                    &user,
                    Feature::Database,
                    min,
                    "legacy connection requires the corresponding Database capability",
                )
                .await?;
        }
        if let Some(ws) = &conn.workspace_id {
            let role = WorkspacesRepo::new(pool.clone()).role_of(&user, ws).await?;
            if !role.is_some_and(|r| r >= min_role) {
                return Err(Error::NotFound("database change target".into()));
            }
        }
        if mode == AccessMode::Enforced {
            let resource = ResourceRef {
                kind: ResourceKind::Connection,
                id: conn.id.clone(),
                child: Some(child(target).to_owned()),
            };
            if !access.evaluate(&user, &resource, "discover").await?.allowed {
                return Err(Error::NotFound("database change target".into()));
            }
            access.check(&user, &resource, op).await?;
        }
    }
    Ok(())
}
pub async fn visible(ctx: &ServerCtx, user: &User, change: &DatabaseChange) -> bool {
    for target in &change.targets {
        let mut allowed = false;
        for op in ["change_submit", "change_approve", "change_execute"] {
            if authorize(ctx, user, std::slice::from_ref(target), op)
                .await
                .is_ok()
            {
                allowed = true;
                break;
            }
        }
        if !allowed {
            return false;
        }
    }
    !change.targets.is_empty()
}
pub async fn load_visible(ctx: &ServerCtx, auth: &AuthContext, id: &str) -> Result<DatabaseChange> {
    token_gate(auth)?;
    let change = DatabaseChangesRepo::new(ctx.pool.clone()).get(id).await?;
    if !visible(ctx, &auth.effective_user, &change).await {
        return Err(Error::NotFound("database change".into()));
    }
    Ok(change)
}
pub fn author(change: &DatabaseChange, auth: &AuthContext) -> Result<()> {
    if auth.effective_user.id != change.author_id {
        return Err(Error::Forbidden(
            "only the author can revise, validate, or submit this change".into(),
        ));
    }
    Ok(())
}
pub async fn snapshots(
    ctx: &ServerCtx,
    change: &DatabaseChange,
    executor: &User,
) -> Result<Vec<TargetSnapshot>> {
    authorize(ctx, executor, &change.targets, "change_execute").await?;
    let mut out = Vec::new();
    for target in &change.targets {
        let conn = ConnectionsRepo::new(ctx.pool.clone())
            .get(&target.connection_id)
            .await?;
        if conn.read_only {
            return Err(Error::Forbidden(
                "a read-only connection cannot execute a reviewed change".into(),
            ));
        }
        // Preflight checks the native credential ceiling without running SQL.
        ctx.db_explorer
            .preflight_approved_change(&conn.id, &executor.id, Some(&target.node), &change.script)
            .await?;
        out.push(TargetSnapshot {
            target: target.clone(),
            environment: conn.environment.as_str().into(),
            policy_revision: ResourceAccessRepo::new(ctx.pool.clone())
                .get_policy(ResourceKind::Connection, &conn.id)
                .await?
                .revision,
            connection_fingerprint: ctx
                .db_explorer
                .change_target_fingerprint(&conn.id, &executor.id, Some(&target.node))
                .await?,
        });
    }
    Ok(out)
}
pub async fn fresh_approval(
    ctx: &ServerCtx,
    change: &DatabaseChange,
) -> Result<Vec<TargetSnapshot>> {
    let users = UsersRepo::new(ctx.pool.clone());
    let author = users.get(&change.author_id).await?;
    authorize(ctx, &author, &change.targets, "change_submit").await?;
    let executor = users
        .get(
            change
                .executor_id
                .as_ref()
                .ok_or_else(|| Error::Conflict("validate and select an executor first".into()))?,
        )
        .await?;
    let snapshots = snapshots(ctx, change, &executor).await?;
    if otto_state::database_changes::artifact_hash(change, &executor.id, &snapshots)?
        != change.content_hash
    {
        return Err(Error::Conflict("approval is stale: revise and validate the change against current permissions and credentials".into()));
    }
    Ok(snapshots)
}
/// Batch adapters return Ok even when a later statement failed. Interpret every
/// nested result before deciding that an execution completed successfully.
fn result_progress(result: &otto_dbviewer::types::QueryResult) -> (bool, usize) {
    let mut success = !result.errored;
    let mut completed = usize::from(!result.errored);
    for next in &result.more_results {
        let (ok, n) = result_progress(next);
        success &= ok;
        completed += n;
    }
    (success, completed)
}
/// Detached after durable claim so closing a browser does not abandon the ledger.
/// Each target is reauthorized, and an unknown target stops the remaining rollout.
pub async fn run_claimed(ctx: ServerCtx, change: DatabaseChange, auth: AuthContext, token: String) {
    let repo = DatabaseChangesRepo::new(ctx.pool.clone());
    let result: Result<()> =
        async {
            for attempt in repo.attempts(&change.id).await? {
                if attempt.state != "queued" {
                    continue;
                }
                let live = ctx.authenticator.authenticate(&token).await?;
                token_gate(&live)?;
                if live.effective_user.id != auth.effective_user.id
                    || live.real_user.id != auth.real_user.id
                {
                    return Err(Error::Forbidden("executor identity changed".into()));
                }
                let current = repo.get(&change.id).await?;
                if current.status != "running" {
                    break;
                }
                fresh_approval(&ctx, &current).await?;
                let approver =
                    UsersRepo::new(ctx.pool.clone())
                        .get(current.approved_by.as_ref().ok_or_else(|| {
                            Error::Forbidden("independent approval missing".into())
                        })?)
                        .await?;
                authorize(&ctx, &approver, &current.targets, "change_approve").await?;
                repo.start_attempt(&attempt.id).await?;
                let execution=ctx.db_explorer.execute_approved_change(&change.id,&attempt.id,&auth.effective_user.id);
                tokio::pin!(execution);
                let mut interval=tokio::time::interval(std::time::Duration::from_millis(500));
                let (success,completed)=loop {
                    tokio::select! {
                        result=&mut execution=>break match result {Ok(result)=>{let (ok,n)=result_progress(&result);(ok,Some(n))},Err(_)=>(false,None)},
                        _=interval.tick()=>{
                            let eligible:Result<()>=async {
                                let live=ctx.authenticator.authenticate(&token).await?;
                                token_gate(&live)?;
                                if live.effective_user.id!=auth.effective_user.id || live.real_user.id!=auth.real_user.id {return Err(Error::Forbidden("executor identity changed".into()))}
                                let author=UsersRepo::new(ctx.pool.clone()).get(&current.author_id).await?;
                                authorize(&ctx,&author,&current.targets,"change_submit").await?;
                                authorize(&ctx,&approver,&current.targets,"change_approve").await
                            }.await;
                            if eligible.is_err() {
                                let _=repo.request_cancel(&current,&auth.effective_user.id,&auth.real_user.id).await;
                                // The DB adapter observes the invalidated claim
                                // and attempts native cancellation. Unknown is
                                // retained when interruption cannot be confirmed.
                                let _=tokio::time::timeout(std::time::Duration::from_secs(5),&mut execution).await;
                                break (false,None);
                            }
                        }
                    }
                };
                // A concurrent cancellation already records unknown; never overwrite
                // that outcome with a late success response.
                let _ = repo.finish_attempt_progress(&attempt.id, success, completed).await;
                if !success {
                    break;
                }
            }
            Ok(())
        }
        .await;
    if result.is_err() {
        tracing::warn!(change_id=%change.id,"reviewed change stopped before the next target; no automatic retry");
    }
    if let Err(error) = repo
        .finish(&change.id, &auth.effective_user.id, &auth.real_user.id)
        .await
    {
        tracing::error!(change_id=%change.id,error=%error,"could not finalize reviewed change ledger");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets_are_canonical_and_cannot_alias_the_lock() {
        let input = ChangeInput {
            title: " test ".into(),
            description: String::new(),
            script: "ALTER TABLE t ADD a int".into(),
            targets: vec![
                ChangeTarget {
                    connection_id: "c".into(),
                    node: "db:test".into(),
                },
                ChangeTarget {
                    connection_id: "c".into(),
                    node: "test".into(),
                },
            ],
        };
        assert!(normalize(input).is_err());
    }
    #[tokio::test]
    async fn change_permissions_intersect_page_workspace_and_direct_denies() {
        use otto_core::access::{AccessActor, AccessRule, RuleEffect, SubjectKind};
        use otto_core::domain::Environment;
        use otto_state::NewConnection;
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("access.db"))
            .await
            .unwrap();
        let user = UsersRepo::new(pool.clone())
            .create("member", "hash", "Member", false)
            .await
            .unwrap();
        let conn = ConnectionsRepo::new(pool.clone())
            .create(NewConnection {
                workspace_id: None,
                name: "DB".into(),
                kind: ConnectionKind::Mysql,
                params: serde_json::json!({"host":"localhost"}),
                secret_ref: None,
                first_command: None,
                section_id: None,
                environment: Environment::Prod,
                read_only: false,
                created_by: user.id.clone(),
            })
            .await
            .unwrap();
        let repo = ResourceAccessRepo::new(pool.clone());
        let mut policy = repo
            .get_policy(ResourceKind::Connection, &conn.id)
            .await
            .unwrap();
        policy.rules = vec![AccessRule {
            id: "grant".into(),
            subject_kind: SubjectKind::User,
            subject_id: user.id.clone(),
            effect: RuleEffect::Allow,
            operations: vec!["discover".into(), "change_submit".into()],
            children: Some(vec!["shop".into()]),
            grantable_operations: vec![],
            credential_connection_id: None,
        }];
        let actor = AccessActor {
            real_user_id: user.id.clone(),
            effective_user_id: None,
        };
        policy = repo
            .put_policy(&policy, policy.revision, &actor)
            .await
            .unwrap();
        let target = vec![ChangeTarget {
            connection_id: conn.id.clone(),
            node: "db:shop".into(),
        }];
        assert!(
            authorize_targets(&pool, &user, &target, "change_submit")
                .await
                .is_err(),
            "page permission remains mandatory"
        );
        sqlx::query("INSERT INTO user_feature_grants(user_id,feature,capability) VALUES(?,'database','view')").bind(&user.id).execute(&pool).await.unwrap();
        assert!(
            authorize_targets(&pool, &user, &target, "change_submit")
                .await
                .is_ok(),
            "resource submit does not require global Edit"
        );
        assert!(authorize_targets(&pool, &user, &target, "change_execute")
            .await
            .is_err());
        assert!(authorize_targets(
            &pool,
            &user,
            &[ChangeTarget {
                connection_id: conn.id.clone(),
                node: "db:hidden".into()
            }],
            "change_submit"
        )
        .await
        .is_err());
        let mut deny = policy.rules[0].clone();
        deny.id = "deny".into();
        deny.effect = RuleEffect::Deny;
        deny.operations = vec!["change_submit".into()];
        policy.rules.push(deny);
        repo.put_policy(&policy, policy.revision, &actor)
            .await
            .unwrap();
        assert!(authorize_targets(&pool, &user, &target, "change_submit")
            .await
            .is_err());
        policy = repo
            .get_policy(ResourceKind::Connection, &conn.id)
            .await
            .unwrap();
        policy.rules.pop();
        repo.put_policy(&policy, policy.revision, &actor)
            .await
            .unwrap();
        sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES('foreign','Foreign','/tmp',datetime('now'))").execute(&pool).await.unwrap();
        sqlx::query("UPDATE connections SET workspace_id='foreign' WHERE id=?")
            .bind(&conn.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            authorize_targets(&pool, &user, &target, "change_submit")
                .await
                .is_err(),
            "resource grant cannot widen workspace membership"
        );
    }
    #[test]
    fn a_failed_later_statement_never_marks_the_batch_successful() {
        let mut result = otto_dbviewer::types::QueryResult::empty();
        let mut failed = otto_dbviewer::types::QueryResult::empty();
        failed.errored = true;
        result.more_results.push(failed);
        assert_eq!(result_progress(&result), (false, 1));
        let mut nested = otto_dbviewer::types::QueryResult::empty();
        nested.more_results.push(result);
        assert_eq!(result_progress(&nested), (false, 2));
        assert_eq!(
            result_progress(&otto_dbviewer::types::QueryResult::empty()),
            (true, 1)
        );
    }
}
