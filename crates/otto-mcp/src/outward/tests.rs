//! Pure tests of the outward catalog, policy lists, routing and pin rules
//! (moved with the code from `otto-server`'s `mcp_outward`).

use otto_core::auth::McpScope;
use otto_core::domain::WorkspaceRole;
use serde_json::json;

use super::*;

fn spec_names() -> Vec<String> {
    otto_tool_specs()
        .iter()
        .filter_map(|t| t["name"].as_str().map(String::from))
        .collect()
}

#[test]
fn self_call_errors_keep_the_whole_actionable_message() {
    let status = reqwest::StatusCode::NOT_FOUND;
    // A candidate listing (the resolver's 404) survives past 400 chars.
    let long = format!("not found: {}", "- r1  repo  (workspace: w)\n".repeat(40));
    let body = json!({"code":"not_found","message": long}).to_string();
    let msg = self_call_error(status, &body);
    assert!(msg.starts_with("404 Not Found: not found: - r1"), "{msg}");
    assert!(msg.len() > 1000, "{}", msg.len());
    // `{error}` bodies too; a raw non-JSON body stays a short snippet.
    assert!(self_call_error(status, r#"{"error":"nope"}"#).ends_with("nope"));
    assert!(self_call_error(status, &"y".repeat(5000)).len() < 450);
    // 204 No Content is a success the agent can read, not `null`.
    assert_eq!(parse_self_ok(""), json!({"ok": true}));
    assert_eq!(parse_self_ok("[1]"), json!([1]));
}

#[test]
fn internal_reviewer_scope_does_not_depend_on_outward_server_toggle() {
    assert!(mcp_tool_enabled_for_token(
        true,
        false,
        false,
        &[],
        "vault_read"
    ));
    assert!(!mcp_tool_enabled_for_token(
        false,
        false,
        false,
        &[],
        "vault_read"
    ));
    assert!(mcp_tool_enabled_for_token(
        false,
        false,
        true,
        &["vault_read".to_string()],
        "vault_read",
    ));
}

/// Q1: a UI-control tool called from an Otto session skips the outward
/// master switch — but never the per-tool enable.
#[test]
fn ui_tools_from_a_session_skip_only_the_master_switch() {
    let on = vec!["ui_db_run_query".to_string()];
    assert!(mcp_tool_enabled_for_token(
        false,
        true,
        false,
        &on,
        "ui_db_run_query"
    ));
    assert!(!mcp_tool_enabled_for_token(
        false,
        true,
        false,
        &[],
        "ui_db_run_query"
    ));
    // Not from a session → the master switch still applies.
    assert!(!mcp_tool_enabled_for_token(
        false,
        false,
        false,
        &on,
        "ui_db_run_query"
    ));
    assert!(mcp_tool_enabled_for_token(
        false,
        false,
        true,
        &on,
        "ui_db_run_query"
    ));
}

#[test]
fn ui_tools_are_default_on_and_survive_an_old_saved_list() {
    let ui = ui_commands::tool_names();
    assert!(!ui.is_empty());
    // Never saved: defaults + every UI tool.
    let d = merge_enabled(None, &[], &ui);
    assert!(ui.iter().all(|t| d.contains(t)));
    assert!(d.contains(&"list_workflows".to_string()));
    // Saved before UI tools existed: they are added.
    let old = merge_enabled(Some(vec!["list_workflows".into()]), &[], &ui);
    assert!(ui.iter().all(|t| old.contains(t)));
    // Saved after: an unchecked UI tool stays off, a checked one stays on,
    // a UI tool added later still turns on.
    let known = vec!["ui_db_run_query".to_string(), "ui_state".to_string()];
    let saved = merge_enabled(Some(vec!["ui_state".into()]), &known, &ui);
    assert!(!saved.contains(&"ui_db_run_query".to_string()));
    assert!(saved.contains(&"ui_state".to_string()));
    assert!(saved.contains(&"ui_db_page".to_string()));
}

#[test]
fn ui_tools_are_classified() {
    for t in ui_commands::catalog() {
        let bare = t.tool();
        assert!(
            !DANGEROUS.contains(&bare.as_str()),
            "{bare} must not be DANGEROUS"
        );
        assert_eq!(tool_is_mutating(&bare), t.risk.mutating(), "{bare}");
        assert!(pin_global(&bare), "{bare}: pin story");
        // A read-only token scope refuses the mutating ones.
        let ro = McpScope {
            tools: None,
            allow_writes: false,
            workspace_id: None,
        };
        assert_eq!(
            ro.deny_reason(&bare, tool_is_mutating(&bare), None)
                .is_some(),
            t.risk.mutating()
        );
    }
    assert!(tool_is_mutating("ui_db_export"));
    assert!(!tool_is_mutating("ui_db_run_query"));
}

#[test]
fn ui_error_envelope_shape() {
    let v = ui_error_envelope("pending_grant", "ask");
    assert_eq!(v["decision"], "error");
    assert_eq!(v["executed"], false);
    assert_eq!(v["is_error"], true);
    assert_eq!(v["code"], "pending_grant");
    assert_eq!(v["content"], json!({"error":"ask","code":"pending_grant"}));
    assert_eq!(ui_error_envelope("timeout", "slow")["executed"], true);
    assert_eq!(
        ui_error_envelope("cancelled_by_user", "no")["executed"],
        true
    );
}

#[test]
fn scheduled_task_tools_are_registered() {
    let names = spec_names();
    for n in [
        "otto.list_scheduled_tasks",
        "otto.list_scheduled_task_runs",
        "otto.create_scheduled_task",
        "otto.update_scheduled_task",
        "otto.set_scheduled_task_enabled",
        "otto.run_scheduled_task",
        "otto.delete_scheduled_task",
    ] {
        assert!(names.contains(&n.to_string()), "missing spec {n}");
    }
}

#[test]
fn write_tools_are_dangerous_reads_are_default_enabled() {
    for w in [
        "create_scheduled_task",
        "update_scheduled_task",
        "delete_scheduled_task",
        "run_scheduled_task",
        "set_scheduled_task_enabled",
    ] {
        assert!(DANGEROUS.contains(&w), "{w} must be DANGEROUS");
        assert!(!DEFAULT_ENABLED.contains(&w), "{w} must be off by default");
    }
    assert!(DEFAULT_ENABLED.contains(&"list_scheduled_tasks"));
    assert!(DEFAULT_ENABLED.contains(&"list_scheduled_task_runs"));
}

#[test]
fn create_tool_is_marked_mutating() {
    let specs = otto_tool_specs();
    let create = specs
        .iter()
        .find(|t| t["name"] == "otto.create_scheduled_task")
        .unwrap();
    assert_eq!(create["mutating"], serde_json::json!(true));
}

#[test]
fn dangerous_detail_surfaces_cadence_and_destination() {
    let args = serde_json::json!({
        "name": "Nightly",
        "schedule": {"cadence": "interval", "every_min": 60},
        "destination": {"type": "slack"},
        "prompt": "do the thing"
    });
    let d = dangerous_detail("otto.create_scheduled_task", &args);
    assert!(d.contains("Nightly"));
    assert!(d.contains("every 60 min"));
    assert!(d.contains("slack"));
    assert!(d.contains("do the thing"));
}

// ----- All-features expansion -----------------------------------------

/// (bare short name, mutating) for every spec.
fn spec_short_mut() -> Vec<(String, bool)> {
    otto_tool_specs()
        .iter()
        .map(|t| {
            let name = t["name"].as_str().unwrap();
            let short = name.strip_prefix("otto.").unwrap_or(name).to_string();
            (short, t["mutating"].as_bool().unwrap())
        })
        .collect()
}

#[test]
fn every_spec_is_well_formed_and_classified() {
    let specs = otto_tool_specs();
    for (short, mutating) in spec_short_mut() {
        let t = specs
            .iter()
            .find(|s| s["name"].as_str().unwrap().strip_prefix("otto.").unwrap() == short)
            .unwrap();
        // category present + non-empty (drives the control-plane UI grouping).
        assert!(
            t["category"]
                .as_str()
                .map(|c| !c.is_empty())
                .unwrap_or(false),
            "{short} missing category"
        );
        // inputSchema is an object; every declared `required` key exists in `properties`.
        assert_eq!(
            t["inputSchema"]["type"],
            json!("object"),
            "{short} schema not an object"
        );
        if let Some(reqd) = t["inputSchema"]["required"].as_array() {
            for r in reqd {
                let key = r.as_str().unwrap();
                assert!(
                    t["inputSchema"]["properties"].get(key).is_some(),
                    "{short}: required '{key}' missing from properties"
                );
            }
        }
        // Classification invariant: mutating ⟺ DANGEROUS; reads are default-on XOR opt-in.
        let s = short.as_str();
        // Agent UI control tools have their own invariants
        // (`ui_tools_are_classified`): never DANGEROUS, default-on via the
        // catalog, mutating iff their risk tier writes.
        if ui_commands::is_ui_tool(s) {
            continue;
        }
        if mutating {
            assert!(
                DANGEROUS.contains(&s),
                "{short} is mutating but not DANGEROUS"
            );
            assert!(
                !DEFAULT_ENABLED.contains(&s),
                "{short} is mutating but default-enabled"
            );
        } else {
            let de = DEFAULT_ENABLED.contains(&s);
            let opt = OPT_IN_READS.contains(&s);
            assert!(
                de ^ opt,
                "{short} (read) must be default-enabled XOR opt-in (de={de}, opt={opt})"
            );
            assert!(
                !DANGEROUS.contains(&s),
                "{short} (read) must not be DANGEROUS"
            );
        }
    }
}

#[test]
fn classification_lists_reference_real_tools() {
    let shorts: std::collections::HashSet<String> =
        spec_short_mut().into_iter().map(|(s, _)| s).collect();
    for n in DEFAULT_ENABLED
        .iter()
        .chain(DANGEROUS.iter())
        .chain(OPT_IN_READS.iter())
    {
        assert!(
            shorts.contains(*n),
            "classification names a non-existent tool '{n}'"
        );
    }
}

#[test]
fn api_client_tools_present_classified_and_routed() {
    const READS: &[&str] = &["api_list", "api_get_request", "api_history"];
    const WRITES: &[&str] = &["api_execute", "api_upsert_request", "api_run_automation"];
    let specs = otto_tool_specs();
    for name in READS.iter().chain(WRITES) {
        let spec = specs
            .iter()
            .find(|spec| spec["name"] == format!("otto.{name}"))
            .unwrap_or_else(|| panic!("missing spec otto.{name}"));
        assert_eq!(spec["category"], json!("API Client"));
    }
    for read in READS {
        assert!(
            DEFAULT_ENABLED.contains(read),
            "{read} must be default-enabled"
        );
        assert!(!DANGEROUS.contains(read), "{read} must not be DANGEROUS");
    }
    for write in WRITES {
        assert!(DANGEROUS.contains(write), "{write} must be DANGEROUS");
        assert!(
            !DEFAULT_ENABLED.contains(write),
            "{write} must be off by default"
        );
    }

    assert_eq!(
        route_for(
            "api_list",
            &json!({"workspace_id":"w1","q":"login","kind":"requests"}),
        )
        .unwrap()
        .path,
        "/api/v1/workspaces/w1/api-client/overview?q=login&kind=requests"
    );
    assert_eq!(
        route_for(
            "api_get_request",
            &json!({"workspace_id":"w1","request_id":"r1"})
        )
        .unwrap()
        .path,
        "/api/v1/workspaces/w1/api-client/requests/r1?shape=agent"
    );
    assert_eq!(
        route_for(
            "api_history",
            &json!({"workspace_id":"w1","limit":5,"source":"agent"}),
        )
        .unwrap()
        .path,
        "/api/v1/workspaces/w1/api-client/history?limit=5&source=agent"
    );
    assert_eq!(
        route_for("api_history", &json!({"workspace_id":"w1","id":"h1"}))
            .unwrap()
            .path,
        "/api/v1/workspaces/w1/api-client/history/h1"
    );

    let execute = route_for(
        "api_execute",
        &json!({"workspace_id":"w1","request_id":"r1","confirm":true}),
    )
    .unwrap();
    assert_eq!(execute.method, Method::Post);
    assert_eq!(
        execute.path,
        "/api/v1/workspaces/w1/api-client/requests/r1/execute"
    );
    assert_eq!(
        execute.body.unwrap(),
        json!({"shape":"agent","confirm":true})
    );

    let update = route_for(
        "api_upsert_request",
        &json!({"workspace_id":"w1","request_id":"r1","name":"Login","method":"post","url":"https://api.example/login"}),
    )
    .unwrap();
    assert_eq!(update.method, Method::Patch);
    assert_eq!(update.path, "/api/v1/workspaces/w1/api-client/requests/r1");
    let create = route_for(
        "api_upsert_request",
        &json!({"workspace_id":"w1","name":"Login","method":"POST","url":"https://api.example/login"}),
    )
    .unwrap();
    assert_eq!(create.method, Method::Post);
    assert_eq!(create.path, "/api/v1/workspaces/w1/api-client/requests");

    let run = route_for(
        "api_run_automation",
        &json!({"workspace_id":"w1","automation_id":"a1"}),
    )
    .unwrap();
    assert_eq!(run.method, Method::Post);
    assert_eq!(
        run.path,
        "/api/v1/workspaces/w1/api-client/automations/a1/run"
    );
    assert_eq!(run.body.unwrap(), json!({}));
}

#[test]
fn api_execute_rejects_brace_overrides_in_route_for() {
    let error = route_for(
        "api_execute",
        &json!({
            "workspace_id":"w1",
            "request_id":"r1",
            "vars":{"base_url":"https://evil.example/?token={{api_token}}"}
        }),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid: vars override 'base_url' must not contain '{{' (no nested substitution)"
    );
}

#[test]
fn dangerous_detail_surfaces_api_targets() {
    assert_eq!(
        dangerous_detail(
            "otto.api_execute",
            &json!({
                "workspace_id":"w1",
                "request_id":"r1",
                "environment_id":"staging",
                "confirm":true,
                "confirm_new_host":false
            }),
        ),
        "API request 'r1' executed against environment 'staging' (confirm=true, confirm_new_host=false) in workspace 'w1'"
    );
    assert_eq!(
        dangerous_detail(
            "otto.api_upsert_request",
            &json!({"workspace_id":"w1","name":"Login","method":"post","url":"https://api.example/login"}),
        ),
        "Save API request 'Login' (POST https://api.example/login) in workspace 'w1'"
    );
    assert_eq!(
        dangerous_detail(
            "otto.api_run_automation",
            &json!({"workspace_id":"w1","automation_id":"smoke"}),
        ),
        "Run API automation 'smoke' in workspace 'w1'"
    );
}

#[test]
fn headline_features_present_and_governed() {
    let names = spec_names();
    for n in [
        "otto.list_workflows",
        "otto.get_workflow_run",
        "otto.run_workflow",
        "otto.cancel_workflow_run",
        "otto.list_broker_clusters",
        "otto.list_broker_topics",
        "otto.consume_broker_messages",
        "otto.produce_broker_message",
    ] {
        assert!(names.contains(&n.to_string()), "missing headline spec {n}");
    }
    assert!(DEFAULT_ENABLED.contains(&"list_workflows"));
    assert!(DEFAULT_ENABLED.contains(&"list_broker_clusters"));
    assert!(DANGEROUS.contains(&"run_workflow"));
    assert!(DANGEROUS.contains(&"produce_broker_message"));
    // Content-heavy reads stay off by default.
    assert!(!DEFAULT_ENABLED.contains(&"consume_broker_messages"));
    assert!(!DEFAULT_ENABLED.contains(&"search_memory"));
}

#[test]
fn room_tools_are_default_enabled_and_route() {
    // Rooms are the auditable agent-to-agent channel: enabled by default,
    // never DANGEROUS (the persistence + user visibility IS the control).
    assert!(DEFAULT_ENABLED.contains(&"room_post"));
    assert!(DEFAULT_ENABLED.contains(&"room_read"));
    assert!(!DANGEROUS.contains(&"room_post"));
    let c = route_for(
        "room_post",
        &json!({"room_id":"r1","text":"hi","session_id":"s1"}),
    )
    .unwrap();
    assert_eq!(c.method, Method::Post);
    assert_eq!(c.path, "/api/v1/agent-rooms/r1/messages");
    assert_eq!(c.body.unwrap(), json!({"text":"hi","session_id":"s1"}));
    let c = route_for(
        "room_read",
        &json!({"room_id":"r1","after":"m9","limit":50}),
    )
    .unwrap();
    assert_eq!(c.method, Method::Get);
    assert_eq!(c.path, "/api/v1/agent-rooms/r1/messages?after=m9&limit=50");
    // No cursor → the room's tail, with a bounded default page.
    let c = route_for("room_read", &json!({"room_id":"r1"})).unwrap();
    assert_eq!(c.path, "/api/v1/agent-rooms/r1/messages?tail=true&limit=50");
    let c = route_for(
        "room_read",
        &json!({"room_id":"r1","before":"m3","limit":"20","session_id":"s1"}),
    )
    .unwrap();
    assert_eq!(
        c.path,
        "/api/v1/agent-rooms/r1/messages?before=m3&limit=20&session_id=s1"
    );
}

#[test]
fn route_for_maps_workflows_and_brokers() {
    assert_eq!(
        route_for("list_workflows", &json!({"workspace_id":"ws1"})).unwrap(),
        SelfCall {
            method: Method::Get,
            path: "/api/v1/workspaces/ws1/workflows".into(),
            body: None
        }
    );
    let c = route_for(
        "run_workflow",
        &json!({"workflow_id":"wf1","input":{"k":1},"start_node":"n2","review_mode":"fan_out"}),
    )
    .unwrap();
    assert_eq!(c.method, Method::Post);
    assert_eq!(c.path, "/api/v1/workflows/wf1/run");
    assert_eq!(
        c.body.unwrap(),
        json!({"input":{"k":1},"start_node":"n2","review_mode":"fan_out"})
    );
    assert_eq!(
        route_for("cancel_workflow_run", &json!({"run_id":"r1"})).unwrap(),
        SelfCall {
            method: Method::Post,
            path: "/api/v1/workflow-runs/r1/cancel".into(),
            body: Some(json!({}))
        }
    );
    assert_eq!(
        route_for(
            "get_broker_topic",
            &json!({"cluster_id":"c1","topic":"orders"})
        )
        .unwrap()
        .path,
        "/api/v1/brokers/clusters/c1/topics/orders"
    );
    let c = route_for(
        "produce_broker_message",
        &json!({"cluster_id":"c1","topic":"orders","value":"hi","key":"k","confirm":true}),
    )
    .unwrap();
    assert_eq!(c.path, "/api/v1/brokers/clusters/c1/topics/orders/produce");
    assert_eq!(
        c.body.unwrap(),
        json!({"value":"hi","key":"k","confirm":true})
    );
    let c = route_for(
        "consume_broker_messages",
        &json!({"cluster_id":"c1","topic":"orders","limit":10,"value_filter":"x"}),
    )
    .unwrap();
    assert_eq!(c.path, "/api/v1/brokers/clusters/c1/topics/orders/consume");
    assert_eq!(c.body.unwrap(), json!({"limit":10,"value_filter":"x"}));
}

#[test]
fn run_workflow_forwards_review_mode() {
    // The override rides alone (no input / start_node) and reaches the route
    // verbatim — the route, not the tool, validates the value.
    let c = route_for(
        "run_workflow",
        &json!({"workflow_id":"wf1","review_mode":"orchestrator"}),
    )
    .unwrap();
    assert_eq!(c.body.unwrap(), json!({"review_mode":"orchestrator"}));
    // Absent ⇒ no key at all, so pre-field callers post exactly what they did.
    let c = route_for("run_workflow", &json!({"workflow_id":"wf1"})).unwrap();
    assert_eq!(c.body.unwrap(), json!({}));
    let spec = otto_tool_specs()
        .into_iter()
        .find(|t| t["name"] == "otto.run_workflow")
        .expect("run_workflow spec");
    assert_eq!(
        spec["inputSchema"]["properties"]["review_mode"]["enum"],
        json!(["fan_out", "orchestrator"])
    );
    assert!(spec["description"]
        .as_str()
        .unwrap()
        .contains("review_mode"));
}

/// "I enabled create_pr, why does it still ask?" — the per-tool exemption
/// is what skips the prompt; without it a mutating tool stays gated.
#[test]
fn approval_gate_honours_the_per_tool_exemption() {
    assert!(DANGEROUS.contains(&"create_pr"));
    // Enabled + mutating + not exempted → gated (the secure default).
    assert!(approval_gated(true, false, false));
    // "Ask before each call" off → no prompt.
    assert!(!approval_gated(true, true, false));
    // A trusted `kind='mcp'` write grant also clears it.
    assert!(!approval_gated(true, false, true));
    // Reads never ask.
    assert!(!approval_gated(false, false, false));
}

#[test]
fn exempt_list_is_validated_normalized_and_pruned() {
    let ok = normalize_exempt_tools(&[
        "otto.create_pr".to_string(),
        "comment_pr".to_string(),
        "create_pr".to_string(),
    ])
    .unwrap();
    assert_eq!(ok, ["create_pr", "comment_pr"]);
    let e = normalize_exempt_tools(&["list_repos".to_string()])
        .unwrap_err()
        .to_string();
    assert!(e.contains("not a mutating tool"), "{e}");
    let e = normalize_exempt_tools(&["nope".to_string()])
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown otto tool"), "{e}");
    // Disabling a tool drops its exemption (re-enabling starts gated).
    let enabled = vec!["create_pr".to_string(), "list_repos".to_string()];
    assert_eq!(prune_exempt_tools(&ok, &enabled), ["create_pr"]);
}

#[test]
fn git_tools_take_a_friendly_repo_ref_and_list_repos_spans_workspaces() {
    let specs = otto_tool_specs();
    let spec = |short: &str| {
        specs
            .iter()
            .find(|t| t["name"] == format!("otto.{short}"))
            .unwrap_or_else(|| panic!("otto.{short}"))
            .clone()
    };
    for short in REPO_REF_TOOLS {
        let s = spec(short);
        let schema = &s["inputSchema"];
        assert_eq!(
            schema["properties"]["repo_id"]["description"],
            json!(REPO_REF_DESC),
            "{short}"
        );
        // Optional: omitted inside a session → the session's repo. And no
        // `workspace_id` property, so the in-session bridge never narrows
        // the cross-workspace resolution to the session's own workspace.
        let required = schema["required"].as_array().cloned().unwrap_or_default();
        assert!(!required.contains(&json!("repo_id")), "{short}");
        assert!(
            schema["properties"].get("workspace_id").is_none(),
            "{short}"
        );
    }
    let lr = spec("list_repos");
    assert!(
        lr["inputSchema"]["required"].is_null(),
        "workspace_id is optional"
    );
    assert!(lr["description"]
        .as_str()
        .unwrap()
        .contains("EVERY workspace"));
    assert_eq!(
        route_for("list_repos", &json!({})).unwrap().path,
        "/api/v1/git/repos/directory"
    );
    assert_eq!(
        route_for("list_repos", &json!({"workspace_id":"ws 1"}))
            .unwrap()
            .path,
        "/api/v1/git/repos/directory?workspace_id=ws%201"
    );
}

#[test]
fn route_for_maps_git_issues_swarm_memory_usage() {
    assert_eq!(
        route_for("get_pr", &json!({"repo_id":"r1","number":7}))
            .unwrap()
            .path,
        "/api/v1/repos/r1/prs/7"
    );
    let c = route_for("create_pr", &json!({"repo_id":"r1","title":"T","description":"D","source_branch":"feat","target_branch":"main"})).unwrap();
    assert_eq!(c.path, "/api/v1/repos/r1/prs");
    assert_eq!(
        c.body.unwrap(),
        json!({"title":"T","description":"D","source_branch":"feat","target_branch":"main"})
    );
    assert_eq!(
        route_for("list_prs", &json!({"repo_id":"r1","state":"open"}))
            .unwrap()
            .path,
        "/api/v1/repos/r1/prs?state=open"
    );
    // The route returns a PAGE, not a bare array — a caller that doesn't
    // know that reads `items` off an array and gets nothing.
    let spec = otto_tool_specs()
        .into_iter()
        .find(|t| t["name"] == "otto.list_prs")
        .expect("list_prs spec");
    assert!(
        spec["description"]
            .as_str()
            .unwrap_or_default()
            .contains("has_more"),
        "list_prs description must describe the page shape: {}",
        spec["description"]
    );

    let c = route_for(
        "search_issues",
        &json!({"account_id":"a1","query":"a = b","project":"X"}),
    )
    .unwrap();
    assert!(c.path.starts_with("/api/v1/issue/search?account_id=a1"));
    assert!(c.path.contains("&q=a%20%3D%20b"), "got {}", c.path);
    assert!(c.path.contains("&project=X"));
    let c = route_for(
        "transition_issue",
        &json!({"account_id":"a1","key":"K-1","transition_id":"21"}),
    )
    .unwrap();
    assert_eq!(c.path, "/api/v1/issue/a1/K-1/transitions");
    assert_eq!(c.body.unwrap(), json!({"transition_id":"21"}));

    let c = route_for(
        "post_swarm_board",
        &json!({"swarm_id":"s1","body":"hello","project_id":"p1"}),
    )
    .unwrap();
    assert_eq!(c.path, "/api/v1/swarm/swarms/s1/board");
    assert_eq!(c.body.unwrap(), json!({"body":"hello","project_id":"p1"}));

    let c = route_for(
        "search_memory",
        &json!({"workspace_id":"ws1","query":"schema","k":5}),
    )
    .unwrap();
    assert_eq!(c.path, "/api/v1/workspaces/ws1/memory/search");
    assert_eq!(c.body.unwrap(), json!({"text":"schema","k":5}));
    assert_eq!(
        route_for(
            "list_memory",
            &json!({"workspace_id":"ws1","collection":"vault"})
        )
        .unwrap()
        .path,
        "/api/v1/workspaces/ws1/memories?collection=vault"
    );

    assert_eq!(
        route_for("get_usage_summary", &json!({"days":7}))
            .unwrap()
            .path,
        "/api/v1/usage/summary?days=7"
    );
    assert_eq!(
        route_for("get_usage_summary", &json!({})).unwrap().path,
        "/api/v1/usage/summary"
    );
    assert_eq!(
        route_for("list_bundled_skills", &json!({})).unwrap(),
        SelfCall {
            method: Method::Get,
            path: "/api/v1/library/bundled".into(),
            body: None
        }
    );
    assert_eq!(
        route_for("list_findings", &json!({"review_id":"rv1"}))
            .unwrap()
            .path,
        "/api/v1/reviews/rv1/findings"
    );
    assert_eq!(
        route_for(
            "broadcast_message",
            &json!({"workspace_id":"ws1","text":"hi"})
        )
        .unwrap()
        .body
        .unwrap(),
        json!({"text":"hi"})
    );
    assert_eq!(
        route_for(
            "test_integration",
            &json!({"workspace_id":"ws1","channel":"slack"})
        )
        .unwrap()
        .path,
        "/api/v1/workspaces/ws1/integrations/slack/test"
    );
}

// ----- AWS / Kubernetes consoles (docs/design/aws-k8s-consoles.md §6) ----

const AWS_READS: &[&str] = &[
    "aws_list_accounts",
    "aws_s3_list_buckets",
    "aws_s3_list_objects",
    "aws_s3_preview",
    "aws_sqs_list_queues",
    "aws_ec2_list_instances",
    "aws_athena_list_tables",
    "aws_athena_get_query",
    "aws_eks_list_clusters",
    "aws_logs_list_groups",
    "aws_logs_filter",
    "aws_logs_insights",
    "aws_logs_get_insights",
];
const K8S_READS: &[&str] = &[
    "k8s_list_clusters",
    "k8s_get_resources",
    "k8s_describe",
    "k8s_logs",
    "k8s_top",
    "k8s_health",
    "k8s_pod_actions_list",
];
const CONSOLE_WRITES: &[&str] = &[
    "aws_athena_query",
    "aws_sqs_send",
    "aws_sqs_peek",
    "k8s_action",
    "k8s_pod_http",
];

#[test]
fn aws_k8s_tools_present_and_classified() {
    let names = spec_names();
    let specs = otto_tool_specs();
    for n in AWS_READS.iter().chain(K8S_READS).chain(CONSOLE_WRITES) {
        assert!(
            names.contains(&format!("otto.{n}")),
            "missing spec otto.{n}"
        );
        let spec = specs
            .iter()
            .find(|s| s["name"] == format!("otto.{n}"))
            .unwrap();
        let cat = spec["category"].as_str().unwrap();
        assert!(
            (n.starts_with("aws_") && cat == "AWS")
                || (n.starts_with("k8s_") && cat == "Kubernetes"),
            "{n} in unexpected category {cat}"
        );
    }
    for r in AWS_READS.iter().chain(K8S_READS) {
        assert!(DEFAULT_ENABLED.contains(r), "{r} must be default-enabled");
        assert!(!DANGEROUS.contains(r), "{r} must not be DANGEROUS");
    }
    for w in CONSOLE_WRITES {
        assert!(DANGEROUS.contains(w), "{w} must be DANGEROUS");
        assert!(!DEFAULT_ENABLED.contains(w), "{w} must be off by default");
        assert!(tool_is_mutating(w));
        let spec = specs
            .iter()
            .find(|s| s["name"] == format!("otto.{w}"))
            .unwrap();
        assert_eq!(spec["mutating"], json!(true));
    }
}

#[test]
fn route_for_maps_aws_console() {
    assert_eq!(
        route_for("aws_list_accounts", &json!({})).unwrap(),
        SelfCall {
            method: Method::Get,
            path: "/api/v1/aws/accounts".into(),
            body: None
        }
    );
    assert_eq!(
        route_for("aws_s3_list_buckets", &json!({"account_id":"a1"}))
            .unwrap()
            .path,
        "/api/v1/aws/accounts/a1/s3/buckets?"
    );
    assert_eq!(
        route_for(
            "aws_s3_list_buckets",
            &json!({"account_id":"a1","region":"eu-west-1"})
        )
        .unwrap()
        .path,
        "/api/v1/aws/accounts/a1/s3/buckets?region=eu-west-1"
    );
    assert_eq!(
        route_for(
            "aws_s3_list_objects",
            &json!({"account_id":"a1","bucket":"b","prefix":"logs/","token":"t","max":50})
        )
        .unwrap()
        .path,
        "/api/v1/aws/accounts/a1/s3/buckets/b/objects?prefix=logs%2F&token=t&max=50"
    );
    assert_eq!(
        route_for(
            "aws_s3_preview",
            &json!({"account_id":"a1","bucket":"b","key":"a b.json","max_bytes":1024})
        )
        .unwrap()
        .path,
        "/api/v1/aws/accounts/a1/s3/buckets/b/preview?key=a%20b.json&max_bytes=1024"
    );
    assert_eq!(
        route_for(
            "aws_s3_preview",
            &json!({"account_id":"a1","bucket":"b","key":"k"})
        )
        .unwrap()
        .path,
        "/api/v1/aws/accounts/a1/s3/buckets/b/preview?key=k"
    );
    assert_eq!(
        route_for(
            "aws_sqs_list_queues",
            &json!({"account_id":"a1","prefix":"orders"})
        )
        .unwrap()
        .path,
        "/api/v1/aws/accounts/a1/sqs/queues?prefix=orders"
    );
    // Peek: POST, visibility timeout pinned to 0, max clamped 1..10.
    let c = route_for(
        "aws_sqs_peek",
        &json!({"account_id":"a1","url":"https://sqs/q","max":99}),
    )
    .unwrap();
    assert_eq!(c.method, Method::Post);
    assert_eq!(c.path, "/api/v1/aws/accounts/a1/sqs/queues/peek?");
    assert_eq!(
        c.body.unwrap(),
        json!({"url":"https://sqs/q","visibility_timeout":0,"max":10})
    );
    let c = route_for("aws_sqs_send", &json!({"account_id":"a1","url":"https://sqs/q.fifo","body":"{}","group_id":"g1","delay_seconds":5})).unwrap();
    assert_eq!(c.method, Method::Post);
    assert_eq!(c.path, "/api/v1/aws/accounts/a1/sqs/queues/send?");
    assert_eq!(
        c.body.unwrap(),
        json!({"url":"https://sqs/q.fifo","body":"{}","group_id":"g1","delay_seconds":5})
    );
    assert_eq!(
        route_for(
            "aws_ec2_list_instances",
            &json!({"account_id":"a1","region":"us-east-1","state":"running","q":"web"})
        )
        .unwrap()
        .path,
        "/api/v1/aws/accounts/a1/ec2/instances?region=us-east-1&state=running&q=web"
    );
    assert_eq!(
        route_for(
            "aws_athena_list_tables",
            &json!({"account_id":"a1","database":"db","catalog":"AwsDataCatalog"})
        )
        .unwrap()
        .path,
        "/api/v1/aws/accounts/a1/athena/tables?database=db&catalog=AwsDataCatalog"
    );
    let c = route_for(
        "aws_athena_query",
        &json!({"account_id":"a1","sql":"SELECT 1","database":"db","workgroup":"primary"}),
    )
    .unwrap();
    assert_eq!(c.method, Method::Post);
    assert_eq!(c.path, "/api/v1/aws/accounts/a1/athena/query?");
    assert_eq!(
        c.body.unwrap(),
        json!({"sql":"SELECT 1","database":"db","workgroup":"primary"})
    );
    assert_eq!(
        route_for(
            "aws_athena_get_query",
            &json!({"account_id":"a1","query_execution_id":"q-1","token":"t2","max":100})
        )
        .unwrap()
        .path,
        "/api/v1/aws/accounts/a1/athena/query/q-1?token=t2&max=100"
    );
    assert_eq!(
        route_for(
            "aws_eks_list_clusters",
            &json!({"account_id":"a1","region":"eu-west-1"})
        )
        .unwrap()
        .path,
        "/api/v1/aws/accounts/a1/eks/clusters?region=eu-west-1"
    );
    // Required ids are enforced.
    assert!(route_for("aws_s3_list_objects", &json!({"account_id":"a1"})).is_err());
    assert!(route_for("aws_athena_query", &json!({"account_id":"a1"})).is_err());
    assert!(route_for("aws_sqs_send", &json!({"account_id":"a1","url":"u"})).is_err());
}

#[test]
fn route_for_maps_k8s_console() {
    assert_eq!(
        route_for("k8s_list_clusters", &json!({})).unwrap(),
        SelfCall {
            method: Method::Get,
            path: "/api/v1/k8s/clusters".into(),
            body: None
        }
    );
    assert_eq!(
        route_for(
            "k8s_get_resources",
            &json!({"cluster_id":"c1","kind":"pods","namespace":"prod","label":"app=web"})
        )
        .unwrap()
        .path,
        "/api/v1/k8s/clusters/c1/resources?kind=pods&ns=prod&label=app%3Dweb"
    );
    // No namespace ⇒ no `ns=` (route default = all namespaces).
    assert_eq!(
        route_for(
            "k8s_get_resources",
            &json!({"cluster_id":"c1","kind":"deployments"})
        )
        .unwrap()
        .path,
        "/api/v1/k8s/clusters/c1/resources?kind=deployments"
    );
    assert_eq!(
        route_for(
            "k8s_describe",
            &json!({"cluster_id":"c1","kind":"deployments","namespace":"prod","name":"web"})
        )
        .unwrap()
        .path,
        "/api/v1/k8s/clusters/c1/resource?kind=deployments&name=web&ns=prod"
    );
    // Cluster-scoped kinds (nodes, namespaces) have no namespace.
    assert_eq!(
        route_for(
            "k8s_describe",
            &json!({"cluster_id":"c1","kind":"nodes","name":"ip-10-0-0-1"})
        )
        .unwrap()
        .path,
        "/api/v1/k8s/clusters/c1/resource?kind=nodes&name=ip-10-0-0-1"
    );
    let c = route_for("k8s_logs", &json!({"cluster_id":"c1","namespace":"prod","pod":"web-1","container":"app","tail":200,"since":"10m","previous":true,"follow":true})).unwrap();
    assert_eq!(c.method, Method::Get);
    assert_eq!(c.path, "/api/v1/k8s/clusters/c1/pods/prod/web-1/logs?container=app&tail=200&since=10m&previous=true");
    assert!(!c.path.contains("follow"), "follow must never be forwarded");
    assert!(TEXT_TOOLS.contains(&"k8s_logs"));
    assert_eq!(
        route_for("k8s_top", &json!({"cluster_id":"c1","namespace":"prod"}))
            .unwrap()
            .path,
        "/api/v1/k8s/clusters/c1/metrics?ns=prod"
    );
    assert_eq!(
        route_for("k8s_health", &json!({"cluster_id":"c1","window":"6h"}))
            .unwrap()
            .path,
        "/api/v1/k8s/clusters/c1/monitor/health?window=6h"
    );
    assert_eq!(
        route_for("k8s_health", &json!({"cluster_id":"c1"}))
            .unwrap()
            .path,
        "/api/v1/k8s/clusters/c1/monitor/health?"
    );
    let c = route_for("k8s_action", &json!({"cluster_id":"c1","action":"scale","kind":"deployments","namespace":"prod","name":"web","params":{"replicas":3}})).unwrap();
    assert_eq!(c.method, Method::Post);
    assert_eq!(c.path, "/api/v1/k8s/clusters/c1/actions");
    assert_eq!(
        c.body.unwrap(),
        json!({"action":"scale","kind":"deployments","ns":"prod","name":"web","params":{"replicas":3}})
    );
    // params defaults to {} so the route's confirm_name check sees an object.
    let c = route_for("k8s_action", &json!({"cluster_id":"c1","action":"restart","kind":"deployments","namespace":"prod","name":"web"})).unwrap();
    assert_eq!(c.body.unwrap()["params"], json!({}));
    assert!(route_for("k8s_action", &json!({"cluster_id":"c1","action":"restart"})).is_err());
    let c = route_for("k8s_pod_http", &json!({"cluster_id":"c1","namespace":"shop","port":8081,"method":"POST","path":"/actuator/loggers/com.acme","workload":{"kind":"deployment","name":"api"},"body":"{}","confirm_name":"api"})).unwrap();
    assert!(c.path.ends_with("/k8s/clusters/c1/pod-http"), "{}", c.path);
    let b = c.body.unwrap();
    assert_eq!(b["workload"]["name"], "api");
    assert_eq!(b["confirm_name"], "api");
    assert!(b.get("pod").is_none());
    assert!(route_for(
        "k8s_pod_http",
        &json!({"cluster_id":"c1","namespace":"shop"})
    )
    .is_err());
    assert!(route_for("k8s_describe", &json!({"cluster_id":"c1","kind":"pods"})).is_err());
}

#[test]
fn dangerous_detail_surfaces_console_targets() {
    let d = dangerous_detail(
        "otto.k8s_action",
        &json!({"cluster_id":"c1","action":"delete_pod","kind":"pods","namespace":"prod","name":"web-1"}),
    );
    assert!(
        d.contains("delete_pod")
            && d.contains("pods/web-1")
            && d.contains("prod")
            && d.contains("c1"),
        "{d}"
    );
    let d = dangerous_detail(
        "otto.aws_sqs_send",
        &json!({"account_id":"a1","url":"https://sqs/q"}),
    );
    assert!(d.contains("https://sqs/q") && d.contains("a1"), "{d}");
    let d = dangerous_detail(
        "otto.aws_athena_query",
        &json!({"account_id":"a1","sql":"SELECT * FROM t","database":"db"}),
    );
    assert!(
        d.contains("SELECT * FROM t") && d.contains("db") && d.contains("a1"),
        "{d}"
    );
}

#[test]
fn route_for_rejects_missing_args_and_unknown_tool() {
    assert!(route_for("list_workflows", &json!({})).is_err());
    assert!(route_for("get_pr", &json!({"repo_id":"r1"})).is_err()); // missing integer `number`
    assert!(route_for("create_pr", &json!({"repo_id":"r1","title":"T"})).is_err());
    assert!(route_for("transition_issue", &json!({"account_id":"a1","key":"K"})).is_err());
    assert!(route_for("frobnicate", &json!({})).is_err());
}

#[test]
fn query_db_readonly_sql_guard_lives_in_route_for() {
    assert!(route_for(
        "query_db_readonly",
        &json!({"connection_id":"c1","statement":"SELECT 1"})
    )
    .is_ok());
    assert!(route_for(
        "query_db_readonly",
        &json!({"connection_id":"c1","statement":"DELETE FROM t"})
    )
    .is_err());
    assert!(route_for(
        "query_db_readonly",
        &json!({"connection_id":"c1","statement":"SELECT 1; DROP TABLE t"})
    )
    .is_err());
}

#[test]
fn assistant_memory_tools_present_classified_and_routed() {
    let names = spec_names();
    for n in [
        "otto.assistant_remember",
        "otto.assistant_forget",
        "otto.assistant_recall",
    ] {
        assert!(names.contains(&n.to_string()), "missing assistant spec {n}");
    }
    // Writes are approval-gated; the recall read is opt-in (personal content).
    assert!(DANGEROUS.contains(&"assistant_remember"));
    assert!(DANGEROUS.contains(&"assistant_forget"));
    assert!(OPT_IN_READS.contains(&"assistant_recall"));
    assert!(!DEFAULT_ENABLED.contains(&"assistant_recall"));
    let c = route_for(
        "assistant_remember",
        &json!({"text":"prefers aisle seats","session_id":"s1","bogus":1}),
    )
    .unwrap();
    assert_eq!(c.method, Method::Post);
    assert_eq!(c.path, "/api/v1/assistant/agent/remember");
    let body = c.body.unwrap();
    assert_eq!(body["text"], "prefers aisle seats");
    assert_eq!(body["session_id"], "s1");
    assert!(
        body.get("bogus").is_none(),
        "only declared args are forwarded"
    );
    assert_eq!(
        route_for("assistant_forget", &json!({"query":"seats"}))
            .unwrap()
            .path,
        "/api/v1/assistant/agent/forget"
    );
    assert_eq!(
        route_for("assistant_recall", &json!({})).unwrap().path,
        "/api/v1/assistant/agent/recall"
    );
    assert!(route_for("assistant_remember", &json!({})).is_err());
    assert!(route_for("assistant_forget", &json!({})).is_err());
    assert!(
        dangerous_detail("otto.assistant_remember", &json!({"text":"likes tea"}))
            .contains("likes tea")
    );
}

#[test]
fn self_improvement_tools_present_classified_and_routed() {
    let names = spec_names();
    for n in [
        "otto.list_improvement_runs",
        "otto.list_improvement_edits",
        "otto.approve_improvement_edit",
        "otto.reject_improvement_edit",
        "otto.rollback_improvement_edit",
        "otto.run_self_improvement",
    ] {
        assert!(
            names.contains(&n.to_string()),
            "missing self-improvement spec {n}"
        );
    }
    assert!(DEFAULT_ENABLED.contains(&"list_improvement_edits"));
    assert!(DANGEROUS.contains(&"approve_improvement_edit"));
    assert!(DANGEROUS.contains(&"reject_improvement_edit"));
    assert!(DANGEROUS.contains(&"rollback_improvement_edit"));
    assert_eq!(
        route_for("list_improvement_edits", &json!({"workspace_id":"ws1"}))
            .unwrap()
            .path,
        "/api/v1/workspaces/ws1/improvement/edits"
    );
    assert_eq!(
        route_for("approve_improvement_edit", &json!({"edit_id":"e1"})).unwrap(),
        SelfCall {
            method: Method::Post,
            path: "/api/v1/improvement/edits/e1/approve".into(),
            body: Some(json!({}))
        }
    );
    assert_eq!(
        route_for("reject_improvement_edit", &json!({"edit_id":"e1"}))
            .unwrap()
            .path,
        "/api/v1/improvement/edits/e1/reject"
    );
    assert_eq!(
        route_for("rollback_improvement_edit", &json!({"edit_id":"e1"}))
            .unwrap()
            .path,
        "/api/v1/improvement/edits/e1/rollback"
    );
    assert!(
        dangerous_detail("otto.approve_improvement_edit", &json!({"edit_id":"e9"})).contains("e9")
    );
}

#[test]
fn dangerous_detail_surfaces_new_tool_targets() {
    assert!(dangerous_detail("otto.run_workflow", &json!({"workflow_id":"wf-9"})).contains("wf-9"));
    assert!(dangerous_detail(
        "otto.produce_broker_message",
        &json!({"topic":"orders","cluster_id":"c1"})
    )
    .contains("orders"));
    let d = dangerous_detail(
        "otto.create_pr",
        &json!({"repo_id":"r1","title":"Fix","source_branch":"f","target_branch":"main"}),
    );
    assert!(d.contains("Fix") && d.contains("main"));
    assert!(
        dangerous_detail("otto.broadcast_message", &json!({"text":"hello team"}))
            .contains("hello team")
    );
}

// ----- Friendly references + the workspace pin (every non-git tool) ----

#[test]
fn pin_verdict_denies_what_it_cannot_verify() {
    let pinned = McpScope {
        tools: None,
        allow_writes: true,
        workspace_id: Some("ws-a".into()),
    };
    // Resolved into the pin → allowed; into another workspace → denied.
    assert!(pin_verdict(
        &pinned,
        "run_workflow",
        &json!({"workflow_id":"W","workspace_id":"ws-a"})
    )
    .is_none());
    let d = pin_verdict(
        &pinned,
        "run_workflow",
        &json!({"workflow_id":"W","workspace_id":"ws-b"}),
    )
    .unwrap();
    assert!(d.contains("scoped to workspace 'ws-a'"), "{d}");
    // No workspace established (e.g. a finding) → fail closed.
    let d = pin_verdict(&pinned, "get_finding", &json!({"finding_id":"F"})).unwrap();
    assert!(d.contains("cannot"), "{d}");
    // Every tool classified unverifiable really is denied to a pin.
    for t in PIN_UNVERIFIABLE {
        assert!(pin_verdict(&pinned, t, &json!({})).is_some(), "{t}");
    }
    // Global rows are fine without a workspace.
    assert!(pin_verdict(&pinned, "k8s_top", &json!({"cluster_id":"C"})).is_none());
    assert!(pin_verdict(&pinned, "list_workflows", &json!({})).is_none());
    // An unpinned scope never needs a workspace.
    let open = McpScope::unrestricted();
    assert!(pin_verdict(&open, "get_finding", &json!({"finding_id":"F"})).is_none());
}

#[test]
fn pr_number_aliases_and_strings_normalize_before_hashing() {
    assert_eq!(
        normalize_args("get_pr", &json!({"repo_id":"r","pr_number":"52"})).unwrap(),
        json!({"repo_id":"r","number":52})
    );
    assert_eq!(
        normalize_args("start_pr_review", &json!({"number":7})).unwrap(),
        json!({"pr_number":7})
    );
    // Already canonical → untouched (no churn in the audit/hash).
    assert!(normalize_args("comment_pr", &json!({"number":3,"body":"x"})).is_none());
    assert!(normalize_args("list_workflows", &json!({"number":3})).is_none());
}

#[test]
fn confluence_page_urls_and_transition_names_resolve() {
    assert_eq!(confluence_page_id("12345").as_deref(), Some("12345"));
    assert_eq!(
        confluence_page_id("https://x.atlassian.net/wiki/spaces/ST/pages/98765/My+Page").as_deref(),
        Some("98765")
    );
    assert_eq!(
        confluence_page_id("https://x/wiki/pages/viewpage.action?pageId=4242").as_deref(),
        Some("4242")
    );
    assert!(confluence_page_id("My Page").is_none());
    let list = json!([
        {"id":"11","name":"Start progress","to_status":"In Progress"},
        {"id":"21","name":"Done","to_status":"Done"}
    ]);
    assert_eq!(match_transition(&list, "in progress").unwrap(), "11");
    assert_eq!(match_transition(&list, "Start Progress").unwrap(), "11");
    let e = match_transition(&list, "Reopen").unwrap_err().to_string();
    assert!(e.contains("21") && e.contains("Done"), "{e}");
}

#[test]
fn an_update_keeps_every_field_it_did_not_send() {
    let stored = json!({"id":"q","name":"Login","method":"POST","url":"u",
        "headers":[{"key":"X-Tenant","value":"7","enabled":true}],"query":[],
        "body_mode":"json","body":"{\"a\":1}","collection_id":"c1","ssh_connection_id":null});
    let merged = merge_stored_request(
        &json!({"workspace_id":"w","request_id":"q","name":"Login","method":"POST","url":"u2"}),
        &stored,
    );
    assert_eq!(merged["url"], "u2", "sent fields win");
    assert_eq!(merged["headers"][0]["key"], "X-Tenant");
    assert_eq!(
        (merged["body_mode"].as_str(), merged["body"].as_str()),
        (Some("json"), Some("{\"a\":1}"))
    );
    assert_eq!(merged["collection_id"], "c1");
    assert!(
        merged.get("ssh_connection_id").is_none(),
        "a null stays absent"
    );
    // An explicit value (even empty) is the agent's choice.
    let merged = merge_stored_request(&json!({"headers":[]}), &stored);
    assert_eq!(merged["headers"], json!([]));
}

#[test]
fn long_running_tools_get_a_longer_self_call_budget() {
    assert_eq!(call_timeout("list_workflows", &json!({})).as_secs(), 30);
    assert_eq!(
        call_timeout("api_execute", &json!({"timeout_ms": 60000})).as_secs(),
        75
    );
    assert_eq!(
        call_timeout("api_execute", &json!({"timeout_ms": 999999})).as_secs(),
        75
    );
    assert!(call_timeout("api_run_automation", &json!({})).as_secs() >= 120);
    assert!(call_timeout("k8s_logs", &json!({})).as_secs() > 60);
}

#[test]
fn new_discovery_and_git_tools_route_and_classify() {
    for r in [
        "list_workspaces",
        "list_goal_loops",
        "list_agent_rooms",
        "list_issue_accounts",
        "list_issue_transitions",
        "get_scheduled_task",
        "list_swarm_projects",
        "list_swarm_tasks",
        "list_design_projects",
        "list_pr_reviews",
        "get_pr_checks",
    ] {
        assert!(
            DEFAULT_ENABLED.contains(&r),
            "{r} should be a default-on read"
        );
    }
    assert!(OPT_IN_READS.contains(&"get_pr_diff"));
    assert!(DANGEROUS.contains(&"merge_pr"));
    let c = route_for(
        "merge_pr",
        &json!({"repo_id":"r","number":4,"strategy":"squash"}),
    )
    .unwrap();
    assert_eq!(
        (c.method, c.path.as_str()),
        (Method::Post, "/api/v1/repos/r/prs/4/merge")
    );
    assert_eq!(c.body.unwrap(), json!({"strategy":"squash"}));
    assert!(dangerous_detail("otto.merge_pr", &json!({"repo_id":"r","number":4})).contains("#4"));
    assert_eq!(
        route_for(
            "list_prs",
            &json!({"repo_id":"r","state":"all","page":2,"per_page":100})
        )
        .unwrap()
        .path,
        "/api/v1/repos/r/prs?state=all&page=2&per_page=100"
    );
    assert_eq!(
        route_for("list_prs", &json!({"repo_id":"r"})).unwrap().path,
        "/api/v1/repos/r/prs"
    );
    assert_eq!(
        route_for("list_pr_reviews", &json!({"repo_id":"r","pr_number":9}))
            .unwrap()
            .path,
        "/api/v1/repos/r/prs/9/reviews"
    );
    assert_eq!(
        route_for("get_pr_checks", &json!({"repo_id":"r","number":9}))
            .unwrap()
            .path,
        "/api/v1/repos/r/prs/9/checks"
    );
    // get_pr_diff pages a capped PR diff per file.
    assert_eq!(
        route_for("get_pr_diff", &json!({"repo_id":"r","number":9}))
            .unwrap()
            .path,
        "/api/v1/repos/r/prs/9/diff"
    );
    assert_eq!(
        route_for(
            "get_pr_diff",
            &json!({"repo_id":"r","number":9,"path":"src/a b.rs","full":true})
        )
        .unwrap()
        .path,
        "/api/v1/repos/r/prs/9/diff?path=src%2Fa%20b.rs&full=true"
    );
    assert_eq!(
        route_for(
            "list_issue_transitions",
            &json!({"account_id":"a","key":"K-1"})
        )
        .unwrap()
        .path,
        "/api/v1/issue/a/K-1/transitions"
    );
    assert_eq!(
        route_for(
            "search_issues",
            &json!({"account_id":"a","query":"x","start_at":25})
        )
        .unwrap()
        .path,
        "/api/v1/issue/search?account_id=a&q=x&start_at=25"
    );
    assert_eq!(
        route_for("get_scheduled_task", &json!({"task_id":"t"}))
            .unwrap()
            .path,
        "/api/v1/scheduled-tasks/t"
    );
    assert_eq!(
        route_for("list_workflow_runs", &json!({"workflow_id":"w"}))
            .unwrap()
            .path,
        "/api/v1/workflows/w/runs?summary=true"
    );
    assert_eq!(
        route_for(
            "list_improvement_edits",
            &json!({"workspace_id":"ws","status":"applied"})
        )
        .unwrap()
        .path,
        "/api/v1/workspaces/ws/improvement/edits?status=applied"
    );
    assert_eq!(
        route_for("list_swarm_projects", &json!({"swarm_id":"s"}))
            .unwrap()
            .path,
        "/api/v1/swarm/swarms/s/projects"
    );
    assert_eq!(
        route_for("get_swarm_board", &json!({"swarm_id":"s","task_id":"t"}))
            .unwrap()
            .path,
        "/api/v1/swarm/swarms/s/board?task_id=t"
    );
    let c = route_for(
        "create_pr",
        &json!({"repo_id":"r","title":"T","description":"D",
        "source_branch":"f","target_branch":"main","draft":true,"reviewers":["ann"]}),
    )
    .unwrap();
    assert_eq!(c.body.unwrap()["reviewers"], json!(["ann"]));
    let c = route_for(
        "start_pr_review",
        &json!({"repo_id":"r","pr_number":3,"context":"focus auth"}),
    )
    .unwrap();
    assert_eq!(c.body.unwrap(), json!({"context":"focus auth"}));
    // update_scheduled_task never PATCHes the filled-in workspace_id.
    let c = route_for(
        "update_scheduled_task",
        &json!({"task_id":"t","workspace_id":"ws","name":"n"}),
    )
    .unwrap();
    assert_eq!(c.body.unwrap(), json!({"name":"n"}));
    // Stringified numbers from clients that stringify every argument.
    assert_eq!(
        route_for("get_pr", &json!({"repo_id":"r","number":"12"}))
            .unwrap()
            .path,
        "/api/v1/repos/r/prs/12"
    );
    let c = route_for(
        "aws_sqs_peek",
        &json!({"account_id":"a","url":"u","max":"3"}),
    )
    .unwrap();
    assert_eq!(c.body.unwrap()["max"], json!(3));
}

// ----- Vault: optional workspace_id ------------------------------------

#[test]
fn confluence_page_tools_route_and_are_tiered() {
    // Reads are default-enabled like the other Confluence read; every write
    // is outward-facing (it publishes to a real wiki) and must be DANGEROUS.
    let names = spec_names();
    for n in [
        "otto.get_confluence_page",
        "otto.list_confluence_page_comments",
        "otto.create_confluence_page",
        "otto.update_confluence_page",
        "otto.comment_confluence_page",
    ] {
        assert!(names.contains(&n.to_string()), "missing spec {n}");
    }
    for r in ["get_confluence_page", "list_confluence_page_comments"] {
        assert!(
            mcp_tool_enabled_for_token(true, false, false, &[], r),
            "{r} should be a default-enabled read"
        );
    }

    let acc = "acc1";
    let pid = "12345";
    let read = route_for(
        "get_confluence_page",
        &json!({"account_id": acc, "page_id": pid}),
    )
    .unwrap();
    assert_eq!(read.method, Method::Get);
    assert_eq!(
        read.path,
        "/api/v1/issue/confluence/pages/12345?account_id=acc1"
    );
    assert!(read.body.is_none());

    // Create sends Markdown, and omits parent_id entirely when not supplied
    // (an empty string would make Confluence reject the call).
    let create = route_for(
        "create_confluence_page",
        &json!({"account_id": acc, "space_key": "STOR", "title": "T", "body_md": "# hi"}),
    )
    .unwrap();
    assert_eq!(create.method, Method::Post);
    assert_eq!(
        create.path,
        "/api/v1/issue/confluence/pages?account_id=acc1"
    );
    let body = create.body.unwrap();
    assert_eq!(body["space_key"], json!("STOR"));
    assert_eq!(body["body_md"], json!("# hi"));
    assert!(body.get("parent_id").is_none());

    let nested = route_for(
        "create_confluence_page",
        &json!({"account_id": acc, "space_key": "STOR", "title": "T",
                "body_md": "x", "parent_id": "999"}),
    )
    .unwrap();
    assert_eq!(nested.body.unwrap()["parent_id"], json!("999"));

    // Update never carries a version — the server resolves it.
    let upd = route_for(
        "update_confluence_page",
        &json!({"account_id": acc, "page_id": pid, "body_md": "b"}),
    )
    .unwrap();
    assert_eq!(upd.method, Method::Put);
    assert_eq!(
        upd.path,
        "/api/v1/issue/confluence/pages/12345?account_id=acc1"
    );
    let ub = upd.body.unwrap();
    assert!(
        ub.get("version").is_none(),
        "callers must not send a version"
    );
    assert!(ub.get("title").is_none(), "absent title must stay absent");

    let cmt = route_for(
        "comment_confluence_page",
        &json!({"account_id": acc, "page_id": pid, "body_md": "answer"}),
    )
    .unwrap();
    assert_eq!(cmt.method, Method::Post);
    assert_eq!(
        cmt.path,
        "/api/v1/issue/confluence/pages/12345/comments?account_id=acc1"
    );
    assert_eq!(cmt.body.unwrap()["body_md"], json!("answer"));
}

#[test]
fn design_tools_are_default_on_reads_and_route_to_the_design_api() {
    const READS: &[&str] = &["design_list", "design_get", "design_links", "design_search"];
    let specs = otto_tool_specs();
    for r in READS {
        let spec = specs
            .iter()
            .find(|s| s["name"] == format!("otto.{r}"))
            .unwrap_or_else(|| panic!("missing spec otto.{r}"));
        assert_eq!(spec["category"], json!("Design"));
        assert_eq!(spec["mutating"], json!(false));
        assert!(DEFAULT_ENABLED.contains(r), "{r} must be default-enabled");
        assert!(!DANGEROUS.contains(r), "{r} must not be DANGEROUS");
        assert!(!tool_is_mutating(r));
        // The library is global: no design tool requires a workspace.
        let reqd = spec["inputSchema"]["required"].as_array().unwrap();
        assert!(!reqd.iter().any(|x| x == "workspace_id"), "{r}");
    }
    assert_eq!(
        route_for("design_list", &json!({})).unwrap().path,
        "/api/v1/design/artifacts"
    );
    assert_eq!(
        route_for("design_list", &json!({"studio": "3d", "limit": 5}))
            .unwrap()
            .path,
        "/api/v1/design/artifacts?studio=3d&limit=5"
    );
    assert_eq!(
        route_for(
            "design_list",
            &json!({"limit": 2, "cursor": "2026-09-23T10:00:00Z|A9"})
        )
        .unwrap()
        .path,
        "/api/v1/design/artifacts?limit=2&cursor=2026-09-23T10%3A00%3A00Z%7CA9"
    );
    assert_eq!(
        route_for("design_get", &json!({"artifact_id": "A1", "version": "v3"}))
            .unwrap()
            .path,
        "/api/v1/design/artifacts/A1?content=true&version=v3"
    );
    assert_eq!(
        route_for("design_links", &json!({"artifact_id": "A1"}))
            .unwrap()
            .path,
        "/api/v1/design/artifacts/A1/links?dir=both"
    );
    let c = route_for(
        "design_search",
        &json!({"query": "hero card", "studio": "site"}),
    )
    .unwrap();
    assert_eq!(c.method, Method::Get);
    assert_eq!(c.path, "/api/v1/design/search?q=hero%20card&studio=site");
    assert!(route_for("design_get", &json!({})).is_err());
    assert!(route_for("design_search", &json!({})).is_err());
}

#[test]
fn design_writes_are_approval_gated_and_route_to_the_design_api() {
    let specs = otto_tool_specs();
    for w in ["design_assist", "design_link"] {
        let spec = specs
            .iter()
            .find(|s| s["name"] == format!("otto.{w}"))
            .unwrap_or_else(|| panic!("missing spec otto.{w}"));
        assert_eq!(spec["category"], json!("Design"));
        assert_eq!(spec["mutating"], json!(true));
        assert!(DANGEROUS.contains(&w), "{w} must be approval-gated");
        assert!(!DEFAULT_ENABLED.contains(&w), "{w} must be off by default");
        assert!(tool_is_mutating(w));
        // The artifact carries the workspace — no design tool requires one.
        let reqd = spec["inputSchema"]["required"].as_array().unwrap();
        assert!(!reqd.iter().any(|x| x == "workspace_id"), "{w}");
        assert!(
            dangerous_detail(&format!("otto.{w}"), &json!({"artifact_id": "A1"})).contains("A1"),
            "{w}"
        );
        // Gated by default; the per-tool exemption is the only opt-out.
        let exempt = normalize_exempt_tools(&[format!("otto.{w}")]).unwrap();
        assert_eq!(exempt, vec![w.to_string()]);
        assert!(approval_gated(DANGEROUS.contains(&w), false, false));
        assert!(!approval_gated(DANGEROUS.contains(&w), true, false));
    }
    // Approving a version stays human-only: no design tool approves.
    assert!(!specs.iter().any(|s| s["name"]
        .as_str()
        .is_some_and(|n| n.starts_with("otto.design_") && n.contains("approve"))));

    let c = route_for(
        "design_assist",
        &json!({"artifact_id": "A 1", "prompt": "bolder hero", "mode": "refine",
                "references": ["B2@v3"], "workspace_id": "ignored"}),
    )
    .unwrap();
    assert_eq!(c.method, Method::Post);
    assert_eq!(c.path, "/api/v1/design/artifacts/A%201/assist");
    let b = c.body.unwrap();
    assert_eq!(b["prompt"], "bolder hero");
    assert_eq!(b["mode"], "refine");
    assert_eq!(b["references"][0], "B2@v3");
    assert!(b.get("workspace_id").is_none());
    let l = route_for(
        "design_link",
        &json!({"artifact_id": "A1", "rel": "implements", "dst_kind": "story",
                "dst_id": "S1", "policy": ""}),
    )
    .unwrap();
    assert_eq!(l.method, Method::Post);
    assert_eq!(l.path, "/api/v1/design/artifacts/A1/links");
    let b = l.body.unwrap();
    assert_eq!(b["rel"], "implements");
    assert_eq!(b["dst_kind"], "story");
    assert_eq!(b["dst_id"], "S1");
    assert!(b.get("policy").is_none(), "empty optional args are dropped");
    assert!(route_for("design_assist", &json!({"artifact_id": "A1"})).is_err());
    assert!(route_for(
        "design_link",
        &json!({"artifact_id": "A1", "rel": "embeds"})
    )
    .is_err());
}

#[test]
fn vault_tools_do_not_require_workspace_id() {
    // Vaults are a global library: `workspace_id` is an optional hint on
    // every vault tool (`fill_vault_workspace` scopes omitted calls
    // server-side), while vault addressing stays strict via `vault_id`.
    for t in otto_tool_specs() {
        if t["category"] != json!("Vault") {
            continue;
        }
        let name = t["name"].as_str().unwrap();
        let reqd = t["inputSchema"]["required"].as_array().unwrap();
        assert!(
            !reqd.iter().any(|r| r == "workspace_id"),
            "{name} must not require workspace_id"
        );
        assert!(
            t["inputSchema"]["properties"].get("workspace_id").is_some(),
            "{name} lost the workspace_id property"
        );
        if name != "otto.vault_list" {
            assert!(
                reqd.iter().any(|r| r == "vault_id"),
                "{name} must require vault_id"
            );
        }
    }
}

#[test]
fn pick_vault_workspace_prefers_writable_for_mutating_tools() {
    fn ws(id: &str) -> otto_core::domain::Workspace {
        otto_core::domain::Workspace {
            id: id.into(),
            name: id.into(),
            root_path: format!("/tmp/{id}"),
            settings: json!({}),
            archived: false,
            created_at: chrono::Utc::now(),
        }
    }
    let rows = vec![
        (ws("view-only"), WorkspaceRole::Viewer),
        (ws("editable"), WorkspaceRole::Editor),
    ];
    // Reads take the first accessible workspace; writes skip ahead to the
    // first Editor+ membership.
    assert_eq!(
        pick_vault_workspace(&rows, false).as_deref(),
        Some("view-only")
    );
    assert_eq!(
        pick_vault_workspace(&rows, true).as_deref(),
        Some("editable")
    );
    // Viewer-only memberships still resolve for a mutating tool — the
    // self-call's native RBAC owns the denial.
    let viewer_only = vec![(ws("view-only"), WorkspaceRole::Viewer)];
    assert_eq!(
        pick_vault_workspace(&viewer_only, true).as_deref(),
        Some("view-only")
    );
    assert_eq!(pick_vault_workspace(&[], false), None);
}
