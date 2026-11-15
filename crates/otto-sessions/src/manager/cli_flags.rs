//! Provider CLI flag expansion helpers split out of `manager.rs` (LOC ratchet).
//!
//! Pure over `(provider, meta, external_sandbox)` — expands `meta.extra_dirs`,
//! `meta.model`, `meta.effort`, `meta.interaction_mode`, `meta.lean_turn`, and
//! `meta.read_only` into provider-specific CLI argv flags.

/// Normalize a provider slug (including custom aliases like `"claudework"` or
/// `"codex-fast"`) to its CLI family when inferred, falling back to `provider`.
pub(super) fn effective_cli_family(provider: &str) -> String {
    crate::providers::infer_model_family(None, "", provider).unwrap_or_else(|| provider.to_string())
}

/// Removes provider flags a **read-only** session must not carry, from argv the
/// provider registry built. Codex's `--search` (the model-side live web-search
/// tool) is dropped: a read-only run has no business browsing, and on an
/// untrusted checkout it is an exfiltration channel a prompt injection can
/// steer.
pub(super) fn read_only_strip(provider: &str, meta: &serde_json::Value, args: &mut Vec<String>) {
    let fam = effective_cli_family(provider);
    if fam == "codex" && meta.get("read_only").and_then(|v| v.as_bool()) == Some(true) {
        args.retain(|a| a != "--search");
    }
}

/// Build `["--add-dir", path, ...]` args for providers that support `--add-dir`
/// (claude, codex, agy — NOT shell). Returns an empty vec for unknown/shell
/// providers or when `meta` has no `extra_dirs` array.
pub(super) fn add_dir_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    if provider == "shell" {
        return vec![];
    }
    let Some(arr) = meta.get("extra_dirs").and_then(|v| v.as_array()) else {
        return vec![];
    };
    let mut out = Vec::with_capacity(arr.len() * 2);
    for item in arr {
        if let Some(dir) = item.as_str() {
            if !dir.is_empty() {
                out.push("--add-dir".to_string());
                out.push(dir.to_string());
            }
        }
    }
    out
}

/// Expand a provider's model-flag TEMPLATE (its `ProviderSpec.model_args`,
/// e.g. `["--model","{model}"]` for claude/codex/agy, `["-m","{model}"]` for a
/// custom provider) against `meta.model`.
pub(super) fn model_args(template: Option<&[String]>, meta: &serde_json::Value) -> Vec<String> {
    let Some(template) = template else {
        return vec![];
    };
    let Some(model) = meta.get("model").and_then(|v| v.as_str()) else {
        return vec![];
    };
    let model = model.trim();
    if model.is_empty() {
        return vec![];
    }
    template
        .iter()
        .map(|a| a.replace("{model}", model))
        .collect()
}

/// Expand `meta.effort` (`low | medium | high | xhigh | max | ultra`) into the
/// provider CLI's reasoning-effort flags. Unknown or blank values are ignored.
pub(super) fn effort_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    let Some(raw) = meta.get("effort").and_then(|v| v.as_str()) else {
        return vec![];
    };
    let effort = raw.trim().to_ascii_lowercase();
    if !matches!(
        effort.as_str(),
        "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
    ) {
        return vec![];
    }
    let fam = effective_cli_family(provider);
    match fam.as_str() {
        "claude" | "agy" => {
            let mapped = if effort == "ultra" {
                "max"
            } else {
                effort.as_str()
            };
            vec!["--effort".to_string(), mapped.to_string()]
        }
        "codex" => vec![
            "-c".to_string(),
            format!("model_reasoning_effort=\"{effort}\""),
        ],
        // pi 1.5: `--thinking off|minimal|low|medium|high|xhigh|max`.
        "pi" => {
            let mapped = if effort == "ultra" {
                "max"
            } else {
                effort.as_str()
            };
            vec!["--thinking".to_string(), mapped.to_string()]
        }
        _ => vec![],
    }
}

/// Apply `meta.interaction_mode` (`plan | accept-edits | auto | full`) to the
/// provider CLI's launch argv. Never touches read-only sessions
/// (`meta.read_only = true`), and avoids enabling Codex's inner Seatbelt when
/// Otto's own outer `process_sandbox` (`external_sandbox`) wraps the process.
pub(super) fn interaction_mode_apply(
    provider: &str,
    meta: &serde_json::Value,
    external_sandbox: bool,
    args: &mut Vec<String>,
) {
    if meta.get("read_only").and_then(|v| v.as_bool()) == Some(true) {
        return;
    }
    let Some(mode) = meta
        .get("interaction_mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let fam = effective_cli_family(provider);
    match (fam.as_str(), mode) {
        ("claude", "plan") => {
            args.retain(|a| a != "--dangerously-skip-permissions");
            args.extend(["--permission-mode".to_string(), "plan".to_string()]);
        }
        ("claude", "accept-edits" | "standard") => {
            args.retain(|a| a != "--dangerously-skip-permissions");
            args.extend(["--permission-mode".to_string(), "acceptEdits".to_string()]);
        }
        ("claude", "auto") => {
            args.retain(|a| a != "--dangerously-skip-permissions");
            args.extend(["--permission-mode".to_string(), "auto".to_string()]);
        }
        ("agy", "plan") => {
            args.retain(|a| a != "--dangerously-skip-permissions");
            args.extend(["--mode".to_string(), "plan".to_string()]);
        }
        ("agy", "accept-edits" | "standard") => {
            args.retain(|a| a != "--dangerously-skip-permissions");
            args.extend(["--mode".to_string(), "accept-edits".to_string()]);
        }
        ("agy", "auto") => {
            args.retain(|a| a != "--dangerously-skip-permissions");
            args.extend(["--mode".to_string(), "auto".to_string()]);
        }
        ("codex", "plan") => {
            if external_sandbox {
                args.extend(["-c".to_string(), "approval_policy=\"never\"".to_string()]);
            } else {
                args.retain(|a| a != "--dangerously-bypass-approvals-and-sandbox");
                args.extend([
                    "--sandbox".to_string(),
                    "read-only".to_string(),
                    "--ask-for-approval".to_string(),
                    "never".to_string(),
                ]);
            }
        }
        ("codex", "accept-edits" | "standard") if !external_sandbox => {
            args.retain(|a| a != "--dangerously-bypass-approvals-and-sandbox");
            args.extend([
                "--sandbox".to_string(),
                "workspace-write".to_string(),
                "--ask-for-approval".to_string(),
                "on-request".to_string(),
            ]);
        }
        ("codex", "auto") if !external_sandbox => {
            args.retain(|a| a != "--dangerously-bypass-approvals-and-sandbox");
            args.push("--approve-for-me".to_string());
        }
        _ => {}
    }
}

/// Extra argv for a **lean turn** (`meta.lean_turn = true`, Claude-only).
pub(super) fn lean_turn_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    let fam = effective_cli_family(provider);
    if fam != "claude" || meta.get("lean_turn").and_then(|v| v.as_bool()) != Some(true) {
        return vec![];
    }
    vec![
        "--strict-mcp-config".to_string(),
        "--disallowed-tools".to_string(),
        "Bash Edit Write NotebookEdit WebFetch WebSearch Task".to_string(),
    ]
}

/// Extra argv for a **read-only** session (`meta.read_only = true`).
pub(super) fn read_only_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    if meta.get("read_only").and_then(|v| v.as_bool()) != Some(true) {
        return vec![];
    }
    let untrusted = meta.get("project_settings").and_then(|v| v.as_bool()) == Some(false);
    let fam = effective_cli_family(provider);
    match fam.as_str() {
        "claude" => {
            let subagents = meta.get("allow_subagents").and_then(|v| v.as_bool()) == Some(true);
            let mut denied = vec!["Bash", "NotebookEdit"];
            if !subagents {
                denied.push("Task");
            }
            if untrusted {
                denied.extend(["WebFetch", "WebSearch"]);
            }
            let mut args = vec!["--disallowed-tools".to_string(), denied.join(" ")];
            if untrusted {
                args.push("--setting-sources".to_string());
                args.push("user".to_string());
                args.push("--strict-mcp-config".to_string());
            }
            args
        }
        "codex" if untrusted => vec!["-c".to_string(), "project_doc_max_bytes=0".to_string()],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claudework_expands_effort_interaction_lean_and_read_only_flags() {
        let meta = json!({
            "effort": "high",
            "interaction_mode": "plan",
            "lean_turn": true,
        });
        assert_eq!(effort_args("claudework", &meta), vec!["--effort", "high"]);

        let mut args = vec!["--dangerously-skip-permissions".to_string()];
        interaction_mode_apply("claudework", &meta, false, &mut args);
        assert!(!args.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(args.windows(2).any(|w| w == ["--permission-mode", "plan"]));

        let lean = lean_turn_args("claudework", &meta);
        assert!(lean.contains(&"--strict-mcp-config".to_string()));
        assert!(lean.contains(&"--disallowed-tools".to_string()));

        let ro_meta = json!({
            "read_only": true,
            "project_settings": false,
        });
        let ro = read_only_args("claudework", &ro_meta);
        assert!(ro.contains(&"--disallowed-tools".to_string()));
        assert!(ro.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn codex_custom_alias_strips_search_when_read_only() {
        let mut args = vec!["--search".to_string(), "--full-auto".to_string()];
        read_only_strip("codex-work", &json!({"read_only": true}), &mut args);
        assert_eq!(args, vec!["--full-auto".to_string()]);
    }
}
