use super::*;
use serde_json::json;

fn graph(v: Value) -> WorkflowGraph {
    serde_json::from_value(v).unwrap()
}

fn classes(g: &WorkflowGraph) -> Vec<(String, IssueClass)> {
    validate(g)
        .into_iter()
        .map(|i| (i.field, i.class))
        .collect()
}

#[test]
fn rejects_missing_nodes_cycles_parameters_and_bad_expressions() {
    let graph = graph(
        json!({"nodes":[{"id":"a","kind":"http_request"},{"id":"b","kind":"condition","params":{"expr":"input.x =="}}],"edges":[{"id":"ab","source":"a","target":"b"},{"id":"ba","source":"b","target":"a"},{"id":"missing","source":"a","target":"no"}]}),
    );
    let issues = validate(&graph);
    for expected in ["url", "Invalid expr", "cycle", "missing"] {
        assert!(
            issues.iter().any(|i| i.message.contains(expected)),
            "missing finding {expected}: {issues:?}"
        );
    }
    let class = |needle: &str| {
        issues
            .iter()
            .find(|i| i.message.contains(needle))
            .unwrap()
            .class
    };
    assert_eq!(class("cycle"), IssueClass::Save);
    assert_eq!(class("missing"), IssueClass::Save);
    assert_eq!(class("url"), IssueClass::Run);
    assert_eq!(class("Invalid expr"), IssueClass::Run);
}

#[test]
fn dynamic_values_and_bounded_loops_are_valid() {
    let g = graph(
        json!({"nodes":[{"id":"loop","kind":"loop","params":{"until":"last.passed == true","steps":[{"kind":"api_run","params":{"url":"{{endpoint}}"}}]}}],"edges":[]}),
    );
    assert!(validate(&g).is_empty());
}

/// DR-028: a blank new workflow and a half-configured step save; running
/// them reports what is missing.
#[test]
fn a_draft_saves_but_does_not_run() {
    let empty = WorkflowGraph::default();
    assert!(ensure_savable(&empty).is_ok());
    assert!(ensure_valid(&empty)
        .unwrap_err()
        .to_string()
        .contains("Add at least one step"));
    let half = graph(json!({"nodes":[
        {"id":"s","kind":"manual_trigger"},
        {"id":"gates","kind":"script","name":"Go PR gates"},
        {"id":"q","kind":"db_query","params":{"connection_id":""}},
        {"id":"chk","kind":"check","name":"Check criteria"},
        {"id":"l","kind":"loop"}],"edges":[]}));
    assert!(ensure_savable(&half).is_ok(), "{:?}", validate(&half));
    let err = ensure_valid(&half).unwrap_err().to_string();
    for want in [
        "set command",
        "set timeout_s",
        "set connection_id",
        "add at least one criterion",
        "inner step",
    ] {
        assert!(err.contains(want), "{want}: {err}");
    }
}

#[test]
fn unknown_values_of_the_new_enums_are_invalid() {
    let cases = [
        ("git_checkout", "mode", "pr", "fork"),
        ("script", "foreach", "repos", "each"),
        ("script", "cwd", "context", "home"),
        ("post_review_comments", "info", "skip", "all"),
        ("manual_trigger", "chat_updates", "per_step", "quiet"),
        ("manual_trigger", "lane", "long", "fast"),
        ("review_run", "scope_assessment", "configured", "open"),
    ];
    for (kind, key, good, bad) in cases {
        let g = |v: &str| {
            graph(
                json!({"nodes":[{"id":"n","kind":kind,"params":{key: v, "command": "true", "timeout_s": 60}}],"edges":[]}),
            )
        };
        assert!(
            !classes(&g(good)).iter().any(|(f, _)| f == key),
            "{kind}.{key}={good}"
        );
        assert!(
            classes(&g(bad)).contains(&(key.to_string(), IssueClass::Save)),
            "{kind}.{key}={bad}"
        );
        assert!(ensure_savable(&g(bad)).is_err());
    }
    // An unknown kind is invalid too.
    let g = graph(json!({"nodes":[{"id":"n","kind":"teleport"}],"edges":[]}));
    assert!(ensure_savable(&g).is_err());
}

/// DR-002: names from a definition are bare file names in the context dir.
#[test]
fn context_file_names_that_leave_the_run_dir_are_invalid() {
    for bad in [
        "../../otto.db",
        "/Users/x/.ssh/id_rsa",
        "sub/x.md",
        "a\\b",
        "",
    ] {
        let g = graph(json!({"nodes":[
            {"id":"s","kind":"script","params":{"command":"true","timeout_s":60,"output_file":bad}},
            {"id":"r","kind":"review_run"},
            {"id":"p","kind":"post_review_comments","params":{"summary_files":["go-standards-evals.md", bad]}}],
            "edges":[{"id":"e1","source":"r","target":"p"}]}));
        let c = classes(&g);
        assert!(
            c.contains(&("output_file".into(), IssueClass::Save)),
            "{bad:?}: {c:?}"
        );
        assert!(
            c.contains(&("summary_files".into(), IssueClass::Save)),
            "{bad:?}: {c:?}"
        );
    }
    let g = graph(
        json!({"nodes":[{"id":"s","kind":"script","params":{"command":"true","timeout_s":60,"output_file":"go-standards-evals.md"}}],"edges":[]}),
    );
    assert!(validate(&g).is_empty());
}

#[test]
fn timeouts_have_ranges() {
    let g = |kind: &str, t: Value| {
        graph(
            json!({"nodes":[{"id":"n","kind":kind,"params":{"prompt":"p","command":"c","timeout_s":t}}],"edges":[]}),
        )
    };
    assert!(validate(&g("agent_prompt", json!(600))).is_empty());
    assert!(
        classes(&g("agent_prompt", json!(30))).contains(&("timeout_s".into(), IssueClass::Save))
    );
    assert!(classes(&g("agent_prompt", json!(40_000)))
        .contains(&("timeout_s".into(), IssueClass::Save)));
    assert!(classes(&g("script", json!(5))).contains(&("timeout_s".into(), IssueClass::Save)));
    assert!(validate(&g("script", json!(900))).is_empty());
    assert!(classes(&g("script", json!("ten"))).contains(&("timeout_s".into(), IssueClass::Save)));
}

fn chain(extra: Value) -> WorkflowGraph {
    let mut nodes = vec![
        json!({"id":"s","kind":"manual_trigger"}),
        json!({"id":"a","kind":"agent_prompt","params":{"prompt":"p","provider":"claude"}}),
        json!({"id":"x","kind":"agent_prompt","params":{"prompt":"p","provider":"codex"}}),
    ];
    nodes.extend(extra.as_array().unwrap().iter().cloned());
    let mut edges = vec![
        json!({"id":"sa","source":"s","target":"a"}),
        json!({"id":"ax","source":"a","target":"x"}),
    ];
    for n in extra.as_array().unwrap() {
        let id = n["id"].as_str().unwrap();
        edges.push(json!({"id": format!("x{id}"), "source": "x", "target": id}));
    }
    graph(json!({"nodes": nodes, "edges": edges}))
}

#[test]
fn continue_from_names_an_earlier_agent_step_on_the_same_provider() {
    let ok = chain(
        json!([{"id":"b","kind":"agent_prompt","params":{"prompt":"p","provider":"claude","continue_from":"a"}}]),
    );
    assert!(validate(&ok).is_empty(), "{:?}", validate(&ok));
    let other = chain(
        json!([{"id":"b","kind":"agent_prompt","params":{"prompt":"p","provider":"claude","continue_from":"x"}}]),
    );
    assert!(validate(&other)[0].message.contains("another provider"));
    let missing = chain(
        json!([{"id":"b","kind":"agent_prompt","params":{"prompt":"p","continue_from":"ghost"}}]),
    );
    assert!(validate(&missing)[0].message.contains("does not exist"));
    let mut later = chain(
        json!([{"id":"b","kind":"agent_prompt","params":{"prompt":"p","provider":"claude"}}]),
    );
    later.nodes[1].params["continue_from"] = json!("b");
    assert!(
        validate(&later)[0].message.contains("earlier Agent step"),
        "a descendant is not an ancestor"
    );
    let two = chain(json!([
        {"id":"b","kind":"agent_prompt","params":{"prompt":"p","provider":"claude","continue_from":"a"}},
        {"id":"c","kind":"agent_prompt","params":{"prompt":"p","provider":"claude","continue_from":"a"}}]));
    let issues = validate(&two);
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(issues[0].message.contains("can run at the same time"));
    assert_eq!(issues[0].class, IssueClass::Save);
    let not_agent = chain(json!([{"id":"b","kind":"log","params":{"continue_from":"a"}}]));
    assert!(validate(&not_agent)[0]
        .message
        .contains("only for Agent steps"));
}

#[test]
fn posting_needs_a_review_before_it() {
    let g = |edges: Value, params: Value| {
        graph(json!({"nodes":[
        {"id":"r","kind":"review_run"},{"id":"g","kind":"agent_prompt","params":{"prompt":"p"}},
        {"id":"p","kind":"post_review_comments","params":params}],"edges":edges}))
    };
    let linked =
        json!([{"id":"rg","source":"r","target":"g"},{"id":"gp","source":"g","target":"p"}]);
    assert!(validate(&g(linked.clone(), json!({"summary_intro_from":"g"}))).is_empty());
    assert!(validate(&g(linked, json!({"review_from":"g"})))[0]
        .message
        .contains("Review Run"));
    let alone = validate(&g(json!([]), json!({"summary_intro_from":"g"})));
    assert!(
        alone.iter().all(|i| i.class == IssueClass::Run),
        "{alone:?}"
    );
    assert_eq!(alone.len(), 2);
}

/// DR-013: a run.repos condition beside a repos writer depends on timing —
/// a warning, never a block.
#[test]
fn a_repos_condition_beside_a_repos_writer_warns() {
    let g = |checkout_first: bool| {
        let mut edges = vec![
            json!({"id":"sc","source":"s","target":"c"}),
            json!({"id":"sv","source":"s","target":"v"}),
            json!({"id":"vx","source":"v","target":"x","condition":"len(run.repos) > 1"}),
        ];
        if checkout_first {
            edges.push(json!({"id":"cv","source":"c","target":"v"}));
        }
        graph(json!({"nodes":[
            {"id":"s","kind":"manual_trigger"},{"id":"c","kind":"git_checkout","name":"Checkout"},
            {"id":"v","kind":"agent_prompt","params":{"prompt":"p"}},{"id":"x","kind":"agent_prompt","params":{"prompt":"p"}}],
            "edges": edges}))
    };
    let side_by_side = validate(&g(false));
    assert_eq!(side_by_side.len(), 1, "{side_by_side:?}");
    assert_eq!(side_by_side[0].class, IssueClass::Warning);
    assert_eq!(side_by_side[0].edge_id.as_deref(), Some("vx"));
    assert!(ensure_valid(&g(false)).is_ok(), "a warning never blocks");
    assert!(validate(&g(true)).is_empty(), "checkout before the reader");
    // An agent that says it rewrites repos.json is a writer too.
    let mut agent = g(false);
    agent.nodes[1] = serde_json::from_value(json!({"id":"c","kind":"agent_prompt","params":{"prompt":"Check out the PR and update repos.json"}})).unwrap();
    assert_eq!(validate(&agent).len(), 1);
}

#[test]
fn check_criteria() {
    let g = |criteria: Value| {
        graph(
            json!({"nodes":[{"id":"k","kind":"check","name":"Check","params":{"criteria":criteria}}],"edges":[]}),
        )
    };
    let ok = g(json!([
        {"id":"tests","text":"Tests pass","verify":"command","command":"go test ./..."},
        {"id":"rule","text":"Reviewed","verify":"expr","expr":"steps.review.passed == true"},
        {"id":"human","text":"Looks right","verify":"human"}]));
    assert!(validate(&ok).is_empty(), "{:?}", validate(&ok));
    let bad = g(json!([
        {"id":"a","text":"x","verify":"command"},
        {"id":"a","text":"","verify":"agent"},
        {"id":"b","text":"y","verify":"psychic"},
        {"id":"c","text":"z","verify":"expr","expr":"a =="}]));
    let issues = validate(&bad);
    let has = |needle: &str, class| {
        issues
            .iter()
            .any(|i| i.message.contains(needle) && i.class == class)
    };
    assert!(has("needs its command", IssueClass::Run), "{issues:?}");
    assert!(has("must be unique", IssueClass::Save));
    assert!(has("needs its text", IssueClass::Run));
    assert!(has("unknown check 'psychic'", IssueClass::Save));
    assert!(has("invalid rule", IssueClass::Save));
}

/// The five exported graphs stay valid with no warning.
#[test]
fn the_clones_have_no_issue() {
    for text in [
        include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/a.json"),
        include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/b.json"),
        include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/c.json"),
        include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/d.json"),
        include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/e.json"),
    ] {
        let def: Value = serde_json::from_str(text).unwrap();
        let g: WorkflowGraph = serde_json::from_value(def["graph"].clone()).unwrap();
        assert!(validate(&g).is_empty(), "{:?}", validate(&g));
    }
}

#[test]
fn engine_settings_out_of_range_are_refused() {
    for (k, v) in [
        ("wf_step_runner", json!("threads")),
        ("wf_step_runner", json!("pty")),
        ("wf_max_parallel_steps", json!(1)),
        ("wf_max_parallel_steps", json!(8)),
        ("workflow_thread_retention_success_hours", json!(24)),
        ("wf_max_parallel_steps", Value::Null),
        ("theme", json!("anything")),
    ] {
        assert!(engine_setting(k, &v).is_ok(), "{k}={v}");
    }
    for (k, v) in [
        ("wf_step_runner", json!("tmux")),
        ("wf_max_parallel_steps", json!(0)),
        ("wf_max_parallel_steps", json!(9)),
        ("wf_max_parallel_steps", json!("4")),
        ("workflow_thread_retention_failed_hours", json!(-1)),
    ] {
        assert!(engine_setting(k, &v).is_err(), "{k}={v}");
    }
}
