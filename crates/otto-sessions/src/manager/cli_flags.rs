//! Provider CLI flag expansion helpers split out of `manager.rs` (LOC ratchet).
//!
//! Pure over `(provider, meta, external_sandbox)` — expands `meta.extra_dirs`,
//! `meta.model`, `meta.effort`, `meta.interaction_mode`, `meta.lean_turn`, and
//! `meta.read_only` into provider-specific CLI argv flags.

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
    match provider {
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
    match (provider, mode) {
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
    if provider != "claude" || meta.get("lean_turn").and_then(|v| v.as_bool()) != Some(true) {
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
    match provider {
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
