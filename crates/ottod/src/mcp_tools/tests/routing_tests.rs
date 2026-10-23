//! architecture-1-05: one tool surface. The session catalog is generated from
//! the control-plane specs, and the routed twins call `route_for` with the
//! session-bound arguments filled first. The snapshots in `twin_tests.rs` pin
//! that nothing a session sees or sends changed; these pin the rules.

use super::*;

/// Parity: every control-plane tool reaches a default session (UI grant on)
/// under its stdio name or a native alias, and every twin is described by
/// its catalog entry — a tool added to `otto_tool_specs` cannot be missed,
/// and a twin cannot drift from its spec.
#[test]
fn every_control_plane_tool_is_in_the_session_listing_as_specified() {
    let info = EnabledInfo {
        names: GOVERNED_SPECS
            .iter()
            .filter_map(|s| s["name"].as_str().map(str::to_string))
            .collect(),
        ui_granted: true,
        filter: None,
    };
    let mut cat = tool_catalog_for_source(None);
    let tools = cat["tools"].as_array_mut().unwrap();
    tools.extend(governed_tools_for_session(&info));
    let listed: std::collections::HashMap<&str, &Value> = tools
        .iter()
        .filter_map(|t| Some((t["name"].as_str()?, t)))
        .collect();
    let mut missing = Vec::new();
    for spec in GOVERNED_SPECS.iter() {
        let full = spec["name"].as_str().unwrap();
        let stdio = governed_stdio_name(full);
        let short = stdio.strip_prefix("otto_").unwrap();
        // The assistant memory tools are native under the same bare names
        // and listed only to assistant sessions (by design).
        let aliased = GOVERNED_ALIASED_BY_NATIVE.iter().any(|(g, n)| {
            *g == short && (listed.contains_key(n) || assistant_segment(n).is_some())
        });
        if !listed.contains_key(stdio.as_str()) && !aliased {
            missing.push(full);
        }
    }
    assert!(
        missing.is_empty(),
        "control-plane tools no session sees: {missing:?}"
    );

    assert_eq!(NATIVE_TWINS.len(), 46);
    for twin in NATIVE_TWINS {
        let full = format!("otto.{}", twin.short);
        let spec = GOVERNED_SPECS
            .iter()
            .find(|s| s["name"] == full.as_str())
            .unwrap_or_else(|| panic!("{full}: a native twin with no catalog entry"));
        let tool = listed[governed_stdio_name(&full).as_str()];
        let described = listing::stdio_description(spec["description"].as_str().unwrap());
        assert!(
            tool["description"]
                .as_str()
                .unwrap()
                .starts_with(&described),
            "{full}: the session description is not the catalog's"
        );
        let generated = listing::native_twin_schema(&spec["inputSchema"], &tool["inputSchema"]);
        assert_eq!(
            tool["inputSchema"], generated,
            "{full}: schema differs from its spec"
        );
    }
}

/// Every routed twin is a native twin whose call `route_for` can build, and
/// none of them is left with a native `run_tool` arm (a routed name reaches
/// `twin_call` / `twin_read`, never "unknown tool").
#[test]
fn routed_twins_are_twins_and_route() {
    for (name, ..) in ROUTED_TWINS {
        let short = name.strip_prefix("otto_").unwrap();
        assert!(NATIVE_TWINS.iter().any(|t| t.short == short), "{name}");
        assert!(
            FEATURE_READ_TOOLS.contains(name) || is_twin_call(name),
            "{name} is not dispatched"
        );
    }
    assert_eq!(ROUTED_TWINS.len(), 30);
}

/// The pre-step: a workspace-scoped twin takes the call's workspace (and
/// errors as the native tools did without one), a room call binds the
/// calling session, a PR number accepts its `pr_number` alias, and a missing
/// argument reads in this surface's wording.
#[test]
fn routed_twins_bind_the_session_before_route_for() {
    let call = |name: &str, args: Value, ws: Option<&str>, session: Option<&str>| {
        twin_route(name, &args, ws, session).expect("routed")
    };
    assert_eq!(
        call(
            "otto_list_sessions",
            json!({"workspace_id": "other"}),
            Some("ws1"),
            None
        )
        .unwrap()
        .path,
        "/workspaces/ws1/sessions"
    );
    assert_eq!(
        call(
            "otto_vault_write",
            json!({"vault_id": 3, "path": "a.md", "content": ""}),
            None,
            None
        ),
        Err("no workspace context (OTTO_WORKSPACE_ID unset)".to_string())
    );
    let post = call(
        "otto_room_post",
        json!({"room_id": "r1", "text": "hi", "session_id": "x"}),
        None,
        Some("s1"),
    )
    .unwrap();
    assert_eq!(post.body, Some(json!({"text": "hi", "session_id": "s1"})));
    assert_eq!(post.reply, TwinReply::Wrap("message"));
    assert_eq!(
        call(
            "otto_get_pr",
            json!({"repo_id": "r", "pr_number": "8"}),
            None,
            None
        )
        .unwrap()
        .path,
        "/repos/r/prs/8"
    );
    assert_eq!(
        call("otto_get_pr", json!({"repo_id": "r"}), None, None),
        Err("missing required integer argument `number` (or `pr_number`)".to_string())
    );
    assert_eq!(
        call("otto_send_message", json!({"session_id": "s"}), None, None),
        Err("missing required string argument `text`".to_string())
    );
    // A mistyped string argument is refused, not dropped.
    assert_eq!(
        call(
            "otto_vault_write_file",
            json!({"vault_id": 3, "path": "a", "content": "", "if_hash": 7}),
            Some("w"),
            None
        ),
        Err("argument `if_hash` must be a string".to_string())
    );
    // An absent optional is not sent as `null` (the native body never was).
    let comment = call(
        "otto_comment_pr",
        json!({"repo_id": "r", "number": 7, "body": "b"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(comment.body, Some(json!({"body": "b"})));
    assert!(twin_route("otto_list_repos", &json!({}), None, None).is_none());
}

/// The directory twins stay native, and each native kind is the one the
/// outward executor's directory uses for the same tool.
#[test]
fn native_directory_kinds_match_the_outward_directory() {
    for (tool, kind) in NATIVE_DIRECTORY_TOOLS {
        let short = tool.strip_prefix("otto_").unwrap();
        let outward = otto_mcp::outward::refs::DIRECTORY_TOOLS
            .iter()
            .find(|(t, _)| *t == short)
            .map(|(_, k)| *k);
        assert_eq!(outward, Some(*kind), "{tool}");
    }
}
