//! The native `tools/list` schemas GENERATED from the control-plane catalog
//! (`otto_mcp::outward::otto_tool_specs`, architecture-1-05). The catalog is
//! the source of truth (user rule: whatever is enabled in the control plane
//! must be visible to sessions, and described one way):
//!
//! - a native tool with a governed twin (`otto_x` ⇄ `otto.x`) advertises the
//!   twin's `inputSchema`, with the session-scoped `workspace_id` /
//!   `session_id` defaulted from the calling session ([`native_twin_schema`]);
//! - every other enabled catalog entry is proxied under its stdio name (see
//!   `GOVERNED_BY_STDIO_NAME` in `mcp_tools.rs`) or served natively under an
//!   aliased name (`GOVERNED_ALIASED_BY_NATIVE`);
//! - the hand-written tools with no twin are this surface's own
//!   (session-bound, assistant, cloud consoles…).
//!
//! The twin's hand-written schema contributes ONLY its scoped
//! `workspace_id` / `session_id` properties — everything else comes from the
//! catalog, so the two can no longer drift.

use serde_json::Value;

/// Arguments the native surface defaults from the calling session: never
/// required natively, and advertised only when the hand-written native tool
/// advertises them (with its own session-aware description).
pub(super) const SESSION_SCOPED_ARGS: [&str; 2] = ["workspace_id", "session_id"];

/// The stdio spelling of the tool references in a catalog description:
/// `otto.x` → `otto_x` (Claude/Codex mangle a dotted tool name).
pub(super) fn stdio_description(s: &str) -> String {
    s.replace("otto.", "otto_")
}

/// The native `inputSchema` of a twin: the governed `catalog` schema with
/// descriptions in the stdio spelling, `workspace_id` / `session_id` dropped
/// from `required`, and those two properties taken from the hand-written
/// `native` schema (absent there → absent here). Pure — unit-tested.
pub(super) fn native_twin_schema(catalog: &Value, native: &Value) -> Value {
    fn rewrite(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    match (k.as_str(), x) {
                        ("description", Value::String(s)) => *s = stdio_description(s),
                        (_, x) => rewrite(x),
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(rewrite),
            _ => {}
        }
    }
    let mut out = catalog.clone();
    rewrite(&mut out);
    let Some(obj) = out.as_object_mut() else {
        return out;
    };
    if let Some(props) = obj.get_mut("properties").and_then(Value::as_object_mut) {
        for scoped in SESSION_SCOPED_ARGS {
            match native.get("properties").and_then(|p| p.get(scoped)) {
                Some(own) => {
                    props.insert(scoped.to_string(), own.clone());
                }
                None => {
                    props.remove(scoped);
                }
            }
        }
    }
    if let Some(req) = obj.get_mut("required").and_then(Value::as_array_mut) {
        req.retain(|r| !r.as_str().is_some_and(|r| SESSION_SCOPED_ARGS.contains(&r)));
    }
    if obj
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        obj.remove("required");
    }
    out
}

/// Replace every twin's hand-written `inputSchema` in `tools` with the one
/// generated from its catalog entry. `stdio_name` maps a catalog name to its
/// stdio spelling.
pub(super) fn generate_twin_schemas(
    tools: &mut [Value],
    catalog: &[Value],
    stdio_name: impl Fn(&str) -> String,
) {
    let by_stdio: std::collections::HashMap<String, &Value> = catalog
        .iter()
        .filter_map(|s| Some((stdio_name(s["name"].as_str()?), s)))
        .collect();
    for tool in tools.iter_mut() {
        let Some(spec) = tool["name"].as_str().and_then(|n| by_stdio.get(n)) else {
            continue;
        };
        let generated = native_twin_schema(&spec["inputSchema"], &tool["inputSchema"]);
        tool["inputSchema"] = generated;
    }
}
