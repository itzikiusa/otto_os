//! [`McpService::invoke`]'s stage bodies for the shared governance runner
//! ([`crate::governance`], stage set `REGISTERED_SERVERS`): policy-as-code,
//! the per-tool / dangerous approval gate, the single-use approval, dry-run and
//! the fail-closed audit insert of a registered-server tool call. Moved out of
//! `invoke` unchanged; the runner owns their order.

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use otto_core::redact::redact_json;
use otto_core::{Error, Result};
use otto_state::{McpServerDetail, NewApproval, NewCallLog};
use serde_json::{json, Value};

use super::{InvokeCtx, InvokeOutcome, McpService, APPROVAL_TTL_MINS};
use crate::governance::{Approval, ApprovalGate, Decision, PolicyVerdict, Stages};
use crate::policy::{self, Effect, PolicyCtx};

pub(super) struct ServerStages<'a> {
    pub svc: &'a McpService,
    pub server: &'a McpServerDetail,
    pub tool_name: &'a str,
    pub args: &'a Value,
    pub ctx: &'a InvokeCtx,
    pub risk_label: &'a str,
    pub injection_risk: &'a str,
    pub mutating: bool,
    pub tool_require_approval: bool,
    pub args_hash: String,
    /// Set once a single-use approval was consumed for this call.
    pub approval_id_used: Option<String>,
}

#[async_trait]
impl Stages for ServerStages<'_> {
    type Outcome = InvokeOutcome;
    type Row = String;
    type Error = Error;

    // 3. policy-as-code (most-restrictive-wins).
    async fn policy(&mut self) -> Result<PolicyVerdict> {
        let rules = self
            .svc
            .policies()
            .list_applicable(self.ctx.workspace_id.as_deref().unwrap_or(""))
            .await?;
        let pctx = PolicyCtx {
            server_id: &self.server.id,
            server_name: &self.server.name,
            tool: self.tool_name,
            risk_label: self.risk_label,
            injection_risk: self.injection_risk,
            mutating: self.mutating,
            direction: &self.ctx.direction,
            caller_kind: &self.ctx.caller_kind,
            workspace_id: self.ctx.workspace_id.as_deref(),
        };
        Ok(match policy::evaluate(&rules, &pctx) {
            Effect::Allow => PolicyVerdict::Allow,
            Effect::Deny(reason) => PolicyVerdict::Deny(reason),
            Effect::RequireApproval(_) => PolicyVerdict::RequireApproval,
            Effect::RequireDryRun(_) => PolicyVerdict::RequireDryRun,
        })
    }

    async fn deny(&mut self, reason: &str) -> Result<InvokeOutcome> {
        self.svc
            .terminal_deny(
                self.server,
                self.tool_name,
                self.args,
                self.risk_label,
                self.injection_risk,
                self.ctx,
                reason,
            )
            .await
    }

    // 4. risk / approval gate: a policy, the per-tool flag, or a dangerous
    //    tool under `mcp_require_approval_dangerous`.
    async fn approval_gate(&mut self, policy_requires: bool) -> ApprovalGate {
        let dangerous_default =
            self.risk_label == "dangerous" && self.svc.require_approval_dangerous().await;
        ApprovalGate {
            needed: policy_requires || self.tool_require_approval || dangerous_default,
            auto_approvable: false,
        }
    }

    /// Not in `REGISTERED_SERVERS`; never called.
    async fn auto_approve(&mut self) -> bool {
        false
    }

    async fn approval(&mut self) -> Result<Approval<InvokeOutcome>> {
        let (server, tool_name, ctx) = (self.server, self.tool_name, self.ctx);
        match self
            .svc
            .approvals()
            .find_usable(
                ctx.workspace_id.as_deref(),
                Some(&server.id),
                tool_name,
                &self.args_hash,
                ctx.caller_user_id.as_deref(),
            )
            .await?
        {
            Some(appr_id) => {
                let approval = self.svc.approvals().get(&appr_id).await?;
                if approval.requested_by != ctx.caller_user_id {
                    return Ok(Approval::Stop(
                        self.deny("approval belongs to another caller").await?,
                    ));
                }
                // Single-use: consume atomically; a lost race => already used.
                if !self.svc.approvals().consume(&appr_id).await? {
                    return Ok(Approval::Stop(
                        self.deny("approval was already used").await?,
                    ));
                }
                self.approval_id_used = Some(appr_id);
                Ok(Approval::Granted)
            }
            None => {
                // Create a pending approval bound to the EXACT args.
                let risk_label = self.risk_label;
                let redacted = redact_json(self.args).value.to_string();
                let expires =
                    (Utc::now() + ChronoDuration::minutes(APPROVAL_TTL_MINS)).to_rfc3339();
                let appr = self
                    .svc
                    .approvals()
                    .create(NewApproval {
                        workspace_id: ctx.workspace_id.clone(),
                        kind: "tool_call".into(),
                        server_id: Some(server.id.clone()),
                        server_name: Some(server.name.clone()),
                        tool: Some(tool_name.to_string()),
                        title: format!("{} → {}", server.name, tool_name),
                        detail: Some(format!(
                            "Approve {risk_label} MCP tool '{tool_name}' on server '{}'.",
                            server.name
                        )),
                        args_redacted_json: redacted,
                        args_hash: Some(self.args_hash.clone()),
                        risk_label: Some(risk_label.to_string()),
                        requested_by: ctx.caller_user_id.clone(),
                        requested_by_kind: Some(ctx.caller_kind.clone()),
                        requested_by_session_id: None,
                        expires_at: Some(expires),
                    })
                    .await?;
                self.svc
                    .audit_terminal(
                        server,
                        tool_name,
                        self.args,
                        risk_label,
                        self.injection_risk,
                        ctx,
                        "pending_approval",
                        Some(&format!("awaiting approval for {risk_label} tool")),
                        Some(&appr.id),
                    )
                    .await?;
                Ok(Approval::Stop(InvokeOutcome::Pending {
                    approval_id: appr.id,
                    title: format!("{} → {}", server.name, tool_name),
                }))
            }
        }
    }

    // 5. dry-run = pure simulation (never calls the tool). design §14 F4.
    async fn dry_run(&mut self) -> Result<InvokeOutcome> {
        let preview = json!({
            "executed": false,
            "mode": "preview",
            "would_call": { "server": self.server.name, "tool": self.tool_name, "arguments": redact_json(self.args).value },
            "note": "dry-run: arguments validated and target resolved; the tool was NOT executed",
        });
        self.svc
            .audit_terminal(
                self.server,
                self.tool_name,
                self.args,
                self.risk_label,
                self.injection_risk,
                self.ctx,
                "dry_run",
                None,
                self.approval_id_used.as_deref(),
            )
            .await?;
        Ok(InvokeOutcome::DryRun { preview })
    }

    // 6. Fail-closed audit: insert the row BEFORE running so an audit failure
    //    aborts the call; `invoke` finalizes it with the outcome.
    async fn audit(&mut self, decision: Decision) -> Result<String> {
        let (server, ctx) = (self.server, self.ctx);
        self.svc
            .call_log()
            .insert(NewCallLog {
                workspace_id: ctx.workspace_id.clone(),
                server_id: Some(server.id.clone()),
                server_name: Some(server.name.clone()),
                tool: self.tool_name.to_string(),
                direction: ctx.direction.clone(),
                caller_user_id: ctx.caller_user_id.clone(),
                caller_kind: Some(ctx.caller_kind.clone()),
                args_redacted_json: redact_json(self.args).value.to_string(),
                decision: decision.as_str().into(),
                decision_reason: None,
                risk_label: Some(self.risk_label.to_string()),
                injection_risk: Some(self.injection_risk.to_string()),
                dry_run: false,
                ok: false, // finalized by `invoke`
                error: None,
                latency_ms: None,
                bytes: None,
                rows: None,
                approval_id: self.approval_id_used.clone(),
                caller_session_id: None,
            })
            .await // ← propagates: no audit row ⇒ no execution (fail-closed)
    }
}
