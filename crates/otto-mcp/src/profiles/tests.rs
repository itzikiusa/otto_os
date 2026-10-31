use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use super::*;
use crate::outward::{jsonrpc::post_response, otto_tool_specs};

fn def(id: &str, groups: &[&str]) -> ProfileDef {
    ProfileDef {
        id: id.into(),
        name: id.to_uppercase(),
        all_tools: false,
        groups: groups.iter().map(|g| g.to_string()).collect(),
        include_tools: Vec::new(),
        exclude_tools: Vec::new(),
    }
}

fn full() -> ProfileDef {
    ProfileDef {
        all_tools: true,
        name: "Full".into(),
        ..def("full", &[])
    }
}

/// Every outward (`otto.*`) tool — the UI-control catalog included — sits in
/// exactly ONE group, and no group lists a name twice. A tool added to the
/// catalog without a group fails here (ottod's `native_catalog_tools_have_
/// exactly_one_group` covers the session-only native tools).
#[test]
fn every_catalog_tool_has_exactly_one_group() {
    let mut missing = Vec::new();
    for spec in otto_tool_specs() {
        let name = spec["name"].as_str().unwrap();
        let key = canonical(name);
        let hits: Vec<&str> = GROUPS
            .iter()
            .filter(|g| g.contains(key))
            .map(|g| g.id)
            .collect();
        if hits.len() != 1 {
            missing.push(format!("{name} → {hits:?}"));
        }
    }
    assert!(
        missing.is_empty(),
        "tools without exactly one MCP profile group (add them to otto_mcp::profiles::GROUPS):\n{}",
        missing.join("\n")
    );
    let mut ids: Vec<&str> = GROUPS.iter().map(|g| g.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), GROUPS.len(), "duplicate group id");
    for g in GROUPS {
        for e in g.exact {
            let owners: Vec<&str> = GROUPS
                .iter()
                .filter(|o| o.contains(e))
                .map(|o| o.id)
                .collect();
            assert_eq!(owners, [g.id], "{e} is claimed by {owners:?}");
        }
    }
}

#[test]
fn canonical_names_unify_the_three_spellings() {
    for n in ["otto.create_pr", "otto_create_pr", "create_pr"] {
        assert_eq!(canonical(n), "create_pr");
        assert_eq!(group_of(n).unwrap().id, "git");
    }
    assert_eq!(group_of("k8s_logs").unwrap().id, "kubernetes");
    assert_eq!(group_of("otto_db_query").unwrap().id, "database");
    assert_eq!(group_of("otto_ui_request_control").unwrap().id, "ui");
    assert_eq!(group_of("mcp__github__list_issues").unwrap().id, "external");
    assert!(group_of("no_such_tool").is_none());
}

#[test]
fn a_profile_narrows_to_its_groups_includes_and_excludes() {
    let db = def("database", &["database"]);
    for t in [
        "otto.query_db_readonly",
        "otto_db_query",
        "otto_db_schema",
        "list_connections",
    ] {
        assert!(db.allows(t), "{t}");
    }
    for t in [
        "otto.create_pr",
        "k8s_logs",
        "otto_search_issues",
        "mcp__x__y",
    ] {
        assert!(!db.allows(t), "{t}");
    }
    let mut tuned = db.clone();
    tuned.include_tools = vec!["list_workspaces".into()];
    tuned.exclude_tools = vec!["db_query".into()];
    assert!(tuned.allows("otto_list_workspaces"));
    assert!(!tuned.allows("otto_db_query"), "an exclude beats the group");
    assert!(tuned.allows("otto_db_schema"));

    let mut most = full();
    most.exclude_tools = vec!["merge_pr".into()];
    assert!(!most.is_full());
    assert!(most.allows("otto.create_pr") && !most.allows("otto.merge_pr"));
}

#[test]
fn the_filter_is_the_union_and_full_or_empty_hides_nothing() {
    let f = ToolFilter::new(vec![
        def("database", &["database"]),
        def("issues", &["issues", "confluence"]),
    ]);
    assert_eq!(f.label, "DATABASE + ISSUES");
    assert_eq!(f.ids, ["database", "issues"]);
    assert!(f.allows("otto_db_query") && f.allows("otto.get_confluence_page"));
    assert!(!f.allows("otto.create_pr"));

    let mut listed = vec![
        json!({"name": "otto_db_query"}),
        json!({"name": "otto_create_pr"}),
        json!({"name": "otto_search_issues"}),
        json!({"name": "k8s_logs"}),
    ];
    f.retain_listed(&mut listed);
    let names: Vec<&str> = listed.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["otto_db_query", "otto_search_issues"]);

    let with_full = ToolFilter::new(vec![def("database", &["database"]), full()]);
    assert!(with_full.is_unrestricted() && with_full.allows("anything_at_all"));
    assert!(ToolFilter::new(Vec::new()).allows("otto.merge_pr"));

    let msg = f.denial("otto_create_pr");
    assert!(
        msg.contains("otto_create_pr") && msg.contains("\"DATABASE + ISSUES\""),
        "{msg}"
    );
}

#[test]
fn normalize_rejects_unknown_groups_and_tools() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let (g, i, e) = normalize_fields(
        &s(&["git", "git"]),
        &s(&["otto.list_workspaces"]),
        &s(&["otto_merge_pr"]),
    )
    .unwrap();
    assert_eq!(
        (g, i, e),
        (s(&["git"]), s(&["list_workspaces"]), s(&["merge_pr"]))
    );
    assert!(normalize_fields(&s(&["nope"]), &[], &[]).is_err());
    assert!(normalize_fields(&[], &s(&["typo_tool"]), &[]).is_err());
    assert_eq!(parse_ids(" database, ,issues "), ["database", "issues"]);

    // Legacy "design" group in persisted profiles normalizes and grants "product".
    let (legacy_g, _, _) = normalize_fields(&s(&["design", "product"]), &[], &[]).unwrap();
    assert_eq!(legacy_g, s(&["product"]));
    let legacy_def = def("legacy", &["design"]);
    assert!(legacy_def.allows("design_list"));
    assert!(legacy_def.allows("canvas_get_scene"));
    assert!(legacy_def.allows("otto.list_product_stories"));
}

// ---- the HTTP transport hook --------------------------------------------

/// A stand-in for the governed choke point: lists three tools, counts calls,
/// and DENIES `otto.merge_pr` itself (as RBAC / a token scope would).
#[derive(Default)]
struct Governed(AtomicUsize);

#[async_trait]
impl OutwardTools for Governed {
    async fn list(&self, _: &AuthContext) -> Vec<Value> {
        ["otto.query_db_readonly", "otto.create_pr", "otto.merge_pr"]
            .iter()
            .map(|n| json!({"name": n, "description": "", "inputSchema": {}}))
            .collect()
    }
    async fn call(&self, _: &AuthContext, tool: &str, _: &Value) -> Result<Value> {
        self.0.fetch_add(1, Ordering::SeqCst);
        if tool == "otto.merge_pr" {
            return Ok(json!({"decision": "denied", "reason": "not allowed for this user"}));
        }
        Ok(json!({"decision": "allowed", "executed": true}))
    }
}

fn auth() -> AuthContext {
    let user = otto_core::domain::User {
        id: "u".into(),
        username: "u".into(),
        display_name: "U".into(),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    };
    AuthContext::automation(&user, "test")
}

async fn rpc<T: OutwardTools>(tools: &T, body: Value) -> Value {
    let resp = post_response(tools, &auth(), &body, "t").await;
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn call(name: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": {}}})
}

#[tokio::test]
async fn http_lists_and_dispatches_only_the_profile_tools() {
    let inner = Governed::default();
    let tools = ProfiledTools {
        inner: &inner,
        filter: Some(ToolFilter::new(vec![def("database", &["database"])])),
    };
    let v = rpc(
        &tools,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    let names: Vec<&str> = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["otto.query_db_readonly"]);

    let v = rpc(&tools, call("otto.create_pr")).await;
    assert_eq!(v["result"]["isError"], true);
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("otto.create_pr")
            && text.contains("MCP profile")
            && text.contains("DATABASE"),
        "{text}"
    );
    assert_eq!(
        inner.0.load(Ordering::SeqCst),
        0,
        "a refused call never reaches the governed path"
    );

    let v = rpc(&tools, call("otto.query_db_readonly")).await;
    assert_eq!(v["result"]["isError"], false);
    assert_eq!(inner.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn governance_still_decides_inside_a_profile() {
    // Git & PRs includes merge_pr, but the governed path still denies it: a
    // profile narrows, it never grants.
    let inner = Governed::default();
    let tools = ProfiledTools {
        inner: &inner,
        filter: Some(ToolFilter::new(vec![def("git", &["git"])])),
    };
    let v = rpc(&tools, call("otto.merge_pr")).await;
    assert_eq!(v["result"]["isError"], true);
    assert!(v["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("not allowed for this user"));
    assert_eq!(inner.0.load(Ordering::SeqCst), 1);

    // No profile asked for (the legacy all-tools connection): untouched.
    let plain = ProfiledTools {
        inner: &inner,
        filter: None,
    };
    let v = rpc(
        &plain,
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
    )
    .await;
    assert_eq!(v["result"]["tools"].as_array().unwrap().len(), 3);
}

// ---- persistence ---------------------------------------------------------

async fn pool() -> otto_state::DbPool {
    let p = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&p)
        .await
        .unwrap();
    p.into()
}

#[tokio::test]
async fn presets_match_the_migration_seed() {
    let pool = pool().await;
    let rows = otto_state::McpProfileRepo::new(pool.clone())
        .list()
        .await
        .unwrap();
    for preset in PRESETS {
        let row = rows.iter().find(|r| r.id == preset.id).expect(preset.id);
        assert_eq!(row.name, preset.name);
        assert_eq!(row.all_tools, preset.all_tools);
        assert_eq!(row.groups, preset.groups);
        assert!(row.builtin);
        for g in &row.groups {
            assert!(
                group(g).is_some(),
                "preset {} names unknown group {g}",
                preset.id
            );
        }
    }
    assert_eq!(rows.len(), PRESETS.len());
}

#[tokio::test]
async fn resolve_builds_the_union_filter_and_defaults() {
    let pool = pool().await;
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let f = resolve(&pool, &ids(&["database", "issues"])).await.unwrap();
    assert_eq!(f.label, "Database + Issues");
    assert!(f.allows("otto_db_query") && f.allows("otto.comment_issue") && !f.allows("k8s_logs"));
    // A session with no stored profile runs under the default (Full).
    let d = resolve(&pool, &ids(&["default"])).await.unwrap();
    assert_eq!(d.ids, ["full"]);
    assert!(d.is_unrestricted());
    assert_eq!(resolve_param(&pool, None).await.unwrap(), None);
}
