//! Wiremock coverage for the round-2 PR features: "review requested from
//! you" (each forge's own filter, or GitHub's `requested_reviewers` rows) and
//! adding / removing reviewers on an existing PR.

use serde_json::json;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{GitProvider, RemoteRef};

fn rr(owner: &str) -> RemoteRef {
    RemoteRef {
        owner: owner.into(),
        repo: "app".into(),
    }
}

async fn ok_json(server: &MockServer, verb: &str, at: &str, body: serde_json::Value) {
    Mock::given(method(verb))
        .and(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn gh_pr(number: u64, reviewers: serde_json::Value) -> serde_json::Value {
    json!({
        "number": number, "title": format!("pr {number}"), "state": "open",
        "user": { "login": "author", "id": 1 },
        "head": { "ref": "feat", "sha": "abc1234def" }, "base": { "ref": "main" },
        "updated_at": "2026-11-12T10:00:00Z", "html_url": "https://x/1",
        "requested_reviewers": reviewers,
    })
}

#[tokio::test]
async fn github_review_requested_keeps_only_rows_that_name_the_account() {
    let server = MockServer::start().await;
    ok_json(&server, "GET", "/user", json!({ "login": "me", "id": 7 })).await;
    ok_json(
        &server,
        "GET",
        "/repos/acme/app/pulls",
        json!([
            gh_pr(1, json!([{ "login": "me", "id": 7 }])),
            gh_pr(2, json!([{ "login": "bob", "id": 2 }])),
            gh_pr(3, json!([])),
        ]),
    )
    .await;
    let gh = super::github::Github::with_base("tok".into(), server.uri());
    let page = gh.list_review_requested(&rr("acme"), 1, 50).await.unwrap();
    let numbers: Vec<u64> = page.items.iter().map(|p| p.number).collect();
    assert_eq!(numbers, vec![1]);
    assert!(!page.has_more);
}

#[tokio::test]
async fn github_review_requested_without_an_identity_is_a_conflict() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    ok_json(&server, "GET", "/repos/acme/app/pulls", json!([])).await;
    let gh = super::github::Github::with_base("tok".into(), server.uri());
    let err = gh
        .list_review_requested(&rr("acme"), 1, 50)
        .await
        .expect_err("no identity, no list");
    assert!(matches!(err, otto_core::Error::Conflict(_)), "{err}");
}

#[tokio::test]
async fn bitbucket_review_requested_filters_by_the_uuid() {
    let server = MockServer::start().await;
    ok_json(&server, "GET", "/user", json!({ "uuid": "{U-1}" })).await;
    Mock::given(method("GET"))
        .and(path("/repositories/acme/app/pullrequests"))
        .and(query_param(
            "q",
            r#"reviewers.uuid="{u-1}" AND state="OPEN""#,
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "values": [], "next": "x" })),
        )
        .mount(&server)
        .await;
    let bb = super::bitbucket::Bitbucket::with_base("u".into(), "tok".into(), server.uri());
    let page = bb.list_review_requested(&rr("acme"), 1, 50).await.unwrap();
    assert!(page.items.is_empty());
    assert!(page.has_more, "Bitbucket's `next` cursor is the signal");
}

#[tokio::test]
async fn gitlab_review_requested_sends_reviewer_id() {
    let server = MockServer::start().await;
    ok_json(&server, "GET", "/api/v4/user", json!({ "id": 42 })).await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/acme%2Fapp/merge_requests"))
        .and(query_param("reviewer_id", "42"))
        .and(query_param("state", "opened"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    let gl = super::gitlab::Gitlab::new("tok".into(), Some(server.uri()));
    let page = gl.list_review_requested(&rr("acme"), 1, 50).await.unwrap();
    assert!(page.items.is_empty());
}

#[tokio::test]
async fn github_set_reviewers_posts_additions_and_deletes_removals() {
    let server = MockServer::start().await;
    let at = "/repos/acme/app/pulls/5/requested_reviewers";
    Mock::given(method("POST"))
        .and(path(at))
        .and(body_json(json!({ "reviewers": ["ann"] })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "number": 5 })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(at))
        .and(body_json(json!({ "reviewers": ["bob"] })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "number": 5 })))
        .expect(1)
        .mount(&server)
        .await;
    let gh = super::github::Github::with_base("tok".into(), server.uri());
    gh.set_reviewers(&rr("acme"), 5, &["ann".into()], &["bob".into()])
        .await
        .unwrap();
}

#[tokio::test]
async fn bitbucket_set_reviewers_puts_the_whole_new_list() {
    let server = MockServer::start().await;
    // An owner of its own: the workspace-member list is memoised per process.
    let pr = "/repositories/acme-round2/app/pullrequests/9";
    ok_json(
        &server,
        "GET",
        pr,
        json!({ "reviewers": [
            { "uuid": "{a}", "display_name": "Ann Lee", "nickname": "ann" },
            { "uuid": "{b}", "display_name": "Bob", "nickname": "bob" },
        ] }),
    )
    .await;
    ok_json(
        &server,
        "GET",
        "/workspaces/acme-round2/members",
        json!({ "values": [{ "user": { "uuid": "{c}", "nickname": "carl", "display_name": "Carl" } }] }),
    )
    .await;
    Mock::given(method("PUT"))
        .and(path(pr))
        .and(body_json(
            json!({ "reviewers": [{ "uuid": "{a}" }, { "uuid": "{c}" }] }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 9 })))
        .expect(1)
        .mount(&server)
        .await;
    let bb = super::bitbucket::Bitbucket::with_base("u".into(), "tok".into(), server.uri());
    bb.set_reviewers(&rr("acme-round2"), 9, &["carl".into()], &["Bob".into()])
        .await
        .unwrap();
}

#[tokio::test]
async fn bitbucket_set_reviewers_refuses_an_unknown_name_before_writing() {
    let server = MockServer::start().await;
    let pr = "/repositories/acme-round2b/app/pullrequests/9";
    ok_json(&server, "GET", pr, json!({ "reviewers": [] })).await;
    Mock::given(method("PUT"))
        .and(path(pr))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let bb = super::bitbucket::Bitbucket::with_base("u".into(), "tok".into(), server.uri());
    let err = bb
        .set_reviewers(&rr("acme-round2b"), 9, &[], &["nobody".into()])
        .await
        .expect_err("unknown reviewer");
    assert!(err.to_string().contains("nobody"), "{err}");
}
