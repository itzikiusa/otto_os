//! Execution accepts only a persisted, claimed immutable review artifact.
use super::*;
use sha2::{Digest, Sha256};
use sqlx::Row;

#[derive(Debug, PartialEq)]
struct Attempt {
    connection_id: Id,
    node: Option<String>,
    script: String,
    content_hash: String,
    policy_revision: i64,
    connection_fingerprint: String,
}

impl DbViewerService {
    /// Stable pre-tunnel fingerprint used by reviewed-change preflight and claim.
    /// Includes the actual selected secret only inside a SHA-256 digest.
    pub async fn change_target_fingerprint(
        &self,
        conn_id: &Id,
        user_id: &Id,
        node: Option<&str>,
    ) -> Result<String> {
        let logical = self.connections.get(conn_id).await?;
        let child = crate::access::child(node);
        let (profile_id, _) = crate::access::credential_profile(
            &self.connections.pool(),
            &logical,
            user_id,
            child.as_deref(),
            "change_execute",
        )
        .await?;
        let profile = self.connections.get(&profile_id).await?;
        let secret = match &profile.secret_ref {
            Some(key) => self.secrets.get(key)?,
            None => None,
        };
        let value = serde_json::json!({
            "logical_id": logical.id, "kind": logical.kind, "params": logical.params,
            "environment": logical.environment, "read_only": logical.read_only,
            "credential_id": profile.id, "credential_params": profile.params,
            "credential_secret": secret, "node": node,
        });
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&value).map_err(|e| Error::Internal(e.to_string()))?)
        ))
    }

    async fn approved_attempt(
        &self,
        change_id: &Id,
        attempt_id: &Id,
        user_id: &Id,
    ) -> Result<Attempt> {
        let row = sqlx::query("SELECT a.connection_id, a.node, a.script, a.content_hash, a.policy_revision, a.connection_fingerprint FROM database_change_attempts a JOIN database_changes c ON c.id=a.change_id WHERE a.id=? AND a.change_id=? AND a.executor_id=? AND a.state='running' AND c.status='running' AND a.content_hash=c.content_hash AND a.script=c.script AND a.executor_id=c.executor_id AND c.approval_hash=c.content_hash AND c.cancellation_requested=0")
            .bind(attempt_id).bind(change_id).bind(user_id).fetch_optional(&self.connections.pool()).await
            .map_err(|e| Error::Internal(format!("load approved change attempt: {e}")))?
            .ok_or_else(|| Error::Forbidden("approved change attempt is not claimed by this executor".into()))?;
        let attempt = Attempt {
            connection_id: row.get("connection_id"),
            node: row.get("node"),
            script: row.get("script"),
            content_hash: row.get("content_hash"),
            policy_revision: row.get("policy_revision"),
            connection_fingerprint: row.get("connection_fingerprint"),
        };
        self.authorize(
            &attempt.connection_id,
            user_id,
            attempt.node.as_deref(),
            "change_execute",
        )
        .await?;
        let connection = self.connections.get(&attempt.connection_id).await?;
        let engine = Engine::from_kind(connection.kind)
            .ok_or_else(|| Error::Invalid("not a database".into()))?;
        let user =
            crate::access::current_user(&self.connections.pool(), &connection, user_id).await?;
        let child = crate::access::child(attempt.node.as_deref());
        let checker = otto_rbac::resource_access::ResourceAccess::new(self.connections.pool());
        for operation in crate::access::operations(engine, &attempt.script)? {
            if checker
                .evaluate(
                    &user,
                    &crate::access::target(&connection.id, child.as_deref()),
                    operation,
                )
                .await?
                .reason
                == "explicit_deny"
            {
                return Err(Error::Forbidden(
                    "approved operation is explicitly denied".into(),
                ));
            }
        }
        let policy =
            crate::access::policy(&self.connections.pool(), &attempt.connection_id).await?;
        if policy.revision != attempt.policy_revision
            || self
                .change_target_fingerprint(&attempt.connection_id, user_id, attempt.node.as_deref())
                .await?
                != attempt.connection_fingerprint
        {
            return Err(Error::Conflict(
                "change approval is stale: access policy or connection credentials changed".into(),
            ));
        }
        Ok(attempt)
    }

    /// Validate syntax, exact operation scope and actual native credentials
    /// without executing the proposed script. Used before review is requested.
    pub async fn preflight_approved_change(
        &self,
        conn_id: &Id,
        user_id: &Id,
        node: Option<&str>,
        script: &str,
    ) -> Result<()> {
        self.approved_preflight(conn_id, user_id, node, script)
            .await
            .map(|_| ())
    }

    async fn approved_preflight(
        &self,
        conn_id: &Id,
        user_id: &Id,
        node: Option<&str>,
        script: &str,
    ) -> Result<Resolved> {
        self.authorize(conn_id, user_id, node, "change_execute")
            .await?;
        let conn = self.connections.get(conn_id).await?;
        let engine =
            Engine::from_kind(conn.kind).ok_or_else(|| Error::Invalid("not a database".into()))?;
        if !matches!(engine, Engine::Mysql | Engine::Postgres) {
            return Err(Error::Forbidden(
                "reviewed execution supports MySQL and PostgreSQL only".into(),
            ));
        }
        if conn.read_only {
            return Err(Error::Forbidden(
                "read-only connection cannot execute database changes".into(),
            ));
        }
        let operations = crate::access::operations(engine, script)?;
        let child = crate::access::child(node);
        let user = crate::access::current_user(&self.connections.pool(), &conn, user_id).await?;
        let access = otto_rbac::resource_access::ResourceAccess::new(self.connections.pool());
        for operation in &operations {
            let decision = access
                .evaluate(
                    &user,
                    &crate::access::target(&conn.id, child.as_deref()),
                    operation,
                )
                .await?;
            if decision.reason == "explicit_deny" {
                return Err(Error::Forbidden(
                    "database operation is explicitly denied despite change approval".into(),
                ));
            }
        }
        let r = self
            .resolve(&conn.id, user_id, child.as_deref(), "change_execute")
            .await?;
        if !user.is_root {
            let target = child.as_deref().ok_or_else(|| {
                Error::Forbidden("approved execution requires one explicit database target".into())
            })?;
            for grant in r.driver.native_grants(&r.config).await? {
                if grant.child != target
                    || (grant.operation != "db_browse" && !operations.contains(&grant.operation))
                {
                    return Err(crate::native_access::setup_error(
                        "migration credential exceeds the approved target or operation scope",
                    ));
                }
            }
        }
        Ok(r)
    }

    /// Execute the immutable artifact of an already claimed attempt. There is
    /// deliberately no public script argument that could bypass review.
    /// Root still needs the persisted running approval/claim and exact bindings.
    pub async fn execute_approved_change(
        &self,
        change_id: &Id,
        attempt_id: &Id,
        user_id: &Id,
    ) -> Result<QueryResult> {
        let attempt = self
            .approved_attempt(change_id, attempt_id, user_id)
            .await?;
        let r = self
            .approved_preflight(
                &attempt.connection_id,
                user_id,
                attempt.node.as_deref(),
                &attempt.script,
            )
            .await?;
        // Revalidate the claim after network/native preflight, immediately before SQL.
        if self
            .approved_attempt(change_id, attempt_id, user_id)
            .await?
            != attempt
        {
            return Err(Error::Conflict(
                "approved artifact changed during preflight".into(),
            ));
        }
        let req = QueryRequest {
            statement: attempt.script.clone(),
            node: crate::access::child(attempt.node.as_deref()),
            confirm_write: false,
            ..Default::default()
        };
        let token = CancelToken::new();
        let execution = r.driver.run_tracked(&r.config, &req, &token);
        tokio::pin!(execution);
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                result = &mut execution => return result,
                _ = interval.tick() => {
                    if !matches!(self.approved_attempt(change_id, attempt_id, user_id).await, Ok(ref current) if current == &attempt) {
                        if let Some(handle) = token.handle() { let _ = r.driver.cancel(&r.config, &handle).await; }
                        return Err(Error::Forbidden("change attempt revoked during execution; cancellation was attempted, outcome requires reconciliation".into()));
                    }
                }
            }
        }
    }
}
