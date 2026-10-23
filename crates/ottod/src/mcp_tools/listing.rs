//! The native `tools/list` entries GENERATED from the control-plane catalog
//! (`otto_mcp::outward::otto_tool_specs`, architecture-1-05). The catalog is
//! the source of truth (user rule: whatever is enabled in the control plane
//! must be visible to sessions, and described one way):
//!
//! - a tool this surface serves natively under the same stdio name (a twin,
//!   `otto_x` ⇄ `otto.x`, listed in `catalog::NATIVE_TWINS`) is advertised
//!   with the catalog's description and `inputSchema`, its session-scoped
//!   `workspace_id` / `session_id` defaulted from the calling session
//!   ([`generate_twin_entries`], [`native_twin_schema`]);
//! - every other enabled catalog entry is proxied under its stdio name (see
//!   `GOVERNED_BY_STDIO_NAME` in `catalog.rs`) or served natively under an
//!   aliased name (`GOVERNED_ALIASED_BY_NATIVE`);
//! - the hand-written tools are only those with no twin — this surface's own
//!   (session-bound, assistant, cloud consoles…).

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

/// One control-plane tool this surface serves natively under its stdio name.
pub(super) struct NativeTwin {
    /// The catalog name without its `otto.` prefix.
    pub(super) short: &'static str,
    /// The session-scoped arguments this surface advertises, `(name,
    /// description)`: defaulted from the calling session, never required.
    pub(super) scoped: &'static [(&'static str, Option<&'static str>)],
    /// A sentence appended to the catalog description where this surface's
    /// answer differs from the outward one.
    pub(super) note: Option<&'static str>,
}

/// The `tools/list` entry of every twin, generated from its `catalog` entry
/// (a twin with no catalog entry is skipped — `every_twin_has_a_dispatch_case`
/// counts them). `stdio_name` maps a catalog name to its stdio spelling.
pub(super) fn generate_twin_entries(
    catalog: &[Value],
    twins: &[NativeTwin],
    stdio_name: impl Fn(&str) -> String,
) -> Vec<Value> {
    twins
        .iter()
        .filter_map(|twin| {
            let full = format!("otto.{}", twin.short);
            let spec = catalog.iter().find(|s| s["name"] == full.as_str())?;
            let mut scoped = serde_json::Map::new();
            for (arg, desc) in twin.scoped {
                let mut prop = serde_json::json!({ "type": "string" });
                if let Some(d) = desc {
                    prop["description"] = Value::from(*d);
                }
                scoped.insert(arg.to_string(), prop);
            }
            let native = serde_json::json!({ "properties": scoped });
            let mut description = stdio_description(spec["description"].as_str().unwrap_or(""));
            if let Some(note) = twin.note {
                description.push(' ');
                description.push_str(note);
            }
            Some(serde_json::json!({
                "name": stdio_name(&full),
                "description": description,
                "inputSchema": native_twin_schema(&spec["inputSchema"], &native),
            }))
        })
        .collect()
}
