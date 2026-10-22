//! MCP profiles on the session stdio surface — the thin hook that narrows
//! `tools/list` and refuses `tools/call` outside the session's profile
//! (`otto_mcp::profiles`). The session's profile ids arrive with its
//! credentials (`OTTO_MCP_PROFILES`, captured at spawn — a change applies on
//! the next agent restart); the daemon resolves them into a [`ToolFilter`]
//! on the enable-list read the bridge already makes (`GET
//! /mcp/otto-server/enabled?profiles=`, cached with it).
//!
//! The filter only narrows: every call it lets through runs exactly the
//! path it ran before (native RBAC, the governed enable list / approvals).
//! An unresolvable profile (daemon unreachable) hides nothing — the same
//! fail-open the enable list itself has.

use otto_server::mcp_profiles::ToolFilter;
use serde_json::{json, Value};

use super::{is_vault_docs_reviewer, tool_result, Ctx};

/// The enable-list path, asking the daemon to resolve the session's
/// profiles when it runs under any.
pub(super) fn enabled_path(profiles: Option<&str>) -> String {
    match profiles {
        Some(p) => format!(
            "/mcp/otto-server/enabled?profiles={}",
            p.split(',')
                .map(|id| id.trim().replace(
                    |c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_',
                    ""
                ))
                .filter(|id| !id.is_empty())
                .collect::<Vec<_>>()
                .join(",")
        ),
        None => "/mcp/otto-server/enabled".to_string(),
    }
}

/// The resolved filter in an enable-list answer (`profile`), if any.
pub(super) fn parse(v: &Value) -> Option<ToolFilter> {
    serde_json::from_value(v.get("profile")?.clone()).ok()
}

/// `tools/call` hook: `Some(result)` refuses a tool outside the session's
/// profile (audited as a failed call), naming the profile; `None` lets the
/// call run its normal path.
pub(super) async fn refuse(ctx: &Ctx, name: &str, args: &Value) -> Option<Value> {
    if ctx.profiles.is_none() || is_vault_docs_reviewer(ctx.source.as_deref()) {
        return None;
    }
    let filter = ctx.enabled_info(false).await?.filter?;
    if filter.allows(name) {
        return None;
    }
    ctx.audit(name, args, false, None).await;
    Some(tool_result(&json!({ "error": filter.denial(name) }), true))
}
