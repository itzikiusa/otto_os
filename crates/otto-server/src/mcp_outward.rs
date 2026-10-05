//! "Otto as an MCP server" — the OUTWARD surface. External agents (Claude Code,
//! Copilot, …) connect to `ottod mcp-server` over stdio with a **restricted**
//! `kind='mcp'` token and call the `otto.*` tools. Every call funnels
//! through `POST /mcp/otto-tools/invoke` (the only route that token may reach —
//! see feature_guard, design §14 F1), which governs (enabled? allowlisted?
//! dangerous→approval?), audits (`mcp_call_log`, direction='inbound'), then
//! executes the capability **as the token's user** by self-calling the real
//! endpoint with a short-lived ephemeral token — so each tool reuses its
//! endpoint's native RBAC (no privilege escalation). It also hosts the live-agent
//! **gateway** (`/mcp/gateway/*`).
//!
//! The state-free half — the tool catalog + policy lists, `route_for` and the
//! self-call executor, the reference/pin tables — lives in
//! [`otto_mcp::outward`] and is re-exported here; this module keeps only the
//! glue that needs [`ServerCtx`] (settings, approvals, audit, ref lookups).

use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use otto_core::api::CreateMcpTokenReq;
use otto_core::auth::{AuthContext, McpScope};
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_mcp::{canonical_hash, InvokeCtx};
use otto_rbac::AuthRepo;
use otto_state::{NewApproval, NewCallLog, SettingsRepo};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

pub use otto_mcp::outward::*;

const MAX_WAIT_SECS: u64 = 30;

/// Settings key recording which `ui_*` tools existed when the operator last
/// saved the enabled-tool list (see [`merge_enabled`]).
const UI_TOOLS_KNOWN_KEY: &str = "mcp_otto_server_ui_tools_known";

/// How long the governed pipeline's settings reads may be served from the
/// per-process cache ([`SettingsRepo::get_cached`]). Every governed call read
/// 4–5 settings rows; a write through `SettingsRepo` (the `PATCH
/// /mcp/otto-server` and `PUT /settings` paths) invalidates at once, so this
/// only bounds staleness for out-of-process writes.
const SETTINGS_TTL: Duration = Duration::from_secs(2);

async fn enabled_tools(ctx: &ServerCtx) -> Vec<String> {
    let settings = SettingsRepo::new(ctx.pool.clone());
    let stored = settings
        .get_cached("mcp_otto_server_tools", SETTINGS_TTL)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok());
    let ui_known = settings
        .get_cached(UI_TOOLS_KNOWN_KEY, SETTINGS_TTL)
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default();
    merge_enabled(stored, &ui_known, &crate::ui_commands::tool_names())
}

async fn outward_enabled(ctx: &ServerCtx) -> bool {
    SettingsRepo::new(ctx.pool.clone())
        .get_cached("mcp_otto_server_enabled", SETTINGS_TTL)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

async fn require_approval_dangerous(ctx: &ServerCtx) -> bool {
    SettingsRepo::new(ctx.pool.clone())
        .get_cached("mcp_require_approval_dangerous", SETTINGS_TTL)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Whether an explicit per-token write grant (`McpScope.allow_writes`) counts as
/// having already approved DANGEROUS tools for that token. Default **true**:
/// minting a read+write MCP token IS the approval decision, and asking again per
/// call strands automation without adding a real check. Set
/// `mcp_trust_token_write_grant: false` to restore the per-call prompt for
/// write-scoped tokens too.
async fn trust_token_write_grant(ctx: &ServerCtx) -> bool {
    SettingsRepo::new(ctx.pool.clone())
        .get_cached("mcp_trust_token_write_grant", SETTINGS_TTL)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Short tool names exempted from the DANGEROUS approval gate EVERYWHERE — the
/// MCP → Otto server per-tool **Auto-approve** switch (formerly "Ask before
/// each call"): the bare targets of the enabled GLOBAL per-tool auto-approve
/// rules (`mcp_auto_approve_rules`, see [`crate::mcp_auto_approve`]). Migration
/// 0146 imported the legacy `mcp_approval_exempt_tools` setting into such rules;
/// the setting is no longer read. Workspace / session / category rules are
/// resolved per call and are not part of this list.
///
/// WHY per-tool exists: the gate was all-or-nothing. An operator who deliberately
/// enabled ONE outward-facing tool (say, PR comments for the review workflow)
/// could only stop the second approval prompt by clearing
/// `mcp_require_approval_dangerous`, which simultaneously disarms `create_pr`,
/// `run_workflow`, `produce_broker_message`, `vault_delete`, `broadcast_message`
/// and every other write. Enabling one tool should not require disarming all of
/// them, so auto-approval is per-tool (or per-category) and opt-in.
///
/// A tool must STILL be enabled (`tool_enabled`) to run at all — this only skips
/// the approval prompt for a capability already granted. Calls remain audited
/// (`auto_approved`, naming the rule).
async fn approval_exempt_tools(ctx: &ServerCtx) -> Vec<String> {
    let rules = otto_state::McpAutoApproveRepo::new(ctx.pool.clone())
        .list()
        .await
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for r in rules
        .iter()
        .filter(|r| crate::mcp_auto_approve::is_active_catalog_toggle(r))
    {
        if !out.contains(&r.target) {
            out.push(r.target.clone());
        }
    }
    out
}

// ===========================================================================
// POST /mcp/otto-tools/invoke  (the governed choke point for every otto.* tool)
// ===========================================================================

#[derive(Deserialize)]
pub struct OttoInvokeReq {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub wait_seconds: Option<u64>,
}

pub async fn otto_tools_invoke(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<OttoInvokeReq>,
) -> ApiResult<Json<Value>> {
    let mut env = governed_invoke(
        &ctx,
        &auth,
        &req.tool,
        &req.arguments,
        req.dry_run,
        req.wait_seconds,
    )
    .await?;
    // perf2/10-mcp R4: a UI tool's reply carries the calling session's UI
    // grant AFTER the call (it may just have been allowed or revoked), so the
    // stdio bridge re-lists from this field instead of a second request to
    // `GET /mcp/otto-server/enabled`.
    let bare = req.tool.strip_prefix("otto.").unwrap_or(&req.tool);
    if crate::ui_commands::is_ui_tool(bare) {
        if let (Some(sid), Some(obj)) = (
            crate::ui_bridge::calling_session(&auth),
            env.as_object_mut(),
        ) {
            let granted = ctx
                .manager
                .get(sid)
                .await
                .ok()
                .is_some_and(|s| crate::ui_bridge::grant_state(&s.meta) == Some(true));
            obj.insert("ui_granted".into(), json!(granted));
        }
    }
    Ok(Json(env))
}

/// The governed choke point for every `otto.*` tool call, shared by the bespoke
/// `POST /mcp/otto-tools/invoke` endpoint AND the MCP HTTP transport's
/// `tools/call`. It enforces — in order — the **per-token scope** (multi-token
/// access control), the global enable + per-tool allow-list, dangerous→approval,
/// dry-run, then executes the capability AS `auth.effective_user` (native RBAC
/// reused via an ephemeral self-call) and audits the whole thing.
///
/// Returns the governed envelope `Value` (`{decision, executed, content, …}`).
pub(crate) async fn governed_invoke(
    ctx: &ServerCtx,
    auth: &AuthContext,
    tool: &str,
    arguments: &Value,
    dry_run: bool,
    wait_seconds: Option<u64>,
) -> ApiResult<Value> {
    let user = &auth.effective_user;
    let short = tool.strip_prefix("otto.").unwrap_or(tool).to_string();
    // Vault calls may omit `workspace_id` (vaults are a global library) —
    // resolve the effective workspace up front so the token's workspace pin,
    // the audit record, and the approval args-hash all see the same scoped
    // arguments.
    let filled = fill_vault_workspace(ctx, auth, &short, arguments)
        .await
        .map_err(ApiError)?;
    let arguments = filled.as_ref().unwrap_or(arguments);
    // `pr_number` / `number` aliases and stringified integers settle into the
    // one canonical spelling each route reads, before anything hashes them.
    let normalized = normalize_args(&short, arguments);
    let arguments = normalized.as_ref().unwrap_or(arguments);
    let mut audit = NewCallLog {
        tool: tool.to_string(),
        direction: "inbound".into(),
        server_name: Some("otto".into()),
        caller_user_id: Some(user.id.clone()),
        caller_kind: Some("mcp_server".into()),
        args_redacted_json: otto_core::redact::redact_json(arguments).value.to_string(),
        // The agent session behind an Otto-minted credential, so the audit
        // can be split per agent (and scoped to its workspace below).
        caller_session_id: crate::ui_bridge::calling_session(auth).cloned(),
        ..Default::default()
    };
    // The calling session's workspace: the audit row's workspace when the
    // call itself names none, so the row is scoped to a workspace (and never
    // falls into the NULL-workspace bucket every MCP viewer could read).
    // The calling session's row, read ONCE per call: its workspace here, and
    // the personal-agent policy below (which used to read it again).
    let calling_session = match &audit.caller_session_id {
        Some(sid) => ctx.manager.get(sid).await.ok(),
        None => None,
    };
    let session_ws = calling_session.as_ref().map(|s| s.workspace_id.clone());
    audit.workspace_id = arguments
        .get("workspace_id")
        .and_then(Value::as_str)
        .filter(|w| crate::agent_refs::looks_like_id(w))
        .map(str::to_string)
        .or_else(|| session_ws.clone());

    // PER-TOKEN SCOPE (multi-token access control). A `kind='mcp'` token carries an
    // [`McpScope`]; a NULL column resolved to the unrestricted scope, so legacy
    // tokens are unaffected. Normal (session/api) tokens have `mcp_scope == None`
    // and are bounded only by the global enable + the user's own RBAC below. This
    // is the gate that makes different tokens / users have different accesses, and
    // it is identical for the HTTP transport and the legacy stdio path.
    if let Some(scope) = &auth.mcp_scope {
        let mutating = tool_is_mutating(&short);
        // A workspace NAME is resolved below and the pin re-checked on the
        // resolved id (`pin_verdict`); only a literal id is comparable here.
        let ws_arg = arguments
            .get("workspace_id")
            .and_then(Value::as_str)
            .filter(|w| crate::agent_refs::looks_like_id(w));
        if let Some(reason) = scope.deny_reason(&short, mutating, ws_arg) {
            return Ok(deny_audit(ctx, &mut audit, &format!("token scope: {reason}")).await);
        }
    }

    let outward_on = outward_enabled(ctx).await;
    let enabled = enabled_tools(ctx).await;
    let is_ui = crate::ui_commands::is_ui_tool(&short);
    let ui_session = is_ui && auth.managed_session_id.is_some();
    if !mcp_tool_enabled_for_token(auth.mcp_internal, ui_session, outward_on, &enabled, &short) {
        let reason = if outward_on {
            "this tool is not enabled on the Otto MCP server"
        } else {
            "the Otto MCP server is disabled"
        };
        return Ok(deny_audit(ctx, &mut audit, reason).await);
    }

    // PERSONAL-AGENT POLICY (crate::personal_agent_policy): a read-only agent
    // session (proactive / read-only schedule) is refused every mutating tool
    // and send; an agent's enforceable custom rules deny or force approval; and
    // an account / credential / sharing action ALWAYS needs a human approval —
    // no auto-approve rule or token write grant skips it. Right after the
    // enable gate and BEFORE reference resolution (like the scope check, a
    // refused call never resolves anything); rules and the sensitive check
    // read the caller's own arguments (the names it typed, e.g. `prod-api`).
    let (agent_gate, calling_agent) = crate::personal_agent_policy::evaluate_with(
        ctx,
        auth,
        &short,
        arguments,
        calling_session.as_ref(),
    )
    .await;
    if let crate::personal_agent_policy::AgentGate::Deny(reason) = &agent_gate {
        return Ok(deny_audit(ctx, &mut audit, reason).await);
    }
    let forced_approval = match &agent_gate {
        crate::personal_agent_policy::AgentGate::ForceApproval { reason, risk } => {
            Some((reason.clone(), *risk))
        }
        _ => None,
    };

    // Git tools take a FRIENDLY repo reference (name / path / remote, or none
    // → the calling session's repo), resolved across every workspace the
    // caller can read. Deliberately AFTER the scope + enable gates above, so a
    // token that may not call this tool never learns the repo directory from a
    // resolution error. Like `fill_vault_workspace`, the resolved arguments
    // (canonical `repo_id` + the repo's own `workspace_id`) replace the
    // caller's for everything below — audit, approval scope, args-hash (so a
    // name and an id for the same repo reuse one approval), and execution.
    let repo_fill = match fill_repo_ref(ctx, auth, &short, arguments).await {
        Ok(fill) => fill,
        Err(Error::Forbidden(reason)) => {
            return Ok(deny_audit(ctx, &mut audit, &reason).await);
        }
        Err(e) => return Ok(unresolved_audit(ctx, &mut audit, &e).await),
    };
    let repo_label = repo_fill.as_ref().and_then(|f| f.label.clone());
    let arguments = repo_fill.as_ref().map_or(arguments, |f| &f.args);

    // Every OTHER friendly reference — a workspace / workflow / connection /
    // swarm / scheduled-task / cluster / account / vault / artifact NAME, a
    // Jira key for a story, a Confluence page URL, a Jira transition name —
    // resolved the same way (`agent_refs`: across every workspace the caller
    // can read, as the caller, candidates listed on a miss), plus the owning
    // workspace of an id-only object for a workspace-pinned token. Same
    // placement and contract as the repo fill: after the scope + enable gates,
    // and the resolved arguments replace the caller's for everything below.
    let ref_fill = match fill_refs(ctx, auth, &short, arguments).await {
        Ok(fill) => fill,
        Err(Error::Forbidden(reason)) => {
            return Ok(deny_audit(ctx, &mut audit, &reason).await);
        }
        Err(e) => return Ok(unresolved_audit(ctx, &mut audit, &e).await),
    };
    let arguments = ref_fill.as_ref().unwrap_or(arguments);
    // Design calls carry the ARTIFACT's workspace (never a caller-supplied
    // one), so a workspace-pinned token, the audit row and the approval scope
    // all see where the call really lands.
    let design_filled = fill_design_workspace(ctx, &short, arguments).await;
    let arguments = design_filled.as_ref().unwrap_or(arguments);
    if repo_fill.is_some() || ref_fill.is_some() || design_filled.is_some() {
        audit.args_redacted_json = otto_core::redact::redact_json(arguments).value.to_string();
    }
    // The workspace pin, re-checked against the RESOLVED arguments — the
    // workspace a named object, a repo or an artifact actually lives in. A
    // pinned token calling a tool whose workspace Otto cannot establish is
    // denied (fail closed) rather than let through unchecked.
    if let Some(scope) = &auth.mcp_scope {
        if let Some(reason) = pin_verdict(scope, &short, arguments) {
            return Ok(deny_audit(ctx, &mut audit, &reason).await);
        }
    }

    let dangerous = DANGEROUS.contains(&short.as_str());
    // A `kind='mcp'` token carrying an EXPLICIT write grant (`allow_writes`) has
    // already cleared this decision at issue time: someone deliberately minted a
    // read+write token for this caller. Re-prompting per call asks the same
    // question twice and strands automation (a workflow step posting 21 review
    // comments filed 21 approvals and posted none). The scope check above still
    // runs first, so a read-only token, a tool outside the token's allowed set,
    // or a workspace-pinned mismatch is denied outright rather than reaching here.
    // Session/api callers have `mcp_scope == None` — no explicit grant, so they
    // stay gated. Governed by `mcp_trust_token_write_grant` for operators who
    // want the belt-and-braces prompt back. Calls remain audited either way.
    let token_write_grant = auth
        .mcp_scope
        .as_ref()
        .is_some_and(|scope| scope.allow_writes)
        && trust_token_write_grant(ctx).await;
    let args_hash = canonical_hash(arguments);
    let ws = arguments
        .get("workspace_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    // The RESOLVED workspace (a named object / repo / artifact's own) wins.
    if let Some(w) = ws.clone().or_else(|| session_ws.clone()) {
        audit.workspace_id = Some(w);
    }
    // The gate applies (before any opt-out rule) to a DANGEROUS tool without a
    // trusted token grant, under the global `mcp_require_approval_dangerous`.
    let gate_applies = approval_gated(dangerous, false, token_write_grant)
        && require_approval_dangerous(ctx).await;
    // A forced approval (sensitive action / agent rule) is never covered by
    // an auto-approve rule.
    let gate_applies = gate_applies && forced_approval.is_none();
    // An operator who explicitly auto-approved this tool (or its category) for
    // this scope — global, the call's workspace, or the calling agent session —
    // has already made the decision: don't ask a second time. Opt-in, off by
    // default; irreversible tools need a per-tool rule with the second toggle.
    // A lookup failure resolves to None ⇒ the call stays gated (fail closed).
    let auto_rule = if gate_applies {
        crate::mcp_auto_approve::resolve_for_call(ctx, auth, &short, ws.as_deref()).await
    } else {
        None
    };
    let needs_approval = (approval_gated(dangerous, auto_rule.is_some(), token_write_grant)
        && gate_applies)
        || forced_approval.is_some();
    if let Some(rule) = &auto_rule {
        // Recorded on every terminal row below (dry-run and execution alike).
        audit.decision_reason = Some(otto_mcp::auto_approve::audit_reason(rule));
    }
    let auto_approved_by = auto_rule
        .as_ref()
        .map(crate::mcp_auto_approve::envelope_ref);

    if needs_approval && !dry_run {
        match ctx
            .mcp
            .approvals()
            .find_usable(
                ws.as_deref(),
                None,
                tool,
                &args_hash,
                Some(&auth.effective_user.id),
            )
            .await
            .map_err(ApiError)?
        {
            Some(appr_id) => {
                if !ctx
                    .mcp
                    .approvals()
                    .consume(&appr_id)
                    .await
                    .map_err(ApiError)?
                {
                    return Ok(deny_audit(ctx, &mut audit, "approval already used").await);
                }
                audit.approval_id = Some(appr_id);
            }
            None => {
                // A retry of a call that is still waiting reuses its card
                // (same tool + args + workspace + requester) instead of
                // stacking a duplicate per poll.
                let pending = ctx
                    .mcp
                    .approvals()
                    .find_pending(ws.as_deref(), None, tool, &args_hash, &user.id)
                    .await
                    .map_err(ApiError)?;
                let appr_id = match pending {
                    Some(id) => id,
                    None => {
                        ctx.mcp
                            .approvals()
                            .create(NewApproval {
                                workspace_id: ws.clone(),
                                kind: "tool_call".into(),
                                server_id: None,
                                server_name: Some("otto".into()),
                                tool: Some(tool.to_string()),
                                title: format!("otto MCP server → {tool}"),
                                detail: Some({
                                    let base = match &repo_label {
                                        // The resolved repo by name, so the approver isn't
                                        // judging an opaque id.
                                        Some(label) => {
                                            format!(
                                                "{} — repo {label}",
                                                dangerous_detail(tool, arguments)
                                            )
                                        }
                                        None => dangerous_detail(tool, arguments),
                                    };
                                    match &forced_approval {
                                        Some((why, _)) => format!("{why}. {base}"),
                                        None => base,
                                    }
                                }),
                                args_redacted_json: audit.args_redacted_json.clone(),
                                args_hash: Some(args_hash.clone()),
                                risk_label: Some(
                                    forced_approval
                                        .as_ref()
                                        .map_or("dangerous", |(_, risk)| *risk)
                                        .into(),
                                ),
                                requested_by: Some(user.id.clone()),
                                requested_by_kind: Some("mcp_server".into()),
                                requested_by_session_id: audit.caller_session_id.clone(),
                                expires_at: Some(
                                    (chrono::Utc::now() + chrono::Duration::minutes(120))
                                        .to_rfc3339(),
                                ),
                            })
                            .await
                            .map_err(ApiError)?
                            .id
                    }
                };
                if let Some(a) = &calling_agent {
                    crate::personal_agent_activity::record_approval_waiting(
                        ctx,
                        &a.workspace_id,
                        &a.agent_id,
                        Some(&a.session_id),
                        &short,
                        &appr_id,
                    );
                }
                match wait_for_decision(ctx, &appr_id, wait_seconds).await {
                    Some(true) => {
                        let _ = ctx.mcp.approvals().consume(&appr_id).await;
                        audit.approval_id = Some(appr_id.clone());
                    }
                    Some(false) => {
                        audit.decision = "denied".into();
                        audit.decision_reason = Some("human denied the request".into());
                        let _ = ctx.mcp.call_log().insert(audit).await;
                        return Ok(
                            json!({"decision":"denied","executed":false,"reason":"human denied the request"}),
                        );
                    }
                    None => {
                        audit.decision = "pending_approval".into();
                        audit.approval_id = Some(appr_id.clone());
                        let _ = ctx.mcp.call_log().insert(audit).await;
                        return Ok(json!({"decision":"pending_approval","executed":false,
                            "approval_id":appr_id,"reason":"awaiting human approval — resubmit after it is approved"}));
                    }
                }
            }
        }
    }

    if dry_run {
        audit.decision = "dry_run".into();
        audit.dry_run = true;
        audit.ok = true;
        let _ = ctx.mcp.call_log().insert(audit).await;
        return Ok(json!({"decision":"dry_run","executed":false,"dry_run":true,
            "preview":{"tool":tool,"arguments":otto_core::redact::redact_json(arguments).value,
                       "note":"dry-run: the tool was NOT executed"}}));
    }

    // Fail-closed audit: insert before executing.
    audit.decision = if audit.approval_id.is_some() {
        "approved".into()
    } else if auto_rule.is_some() {
        // Never silent: the row names the rule (`decision_reason`, set above).
        "auto_approved".into()
    } else {
        "allowed".into()
    };
    let audit_id = ctx.mcp.call_log().insert(audit).await.map_err(ApiError)?;

    let started = std::time::Instant::now();
    // An internal per-session MCP credential carries an immutable session
    // binding; for the room tools it OVERRIDES any client-supplied session_id
    // so a bound token can only ever speak as its own session's agent. The
    // room tools bind ANY agent-session credential (an Otto-issued API token
    // carries only `managed_session_id`): without it such a session could
    // omit `session_id` and post as the human, or read a room it is not a
    // member of. The route then refuses a session that is not a room-member
    // personal agent.
    let bound_session = match short.as_str() {
        "room_post" | "room_read" => crate::ui_bridge::calling_session(auth).map(String::as_str),
        "assistant_remember" | "assistant_forget" | "assistant_recall" => {
            auth.mcp_session_id.as_deref()
        }
        _ => None,
    };
    let rebound;
    let arguments = match bound_session {
        Some(sid) => {
            let mut a = arguments.clone();
            if let Some(o) = a.as_object_mut() {
                o.insert("session_id".into(), json!(sid));
            }
            rebound = a;
            &rebound
        }
        _ => arguments,
    };
    // Agent UI control: routed to the session owner's Otto window by the
    // bridge (session-bound, grant-gated, target-picked), not a self-call —
    // it needs `auth`'s session binding, which `execute_otto_tool` discards.
    if is_ui {
        let r = crate::ui_bridge::run(ctx, auth, &short, arguments).await;
        let latency = started.elapsed().as_millis() as i64;
        return Ok(match r {
            Ok(value) => {
                let bytes = serde_json::to_vec(&value)
                    .map(|v| v.len() as i64)
                    .unwrap_or(0);
                let _ = ctx
                    .mcp
                    .call_log()
                    .finalize(&audit_id, true, None, Some(latency), Some(bytes), None)
                    .await;
                json!({"decision":"allowed","executed":true,"content":value})
            }
            Err(e) => {
                let msg = otto_core::redact::redact_text(&e.message).value;
                let _ = ctx
                    .mcp
                    .call_log()
                    .finalize(
                        &audit_id,
                        false,
                        Some(&format!("{}: {msg}", e.code)),
                        Some(latency),
                        None,
                        None,
                    )
                    .await;
                ui_error_envelope(&e.code, &msg)
            }
        });
    }
    let result = match directory_kind(&short) {
        // Cross-workspace list tools: the `agent_refs` directory (as the
        // caller, pin applied), not a single-workspace route.
        Some(kind) => {
            let ws = arguments
                .get("workspace_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            crate::agent_refs::directory_json(ctx, auth, kind, ws, session_ws.as_deref().or(ws))
                .await
        }
        None => execute_otto_tool(ctx, user, &short, arguments).await,
    };
    let latency = started.elapsed().as_millis() as i64;
    match result {
        Ok(value) => {
            let bytes = serde_json::to_vec(&value)
                .map(|v| v.len() as i64)
                .unwrap_or(0);
            let _ = ctx
                .mcp
                .call_log()
                .finalize(&audit_id, true, None, Some(latency), Some(bytes), None)
                .await;
            let mut env = json!({"decision":"allowed","executed":true,"content":value});
            if let Some(by) = &auto_approved_by {
                env["auto_approved_by"] = by.clone();
            }
            Ok(env)
        }
        Err(e) => {
            let err = otto_core::redact::redact_text(&e.to_string()).value;
            let _ = ctx
                .mcp
                .call_log()
                .finalize(&audit_id, false, Some(&err), Some(latency), None, None)
                .await;
            let mut env =
                json!({"decision":"error","executed":true,"is_error":true,"content":{"error":err}});
            if let Some(by) = &auto_approved_by {
                env["auto_approved_by"] = by.clone();
            }
            Ok(env)
        }
    }
}

async fn deny_audit(ctx: &ServerCtx, audit: &mut NewCallLog, reason: &str) -> Value {
    audit.decision = "denied".into();
    audit.decision_reason = Some(reason.to_string());
    let _ = ctx.mcp.call_log().insert(audit.clone()).await;
    json!({"decision":"denied","executed":false,"reason":reason})
}

/// Audit + envelope for a reference (repo, workflow, account, …) that did not resolve (unknown or
/// ambiguous). Nothing executed, so it is an `error` row, not a denial; the
/// message lists the candidates / what IS available so the agent can retry
/// with a concrete id in one step.
async fn unresolved_audit(ctx: &ServerCtx, audit: &mut NewCallLog, err: &Error) -> Value {
    let msg = otto_core::redact::redact_text(&err.to_string()).value;
    audit.decision = "error".into();
    audit.decision_reason = Some("reference did not resolve".into());
    audit.error = Some(msg.clone());
    let _ = ctx.mcp.call_log().insert(audit.clone()).await;
    json!({"decision":"error","executed":false,"is_error":true,"content":{"error":msg}})
}

/// Wait (bounded) for a human decision on an approval. `Some(true)`=approved,
/// `Some(false)`=denied/expired/cancelled, `None`=still pending after the wait
/// (caller resubmits later).
///
/// Event-driven: it subscribes to the bus BEFORE the first status read, then
/// re-reads only when an `mcp_approval_changed` names this approval (or a bulk
/// expiry, `approval_id: None`) — so a decision resumes the call immediately
/// instead of up to 1 s late, and a waiting call issues a few reads instead
/// of one per second. A slow fallback re-read ([`APPROVAL_FALLBACK_TICK`])
/// covers a writer that bypassed the hook (another process) and a lagged or
/// closed receiver.
async fn wait_for_decision(
    ctx: &ServerCtx,
    approval_id: &str,
    wait_seconds: Option<u64>,
) -> Option<bool> {
    let budget = Duration::from_secs(wait_seconds.unwrap_or(0).min(MAX_WAIT_SECS));
    // Undecided on daemon shutdown: the caller gets its normal "still
    // pending" answer instead of pinning the HTTP drain for up to 30 s.
    tokio::select! {
        decided = wait_for_decision_within(ctx, approval_id, budget, APPROVAL_FALLBACK_TICK) => decided,
        _ = crate::shutdown::cancelled() => None,
    }
}

/// Fallback re-read period for [`wait_for_decision`] when no event arrives.
const APPROVAL_FALLBACK_TICK: Duration = Duration::from_secs(5);

async fn wait_for_decision_within(
    ctx: &ServerCtx,
    approval_id: &str,
    budget: Duration,
    fallback: Duration,
) -> Option<bool> {
    let mut rx = ctx.events.subscribe();
    let deadline = tokio::time::Instant::now() + budget;
    let id = approval_id.to_string();
    loop {
        if let Ok(a) = ctx.mcp.approvals().get(&id).await {
            match a.status.as_str() {
                "approved" => return Some(true),
                "denied" | "expired" | "cancelled" => return Some(false),
                _ => {}
            }
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return None;
        }
        let tick = (now + fallback).min(deadline);
        // Wait for a relevant change, the fallback tick, or the deadline —
        // whichever is first — then re-read.
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Ok(Event::McpApprovalChanged { approval_id, .. })
                        if approval_id.as_deref().is_none_or(|a| a == id) => break,
                    Ok(_) => continue,
                    // Lagged: a change may have been dropped — re-read now.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => break,
                    // Bus gone: plain polling at the fallback period.
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tokio::time::sleep_until(tick).await;
                        break;
                    }
                },
                _ = tokio::time::sleep_until(tick) => break,
            }
        }
    }
}

// ===========================================================================
// The executor — runs each tool AS the user via an ephemeral self-call.
// ===========================================================================

/// Vaults are a GLOBAL library (`otto_vault::VaultEngine::list`): the `{ws}`
/// segment on the vault REST routes only picks the workspace the caller's role
/// is checked in — it never narrows which vault a `vault_id` addresses. Outward
/// callers therefore don't have to know a workspace at all: when a `vault_*`
/// call omits `workspace_id`, scope it to the token's workspace pin when one is
/// set, else the caller's first accessible workspace (preferring one they can
/// write in when the tool mutates). Returns `None` when the args need no
/// filling. Runs BEFORE the scope check so a pinned token's filled call still
/// faces `McpScope::deny_reason` with the pin satisfied — never bypassed.
/// [`DESIGN_WS_TOOLS`] arguments with `workspace_id` set to the target
/// artifact's own workspace (overriding any caller value). `None` when the
/// tool doesn't address one artifact or it doesn't resolve (the self-call then
/// answers 404 under the caller's own RBAC).
async fn fill_design_workspace(ctx: &ServerCtx, tool: &str, args: &Value) -> Option<Value> {
    if !DESIGN_WS_TOOLS.contains(&tool) || !args.is_object() {
        return None;
    }
    let id = args
        .get("artifact_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let a = crate::design_hall::service(ctx)
        .store()
        .get_artifact(id)
        .await
        .ok()
        .flatten()?;
    let mut filled = args.clone();
    filled["workspace_id"] = json!(a.workspace_id);
    Some(filled)
}

async fn fill_vault_workspace(
    ctx: &ServerCtx,
    auth: &AuthContext,
    tool: &str,
    args: &Value,
) -> Result<Option<Value>, Error> {
    let has_ws = args
        .get("workspace_id")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if !tool.starts_with("vault_") || has_ws {
        return Ok(None);
    }
    let user = &auth.effective_user;
    if let Some(pin) = auth
        .mcp_scope
        .as_ref()
        .and_then(|s| s.workspace_id.as_deref())
        .filter(|s| !s.is_empty())
    {
        let mut filled = args.clone();
        filled["workspace_id"] = json!(pin);
        return Ok(Some(filled));
    }
    let repo = otto_state::WorkspacesRepo::new(ctx.pool.clone());
    // User-facing variants: the system-owned scratch workspace must never be
    // picked here — for root it is the oldest row, and a vault rooted at
    // `$HOME` is not what an omitted `workspace_id` means.
    let rows: Vec<(otto_core::domain::Workspace, WorkspaceRole)> = if user.is_root {
        repo.list_user_all()
            .await?
            .into_iter()
            .map(|w| (w, WorkspaceRole::Admin))
            .collect()
    } else {
        repo.list_user_for_user(&user.id).await?
    };
    let ws = pick_vault_workspace(&rows, tool_is_mutating(tool)).ok_or_else(|| {
        Error::Invalid(
            "no accessible workspace to scope this vault call — pass 'workspace_id'".into(),
        )
    })?;
    let mut filled = args.clone();
    filled["workspace_id"] = json!(ws);
    Ok(Some(filled))
}

// ===========================================================================
// Friendly references + the workspace pin for every non-git tool
// ===========================================================================

/// The directory kind a governed list tool is served by, if any.
fn directory_kind(tool: &str) -> Option<&'static crate::agent_refs::RefKind> {
    DIRECTORY_TOOLS
        .iter()
        .find(|(t, _)| *t == tool)
        .and_then(|(_, k)| crate::agent_refs::kind_of(k))
}

/// Whether [`fill_refs`] needs directory lookups for this call (so a plain-id
/// call from an unpinned caller costs nothing extra). Pure — unit-tested.
fn refs_need_lookup(tool: &str, args: &Value, pinned: bool) -> bool {
    let s = |k: &str| -> Option<String> {
        match args.get(k)? {
            Value::String(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    if s("workspace_id").is_some_and(|w| !crate::agent_refs::looks_like_id(&w)) {
        return true;
    }
    for (t, arg, kind) in REF_ARGS {
        if *t != tool {
            continue;
        }
        let Some(k) = crate::agent_refs::kind_of(kind) else {
            continue;
        };
        match s(arg) {
            None => {
                if k.sole_default {
                    return true;
                }
            }
            Some(v) => {
                if !crate::agent_refs::looks_like_id(&v)
                    || (pinned && k.scope == crate::agent_refs::Scope::Workspace)
                {
                    return true;
                }
            }
        }
    }
    if tool == "transition_issue"
        && s("transition_id").is_some_and(|t| !t.bytes().all(|b| b.is_ascii_digit()))
    {
        return true;
    }
    pinned
        && s("workspace_id").is_none()
        && PIN_PROBES
            .iter()
            .any(|(t, arg, _, _)| *t == tool && s(arg).is_some())
}

/// Resolve every friendly reference in a governed call (see [`REF_ARGS`]), a
/// workspace NAME in `workspace_id`, a Confluence page URL, a Jira transition
/// name, and — for a pinned token — an id-only object's workspace
/// ([`PIN_PROBES`]) and the pin narrowing of [`PIN_NARROW_TOOLS`]. `None` when
/// the arguments are already canonical. Errors: `Forbidden` (pin / RBAC),
/// `NotFound` / `Conflict` listing the candidates.
async fn fill_refs(
    ctx: &ServerCtx,
    auth: &AuthContext,
    tool: &str,
    args: &Value,
) -> Result<Option<Value>, Error> {
    let Some(obj) = args.as_object() else {
        return Ok(None);
    };
    let pin = crate::agent_refs::pin_of(auth);
    let mut out = obj.clone();
    let mut changed = false;

    // Pure normalizations first (no lookups).
    for key in ["page_id", "parent_id"] {
        if let Some(raw) = out.get(key).and_then(Value::as_str) {
            if let Some(id) = confluence_page_id(raw).filter(|id| id != raw) {
                out.insert(key.to_string(), json!(id));
                changed = true;
            }
        }
    }
    let has_ws = out
        .get("workspace_id")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if let (Some(p), false) = (pin, has_ws) {
        if PIN_NARROW_TOOLS.contains(&tool) {
            out.insert("workspace_id".into(), json!(p));
            changed = true;
        }
    }

    let current = Value::Object(out.clone());
    if !refs_need_lookup(tool, &current, pin.is_some()) {
        return Ok(changed.then_some(current));
    }
    let prefer = crate::agent_refs::caller_session_ws(ctx, auth).await;
    let caller = crate::agent_refs::SelfCaller::open(ctx, &auth.effective_user).await?;
    let r = fill_refs_with(&caller, auth, tool, out, prefer.as_deref()).await;
    caller.close().await;
    r.map(Some)
}

/// The lookup half of [`fill_refs`], over an open caller.
async fn fill_refs_with(
    caller: &crate::agent_refs::SelfCaller,
    auth: &AuthContext,
    tool: &str,
    mut out: serde_json::Map<String, Value>,
    prefer: Option<&str>,
) -> Result<Value, Error> {
    use crate::agent_refs::{kind_of, looks_like_id, resolve_with, Scope};
    let pinned = crate::agent_refs::pin_of(auth).is_some();
    let text = |m: &serde_json::Map<String, Value>, k: &str| -> Option<String> {
        match m.get(k)? {
            Value::String(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };

    // 1. A workspace given by NAME.
    if let Some(ws) = text(&out, "workspace_id").filter(|w| !looks_like_id(w)) {
        let kind = kind_of("workspace").ok_or_else(|| Error::Internal("kind workspace".into()))?;
        let (c, _) = resolve_with(
            caller,
            auth,
            kind,
            "workspace_id",
            Some(ws.as_str()),
            None,
            prefer,
        )
        .await?;
        out.insert("workspace_id".into(), json!(c.id));
    }

    // 2. Friendly id arguments.
    for (t, arg, kind_key) in REF_ARGS {
        if *t != tool {
            continue;
        }
        let Some(kind) = kind_of(kind_key) else {
            continue;
        };
        let value = text(&out, arg);
        let needs = match &value {
            None => kind.sole_default,
            Some(v) => !looks_like_id(v) || (pinned && kind.scope == Scope::Workspace),
        };
        if !needs {
            continue;
        }
        let ws_filter = match kind.scope {
            Scope::Workspace => text(&out, "workspace_id"),
            _ => None,
        };
        let (c, _) = resolve_with(
            caller,
            auth,
            kind,
            arg,
            value.as_deref(),
            ws_filter.as_deref(),
            prefer,
        )
        .await?;
        let id = if kind.key == "vault" {
            c.id.parse::<i64>()
                .map(|n| json!(n))
                .unwrap_or_else(|_| json!(c.id))
        } else {
            json!(c.id)
        };
        out.insert((*arg).to_string(), id);
        if kind.scope == Scope::Workspace && ws_filter.is_none() {
            if let Some(ws) = &c.workspace_id {
                out.insert("workspace_id".into(), json!(ws));
            }
        }
    }

    // 3. A Jira transition given by name / target status.
    if tool == "transition_issue" {
        if let (Some(acc), Some(key), Some(tr)) = (
            text(&out, "account_id"),
            text(&out, "key"),
            text(&out, "transition_id"),
        ) {
            if !tr.bytes().all(|b| b.is_ascii_digit()) {
                let list = caller
                    .get(&format!(
                        "/api/v1/issue/{}/{}/transitions",
                        seg(&acc),
                        seg(&key)
                    ))
                    .await?;
                out.insert("transition_id".into(), json!(match_transition(&list, &tr)?));
            }
        }
    }

    // 4. Pinned token, id-only object: learn its real workspace.
    if pinned && text(&out, "workspace_id").is_none() {
        if let Some((_, arg, prefix, ptr)) = PIN_PROBES.iter().find(|(t, ..)| *t == tool) {
            if let Some(id) = text(&out, arg) {
                let v = caller.get(&format!("{prefix}{}", seg(&id))).await?;
                if let Some(ws) = v.pointer(ptr).and_then(Value::as_str) {
                    out.insert("workspace_id".into(), json!(ws));
                }
            }
        }
    }
    Ok(Value::Object(out))
}

/// Resolved arguments for a git tool (see [`fill_repo_ref`]).
struct RepoFill {
    args: Value,
    /// `name (workspace: X)` of the resolved repo, for the approval prompt.
    label: Option<String>,
}

/// The repo analogue of [`fill_vault_workspace`]. A repo is registered in
/// exactly ONE workspace, but the caller (often an agent session opened in a
/// different one) knows it by name, path or remote — so for
/// [`REPO_REF_TOOLS`] the `repo_id` argument is resolved across every
/// workspace the caller can read (`repo_directory::resolve_repo`: Git:View +
/// workspace Viewer, narrowed to a token's workspace pin; omitted → the
/// calling session's cwd) and rewritten to the canonical id, with the repo's
/// OWN `workspace_id` added so the pin check, the approval's workspace and
/// the args-hash all key on where the repo actually lives. `list_repos`
/// without a `workspace_id` from a pinned token is narrowed to the pin — the
/// executor's self-call is unpinned and would otherwise list every workspace.
/// `None` when the tool needs no filling.
async fn fill_repo_ref(
    ctx: &ServerCtx,
    auth: &AuthContext,
    tool: &str,
    args: &Value,
) -> Result<Option<RepoFill>, Error> {
    let ws_arg = args
        .get("workspace_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    if tool == "list_repos" {
        let pin = auth
            .mcp_scope
            .as_ref()
            .and_then(|s| s.workspace_id.as_deref())
            .filter(|s| !s.is_empty());
        return Ok(match (ws_arg, pin) {
            (None, Some(pin)) => {
                let mut filled = args.clone();
                filled["workspace_id"] = json!(pin);
                Some(RepoFill {
                    args: filled,
                    label: None,
                })
            }
            _ => None,
        });
    }
    if !REPO_REF_TOOLS.contains(&tool) {
        return Ok(None);
    }
    let reference = args.get("repo_id").and_then(Value::as_str);
    let resolved = crate::repo_directory::resolve_repo(ctx, auth, reference, ws_arg).await?;
    let mut filled = args.clone();
    filled["repo_id"] = json!(resolved.entry.repo.id);
    filled["workspace_id"] = json!(resolved.entry.repo.workspace_id);
    Ok(Some(RepoFill {
        args: filled,
        label: Some(resolved.label()),
    }))
}

pub(crate) async fn execute_otto_tool(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    tool: &str,
    args: &Value,
) -> Result<Value, Error> {
    if tool == "ask_human_approval" {
        return ask_human_approval(ctx, user, args).await;
    }
    // A short-lived token of the effective user, so the self-call reuses the
    // target endpoint's native RBAC. Reused across calls for a minute and
    // revoked once aged out and released (see `self_call`), and the HTTP
    // client — with its keep-alive loopback connections — is shared: a call
    // no longer costs a token INSERT + DELETE, an auth-cache miss, a TLS
    // config build and a TCP connect (r3-06-04).
    let lease = crate::self_call::lease(
        &ctx.pool,
        &ctx.base_url,
        &user.id,
        crate::self_call::LABEL_EXEC,
    )
    .await?;
    let client = crate::self_call::client();
    let base = ctx.base_url.trim_end_matches('/').to_string();
    let budget = call_timeout(tool, args);
    let call = async {
        if tool == "api_upsert_request" {
            upsert_request_preserving(client, &base, lease.token(), args).await
        } else {
            run_tool(client, &base, lease.token(), tool, args).await
        }
    };
    let result = match tokio::time::timeout(budget, call).await {
        Ok(r) => r,
        Err(_) => Err(Error::Upstream(format!(
            "self-call: timed out after {}s",
            budget.as_secs()
        ))),
    };
    drop(lease);
    result
}

async fn ask_human_approval(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    args: &Value,
) -> Result<Value, Error> {
    let title = arg_str(args, "title")?;
    let ws = args
        .get("workspace_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let detail = args
        .get("detail")
        .and_then(Value::as_str)
        .map(str::to_string);
    let appr = ctx
        .mcp
        .approvals()
        .create(NewApproval {
            workspace_id: ws,
            kind: "human_ask".into(),
            server_id: None,
            server_name: Some("otto".into()),
            tool: Some("otto.ask_human_approval".into()),
            title,
            detail,
            args_redacted_json: otto_core::redact::redact_json(args).value.to_string(),
            args_hash: None,
            risk_label: None,
            requested_by: Some(user.id.clone()),
            requested_by_kind: Some("mcp_server".into()),
            requested_by_session_id: None,
            expires_at: Some((chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339()),
        })
        .await?;
    let wait = args.get("wait_seconds").and_then(u64_lenient);
    let decided = wait_for_decision(ctx, &appr.id, wait).await;
    Ok(json!({
        "approval_id": appr.id,
        "status": match decided { Some(true) => "approved", Some(false) => "denied", None => "pending" },
        "note": "poll the MCP approvals queue, or pass wait_seconds (≤30) to block briefly",
    }))
}

// ===========================================================================
// GET / PATCH /mcp/otto-server  (status + config + token mint)
// ===========================================================================

pub async fn otto_server_status(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let enabled = outward_enabled(&ctx).await;
    let on = enabled_tools(&ctx).await;
    let exempt = approval_exempt_tools(&ctx).await;
    let rules = otto_state::McpAutoApproveRepo::new(ctx.pool.clone())
        .list()
        .await
        .map_err(ApiError)?;
    let tools: Vec<Value> = otto_tool_specs_cached()
        .iter()
        .map(|t| {
            let name = t["name"].as_str().unwrap_or("").to_string();
            let short = name.strip_prefix("otto.").unwrap_or(&name).to_string();
            let gated = tool_is_dangerous(&short);
            json!({
                "name": name,
                "description": t["description"],
                "mutating": t["mutating"],
                "category": t["category"],
                "enabled": on.contains(&short),
                // Auto-approved EVERYWHERE (an enabled global per-tool rule —
                // the catalog's Auto-approve switch): calls skip the approval
                // gate and are audited `auto_approved`.
                "approval_exempt": exempt.contains(&short),
                // The guardrail tier: no category rule reaches it; a per-tool
                // rule needs the second toggle (`allow_irreversible`).
                "irreversible": tool_is_irreversible(&short),
                // Every enabled rule (any scope) that auto-approves this tool
                // somewhere — what the catalog shows as badges.
                "auto_approved_by": if gated {
                    crate::mcp_auto_approve::rules_covering_tool(&rules, &short)
                } else {
                    Vec::new()
                },
            })
        })
        .collect();
    let prefix = AuthRepo::new(ctx.pool.clone())
        .mcp_token_prefix(&user.id)
        .await
        .map_err(ApiError)?;
    let require_dangerous = require_approval_dangerous(&ctx).await;
    Ok(Json(json!({
        "enabled": enabled,
        "tools": tools,
        "has_token": prefix.is_some(),
        "token_prefix": prefix,
        // The global switch the per-tool gate sits under: when false no
        // otto.* call asks, whatever the per-tool setting says.
        "require_approval_dangerous": require_dangerous,
        "approval_exempt_tools": exempt,
    })))
}

/// `GET /mcp/otto-server/enabled` — the LIGHT read behind every agent
/// session's `tools/list` and governed stdio call: just the ENABLED full
/// `otto.*` names (catalog order), the master switch, and whether the CALLING
/// session holds the per-session "Allow UI control" grant (`ui_granted`;
/// false for a credential not bound to a session). Same MCP View policy as
/// [`otto_server_status`], which also builds per-tool rule badges, the token
/// prefix and ~100 KB of descriptions the bridge threw away. Never a gate by
/// itself: `governed_invoke` re-checks the enable list on every call, and the
/// UI bridge re-checks the grant.
pub async fn otto_server_enabled(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<Value>> {
    let on = enabled_tools(&ctx).await;
    let outward_on = outward_enabled(&ctx).await;
    let enabled: Vec<&str> = otto_tool_specs_cached()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .filter(|name| {
            on.iter()
                .any(|e| e == name.strip_prefix("otto.").unwrap_or(name))
        })
        .collect();
    let ui_granted = match crate::ui_bridge::calling_session(&auth) {
        Some(sid) => ctx
            .manager
            .get(sid)
            .await
            .ok()
            .is_some_and(|s| crate::ui_bridge::grant_state(&s.meta) == Some(true)),
        None => false,
    };
    Ok(Json(json!({
        "enabled": enabled,
        "outward_enabled": outward_on,
        "ui_granted": ui_granted,
    })))
}

/// Body of `POST /mcp/tool-calls` — one stdio-bridge tool call.
#[derive(Deserialize)]
pub struct ToolCallAuditReq {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    pub ok: bool,
    #[serde(default)]
    pub rows: Option<i64>,
}

/// `POST /mcp/tool-calls` — append one row to the stdio bridge's tool-call
/// ledger (`mcp_tool_calls`), on behalf of the CALLING agent session
/// (perf2/10-mcp R7). The bridge used to open its own writer connection to
/// the live database for this — one per agent session, contending with the
/// daemon's writers. Only a session credential may append, and the row is
/// stamped with that session and its workspace (never a body field), so a
/// session cannot write rows for another. Arguments are redacted here and
/// capped by the repo. `204` on success.
pub async fn record_tool_call(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<ToolCallAuditReq>,
) -> ApiResult<axum::http::StatusCode> {
    let Some(sid) = crate::ui_bridge::calling_session(&auth) else {
        return Err(ApiError(otto_core::Error::Forbidden(
            "only an agent session's credential records its tool calls".into(),
        )));
    };
    let tool = req.tool.trim();
    if tool.is_empty() || tool.len() > 200 {
        return Err(ApiError(otto_core::Error::Invalid(
            "tool must be 1–200 characters".into(),
        )));
    }
    let workspace_id = ctx.manager.get(sid).await.ok().map(|s| s.workspace_id);
    otto_state::McpAuditRepo::new(ctx.pool.clone())
        .record(otto_state::NewMcpToolCall {
            workspace_id,
            session_id: Some(sid.clone()),
            tool: tool.to_string(),
            args_json: otto_core::redact::redact_json(&req.arguments)
                .value
                .to_string(),
            ok: req.ok,
            rows: req.rows,
        })
        .await
        .map_err(ApiError)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct OttoServerConfigReq {
    pub enabled: Option<bool>,
    pub tools: Option<Vec<String>>,
    /// Compatibility shim over the auto-approve rules: the COMPLETE set of
    /// mutating tools auto-approved EVERYWHERE (global per-tool rules); missing
    /// rules are created, enabled ones not listed are deleted. Bare or
    /// `otto.`-prefixed names; non-mutating/unknown names are a 400, and so is
    /// an irreversible tool without an existing rule (that needs the second
    /// toggle — `POST /mcp/auto-approve {allow_irreversible:true}`).
    pub approval_exempt_tools: Option<Vec<String>>,
    #[serde(default)]
    pub rotate_token: bool,
}

pub async fn otto_server_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<OttoServerConfigReq>,
) -> ApiResult<Json<Value>> {
    // Auto-approving tools is a person's decision (never an agent session's own).
    if req.approval_exempt_tools.is_some() {
        crate::mcp_auto_approve::require_human(&auth).map_err(ApiError)?;
    }
    let settings = SettingsRepo::new(ctx.pool.clone());
    // Validate the exemption list BEFORE any write, so a bad name never
    // leaves a half-applied config behind.
    let requested_exempt = req
        .approval_exempt_tools
        .as_deref()
        .map(normalize_exempt_tools)
        .transpose()
        .map_err(ApiError)?;
    if let Some(list) = &requested_exempt {
        // The irreversible guardrail: this shim cannot carry the second toggle,
        // so it may only KEEP an irreversible tool that is already auto-approved.
        let current = approval_exempt_tools(&ctx).await;
        if let Some(t) = list
            .iter()
            .find(|t| tool_is_irreversible(t) && !current.contains(t))
        {
            return Err(ApiError(Error::Invalid(format!(
                "'{t}' is irreversible — auto-approving it needs the second explicit toggle: \
                 POST /mcp/auto-approve with allow_irreversible: true"
            ))));
        }
    }
    if let Some(en) = req.enabled {
        settings
            .put("mcp_otto_server_enabled", &json!(en))
            .await
            .map_err(ApiError)?;
    }
    if let Some(tools) = &req.tools {
        let known: Vec<String> = otto_tool_specs_cached()
            .iter()
            .filter_map(|t| {
                t["name"]
                    .as_str()
                    .map(|n| n.strip_prefix("otto.").unwrap_or(n).to_string())
            })
            .collect();
        // The UI sends full `otto.*` names; the read path (`enabled_tools`) keys on
        // the bare name. Accept either form, validate + STORE the bare name so the
        // stored set matches what the dispatcher/status compare against.
        let mut normalized: Vec<String> = Vec::with_capacity(tools.len());
        for t in tools {
            let bare = t.strip_prefix("otto.").unwrap_or(t).to_string();
            if !known.contains(&bare) {
                return Err(ApiError(Error::Invalid(format!("unknown otto tool '{t}'"))));
            }
            normalized.push(bare);
        }
        settings
            .put("mcp_otto_server_tools", &json!(normalized))
            .await
            .map_err(ApiError)?;
        // The operator has now seen every current UI tool: from here on only
        // the ones left checked are enabled (see `merge_enabled`).
        settings
            .put(UI_TOOLS_KNOWN_KEY, &json!(crate::ui_commands::tool_names()))
            .await
            .map_err(ApiError)?;
    }
    // Auto-approve, everywhere-per-tool: an explicit list replaces the set of
    // global per-tool rules (audited — it loosens the posture); either way every
    // per-tool rule (any scope) of a tool that is no longer enabled is dropped,
    // so re-enabling a tool starts from the secure default (gated).
    if requested_exempt.is_some() || req.tools.is_some() {
        let current_exempt = approval_exempt_tools(&ctx).await;
        let enabled_now = enabled_tools(&ctx).await;
        let next = prune_exempt_tools(
            requested_exempt.as_ref().unwrap_or(&current_exempt),
            &enabled_now,
        );
        crate::mcp_auto_approve::sync_catalog_toggles(&ctx, &user, &next, &enabled_now)
            .await
            .map_err(ApiError)?;
        if next != current_exempt {
            ctx.audit(otto_state::NewAuditEntry {
                user_id: Some(user.id.clone()),
                action: "mcp.otto_server.approval_exempt".into(),
                target: None,
                detail: Some(json!({ "from": current_exempt, "to": next })),
                ip: None,
            })
            .await;
        }
    }
    let mut minted: Option<String> = None;
    if req.rotate_token {
        let repo = AuthRepo::new(ctx.pool.clone());
        repo.revoke_mcp_tokens(&user.id).await.map_err(ApiError)?;
        minted = Some(
            repo.issue_mcp_token(&user.id, Some("otto-mcp-server"))
                .await
                .map_err(ApiError)?,
        );
        ctx.audit(otto_state::NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: "mcp.otto_server.token_mint".into(),
            target: None,
            detail: None,
            ip: None,
        })
        .await;
    }
    let mut status = otto_server_status(State(ctx), CurrentUser(user)).await?;
    if let Some(tok) = minted {
        status.0["token"] = json!(tok);
    }
    Ok(status)
}

// ===========================================================================
// MCP `tools/list` projection (scope-aware) — shared by the HTTP transport.
// ===========================================================================

/// The MCP `tools/list` payload for a caller, as `[{name, description,
/// inputSchema}]`. A tool appears iff it is in the server's globally-enabled set
/// AND permitted by the caller's per-token [`McpScope`] (`None` ⇒ no per-token
/// narrowing — a normal token sees every enabled tool). This is what makes a
/// read-only token never even *see* a mutating tool, and a workspace/tool-scoped
/// token see only its slice. Listing is independent of the master on/off switch
/// (so clients can introspect); execution still honours it in [`governed_invoke`].
pub(crate) async fn mcp_tools_list(ctx: &ServerCtx, scope: Option<&McpScope>) -> Vec<Value> {
    let enabled = enabled_tools(ctx).await;
    otto_tool_specs_cached()
        .iter()
        .filter(|s| {
            let name = s.get("name").and_then(Value::as_str).unwrap_or("");
            let bare = name.strip_prefix("otto.").unwrap_or(name);
            if !enabled.iter().any(|e| e == bare) {
                return false;
            }
            match scope {
                None => true,
                Some(sc) => sc.deny_reason(bare, tool_is_mutating(bare), None).is_none(),
            }
        })
        .map(|s| json!({"name": s["name"], "description": s["description"], "inputSchema": s["inputSchema"]}))
        .collect()
}

// ===========================================================================
// MCP token management — multiple scoped tokens per user (the access layer).
//   GET    /mcp/tokens        list all (admin)
//   POST   /mcp/tokens        mint a scoped token (admin)
//   DELETE /mcp/tokens/{id}   revoke one (admin)
// ===========================================================================

pub async fn list_mcp_tokens(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let tokens = AuthRepo::new(ctx.pool.clone())
        .list_mcp_tokens()
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({ "tokens": tokens })))
}

pub async fn create_mcp_token(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateMcpTokenReq>,
) -> ApiResult<Json<Value>> {
    // Owner: defaults to the caller. Minting a token for ANOTHER user hands out a
    // credential that authenticates AS that user, so only root may do it (a non-root
    // mcp:admin minting a root-owned token would otherwise be a privilege
    // escalation — mirrors the impersonation "no minting up/sideways" rule).
    let owner = req.user_id.clone().unwrap_or_else(|| user.id.clone());
    if owner != user.id && !user.is_root {
        return Err(ApiError(Error::Forbidden(
            "only root may mint an MCP token owned by another user".into(),
        )));
    }
    // Validate the scope's tool list against the live catalog so a typo can't
    // silently produce a token that reaches nothing / drifts from the UI.
    let scope = req.scope.clone().unwrap_or_else(McpScope::unrestricted);
    if let Some(tools) = &scope.tools {
        let known: Vec<String> = otto_tool_specs_cached()
            .iter()
            .filter_map(|t| {
                t["name"]
                    .as_str()
                    .map(|n| n.strip_prefix("otto.").unwrap_or(n).to_string())
            })
            .collect();
        for t in tools {
            let bare = t.strip_prefix("otto.").unwrap_or(t);
            if !known.iter().any(|k| k == bare) {
                return Err(ApiError(Error::Invalid(format!("unknown otto tool '{t}'"))));
            }
        }
    }
    // Normalize tool names to bare form so enforcement (which keys on bare names)
    // matches regardless of whether the UI sent `otto.x` or `x`.
    let scope = McpScope {
        tools: scope.tools.map(|ts| {
            ts.iter()
                .map(|t| t.strip_prefix("otto.").unwrap_or(t).to_string())
                .collect()
        }),
        allow_writes: scope.allow_writes,
        workspace_id: scope.workspace_id.filter(|w| !w.is_empty()),
    };
    let (token, info) = AuthRepo::new(ctx.pool.clone())
        .issue_mcp_token_with_scope(&owner, req.label.as_deref(), &scope)
        .await
        .map_err(ApiError)?;
    ctx.audit(otto_state::NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "mcp.token.create".into(),
        target: Some(info.id.clone()),
        detail: Some(json!({ "owner": owner, "allow_writes": scope.allow_writes })),
        ip: None,
    })
    .await;
    Ok(Json(json!({ "token": token, "info": info })))
}

pub async fn revoke_mcp_token(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> ApiResult<axum::http::StatusCode> {
    // Use the SHARED auth cache so the revocation takes effect immediately: the
    // authenticator caches mcp tokens by hash, and `revoke_mcp_token_by_id`
    // evicts the owner from this same cache once it knows who owned the token.
    let removed = AuthRepo::with_cache(ctx.pool.clone(), ctx.auth_cache.clone())
        .revoke_mcp_token_by_id(&id)
        .await
        .map_err(ApiError)?;
    if !removed {
        return Err(ApiError(Error::NotFound("mcp token not found".into())));
    }
    ctx.audit(otto_state::NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "mcp.token.revoke".into(),
        target: Some(id),
        detail: None,
        ip: None,
    })
    .await;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

// ===========================================================================
// Gateway — governs LIVE-AGENT downstream calls through the same pipeline.
// ===========================================================================

#[derive(Deserialize)]
pub struct GatewayToolsQuery {
    pub workspace_id: String,
}

/// `GET /mcp/gateway/tools?workspace_id=` — the governed downstream tools for a
/// workspace, namespaced `mcp__<server>__<tool>`. The inward `ottod mcp-tools`
/// surfaces these to the agent and proxies each call through `/mcp/gateway/invoke`.
pub async fn gateway_tools(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<GatewayToolsQuery>,
) -> ApiResult<Json<Value>> {
    crate::auth::require_ws_role(&ctx, &user, &q.workspace_id, WorkspaceRole::Viewer).await?;
    let servers = ctx
        .mcp
        .registry()
        .list_for_ws(&q.workspace_id)
        .await
        .map_err(ApiError)?;
    let mut tools: Vec<Value> = Vec::new();
    // The caller's capability / groups / membership: read once for every
    // server below, not once per server (perf2/10-mcp R1).
    let mut access = otto_mcp::CallerAccess::default();
    for s in servers.into_iter().filter(|s| s.enabled) {
        let policy = otto_state::ResourceAccessRepo::new(ctx.pool.clone())
            .get_live_policy(otto_core::access::ResourceKind::McpServer, &s.id)
            .await?;
        if !s.managed && policy.mode == otto_core::access::AccessMode::Legacy {
            continue;
        }
        // The live policy loaded above serves every check for this server
        // (server discover + per-tool discover/invoke/configure), so a server
        // costs a fixed handful of statements, not ~5–8 per tool × op.
        if !ctx
            .mcp
            .resource_allowed_with(&mut access, &policy, &s, &user, &[("discover", None)])
            .await?[0]
        {
            continue;
        }
        for t in ctx
            .mcp
            .visible_tools_with(&mut access, &policy, &s, &user)
            .await
            .map_err(ApiError)?
            .into_iter()
            .filter(|t| t.enabled)
        {
            tools.push(json!({
                "name": format!("mcp__{}__{}", s.name, t.name),
                "server_id": s.id,
                "server_name": s.name,
                "tool": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
                "risk_label": t.risk_label,
            }));
        }
    }
    Ok(Json(json!({ "tools": tools })))
}

#[derive(Deserialize)]
pub struct GatewayInvokeReq {
    pub server_id: Id,
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub dry_run: bool,
    pub workspace_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

/// `POST /mcp/gateway/invoke` — run a downstream call through the SAME governance
/// pipeline (allowlist→policy→approval→dry-run→execute→audit), tagged
/// `caller_kind='gateway'`. This is what puts the control plane in the path of a
/// live agent's every downstream MCP call.
pub async fn gateway_invoke(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GatewayInvokeReq>,
) -> ApiResult<Json<Value>> {
    let server = ctx.mcp.registry().get(&req.server_id).await?;
    if server.workspace_id != req.workspace_id {
        return Err(Error::Forbidden("MCP workspace mismatch".into()).into());
    }
    let policy = otto_state::ResourceAccessRepo::new(ctx.pool.clone())
        .get_policy(otto_core::access::ResourceKind::McpServer, &server.id)
        .await?;
    let role = if policy.mode == otto_core::access::AccessMode::Legacy {
        WorkspaceRole::Editor
    } else {
        WorkspaceRole::Viewer
    };
    crate::auth::require_ws_role(&ctx, &user, &server.workspace_id, role).await?;
    if policy.mode == otto_core::access::AccessMode::Legacy {
        otto_state::GrantsRepo::new(ctx.pool.clone())
            .check_global(
                &user,
                otto_core::domain::Feature::Mcp,
                otto_core::domain::Capability::Edit,
                "legacy MCP invocation requires edit",
            )
            .await?;
    }
    if !ctx
        .mcp
        .resource_allowed(&server, &user, "invoke", Some(&req.tool))
        .await?
    {
        return Err(Error::Forbidden("MCP tool access denied".into()).into());
    }
    let _ = &req.session_id;
    let ictx = InvokeCtx {
        workspace_id: Some(req.workspace_id.clone()),
        dry_run: req.dry_run,
        caller_user_id: Some(user.id.clone()),
        caller_kind: "gateway".into(),
        direction: "outbound".into(),
    };
    let outcome = ctx
        .mcp
        .invoke(&req.server_id, &req.tool, &req.arguments, &ictx)
        .await
        .map_err(ApiError)?;
    let resp = otto_mcp::outcome_to_resp(outcome);
    if resp.decision == "pending_approval" {
        let _ = ctx.events.send(otto_core::event::Event::Notice {
            level: "warn".into(),
            title: "MCP approval needed".into(),
            body: format!("A governed MCP tool '{}' is awaiting approval.", req.tool),
        });
    }
    Ok(Json(serde_json::to_value(resp).unwrap_or(Value::Null)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The pure catalog / routing / pin tests moved with the code to
    // `otto_mcp::outward`; these need `agent_refs` (server-side).

    #[test]
    fn every_ref_arg_names_a_real_tool_argument_and_kind() {
        let specs = otto_tool_specs();
        for (tool, arg, kind) in REF_ARGS {
            let spec = specs
                .iter()
                .find(|s| s["name"] == format!("otto.{tool}"))
                .unwrap_or_else(|| panic!("REF_ARGS names unknown tool {tool}"));
            assert!(
                spec["inputSchema"]["properties"].get(*arg).is_some(),
                "{tool}: REF_ARGS arg '{arg}' is not in its schema"
            );
            assert!(
                crate::agent_refs::kind_of(kind).is_some(),
                "{tool}: unknown kind {kind}"
            );
        }
        for (tool, kind) in DIRECTORY_TOOLS {
            let spec = specs
                .iter()
                .find(|s| s["name"] == format!("otto.{tool}"))
                .unwrap_or_else(|| panic!("DIRECTORY_TOOLS names unknown tool {tool}"));
            assert!(crate::agent_refs::kind_of(kind).is_some(), "{tool}");
            // Cross-workspace by default: never REQUIRES a workspace.
            let reqd = spec["inputSchema"]["required"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            assert!(
                !reqd.contains(&json!("workspace_id")),
                "{tool} must not require workspace_id"
            );
            assert_eq!(spec["mutating"], json!(false), "{tool}");
        }
        // An issue account may be omitted (sole account), so no Issues tool
        // requires it any more.
        for s in &specs {
            if s["category"] == json!("Issues") {
                let reqd = s["inputSchema"]["required"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                assert!(!reqd.contains(&json!("account_id")), "{}", s["name"]);
            }
        }
    }

    /// The workspace pin must be verifiable for EVERY tool: it either carries
    /// a workspace, resolves one (a named object / repo / artifact / probe),
    /// addresses no workspace-owned object, or is consciously denied to
    /// pinned tokens. A new tool that fits none of these fails here.
    #[test]
    fn every_tool_has_a_workspace_pin_story() {
        for spec in otto_tool_specs() {
            let name = spec["name"].as_str().unwrap();
            let short = name.strip_prefix("otto.").unwrap();
            let has_ws = spec["inputSchema"]["properties"]
                .get("workspace_id")
                .is_some();
            let ws_kind_ref = REF_ARGS.iter().any(|(t, _, k)| {
                *t == short
                    && crate::agent_refs::kind_of(k)
                        .is_some_and(|k| k.scope == crate::agent_refs::Scope::Workspace)
            });
            let covered = has_ws
                || ws_kind_ref
                || REPO_REF_TOOLS.contains(&short)
                || short == "list_repos"
                || DESIGN_WS_TOOLS.contains(&short)
                || PIN_PROBES.iter().any(|(t, ..)| *t == short)
                || pin_global(short)
                || PIN_UNVERIFIABLE.contains(&short);
            assert!(covered, "{short}: no workspace-pin classification");
        }
    }

    #[test]
    fn plain_ids_from_an_unpinned_caller_need_no_lookup() {
        let id = "01KZTKNK3Z8N6VD9Q0MTDQSJ3V";
        assert!(!refs_need_lookup(
            "get_workflow",
            &json!({"workflow_id": id}),
            false
        ));
        assert!(refs_need_lookup(
            "get_workflow",
            &json!({"workflow_id": "Nightly"}),
            false
        ));
        // A pinned token must learn a workspace-owned object's workspace.
        assert!(refs_need_lookup(
            "get_workflow",
            &json!({"workflow_id": id}),
            true
        ));
        // …but not a global row's.
        assert!(!refs_need_lookup(
            "k8s_top",
            &json!({"cluster_id": id}),
            true
        ));
        // Omitted issue account → the sole account is looked up.
        assert!(refs_need_lookup(
            "search_issues",
            &json!({"query":"x"}),
            false
        ));
        // A workspace NAME, a transition NAME, a pinned probe.
        assert!(refs_need_lookup(
            "list_sessions",
            &json!({"workspace_id":"Casino"}),
            false
        ));
        assert!(refs_need_lookup(
            "transition_issue",
            &json!({"account_id": id, "key":"K-1", "transition_id":"Done"}),
            false
        ));
        assert!(refs_need_lookup(
            "get_session",
            &json!({"session_id": id}),
            true
        ));
        assert!(!refs_need_lookup(
            "get_session",
            &json!({"session_id": id}),
            false
        ));
    }
}
