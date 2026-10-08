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
        // Swarm task: wired — enqueues via SwarmRepo. Requires swarm_id +
        // project_id in params; the task is created in "todo" status so the
        // swarm coordinator picks it up on its next tick.
        n("swarm_task", "Swarm Task", "AI",
          "Enqueue a task in a running Agent Swarm project.", 1, 1, "#a070ff", "users"),
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
    ];
    // `review_run` is the one kind with a declared PARAM schema today: the
    // inspector renders the execution-mode picker from it (R2). Assigned after
    // the fact so the shared `n(…)` closure stays untouched.
    if let Some(spec) = specs.iter_mut().find(|s| s.kind == "review_run") {
        spec.params_schema = Some(json!({
            "type": "object",
            "properties": {
                "allow_summary_fallback": {
                    "type": "boolean", "default": false,
                    "description": "Allow a deterministic summary when reviewer coverage is complete"
                },
                "mode": {
                    "type": "string",
                    "enum": ["fan_out", "orchestrator"],
                    "description": "Execution mode; absent = follow the run override / stored config / fan_out"
                }
            }
        }));
    }
    specs
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
        "http_request" | "api_run" => obj(&[("status", "number"), ("body", "any")]),
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
        "swarm_task" => obj(&[("task_id", "string"), ("title", "string")]),
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
        // Every other kind stays schema-free (the inspector falls back to its
        // hand-written forms).
        assert!(node_catalog()
            .iter()
            .filter(|s| s.kind != "review_run")
            .all(|s| s.params_schema.is_none()));
    }
}
