//! MCP **auto-approve rules** — the explicit, opt-in policy under which a
//! MUTATING `otto.*` tool call (e.g. `otto.create_pr`) runs WITHOUT a per-call
//! human approval.
//!
//! - **What**: one tool (`create_pr`) or every mutating tool of a catalog
//!   category (`Git`, `Issues`, `Sessions`, …).
//! - **Where**: `global`, one `workspace` (the workspace the call lands in), or
//!   one agent `session` (the calling Otto session's credential).
//! - **Off by default**: no rule ⇒ the call is approval-gated as before.
//! - **Never silent**: an auto-approved call is audited in `mcp_call_log` with
//!   decision `auto_approved` and a reason naming the rule
//!   (`otto_mcp::auto_approve::audit_reason`); every rule change is written to
//!   the admin audit log (`mcp.auto_approve.*`).
//! - **Guardrail**: an irreversible tool (`mcp_outward::IRREVERSIBLE` — merge a
//!   PR, kubectl ops, produce to a live topic/queue, arbitrary HTTP, hard
//!   deletes) is never covered by a category rule; a per-tool rule covers it
//!   only with the second explicit toggle `allow_irreversible`.
//!
//! Routes (MCP View to read, MCP Admin to change — `policy.rs`):
//! `GET/POST /mcp/auto-approve`, `PATCH/DELETE /mcp/auto-approve/{id}`.
//! The decision itself runs in `mcp_outward::governed_invoke` through
//! [`resolve_for_call`].

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::auth::AuthContext;
use otto_core::domain::User;
use otto_core::Error;
use otto_mcp::auto_approve::{
    self as aa, AutoApproveCall, SCOPE_GLOBAL, SCOPE_SESSION, SCOPE_WORKSPACE, TARGET_CATEGORY,
    TARGET_TOOL,
};
use otto_state::{
    AutoApproveRulePatch, McpAutoApproveRepo, McpAutoApproveRule, NewAutoApproveRule,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::mcp_outward::{
    mutating_categories, tool_category, tool_is_dangerous, tool_is_irreversible,
};
use crate::state::ServerCtx;

fn repo(ctx: &ServerCtx) -> McpAutoApproveRepo {
    McpAutoApproveRepo::new(ctx.pool.clone())
}

/// Rule writes loosen the approval posture, so they need a PERSON's own Otto
/// credential: an agent session's managed token (which authorizes as its
/// owner, often root) or any MCP credential must never auto-approve its own
/// calls. Also applied to the `approval_exempt_tools` shim (`PATCH
/// /mcp/otto-server`).
pub(crate) fn require_human(auth: &AuthContext) -> Result<(), Error> {
    if crate::ui_bridge::is_human(auth) {
        return Ok(());
    }
    Err(Error::Forbidden(
        "auto-approve rules can only be changed by a person signed in to Otto — an agent \
         session's or MCP credential cannot approve its own calls"
            .into(),
    ))
}

/// The call facts for a bare tool name, minus the scope (filled per call).
fn call_for<'a>(short: &'a str) -> AutoApproveCall<'a> {
    AutoApproveCall {
        tool: short,
        category: tool_category(short),
        irreversible: tool_is_irreversible(short),
        workspace_id: None,
        session_id: None,
    }
}

/// Would `rule` cover `short` in its own scope? (Scope-agnostic: the call is
/// placed in the rule's own workspace / session.)
fn rule_covers_tool(rule: &McpAutoApproveRule, short: &str) -> bool {
    let mut call = call_for(short);
    call.workspace_id = rule.workspace_id.as_deref();
    call.session_id = rule.session_id.as_deref();
    aa::covers(rule, &call)
}

/// The catalog's per-tool **Auto-approve** switch is exactly an enabled GLOBAL
/// per-tool rule that actually covers its tool (an irreversible tool needs
/// `allow_irreversible`).
pub(crate) fn is_active_catalog_toggle(rule: &McpAutoApproveRule) -> bool {
    rule.scope == SCOPE_GLOBAL
        && rule.target_kind == TARGET_TOOL
        && rule_covers_tool(rule, &rule.target)
}

/// The short reference to a rule carried by the governed envelope
/// (`auto_approved_by`) and the status catalog.
pub(crate) fn envelope_ref(rule: &McpAutoApproveRule) -> Value {
    json!({
        "id": rule.id,
        "name": rule.name,
        "scope": rule.scope,
        "workspace_id": rule.workspace_id,
        "session_id": rule.session_id,
        "target_kind": rule.target_kind,
        "target": rule.target,
    })
}

/// Every enabled rule (any scope) that auto-approves `short` somewhere — the
/// badges the MCP → Otto server catalog shows under a tool.
pub(crate) fn rules_covering_tool(rules: &[McpAutoApproveRule], short: &str) -> Vec<Value> {
    rules
        .iter()
        .filter(|r| rule_covers_tool(r, short))
        .map(envelope_ref)
        .collect()
}

/// The rule that auto-approves this governed call, if any. Scope facts: the
/// call's resolved `workspace_id` argument (else the calling session's
/// workspace) and the credential's bound agent session. A DB failure resolves
/// to `None` — the call stays approval-gated (fail closed).
pub(crate) async fn resolve_for_call(
    ctx: &ServerCtx,
    auth: &AuthContext,
    short: &str,
    call_ws: Option<&str>,
) -> Option<McpAutoApproveRule> {
    if !tool_is_dangerous(short) {
        return None;
    }
    let session = auth
        .managed_session_id
        .as_deref()
        .or(auth.mcp_session_id.as_deref());
    let ws = match call_ws {
        Some(w) => Some(w.to_string()),
        None => crate::agent_refs::caller_session_ws(ctx, auth).await,
    };
    let rules = match repo(ctx).list_applicable(ws.as_deref(), session).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                tool = short,
                "auto-approve rules unavailable, staying gated: {e}"
            );
            return None;
        }
    };
    let mut call = call_for(short);
    call.workspace_id = ws.as_deref();
    call.session_id = session;
    aa::resolve(&rules, &call).cloned()
}

/// Make the GLOBAL per-tool rules match `next` (the catalog switch / the
/// `approval_exempt_tools` compatibility shim), and drop every per-tool rule —
/// any scope — whose tool is no longer `enabled` (re-enabling starts gated).
/// Disabled global rules for a listed tool are re-enabled rather than
/// duplicated. Irreversible tools have been validated by the caller.
pub(crate) async fn sync_catalog_toggles(
    ctx: &ServerCtx,
    user: &User,
    next: &[String],
    enabled: &[String],
) -> otto_core::Result<()> {
    let r = repo(ctx);
    let rules = r.list().await?;
    for rule in &rules {
        if rule.target_kind != TARGET_TOOL {
            continue;
        }
        let stale = !enabled.contains(&rule.target);
        let unlisted = is_active_catalog_toggle(rule) && !next.contains(&rule.target);
        if stale || unlisted {
            r.delete(&rule.id).await?;
            audit_change(ctx, user, "delete", rule).await;
        }
    }
    let rules = r.list().await?;
    for t in next {
        let existing = rules
            .iter()
            .find(|x| x.scope == SCOPE_GLOBAL && x.target_kind == TARGET_TOOL && &x.target == t);
        match existing {
            Some(x) if x.enabled => {}
            Some(x) => {
                let x = r
                    .update(
                        &x.id,
                        &AutoApproveRulePatch {
                            enabled: Some(true),
                            ..Default::default()
                        },
                    )
                    .await?;
                audit_change(ctx, user, "update", &x).await;
            }
            None => {
                let created = r
                    .create(NewAutoApproveRule {
                        name: format!("otto.{t}"),
                        enabled: true,
                        scope: SCOPE_GLOBAL.into(),
                        workspace_id: None,
                        session_id: None,
                        target_kind: TARGET_TOOL.into(),
                        target: t.clone(),
                        allow_irreversible: false,
                        note: None,
                        created_by: user.id.clone(),
                    })
                    .await?;
                audit_change(ctx, user, "create", &created).await;
            }
        }
    }
    Ok(())
}

async fn audit_change(ctx: &ServerCtx, user: &User, verb: &str, rule: &McpAutoApproveRule) {
    ctx.audit(otto_state::NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: format!("mcp.auto_approve.{verb}"),
        target: Some(rule.id.clone()),
        detail: serde_json::to_value(rule).ok(),
        ip: None,
    })
    .await;
}

// ===========================================================================
// HTTP: GET/POST /mcp/auto-approve, PATCH/DELETE /mcp/auto-approve/{id}
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct CreateAutoApproveReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub scope: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    pub target_kind: String,
    pub target: String,
    #[serde(default)]
    pub allow_irreversible: bool,
    #[serde(default)]
    pub note: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateAutoApproveReq {
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub allow_irreversible: Option<bool>,
    pub note: Option<String>,
}

/// Validate a rule's target + guardrail. Returns the normalized
/// `(target, allow_irreversible)`. Pure (catalog lookups only).
fn validate_target(
    kind: &str,
    target: &str,
    allow_irreversible: bool,
) -> Result<(String, bool), Error> {
    let target = target.trim();
    match kind {
        TARGET_TOOL => {
            let bare = target.strip_prefix("otto.").unwrap_or(target).to_string();
            if !tool_is_dangerous(&bare) {
                return Err(Error::Invalid(if tool_category(&bare).is_some() {
                    format!(
                        "'{target}' is not a mutating tool — only mutating tools ask for approval"
                    )
                } else {
                    format!("unknown otto tool '{target}'")
                }));
            }
            if tool_is_irreversible(&bare) {
                if !allow_irreversible {
                    return Err(Error::Invalid(format!(
                        "otto.{bare} is irreversible — auto-approving it needs the second explicit \
                         toggle (allow_irreversible: true)"
                    )));
                }
                Ok((bare, true))
            } else {
                // The flag means nothing for a reversible tool: store it off.
                Ok((bare, false))
            }
        }
        TARGET_CATEGORY => {
            if allow_irreversible {
                return Err(Error::Invalid(
                    "a category rule never covers irreversible tools — add a per-tool rule for one \
                     (allow_irreversible is only valid with target_kind 'tool')"
                        .into(),
                ));
            }
            if !mutating_categories().iter().any(|(c, _)| c == target) {
                return Err(Error::Invalid(format!(
                    "unknown category '{target}' (or it has no mutating tools)"
                )));
            }
            Ok((target.to_string(), false))
        }
        other => Err(Error::Invalid(format!(
            "target_kind must be 'tool' or 'category', not '{other}'"
        ))),
    }
}

fn default_name(kind: &str, target: &str, scope: &str) -> String {
    let what = if kind == TARGET_CATEGORY {
        format!("{target} writes")
    } else {
        format!("otto.{target}")
    };
    let wher = match scope {
        SCOPE_WORKSPACE => "this workspace",
        SCOPE_SESSION => "one session",
        _ => "everywhere",
    };
    format!("{what} — {wher}")
}

/// A rule plus display labels (workspace name / session title) for the UI.
async fn rule_view(ctx: &ServerCtx, rule: &McpAutoApproveRule) -> Value {
    let mut v = serde_json::to_value(rule).unwrap_or_else(|_| json!({}));
    if let Some(ws) = &rule.workspace_id {
        if let Ok(w) = ctx.workspaces.get(ws).await {
            v["workspace_name"] = json!(w.name);
        }
    }
    if let Some(sid) = &rule.session_id {
        if let Ok(s) = ctx.manager.get(sid).await {
            v["session_title"] = json!(s.title);
            v["workspace_id_of_session"] = json!(s.workspace_id);
        }
    }
    v["irreversible"] =
        json!(rule.target_kind == TARGET_TOOL && tool_is_irreversible(&rule.target));
    v
}

/// `GET /mcp/auto-approve` — every rule (with display labels) + the catalog a
/// rule can name: the categories with mutating tools and which tools are in
/// the irreversible guardrail tier.
pub async fn list_rules(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let rules = repo(&ctx).list().await.map_err(ApiError)?;
    let mut views = Vec::with_capacity(rules.len());
    for r in &rules {
        views.push(rule_view(&ctx, r).await);
    }
    let categories: Vec<Value> = mutating_categories()
        .into_iter()
        .map(|(category, tools)| {
            let tools: Vec<Value> = tools
                .iter()
                .map(|t| json!({"name": format!("otto.{t}"), "irreversible": tool_is_irreversible(t)}))
                .collect();
            json!({"category": category, "tools": tools})
        })
        .collect();
    Ok(Json(json!({ "rules": views, "categories": categories })))
}

/// `POST /mcp/auto-approve` (MCP Admin) — create one rule.
pub async fn create_rule(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<CreateAutoApproveReq>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    require_human(&auth).map_err(ApiError)?;
    let (target, allow_irreversible) = validate_target(
        req.target_kind.as_str(),
        &req.target,
        req.allow_irreversible,
    )
    .map_err(ApiError)?;
    let nonempty = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let workspace_id = nonempty(&req.workspace_id);
    let session_id = nonempty(&req.session_id);
    match req.scope.as_str() {
        SCOPE_GLOBAL => {
            if workspace_id.is_some() || session_id.is_some() {
                return Err(ApiError(Error::Invalid(
                    "a global rule takes neither workspace_id nor session_id".into(),
                )));
            }
        }
        SCOPE_WORKSPACE => {
            let Some(ws) = &workspace_id else {
                return Err(ApiError(Error::Invalid(
                    "scope 'workspace' needs workspace_id".into(),
                )));
            };
            if session_id.is_some() {
                return Err(ApiError(Error::Invalid(
                    "a workspace rule takes no session_id".into(),
                )));
            }
            ctx.workspaces.get(ws).await.map_err(ApiError)?;
        }
        SCOPE_SESSION => {
            let Some(sid) = &session_id else {
                return Err(ApiError(Error::Invalid(
                    "scope 'session' needs session_id".into(),
                )));
            };
            if workspace_id.is_some() {
                return Err(ApiError(Error::Invalid(
                    "a session rule takes no workspace_id".into(),
                )));
            }
            ctx.manager.get(sid).await.map_err(ApiError)?;
        }
        other => {
            return Err(ApiError(Error::Invalid(format!(
                "scope must be 'global', 'workspace' or 'session', not '{other}'"
            ))))
        }
    }
    let name =
        nonempty(&req.name).unwrap_or_else(|| default_name(&req.target_kind, &target, &req.scope));
    let rule = repo(&ctx)
        .create(NewAutoApproveRule {
            name,
            enabled: req.enabled,
            scope: req.scope.clone(),
            workspace_id,
            session_id,
            target_kind: req.target_kind.clone(),
            target,
            allow_irreversible,
            note: nonempty(&req.note),
            created_by: user.id.clone(),
        })
        .await
        .map_err(ApiError)?;
    audit_change(&ctx, &user, "create", &rule).await;
    Ok((StatusCode::CREATED, Json(rule_view(&ctx, &rule).await)))
}

/// `PATCH /mcp/auto-approve/{id}` (MCP Admin) — rename, enable/disable, or flip
/// the irreversible acknowledgement (only meaningful on a per-tool rule of an
/// irreversible tool; a category rule can never carry it).
pub async fn update_rule(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
    Json(req): Json<UpdateAutoApproveReq>,
) -> ApiResult<Json<Value>> {
    require_human(&auth).map_err(ApiError)?;
    let cur = repo(&ctx).get(&id).await.map_err(ApiError)?;
    let allow_irreversible = match req.allow_irreversible {
        Some(true) if cur.target_kind == TARGET_CATEGORY => {
            return Err(ApiError(Error::Invalid(
                "a category rule never covers irreversible tools".into(),
            )))
        }
        // Meaningless for a reversible tool: keep it off.
        Some(true) if !tool_is_irreversible(&cur.target) => Some(false),
        other => other,
    };
    if let Some(n) = &req.name {
        if n.trim().is_empty() {
            return Err(ApiError(Error::Invalid("name must not be empty".into())));
        }
    }
    let rule = repo(&ctx)
        .update(
            &id,
            &AutoApproveRulePatch {
                name: req.name.as_deref().map(|n| n.trim().to_string()),
                enabled: req.enabled,
                allow_irreversible,
                note: req.note.clone(),
            },
        )
        .await
        .map_err(ApiError)?;
    if rule != cur {
        audit_change(&ctx, &user, "update", &rule).await;
    }
    Ok(Json(rule_view(&ctx, &rule).await))
}

/// `DELETE /mcp/auto-approve/{id}` (MCP Admin) — the tool(s) ask again.
pub async fn delete_rule(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_human(&auth).map_err(ApiError)?;
    let cur = repo(&ctx).get(&id).await.map_err(ApiError)?;
    repo(&ctx).delete(&id).await.map_err(ApiError)?;
    audit_change(&ctx, &user, "delete", &cur).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_pr_is_auto_approvable_merge_needs_the_second_toggle() {
        assert_eq!(
            validate_target("tool", "otto.create_pr", false).unwrap(),
            ("create_pr".into(), false)
        );
        // Reversible tool: the flag is normalized off.
        assert_eq!(
            validate_target("tool", "create_pr", true).unwrap(),
            ("create_pr".into(), false)
        );
        let e = validate_target("tool", "merge_pr", false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("irreversible"), "{e}");
        assert_eq!(
            validate_target("tool", "merge_pr", true).unwrap(),
            ("merge_pr".into(), true)
        );
    }

    #[test]
    fn reads_and_unknown_tools_are_rejected() {
        let e = validate_target("tool", "list_repos", false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("not a mutating tool"), "{e}");
        let e = validate_target("tool", "nope", false)
            .unwrap_err()
            .to_string();
        assert!(e.contains("unknown otto tool"), "{e}");
        let e = validate_target("glob", "x", false).unwrap_err().to_string();
        assert!(e.contains("target_kind"), "{e}");
    }

    #[test]
    fn categories_are_validated_and_never_carry_the_irreversible_flag() {
        assert_eq!(
            validate_target("category", "Git", false).unwrap(),
            ("Git".into(), false)
        );
        assert!(validate_target("category", "Git", true).is_err());
        // A category with only reads (Approvals) or none at all is refused.
        assert!(validate_target("category", "Approvals", false).is_err());
        assert!(validate_target("category", "Nope", false).is_err());
    }

    #[test]
    fn the_guardrail_tier_is_a_subset_of_the_mutating_set() {
        // Every irreversible tool is a real, approval-gated catalog tool, and
        // the PR-opening path the user asked for is NOT in the tier.
        for t in [
            "merge_pr",
            "k8s_action",
            "produce_broker_message",
            "aws_sqs_send",
            "api_execute",
            "api_run_automation",
            "delete_scheduled_task",
            "assistant_forget",
        ] {
            assert!(tool_is_irreversible(t), "{t}");
            assert!(tool_is_dangerous(t), "{t} must be approval-gated");
            assert!(tool_category(t).is_some(), "{t} must be in the catalog");
        }
        for t in [
            "create_pr",
            "comment_pr",
            "comment_issue",
            "transition_issue",
            "send_message",
            "broadcast_message",
            "vault_delete",
        ] {
            assert!(!tool_is_irreversible(t), "{t}");
        }
        assert_eq!(tool_category("create_pr"), Some("Git"));
    }

    fn rule(scope: &str, kind: &str, target: &str, allow_irreversible: bool) -> McpAutoApproveRule {
        McpAutoApproveRule {
            id: format!("{scope}-{target}"),
            name: target.into(),
            enabled: true,
            scope: scope.into(),
            workspace_id: (scope == SCOPE_WORKSPACE).then(|| "ws".into()),
            session_id: (scope == SCOPE_SESSION).then(|| "s".into()),
            target_kind: kind.into(),
            target: target.into(),
            allow_irreversible,
            note: None,
            created_by: "u".into(),
            created_at: "now".into(),
            updated_at: "now".into(),
        }
    }

    #[test]
    fn catalog_badges_and_toggle_follow_the_guardrail() {
        let rules = vec![
            rule(SCOPE_GLOBAL, TARGET_CATEGORY, "Git", false),
            rule(SCOPE_WORKSPACE, TARGET_TOOL, "create_pr", false),
            rule(SCOPE_GLOBAL, TARGET_TOOL, "merge_pr", false), // inert without the ack
        ];
        assert_eq!(rules_covering_tool(&rules, "create_pr").len(), 2);
        // The Git category does not reach merge_pr; the un-acked per-tool rule neither.
        assert!(rules_covering_tool(&rules, "merge_pr").is_empty());
        assert!(!is_active_catalog_toggle(&rules[2]));
        assert!(is_active_catalog_toggle(&rule(
            SCOPE_GLOBAL,
            TARGET_TOOL,
            "merge_pr",
            true
        )));
        assert!(
            !is_active_catalog_toggle(&rules[1]),
            "a workspace rule is not the global switch"
        );
    }
}
