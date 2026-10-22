//! Catalog-shape tests: native `otto_x` / governed `otto.x` twins advertise
//! one `inputSchema` (architecture-1-05, interim), the PR-number alias, and
//! the `tools/list` byte budget + narrow-source scoping. Split out of
//! `mcp_tools.rs`'s test module to keep that file under its LOC ratchet.

use super::*;

/// Twins whose native schema is still deliberately different and not yet
/// reconciled — the deferred remainder of architecture-1-05 (generate the
/// native listing from `otto_mcp::outward::catalog`). Each needs executor
/// work, not a schema edit: `otto_list_connections` has a native-only
/// `kind` filter, and the API-client tools resolve saved requests /
/// automations / environments by a native-only `name` where the governed
/// executor takes an id-or-name `request_id`/`environment_id`. This list
/// may only SHRINK: the test fails when a listed twin stops drifting.
const DEFERRED_SCHEMA_DRIFT: &[&str] = &[
    "otto_list_connections",
    "otto_api_list",
    "otto_api_get_request",
    "otto_api_history",
    "otto_api_execute",
    "otto_api_upsert_request",
    "otto_api_run_automation",
];

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

/// architecture-1-05 (interim, until the native listing is generated
/// from the outward catalog): a capability served natively as `otto_x`
/// AND governed as `otto.x` must advertise the SAME `inputSchema` (modulo
/// [`comparable_schema`]), so a session and the Control Plane describe one
/// tool one way. Native wins by name in `tools/list`, so a drifted native
/// schema is what agents see while the Control Plane shows (and
/// validates) the governed one.
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
