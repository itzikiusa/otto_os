//! The workflow node-kind catalog: drives the editor palette, validates
//! generated graphs and declares each kind's output shape. Pure data — moved
//! out of otto-server's `workflow_engine` (keep in sync with its `execute_node`).

use otto_core::workflows::NodeTypeSpec;
use serde_json::{json, Value};

/// The node-kind catalog: drives the editor palette and validates generated
/// graphs. Keep in sync with `execute_node` below.
pub fn node_catalog() -> Vec<NodeTypeSpec> {
    let n = |kind: &str,
             label: &str,
             category: &str,
             description: &str,
             inputs: u8,
             outputs: u8,
             color: &str,
             icon: &str| NodeTypeSpec {
        kind: kind.to_string(),
        label: label.to_string(),
        category: category.to_string(),
        description: description.to_string(),
        inputs,
        outputs,
        color: color.to_string(),
        icon: icon.to_string(),
        output_schema: output_schema_for(kind),
        params_schema: None,
    };
    let mut specs = vec![
        n("manual_trigger", "Manual Trigger", "Triggers",
          "Starts the workflow and emits its input payload.", 0, 1, "#6b7bff", "play"),
        n("agent_prompt", "Agent", "AI",
          "Run an agent turn with a prompt (params: provider, skill/skills to inject, cwd); outputs its reply.", 1, 1, "#d97cff", "command"),
        n("prepare_context", "Prepare relevant data", "AI",
          "App-side context gathering: fetches a referenced Jira ticket (params: key/account_id/require) into jira-<KEY>.md, then optionally runs an analysis agent (params: prompt/provider).", 1, 1, "#d97cff", "download"),
        n("http_request", "HTTP Request", "Network",
          "Call an HTTP endpoint and capture the response.", 1, 1, "#46c0a0", "globe"),
        n("transform", "Set / Transform", "Data",
          "Merge static JSON into the data flowing through.", 1, 1, "#9aa0aa", "edit"),
        n("delay", "Delay", "Flow",
          "Wait a number of milliseconds, then continue.", 1, 1, "#9aa0aa", "clock"),
        n("log", "Log", "Flow",
          "Record the incoming data in the run log; pass it through.", 1, 1, "#9aa0aa", "note"),
        n("game_engine", "Game Engine", "Game",
          "Assemble a slot game from approved assets (RNG, paytable, reels).", 1, 1, "#57b9ff", "box"),
        n("verifier", "Verifier", "Game",
          "Verify the built game (RNG fairness, RTP, asset integrity).", 1, 1, "#57d98b", "check"),
        // --- Module-native nodes (wired into in-process services) -----------
        n("db_query", "DB Query", "Data",
          "Run a read-only SQL query against a saved DB-Explorer connection.", 1, 1, "#5aafdf", "database"),
        n("broker_peek", "Broker Peek", "Data",
          "Consume up to N recent messages from a Kafka topic.", 1, 1, "#f0a040", "list"),
        n("channel_notify", "Channel Notify", "Integrations",
          "Send a message to a configured Slack/Telegram integration.", 1, 1, "#46c56a", "message-square"),
        n("budget_gate", "Budget Gate", "Flow",
          "Check spend caps: continue if under budget, stop (error) if blocked.", 1, 1, "#e04c4c", "shield"),
        n("human_approval", "Human Approval", "Flow",
          "Pause the run until an operator calls the resume endpoint.", 1, 1, "#f0c040", "user-check"),
        n("condition", "Condition", "Flow",
          "Evaluate an expression on the input; outputs { result, value }. Pair with edge conditions to branch.", 1, 1, "#f0c040", "git-branch"),
        n("loop", "Repeat (Until)", "Flow",
          "Re-run inner steps until an expression holds or max iterations (e.g. fix → review until score ≥ 80).", 1, 1, "#f0c040", "repeat"),
        // --- Product nodes (wired: run a real single-agent turn over the story's
        // context + the matching product skill, as a visible session). ---------
        n("product_analyze", "Product Analyze", "Product",
          "Analyze a product story (grill lens) over its real context; outputs the analysis.", 1, 1, "#ff8c42", "file-text"),
        n("product_rewrite", "Product Rewrite", "Product",
          "Rewrite a product story (jira-story-writer); optionally save a new version.", 1, 1, "#ff8c42", "edit"),
        n("product_plan", "Product Plan", "Product",
          "Break a story into an implementation plan (story-task-breakdown); optional version.", 1, 1, "#ff8c42", "map"),
        n("product_publish", "Product Publish", "Product",
          "Publish a story as a Confluence RFC or a Jira issue (dry-run by default).", 1, 1, "#ff8c42", "upload"),
        n("canvas", "Canvas Diagram", "Product",
          "Generate/update a Canvas scene (mermaid/excalidraw) from a prompt via an agent.", 1, 1, "#57b9ff", "image"),
        // review_run: wired to the local-review engine (run_review_for_branch).
        n("review_run", "Review Run", "AI",
          "Multi-agent code review (params: providers[], lenses[]/skills[], threshold, require_pass) — fans out like PR review, summarizer scores; outputs findings + a 0–100 score + passed.", 1, 1, "#c080ff", "search"),
        n("git_pr", "Git PR", "Network",
          "Draft a PR; with open=true (gate the incoming edge on the review passing) opens it on the remote.", 1, 1, "#46c0a0", "git-pull-request"),
        // api_run: executes an HTTP request via the api-client engine so
        // environment variable substitution and auth apply.  Wired.
        n("api_run", "API Run", "Network",
          "Execute an API-client request with env-var substitution.", 1, 1, "#46c0a0", "send"),
        // self_improve: runs the self-improvement engine in OFFER-ONLY mode
        // (Autonomy::Propose → every edit is queued for approval, never applied)
        // and posts the offered improvements to the trigger's chat thread.
        n("self_improve", "Self-Improve (offer)", "AI",
          "Reflect on recent sessions and OFFER skill/memory improvements (never auto-applied — queued for approval). Posts the offered list to the chat thread.", 1, 1, "#d97cff", "zap"),
        // --- Deterministic glue (engine.md §7): what an agent did by recipe,
        // done in code. -------------------------------------------------------
        n("git_checkout", "Check out PR / branch", "Network",
          "Find the PR (the pr input, then the pr_branch input, then a branch with the Jira key; exactly one open PR or the entry fails), check it out and write source, merge-base and base branch to repos.json.", 1, 1, "#46c0a0", "git-branch"),
        n("script", "Run script", "Flow",
          "Run a shell command (once, or once per repository) with a time limit; run values arrive as OTTO_* environment variables. Its output file becomes the step's handoff.", 1, 1, "#9aa0aa", "command"),
        n("post_review_comments", "Post review comments", "Network",
          "Post a review's findings to the PR: chosen severities inline, notes combined, a summary with the score; never twice.", 1, 1, "#46c0a0", "message-square"),
        n("check", "Check criteria", "Flow",
          "Checks each criterion and records the evidence.", 1, 1, "#f0c040", "target"),
    ];
    // Declared PARAM schemas (the inspector renders pickers from them; a kind
    // without one keeps its hand-written form). Assigned after the fact so
    // the shared `n(…)` closure stays untouched. Every param is optional
    // unless `required`; absent = today's behaviour.
    for spec in specs.iter_mut() {
        spec.params_schema = params_schema_for(&spec.kind);
    }
    specs
}

/// The declared params of a kind (agreed-names.md §4), `None` for the rest.
fn params_schema_for(kind: &str) -> Option<Value> {
    let obj = |props: Value, required: &[&str]| {
        Some(json!({"type": "object", "properties": props, "required": required}))
    };
    let timeout = |min: u32, max: u32, what: &str| json!({"type": "integer", "minimum": min, "maximum": max, "description": what});
    match kind {
        "review_run" => obj(
            json!({
                "allow_summary_fallback": {
                    "type": "boolean", "default": false,
                    "description": "When only the final verifier failed, score the reviewers' unverified findings as the result (every reviewer must still finish); off reports the review as incomplete instead"
                },
                "mode": {
                    "type": "string",
                    "enum": ["fan_out", "orchestrator"],
                    "description": "Execution mode; absent = follow the run override / stored config / fan_out"
                },
                "scope_assessment": {
                    "type": "string", "enum": ["configured"],
                    "description": "Keep to the declared lenses: the scope agent may drop one, never add one; absent = today's behaviour"
                },
                "timeout_s": timeout(60, 36_000, "Fail the step after this many seconds")
            }),
            &[],
        ),
        "agent_prompt" => obj(
            json!({
                "effort": {"type": "string", "description": "Reasoning effort the provider offers; empty = its default"},
                "timeout_s": timeout(60, 36_000, "Fail the step after this many seconds, time waiting on you included"),
                "continue_from": {"type": "string", "description": "Node id of an earlier Agent step on the same provider whose thread this step continues; empty = a new thread"}
            }),
            &[],
        ),
        "manual_trigger" => obj(
            json!({
                "chat_updates": {"type": "string", "enum": ["live", "per_step"], "default": "live",
                    "description": "Chat progress: one live status message, or today's line per step"},
                "inputs": {"type": "array", "description": "The start-a-run form's fields (read by the UI only, never seeded into the run input)",
                    "items": {"type": "object", "properties": {"key": {"type": "string"}, "label": {"type": "string"},
                        "required": {"type": "boolean"}, "multiline": {"type": "boolean"}, "hint": {"type": "string"}}, "required": ["key", "label"]}},
                "lane": {"type": "string", "enum": ["normal", "long"], "default": "normal", "description": "Run queue lane"}
            }),
            &[],
        ),
        "loop" => obj(
            json!({
                "steps": {"type": "array", "items": {"type": "object", "properties": {
                    "same_thread": {"type": "boolean", "default": false, "description": "Continue this inner step's thread from the previous iteration"}}}},
                "max_active_s": {"type": "integer", "minimum": 1, "description": "Active-time budget of the loop"},
                "stop_on_repeat_failure": {"type": "boolean", "description": "Hold the run as blocked when the same check fails twice"},
                "hold_when_unmet": {"type": "boolean", "description": "Hold the run instead of failing when the loop ends unmet"}
            }),
            &[],
        ),
        "git_checkout" => obj(
            json!({"mode": {"type": "string", "enum": ["pr", "branch"], "default": "pr"}}),
            &[],
        ),
        "script" => obj(
            json!({
                "command": {"type": "string", "description": "Run with /bin/sh -c; values come as OTTO_* environment variables, never in the text"},
                "timeout_s": timeout(10, 7_200, "Kill the command (its whole process group) after this many seconds; default 900"),
                "foreach": {"type": "string", "enum": ["none", "repos"], "default": "none"},
                "cwd": {"type": "string", "enum": ["worktree", "context"], "default": "worktree"},
                "output_file": {"type": "string", "description": "A file name in the run's context folder the command writes; its text becomes the step's reply ($OTTO_OUT)"},
                "ok_exit_codes": {"type": "array", "items": {"type": "integer"}, "default": [0]},
                "allow_failure": {"type": "boolean", "default": false},
                "idempotent": {"type": "boolean", "default": false, "description": "Safe to run again after a restart"}
            }),
            &["command", "timeout_s"],
        ),
        "post_review_comments" => obj(
            json!({
                "review_from": {"type": "string", "description": "Node id of the Review Run to post; default the nearest one before this step"},
                "inline": {"type": "array", "items": {"type": "string"}, "default": ["bug", "warn"]},
                "info": {"type": "string", "enum": ["combined", "skip"], "default": "combined"},
                "summary": {"type": "boolean", "default": true},
                "summary_intro_from": {"type": "string", "description": "Node id whose step file opens the summary comment"},
                "summary_files": {"type": "array", "items": {"type": "string"}, "description": "Context file names appended to the summary"},
                "max_inline": {"type": "integer", "minimum": 1, "default": 150}
            }),
            &[],
        ),
        "check" => obj(
            json!({
                "criteria": {"type": "array", "minItems": 1, "maxItems": 40, "items": {"type": "object", "properties": {
                    "id": {"type": "string"}, "text": {"type": "string"},
                    "verify": {"type": "string", "enum": ["command", "agent", "human", "expr"]},
                    "how": {"type": "string"}, "command": {"type": "string"}, "expr": {"type": "string"},
                    "timeout_s": {"type": "integer", "minimum": 10, "maximum": 3_600}}, "required": ["id", "text", "verify"]}},
                "evaluator": {"type": "object", "properties": {"provider": {"type": "string"}, "model": {"type": "string"}, "effort": {"type": "string"}}},
                "instructions": {"type": "string"},
                "final_review": {"type": "boolean", "default": false}
            }),
            &["criteria"],
        ),
        _ => None,
    }
}

/// True when `kind` is a node the executor understands.
pub fn is_known_kind(kind: &str) -> bool {
    node_catalog().iter().any(|s| s.kind == kind)
}

/// Declared output shape per node kind (drives UI expression hints + warn-only
/// runtime validation). Keys map to JSON types; `None` means free-form output.
pub fn output_schema_for(kind: &str) -> Option<Value> {
    let obj = |pairs: &[(&str, &str)]| {
        let mut m = serde_json::Map::new();
        for (k, t) in pairs {
            m.insert((*k).to_string(), json!(t));
        }
        Some(json!({ "type": "object", "fields": Value::Object(m) }))
    };
    match kind {
        "agent_prompt" => obj(&[("reply", "string"), ("working_directory", "string")]),
        "prepare_context" => obj(&[("jira", "object")]),
        "http_request" => obj(&[("status", "number"), ("body", "any")]),
        "api_run" => obj(&[
            ("status", "number"),
            ("body", "any"),
            ("truncated", "boolean"),
            ("too_large", "boolean"),
        ]),
        "db_query" => obj(&[
            ("columns", "array"),
            ("rows", "array"),
            ("rows_returned", "number"),
        ]),
        "broker_peek" => obj(&[
            ("topic", "string"),
            ("messages", "array"),
            ("count", "number"),
        ]),
        "budget_gate" => obj(&[("exceeded", "boolean"), ("blocked", "boolean")]),
        "human_approval" => obj(&[("approved", "boolean"), ("approved_by", "string")]),
        "condition" => obj(&[("result", "boolean"), ("value", "any")]),
        "loop" => obj(&[
            ("iterations", "number"),
            ("satisfied", "boolean"),
            ("last", "any"),
        ]),
        "review_run" => obj(&[
            ("review_id", "string"),
            ("status", "string"),
            ("repo_id", "string"),
            ("base", "string"),
            ("worktree", "string"),
            ("blocking", "number"),
            ("advisory", "number"),
            ("checks_requested", "array"),
            ("score", "number"),
            ("threshold", "number"),
            ("passed", "boolean"),
        ]),
        "product_analyze" => obj(&[("story_id", "string"), ("analysis", "string")]),
        "product_rewrite" => obj(&[("story_id", "string"), ("body_md", "string")]),
        "product_plan" => obj(&[("story_id", "string"), ("plan_md", "string")]),
        "product_publish" => obj(&[
            ("story_id", "string"),
            ("kind", "string"),
            ("dry_run", "boolean"),
        ]),
        "git_pr" => obj(&[
            ("prs", "array"),
            ("opened", "boolean"),
            ("opened_count", "number"),
            ("title", "string"),
            ("description", "string"),
        ]),
        "self_improve" => obj(&[
            ("run_id", "string"),
            ("summary", "string"),
            ("offered", "number"),
            ("edits", "array"),
        ]),
        "canvas" => obj(&[("scene_id", "string"), ("summary", "string")]),
        "git_checkout" => obj(&[
            ("repos", "array"),
            ("resolved", "number"),
            ("unresolved", "array"),
            ("rows", "array"),
        ]),
        "script" => obj(&[
            ("exit_code", "number"),
            ("timed_out", "boolean"),
            ("duration_ms", "number"),
            ("runs", "array"),
            ("reply", "string"),
        ]),
        "post_review_comments" => obj(&[
            ("posted", "number"),
            ("failed", "number"),
            ("duplicates", "number"),
            ("overflow", "number"),
            ("pr", "object"),
        ]),
        "check" => obj(&[
            ("criteria", "array"),
            ("all_met", "boolean"),
            ("met", "number"),
            ("total", "number"),
            ("verdict", "string"),
        ]),
        _ => None,
    }
}

/// Warn-only validation of a node's output against its declared schema. Returns a
/// list of human-readable warnings (missing keys / wrong types). Never fails a run.
pub fn validate_node_output(kind: &str, output: &Value) -> Vec<String> {
    let Some(schema) = output_schema_for(kind) else {
        return vec![];
    };
    let Some(fields) = schema.get("fields").and_then(Value::as_object) else {
        return vec![];
    };
    let Some(obj) = output.as_object() else {
        return vec![format!("{kind}: expected an object output")];
    };
    let mut warns = Vec::new();
    for (key, ty) in fields {
        let ty = ty.as_str().unwrap_or("any");
        match obj.get(key) {
            None => warns.push(format!("{kind}: missing output field '{key}'")),
            Some(v) => {
                let ok = match ty {
                    "string" => v.is_string(),
                    "number" => v.is_number(),
                    "boolean" => v.is_boolean(),
                    "array" => v.is_array(),
                    "object" => v.is_object(),
                    _ => true,
                };
                if !ok && !v.is_null() {
                    warns.push(format!("{kind}: output field '{key}' is not {ty}"));
                }
            }
        }
    }
    warns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_kinds_are_known() {
        assert!(is_known_kind("agent_prompt"));
        assert!(is_known_kind("game_engine"));
        assert!(is_known_kind("prepare_context"));
        assert!(!is_known_kind("nope"));
    }

    #[test]
    fn prepare_context_output_schema_declares_jira() {
        let schema = output_schema_for("prepare_context").expect("schema present");
        assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));
        let fields = schema
            .get("fields")
            .and_then(|v| v.as_object())
            .expect("fields present");
        assert_eq!(fields.get("jira").and_then(|v| v.as_str()), Some("object"));
    }

    #[test]
    fn review_run_catalog_declares_mode_param() {
        let spec = node_catalog()
            .into_iter()
            .find(|s| s.kind == "review_run")
            .expect("review_run in the catalog");
        let schema = spec.params_schema.expect("params_schema");
        let modes = schema["properties"]["mode"]["enum"].clone();
        assert_eq!(modes, json!(["fan_out", "orchestrator"]));
        assert_eq!(
            schema["properties"]["scope_assessment"]["enum"],
            json!(["configured"])
        );
    }

    #[test]
    fn new_kinds_and_params_are_in_the_catalog() {
        let specs = node_catalog();
        let schema = |kind: &str| {
            specs
                .iter()
                .find(|s| s.kind == kind)
                .unwrap_or_else(|| panic!("{kind} in the catalog"))
                .params_schema
                .clone()
        };
        for kind in ["git_checkout", "script", "post_review_comments", "check"] {
            assert!(is_known_kind(kind), "{kind}");
            assert!(
                output_schema_for(kind).is_some(),
                "{kind} declares its output"
            );
        }
        assert_eq!(
            schema("script").unwrap()["required"],
            json!(["command", "timeout_s"])
        );
        assert_eq!(
            schema("git_checkout").unwrap()["properties"]["mode"]["enum"],
            json!(["pr", "branch"])
        );
        let agent = schema("agent_prompt").unwrap();
        for p in ["effort", "timeout_s", "continue_from"] {
            assert!(agent["properties"].get(p).is_some(), "agent_prompt.{p}");
        }
        let start = schema("manual_trigger").unwrap();
        assert_eq!(
            start["properties"]["chat_updates"]["enum"],
            json!(["live", "per_step"])
        );
        assert!(start["properties"].get("inputs").is_some());
        assert!(
            schema("loop").unwrap()["properties"]["steps"]["items"]["properties"]
                .get("same_thread")
                .is_some()
        );
        // Kinds without declared params keep their hand-written forms.
        assert!(schema("http_request").is_none() && schema("log").is_none());
    }
}
