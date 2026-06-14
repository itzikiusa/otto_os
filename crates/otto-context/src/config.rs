//! Read/write the per-workspace context config under `Workspace.settings`.
//!
//! Mirrors `otto-improve::config`: the `context` block lives alongside other
//! settings keys (`self_improvement`, `network_listener`, …) and is merged back
//! without disturbing them.

use otto_core::api::WorkspaceContextConfig;
use serde_json::Value;

/// Parse the `context` block out of `Workspace.settings`, falling back to
/// defaults when the key is absent or malformed.
pub fn from_settings(settings: &Value) -> WorkspaceContextConfig {
    settings
        .get("context")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Merge a context config back into a settings JSON value under the `context`
/// key, leaving every other settings key untouched. Returns the new settings
/// object.
pub fn write_into_settings(settings: &Value, cfg: &WorkspaceContextConfig) -> Value {
    let mut obj = settings.as_object().cloned().unwrap_or_default();
    obj.insert(
        "context".to_string(),
        serde_json::to_value(cfg).unwrap_or(Value::Null),
    );
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_block_yields_defaults() {
        let cfg = from_settings(&json!({"other": 1}));
        assert!(cfg.skills.is_none());
        assert!(cfg.soul.is_none());
        assert_eq!(cfg.extra_context_md, "");
        assert!(cfg.include_memory);
    }

    #[test]
    fn null_skills_parses_to_none() {
        let cfg = from_settings(&json!({"context": {"skills": null, "soul": "otto"}}));
        assert!(cfg.skills.is_none());
        assert_eq!(cfg.soul.as_deref(), Some("otto"));
        // include_memory defaults to true when omitted.
        assert!(cfg.include_memory);
    }

    #[test]
    fn explicit_values_parse() {
        let cfg = from_settings(&json!({
            "context": {
                "skills": ["a", "b"],
                "soul": "otto",
                "extra_context_md": "hello",
                "include_memory": false
            }
        }));
        assert_eq!(cfg.skills.as_deref(), Some(&["a".to_string(), "b".to_string()][..]));
        assert_eq!(cfg.extra_context_md, "hello");
        assert!(!cfg.include_memory);
    }

    #[test]
    fn write_preserves_other_keys() {
        let settings = json!({"keep": "me", "self_improvement": {"enabled": true}});
        let cfg = WorkspaceContextConfig {
            soul: Some("otto".into()),
            ..Default::default()
        };
        let out = write_into_settings(&settings, &cfg);
        assert_eq!(out.get("keep").unwrap(), "me");
        assert!(out
            .get("self_improvement")
            .unwrap()
            .get("enabled")
            .unwrap()
            .as_bool()
            .unwrap());
        assert_eq!(
            out.get("context").unwrap().get("soul").unwrap().as_str().unwrap(),
            "otto"
        );
    }
}
