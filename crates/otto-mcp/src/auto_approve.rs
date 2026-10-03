//! Auto-approve resolution — the explicit, opt-in policy under which a
//! MUTATING governed call skips the per-call human approval.
//!
//! A rule (`mcp_auto_approve_rules`, see `otto_state::McpAutoApproveRule`)
//! covers ONE tool or every mutating tool of a catalog CATEGORY, in ONE scope
//! (global / a workspace / an agent session). This module is the pure decision:
//! given the applicable rules and the call, which rule (if any) auto-approves
//! it. The caller (the `otto.*` choke point) still runs every other gate first
//! (per-token scope, enable, RBAC) and audits the call with [`audit_reason`].
//!
//! Precedence is by specificity so the audit names the narrowest decision:
//! session > workspace > global, and within a scope a per-tool rule beats a
//! category rule. Any covering rule auto-approves; specificity only picks
//! which one is recorded.
//!
//! **Guardrail.** An *irreversible* tool (the daemon's classification — e.g.
//! merging a PR, kubectl ops, producing to a live topic) is never covered by a
//! category rule, and a per-tool rule covers it only when the rule carries the
//! second explicit toggle `allow_irreversible`.

use otto_state::McpAutoApproveRule;

pub const SCOPE_GLOBAL: &str = "global";
pub const SCOPE_WORKSPACE: &str = "workspace";
pub const SCOPE_SESSION: &str = "session";
pub const TARGET_TOOL: &str = "tool";
pub const TARGET_CATEGORY: &str = "category";

/// The facts about one call that a rule is matched against.
#[derive(Debug, Clone, Copy)]
pub struct AutoApproveCall<'a> {
    /// Bare tool name (`create_pr`, no `otto.` prefix).
    pub tool: &'a str,
    /// The tool's catalog category (`Git`, `Issues`, …), when it has one.
    pub category: Option<&'a str>,
    /// The daemon classifies this tool as irreversible (guardrail tier).
    pub irreversible: bool,
    /// The workspace the call lands in (resolved arguments, else the calling
    /// session's workspace).
    pub workspace_id: Option<&'a str>,
    /// The calling agent session, when the credential is bound to one.
    pub session_id: Option<&'a str>,
}

/// Does `rule` cover `call`? Pure; ignores nothing but ordering.
pub fn covers(rule: &McpAutoApproveRule, call: &AutoApproveCall) -> bool {
    if !rule.enabled {
        return false;
    }
    let in_scope = match rule.scope.as_str() {
        SCOPE_GLOBAL => true,
        SCOPE_WORKSPACE => {
            rule.workspace_id.is_some() && rule.workspace_id.as_deref() == call.workspace_id
        }
        SCOPE_SESSION => rule.session_id.is_some() && rule.session_id.as_deref() == call.session_id,
        _ => false, // unknown scope: fail closed
    };
    if !in_scope {
        return false;
    }
    match rule.target_kind.as_str() {
        TARGET_TOOL => rule.target == call.tool && (!call.irreversible || rule.allow_irreversible),
        // A category never reaches an irreversible tool, whatever the flag says.
        TARGET_CATEGORY => !call.irreversible && call.category == Some(rule.target.as_str()),
        _ => false,
    }
}

/// Specificity rank (higher wins): scope first, then tool over category.
fn rank(rule: &McpAutoApproveRule) -> u8 {
    let scope = match rule.scope.as_str() {
        SCOPE_SESSION => 3,
        SCOPE_WORKSPACE => 2,
        _ => 1,
    };
    scope * 2 + u8::from(rule.target_kind == TARGET_TOOL)
}

/// The rule that auto-approves `call`, if any — the most specific covering
/// one (ties: the earliest in `rules`, i.e. the oldest).
pub fn resolve<'a>(
    rules: &'a [McpAutoApproveRule],
    call: &AutoApproveCall,
) -> Option<&'a McpAutoApproveRule> {
    let mut best: Option<&McpAutoApproveRule> = None;
    for r in rules.iter().filter(|r| covers(r, call)) {
        if best.is_none_or(|b| rank(r) > rank(b)) {
            best = Some(r);
        }
    }
    best
}

/// Human-readable scope of a rule (`global`, `workspace <id>`, `session <id>`).
pub fn scope_label(rule: &McpAutoApproveRule) -> String {
    match rule.scope.as_str() {
        SCOPE_WORKSPACE => format!("workspace {}", rule.workspace_id.as_deref().unwrap_or("?")),
        SCOPE_SESSION => format!("session {}", rule.session_id.as_deref().unwrap_or("?")),
        other => other.to_string(),
    }
}

/// The `decision_reason` recorded on an auto-approved audit row — names the
/// rule, its scope and what it covers, so nothing about the skip is silent.
pub fn audit_reason(rule: &McpAutoApproveRule) -> String {
    let target = match rule.target_kind.as_str() {
        TARGET_CATEGORY => format!("category '{}'", rule.target),
        _ => format!("tool otto.{}", rule.target),
    };
    format!(
        "auto-approved by policy '{}' ({}, {}{}) [rule {}]",
        rule.name,
        scope_label(rule),
        target,
        if rule.allow_irreversible {
            ", irreversible allowed"
        } else {
            ""
        },
        rule.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, scope: &str, kind: &str, target: &str) -> McpAutoApproveRule {
        McpAutoApproveRule {
            id: id.into(),
            name: id.into(),
            enabled: true,
            scope: scope.into(),
            workspace_id: (scope == SCOPE_WORKSPACE).then(|| "ws1".to_string()),
            session_id: (scope == SCOPE_SESSION).then(|| "s1".to_string()),
            target_kind: kind.into(),
            target: target.into(),
            allow_irreversible: false,
            note: None,
            created_by: "u".into(),
            created_at: "now".into(),
            updated_at: "now".into(),
        }
    }

    fn call<'a>(tool: &'a str, category: &'a str) -> AutoApproveCall<'a> {
        AutoApproveCall {
            tool,
            category: Some(category),
            irreversible: false,
            workspace_id: Some("ws1"),
            session_id: Some("s1"),
        }
    }

    #[test]
    fn no_rules_no_auto_approval() {
        assert!(resolve(&[], &call("create_pr", "Git")).is_none());
    }

    #[test]
    fn tool_and_category_rules_cover() {
        let rules = vec![rule("t", SCOPE_GLOBAL, TARGET_TOOL, "create_pr")];
        assert_eq!(resolve(&rules, &call("create_pr", "Git")).unwrap().id, "t");
        assert!(resolve(&rules, &call("comment_pr", "Git")).is_none());
        let rules = vec![rule("c", SCOPE_GLOBAL, TARGET_CATEGORY, "Git")];
        assert_eq!(resolve(&rules, &call("comment_pr", "Git")).unwrap().id, "c");
        assert!(resolve(&rules, &call("comment_issue", "Issues")).is_none());
    }

    #[test]
    fn scope_must_match_the_call() {
        let ws = rule("w", SCOPE_WORKSPACE, TARGET_TOOL, "create_pr");
        let sess = rule("s", SCOPE_SESSION, TARGET_TOOL, "create_pr");
        let mut c = call("create_pr", "Git");
        c.workspace_id = Some("ws2");
        c.session_id = Some("s2");
        assert!(resolve(&[ws.clone(), sess.clone()], &c).is_none());
        c.workspace_id = None;
        c.session_id = None;
        assert!(
            resolve(&[ws, sess], &c).is_none(),
            "a scoped rule never matches an unscoped call"
        );
    }

    #[test]
    fn disabled_and_unknown_rules_never_cover() {
        let mut off = rule("off", SCOPE_GLOBAL, TARGET_TOOL, "create_pr");
        off.enabled = false;
        let odd = rule("odd", "planet", TARGET_TOOL, "create_pr");
        let odd_kind = rule("k", SCOPE_GLOBAL, "glob", "create_pr");
        assert!(resolve(&[off, odd, odd_kind], &call("create_pr", "Git")).is_none());
    }

    #[test]
    fn irreversible_needs_a_per_tool_rule_with_the_second_toggle() {
        let mut c = call("merge_pr", "Git");
        c.irreversible = true;
        let cat = rule("cat", SCOPE_GLOBAL, TARGET_CATEGORY, "Git");
        let mut cat_flagged = cat.clone();
        cat_flagged.id = "cat2".into();
        cat_flagged.allow_irreversible = true; // never honoured on a category
        let tool = rule("tool", SCOPE_GLOBAL, TARGET_TOOL, "merge_pr");
        assert!(resolve(&[cat.clone(), cat_flagged, tool.clone()], &c).is_none());
        let mut ack = tool;
        ack.allow_irreversible = true;
        assert_eq!(resolve(&[cat, ack], &c).unwrap().id, "tool");
    }

    #[test]
    fn most_specific_rule_is_the_one_recorded() {
        let rules = vec![
            rule("g-cat", SCOPE_GLOBAL, TARGET_CATEGORY, "Git"),
            rule("g-tool", SCOPE_GLOBAL, TARGET_TOOL, "create_pr"),
            rule("w-cat", SCOPE_WORKSPACE, TARGET_CATEGORY, "Git"),
            rule("s-cat", SCOPE_SESSION, TARGET_CATEGORY, "Git"),
        ];
        assert_eq!(
            resolve(&rules, &call("create_pr", "Git")).unwrap().id,
            "s-cat"
        );
        assert_eq!(
            resolve(&rules[..3], &call("create_pr", "Git")).unwrap().id,
            "w-cat"
        );
        assert_eq!(
            resolve(&rules[..2], &call("create_pr", "Git")).unwrap().id,
            "g-tool"
        );
    }

    #[test]
    fn audit_reason_names_the_policy() {
        let r = rule("01ABC", SCOPE_WORKSPACE, TARGET_CATEGORY, "Git");
        let reason = audit_reason(&r);
        assert!(reason.starts_with("auto-approved by policy '01ABC'"));
        assert!(reason.contains("workspace ws1") && reason.contains("category 'Git'"));
    }
}
