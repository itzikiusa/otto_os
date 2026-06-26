//! Policy-as-code evaluation (control-plane requirement 11).
//!
//! Rules (`mcp_policies`) are data: a `match` object + an `effect`. A call's
//! evaluation context is matched against every enabled, applicable rule (global +
//! the call's workspace). Evaluation is **most-restrictive-wins**, independent of
//! priority (design §14 F3): if any matching rule says `deny`, the call is denied;
//! else `require_approval`; else `require_dry_run`; else `allow`. Priority orders
//! display only. The full ordered ruleset is the policy-as-code artifact
//! (exportable/importable as one JSON document).

use otto_state::McpPolicy;
use serde_json::Value;

/// The context a call is evaluated against.
pub struct PolicyCtx<'a> {
    pub server_id: &'a str,
    pub server_name: &'a str,
    pub tool: &'a str,
    pub risk_label: &'a str,
    pub injection_risk: &'a str,
    pub mutating: bool,
    pub direction: &'a str,
    pub caller_kind: &'a str,
    pub workspace_id: Option<&'a str>,
}

/// The decided policy effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Allow,
    Deny(String),
    RequireApproval(String),
    RequireDryRun(String),
}

fn injection_rank(level: &str) -> u8 {
    match level {
        "low" => 0,
        "medium" => 1,
        "high" => 2,
        _ => 1,
    }
}

/// Simple `*`-glob match (only `*` wildcards, case-sensitive).
fn glob_match(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let mut idx = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !s[idx..].starts_with(part) {
                return false;
            }
            idx += part.len();
        } else if i == parts.len() - 1 {
            if !s[idx..].ends_with(part) {
                return false;
            }
        } else if let Some(found) = s[idx..].find(part) {
            idx += found + part.len();
        } else {
            return false;
        }
    }
    true
}

fn str_field<'a>(m: &'a Value, key: &str) -> Option<&'a str> {
    m.get(key).and_then(Value::as_str)
}

/// Does this rule's `match` object match the context? All present fields must
/// match (AND). An empty match object matches everything.
fn rule_matches(m: &Value, ctx: &PolicyCtx) -> bool {
    if let Some(v) = str_field(m, "server_id") {
        if v != ctx.server_id {
            return false;
        }
    }
    if let Some(v) = str_field(m, "server_name") {
        if v != ctx.server_name {
            return false;
        }
    }
    if let Some(v) = str_field(m, "tool") {
        if v != ctx.tool {
            return false;
        }
    }
    if let Some(v) = str_field(m, "tool_glob") {
        if !glob_match(v, ctx.tool) {
            return false;
        }
    }
    if let Some(v) = str_field(m, "risk_label") {
        if v != ctx.risk_label {
            return false;
        }
    }
    if let Some(v) = str_field(m, "min_injection_risk") {
        if injection_rank(ctx.injection_risk) < injection_rank(v) {
            return false;
        }
    }
    if let Some(v) = m.get("mutating").and_then(Value::as_bool) {
        if v != ctx.mutating {
            return false;
        }
    }
    if let Some(v) = str_field(m, "direction") {
        if v != ctx.direction {
            return false;
        }
    }
    if let Some(v) = str_field(m, "caller_kind") {
        if v != ctx.caller_kind {
            return false;
        }
    }
    if let Some(v) = str_field(m, "workspace_id") {
        if Some(v) != ctx.workspace_id {
            return false;
        }
    }
    true
}

/// Severity rank: higher = more restrictive. Used for most-restrictive-wins.
fn effect_rank(effect: &str) -> u8 {
    match effect {
        "deny" => 3,
        "require_approval" => 2,
        "require_dry_run" => 1,
        _ => 0, // allow / unknown
    }
}

/// Evaluate the applicable rules for a call. `rules` should already be filtered
/// to enabled + (global or this workspace).
pub fn evaluate(rules: &[McpPolicy], ctx: &PolicyCtx) -> Effect {
    let mut best: Option<(&McpPolicy, u8)> = None;
    for r in rules {
        if !r.enabled {
            continue;
        }
        if !rule_matches(&r.match_json, ctx) {
            continue;
        }
        let rank = effect_rank(&r.effect);
        match best {
            Some((_, br)) if br >= rank => {}
            _ => best = Some((r, rank)),
        }
    }
    match best {
        None => Effect::Allow,
        Some((r, _)) => {
            let reason = r
                .reason
                .clone()
                .unwrap_or_else(|| format!("policy '{}'", r.name));
            match r.effect.as_str() {
                "deny" => Effect::Deny(reason),
                "require_approval" => Effect::RequireApproval(reason),
                "require_dry_run" => Effect::RequireDryRun(reason),
                _ => Effect::Allow,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(name: &str, priority: i64, effect: &str, m: Value) -> McpPolicy {
        McpPolicy {
            id: name.into(),
            workspace_id: None,
            name: name.into(),
            enabled: true,
            priority,
            match_json: m,
            effect: effect.into(),
            reason: None,
            created_by: "u".into(),
            created_at: "now".into(),
            updated_at: "now".into(),
        }
    }

    fn ctx<'a>() -> PolicyCtx<'a> {
        PolicyCtx {
            server_id: "srv1",
            server_name: "github",
            tool: "delete_repo",
            risk_label: "dangerous",
            injection_risk: "high",
            mutating: true,
            direction: "outbound",
            caller_kind: "agent",
            workspace_id: Some("ws1"),
        }
    }

    #[test]
    fn no_rules_allows() {
        assert_eq!(evaluate(&[], &ctx()), Effect::Allow);
    }

    #[test]
    fn deny_wins_over_low_priority_allow() {
        // A low-priority (0) allow must NOT override a deny — most-restrictive-wins.
        let rules = vec![
            rule("allow_all", 0, "allow", json!({})),
            rule("deny_danger", 100, "deny", json!({"risk_label": "dangerous"})),
        ];
        assert!(matches!(evaluate(&rules, &ctx()), Effect::Deny(_)));
    }

    #[test]
    fn require_approval_for_glob() {
        let rules = vec![rule("approve_deletes", 50, "require_approval", json!({"tool_glob": "delete_*"}))];
        assert!(matches!(evaluate(&rules, &ctx()), Effect::RequireApproval(_)));
    }

    #[test]
    fn min_injection_matches_at_or_above() {
        let rules = vec![rule("dry_high_injection", 10, "require_dry_run", json!({"min_injection_risk": "high"}))];
        assert!(matches!(evaluate(&rules, &ctx()), Effect::RequireDryRun(_)));
        // A medium-injection context shouldn't match a high threshold.
        let mut c = ctx();
        c.injection_risk = "medium";
        c.risk_label = "read";
        assert_eq!(evaluate(&rules, &c), Effect::Allow);
    }

    #[test]
    fn most_restrictive_wins_among_matches() {
        let rules = vec![
            rule("a", 1, "require_dry_run", json!({})),
            rule("b", 2, "require_approval", json!({})),
            rule("c", 3, "allow", json!({})),
        ];
        assert!(matches!(evaluate(&rules, &ctx()), Effect::RequireApproval(_)));
    }

    #[test]
    fn glob_helper() {
        assert!(glob_match("delete_*", "delete_repo"));
        assert!(glob_match("*_url", "fetch_url"));
        assert!(glob_match("a*b*c", "axxbyyc"));
        assert!(!glob_match("delete_*", "create_repo"));
    }
}
