//! Credential scrubbing for IDE state-archive tables (`ide_project_environments`,
//! `ide_run_configs`), preserving internal `__otto_*` metadata JSON intact.

use regex::{Captures, Regex};
use serde_json::{json, Value};
use std::sync::OnceLock;

use super::super::ArchiveRow;
use super::{scrub_json, scrub_shell_text, scrub_string, secret_key};

pub(super) fn is_otto_internal_key(key: &str) -> bool {
    key.trim().starts_with("__otto_")
}

pub(super) fn disable_network_bridge_entry(obj: &mut serde_json::Map<String, Value>) {
    if let Some(Value::String(val_str)) = obj.get_mut("value") {
        if let Ok(Value::Object(mut bridge)) = serde_json::from_str::<Value>(val_str) {
            bridge.insert("enabled".into(), json!(false));
            if let Ok(serialized) = serde_json::to_string(&bridge) {
                *val_str = serialized;
            }
        }
    }
}

pub(super) fn disable_ide_network_bridge_row(row: &mut ArchiveRow) {
    if let Some(Value::String(raw)) = row.get_mut("env_vars_json") {
        if let Ok(Value::Array(mut entries)) = serde_json::from_str::<Value>(raw) {
            for entry in &mut entries {
                if let Some(obj) = entry.as_object_mut() {
                    if obj.get("key").and_then(Value::as_str).map(str::trim)
                        == Some("__otto_network_bridge__")
                    {
                        disable_network_bridge_entry(obj);
                    }
                }
            }
            if let Ok(serialized) = serde_json::to_string(&Value::Array(entries)) {
                *raw = serialized;
            }
        }
    }
}

/// Scrub `ide_project_environments.env_vars_json` (array of `{key, value, secret, secret_ref}`),
/// preserving all `__otto_*` metadata entries (`__otto_module_sdks__`, `__otto_go_build_tags__`,
/// `__otto_module_go_build_tags__`, `__otto_network_bridge__`) untouched.
pub(super) fn sanitize_ide_project_env_vars(document: &mut Value) -> bool {
    let Some(entries) = document.as_array_mut() else {
        return false;
    };
    let mut scrubbed_any = false;
    for entry in entries.iter_mut() {
        let Some(obj) = entry.as_object_mut() else {
            continue;
        };
        let key_name = obj
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if is_otto_internal_key(&key_name) {
            if key_name.trim() == "__otto_network_bridge__" {
                disable_network_bridge_entry(obj);
            }
            continue;
        }
        let is_sec = obj.get("secret").and_then(Value::as_bool) == Some(true);
        if is_sec {
            if obj.get("secret_ref").is_some_and(|v| !v.is_null())
                || obj.get("value").is_some_and(|v| !v.is_null())
            {
                scrubbed_any = true;
            }
            obj.insert("secret_ref".into(), Value::Null);
            obj.insert("value".into(), Value::Null);
        } else if secret_key(&key_name) {
            if obj
                .get("value")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
            {
                scrubbed_any = true;
            }
            obj.insert("value".into(), json!(""));
        } else if let Some(Value::String(val_str)) = obj.get_mut("value") {
            if scrub_string_preserving_otto_json(val_str) {
                scrubbed_any = true;
            }
        }
    }
    scrubbed_any
}

/// Scrub all credential-bearing columns of an `ide_run_configs` row (`args_json`,
/// `build_flags_json`, `env_overrides_json`, `command`, `before_launch`) without
/// corrupting valid JSON or `__otto_*` internal metadata keys.
pub(super) fn sanitize_ide_run_config_row(row: &mut ArchiveRow) -> bool {
    let mut scrubbed_any = false;

    if let Some(Value::String(text)) = row.get_mut("env_overrides_json") {
        if let Ok(mut doc) = serde_json::from_str::<Value>(text) {
            let changed = match &mut doc {
                Value::Object(map) => sanitize_env_override_map(map),
                Value::Array(_) => sanitize_ide_project_env_vars(&mut doc),
                _ => false,
            };
            if changed {
                *text = doc.to_string();
                scrubbed_any = true;
            }
        }
    }

    for col in ["args_json", "build_flags_json"] {
        if let Some(Value::String(text)) = row.get_mut(col) {
            if let Ok(mut doc) = serde_json::from_str::<Value>(text) {
                if sanitize_cli_argv_json(&mut doc) {
                    *text = doc.to_string();
                    scrubbed_any = true;
                }
            }
        }
    }

    for col in ["command", "before_launch"] {
        if let Some(Value::String(text)) = row.get_mut(col) {
            if let Ok(mut doc) = serde_json::from_str::<Value>(text) {
                if doc.is_object() || doc.is_array() {
                    let before = doc.clone();
                    scrub_json_preserving_otto(&mut doc);
                    let shell_changed = scrub_shell_json_preserving_otto(&mut doc);
                    if shell_changed || doc != before {
                        *text = doc.to_string();
                        scrubbed_any = true;
                    }
                    continue;
                }
            }
            let (next, changed) = scrub_shell_text(text);
            if changed {
                *text = next;
                scrubbed_any = true;
            }
        }
    }

    scrubbed_any
}

fn sanitize_env_override_map(map: &mut serde_json::Map<String, Value>) -> bool {
    let mut changed = false;
    for (k, v) in map.iter_mut() {
        if is_otto_internal_key(k) {
            continue;
        }
        if secret_key(k) {
            if !v.is_null() && v.as_str() != Some("") {
                *v = json!("");
                changed = true;
            }
            continue;
        }
        match v {
            Value::String(s) => {
                if scrub_string_preserving_otto_json(s) {
                    changed = true;
                }
            }
            other => {
                let before = other.clone();
                scrub_json_preserving_otto(other);
                if *other != before {
                    changed = true;
                }
            }
        }
    }
    changed
}

fn sanitize_cli_argv_json(doc: &mut Value) -> bool {
    let Some(items) = doc.as_array_mut() else {
        let before = doc.clone();
        scrub_json_preserving_otto(doc);
        return *doc != before;
    };
    let mut changed = false;
    let mut i = 0usize;
    while i < items.len() {
        if let Some(flag_key) = items[i].as_str().and_then(secret_cli_flag_expecting_next) {
            if !is_otto_internal_key(flag_key) {
                if let Some(Value::String(next_val)) = items.get_mut(i + 1) {
                    if !next_val.is_empty()
                        && next_val != "[REDACTED]"
                        && !next_val.starts_with('-')
                    {
                        *next_val = "[REDACTED]".into();
                        changed = true;
                        i += 2;
                        continue;
                    }
                }
            }
        }
        if let Value::String(arg) = &mut items[i] {
            if scrub_cli_token(arg) {
                changed = true;
            }
        } else {
            let before = items[i].clone();
            scrub_json_preserving_otto(&mut items[i]);
            if items[i] != before {
                changed = true;
            }
        }
        i += 1;
    }
    changed
}

fn secret_cli_flag_expecting_next(token: &str) -> Option<&str> {
    if token.contains('=') || !token.starts_with('-') {
        return None;
    }
    let name = token.trim_start_matches('-');
    if name.is_empty() || is_otto_internal_key(name) {
        return None;
    }
    secret_key(name).then_some(name)
}

fn scrub_cli_token(arg: &mut String) -> bool {
    if arg.is_empty() || is_otto_internal_key(arg) {
        return false;
    }
    if let Some((k, _)) = arg.split_once('=') {
        let bare = k.trim_start_matches('-');
        if is_otto_internal_key(bare) {
            return false;
        }
    }
    let mut changed = scrub_string_preserving_otto_json(arg);
    static RE_PROP_EQ: OnceLock<Regex> = OnceLock::new();
    let re_prop = RE_PROP_EQ.get_or_init(|| {
        Regex::new(r#"(^|[\s'"=])(-D|--?X\s*|--?|)([A-Za-z_][A-Za-z0-9_.-]*)=([^\s'"]+)"#)
            .expect("regex")
    });
    let before = arg.clone();
    let replaced = re_prop
        .replace_all(&before, |caps: &Captures<'_>| {
            let full_key = &caps[3];
            let leaf_key = full_key.rsplit('.').next().unwrap_or(full_key);
            if !is_otto_internal_key(full_key)
                && !is_otto_internal_key(leaf_key)
                && (secret_key(full_key) || secret_key(leaf_key))
                && &caps[4] != "[REDACTED]"
            {
                format!("{}{}{}=[REDACTED]", &caps[1], &caps[2], full_key)
            } else {
                caps[0].to_owned()
            }
        })
        .into_owned();
    if replaced != *arg {
        *arg = replaced;
        changed = true;
    }
    changed
}

fn scrub_string_preserving_otto_json(s: &mut String) -> bool {
    if let Ok(mut nested) = serde_json::from_str::<Value>(s) {
        if nested.is_object() || nested.is_array() {
            let before = nested.clone();
            scrub_json_preserving_otto(&mut nested);
            if nested != before {
                *s = nested.to_string();
                return true;
            }
            return false;
        }
    }
    let s1 = scrub_string(s);
    let (s2, _) = scrub_shell_text(&s1);
    if s2 != *s {
        *s = s2;
        true
    } else {
        false
    }
}

fn scrub_json_preserving_otto(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let preserved: Vec<(String, Value)> = map
                .iter()
                .filter(|(k, _)| is_otto_internal_key(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            scrub_json(value);
            if let Value::Object(after) = value {
                for (k, v) in preserved {
                    after.insert(k, v);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                scrub_json_preserving_otto(item);
            }
        }
        Value::String(s) => {
            let _ = scrub_string_preserving_otto_json(s);
        }
        _ => {}
    }
}

fn scrub_shell_json_preserving_otto(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = false;
            for (k, v) in map.iter_mut() {
                if is_otto_internal_key(k) {
                    continue;
                }
                changed |= scrub_shell_json_preserving_otto(v);
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for v in items.iter_mut() {
                changed |= scrub_shell_json_preserving_otto(v);
            }
            changed
        }
        Value::String(s) => {
            let (next, changed) = scrub_shell_text(s);
            if changed {
                *s = next;
            }
            changed
        }
        _ => false,
    }
}
