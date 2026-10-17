//! The issues router end to end: account CRUD against a real (in-memory)
//! state DB and secret store, the per-account ownership gate on EVERY route
//! that acts with an account's credentials, and the provider gates that must
//! refuse before any upstream call (a JQL string never reaches Linear, a
//! Linear key never reaches Confluence).
//!
//! Requests that pass every gate would go upstream, so the Jira accounts
//! here point at a closed loopback port and the Linear-gated cases are only
//! ones the router refuses locally — nothing in this file touches the network.

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
