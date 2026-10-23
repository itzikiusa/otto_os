//! Catalog-shape tests: native `otto_x` / governed `otto.x` twins advertise
//! one `inputSchema` (architecture-1-05, interim), the PR-number alias, and
//! the `tools/list` byte budget + narrow-source scoping. Split out of
//! `mcp_tools.rs`'s test module to keep that file under its LOC ratchet.

use super::*;

/// Twins whose native schema may still differ from the catalog. EMPTY since
/// the twin schemas are generated from `otto_mcp::outward` (listing.rs,
/// architecture-1-05); the native dispatch takes the catalog's arguments
/// (`request_id` / `automation_id` / `environment_id` as id-or-name). This
/// list may only stay empty: the test fails when a listed twin stops drifting.
const DEFERRED_SCHEMA_DRIFT: &[&str] = &[];

/// Catalog entries deliberately NOT visible on this surface — neither served
/// natively (as a twin or under an aliased name) nor proxied. Empty: the user
/// rule is that whatever the control plane enables reaches sessions.
const INTENTIONALLY_ABSENT: &[&str] = &[];

/// The schema a twin is compared by. Two differences are the native
/// surface's design, not drift: `workspace_id` / `session_id` are
/// defaulted from the calling session (optional or absent natively,
/// required by the governed executor), and tool references in
/// descriptions use this surface's `otto_x` spelling for `otto.x`.
fn comparable_schema(schema: &Value) -> Value {
    fn walk(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    if k == "description" {
                        if let Value::String(s) = x {
                            *s = s.replace("otto.", "otto_");
                        }
                    } else {
                        walk(x);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(walk),
            _ => {}
        }
    }
    let mut s = schema.clone();
    for scoped in ["workspace_id", "session_id"] {
        if let Some(props) = s.get_mut("properties").and_then(Value::as_object_mut) {
            props.remove(scoped);
        }
        if let Some(req) = s.get_mut("required").and_then(Value::as_array_mut) {
            req.retain(|r| r != scoped);
        }
    }
    if s.get("required")
        .and_then(Value::as_array)
        .is_some_and(|r| r.is_empty())
    {
        s.as_object_mut().unwrap().remove("required");
    }
    walk(&mut s);
    s
}

/// architecture-1-05: a capability served natively as `otto_x` AND governed
/// as `otto.x` advertises the SAME `inputSchema` (modulo
/// [`comparable_schema`]), so a session and the Control Plane describe one
/// tool one way. The listing is generated from the catalog; this guards the
/// generator (native wins by name in `tools/list`).
#[test]
fn native_and_governed_twins_share_one_input_schema() {
    let governed: std::collections::HashMap<String, &Value> = GOVERNED_SPECS
        .iter()
        .filter_map(|s| Some((governed_stdio_name(s["name"].as_str()?), s)))
        .collect();
    let mut drift = Vec::new();
    let mut still_deferred = Vec::new();
    let mut pairs = 0;
    for tool in TOOL_CATALOG["tools"].as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        let Some(spec) = governed.get(name) else {
            continue;
        };
        pairs += 1;
        let native = comparable_schema(&tool["inputSchema"]);
        let outward = comparable_schema(&spec["inputSchema"]);
        if DEFERRED_SCHEMA_DRIFT.contains(&name) {
            if native != outward {
                still_deferred.push(name);
            }
            continue;
        }
        if native != outward {
            drift.push(format!(
                "{name}:\n  native:   {native}\n  governed: {outward}"
            ));
        }
    }
    assert!(
        pairs > 0,
        "no native/governed twins found — name mapping changed?"
    );
    assert!(
        drift.is_empty(),
        "{} of {pairs} twins drifted (fix the native schema; the outward catalog is the source of truth):\n{}",
        drift.len(),
        drift.join("\n")
    );
    let reconciled: Vec<&&str> = DEFERRED_SCHEMA_DRIFT
        .iter()
        .filter(|n| !still_deferred.contains(*n))
        .collect();
    assert!(
        reconciled.is_empty(),
        "no longer drifting (or no longer twins) — remove from DEFERRED_SCHEMA_DRIFT: {reconciled:?}"
    );
}

/// The outward spelling `number` reaches the same PR as the native
/// `pr_number` (both catalogs now advertise `number` + the alias).
#[test]
fn pr_number_accepts_the_outward_name_and_its_alias() {
    assert_eq!(arg_pr_number(&json!({"number": 7})), Ok(7));
    assert_eq!(arg_pr_number(&json!({"pr_number": "8"})), Ok(8));
    assert_eq!(arg_pr_number(&json!({"number": 9, "pr_number": 1})), Ok(9));
    assert!(arg_pr_number(&json!({})).unwrap_err().contains("`number`"));
}

/// The `tools/list` size budget: everything a default session can be
/// shown, UI tools hidden (no grant). Fails when the catalog silently
/// grows past the budget — raise it deliberately, with a reason.
#[test]
fn tools_list_catalog_stays_within_its_byte_budget() {
    let info = EnabledInfo {
        names: all_governed_names(),
        ui_granted: false,
        filter: None,
    };
    let listing = |source: Option<&str>, info: &EnabledInfo| -> usize {
        let mut cat = tool_catalog_for_source(source);
        let arr = cat["tools"].as_array_mut().unwrap();
        arr.extend(governed_tools_for_session(info));
        scope_listing_for_source(arr, source);
        serde_json::to_vec(&cat).unwrap().len()
    };
    let ungranted = listing(None, &info);
    let granted_info = EnabledInfo {
        ui_granted: true,
        ..info.clone()
    };
    let granted = listing(None, &granted_info);
    let narrow = listing(Some("review"), &info);
    eprintln!("tools/list bytes: ungranted={ungranted} granted={granted} review={narrow}");
    assert!(
        ungranted < granted * 3 / 4,
        "hiding the UI tools must cut the catalog by >25% ({ungranted} vs {granted})"
    );
    assert!(
        ungranted <= TOOLS_LIST_BYTE_BUDGET,
        "tools/list grew to {ungranted} bytes (budget {TOOLS_LIST_BYTE_BUDGET})"
    );
    assert!(
        narrow <= NARROW_TOOLS_LIST_BYTE_BUDGET,
        "a review session's tools/list grew to {narrow} bytes \
         (budget {NARROW_TOOLS_LIST_BYTE_BUDGET})"
    );
    // Scoping is listing-only and really trims: no infra console left.
    assert!(narrow < ungranted);
}

/// A narrow (review) session's listing hides the infra/canvas/browser
/// families, but the default and assistant listings keep them.
#[test]
fn narrow_sources_drop_the_infra_consoles_from_the_listing() {
    let names = |source: Option<&str>| -> Vec<String> {
        let mut cat = tool_catalog_for_source(source);
        let arr = cat["tools"].as_array_mut().unwrap();
        scope_listing_for_source(arr, source);
        arr.iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    let review = names(Some("review"));
    assert!(!review
        .iter()
        .any(|n| n.starts_with("k8s_") || n.starts_with("aws_")));
    assert!(review.iter().any(|n| n == "otto_get_pr"), "{review:?}");
    let default = names(None);
    assert!(default.iter().any(|n| n.starts_with("k8s_")));
    assert!(names(Some("workflow"))
        .iter()
        .any(|n| n.starts_with("aws_")));
}

/// Every control-plane catalog entry reaches a session: generated as a native
/// twin, served natively under an aliased name, proxied under its stdio name,
/// or listed in [`INTENTIONALLY_ABSENT`] — never silently dropped.
#[test]
fn every_catalog_entry_is_generated_or_intentionally_absent() {
    let native: std::collections::HashSet<String> = native_tool_names().into_iter().collect();
    let mut missing = Vec::new();
    for spec in GOVERNED_SPECS.iter() {
        let full = spec["name"].as_str().unwrap();
        let stdio = governed_stdio_name(full);
        let short = stdio.strip_prefix("otto_").unwrap();
        let twin = native.contains(&stdio);
        let aliased = GOVERNED_ALIASED_BY_NATIVE
            .iter()
            .any(|(g, n)| *g == short && native.contains(*n));
        let proxied = governed_tool_for_stdio_name(&stdio).as_deref() == Some(full);
        if !(twin || aliased || proxied || INTENTIONALLY_ABSENT.contains(&full)) {
            missing.push(full.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "catalog entries no session can see: {missing:?}"
    );
    // A stale INTENTIONALLY_ABSENT row names a real catalog entry.
    for full in INTENTIONALLY_ABSENT {
        assert!(GOVERNED_SPECS.iter().any(|s| s["name"] == *full), "{full}");
    }
}

/// The former drift pairs now advertise the catalog's arguments, with the
/// session-scoped `workspace_id` never required natively.
#[test]
fn former_drift_twins_advertise_the_catalog_arguments() {
    let cat = tool_catalog();
    let tool = |n: &str| -> Value {
        cat["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == n)
            .unwrap_or_else(|| panic!("{n} not listed"))["inputSchema"]
            .clone()
    };
    let exec = tool("otto_api_execute");
    assert!(exec["properties"]["environment_id"].is_object(), "{exec}");
    assert!(exec["properties"].get("name").is_none(), "{exec}");
    assert_eq!(exec["required"], json!(["request_id"]));
    assert_eq!(
        tool("otto_api_run_automation")["required"],
        json!(["automation_id"])
    );
    // Description references use this surface's spelling.
    assert!(
        tool("otto_api_run_automation")["properties"]["automation_id"]["description"]
            .as_str()
            .unwrap()
            .contains("otto_api_list")
    );
    let conns = tool("otto_list_connections");
    assert!(conns["properties"].get("kind").is_none(), "{conns}");
    assert_eq!(
        conns["properties"]["workspace_id"]["description"],
        json!(WS_DIR_DESC),
        "the native, session-aware scoped description is kept"
    );
    let upsert = tool("otto_api_upsert_request");
    assert!(upsert["properties"]["extras"].is_object(), "{upsert}");
    assert!(upsert.get("required").is_some_and(|r| !r
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x == "workspace_id")));
}

/// The pure generator: catalog schema wins, scoped arguments come from the
/// hand-written native schema and are never required.
#[test]
fn native_twin_schema_keeps_only_the_native_scoped_arguments() {
    let catalog = json!({"type":"object","required":["workspace_id","request_id"],"properties":{
        "workspace_id":{"type":"string"},"session_id":{"type":"string"},
        "request_id":{"type":"string","description":"see otto.api_list"}}});
    let native = json!({"type":"object","properties":{
        "workspace_id":{"type":"string","description":"defaults to this session's"},
        "stale":{"type":"string"}}});
    let out = listing::native_twin_schema(&catalog, &native);
    assert_eq!(out["required"], json!(["request_id"]));
    assert_eq!(
        out["properties"]["workspace_id"]["description"],
        "defaults to this session's"
    );
    assert!(out["properties"].get("session_id").is_none());
    assert!(out["properties"].get("stale").is_none());
    assert_eq!(
        out["properties"]["request_id"]["description"],
        "see otto_api_list"
    );
    // Nothing left required → no `required` key at all.
    let only_scoped = json!({"type":"object","required":["workspace_id"],"properties":{"workspace_id":{"type":"string"}}});
    assert!(listing::native_twin_schema(&only_scoped, &json!({}))
        .get("required")
        .is_none());
}

/// Control-plane parity for the issue-create family: enabling
/// `otto.create_issue` (approval-gated) and its discovery reads under MCP →
/// Otto server makes them visible to a session under the stdio names, with
/// the outward catalog's schema, and their calls route to the governed path.
#[test]
fn issue_create_tools_reach_sessions_through_the_governed_bridge() {
    let enabled: Vec<String> = [
        "otto.create_issue",
        "otto.list_issue_projects",
        "otto.list_issue_types",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let tools = governed_tools_for(&enabled);
    let mut names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "otto_create_issue",
            "otto_list_issue_projects",
            "otto_list_issue_types"
        ]
    );
    let create = tools
        .iter()
        .find(|t| t["name"] == "otto_create_issue")
        .unwrap();
    assert_eq!(
        create["inputSchema"]["required"],
        json!(["project_key", "issue_type", "summary"])
    );
    for stdio in [
        "otto_create_issue",
        "otto_list_issue_projects",
        "otto_list_issue_types",
    ] {
        assert_eq!(
            governed_tool_for_stdio_name(stdio).as_deref(),
            Some(stdio.replacen("otto_", "otto.", 1).as_str()),
            "{stdio} must proxy to its governed twin"
        );
    }
}

/// Control-plane parity for the database writer: enabling `otto.query_db`
/// under MCP → Otto server makes it visible to a session as `otto_query_db`
/// with the catalog's schema, proxied through the governed path (approval +
/// audit). The read-only twin stays served natively as `otto_db_query`.
#[test]
fn query_db_reaches_sessions_through_the_governed_bridge() {
    let enabled = vec![
        "otto.query_db".to_string(),
        "otto.query_db_readonly".to_string(),
    ];
    let tools = governed_tools_for(&enabled);
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(names, ["otto_query_db"], "the read-only twin is native");
    let spec = GOVERNED_SPECS
        .iter()
        .find(|s| s["name"] == "otto.query_db")
        .unwrap();
    assert_eq!(
        comparable_schema(&tools[0]["inputSchema"]),
        comparable_schema(&spec["inputSchema"])
    );
    assert_eq!(
        governed_tool_for_stdio_name("otto_query_db").as_deref(),
        Some("otto.query_db")
    );
    assert!(native_tool_names().iter().any(|n| n == "otto_db_query"));
    assert!(governed_tool_for_stdio_name("otto_db_query").is_none());
}
