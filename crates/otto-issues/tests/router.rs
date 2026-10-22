//! The issues router end to end: account CRUD against a real (in-memory)
//! state DB and secret store, the per-account ownership gate on EVERY route
//! that acts with an account's credentials, and the provider gates that must
//! refuse before any upstream call (a JQL string never reaches Linear, a
//! Linear key never reaches Confluence).
//!
//! Requests that pass every gate would go upstream, so the Jira accounts
//! here point at a closed loopback port (or, for the create-issue round trip,
//! a loopback stand-in Jira) and the Linear-gated cases are only ones the
//! router refuses locally — nothing in this file leaves the machine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::Router;
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::secrets::SecretStore;
use otto_issues::IssuesCtx;
use otto_state::users::UsersRepo;
use otto_state::IssuesRepo;
use serde_json::{json, Value};

#[derive(Default)]
struct MemSecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemSecrets {
    fn put(&self, k: &str, v: &str) -> otto_core::Result<()> {
        self.0.lock().unwrap().insert(k.into(), v.into());
        Ok(())
    }
    fn get(&self, k: &str) -> otto_core::Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(k).cloned())
    }
    fn delete(&self, k: &str) -> otto_core::Result<()> {
        self.0.lock().unwrap().remove(k);
        Ok(())
    }
}

#[derive(Clone)]
struct Ctx {
    issues: IssuesRepo,
    secrets: Arc<dyn SecretStore>,
}

impl IssuesCtx for Ctx {
    fn issues(&self) -> &IssuesRepo {
        &self.issues
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
}

struct App {
    base: String,
    http: reqwest::Client,
    mem: Arc<MemSecrets>,
    repo: IssuesRepo,
    alice: User,
    bob: User,
    root: User,
}

/// What a request came back with.
struct Resp {
    status: u16,
    body: Value,
}

impl App {
    async fn call(&self, who: &User, method: &str, path: &str, body: Option<Value>) -> Resp {
        let url = format!("{}{path}", self.base);
        let mut req = self
            .http
            .request(method.parse().unwrap(), url)
            .header("x-test-user", &who.id);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let r = req.send().await.unwrap();
        let status = r.status().as_u16();
        let text = r.text().await.unwrap();
        Resp {
            status,
            body: serde_json::from_str(&text).unwrap_or(Value::String(text)),
        }
    }

    fn secret(&self, key: &str) -> Option<String> {
        self.mem.0.lock().unwrap().get(key).cloned()
    }

    /// The Keychain item an account names — read from the DB, because the
    /// API never serialises it.
    async fn token_ref(&self, account: &Value) -> String {
        let id = account["id"].as_str().unwrap().to_string();
        self.repo.get_account(&id).await.unwrap().token_ref
    }

    async fn create(&self, who: &User, body: Value) -> Value {
        let r = self
            .call(who, "POST", "/issue/accounts", Some(body.clone()))
            .await;
        assert_eq!(r.status, 200, "create {body}: {}", r.body);
        assert!(r.body.get("token_ref").is_none(), "{}", r.body);
        r.body
    }

    async fn jira(&self, who: &User) -> Value {
        self.create(
            who,
            json!({"provider": "jira", "label": "work", "email": "a@example.com",
                   "base_url": "http://127.0.0.1:1", "token": "jira-secret-token"}),
        )
        .await
    }

    async fn linear(&self, who: &User) -> Value {
        self.create(
            who,
            json!({"provider": "linear", "label": "lin", "email": "", "base_url": "",
                   "token": "lin_api_secret"}),
        )
        .await
    }
}

async fn app() -> App {
    let pool = otto_state::db::test_pool().await;
    let users = UsersRepo::new(pool.clone());
    let alice = users.create("alice", "x", "Alice", false).await.unwrap();
    let bob = users.create("bob", "x", "Bob", false).await.unwrap();
    let root = users.create("root", "x", "Root", true).await.unwrap();
    let mem = Arc::new(MemSecrets::default());
    let repo = IssuesRepo::new(pool);
    let ctx = Ctx {
        issues: repo.clone(),
        secrets: mem.clone(),
    };
    let by_id: Arc<HashMap<String, User>> = Arc::new(
        [&alice, &bob, &root]
            .into_iter()
            .map(|u| (u.id.clone(), u.clone()))
            .collect(),
    );
    // Stand-in for the server's auth layer: the caller named by a header.
    let auth = move |mut req: Request, next: Next| {
        let by_id = Arc::clone(&by_id);
        async move {
            let id = req
                .headers()
                .get("x-test-user")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let user = by_id.get(&id).cloned().expect("known test user");
            req.extensions_mut().insert(AuthUser(user));
            Ok::<Response, std::convert::Infallible>(next.run(req).await)
        }
    };
    let router: Router = otto_issues::router::<Ctx>()
        .layer(middleware::from_fn(auth))
        .with_state(ctx);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    App {
        base: format!("http://{addr}"),
        http: reqwest::Client::new(),
        mem,
        repo,
        alice,
        bob,
        root,
    }
}

/// Every route that resolves an account and acts with its credentials.
fn account_routes(id: &str) -> Vec<(&'static str, String, Option<Value>)> {
    let q = format!("account_id={id}");
    vec![
        ("GET", format!("/issue/projects?{q}"), None),
        ("GET", format!("/issue/search?{q}&q=x"), None),
        ("GET", format!("/issue/my-work?{q}"), None),
        ("GET", format!("/issue/confluence/spaces?{q}"), None),
        ("GET", format!("/issue/confluence/search?{q}&q=x"), None),
        (
            "POST",
            format!("/issue/confluence/pages?{q}"),
            Some(json!({"space_key": "S", "title": "T", "body_md": "b"})),
        ),
        ("GET", format!("/issue/confluence/pages/1?{q}"), None),
        (
            "PUT",
            format!("/issue/confluence/pages/1?{q}"),
            Some(json!({"body_md": "b"})),
        ),
        (
            "GET",
            format!("/issue/confluence/pages/1/comments?{q}"),
            None,
        ),
        (
            "POST",
            format!("/issue/confluence/pages/1/comments?{q}"),
            Some(json!({"body_md": "b"})),
        ),
        ("GET", format!("/issue/{id}/K-1"), None),
        ("GET", format!("/issue/{id}/K-1/full"), None),
        ("GET", format!("/issue/{id}/K-1/devstatus"), None),
        ("GET", format!("/issue/{id}/K-1/transitions"), None),
        (
            "POST",
            format!("/issue/{id}/K-1/transitions"),
            Some(json!({"transition_id": "21"})),
        ),
        ("GET", format!("/issue/{id}/K-1/assignable"), None),
        (
            "PUT",
            format!("/issue/{id}/K-1/assignee"),
            Some(json!({"account_id": "-1"})),
        ),
        ("GET", format!("/issue/{id}/K-1/attachment/9"), None),
        ("GET", format!("/issue/{id}/PRJ/issue-types"), None),
        (
            "POST",
            format!("/issue/{id}/issues"),
            Some(json!({"project_key": "PRJ", "issue_type": "Task", "summary": "s"})),
        ),
        (
            "POST",
            format!("/issue/{id}/K-1/comment"),
            Some(json!({"body": "hi"})),
        ),
        ("GET", format!("/issue/{id}/K-1/editmeta"), None),
        (
            "PUT",
            format!("/issue/{id}/K-1/fields"),
            Some(json!({"fields": {}})),
        ),
        (
            "PUT",
            format!("/issue/{id}/K-1/description"),
            Some(json!({"body_md": ""})),
        ),
        ("PATCH", format!("/issue/accounts/{id}"), Some(json!({}))),
        ("DELETE", format!("/issue/accounts/{id}"), None),
    ]
}

#[tokio::test]
async fn nobody_but_the_owner_can_use_an_account_on_any_route() {
    let a = app().await;
    let acct = a.jira(&a.alice).await;
    let id = acct["id"].as_str().unwrap();
    let token_ref = a.token_ref(&acct).await;

    for (method, path, body) in account_routes(id) {
        let r = a.call(&a.bob, method, &path, body).await;
        assert_eq!(r.status, 403, "{method} {path}: {}", r.body);
        assert_eq!(r.body["code"], "forbidden", "{method} {path}");
    }
    // Bob's failed PATCH / DELETE changed nothing.
    assert_eq!(a.secret(&token_ref).as_deref(), Some("jira-secret-token"));
    let mine = a.call(&a.alice, "GET", "/issue/accounts", None).await;
    assert_eq!(mine.body.as_array().unwrap().len(), 1);
    assert_eq!(mine.body[0]["label"], "work");
}

#[tokio::test]
async fn an_unknown_account_is_404_not_a_leak_or_a_500() {
    let a = app().await;
    for (method, path, body) in account_routes("no-such-account") {
        let r = a.call(&a.alice, method, &path, body).await;
        assert_eq!(r.status, 404, "{method} {path}: {}", r.body);
    }
}

#[tokio::test]
async fn the_owner_and_root_pass_the_gate() {
    let a = app().await;
    let acct = a.jira(&a.alice).await;
    let id = acct["id"].as_str().unwrap();
    // Past the ownership gate the request goes upstream — here a closed port,
    // so a 502 (not 403/404) proves the gate let the caller through.
    for who in [&a.alice, &a.root] {
        let r = a
            .call(who, "GET", &format!("/issue/{id}/K-1/transitions"), None)
            .await;
        assert_eq!(r.status, 502, "{}: {}", who.username, r.body);
    }
}

#[tokio::test]
async fn listings_only_show_the_callers_own_accounts() {
    let a = app().await;
    a.jira(&a.alice).await;
    a.linear(&a.bob).await;
    let alice = a.call(&a.alice, "GET", "/issue/accounts", None).await;
    let bob = a.call(&a.bob, "GET", "/issue/accounts", None).await;
    assert_eq!(alice.body.as_array().unwrap().len(), 1);
    assert_eq!(alice.body[0]["provider"], "jira");
    assert_eq!(bob.body.as_array().unwrap().len(), 1);
    assert_eq!(bob.body[0]["provider"], "linear");
}

#[tokio::test]
async fn create_validates_per_provider_and_never_echoes_the_token() {
    let a = app().await;
    let base = json!({"provider": "jira", "label": "l", "email": "e@x.io",
                      "base_url": "https://x.atlassian.net", "token": "t"});
    for (field, value) in [("token", "  "), ("email", ""), ("base_url", " ")] {
        let mut b = base.clone();
        b[field] = json!(value);
        let r = a.call(&a.alice, "POST", "/issue/accounts", Some(b)).await;
        assert_eq!(r.status, 400, "jira without {field}: {}", r.body);
    }
    assert!(
        a.mem.0.lock().unwrap().is_empty(),
        "no secret for a refused create"
    );

    // Linear: the API key alone; the base URL defaults to the workspace site.
    let lin = a.linear(&a.alice).await;
    assert_eq!(lin["provider"], "linear");
    assert_eq!(lin["email"], "");
    assert_eq!(lin["base_url"], "https://linear.app");
    let token_ref = a.token_ref(&lin).await;
    assert!(token_ref.starts_with("issueacct-"), "{token_ref}");
    assert_eq!(a.secret(&token_ref).as_deref(), Some("lin_api_secret"));
    assert!(
        !lin.to_string().contains("lin_api_secret"),
        "the token is write-only: {lin}"
    );
    // A Linear account still needs a key.
    let r = a
        .call(
            &a.alice,
            "POST",
            "/issue/accounts",
            Some(json!({"provider": "linear", "label": "l", "email": "", "base_url": "", "token": ""})),
        )
        .await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn moving_an_account_to_another_host_needs_a_fresh_token() {
    let a = app().await;
    let acct = a
        .create(
            &a.alice,
            json!({"provider": "jira", "label": "work", "email": "a@example.com",
                   "base_url": "https://acme.atlassian.net", "token": "old-token"}),
        )
        .await;
    let id = acct["id"].as_str().unwrap();
    let old_ref = a.token_ref(&acct).await;
    let path = format!("/issue/accounts/{id}");

    // Another host without a token: refused, nothing changed.
    let r = a
        .call(
            &a.alice,
            "PATCH",
            &path,
            Some(json!({"base_url": "https://evil.example"})),
        )
        .await;
    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(a.secret(&old_ref).as_deref(), Some("old-token"));

    // Same host, other path; label/email edits: the stored token stays.
    let r = a
        .call(
            &a.alice,
            "PATCH",
            &path,
            Some(json!({"base_url": "https://ACME.atlassian.net/wiki", "label": "renamed"})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body["label"], "renamed");
    assert_eq!(r.body["email"], "a@example.com", "absent field kept");
    assert_eq!(a.token_ref(&acct).await, old_ref);

    // Another host WITH a new token: the token rotates, the old one is gone.
    let r = a
        .call(
            &a.alice,
            "PATCH",
            &path,
            Some(json!({"base_url": "https://other.atlassian.net", "token": "new-token"})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let new_ref = a.token_ref(&acct).await;
    assert_ne!(new_ref, old_ref);
    assert_eq!(a.secret(&new_ref).as_deref(), Some("new-token"));
    assert_eq!(
        a.secret(&old_ref),
        None,
        "the rotated-out secret is deleted"
    );
}

#[tokio::test]
async fn token_expiry_is_kept_cleared_or_set_by_patch() {
    let a = app().await;
    let acct = a
        .create(
            &a.alice,
            json!({"provider": "jira", "label": "w", "email": "a@x.io",
                   "base_url": "https://x.atlassian.net", "token": "t",
                   "token_expires_at": "2027-01-01T00:00:00Z"}),
        )
        .await;
    let path = format!("/issue/accounts/{}", acct["id"].as_str().unwrap());
    let r = a
        .call(&a.alice, "PATCH", &path, Some(json!({"label": "x"})))
        .await;
    assert!(
        r.body["token_expires_at"]
            .as_str()
            .is_some_and(|s| s.starts_with("2027-01-01")),
        "absent keeps: {}",
        r.body
    );
    let r = a
        .call(
            &a.alice,
            "PATCH",
            &path,
            Some(json!({"token_expires_at": null})),
        )
        .await;
    assert!(
        r.body["token_expires_at"].is_null(),
        "null clears: {}",
        r.body
    );
}

#[tokio::test]
async fn delete_removes_the_row_and_its_secret() {
    let a = app().await;
    let acct = a.jira(&a.alice).await;
    let id = acct["id"].as_str().unwrap();
    let token_ref = a.token_ref(&acct).await;
    let r = a
        .call(&a.alice, "DELETE", &format!("/issue/accounts/{id}"), None)
        .await;
    assert_eq!(r.status, 204);
    assert_eq!(a.secret(&token_ref), None);
    let mine = a.call(&a.alice, "GET", "/issue/accounts", None).await;
    assert!(mine.body.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_keychain_secret_is_a_clear_400() {
    let a = app().await;
    let acct = a.jira(&a.alice).await;
    let gone = a.token_ref(&acct).await;
    a.mem.0.lock().unwrap().remove(&gone);
    let id = acct["id"].as_str().unwrap();
    let r = a
        .call(&a.alice, "GET", &format!("/issue/{id}/K-1"), None)
        .await;
    assert_eq!(r.status, 400);
    assert!(
        r.body["message"]
            .as_str()
            .unwrap()
            .contains("token missing"),
        "{}",
        r.body
    );
}

#[tokio::test]
async fn a_query_in_the_other_trackers_language_is_refused_locally() {
    let a = app().await;
    let lin = a.linear(&a.alice).await;
    let jira = a.jira(&a.alice).await;
    let (lin, jira) = (lin["id"].as_str().unwrap(), jira["id"].as_str().unwrap());

    let r = a
        .call(
            &a.alice,
            "GET",
            &format!("/issue/search?account_id={lin}&jql=project%20%3D%20X"),
            None,
        )
        .await;
    assert_eq!(r.status, 400);
    assert!(r.body["message"]
        .as_str()
        .unwrap()
        .contains("Linear account"));

    // `query` speaks the account's own language: not JSON → not a filter.
    let r = a
        .call(
            &a.alice,
            "GET",
            &format!("/issue/search?account_id={lin}&query=project%20%3D%20X"),
            None,
        )
        .await;
    assert_eq!(r.status, 400);
    assert!(r.body["message"].as_str().unwrap().contains("IssueFilter"));

    let r = a
        .call(
            &a.alice,
            "GET",
            &format!("/issue/search?account_id={jira}&filter=%7B%7D"),
            None,
        )
        .await;
    assert_eq!(r.status, 400);
    assert!(r.body["message"].as_str().unwrap().contains("Jira account"));
}

#[tokio::test]
async fn jira_only_features_answer_400_naming_linear_and_confluence_refuses_its_key() {
    let a = app().await;
    let lin = a.linear(&a.alice).await;
    let id = lin["id"].as_str().unwrap();
    let q = format!("account_id={id}");

    let gated: Vec<(&str, String, Option<Value>)> = vec![
        ("GET", format!("/issue/{id}/K-1/attachment/1"), None),
        (
            "PUT",
            format!("/issue/{id}/K-1/fields"),
            Some(json!({"fields": {"customfield_1": 3}})),
        ),
        ("GET", format!("/issue/confluence/spaces?{q}"), None),
        (
            "GET",
            format!("/issue/confluence/search?{q}&cql=type%3Dpage"),
            None,
        ),
        ("GET", format!("/issue/confluence/pages/1?{q}"), None),
        (
            "POST",
            format!("/issue/confluence/pages?{q}"),
            Some(json!({"space_key": "S", "title": "T", "body_md": "b"})),
        ),
        (
            "PUT",
            format!("/issue/confluence/pages/1?{q}"),
            Some(json!({"body_md": "b"})),
        ),
        (
            "GET",
            format!("/issue/confluence/pages/1/comments?{q}"),
            None,
        ),
        (
            "POST",
            format!("/issue/confluence/pages/1/comments?{q}"),
            Some(json!({"body_md": "b"})),
        ),
    ];
    for (method, path, body) in gated {
        let r = a.call(&a.alice, method, &path, body).await;
        assert_eq!(r.status, 400, "{method} {path}: {}", r.body);
        assert!(
            r.body["message"].as_str().unwrap().contains("Linear"),
            "{method} {path}: {}",
            r.body
        );
    }
    // Linear has no editmeta: an empty field list, not an error.
    let r = a
        .call(&a.alice, "GET", &format!("/issue/{id}/K-1/editmeta"), None)
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, json!([]));
}

#[tokio::test]
async fn write_bodies_are_validated_before_the_account_is_used() {
    let a = app().await;
    let acct = a.jira(&a.alice).await;
    let id = acct["id"].as_str().unwrap();
    let cases: Vec<(&str, String, Value, &str)> = vec![
        (
            "POST",
            format!("/issue/{id}/K-1/transitions"),
            json!({}),
            "transition_id",
        ),
        (
            "PUT",
            format!("/issue/{id}/K-1/assignee"),
            json!({}),
            "account_id",
        ),
        (
            "POST",
            format!("/issue/{id}/K-1/comment"),
            json!({}),
            "body",
        ),
        (
            "POST",
            format!("/issue/{id}/issues"),
            json!({"project_key": "PRJ", "issue_type": "Task", "summary": " "}),
            "summary",
        ),
        (
            "POST",
            format!("/issue/{id}/issues"),
            json!({"project_key": "", "issue_type": "Task", "summary": "s"}),
            "project_key",
        ),
        (
            "PUT",
            format!("/issue/{id}/K-1/fields"),
            json!({"fields": [1]}),
            "object",
        ),
        (
            "PUT",
            format!("/issue/{id}/K-1/description"),
            json!({}),
            "body_md",
        ),
        (
            "POST",
            format!("/issue/confluence/pages?account_id={id}"),
            json!({"space_key": " ", "title": "T"}),
            "space_key",
        ),
        (
            "POST",
            format!("/issue/confluence/pages/1/comments?account_id={id}"),
            json!({}),
            "body_md",
        ),
    ];
    for (method, path, body, needle) in cases {
        let r = a.call(&a.alice, method, &path, Some(body)).await;
        assert_eq!(r.status, 400, "{method} {path}: {}", r.body);
        assert!(
            r.body["message"].as_str().unwrap().contains(needle),
            "{method} {path}: {}",
            r.body
        );
    }
    // Listing endpoints need the account named.
    let r = a.call(&a.alice, "GET", "/issue/search?q=x", None).await;
    assert_eq!(r.status, 400);
}

/// What the stand-in Jira saw and how it should misbehave.
#[derive(Default)]
struct FakeJira {
    posted: Option<Value>,
    /// Store something other than what was sent (verification must notice).
    tamper: bool,
}

/// A stand-in Jira Cloud site: createmeta for project PRJ (Task / Bug /
/// Epic; Bug needs a `Team` custom field), parent lookups (PRJ-1 is an
/// Epic, PRJ-2 a Story), the create call, and the read-back of PRJ-7 echoing
/// what was posted.
async fn fake_jira() -> (String, Arc<Mutex<FakeJira>>) {
    use axum::extract::{Path as P, State as St};
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    type S = Arc<Mutex<FakeJira>>;
    let state: S = Arc::default();
    let field = |id: &str, name: &str, required: bool| json!({"fieldId": id, "key": id, "name": name, "required": required, "hasDefaultValue": false});
    let mut fields = vec![
        field("summary", "Summary", true),
        field("issuetype", "Issue Type", true),
        field("project", "Project", true),
        field("description", "Description", false),
        field("labels", "Labels", false),
        field("parent", "Parent", false),
        json!({"fieldId": "reporter", "name": "Reporter", "required": true, "hasDefaultValue": true}),
        json!({"fieldId": "priority", "name": "Priority", "required": false, "hasDefaultValue": true,
               "allowedValues": [{"id": "2", "name": "High"}, {"id": "3", "name": "Low"}]}),
    ];
    let task_fields = json!({"fields": fields.clone(), "total": fields.len()});
    fields.push(field("customfield_100", "Team", true));
    let bug_fields = json!({"fields": fields.clone(), "total": fields.len()});
    let app = Router::new()
        .route(
            "/rest/api/3/issue/createmeta/{project}/issuetypes",
            get(|P(project): P<String>| async move {
                if project != "PRJ" {
                    return (StatusCode::NOT_FOUND, axum::Json(json!({"errorMessages": ["No project could be found"]})));
                }
                (StatusCode::OK, axum::Json(json!({"issueTypes": [
                    {"id": "10001", "name": "Task", "subtask": false},
                    {"id": "10002", "name": "Bug", "subtask": false},
                    {"id": "10003", "name": "Epic", "subtask": false}
                ], "total": 3})))
            }),
        )
        .route(
            "/rest/api/3/issue/createmeta/{project}/issuetypes/{id}",
            get(move |P((_, id)): P<(String, String)>| {
                let body = if id == "10002" { bug_fields.clone() } else { task_fields.clone() };
                async move { axum::Json(body) }
            }),
        )
        .route(
            "/rest/api/3/issue",
            post(|St(st): St<S>, axum::Json(body): axum::Json<Value>| async move {
                st.lock().unwrap().posted = Some(body);
                (StatusCode::CREATED, axum::Json(json!({"id": "10050", "key": "PRJ-7"})))
            }),
        )
        .route(
            "/rest/api/3/issue/{key}",
            get(|St(st): St<S>, P(key): P<String>| async move {
                let typed = |name: &str, level: i64| {
                    (StatusCode::OK, axum::Json(json!({"key": key, "fields": {"issuetype": {"name": name, "hierarchyLevel": level}}})))
                };
                match key.as_str() {
                    "PRJ-1" => return typed("Epic", 1),
                    "PRJ-2" => return typed("Story", 0),
                    "PRJ-7" => {}
                    _ => return (StatusCode::NOT_FOUND, axum::Json(json!({"errorMessages": ["Issue does not exist"]}))),
                }
                let st = st.lock().unwrap();
                let sent = st.posted.clone().unwrap_or_default()["fields"].clone();
                let mut f = json!({
                    "project": {"key": sent["project"]["key"]},
                    "issuetype": {"id": sent["issuetype"]["id"], "name": "Task"},
                    "summary": sent["summary"],
                    "labels": sent.get("labels").cloned().unwrap_or(json!([])),
                    "priority": {"id": sent["priority"]["id"], "name": "High"},
                    "parent": {"key": sent["parent"]["key"]},
                    "description": sent["description"],
                });
                if st.tamper {
                    f["summary"] = json!("someone else's summary");
                    // Jira "lost" the last table row.
                    if let Some(table) = f["description"]["content"]
                        .as_array_mut()
                        .and_then(|c| c.iter_mut().find(|n| n["type"] == "table"))
                    {
                        table["content"].as_array_mut().unwrap().pop();
                    }
                }
                (StatusCode::OK, axum::Json(json!({"key": "PRJ-7", "fields": f})))
            }),
        )
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let site = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (site, state)
}

async fn jira_at(a: &App, site: &str) -> String {
    let acct = a
        .create(
            &a.alice,
            json!({"provider": "jira", "label": "work", "email": "a@example.com",
                   "base_url": site, "token": "jira-secret-token"}),
        )
        .await;
    acct["id"].as_str().unwrap().to_string()
}

const TABLE_MD: &str = "## Steps\n\n1. open\n2. click\n\n| Env | Status |\n|---|---|\n| prod | **down** |\n| stg | ok |\n\n```sh\ncurl x\n```";

#[tokio::test]
async fn create_issue_validates_creates_and_verifies_against_jira() {
    let (site, jira) = fake_jira().await;
    let a = app().await;
    let id = jira_at(&a, &site).await;
    let r = a
        .call(
            &a.alice,
            "POST",
            &format!("/issue/{id}/issues"),
            Some(
                json!({"project_key": " PRJ ", "issue_type": "task", "summary": " Login fails ",
                        "description": TABLE_MD, "labels": ["sso", "p1"], "priority": "high",
                        "epic": "PRJ-1"}),
            ),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body["key"], "PRJ-7");
    assert_eq!(r.body["url"], format!("{site}/browse/PRJ-7"));
    let v = &r.body["verification"];
    assert_eq!(v["ok"], true, "{v}");
    let names: Vec<&str> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "key",
            "project",
            "issue_type",
            "summary",
            "labels",
            "priority",
            "parent",
            "description_structure",
            "description_text"
        ]
    );
    assert!(
        v["checks"][7]["detail"]
            .as_str()
            .unwrap()
            .contains("table[3x2,header]"),
        "{v}"
    );

    // What Jira received: ids resolved from createmeta, the epic as parent,
    // the description as a real ADF table (header row + body rows).
    let sent = jira
        .lock()
        .unwrap()
        .posted
        .clone()
        .expect("Jira saw the create");
    let f = &sent["fields"];
    assert_eq!(f["project"], json!({"key": "PRJ"}));
    assert_eq!(f["issuetype"], json!({"id": "10001"}));
    assert_eq!(f["summary"], "Login fails");
    assert_eq!(f["labels"], json!(["sso", "p1"]));
    assert_eq!(f["priority"], json!({"id": "2"}));
    assert_eq!(f["parent"], json!({"key": "PRJ-1"}));
    let kinds: Vec<&str> = f["description"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["heading", "orderedList", "table", "codeBlock"]);
    let table = &f["description"]["content"][2];
    assert_eq!(table["content"][0]["content"][0]["type"], "tableHeader");
    assert_eq!(
        table["content"][1]["content"][1]["content"][0]["content"][0]["marks"][0]["type"],
        "strong"
    );
    assert_eq!(f.as_object().unwrap().len(), 7, "{f}");
}

#[tokio::test]
async fn create_issue_reports_every_createmeta_problem_before_creating() {
    let (site, jira) = fake_jira().await;
    let a = app().await;
    let id = jira_at(&a, &site).await;
    let path = format!("/issue/{id}/issues");
    let msg = |r: &Resp| r.body["message"].as_str().unwrap_or_default().to_string();

    let r = a
        .call(
            &a.alice,
            "POST",
            &path,
            Some(json!({"project_key": "NOPE", "issue_type": "Task", "summary": "s"})),
        )
        .await;
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(
        msg(&r).contains("project 'NOPE' does not exist"),
        "{}",
        msg(&r)
    );

    let r = a
        .call(
            &a.alice,
            "POST",
            &path,
            Some(json!({"project_key": "PRJ", "issue_type": "Story", "summary": "s"})),
        )
        .await;
    assert_eq!(r.status, 400);
    assert!(
        msg(&r).contains("valid types: Task, Bug, Epic"),
        "{}",
        msg(&r)
    );

    // Several problems → ONE 400 naming each, with the valid values.
    let r = a
        .call(
            &a.alice,
            "POST",
            &path,
            Some(
                json!({"project_key": "PRJ", "issue_type": "10002", "summary": "s",
                        "priority": "Urgent", "epic": "PRJ-2",
                        "fields": {"bogus": 1, "summary": "x"}}),
            ),
        )
        .await;
    assert_eq!(r.status, 400);
    let m = msg(&r);
    for needle in [
        "as 'Bug'",
        "priority 'Urgent' is not valid; valid priorities: High, Low",
        "epic 'PRJ-2' is a Story, not an Epic",
        "fields.bogus: no such field",
        "fields.summary: set it with its own argument",
        "required fields missing: Team (`customfield_100`)",
    ] {
        assert!(m.contains(needle), "missing {needle:?} in:\n{m}");
    }
    assert!(
        !m.contains("Reporter"),
        "a field with a default is not 'missing': {m}"
    );

    let r = a
        .call(
            &a.alice,
            "POST",
            &path,
            Some(json!({"project_key": "PRJ", "issue_type": "Task", "summary": "s", "parent": "PRJ-404"})),
        )
        .await;
    assert_eq!(r.status, 400);
    assert!(
        msg(&r).contains("parent 'PRJ-404' does not exist"),
        "{}",
        msg(&r)
    );

    // Local checks fail before any upstream call.
    let r = a
        .call(
            &a.alice,
            "POST",
            &path,
            Some(
                json!({"project_key": "PRJ", "issue_type": "Task", "summary": "x".repeat(256),
                        "labels": ["two words"]}),
            ),
        )
        .await;
    assert_eq!(r.status, 400);
    assert!(
        msg(&r).contains("256 characters; the limit is 255"),
        "{}",
        msg(&r)
    );
    assert!(msg(&r).contains("\"two words\""), "{}", msg(&r));

    assert!(jira.lock().unwrap().posted.is_none(), "nothing was created");
}

#[tokio::test]
async fn create_issue_reports_a_verification_mismatch_without_deleting() {
    let (site, jira) = fake_jira().await;
    jira.lock().unwrap().tamper = true;
    let a = app().await;
    let id = jira_at(&a, &site).await;
    let r = a
        .call(
            &a.alice,
            "POST",
            &format!("/issue/{id}/issues"),
            Some(
                json!({"project_key": "PRJ", "issue_type": "Task", "summary": "Login fails",
                        "description": TABLE_MD}),
            ),
        )
        .await;
    // The issue exists: 200 with its key, and a verification that says what's off.
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body["key"], "PRJ-7");
    let v = &r.body["verification"];
    assert_eq!(v["ok"], false, "{v}");
    let failed: Vec<(&str, &str)> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["ok"] == false)
        .map(|c| (c["name"].as_str().unwrap(), c["detail"].as_str().unwrap()))
        .collect();
    let names: Vec<&str> = failed.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names,
        ["summary", "description_structure", "description_text"],
        "{v}"
    );
    assert!(
        failed[0].1.contains("someone else's summary"),
        "{}",
        failed[0].1
    );
    assert!(
        failed[1]
            .1
            .contains("sent table[3x2,header], stored table[2x2,header]"),
        "{}",
        failed[1].1
    );
}

#[tokio::test]
async fn create_issue_surfaces_an_unreachable_jira_as_502() {
    let a = app().await;
    let acct = a.jira(&a.alice).await;
    let id = acct["id"].as_str().unwrap();
    // The account points at a closed port: the gate and the local checks
    // pass, the first upstream call fails, and the caller sees a 502.
    let r = a
        .call(
            &a.alice,
            "POST",
            &format!("/issue/{id}/issues"),
            Some(json!({"project_key": "PRJ", "issue_type": "Task", "summary": "s"})),
        )
        .await;
    assert_eq!(r.status, 502, "{}", r.body);
    assert!(
        r.body["message"]
            .as_str()
            .unwrap()
            .contains("jira createmeta"),
        "{}",
        r.body
    );
}

/// A stand-in Confluence site: page create / update / read (with ancestors)
/// and footer comments, storing the storage body exactly as sent. `tamper`
/// makes reads return a body missing its last table row and a retitled page.
#[derive(Default)]
struct FakeWiki {
    content: HashMap<String, Value>,
    writes: usize,
    tamper: bool,
}

async fn fake_confluence() -> (String, Arc<Mutex<FakeWiki>>) {
    use axum::extract::{Path as P, State as St};
    use axum::routing::{get, post};
    type S = Arc<Mutex<FakeWiki>>;
    let state: S = Arc::default();
    let app = Router::new()
        .route(
            "/wiki/rest/api/content",
            post(|St(st): St<S>, axum::Json(b): axum::Json<Value>| async move {
                let mut st = st.lock().unwrap();
                st.writes += 1;
                let id = format!("{}", 100 + st.content.len());
                let stored = if b["type"] == "comment" {
                    json!({"id": id, "type": "comment", "container": {"id": b["container"]["id"]},
                           "body": b["body"], "_links": {"self": format!("/rest/api/content/{id}")}})
                } else {
                    json!({"id": id, "type": "page", "title": b["title"], "space": {"key": b["space"]["key"]},
                           "version": {"number": 1}, "ancestors": b.get("ancestors").cloned().unwrap_or(json!([])),
                           "body": b["body"], "_links": {"webui": format!("/spaces/X/pages/{id}")}})
                };
                st.content.insert(id, stored.clone());
                axum::Json(stored)
            }),
        )
        .route(
            "/wiki/rest/api/content/{id}/property",
            post(|| async { axum::Json(json!({})) }),
        )
        .route(
            "/wiki/rest/api/content/{id}",
            get(|St(st): St<S>, P(id): P<String>| async move {
                let st = st.lock().unwrap();
                let mut c = st.content.get(&id).cloned().unwrap_or(Value::Null);
                if st.tamper {
                    if let Some(v) = c["body"]["storage"]["value"].as_str() {
                        let cut = v.rfind("<tr>").map_or(v.to_string(), |n| {
                            let end = v[n..].find("</tr>").unwrap() + n + 5;
                            format!("{}{}", &v[..n], &v[end..])
                        });
                        c["body"]["storage"]["value"] = json!(cut);
                    }
                    if c["type"] == "page" {
                        c["title"] = json!("Someone else's title");
                    }
                }
                axum::Json(c)
            })
            .put(|St(st): St<S>, P(id): P<String>, axum::Json(b): axum::Json<Value>| async move {
                let mut st = st.lock().unwrap();
                st.writes += 1;
                let page = st.content.get_mut(&id).unwrap();
                page["title"] = b["title"].clone();
                page["body"] = b["body"].clone();
                page["version"] = b["version"].clone();
                axum::Json(page.clone())
            }),
        )
        .with_state(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let site = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (site, state)
}

const PAGE_MD: &str = "## Matrix\n\n| TC | Result |\n|---|---|\n| 1 | **ok** |\n| 2 | `fail` |\n\n```sql\nselect ']]>' from t\n```\n\n- [ ] rerun";

fn check_names(v: &Value) -> Vec<String> {
    v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn confluence_page_and_comment_writes_are_verified() {
    let (site, wiki) = fake_confluence().await;
    let a = app().await;
    let id = jira_at(&a, &site).await;
    let q = format!("account_id={id}");

    let r = a
        .call(
            &a.alice,
            "POST",
            &format!("/issue/confluence/pages?{q}"),
            Some(
                json!({"space_key": "ENG", "title": "Plan", "parent_id": "42", "body_md": PAGE_MD}),
            ),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let page_id = r.body["id"].as_str().unwrap().to_string();
    assert_eq!(r.body["title"], "Plan");
    let v = &r.body["verification"];
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(
        check_names(v),
        [
            "title",
            "space",
            "parent",
            "version",
            "body_structure",
            "body_text"
        ]
    );
    let structure = v["checks"][4]["detail"].as_str().unwrap();
    assert!(structure.contains("table[3x2,header]"), "{structure}");
    // What Confluence received: storage XHTML with a th header row, the code
    // macro with its language and the `]]>` split across CDATA sections.
    let sent = wiki.lock().unwrap().content[&page_id]["body"]["storage"]["value"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        sent.contains("<tr><th><p>TC</p></th><th><p>Result</p></th></tr>"),
        "{sent}"
    );
    assert!(
        sent.contains("<ac:parameter ac:name=\"language\">sql</ac:parameter>"),
        "{sent}"
    );
    assert!(sent.contains("select ']]]]><![CDATA[>' from t"), "{sent}");
    assert!(sent.contains("<ac:task-list>"), "{sent}");

    let r = a
        .call(
            &a.alice,
            "PUT",
            &format!("/issue/confluence/pages/{page_id}?{q}"),
            Some(json!({"body_md": "# Updated\n\n| a |\n|---|\n| b |", "base_version": 1})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let v = &r.body["verification"];
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(
        check_names(v),
        ["title", "space", "version", "body_structure", "body_text"]
    );
    assert_eq!(v["checks"][2]["detail"], "expected 2, got 2");

    let r = a
        .call(
            &a.alice,
            "POST",
            &format!("/issue/confluence/pages/{page_id}/comments?{q}"),
            Some(json!({"body_md": "Looks good:\n\n| k | v |\n|---|---|\n| a | 1 |"})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body["id"].is_string(), "{}", r.body);
    let v = &r.body["verification"];
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(check_names(v), ["container", "body_structure", "body_text"]);
}

#[tokio::test]
async fn confluence_write_mismatches_are_reported_not_undone() {
    let (site, wiki) = fake_confluence().await;
    wiki.lock().unwrap().tamper = true;
    let a = app().await;
    let id = jira_at(&a, &site).await;
    let r = a
        .call(
            &a.alice,
            "POST",
            &format!("/issue/confluence/pages?account_id={id}"),
            Some(json!({"space_key": "ENG", "title": "Plan", "body_md": PAGE_MD})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let v = &r.body["verification"];
    assert_eq!(v["ok"], false, "{v}");
    let failed: Vec<(String, String)> = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["ok"] == false)
        .map(|c| {
            (
                c["name"].as_str().unwrap().into(),
                c["detail"].as_str().unwrap().into(),
            )
        })
        .collect();
    let names: Vec<&str> = failed.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["title", "body_structure", "body_text"], "{v}");
    assert!(
        failed[1]
            .1
            .contains("sent table[3x2,header], stored table[2x2,header]"),
        "{}",
        failed[1].1
    );
    assert_eq!(wiki.lock().unwrap().content.len(), 1, "the page is kept");

    let page_id = r.body["id"].as_str().unwrap();
    let r = a
        .call(
            &a.alice,
            "POST",
            &format!("/issue/confluence/pages/{page_id}/comments?account_id={id}"),
            Some(json!({"body_md": "| k |\n|---|\n| a |"})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body["verification"]["ok"], false, "{}", r.body);
}

#[tokio::test]
async fn malformed_body_html_is_refused_before_anything_is_sent() {
    let (site, wiki) = fake_confluence().await;
    let a = app().await;
    let id = jira_at(&a, &site).await;
    for (method, path, body) in [
        (
            "POST",
            format!("/issue/confluence/pages?account_id={id}"),
            json!({"space_key": "ENG", "title": "T", "body_html": "<p>a<br>b</p>"}),
        ),
        (
            "POST",
            format!("/issue/confluence/pages/1/comments?account_id={id}"),
            json!({"body_html": "<p>R&D</p>"}),
        ),
    ] {
        let r = a.call(&a.alice, method, &path, Some(body)).await;
        assert_eq!(r.status, 400, "{path}: {}", r.body);
        assert!(
            r.body["message"]
                .as_str()
                .unwrap()
                .contains("not well-formed XHTML"),
            "{}",
            r.body
        );
    }
    // Well-formed caller storage with any macro still passes through as-is.
    let html = "<ac:structured-macro ac:name=\"toc\" /><p>R&amp;D</p>";
    let r = a
        .call(
            &a.alice,
            "POST",
            &format!("/issue/confluence/pages?account_id={id}"),
            Some(json!({"space_key": "ENG", "title": "T", "body_html": html})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body["verification"]["ok"], true, "{}", r.body);
    let w = wiki.lock().unwrap();
    assert_eq!(w.writes, 1, "only the valid body was sent");
    assert_eq!(
        w.content.values().next().unwrap()["body"]["storage"]["value"],
        html
    );
}
