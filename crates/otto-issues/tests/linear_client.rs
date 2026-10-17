//! The Linear client over the wire, against a loopback GraphQL stub.
//!
//! `linear.rs`'s own tests pin the pure mappers; these pin what only shows up
//! with a server on the other end: how GraphQL/HTTP failures surface, the
//! cursor walk behind offset "Load more", the identifier-vs-full-text search
//! routing, comment paging, and the exact mutation inputs (unassign as `null`,
//! labels standing in for issue types).
//!
//! The cursor-walk memo is process-global and keyed by credential + query, so
//! every test uses its own token to stay independent under parallel runs.

use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use otto_core::Error;
use otto_issues::LinearClient;
use serde_json::{json, Value};

/// One request the stub saw: the `Authorization` header and the GraphQL body.
#[derive(Clone, Debug)]
struct Seen {
    auth: String,
    query: String,
    vars: Value,
}

type Reply = (u16, String);
type Handler = Arc<dyn Fn(&str, &Value) -> Reply + Send + Sync>;

struct Stub {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Stub {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
    /// Requests whose document contains `needle`.
    fn calls(&self, needle: &str) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|s| s.query.contains(needle))
            .collect()
    }
}

/// Serve `handler` as a GraphQL endpoint on an ephemeral loopback port.
async fn stub(handler: impl Fn(&str, &Value) -> Reply + Send + Sync + 'static) -> Stub {
    let handler: Handler = Arc::new(handler);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let app = Router::new().route(
        "/graphql",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let handler = Arc::clone(&handler);
            let log = Arc::clone(&log);
            async move {
                let query = body["query"].as_str().unwrap_or("").to_string();
                let vars = body["variables"].clone();
                log.lock().unwrap().push(Seen {
                    auth: headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string(),
                    query: query.clone(),
                    vars: vars.clone(),
                });
                let (status, text) = handler(&query, &vars);
                (StatusCode::from_u16(status).unwrap(), text)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Stub {
        url: format!("http://{addr}/graphql"),
        seen,
    }
}

fn ok(data: Value) -> Reply {
    (200, json!({ "data": data }).to_string())
}

fn issue_node(n: usize) -> Value {
    json!({
        "id": format!("uuid-{n}"), "identifier": format!("ENG-{n}"), "title": format!("issue {n}"),
        "url": format!("https://linear.app/acme/issue/ENG-{n}"),
        "priority": 0, "priorityLabel": "No priority", "updatedAt": "2026-10-01T00:00:00.000Z",
        "state": {"name": "Todo", "type": "unstarted"}, "team": {"key": "ENG", "name": "Engineering"},
        "assignee": null, "parent": null
    })
}

/// A connection over `total` issues (`total = None`: endless), paged by
/// `first` and an `after` cursor of the form `c<offset>`.
fn page(vars: &Value, total: Option<usize>) -> Value {
    let start = vars["after"]
        .as_str()
        .map(|c| c.trim_start_matches('c').parse::<usize>().unwrap())
        .unwrap_or(0);
    let first = vars["first"].as_u64().unwrap() as usize;
    let end = total.map_or(start + first, |t| (start + first).min(t));
    let nodes: Vec<Value> = (start..end).map(issue_node).collect();
    let more = total.is_none_or(|t| end < t);
    json!({ "nodes": nodes, "pageInfo": { "hasNextPage": more, "endCursor": format!("c{end}") } })
}

fn keys(v: &[otto_core::domain::IssueSummary]) -> Vec<String> {
    v.iter().map(|i| i.key.clone()).collect()
}

fn range(a: usize, b: usize) -> Vec<String> {
    (a..b).map(|n| format!("ENG-{n}")).collect()
}

// ---------------------------------------------------------------------------
// Transport + error mapping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_keys_travel_bare_and_oauth_tokens_as_bearer() {
    let s = stub(|_, _| ok(json!({"viewer": {"assignedIssues": {"nodes": []}}}))).await;
    LinearClient::with_endpoint(&s.url, "lin_api_key1")
        .my_work()
        .await
        .unwrap();
    LinearClient::with_endpoint(&s.url, "oauth-access")
        .my_work()
        .await
        .unwrap();
    let auth: Vec<String> = s.seen().into_iter().map(|x| x.auth).collect();
    assert_eq!(auth, ["lin_api_key1", "Bearer oauth-access"]);
}

#[tokio::test]
async fn graphql_and_http_failures_surface_as_typed_errors() {
    let s = stub(|_, vars| match vars["id"].as_str().unwrap_or("") {
        // Linear reports a missing entity as a GraphQL error with HTTP 200.
        "GONE-1" => (
            200,
            json!({"data": null, "errors": [{"message": "Entity not found: Issue",
                "extensions": {"code": "INVALID_INPUT"}}]})
            .to_string(),
        ),
        // A null `issue` without an error is "no such issue" too.
        "NULL-1" => ok(json!({"issue": null})),
        "AUTH-1" => (
            401,
            json!({"errors": [{"message": "Authentication required",
                "extensions": {"type": "authentication error"}}]})
            .to_string(),
        ),
        "BAD-1" => (
            400,
            json!({"errors": [{"message": "Variable \"$id\" got invalid value",
                "extensions": {"code": "GRAPHQL_VALIDATION_FAILED"}}]})
            .to_string(),
        ),
        // A proxy's HTML error page: not JSON, not 2xx.
        "DOWN-1" => (502, "<html>bad gateway</html>".into()),
        // 200 with neither data nor errors.
        "EMPTY-1" => (200, "{}".into()),
        other => panic!("unexpected id {other}"),
    })
    .await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_errors");
    assert!(matches!(
        c.get_issue("GONE-1").await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(c.get_issue("NULL-1").await, Err(Error::NotFound(m)) if m.contains("NULL-1")));
    assert!(
        matches!(c.get_issue("AUTH-1").await, Err(Error::Upstream(m)) if m.contains("API key")),
        "a 401 tells the user to check the key"
    );
    assert!(matches!(c.get_issue("BAD-1").await, Err(Error::Invalid(_))));
    assert!(
        matches!(c.get_issue("DOWN-1").await, Err(Error::Upstream(m)) if m.contains("502") && m.contains("bad gateway"))
    );
    assert!(
        matches!(c.get_issue("EMPTY-1").await, Err(Error::Upstream(m)) if m.contains("no data"))
    );
}

#[tokio::test]
async fn an_unreachable_endpoint_is_an_upstream_error() {
    // Bind then drop: the port is (almost certainly) closed.
    let addr = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let c = LinearClient::with_endpoint(&format!("http://{addr}/graphql"), "lin_api_down");
    assert!(matches!(
        c.get_issue("ENG-1").await,
        Err(Error::Upstream(_))
    ));
}

// ---------------------------------------------------------------------------
// Offset paging over Linear's cursor-only connections
// ---------------------------------------------------------------------------

#[tokio::test]
async fn load_more_reuses_remembered_cursors_instead_of_rewalking() {
    let s = stub(|q, vars| {
        assert!(q.contains("issues(filter:"), "{q}");
        ok(json!({ "issues": page(vars, Some(250)) }))
    })
    .await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_walk");
    let f = r#"{"team":{"key":{"eq":"ENG"}}}"#;

    // First page: one call, no cursor.
    assert_eq!(
        keys(&c.search_filter(f, 0, 25).await.unwrap()),
        range(0, 25)
    );
    // "Load more" at 25: exactly one call, straight from the cursor at 25.
    assert_eq!(
        keys(&c.search_filter(f, 25, 25).await.unwrap()),
        range(25, 50)
    );
    let seen = s.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].vars["after"], Value::Null);
    assert_eq!(seen[0].vars["first"], 25);
    assert_eq!(seen[1].vars["after"], "c25");
    // The filter itself goes through as the `filter` variable, untouched.
    assert_eq!(seen[1].vars["filter"], json!({"team":{"key":{"eq":"ENG"}}}));

    // An offset between remembered cursors walks only the gap: skip 50→60,
    // then read 60..85.
    assert_eq!(
        keys(&c.search_filter(f, 60, 25).await.unwrap()),
        range(60, 85)
    );
    let tail: Vec<(Value, Value)> = s.seen()[2..]
        .iter()
        .map(|x| (x.vars["after"].clone(), x.vars["first"].clone()))
        .collect();
    assert_eq!(tail, [(json!("c50"), json!(10)), (json!("c60"), json!(25))]);

    // The last, short page and an offset past the end.
    assert_eq!(
        keys(&c.search_filter(f, 225, 50).await.unwrap()),
        range(225, 250)
    );
    assert!(c.search_filter(f, 400, 25).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_new_search_from_zero_forgets_the_old_walk() {
    let s = stub(|_, vars| ok(json!({ "issues": page(vars, Some(500)) }))).await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_reset");
    let f = r#"{"state":{"type":{"eq":"started"}}}"#;
    c.search_filter(f, 0, 50).await.unwrap();
    c.search_filter(f, 50, 50).await.unwrap();
    // Start over: the next "load more" must not jump to a stale cursor at 100.
    c.search_filter(f, 0, 50).await.unwrap();
    let n = s.seen().len();
    assert_eq!(
        keys(&c.search_filter(f, 100, 50).await.unwrap()),
        range(100, 150)
    );
    let after: Vec<Value> = s.seen()[n..]
        .iter()
        .map(|x| x.vars["after"].clone())
        .collect();
    assert_eq!(after, [json!("c50"), json!("c100")], "walked from 50 again");
}

#[tokio::test]
async fn a_walk_past_its_call_cap_errors_instead_of_reading_as_the_end() {
    let s = stub(|_, vars| ok(json!({ "issues": page(vars, None) }))).await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_far");
    // Nothing remembered: 10 calls of 100 cannot reach offset 5000.
    let e = c
        .search_filter(r#"{"title":{"contains":"x"}}"#, 5000, 25)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, Error::Invalid(m) if m.contains("too far")),
        "{e}"
    );
    assert_eq!(s.seen().len(), 10, "bounded by the per-request call cap");
}

#[tokio::test]
async fn page_size_is_clamped_to_linears_maximum() {
    let s = stub(|_, vars| ok(json!({ "issues": page(vars, Some(10)) }))).await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_clamp");
    c.search_filter("{}", 0, 5000).await.unwrap();
    c.search_filter(r#"{"a":{}}"#, 0, 0).await.unwrap();
    let firsts: Vec<Value> = s.seen().iter().map(|x| x.vars["first"].clone()).collect();
    assert_eq!(firsts, [json!(100), json!(1)]);
}

#[tokio::test]
async fn a_malformed_filter_never_reaches_linear() {
    let s = stub(|_, _| panic!("no call expected")).await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_badfilter");
    for bad in ["project = ENG", "[1,2]", ""] {
        assert!(matches!(
            c.search_filter(bad, 0, 25).await,
            Err(Error::Invalid(m)) if m.contains("IssueFilter")
        ));
    }
    assert!(s.seen().is_empty());
}

// ---------------------------------------------------------------------------
// Picker search routing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_identifier_is_fetched_directly_and_a_miss_falls_back_to_full_text() {
    let s = stub(|q, vars| {
        if q.contains("issue(id:") {
            return match vars["id"].as_str().unwrap() {
                "ENG-12" => ok(json!({ "issue": issue_node(12) })),
                "ENG-7" => ok(json!({ "issue": issue_node(7) })),
                _ => ok(json!({ "issue": null })),
            };
        }
        assert!(q.contains("searchIssues"), "{q}");
        ok(json!({ "searchIssues": page(vars, Some(3)) }))
    })
    .await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_search");

    // Lower-case identifier → normalised, one direct lookup.
    let hit = c.search(" eng-12 ", None, 0, 25).await.unwrap();
    assert_eq!(keys(&hit), ["ENG-12"]);
    assert_eq!(s.calls("issue(id:")[0].vars["id"], "ENG-12");
    assert!(s.calls("searchIssues").is_empty());

    // A bare number inside a team is that team's identifier.
    assert_eq!(
        keys(&c.search("7", Some("ENG"), 0, 25).await.unwrap()),
        ["ENG-7"]
    );

    // An identifier-shaped term with no such issue → full-text search.
    let res = c.search("ABC-999", None, 0, 25).await.unwrap();
    assert_eq!(res.len(), 3);
    assert_eq!(s.calls("searchIssues")[0].vars["term"], "ABC-999");

    // "Load more" on a direct hit has nothing more and calls nothing.
    let before = s.seen().len();
    assert!(c.search("ENG-12", None, 25, 25).await.unwrap().is_empty());
    assert_eq!(s.seen().len(), before);

    // Free text inside a team carries the team filter.
    c.search("login bug", Some("OPS"), 0, 25).await.unwrap();
    let last = s.calls("searchIssues").pop().unwrap();
    assert_eq!(last.vars["term"], "login bug");
    assert_eq!(last.vars["filter"], json!({"team": {"key": {"eq": "OPS"}}}));
}

#[tokio::test]
async fn an_empty_query_lists_my_issues_optionally_in_one_team() {
    let s = stub(|_, vars| ok(json!({ "issues": page(vars, Some(2)) }))).await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_mine");
    assert_eq!(c.search("  ", None, 0, 25).await.unwrap().len(), 2);
    c.search("", Some("ENG"), 0, 25).await.unwrap();
    let f: Vec<Value> = s.seen().iter().map(|x| x.vars["filter"].clone()).collect();
    assert_eq!(f[0], json!({"assignee": {"isMe": {"eq": true}}}));
    assert_eq!(
        f[1],
        json!({"and": [{"assignee": {"isMe": {"eq": true}}}, {"team": {"key": {"eq": "ENG"}}}]})
    );
}

#[tokio::test]
async fn a_lookup_failure_other_than_not_found_is_not_masked_by_full_text() {
    let s = stub(|q, _| {
        assert!(
            !q.contains("searchIssues"),
            "must not fall back on an auth error"
        );
        (
            401,
            json!({"errors": [{"message": "Authentication required"}]}).to_string(),
        )
    })
    .await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_authfail");
    assert!(matches!(
        c.search("ENG-1", None, 0, 25).await,
        Err(Error::Upstream(m)) if m.contains("authentication")
    ));
}

#[tokio::test]
async fn teams_are_listed_by_name_with_the_more_flag() {
    let s = stub(|_, _| {
        ok(json!({"teams": {"nodes": [
            {"key": "OPS", "name": "operations"}, {"key": "ENG", "name": "Engineering"},
            {"key": "DES", "name": "design"}
        ], "pageInfo": {"hasNextPage": true}}}))
    })
    .await;
    let (teams, more) = LinearClient::with_endpoint(&s.url, "lin_api_teams")
        .list_projects_paged()
        .await
        .unwrap();
    let k: Vec<&str> = teams.iter().map(|t| t.key.as_str()).collect();
    assert_eq!(k, ["DES", "ENG", "OPS"], "case-insensitive by name");
    assert!(more);
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

#[tokio::test]
async fn comments_walk_every_page_and_come_back_oldest_first() {
    let s = stub(|q, vars| {
        assert!(q.contains("comments(first: 100"));
        let c = |id: &str, at: &str| {
            json!({"id": id, "body": format!("body {id}"), "createdAt": at,
                   "user": {"name": "Dana"}, "botActor": null})
        };
        match vars["after"].as_str() {
            None => ok(json!({"issue": {"comments": {
                "nodes": [c("c3", "2026-10-03T00:00:00.000Z"), c("c1", "2026-10-01T00:00:00.000Z")],
                "pageInfo": {"hasNextPage": true, "endCursor": "p2"}}}})),
            Some("p2") => ok(json!({"issue": {"comments": {
                "nodes": [c("c2", "2026-10-02T00:00:00.000Z")],
                "pageInfo": {"hasNextPage": false, "endCursor": "p3"}}}})),
            other => panic!("unexpected cursor {other:?}"),
        }
    })
    .await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_comments");
    let all = c.list_comments("ENG-5").await.unwrap();
    let ids: Vec<&str> = all.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["c1", "c2", "c3"]);
    assert_eq!(all[0].author, "Dana");
    // `since` is inclusive (the watcher's cursor is the last seen timestamp).
    let since = c
        .list_comments_since("ENG-5", "2026-10-02T00:00:00.000Z")
        .await
        .unwrap();
    let ids: Vec<&str> = since.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["c2", "c3"]);
}

#[tokio::test]
async fn the_full_issue_carries_its_comments() {
    let s = stub(|q, _| {
        if q.contains("comments(") {
            return ok(json!({"issue": {"comments": {"nodes": [
                {"id": "k1", "body": "hi", "createdAt": "2026-10-01T00:00:00.000Z",
                 "user": null, "botActor": {"name": "GitHub"}}
            ], "pageInfo": {"hasNextPage": false}}}}));
        }
        let mut i = issue_node(9);
        i["description"] = json!("desc");
        ok(json!({ "issue": i }))
    })
    .await;
    let full = LinearClient::with_endpoint(&s.url, "lin_api_full")
        .get_issue_full("ENG-9")
        .await
        .unwrap();
    assert_eq!(full.key, "ENG-9");
    assert_eq!(full.description_md, "desc");
    assert_eq!(full.comments.len(), 1);
    assert_eq!(full.comments[0].author, "GitHub");
}

#[tokio::test]
async fn a_comment_is_posted_against_the_issue_uuid() {
    let s = stub(|q, vars| {
        if q.contains("commentCreate") {
            assert_eq!(vars["input"]["issueId"], "uuid-4");
            assert_eq!(vars["input"]["body"], "**done**");
            return ok(json!({"commentCreate": {"success": true,
                "comment": {"id": "cm1", "url": "https://linear.app/c/cm1"}}}));
        }
        ok(json!({"issue": {"id": "uuid-4"}}))
    })
    .await;
    let r = LinearClient::with_endpoint(&s.url, "lin_api_comment")
        .add_comment("ENG-4", "**done**")
        .await
        .unwrap();
    assert_eq!(r.id, "cm1");
    assert_eq!(r.url.as_deref(), Some("https://linear.app/c/cm1"));
    // The identifier lookup came first.
    assert_eq!(s.seen()[0].vars["id"], "ENG-4");
}

#[tokio::test]
async fn a_mutation_linear_did_not_apply_is_an_error() {
    let s = stub(|q, _| {
        if q.contains("commentCreate") {
            return ok(json!({"commentCreate": {"success": false, "comment": null}}));
        }
        if q.contains("issueUpdate") {
            return ok(json!({"issueUpdate": {"success": false}}));
        }
        ok(json!({"issue": {"id": "uuid-4"}}))
    })
    .await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_notapplied");
    assert!(matches!(
        c.add_comment("ENG-4", "x").await,
        Err(Error::Upstream(m)) if m.contains("not applied")
    ));
    assert!(matches!(
        c.transition_issue("ENG-4", "s1").await,
        Err(Error::Upstream(m)) if m.contains("not applied")
    ));
}

// ---------------------------------------------------------------------------
// Issue updates
// ---------------------------------------------------------------------------

#[tokio::test]
async fn updates_send_the_exact_issue_update_input() {
    let s = stub(|_, _| ok(json!({"issueUpdate": {"success": true}}))).await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_update");
    c.transition_issue("ENG-1", "state-9").await.unwrap();
    c.assign_issue("ENG-1", "user-3").await.unwrap();
    // Jira's "-1" (and an empty id) means unassign: Linear wants null.
    c.assign_issue("ENG-1", "-1").await.unwrap();
    c.assign_issue("ENG-1", " ").await.unwrap();
    c.update_description("ENG-1", "# New\nbody").await.unwrap();
    let inputs: Vec<Value> = s.seen().iter().map(|x| x.vars["input"].clone()).collect();
    assert_eq!(
        inputs,
        [
            json!({"stateId": "state-9"}),
            json!({"assigneeId": "user-3"}),
            json!({"assigneeId": null}),
            json!({"assigneeId": null}),
            json!({"description": "# New\nbody"}),
        ]
    );
    assert!(s.seen().iter().all(|x| x.vars["id"] == "ENG-1"));
}

#[tokio::test]
async fn transitions_and_assignees_come_from_the_issues_team() {
    let s = stub(|q, _| {
        if q.contains("states(") {
            return ok(
                json!({"issue": {"state": {"id": "s1"}, "team": {"states": {"nodes": [
                    {"id": "s1", "name": "Todo", "type": "unstarted", "position": 0},
                    {"id": "s2", "name": "Done", "type": "completed", "position": 0}
                ]}}}}),
            );
        }
        ok(json!({"issue": {"team": {"members": {"nodes": [
            {"id": "u1", "name": "Dana", "displayName": "dana", "avatarUrl": null, "active": true},
            {"id": "u2", "name": "Gone", "displayName": "gone", "avatarUrl": null, "active": false},
            {"id": "u3", "name": "", "displayName": "lee", "avatarUrl": "https://a/l.png"}
        ]}}}}))
    })
    .await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_team");
    let t = c.list_transitions("ENG-1").await.unwrap();
    assert_eq!(t.len(), 1, "the current state is not a transition");
    assert_eq!((t[0].id.as_str(), t[0].name.as_str()), ("s2", "Done"));
    let users = c.list_assignable("ENG-1").await.unwrap();
    let names: Vec<&str> = users.iter().map(|u| u.display_name.as_str()).collect();
    assert_eq!(
        names,
        ["Dana", "lee"],
        "deactivated members are not offered"
    );
}

// ---------------------------------------------------------------------------
// Create + issue types (labels)
// ---------------------------------------------------------------------------

/// A workspace with team ENG, workspace + team labels and a label group.
fn create_stub(q: &str, vars: &Value) -> Reply {
    if q.contains("teams(filter") {
        return match vars["key"].as_str().unwrap() {
            "ENG" => ok(json!({"teams": {"nodes": [{"id": "team-eng"}]}})),
            _ => ok(json!({"teams": {"nodes": []}})),
        };
    }
    if q.contains("issueLabels") {
        return ok(json!({"issueLabels": {"nodes": [
            {"id": "l-bug", "name": "Bug", "isGroup": false, "team": null},
            {"id": "l-feat", "name": "feature", "isGroup": false, "team": {"key": "ENG"}},
            {"id": "l-grp", "name": "Area", "isGroup": true, "team": null},
            {"id": "l-ops", "name": "Ops only", "isGroup": false, "team": {"key": "OPS"}},
            {"id": "l-bug2", "name": "Bug", "isGroup": false, "team": {"key": "ENG"}}
        ]}}));
    }
    assert!(q.contains("issueCreate"), "{q}");
    ok(json!({"issueCreate": {"success": true,
        "issue": {"identifier": "ENG-77", "url": "https://linear.app/acme/issue/ENG-77"}}}))
}

#[tokio::test]
async fn create_applies_a_matching_label_as_the_issue_type() {
    let s = stub(create_stub).await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_create");
    let created = c
        .create_issue("ENG", "bug", "Crash", "steps")
        .await
        .unwrap();
    assert_eq!(created.key, "ENG-77");
    let input = s.calls("issueCreate")[0].vars["input"].clone();
    assert_eq!(input["teamId"], "team-eng");
    assert_eq!(input["title"], "Crash");
    assert_eq!(input["description"], "steps");
    assert_eq!(
        input["labelIds"],
        json!(["l-bug"]),
        "case-insensitive match"
    );

    // The generic type and an unknown one create without labels; the generic
    // type does not even look labels up.
    let before = s.calls("issueLabels").len();
    c.create_issue("ENG", "Issue", "A", "").await.unwrap();
    assert_eq!(s.calls("issueLabels").len(), before);
    c.create_issue("ENG", "Epic", "B", "").await.unwrap();
    for call in s.calls("issueCreate").iter().skip(1) {
        assert!(
            call.vars["input"].get("labelIds").is_none(),
            "{:?}",
            call.vars
        );
    }
}

#[tokio::test]
async fn create_in_an_unknown_team_is_not_found() {
    let s = stub(create_stub).await;
    let c = LinearClient::with_endpoint(&s.url, "lin_api_noteam");
    assert!(matches!(
        c.create_issue("NOPE", "Issue", "x", "").await,
        Err(Error::NotFound(m)) if m.contains("NOPE")
    ));
    assert!(s.calls("issueCreate").is_empty());
}

#[tokio::test]
async fn issue_types_are_the_generic_type_then_usable_labels() {
    let s = stub(create_stub).await;
    let types = LinearClient::with_endpoint(&s.url, "lin_api_types")
        .list_issue_types("ENG")
        .await
        .unwrap();
    // Generic first; groups and other teams' labels excluded; deduped; sorted.
    assert_eq!(types, ["Issue", "Bug", "feature"]);
}

#[tokio::test]
async fn dev_status_lists_only_pull_request_attachments() {
    let s = stub(|_, _| {
        ok(json!({"issue": {"attachments": {"nodes": [
            {"id": "a1", "title": "PR #1", "subtitle": "Merged", "url": "https://github.com/o/r/pull/1",
             "sourceType": "github", "updatedAt": "t1", "metadata": {}},
            {"id": "a2", "title": "MR !2", "subtitle": null, "url": "https://gitlab.com/o/r/-/merge_requests/2",
             "sourceType": "gitlab", "updatedAt": "t2", "metadata": {"status": "opened", "repoName": "r"}},
            {"id": "a3", "title": "Figma", "subtitle": null, "url": "https://figma.com/file/x",
             "sourceType": "figma", "updatedAt": "t3", "metadata": {}}
        ]}}}))
    })
    .await;
    let dev = LinearClient::with_endpoint(&s.url, "lin_api_dev")
        .dev_status("ENG-1")
        .await
        .unwrap();
    let prs: Vec<(&str, &str)> = dev
        .pull_requests
        .iter()
        .map(|p| (p.id.as_str(), p.status.as_str()))
        .collect();
    assert_eq!(prs, [("a1", "Merged"), ("a2", "opened")]);
    assert!(dev.branches.is_empty() && dev.commits.is_empty());
}
