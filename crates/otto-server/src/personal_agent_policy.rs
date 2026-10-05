//! Personal-agent permission enforcement at the **tool layer** — not just the
//! prompt. Three gates, all evaluated by the daemon on every governed `otto.*`
//! call ([`crate::mcp_outward::governed_invoke`]) and, for the read-only axis,
//! on every non-GET request an agent session's own token makes
//! ([`crate::feature_guard`]):
//!
//! 1. **Read-only sessions** (`meta.read_only = true` — every *Proactive* run
//!    and every run of a schedule whose permission set is `read_only`): any
//!    mutating tool, any send (room posts), memory writes and approval requests
//!    are refused outright. Auto-approve rules and token write grants do not
//!    apply — a read-only session cannot act, it can only report.
//! 2. **Sensitive actions** — account / credential / sharing changes — always
//!    need a human approval, whatever auto-approve rule or token grant would
//!    otherwise let the call through. Applies to every caller, not only
//!    personal agents.
//! 3. **Custom rules** — an agent's plain-language rule that names a target
//!    ("ask before touching prod") is enforced as an approval (or deny) policy
//!    for the agent's mutating calls that mention the target.
//!
//! The decision itself ([`decide`]) is pure and unit-tested; [`evaluate`] only
//! resolves the calling session's meta and the agent's rules.

use otto_core::auth::AuthContext;
use otto_state::{AgentRule, RuleEnforcement};
use serde_json::Value;

use crate::state::ServerCtx;

/// What the tool-layer gate decided for one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentGate {
    /// No personal-agent policy applies — the normal governance runs.
    Pass,
    /// Refuse the call (audited `denied` with this reason).
    Deny(String),
    /// The call needs a human approval even if an auto-approve rule or a token
    /// write grant covers it. `risk` is the approval's `risk_label`
    /// (`sensitive` | `agent_rule`).
    ForceApproval { reason: String, risk: &'static str },
}

/// Non-mutating catalog tools a read-only session still may not call: a room
/// post is a SEND, memory writes are writes, and an approval request asks a
/// human to let the agent act — a read-only run reports instead.
const READ_ONLY_EXTRA_DENY: &[&str] = &[
    "room_post",
    "ask_human_approval",
    "assistant_remember",
    "assistant_forget",
];

/// True iff a read-only session may NOT call the bare tool.
pub fn read_only_denies(bare: &str) -> bool {
    crate::mcp_outward::tool_is_mutating(bare) || READ_ONLY_EXTRA_DENY.contains(&bare)
}

/// Mutating tools that act on accounts or credentials by their nature: testing
/// an integration exercises its stored credentials against the live service.
const SENSITIVE_TOOLS: &[&str] = &["test_integration"];

/// Tools whose `destination` argument SHARES output outward (who sees a
/// recurring job's reports) — sensitive whenever it names a real destination.
const SHARING_DESTINATION_TOOLS: &[&str] = &["create_scheduled_task", "update_scheduled_task"];

/// Argument KEYS that mark an account / credential / sharing change.
const SENSITIVE_KEYS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "client_secret",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "private_key",
    "credential",
    "credentials",
    "share",
    "sharing",
    "share_with",
    "visibility",
    "permissions",
    "grant",
    "grants",
    "invite",
    "invitees",
    "collaborators",
    "acl",
];

/// Substrings in argument VALUES (URLs, paths, payloads) that mark one —
/// e.g. an API-client call to `/users/42/password` or `/repos/x/collaborators`.
const SENSITIVE_VALUE_MARKERS: &[&str] = &[
    "password",
    "credential",
    "/share",
    "/sharing",
    "/permissions",
    "/collaborators",
    "/invitations",
    "/invite",
    "/api-keys",
    "/api_keys",
    "/apikeys",
    "/tokens",
    "/oauth",
    "/members",
    "/grants",
    "/acl",
    "keychain",
];

fn key_is_sensitive(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    // Pagination cursors are not credentials (`page_token`, `next_token`).
    if k.contains("page") || k.contains("next") || k.contains("cursor") {
        return false;
    }
    SENSITIVE_KEYS.contains(&k.as_str())
        || k.ends_with("_password")
        || k.ends_with("_secret")
        || k.ends_with("_token")
        || k.ends_with("_api_key")
}

fn find_sensitive(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                if key_is_sensitive(k) {
                    return Some(format!("argument `{k}`"));
                }
                if let Some(hit) = find_sensitive(val) {
                    return Some(hit);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(find_sensitive),
        // Only URL / path-shaped values: free text (a PR comment that says
        // "password reset bug") is not an account change.
        Value::String(s) if s.starts_with('/') || s.starts_with("http") => {
            let lower = s.to_ascii_lowercase();
            SENSITIVE_VALUE_MARKERS
                .iter()
                .find(|m| lower.contains(*m))
                .map(|m| format!("a value mentioning `{}`", m.trim_start_matches('/')))
        }
        _ => None,
    }
}

/// Why a MUTATING call is an account / credential / sharing action, if it is.
/// Read tools are never gated here (they change nothing).
pub fn sensitive_reason(bare: &str, args: &Value) -> Option<String> {
    if !crate::mcp_outward::tool_is_mutating(bare) {
        return None;
    }
    if SENSITIVE_TOOLS.contains(&bare) {
        return Some(format!("otto.{bare} uses stored account credentials"));
    }
    if SHARING_DESTINATION_TOOLS.contains(&bare) {
        let kind = args
            .get("destination")
            .and_then(|d| d.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("none");
        if kind != "none" {
            return Some(format!(
                "otto.{bare} shares its reports to a {kind} destination"
            ));
        }
    }
    find_sensitive(args).map(|hit| {
        format!("otto.{bare} looks like an account / credential / sharing change ({hit})")
    })
}

// ---------------------------------------------------------------------------
// Custom rules
// ---------------------------------------------------------------------------

/// Phrases that make a rule an approval policy.
const APPROVAL_PHRASES: &[&str] = &[
    "ask before",
    "ask me before",
    "ask first",
    "check with me",
    "confirm before",
    "confirm with me",
    "approval before",
    "approve before",
    "require approval",
    "requires approval",
    "needs approval",
    "need approval",
    "get approval",
    "get my approval",
    "get my ok",
];

/// Phrases that make a rule a deny policy.
const DENY_PHRASES: &[&str] = &[
    "never ",
    "don't ",
    "dont ",
    "do not ",
    "must not ",
    "mustn't ",
    "not allowed to ",
];

/// Target words a rule can name, with their canonical match term.
const TARGET_VOCAB: &[(&str, &str)] = &[
    ("production", "prod"),
    ("prod", "prod"),
    ("staging", "staging"),
    ("main branch", "main"),
    ("master", "master"),
    ("billing", "billing"),
    ("payments", "payment"),
    ("payment", "payment"),
    ("customers", "customer"),
    ("customer", "customer"),
    ("finance", "finance"),
    ("secrets", "secret"),
    ("slack", "slack"),
    ("telegram", "telegram"),
    ("email", "email"),
    ("jira", "jira"),
    ("confluence", "confluence"),
    ("kubernetes", "k8s"),
    ("k8s", "k8s"),
    ("database", "db"),
    ("merge", "merge"),
    ("deploy", "deploy"),
    ("delete", "delete"),
];

fn push_term(terms: &mut Vec<String>, t: &str) {
    let t = t.trim().to_ascii_lowercase();
    if t.len() >= 2 && !terms.contains(&t) {
        terms.push(t);
    }
}

/// Derive the enforceable part of a plain-language rule. `None` when the rule
/// is instructions-only (no approval/deny phrasing, or no target we can match
/// on). Server-side only — a client never supplies `enforce`.
pub fn derive_enforcement(text: &str) -> Option<RuleEnforcement> {
    let lower = format!(" {} ", text.to_ascii_lowercase());
    let kind = if APPROVAL_PHRASES.iter().any(|p| lower.contains(p)) {
        "approval"
    } else if DENY_PHRASES
        .iter()
        .any(|p| lower.contains(&format!(" {p}")))
    {
        "deny"
    } else {
        return None;
    };
    let mut terms: Vec<String> = Vec::new();
    // Quoted targets: "billing-api", 'eu-west'.
    for q in ['"', '\'', '`'] {
        let parts: Vec<&str> = text.split(q).collect();
        // Odd indexes are inside quotes (only when the quotes are balanced).
        if parts.len() >= 3 && parts.len() % 2 == 1 {
            for inner in parts.iter().skip(1).step_by(2) {
                if inner.len() <= 64 {
                    push_term(&mut terms, inner);
                }
            }
        }
    }
    // #channels.
    for word in text.split_whitespace() {
        if let Some(ch) = word.strip_prefix('#') {
            let ch: String = ch
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !ch.is_empty() {
                push_term(&mut terms, &format!("#{ch}"));
            }
        }
    }
    for (word, term) in TARGET_VOCAB {
        if contains_word(&lower, word) {
            push_term(&mut terms, term);
        }
    }
    (!terms.is_empty()).then(|| RuleEnforcement {
        kind: kind.into(),
        terms,
    })
}

/// `needle` occurs in `hay` starting at a word boundary (so `prod` matches
/// `prod`, `prod-eu`, `production`, but not `reproduce`).
fn contains_word(hay: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let at = from + pos;
        let before_ok = at == 0
            || !hay[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric());
        if before_ok {
            return true;
        }
        from = at + needle.len();
    }
    false
}

/// The first rule whose enforcement matches this MUTATING call: `(kind,
/// rule text, matched term)`.
pub fn rule_verdict<'a>(
    rules: &'a [AgentRule],
    bare: &str,
    args: &Value,
) -> Option<(&'a str, &'a str, &'a str)> {
    if !crate::mcp_outward::tool_is_mutating(bare) {
        return None;
    }
    let hay = format!("{bare} {}", args).to_ascii_lowercase();
    // Deny outranks approval when two rules match.
    let mut approval = None;
    for rule in rules {
        let Some(enf) = &rule.enforce else { continue };
        if let Some(term) = enf.terms.iter().find(|t| contains_word(&hay, t)) {
            if enf.kind == "deny" {
                return Some(("deny", rule.text.as_str(), term.as_str()));
            }
            if enf.kind == "approval" && approval.is_none() {
                approval = Some(("approval", rule.text.as_str(), term.as_str()));
            }
        }
    }
    approval
}

/// The pure decision. `read_only` is the calling session's confinement;
/// `rules` are the calling personal agent's rules (empty for a non-agent).
pub fn decide(read_only: bool, rules: &[AgentRule], bare: &str, args: &Value) -> AgentGate {
    if read_only && read_only_denies(bare) {
        return AgentGate::Deny(format!(
            "this agent session is read-only (proactive or a read-only schedule): otto.{bare} \
             is not allowed — record what you would do as a finding in your report instead"
        ));
    }
    let rule = rule_verdict(rules, bare, args);
    if let Some(("deny", text, term)) = rule {
        return AgentGate::Deny(format!(
            "blocked by the agent's rule \"{text}\" (the call mentions `{term}`)"
        ));
    }
    if let Some(reason) = sensitive_reason(bare, args) {
        return AgentGate::ForceApproval {
            reason: format!("sensitive action — always needs your approval: {reason}"),
            risk: "sensitive",
        };
    }
    if let Some((_, text, term)) = rule {
        return AgentGate::ForceApproval {
            reason: format!("the agent's rule \"{text}\" asks first (the call mentions `{term}`)"),
            risk: "agent_rule",
        };
    }
    AgentGate::Pass
}

/// Is a session confined read-only? (`meta.read_only = true`, set by the
/// personal-agents engine for proactive and read-only-schedule runs.)
pub fn meta_is_read_only(meta: &Value) -> bool {
    meta.get("read_only").and_then(Value::as_bool) == Some(true)
}

/// Is this a session the scheduled-task or workflow engine spawned (meta
/// `source`), i.e. automation running unattended on its own schedule?
pub fn unattended_automation(meta: &Value) -> bool {
    matches!(
        meta.get("source").and_then(Value::as_str),
        Some("scheduled_task" | "workflow")
    )
}

/// The session an agent-held credential is bound to.
fn bound_session(auth: &AuthContext) -> Option<String> {
    auth.mcp_session_id
        .clone()
        .or_else(|| auth.managed_session_id.clone())
        .map(|id| id.to_string())
}

/// The calling personal agent, when the credential is an agent session's.
#[derive(Debug, Clone, Default)]
pub struct CallingAgent {
    pub agent_id: String,
    pub workspace_id: String,
    pub session_id: String,
}

/// Resolve the calling session (if the credential is session-bound) and the
/// personal agent behind it, then [`decide`] and record the call on the
/// agent's activity feed. A session-bound credential whose session row cannot
/// be read fails CLOSED for mutating tools.
pub async fn evaluate(
    ctx: &ServerCtx,
    auth: &AuthContext,
    bare: &str,
    args: &Value,
) -> (AgentGate, Option<CallingAgent>) {
    evaluate_with(ctx, auth, bare, args, None).await
}

/// [`evaluate`] reusing a calling-session row the caller already loaded this
/// request (the governed pipeline reads it for the audit workspace). Used
/// only when it IS the bound session; otherwise the row is read here, so the
/// verdict is identical either way.
pub async fn evaluate_with(
    ctx: &ServerCtx,
    auth: &AuthContext,
    bare: &str,
    args: &Value,
    preloaded: Option<&otto_core::domain::Session>,
) -> (AgentGate, Option<CallingAgent>) {
    let Some(sid) = bound_session(auth) else {
        return (decide(false, &[], bare, args), None);
    };
    let loaded = match preloaded.filter(|s| s.id == sid) {
        Some(s) => Some(SessionBinding::of(s)),
        None => session_binding(&ctx.pool, &sid).await,
    };
    let Some(binding) = loaded else {
        if read_only_denies(bare) {
            return (
                AgentGate::Deny(
                    "the calling session could not be resolved — refusing a mutating call".into(),
                ),
                None,
            );
        }
        return (decide(false, &[], bare, args), None);
    };
    let read_only = binding.read_only;
    let agent = binding.agent_id.as_ref().map(|a| CallingAgent {
        agent_id: a.clone(),
        workspace_id: binding.workspace_id.clone(),
        session_id: sid.clone(),
    });
    // Cached in the repo, invalidated on save/delete (perf N4).
    let rules = match &agent {
        Some(a) => otto_state::PersonalAgentsRepo::new(ctx.pool.clone())
            .autonomy(&a.agent_id)
            .await
            .map(|cfg| cfg.rules)
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let mut gate = decide(read_only, &rules, bare, args);
    // A scheduled-task / workflow session runs unattended on a schedule the
    // operator already configured (its results are delivered by the engine,
    // never through this gate): the sensitive-action gate would park it on an
    // approval nobody is there to give. It keeps the ordinary dangerous gate
    // + auto-approve rules instead. Personal agents (and their rules) are
    // never exempt.
    if agent.is_none()
        && binding.unattended
        && matches!(
            gate,
            AgentGate::ForceApproval {
                risk: "sensitive",
                ..
            }
        )
    {
        gate = AgentGate::Pass;
    }
    if let Some(a) = &agent {
        crate::personal_agent_activity::record_tool_call(
            ctx,
            &a.workspace_id,
            &a.agent_id,
            &a.session_id,
            bare,
            &gate,
        );
    }
    (gate, agent)
}

// ---------------------------------------------------------------------------
// Read-only sessions vs. direct HTTP (the native stdio tools)
// ---------------------------------------------------------------------------

/// Non-GET routes a read-only session's own token may still call, because they
/// READ: searches, schema/object introspection, a read-only query, a browser
/// summary. (Not the SQS peek: a receive bumps the receive count and can
/// dead-letter a message, so it is an Edit-gated write.) Templates without the `/api/v1` prefix; `{}`
/// matches any one path segment. The governed tool routes are listed too —
/// they apply [`decide`] themselves.
const READ_ONLY_POST_ALLOW: &[&str] = &[
    "/mcp/otto-tools/invoke",
    // The session's own tool-call audit row (an append to its ledger, R7).
    "/mcp/tool-calls",
    "/mcp/http",
    "/workspaces/{}/memory/search",
    "/workspaces/{}/vault/vaults/{}/search",
    "/workspaces/{}/vault/vaults/{}/okf/validate",
    "/connections/{}/db/schema/children",
    "/connections/{}/db/object",
    "/connections/{}/db/mcp-query",
    "/workspaces/{}/browser/summarize",
];

fn template_matches(pattern: &str, template: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').collect();
    let t: Vec<&str> = template.split('/').collect();
    p.len() == t.len()
        && p.iter().zip(&t).all(|(ps, ts)| {
            if *ps == "{}" {
                ts.starts_with('{') && ts.ends_with('}')
            } else {
                ps == ts
            }
        })
}

/// May a read-only session's token make this request? GET/HEAD/OPTIONS always;
/// any other method only for an allow-listed read route.
pub fn read_only_route_allowed(method: &axum::http::Method, template: &str) -> bool {
    use axum::http::Method;
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    let t = template.strip_prefix("/api/v1").unwrap_or(template);
    READ_ONLY_POST_ALLOW.iter().any(|p| template_matches(p, t))
}

/// What the policy needs from a calling session's row — all of it fixed when
/// the session is created (`read_only`, the `personal_agent` binding and the
/// automation `source` are written into the creation meta and no route or
/// engine rewrites them), so it is cached per session (perf N4).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionBinding {
    pub read_only: bool,
    pub agent_id: Option<String>,
    pub workspace_id: String,
    pub unattended: bool,
}

impl SessionBinding {
    pub fn of(session: &otto_core::domain::Session) -> Self {
        Self {
            read_only: meta_is_read_only(&session.meta),
            agent_id: session
                .meta
                .get("personal_agent")
                .and_then(Value::as_str)
                .map(str::to_string),
            workspace_id: session.workspace_id.to_string(),
            unattended: unattended_automation(&session.meta),
        }
    }
}

/// Process cache of [`SessionBinding`]s keyed by (database handle, session
/// id). Capped (cleared when full — a hit-rate aid, never state). Only
/// successful reads are cached, so a DB error still fails closed.
mod binding_cache {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    use super::SessionBinding;

    const CAP: usize = 512;
    type Key = (u64, String);

    fn map() -> &'static Mutex<HashMap<Key, SessionBinding>> {
        static MAP: OnceLock<Mutex<HashMap<Key, SessionBinding>>> = OnceLock::new();
        MAP.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(super) fn get(key: &Key) -> Option<SessionBinding> {
        map().lock().ok()?.get(key).cloned()
    }

    pub(super) fn put(key: Key, b: SessionBinding) {
        let Ok(mut m) = map().lock() else { return };
        if m.len() >= CAP && !m.contains_key(&key) {
            m.clear();
        }
        m.insert(key, b);
    }
}

/// The calling session's [`SessionBinding`], from the cache or one row read
/// (`None` when the row cannot be read — the caller decides; not cached).
pub async fn session_binding(
    pool: &otto_state::DbPool,
    session_id: &str,
) -> Option<SessionBinding> {
    let key = (pool.id(), session_id.to_string());
    if let Some(hit) = binding_cache::get(&key) {
        return Some(hit);
    }
    let s = otto_state::SessionsRepo::new(pool.clone())
        .get(&session_id.to_string())
        .await
        .ok()?;
    let b = SessionBinding::of(&s);
    binding_cache::put(key, b.clone());
    Some(b)
}

/// Is the session confined read-only (`None` when its row cannot be read —
/// the caller decides). Cached ([`session_binding`]): the feature guard asks
/// on every non-GET request an agent session's token makes.
pub async fn session_read_only(pool: &otto_state::DbPool, session_id: &str) -> Option<bool> {
    session_binding(pool, session_id).await.map(|b| b.read_only)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;
    use serde_json::json;

    fn rule(text: &str) -> AgentRule {
        AgentRule {
            id: "r".into(),
            text: text.into(),
            enforce: derive_enforcement(text),
        }
    }

    #[test]
    fn read_only_refuses_every_mutating_tool_and_sends() {
        // Every DANGEROUS (mutating) catalog tool is refused in a read-only session.
        for spec in crate::mcp_outward::otto_tool_specs() {
            let name = spec["name"].as_str().unwrap();
            let bare = name.strip_prefix("otto.").unwrap_or(name);
            if crate::mcp_outward::tool_is_mutating(bare) {
                assert!(
                    matches!(decide(true, &[], bare, &json!({})), AgentGate::Deny(_)),
                    "{bare} must be denied in a read-only session"
                );
            }
        }
        // Sends / memory writes / approval requests, though not "mutating".
        for bare in ["room_post", "ask_human_approval", "assistant_remember"] {
            assert!(matches!(
                decide(true, &[], bare, &json!({})),
                AgentGate::Deny(_)
            ));
        }
        // Explicit spot checks of the outward-facing ones.
        for bare in [
            "create_pr",
            "merge_pr",
            "comment_issue",
            "send_message",
            "k8s_action",
        ] {
            assert!(matches!(
                decide(true, &[], bare, &json!({})),
                AgentGate::Deny(_)
            ));
        }
    }

    #[test]
    fn read_only_still_reads() {
        for bare in [
            "list_prs",
            "get_pr",
            "search_issues",
            "vault_read",
            "room_read",
            "k8s_logs",
        ] {
            assert_eq!(
                decide(true, &[], bare, &json!({})),
                AgentGate::Pass,
                "{bare}"
            );
        }
    }

    #[test]
    fn read_only_is_not_bypassed_by_rules_or_sensitivity() {
        // A permissive-looking rule cannot re-open a mutating tool.
        let rules = vec![rule("Ask before touching prod")];
        assert!(matches!(
            decide(true, &rules, "create_pr", &json!({"title":"prod fix"})),
            AgentGate::Deny(_)
        ));
    }

    #[test]
    fn directed_sessions_keep_normal_governance() {
        assert_eq!(
            decide(false, &[], "create_pr", &json!({"title":"x"})),
            AgentGate::Pass
        );
        assert_eq!(
            decide(false, &[], "room_post", &json!({"text":"hi"})),
            AgentGate::Pass
        );
    }

    #[test]
    fn sensitive_actions_always_force_approval() {
        // By tool (credentials).
        assert!(matches!(
            decide(false, &[], "test_integration", &json!({"channel":"slack"})),
            AgentGate::ForceApproval {
                risk: "sensitive",
                ..
            }
        ));
        // Sharing: a scheduled task delivering outward; none stays normal.
        assert!(matches!(
            decide(
                false,
                &[],
                "create_scheduled_task",
                &json!({"name":"x","destination":{"type":"slack","channel":"C1"}})
            ),
            AgentGate::ForceApproval {
                risk: "sensitive",
                ..
            }
        ));
        assert_eq!(
            decide(
                false,
                &[],
                "create_scheduled_task",
                &json!({"name":"x","destination":{"type":"none"}})
            ),
            AgentGate::Pass
        );
        // By argument key (credential / sharing).
        let g = decide(
            false,
            &[],
            "api_upsert_request",
            &json!({"name":"x","headers":{"api_key":"…"}}),
        );
        assert!(matches!(
            g,
            AgentGate::ForceApproval {
                risk: "sensitive",
                ..
            }
        ));
        let g = decide(
            false,
            &[],
            "vault_write",
            &json!({"path":"a.md","visibility":"public"}),
        );
        assert!(matches!(
            g,
            AgentGate::ForceApproval {
                risk: "sensitive",
                ..
            }
        ));
        // By value (an API call to a password / collaborators endpoint).
        let g = decide(
            false,
            &[],
            "api_execute",
            &json!({"url":"https://api.example.com/users/42/password"}),
        );
        assert!(matches!(
            g,
            AgentGate::ForceApproval {
                risk: "sensitive",
                ..
            }
        ));
        let g = decide(
            false,
            &[],
            "api_execute",
            &json!({"url":"/repos/o/r/collaborators/bob"}),
        );
        assert!(matches!(
            g,
            AgentGate::ForceApproval {
                risk: "sensitive",
                ..
            }
        ));
    }

    #[test]
    fn sensitive_gate_ignores_reads_and_pagination_tokens() {
        assert_eq!(
            decide(
                false,
                &[],
                "list_prs",
                &json!({"page_token":"abc","password":"x"})
            ),
            AgentGate::Pass
        );
        assert_eq!(
            decide(
                false,
                &[],
                "comment_pr",
                &json!({"body":"lgtm","next_token":"t"})
            ),
            AgentGate::Pass
        );
        // Free text mentioning a password is not an account change.
        assert_eq!(
            decide(
                false,
                &[],
                "comment_pr",
                &json!({"body":"fixes the password reset bug"})
            ),
            AgentGate::Pass
        );
    }

    #[test]
    fn rules_derive_enforcement_from_plain_language() {
        let e = derive_enforcement("Ask before touching prod").unwrap();
        assert_eq!(e.kind, "approval");
        assert_eq!(e.terms, vec!["prod".to_string()]);
        let e = derive_enforcement("Never post in #general or Slack").unwrap();
        assert_eq!(e.kind, "deny");
        assert!(e.terms.contains(&"#general".to_string()));
        assert!(e.terms.contains(&"slack".to_string()));
        let e = derive_enforcement("Get my approval before changing \"billing-api\"").unwrap();
        assert!(e.terms.contains(&"billing-api".to_string()));
        assert!(e.terms.contains(&"billing".to_string()));
        // Instructions-only rules stay prompt-only.
        assert!(derive_enforcement("Be concise and friendly").is_none());
        assert!(derive_enforcement("Ask before doing anything weird").is_none());
    }

    #[test]
    fn rules_force_approval_or_deny_for_matching_mutations() {
        let rules = vec![rule("Ask before touching production")];
        let g = decide(
            false,
            &rules,
            "k8s_action",
            &json!({"cluster":"prod-eu","verb":"scale"}),
        );
        assert!(matches!(
            g,
            AgentGate::ForceApproval {
                risk: "agent_rule",
                ..
            }
        ));
        // No mention → normal governance.
        assert_eq!(
            decide(false, &rules, "k8s_action", &json!({"cluster":"staging"})),
            AgentGate::Pass
        );
        // Word boundary: "reproduce" is not prod.
        assert_eq!(
            decide(
                false,
                &rules,
                "comment_issue",
                &json!({"body":"cannot reproduce"})
            ),
            AgentGate::Pass
        );
        // Reads are never gated by rules.
        assert_eq!(
            decide(false, &rules, "k8s_logs", &json!({"cluster":"prod"})),
            AgentGate::Pass
        );
        // Deny wins.
        let rules = vec![
            rule("Ask before touching prod"),
            rule("Never merge to prod"),
        ];
        assert!(matches!(
            decide(false, &rules, "merge_pr", &json!({"repo_id":"prod-api"})),
            AgentGate::Deny(_)
        ));
    }

    #[test]
    fn read_only_http_allows_reads_and_listed_read_posts_only() {
        assert!(read_only_route_allowed(
            &Method::GET,
            "/api/v1/workspaces/{wid}/sessions"
        ));
        assert!(read_only_route_allowed(
            &Method::POST,
            "/api/v1/mcp/otto-tools/invoke"
        ));
        assert!(read_only_route_allowed(
            &Method::POST,
            "/api/v1/workspaces/{wid}/vault/vaults/{vid}/search"
        ));
        assert!(read_only_route_allowed(
            &Method::POST,
            "/api/v1/connections/{id}/db/mcp-query"
        ));
        for (m, t) in [
            (Method::POST, "/api/v1/agent-rooms/{id}/messages"),
            (Method::POST, "/api/v1/repos/{id}/prs/{n}/comments"),
            (Method::POST, "/api/v1/workspaces/{wid}/canvas/scenes"),
            (
                Method::PUT,
                "/api/v1/workspaces/{wid}/vault/vaults/{vid}/notes",
            ),
            (Method::DELETE, "/api/v1/sessions/{id}"),
            (Method::POST, "/api/v1/workspaces/{wid}/browser/login"),
            (Method::POST, "/api/v1/k8s/clusters/{id}/actions"),
            (Method::PATCH, "/api/v1/personal-agents/{id}"),
        ] {
            assert!(!read_only_route_allowed(&m, t), "{m} {t} must be refused");
        }
    }

    #[test]
    fn meta_flag_parsing() {
        assert!(meta_is_read_only(&json!({"read_only": true})));
        assert!(!meta_is_read_only(&json!({"read_only": "true"})));
        assert!(!meta_is_read_only(&json!({})));
    }
}
