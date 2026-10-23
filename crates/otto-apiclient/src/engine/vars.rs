//! `{{var}}` substitution (nested + Postman dynamics) and variable-map helpers.

use super::*;

/// Replace `{{name}}` occurrences with the corresponding variable value
/// (string-coerced). Unknown placeholders are left intact.
/// USER/ENV variables win over the built-in dynamics (`$timestamp` & co) — a
/// user who defines their own `$timestamp` means theirs. Values that
/// themselves contain `{{…}}` re-expand for up to 3 bounded passes (nested
/// vars like `base_url = {{host}}/api` now resolve; the bound keeps
/// self-referencing definitions from looping).
pub fn substitute(input: &str, vars: &serde_json::Map<String, Value>) -> String {
    let mut cur = substitute_once(input, vars);
    for _ in 0..2 {
        if !cur.contains("{{") {
            break;
        }
        let next = substitute_once(&cur, vars);
        if next == cur {
            break; // only unknown placeholders remain
        }
        cur = next;
    }
    cur
}

/// One substitution pass (see [`substitute`] for ordering + recursion rules).
pub(crate) fn substitute_once(input: &str, vars: &serde_json::Map<String, Value>) -> String {
    if !input.contains("{{") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            if let Some(end) = input[i + 2..].find("}}") {
                let name = input[i + 2..i + 2 + end].trim();
                if let Some(v) = vars.get(name) {
                    out.push_str(&value_to_string(v));
                } else if let Some(dynamic) = resolve_dynamic_var(name) {
                    out.push_str(&dynamic);
                } else {
                    // leave placeholder as-is
                    out.push_str(&input[i..i + 2 + end + 2]);
                }
                i = i + 2 + end + 2;
                continue;
            }
        }
        // push the current char (handle utf-8 boundaries via char iteration)
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub(crate) fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Resolve Postman-style dynamic variables (`{{$guid}}`, `{{$timestamp}}`, …).
/// Returns `None` for anything that isn't a known dynamic name.
pub(crate) fn resolve_dynamic_var(name: &str) -> Option<String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    match name {
        "$guid" | "$randomUUID" => Some(uuid::Uuid::new_v4().to_string()),
        "$timestamp" => Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .to_string(),
        ),
        "$isoTimestamp" => Some(chrono::Utc::now().to_rfc3339()),
        "$randomInt" => Some((rand::random::<u32>() % 1000).to_string()),
        _ => None,
    }
}

/// Extract enabled `(key, value)` pairs from a `[{key,value,enabled}]` array.
/// A missing `enabled` field defaults to true.
pub(crate) fn enabled_kv(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(arr) = v.as_array() {
        for item in arr {
            let enabled = item.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            if !enabled {
                continue;
            }
            let key = item.get("key").and_then(Value::as_str).unwrap_or("");
            if key.is_empty() {
                continue;
            }
            let value = item.get("value").and_then(Value::as_str).unwrap_or("");
            out.push((key.to_string(), value.to_string()));
        }
    }
    out
}

/// Reject overrides that could recursively expand a workspace secret.
pub fn check_override_vars(vars: &serde_json::Map<String, Value>) -> Result<(), String> {
    for (key, value) in vars {
        if value.as_str().is_some_and(|value| value.contains("{{")) {
            return Err(format!(
                "vars override '{key}' must not contain '{{{{' (no nested substitution)"
            ));
        }
    }
    Ok(())
}

/// String view of the chained variable map for the script engine.
pub fn string_vars(vars: &serde_json::Map<String, Value>) -> BTreeMap<String, String> {
    vars.iter()
        .map(|(k, v)| (k.clone(), value_to_string(v)))
        .collect()
}

/// The script engine returns a complete snapshot, including deletions.
pub fn merge_string_vars(
    vars: &mut serde_json::Map<String, Value>,
    svars: &BTreeMap<String, String>,
) {
    vars.retain(|key, _| svars.contains_key(key));
    for (k, v) in svars {
        vars.insert(k.clone(), Value::String(v.clone()));
    }
}
