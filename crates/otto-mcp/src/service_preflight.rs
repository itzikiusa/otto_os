//! Live authorization at the final outbound transport boundary.
use super::*;

impl McpService {
    pub(super) async fn authorize_before_send(
        &self,
        original: &McpServerDetail,
        tool_name: &str,
        ctx: &InvokeCtx,
        approved: bool,
    ) -> std::result::Result<(), String> {
        self.check_before_send(original, tool_name, ctx, approved)
            .await
            .map_err(|error| redact_text(&error.to_string()).value)
    }

    async fn check_before_send(
        &self,
        original: &McpServerDetail,
        tool_name: &str,
        ctx: &InvokeCtx,
        approved: bool,
    ) -> Result<()> {
        let deny = |reason: &str| Error::Conflict(reason.into());
        let server = self.registry().get(&original.id).await?;
        if !server.enabled {
            return Err(deny("server disabled before execution"));
        }
        // A changed endpoint or credential configuration needs a new client
        // and a new governed request, never an old queued transport.
        if server.updated_at != original.updated_at
            || server.transport != original.transport
            || server.command != original.command
            || server.args != original.args
            || server.env != original.env
            || server.headers != original.headers
            || server.url != original.url
        {
            return Err(deny("server configuration changed; retry the tool call"));
        }
        let tool = self.tools().get_by_name(&server.id, tool_name).await?;
        if !tool.enabled {
            return Err(deny("tool disabled before execution"));
        }
        if let Some(ws) = ctx.workspace_id.as_ref() {
            let mode = self.allowlist().resolve(ws, &server.id, tool_name).await?;
            if mode.as_deref() == Some("deny")
                || (mode.is_none() && server.default_tool_access == "deny")
            {
                return Err(deny("workspace access revoked before execution"));
            }
        }
        let rules = self
            .policies()
            .list_applicable(ctx.workspace_id.as_deref().unwrap_or(""))
            .await?;
        let effect = policy::evaluate(
            &rules,
            &PolicyCtx {
                server_id: &server.id,
                server_name: &server.name,
                tool: tool_name,
                risk_label: &tool.risk_label,
                injection_risk: &tool.injection_risk,
                mutating: tool.mutating,
                direction: &ctx.direction,
                caller_kind: &ctx.caller_kind,
                workspace_id: ctx.workspace_id.as_deref(),
            },
        );
        match &effect {
            Effect::Deny(reason) => return Err(deny(&format!("policy denied: {reason}"))),
            Effect::RequireDryRun(_) => {
                return Err(deny("policy now requires a dry run; retry for a preview"));
            }
            _ => {}
        }
        let approval_required = tool.require_approval
            || matches!(effect, Effect::RequireApproval(_))
            || (tool.risk_label == "dangerous" && self.require_approval_dangerous().await);
        if approval_required && !approved {
            return Err(deny("approval now required; retry to request approval"));
        }
        let access = otto_state::ResourceAccessRepo::new(self.pool.clone())
            .get_live_policy(otto_core::access::ResourceKind::McpServer, &server.id)
            .await?;
        if access.mode == otto_core::access::AccessMode::Legacy {
            if !server.managed {
                return Err(deny("server is no longer managed"));
            }
        } else {
            let Some(id) = &ctx.caller_user_id else {
                return Err(deny("resource access revoked before execution"));
            };
            let user = otto_state::UsersRepo::new(self.pool.clone())
                .get(id)
                .await?;
            if !self
                .resource_allowed_under(&access, &server, &user, &[("invoke", Some(tool_name))])
                .await?[0]
            {
                return Err(deny("resource access revoked before execution"));
            }
        }
        Ok(())
    }
}
