//! Wiremock coverage for the three provider clients and the shared [`Http`]
//! layer: real pagination signals (`Link rel="next"` / `x-next-page` / `next`),
//! rate-limit classification with `Retry-After`, and Bitbucket's
//! comment-before-request-changes ordering.
//!
//! Every client is pointed at a local `MockServer` through the `with_base`
//! constructors (GitLab already takes a base), so nothing here touches the
//! network — an unmocked hop is a 404 from the stub, never a real request.

use otto_core::api::PrState;
use serde_json::json;
use wiremock::matchers::{method, path, path_regex, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{GitProvider, RemoteRef};

fn rr() -> RemoteRef {
    RemoteRef {
        owner: "acme".into(),
        repo: "app".into(),
    }
}

// ---------------------------------------------------------------------------
// GitHub
// ---------------------------------------------------------------------------

mod github {
    use super::*;
    use crate::providers::github::Github;

    fn pr(number: u64) -> serde_json::Value {
        json!({
            "number": number,
            "title": format!("PR {number}"),
            "state": "open",
            "user": { "login": "dev" },
            "head": { "ref": "feat", "sha": "deadbeef" },
            "base": { "ref": "main" },
            "updated_at": "2026-09-01T10:00:00Z",
            "html_url": format!("https://github.com/acme/app/pull/{number}"),
        })
    }

    #[tokio::test]
    async fn list_prs_two_pages_sets_has_more() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());

        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .and(query_param("page", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(
                        "link",
                        format!(r#"<{}/repos/acme/app/pulls?page=2>; rel="next""#, server.uri())
                            .as_str(),
                    )
                    .set_body_json(json!([pr(1), pr(2)])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([pr(3)])))
            .mount(&server)
            .await;

        let p1 = gh.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap();
        assert!(p1.has_more, "a Link rel=next means another page exists");
        assert_eq!(p1.items.len(), 2);
        assert_eq!(p1.items[0].number, 1);

        let p2 = gh.list_prs(&rr(), PrState::Open, 2, 50).await.unwrap();
        assert!(!p2.has_more, "no Link header ⇒ last page");
        assert_eq!(p2.items.len(), 1);
        assert_eq!(p2.items[0].number, 3);
        // One request per page — no hidden 20-page sweep.
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn get_pr_comments_span_two_pages() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());

        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pr(7)))
            .mount(&server)
            .await;
        // Page 1 carries the `next` link; page 2 (no `per_page`) ends it.
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/issues/7/comments"))
            .and(query_param("per_page", "100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(
                        "link",
                        format!(
                            r#"<{}/repos/acme/app/issues/7/comments?page=2>; rel="next""#,
                            server.uri()
                        )
                        .as_str(),
                    )
                    .set_body_json(json!([{
                        "id": 1, "body": "first", "user": {"login": "dev"},
                        "created_at": "2026-09-01T10:00:00Z"
                    }])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/issues/7/comments"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 2, "body": "second", "user": {"login": "dev"},
                "created_at": "2026-09-01T11:00:00Z"
            }])))
            .mount(&server)
            .await;
        // Inline comments + reviews are empty; CI/check-runs is left unmocked
        // (404 → `CiStatus::none`), which is exactly the best-effort path.
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7/reviews"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let detail = gh.get_pr(&rr(), 7).await.unwrap();
        let bodies: Vec<&str> = detail.comments.iter().map(|c| c.body.as_str()).collect();
        assert_eq!(bodies, vec!["first", "second"], "both pages must be present");
    }

    #[tokio::test]
    async fn forbidden_with_ratelimit_zero_retries_once_then_succeeds() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());

        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("retry-after", "1")
                    .set_body_json(json!({"message": "API rate limit exceeded"})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([pr(1)])))
            .mount(&server)
            .await;

        let page = gh.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            2,
            "a short rate limit is slept off and retried exactly once"
        );
    }

    #[tokio::test]
    async fn too_many_requests_long_wait_is_upstream_no_retry() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());

        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "120")
                    .set_body_json(json!({"message": "slow down"})),
            )
            .mount(&server)
            .await;

        let err = gh.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("rate limited"), "got {msg}");
        assert!(msg.contains("120s"), "the wait is quoted back: {msg}");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "a two-minute wait is reported, never slept off"
        );
    }
}

// ---------------------------------------------------------------------------
// GitLab
// ---------------------------------------------------------------------------

mod gitlab {
    use super::*;
    use crate::providers::gitlab::Gitlab;

    fn mr(iid: u64) -> serde_json::Value {
        json!({
            "iid": iid,
            "title": format!("MR {iid}"),
            "state": "opened",
            "author": { "name": "Dev" },
            "source_branch": "feat",
            "target_branch": "main",
            "updated_at": "2026-09-01T10:00:00Z",
            "web_url": format!("https://gitlab.com/acme/app/-/merge_requests/{iid}"),
            "merge_status": "can_be_merged",
        })
    }

    #[tokio::test]
    async fn list_prs_two_pages_sets_has_more() {
        let server = MockServer::start().await;
        let gl = Gitlab::new("tok".into(), Some(server.uri()));

        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v4/projects/.+/merge_requests$"))
            .and(query_param("page", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-next-page", "2")
                    .set_body_json(json!([mr(1)])),
            )
            .mount(&server)
            .await;
        // Last page: GitLab sends the header EMPTY rather than omitting it.
        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v4/projects/.+/merge_requests$"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-next-page", "")
                    .set_body_json(json!([mr(2)])),
            )
            .mount(&server)
            .await;

        let p1 = gl.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap();
        assert!(p1.has_more, "x-next-page: 2 ⇒ another page");
        assert_eq!(p1.items[0].number, 1);

        let p2 = gl.list_prs(&rr(), PrState::Open, 2, 50).await.unwrap();
        assert!(!p2.has_more, "an EMPTY x-next-page is the last page");
        assert_eq!(p2.items[0].number, 2);
    }

    #[tokio::test]
    async fn get_pr_comments_span_two_pages() {
        let server = MockServer::start().await;
        let gl = Gitlab::new("tok".into(), Some(server.uri()));

        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v4/projects/.+/merge_requests/7$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mr(7)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v4/projects/.+/merge_requests/7/discussions$"))
            .and(query_param("per_page", "100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(
                        "link",
                        format!(
                            r#"<{}/api/v4/projects/acme%2Fapp/merge_requests/7/discussions?page=2>; rel="next""#,
                            server.uri()
                        )
                        .as_str(),
                    )
                    .set_body_json(json!([{
                        "id": "d1",
                        "notes": [{"id": 1, "body": "first", "author": {"name": "Dev"},
                                   "created_at": "2026-09-01T10:00:00Z", "system": false}]
                    }])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v4/projects/.+/merge_requests/7/discussions$"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": "d2",
                "notes": [{"id": 2, "body": "second", "author": {"name": "Dev"},
                           "created_at": "2026-09-01T11:00:00Z", "system": false}]
            }])))
            .mount(&server)
            .await;

        let detail = gl.get_pr(&rr(), 7).await.unwrap();
        let bodies: Vec<&str> = detail.comments.iter().map(|c| c.body.as_str()).collect();
        assert_eq!(bodies, vec!["first", "second"]);
    }
}

// ---------------------------------------------------------------------------
// Bitbucket
// ---------------------------------------------------------------------------

mod bitbucket {
    use super::*;
    use crate::providers::bitbucket::Bitbucket;

    fn pr(id: u64) -> serde_json::Value {
        json!({
            "id": id,
            "title": format!("PR {id}"),
            "state": "OPEN",
            "author": { "display_name": "Dev" },
            "source": { "branch": { "name": "feat" } },
            "destination": { "branch": { "name": "main" } },
            "updated_on": "2026-09-01T10:00:00Z",
            "links": { "html": { "href": format!("https://bitbucket.org/acme/app/pull-requests/{id}") } },
        })
    }

    fn comment(id: u64, body: &str) -> serde_json::Value {
        json!({
            "id": id,
            "content": { "raw": body },
            "user": { "display_name": "Dev" },
            "created_on": "2026-09-01T10:00:00Z",
        })
    }

    #[tokio::test]
    async fn list_prs_two_pages_sets_has_more() {
        let server = MockServer::start().await;
        let bb = Bitbucket::with_base("user".into(), "tok".into(), server.uri());

        Mock::given(method("GET"))
            .and(path("/repositories/acme/app/pullrequests"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "values": [pr(1)],
                "next": format!("{}/repositories/acme/app/pullrequests?page=2", server.uri()),
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repositories/acme/app/pullrequests"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "values": [pr(2)] })))
            .mount(&server)
            .await;

        let p1 = bb.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap();
        assert!(p1.has_more, "a `next` cursor ⇒ another page");
        assert_eq!(p1.items[0].number, 1);

        let p2 = bb.list_prs(&rr(), PrState::Open, 2, 50).await.unwrap();
        assert!(!p2.has_more, "no `next` ⇒ last page");
        assert_eq!(p2.items[0].number, 2);
    }

    #[tokio::test]
    async fn get_pr_comments_span_two_pages() {
        let server = MockServer::start().await;
        let bb = Bitbucket::with_base("user".into(), "tok".into(), server.uri());

        Mock::given(method("GET"))
            .and(path("/repositories/acme/app/pullrequests/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pr(7)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repositories/acme/app/pullrequests/7/comments"))
            .and(query_param("pagelen", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "values": [comment(1, "first")],
                "next": format!(
                    "{}/repositories/acme/app/pullrequests/7/comments?page=2",
                    server.uri()
                ),
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repositories/acme/app/pullrequests/7/comments"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "values": [comment(2, "second")] })),
            )
            .mount(&server)
            .await;

        let detail = bb.get_pr(&rr(), 7).await.unwrap();
        let bodies: Vec<&str> = detail.comments.iter().map(|c| c.body.as_str()).collect();
        assert_eq!(bodies, vec!["first", "second"]);
    }

    #[tokio::test]
    async fn request_changes_with_body_comments_first() {
        let server = MockServer::start().await;
        let bb = Bitbucket::with_base("user".into(), "tok".into(), server.uri());

        Mock::given(method("POST"))
            .and(path("/repositories/acme/app/pullrequests/7/comments"))
            .respond_with(ResponseTemplate::new(201).set_body_json(comment(9, "please fix")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repositories/acme/app/pullrequests/7/request-changes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        bb.request_changes(&rr(), 7, Some("please fix")).await.unwrap();

        let reqs = server.received_requests().await.unwrap();
        let paths: Vec<&str> = reqs.iter().map(|r| r.url.path()).collect();
        assert_eq!(
            paths,
            vec![
                "/repositories/acme/app/pullrequests/7/comments",
                "/repositories/acme/app/pullrequests/7/request-changes",
            ],
            "the reasoning is posted BEFORE the verdict, or it is lost"
        );
    }

    #[tokio::test]
    async fn request_changes_without_body_posts_no_comment() {
        let server = MockServer::start().await;
        let bb = Bitbucket::with_base("user".into(), "tok".into(), server.uri());

        Mock::given(method("POST"))
            .and(path("/repositories/acme/app/pullrequests/7/request-changes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        // Whitespace-only is "no body" — an empty comment helps nobody.
        bb.request_changes(&rr(), 7, Some("   ")).await.unwrap();
        bb.request_changes(&rr(), 7, None).await.unwrap();

        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2);
        assert!(reqs.iter().all(|r| r.url.path().ends_with("/request-changes")));
    }
}

// ---------------------------------------------------------------------------
// Shared Http layer
// ---------------------------------------------------------------------------

mod client {
    use super::*;
    use crate::providers::client::Http;

    /// The ETag/TTL cache bypasses `send`, so it needs its own proof that a
    /// quota refusal is slept off and retried rather than surfaced as a 403.
    #[tokio::test]
    async fn get_cached_gets_rate_limit_treatment() {
        let server = MockServer::start().await;
        let http = Http::new("github");

        Mock::given(method("GET"))
            .and(path("/user/repos"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("retry-after", "1")
                    .set_body_json(json!({"message": "API rate limit exceeded"})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/user/repos"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .mount(&server)
            .await;

        let rb = http
            .client()
            .get(format!("{}/user/repos", server.uri()))
            .bearer_auth("tok");
        let body = http.get_cached(rb).await.unwrap();
        assert_eq!(body, "[]");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            2,
            "the cached-GET hop retries the short rate limit too"
        );
    }

    #[tokio::test]
    async fn get_cached_long_rate_limit_is_upstream() {
        let server = MockServer::start().await;
        let http = Http::new("gitlab");

        Mock::given(method("GET"))
            .and(path("/projects"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "90"))
            .mount(&server)
            .await;

        let rb = http
            .client()
            .get(format!("{}/projects", server.uri()))
            .header("PRIVATE-TOKEN", "tok");
        let err = http.get_cached(rb).await.unwrap_err();
        assert!(err.to_string().contains("rate limited"), "got {err}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
