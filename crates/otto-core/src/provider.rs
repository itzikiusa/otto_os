//! Default-agent resolution shared across the daemon.
//!
//! The "which agent CLI should we spawn?" decision follows one precedence chain
//! everywhere a provider isn't explicitly chosen by the user:
//!
//! ```text
//! explicit pick  ->  per-workspace default  ->  global default  ->  "claude"
//! ```
//!
//! Channel replies, the ⌘K orchestrator, and the PR-review default config all
//! resolve through [`resolve_provider`] so the behaviour stays consistent.

use serde_json::Value;

/// The ultimate fallback agent when nothing is configured anywhere.
pub const FALLBACK_PROVIDER: &str = "claude";

/// Resolve a provider name from an ordered list of candidate strings: the first
/// non-blank candidate wins; if every candidate is blank, returns
/// [`FALLBACK_PROVIDER`]. Whitespace-only candidates count as blank.
///
/// Pass candidates highest-precedence first, e.g.
/// `resolve_provider(&[explicit_pick, workspace_default, global_default])`.
pub fn resolve_provider(candidates: &[&str]) -> String {
    candidates
        .iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .unwrap_or(FALLBACK_PROVIDER)
        .to_string()
}

/// Read the `default_provider` field out of a workspace's `settings` JSON
/// object. Returns `""` when absent or not a string (i.e. "no workspace
/// default"), which [`resolve_provider`] treats as unset.
pub fn workspace_default(settings: &Value) -> &str {
    settings
        .get("default_provider")
        .and_then(Value::as_str)
        .unwrap_or("")
}

/// Interpret a global setting value (the `default_provider` settings key is
/// stored as a bare JSON string) as a provider name. Returns `""` when the
/// value is absent or not a string.
pub fn global_default(setting: Option<&Value>) -> &str {
    setting.and_then(Value::as_str).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_pick_wins_then_workspace_then_global_then_claude() {
        assert_eq!(resolve_provider(&["codex", "agy", "shell"]), "codex");
        assert_eq!(resolve_provider(&["", "agy", "shell"]), "agy");
        assert_eq!(resolve_provider(&["", "", "shell"]), "shell");
        assert_eq!(resolve_provider(&["", "", ""]), "claude");
        assert_eq!(resolve_provider(&[]), "claude");
    }

    #[test]
    fn blank_and_whitespace_candidates_are_skipped() {
        assert_eq!(resolve_provider(&["   ", "codex"]), "codex");
        assert_eq!(resolve_provider(&["  codex  "]), "codex");
        assert_eq!(resolve_provider(&["   ", "  "]), "claude");
    }

    #[test]
    fn workspace_default_reads_string_field_only() {
        assert_eq!(workspace_default(&json!({"default_provider": "codex"})), "codex");
        assert_eq!(workspace_default(&json!({"default_provider": 42})), "");
        assert_eq!(workspace_default(&json!({"notes": "hi"})), "");
        assert_eq!(workspace_default(&json!({})), "");
    }

    #[test]
    fn global_default_reads_bare_string_value() {
        assert_eq!(global_default(Some(&json!("agy"))), "agy");
        assert_eq!(global_default(Some(&json!(true))), "");
        assert_eq!(global_default(None), "");
    }
}
