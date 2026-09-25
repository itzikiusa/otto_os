//! Agent UI control — the command catalog (`docs/contracts/ui-commands.json`).
//!
//! The JSON file is the ONE source of truth for the `otto.ui_*` tools: the
//! daemon embeds it here (`include_str!`), the UI imports the same file
//! (`ui/src/lib/uiCommands/catalog.ts`), and a test on each side pins them
//! together — the Rust tests below validate every entry, the UI's parity test
//! asserts every entry has a registered handler. Each entry becomes one governed
//! tool (`otto.ui_<name>`, stdio `otto_ui_<name>`) via [`specs`], appended to
//! `mcp_outward::otto_tool_specs()` under the "UI control" category, so both
//! MCP surfaces advertise exactly what the catalog declares.
//!
//! Also here: the dependency-free argument validator the bridge runs before a
//! command leaves the daemon ([`validate_args`]), and a port of the UI's
//! `paneKey` (`ui/src/lib/sidePane.ts`) so a catalog entry's `module` is
//! checked against its `route` exactly as the UI will route it.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The embedded catalog text (also served verbatim by `GET /ui/commands/catalog`).
pub const CATALOG_JSON: &str = include_str!("../../../docs/contracts/ui-commands.json");

/// Prefix every catalog command carries as a governed tool (`ui_<name>`).
pub const TOOL_PREFIX: &str = "ui_";

/// The governed-tool category the control plane groups these under.
pub const CATEGORY: &str = "UI control";

/// Upper bound on any command's deadline (and on the human-confirm extension).
pub const MAX_TIMEOUT_MS: u64 = 120_000;
/// Lower bound on a catalog `timeout_ms`.
pub const MIN_TIMEOUT_MS: u64 = 1_000;

/// How much a command may do. `read` / `navigate` run once the session is
/// granted; `local_write` / `outward` additionally confirm in the UI every
/// time (and count as mutating for a read-only MCP token scope).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Read,
    Navigate,
    LocalWrite,
    Outward,
}

impl Risk {
    /// Mutating tiers — denied to an MCP token whose scope is read-only.
    pub fn mutating(self) -> bool {
        matches!(self, Risk::LocalWrite | Risk::Outward)
    }
}

/// Daemon-side fallbacks a `read` command may run when no Otto window can.
pub const HEADLESS: &[&str] = &["presence", "db_list_connections", "db_mcp_query"];

/// The paneKeys a catalog entry may target: every sidebar module id
/// (`ui/src/lib/sidebar.ts` `SIDEBAR_MODULES`, which `paneKey` mirrors), plus
/// `shell` (any Otto document — navigation / state commands).
pub const KNOWN_MODULES: &[&str] = &[
    "shell",
    "home",
    "assistant",
    "agents",
    "history",
    "run-with-otto",
    "mission-control",
    "swarm",
    "loops",
    "workflows",
    "scheduled-tasks",
    "personal-agents",
    "git",
    "proof",
    "product",
    "vault",
    "design",
    "skills-eval",
    "connections",
    "aws",
    "kubernetes",
    "api",
    "browser",
    "mcp",
    "insights",
    "usage",
    "settings",
];

/// One catalog entry (see the file's `$comment` for field semantics).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiCommandSpec {
    pub name: String,
    pub module: String,
    pub route: String,
    pub risk: Risk,
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headless: Option<String>,
    pub description: String,
    pub input_schema: Value,
    /// Block marker / note — ignored.
    #[serde(default, rename = "$comment", skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

impl UiCommandSpec {
    /// The bare governed tool name (`ui_db_run_query`).
    pub fn tool(&self) -> String {
        format!("{TOOL_PREFIX}{}", self.name)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    #[serde(default, rename = "$comment")]
    _comment: Option<String>,
    version: u32,
    commands: Vec<UiCommandSpec>,
}

/// Parse catalog text. Pure — the tests run it on the embedded file.
fn parse(text: &str) -> Result<(u32, Vec<UiCommandSpec>), String> {
    let f: CatalogFile =
        serde_json::from_str(text).map_err(|e| format!("ui-commands.json: {e}"))?;
    Ok((f.version, f.commands))
}

/// The parsed catalog. A malformed embedded file is a build-time bug the
/// tests catch; at runtime it degrades to an EMPTY catalog (no `ui_*` tools)
/// rather than taking the daemon down.
pub fn catalog() -> &'static [UiCommandSpec] {
    static CAT: OnceLock<Vec<UiCommandSpec>> = OnceLock::new();
    CAT.get_or_init(|| match parse(CATALOG_JSON) {
        Ok((_, cmds)) => cmds,
        Err(e) => {
            tracing::error!("agent UI control disabled: {e}");
            Vec::new()
        }
    })
}

/// Look a command up by its catalog name (`db_run_query`).
pub fn get(name: &str) -> Option<&'static UiCommandSpec> {
    catalog().iter().find(|c| c.name == name)
}

/// Look a command up by its bare governed tool name (`ui_db_run_query`).
pub fn by_tool(tool: &str) -> Option<&'static UiCommandSpec> {
    tool.strip_prefix(TOOL_PREFIX).and_then(get)
}

/// True iff `tool` (bare) is a catalog UI-control tool.
pub fn is_ui_tool(tool: &str) -> bool {
    by_tool(tool).is_some()
}

/// Every catalog tool's bare name.
pub fn tool_names() -> Vec<String> {
    catalog().iter().map(UiCommandSpec::tool).collect()
}

/// The governed-tool specs (`otto_tool_specs` shape) for every catalog entry.
pub fn specs() -> Vec<Value> {
    catalog()
        .iter()
        .map(|c| {
            json!({
                "name": format!("otto.{}", c.tool()),
                "mutating": c.risk.mutating(),
                "category": CATEGORY,
                "description": format!(
                    "{} [Agent UI control — runs visibly in the user's Otto window; needs the \
                     user's per-session \"Allow UI control\" grant. Risk: {}.]",
                    c.description,
                    risk_label(c.risk)
                ),
                "inputSchema": c.input_schema,
            })
        })
        .collect()
}

fn risk_label(r: Risk) -> &'static str {
    match r {
        Risk::Read => "read",
        Risk::Navigate => "navigate",
        Risk::LocalWrite => "local write — the user confirms each one",
        Risk::Outward => "outward — the user confirms each one",
    }
}

/// Port of `paneKey` (`ui/src/lib/sidePane.ts`): the sidebar entry a hash
/// route belongs to. `''` → agents, `plugin/<slug>` per plugin, the Database /
/// Message Brokers views → connections, Canvas → design.
pub fn pane_key(route: &str) -> String {
    let r = route.trim_start_matches('#').trim_start_matches('/');
    let mut parts = r.split(['/', '?']);
    let m = parts.next().unwrap_or("");
    match m {
        "" => "agents".into(),
        "plugin" => format!("plugin/{}", parts.next().unwrap_or("")),
        "database" | "brokers" => "connections".into(),
        "canvas" => "design".into(),
        other => other.into(),
    }
}

// ===========================================================================
// Argument validation — the subset of JSON Schema the catalog uses.
// ===========================================================================

/// Validate `args` against a catalog `input_schema`. Supports what the catalog
/// uses: `type` (object/string/integer/number/boolean/array), `required`,
/// `properties`, `additionalProperties: false`, `enum`, `minimum`/`maximum`,
/// `minLength`/`maxLength`, `minItems`/`maxItems`, `items`. Returns the first
/// problem as a message the agent can act on. Pure.
pub fn validate_args(schema: &Value, args: &Value) -> Result<(), String> {
    check(schema, args, "arguments")
}

fn type_ok(ty: &str, v: &Value) -> bool {
    match ty {
        "object" => v.is_object(),
        "string" => v.is_string(),
        "integer" => v.as_i64().is_some() || v.as_u64().is_some(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "null" => v.is_null(),
        _ => true,
    }
}

fn check(schema: &Value, v: &Value, at: &str) -> Result<(), String> {
    if let Some(ty) = schema.get("type").and_then(Value::as_str) {
        if !type_ok(ty, v) {
            return Err(format!("{at} must be of type {ty}"));
        }
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.iter().any(|a| a == v) {
            let list = allowed
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!("{at} must be one of: {list}"));
        }
    }
    if let Some(n) = v.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_f64) {
            if n < min {
                return Err(format!("{at} must be ≥ {min}"));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_f64) {
            if n > max {
                return Err(format!("{at} must be ≤ {max}"));
            }
        }
    }
    if let Some(s) = v.as_str() {
        let len = s.chars().count() as u64;
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
            if len < min {
                return Err(format!("{at} must be at least {min} characters"));
            }
        }
        if let Some(max) = schema.get("maxLength").and_then(Value::as_u64) {
            if len > max {
                return Err(format!("{at} must be at most {max} characters"));
            }
        }
    }
    if let Some(items) = v.as_array() {
        let n = items.len() as u64;
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64) {
            if n < min {
                return Err(format!("{at} needs at least {min} item(s)"));
            }
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64) {
            if n > max {
                return Err(format!("{at} takes at most {max} item(s)"));
            }
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                check(item_schema, item, &format!("{at}[{i}]"))?;
            }
        }
    }
    if let Some(obj) = v.as_object() {
        let props = schema.get("properties").and_then(Value::as_object);
        if let Some(req) = schema.get("required").and_then(Value::as_array) {
            for k in req.iter().filter_map(Value::as_str) {
                if !obj.contains_key(k) {
                    return Err(format!("missing required argument '{k}'"));
                }
            }
        }
        let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
        for (k, val) in obj {
            match props.and_then(|p| p.get(k)) {
                Some(sub) => check(sub, val, k)?,
                None if closed => {
                    let known = props
                        .map(|p| p.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_default();
                    return Err(format!(
                        "unknown argument '{k}' (accepted: {})",
                        if known.is_empty() { "none" } else { &known }
                    ));
                }
                None => {}
            }
        }
    }
    Ok(())
}

/// Every structural rule a catalog entry must satisfy. Returns all problems.
/// Pure; run by the tests on the embedded file (and usable by tooling).
pub fn catalog_problems(version: u32, cmds: &[UiCommandSpec]) -> Vec<String> {
    let mut out = Vec::new();
    if version != 1 {
        out.push(format!("unsupported catalog version {version}"));
    }
    let mut seen = std::collections::HashSet::new();
    for c in cmds {
        let n = &c.name;
        if !seen.insert(n.clone()) {
            out.push(format!("{n}: duplicate name"));
        }
        if n.is_empty()
            || !n
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || n.starts_with('_')
            || n.ends_with('_')
        {
            out.push(format!("{n}: name must be snake_case [a-z0-9_]"));
        }
        if !KNOWN_MODULES.contains(&c.module.as_str()) {
            out.push(format!("{n}: unknown module '{}'", c.module));
        }
        if c.module == "shell" {
            if !c.route.is_empty() {
                out.push(format!("{n}: a shell command has no route"));
            }
        } else if pane_key(&c.route) != c.module {
            out.push(format!(
                "{n}: route '{}' belongs to '{}', not module '{}'",
                c.route,
                pane_key(&c.route),
                c.module
            ));
        }
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&c.timeout_ms) {
            out.push(format!(
                "{n}: timeout_ms {} outside {MIN_TIMEOUT_MS}..={MAX_TIMEOUT_MS}",
                c.timeout_ms
            ));
        }
        if let Some(h) = &c.headless {
            if c.risk != Risk::Read {
                out.push(format!("{n}: headless is only allowed on a read command"));
            }
            if !HEADLESS.contains(&h.as_str()) {
                out.push(format!("{n}: unknown headless fallback '{h}'"));
            }
        }
        if c.description.trim().len() < 20 {
            out.push(format!("{n}: description too short"));
        }
        let s = &c.input_schema;
        if s.get("type") != Some(&json!("object")) {
            out.push(format!("{n}: input_schema.type must be \"object\""));
        }
        if s.get("additionalProperties") != Some(&Value::Bool(false)) {
            out.push(format!(
                "{n}: input_schema.additionalProperties must be false"
            ));
        }
        let props = s.get("properties").and_then(Value::as_object);
        if props.is_none() {
            out.push(format!("{n}: input_schema.properties must be an object"));
        }
        if let Some(req) = s.get("required") {
            match req.as_array() {
                Some(list) => {
                    for r in list {
                        match r.as_str() {
                            Some(k) if props.is_some_and(|p| p.contains_key(k)) => {}
                            _ => out.push(format!(
                                "{n}: required '{}' is not a property",
                                r.as_str().map_or_else(|| r.to_string(), str::to_string)
                            )),
                        }
                    }
                }
                None => out.push(format!("{n}: input_schema.required must be an array")),
            }
        }
        if let Some(p) = props {
            for (k, sub) in p {
                if sub.get("type").and_then(Value::as_str).is_none() {
                    out.push(format!("{n}: property '{k}' has no type"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_parses_and_is_valid() {
        let (version, cmds) = parse(CATALOG_JSON).expect("ui-commands.json parses");
        assert!(!cmds.is_empty(), "catalog is empty");
        let problems = catalog_problems(version, &cmds);
        assert!(problems.is_empty(), "catalog problems:\n{}", problems.join("\n"));
        assert_eq!(catalog().len(), cmds.len());
    }

    #[test]
    fn phase_one_commands_exist() {
        for n in [
            "state",
            "open",
            "focus",
            "db_list_connections",
            "db_open_connection",
            "db_new_tab",
            "db_set_statement",
            "db_run_query",
            "db_get_result",
            "db_page",
            "db_set_view",
            "db_open_object",
            "db_explain",
            "db_stop",
            "db_export",
        ] {
            assert!(get(n).is_some(), "missing catalog command {n}");
        }
        assert_eq!(get("db_run_query").unwrap().headless.as_deref(), Some("db_mcp_query"));
        assert_eq!(
            get("db_list_connections").unwrap().headless.as_deref(),
            Some("db_list_connections")
        );
        assert_eq!(get("state").unwrap().headless.as_deref(), Some("presence"));
        assert_eq!(by_tool("ui_db_run_query").unwrap().name, "db_run_query");
        assert!(by_tool("db_run_query").is_none());
        assert!(!is_ui_tool("ui_nope"));
    }

    #[test]
    fn specs_mirror_the_catalog() {
        let specs = specs();
        assert_eq!(specs.len(), catalog().len());
        for (s, c) in specs.iter().zip(catalog()) {
            assert_eq!(s["name"], json!(format!("otto.ui_{}", c.name)));
            assert_eq!(s["category"], json!(CATEGORY));
            assert_eq!(s["mutating"], json!(c.risk.mutating()));
            assert_eq!(s["inputSchema"], c.input_schema);
        }
    }

    #[test]
    fn pane_key_matches_the_ui() {
        assert_eq!(pane_key(""), "agents");
        assert_eq!(pane_key("#/"), "agents");
        assert_eq!(pane_key("database"), "connections");
        assert_eq!(pane_key("/database/x"), "connections");
        assert_eq!(pane_key("brokers"), "connections");
        assert_eq!(pane_key("canvas/abc"), "design");
        assert_eq!(pane_key("plugin/foo/bar"), "plugin/foo");
        assert_eq!(pane_key("git/repo?x=1"), "git");
        assert_eq!(pane_key("kubernetes"), "kubernetes");
        assert_eq!(pane_key("scheduled-tasks"), "scheduled-tasks");
    }

    fn spec(name: &str, module: &str, route: &str, risk: Risk, headless: Option<&str>) -> UiCommandSpec {
        UiCommandSpec {
            name: name.into(),
            module: module.into(),
            route: route.into(),
            risk,
            timeout_ms: 10_000,
            headless: headless.map(str::to_string),
            description: "A long enough description for the check.".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            comment: None,
        }
    }

    #[test]
    fn catalog_problems_catch_each_rule() {
        let ok = spec("x", "connections", "database", Risk::Read, Some("db_mcp_query"));
        assert!(catalog_problems(1, std::slice::from_ref(&ok)).is_empty());
        let has = |cmds: &[UiCommandSpec], needle: &str| {
            let p = catalog_problems(1, cmds);
            assert!(p.iter().any(|m| m.contains(needle)), "{needle} not in {p:?}");
        };
        has(&[ok.clone(), ok.clone()], "duplicate name");
        has(&[spec("Bad-Name", "connections", "database", Risk::Read, None)], "snake_case");
        has(&[spec("x", "nowhere", "nowhere", Risk::Read, None)], "unknown module");
        has(&[spec("x", "git", "database", Risk::Read, None)], "belongs to");
        has(&[spec("x", "shell", "git", Risk::Read, None)], "no route");
        has(&[spec("x", "connections", "database", Risk::Navigate, Some("db_mcp_query"))], "only allowed on a read");
        has(&[spec("x", "connections", "database", Risk::Read, Some("rm_rf"))], "unknown headless");
        let mut t = ok.clone();
        t.timeout_ms = 500;
        has(&[t], "timeout_ms");
        let mut t = ok.clone();
        t.timeout_ms = 999_999;
        has(&[t], "timeout_ms");
        let mut s = ok.clone();
        s.input_schema = json!({"type":"object","properties":{}});
        has(&[s], "additionalProperties");
        let mut s = ok.clone();
        s.input_schema = json!({"type":"array","additionalProperties":false,"properties":{}});
        has(&[s], "type must be");
        let mut s = ok.clone();
        s.input_schema = json!({"type":"object","additionalProperties":false,"properties":{},"required":["a"]});
        has(&[s], "required 'a'");
        let mut s = ok.clone();
        s.input_schema = json!({"type":"object","additionalProperties":false,"properties":{"a":{}}});
        has(&[s], "no type");
        assert!(!catalog_problems(2, &[]).is_empty());
    }

    #[test]
    fn unknown_fields_in_the_file_are_rejected() {
        let bad = r#"{"version":1,"commands":[{"name":"x","module":"shell","route":"","risk":"read",
            "timeout_ms":5000,"description":"long enough description here","input_schema":{},"surprise":1}]}"#;
        assert!(parse(bad).is_err());
        let bad_risk = r#"{"version":1,"commands":[{"name":"x","module":"shell","route":"","risk":"nuke",
            "timeout_ms":5000,"description":"long enough description here","input_schema":{}}]}"#;
        assert!(parse(bad_risk).is_err());
    }

    #[test]
    fn validate_args_enforces_the_schema() {
        let s = &get("db_run_query").unwrap().input_schema;
        assert!(validate_args(s, &json!({"tab_id":"t","statement":"select 1"})).is_ok());
        assert!(validate_args(s, &json!({})).is_ok());
        let e = validate_args(s, &json!({"tab_id": 5})).unwrap_err();
        assert!(e.contains("tab_id must be of type string"), "{e}");
        let e = validate_args(s, &json!({"sql":"x"})).unwrap_err();
        assert!(e.contains("unknown argument 'sql'"), "{e}");
        let e = validate_args(s, &json!({"row_limit": 0})).unwrap_err();
        assert!(e.contains("≥"), "{e}");
        let e = validate_args(s, &json!({"timeout_ms": 1.5})).unwrap_err();
        assert!(e.contains("integer"), "{e}");
        let e = validate_args(s, &json!([1])).unwrap_err();
        assert!(e.contains("object"), "{e}");

        let s = &get("db_set_statement").unwrap().input_schema;
        let e = validate_args(s, &json!({"tab_id":"t"})).unwrap_err();
        assert!(e.contains("missing required argument 'statement'"), "{e}");

        let s = &get("db_page").unwrap().input_schema;
        assert!(validate_args(s, &json!({"tab_id":"t","delta":-1})).is_ok());
        let e = validate_args(s, &json!({"tab_id":"t","delta":2})).unwrap_err();
        assert!(e.contains("one of"), "{e}");

        let s = &get("db_open_object").unwrap().input_schema;
        let e = validate_args(s, &json!({"connection_id":"c","path":[]})).unwrap_err();
        assert!(e.contains("at least 1"), "{e}");
        let e = validate_args(s, &json!({"connection_id":"c","path":["a", 1]})).unwrap_err();
        assert!(e.contains("path[1]"), "{e}");
    }

    #[test]
    fn mutating_follows_risk() {
        assert!(!Risk::Read.mutating());
        assert!(!Risk::Navigate.mutating());
        assert!(Risk::LocalWrite.mutating());
        assert!(Risk::Outward.mutating());
        assert!(get("db_export").unwrap().risk.mutating());
    }
}
