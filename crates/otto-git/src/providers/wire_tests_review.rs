//! Wiremock coverage for the PR review surface: who "you" are (stable-id
//! identity → `is_me` / `is_mine` / `can_edit`), Bitbucket's nested reply
//! chains, GitHub review summaries and requested reviewers, review-thread
//! paging, replies to general comments, comment edit/delete, withdrawing an
//! approval or change request, and Bitbucket's whitespace-ignoring diffs.

use serde_json::json;
use wiremock::matchers::{body_partial_json, body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{GitProvider, RemoteRef};

fn rr() -> RemoteRef {
    RemoteRef {
        owner: "acme".into(),
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

async fn hits(server: &MockServer, verb: &str, at: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == at)
        .count()
}

// ---------------------------------------------------------------------------
// Bitbucket
// ---------------------------------------------------------------------------

mod bitbucket {
    use super::*;
    use crate::providers::bitbucket::{thread_comments, Bitbucket};

    const PR: &str = "/repositories/acme/app/pullrequests/7";

    /// A PR whose participants name the owner by DISPLAY name — which never
    /// equals the account's email username — but carry their uuid.
    async fn mount_pr(server: &MockServer) {
        ok_json(
            server,
            "GET",
            PR,
            json!({
                "id": 7, "title": "PR 7", "state": "OPEN",
                "author": { "display_name": "Itzik A.", "uuid": "{U-ME}" },
                "source": { "branch": { "name": "feat" } },
                "destination": { "branch": { "name": "main" } },
                "updated_on": "2026-09-01T10:00:00Z",
                "participants": [
                    { "user": { "display_name": "Itzik A.", "uuid": "{u-me}" },
                      "role": "REVIEWER", "approved": true, "state": "approved" },
                    { "user": { "display_name": "Other", "uuid": "{u-2}" },
                      "role": "REVIEWER", "approved": false, "state": null },
                    // A mere commenter is not a reviewer…
                    { "user": { "display_name": "Talker", "uuid": "{u-3}" },
                      "role": "PARTICIPANT", "approved": false, "state": null },
                    // …but a participant's verdict still counts.
                    { "user": { "display_name": "Drive-by", "uuid": "{u-4}" },
                      "role": "PARTICIPANT", "approved": true, "state": "approved" }
                ]
            }),
        )
        .await;
        ok_json(
            server,
            "GET",
            &format!("{PR}/comments"),
            json!({ "values": [
                { "id": 1, "content": { "raw": "mine" }, "created_on": "2026-09-01T10:00:00Z",
                  "user": { "display_name": "Itzik A.", "uuid": "{U-ME}" } },
                { "id": 2, "content": { "raw": "theirs" }, "created_on": "2026-09-01T11:00:00Z",
                  "user": { "display_name": "Itzik A.", "uuid": "{u-2}" } }
            ] }),
        )
        .await;
        ok_json(
            server,
            "GET",
            &format!("{PR}/statuses"),
            json!({ "values": [] }),
        )
        .await;
    }

    #[tokio::test]
    async fn reviewer_is_me_by_uuid_even_when_names_differ() {
        let server = MockServer::start().await;
        mount_pr(&server).await;
        ok_json(
            &server,
            "GET",
            "/user",
            json!({ "uuid": "{u-me}", "account_id": "acc-1", "display_name": "Someone Else" }),
        )
        .await;
        let bb = Bitbucket::with_base("me@example.com".into(), "tok".into(), server.uri());
        let d = bb.get_pr(&rr(), 7).await.unwrap();
        let got: Vec<_> = d
            .reviewers
            .iter()
            .map(|r| (r.name.as_str(), r.is_me))
            .collect();
        assert_eq!(
            got,
            [("Itzik A.", true), ("Other", false), ("Drive-by", false)],
            "commenter dropped, approver kept, `you` by uuid"
        );
        assert!(d.summary.is_mine, "author uuid matches (case-insensitive)");
        let edit: Vec<_> = d
            .comments
            .iter()
            .map(|c| (c.id.as_str(), c.can_edit))
            .collect();
        // Comment 2's author shares the display name, not the uuid.
        assert_eq!(edit, [("1", true), ("2", false)]);
    }

    #[tokio::test]
    async fn a_different_uuid_is_not_me() {
        let server = MockServer::start().await;
        mount_pr(&server).await;
        ok_json(
            &server,
            "GET",
            "/user",
            json!({ "uuid": "{u-9}", "display_name": "Itzik A." }),
        )
        .await;
        let bb = Bitbucket::with_base("me@example.com".into(), "tok".into(), server.uri());
        let d = bb.get_pr(&rr(), 7).await.unwrap();
        assert!(
            d.reviewers.iter().all(|r| !r.is_me),
            "a display-name match is not you"
        );
        assert!(!d.summary.is_mine);
    }

    #[tokio::test]
    async fn identity_lookup_failure_marks_nobody_and_the_pr_still_loads() {
        let server = MockServer::start().await;
        mount_pr(&server).await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(403).set_body_json(json!({"error": {"message": "scope"}})),
            )
            .mount(&server)
            .await;
        let bb = Bitbucket::with_base("me@example.com".into(), "tok".into(), server.uri());
        let d = bb
            .get_pr(&rr(), 7)
            .await
            .expect("the PR loads without an identity");
        assert_eq!(d.reviewers.len(), 3);
        assert!(d.reviewers.iter().all(|r| !r.is_me));
        assert!(!d.summary.is_mine);
        assert!(d.comments.iter().all(|c| !c.can_edit));
        let page = bb
            .list_prs(&rr(), otto_core::api::PrState::Open, 1, 50)
            .await;
        // The list's own read is unmocked here (404): its error is the list's,
        // never the identity's.
        assert!(page.is_err());
    }

    /// A 3-level reply chain stays ONE thread under its root (chronological,
    /// each reply keeping whom it answered); an orphan heads its own thread
    /// without a resolve id; a deleted root's replies keep the root's id.
    #[test]
    fn nested_replies_thread_under_their_root() {
        let c = |id: u64, parent: Option<u64>, at: &str| {
            let mut v = json!({
                "id": id, "content": { "raw": format!("c{id}") },
                "user": { "display_name": "Dev" }, "created_on": at,
            });
            if let Some(p) = parent {
                v["parent"] = json!({ "id": p });
            }
            v
        };
        let mut deleted_root = c(6, None, "2026-09-01T09:00:00Z");
        deleted_root["deleted"] = json!(true);
        deleted_root["resolution"] = json!({ "date": "2026-09-02T00:00:00Z" });
        let values = vec![
            c(1, None, "2026-09-01T10:00:00Z"),
            c(3, Some(2), "2026-09-01T10:20:00Z"),
            c(2, Some(1), "2026-09-01T10:10:00Z"),
            c(4, Some(3), "2026-09-01T10:30:00Z"),
            c(5, Some(99), "2026-09-01T11:00:00Z"),
            deleted_root,
            c(7, Some(6), "2026-09-01T12:00:00Z"),
        ];
        let threads = thread_comments(&values, None);
        let heads: Vec<_> = threads
            .iter()
            .map(|t| (t.id.as_str(), t.thread_id.as_deref()))
            .collect();
        assert_eq!(
            heads,
            [("1", Some("1")), ("5", None), ("7", Some("6"))],
            "one thread per real root; the orphan is not resolvable"
        );
        let replies: Vec<_> = threads[0]
            .replies
            .iter()
            .map(|r| (r.id.as_str(), r.in_reply_to.as_deref()))
            .collect();
        assert_eq!(
            replies,
            [("2", Some("1")), ("3", Some("2")), ("4", Some("3"))]
        );
        assert!(threads[0].in_reply_to.is_none());
        assert!(threads[2].resolved, "the deleted root's resolution holds");
        assert!(
            threads.iter().all(|t| t.id != "6"),
            "a deleted comment is never shown"
        );
    }

    #[tokio::test]
    async fn withdraw_approval_and_change_request_are_deletes() {
        let server = MockServer::start().await;
        for tail in ["approve", "request-changes"] {
            Mock::given(method("DELETE"))
                .and(path(format!("{PR}/{tail}")))
                .respond_with(ResponseTemplate::new(204))
                .mount(&server)
                .await;
        }
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        bb.unapprove(&rr(), 7).await.unwrap();
        bb.withdraw_request_changes(&rr(), 7).await.unwrap();
        assert_eq!(hits(&server, "DELETE", &format!("{PR}/approve")).await, 1);
        assert_eq!(
            hits(&server, "DELETE", &format!("{PR}/request-changes")).await,
            1
        );
    }

    #[tokio::test]
    async fn edit_and_delete_comment_request_shape() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(format!("{PR}/comments/9")))
            .and(body_partial_json(
                json!({ "content": { "raw": "fixed text" } }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": 9, "content": { "raw": "fixed text" }, "user": { "display_name": "Dev" },
                "created_on": "2026-09-01T10:00:00Z"
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(format!("{PR}/comments/9")))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        let c = bb.edit_comment(&rr(), 7, "9", "fixed text").await.unwrap();
        assert_eq!((c.body.as_str(), c.can_edit), ("fixed text", true));
        bb.delete_comment(&rr(), 7, "9").await.unwrap();
        assert_eq!(
            hits(&server, "DELETE", &format!("{PR}/comments/9")).await,
            1
        );
        // A non-numeric id never reaches the forge.
        assert!(bb.delete_comment(&rr(), 7, "../9").await.is_err());
    }

    #[tokio::test]
    async fn whitespace_twin_sends_ignore_whitespace_on_every_diff_read() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("{PR}/diff")))
            .and(query_param("ignore_whitespace", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{PR}/diffstat")))
            .and(query_param("ignore_whitespace", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "values": [] })))
            .mount(&server)
            .await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        let ws = bb
            .ignoring_whitespace()
            .expect("Bitbucket ignores whitespace");
        ws.get_pr_diff(&rr(), 7).await.unwrap();
        ws.get_pr_diff_summary(&rr(), 7).await.unwrap();
        ws.get_pr_file_diff(&rr(), 7, "a.rs", None).await.unwrap();
        assert_eq!(hits(&server, "GET", &format!("{PR}/diff")).await, 2);
        assert_eq!(hits(&server, "GET", &format!("{PR}/diffstat")).await, 1);
        // The plain client never asks for it.
        assert!(
            bb.get_pr_diff(&rr(), 7).await.is_err(),
            "unmocked without the flag"
        );
    }
}

// ---------------------------------------------------------------------------
// GitHub
// ---------------------------------------------------------------------------

mod github {
    use super::*;
    use crate::providers::github::{own_review_to_dismiss, quote_reply, Github};
    use crate::providers::identity::Me;
    use otto_core::api::NewPrCommentReq;
    use otto_core::Error;

    const PR: &str = "/repos/acme/app/pulls/7";

    fn reply(to: &str) -> NewPrCommentReq {
        NewPrCommentReq {
            body: "agreed".into(),
            path: None,
            line: None,
            in_reply_to: Some(to.into()),
            side: None,
            old_line: None,
            commit_id: None,
        }
    }

    async fn mount_detail(server: &MockServer, reviews: serde_json::Value) {
        ok_json(
            server,
            "GET",
            PR,
            json!({
                "number": 7, "title": "PR 7", "state": "open",
                "user": { "login": "Dev", "id": 2 },
                "head": { "ref": "feat", "sha": "deadbeef" }, "base": { "ref": "main" },
                "updated_at": "2026-09-01T10:00:00Z",
                "requested_reviewers": [
                    { "login": "silent", "id": 9 },
                    { "login": "rev", "id": 3 }
                ]
            }),
        )
        .await;
        ok_json(
            server,
            "GET",
            "/repos/acme/app/issues/7/comments",
            json!([]),
        )
        .await;
        ok_json(server, "GET", &format!("{PR}/comments"), json!([])).await;
        ok_json(server, "GET", &format!("{PR}/reviews"), reviews).await;
    }

    #[tokio::test]
    async fn review_summaries_show_and_requested_reviewers_are_pending() {
        let server = MockServer::start().await;
        mount_detail(
            &server,
            json!([
                { "id": 5, "user": { "login": "rev", "id": 3 }, "state": "CHANGES_REQUESTED",
                  "body": "Request changes: the retry loop never ends",
                  "submitted_at": "2026-09-01T12:00:00Z" },
                { "id": 6, "user": { "login": "dev2", "id": 4 }, "state": "APPROVED", "body": "",
                  "submitted_at": "2026-09-01T13:00:00Z" },
                { "id": 7, "user": { "login": "rev", "id": 3 }, "state": "PENDING",
                  "body": "half-written" }
            ]),
        )
        .await;
        ok_json(&server, "GET", "/user", json!({ "login": "REV", "id": 3 })).await;
        let d = Github::with_base("tok".into(), server.uri())
            .get_pr(&rr(), 7)
            .await
            .unwrap();
        let summaries: Vec<_> = d
            .comments
            .iter()
            .map(|c| (c.id.as_str(), c.review_state.as_deref(), c.can_edit))
            .collect();
        assert_eq!(
            summaries,
            [("review-5", Some("changes_requested"), false)],
            "non-empty submitted bodies only; never editable from Otto"
        );
        let reviewers: Vec<_> = d
            .reviewers
            .iter()
            .map(|r| (r.name.as_str(), r.changes_requested, r.is_me))
            .collect();
        assert_eq!(
            reviewers,
            [
                ("rev", true, true),
                ("dev2", false, false),
                ("silent", false, false)
            ],
            "login matched case-insensitively; a requested-but-silent reviewer is listed"
        );
        assert!(!d.summary.is_mine);
    }

    #[tokio::test]
    async fn review_threads_page_through_the_cursor() {
        let server = MockServer::start().await;
        ok_json(
            &server,
            "GET",
            PR,
            json!({ "number": 7, "state": "open", "head": { "sha": "deadbeef" } }),
        )
        .await;
        ok_json(
            &server,
            "GET",
            "/repos/acme/app/issues/7/comments",
            json!([]),
        )
        .await;
        ok_json(&server, "GET", &format!("{PR}/reviews"), json!([])).await;
        let inline = |id: u64| {
            json!({ "id": id, "body": "nit", "user": { "login": "rev" }, "path": "a.rs",
                    "line": 3, "created_at": "2026-09-01T10:00:00Z" })
        };
        ok_json(
            &server,
            "GET",
            &format!("{PR}/comments"),
            json!([inline(11), inline(12)]),
        )
        .await;
        let page = |id: &str, db: u64, next: bool| {
            json!({ "data": { "repository": { "pullRequest": { "reviewThreads": {
                "pageInfo": { "hasNextPage": next, "endCursor": if next { "c1" } else { "c2" } },
                "nodes": [ { "id": id, "isResolved": true,
                             "comments": { "nodes": [ { "databaseId": db } ] } } ]
            } } } } })
        };
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("\"after\":\"c1\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(page("T2", 12, false)))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page("T1", 11, true)))
            .with_priority(2)
            .mount(&server)
            .await;
        let d = Github::with_base("tok".into(), server.uri())
            .get_pr(&rr(), 7)
            .await
            .unwrap();
        let threads: Vec<_> = d
            .comments
            .iter()
            .map(|c| (c.id.as_str(), c.thread_id.as_deref(), c.resolved))
            .collect();
        assert_eq!(
            threads,
            [("11", Some("T1"), true), ("12", Some("T2"), true)]
        );
        assert_eq!(hits(&server, "POST", "/graphql").await, 2);
    }

    #[tokio::test]
    async fn reply_to_a_general_comment_is_a_quoting_issue_comment() {
        let server = MockServer::start().await;
        // 11 is an issue comment on this PR (not a review comment).
        ok_json(
            &server,
            "GET",
            "/repos/acme/app/issues/comments/11",
            json!({ "id": 11, "body": "Why a new crate?\n\nSecond para", "user": { "login": "ann" },
                    "issue_url": "https://api.github.com/repos/acme/app/issues/7" }),
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/app/issues/7/comments"))
            .and(body_string_contains("> @ann wrote:"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": 12, "body": "quoted", "user": { "login": "me" },
                "created_at": "2026-09-01T10:00:00Z"
            })))
            .mount(&server)
            .await;
        let gh = Github::with_base("tok".into(), server.uri());
        let c = gh.comment(&rr(), 7, &reply("11")).await.unwrap();
        assert_eq!((c.id.as_str(), c.can_edit), ("12", true));
        assert_eq!(hits(&server, "POST", &format!("{PR}/comments")).await, 0);

        // A review-comment parent still threads with `in_reply_to`.
        ok_json(
            &server,
            "GET",
            "/repos/acme/app/pulls/comments/21",
            json!({ "id": 21, "pull_request_url": "https://api.github.com/repos/acme/app/pulls/7" }),
        )
        .await;
        Mock::given(method("POST"))
            .and(path(format!("{PR}/comments")))
            .and(body_partial_json(json!({ "in_reply_to": 21 })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": 22, "body": "agreed", "user": { "login": "me" }, "path": "a.rs", "line": 3,
                "created_at": "2026-09-01T10:00:00Z"
            })))
            .mount(&server)
            .await;
        let c = gh.comment(&rr(), 7, &reply("21")).await.unwrap();
        assert_eq!(c.path.as_deref(), Some("a.rs"));
    }

    #[tokio::test]
    async fn edit_and_delete_pick_the_comment_s_own_endpoint() {
        let server = MockServer::start().await;
        ok_json(
            &server,
            "GET",
            "/repos/acme/app/issues/comments/11",
            json!({ "id": 11, "issue_url": "https://api.github.com/repos/acme/app/issues/7" }),
        )
        .await;
        ok_json(
            &server,
            "GET",
            "/repos/acme/app/pulls/comments/21",
            json!({ "id": 21, "pull_request_url": "https://api.github.com/repos/acme/app/pulls/7" }),
        )
        .await;
        Mock::given(method("PATCH"))
            .and(path("/repos/acme/app/issues/comments/11"))
            .and(body_partial_json(json!({ "body": "better" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": 11, "body": "better", "user": { "login": "me" },
                "created_at": "2026-09-01T10:00:00Z"
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/repos/acme/app/pulls/comments/21"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let gh = Github::with_base("tok".into(), server.uri());
        let c = gh.edit_comment(&rr(), 7, "11", "better").await.unwrap();
        assert_eq!((c.body.as_str(), c.can_edit), ("better", true));
        gh.delete_comment(&rr(), 7, "21").await.unwrap();
        assert_eq!(
            hits(&server, "DELETE", "/repos/acme/app/pulls/comments/21").await,
            1
        );
        assert!(matches!(
            gh.edit_comment(&rr(), 7, "review-5", "x").await,
            Err(Error::Invalid(_))
        ));
        // A comment on another PR is not this PR's to edit.
        ok_json(
            &server,
            "GET",
            "/repos/acme/app/issues/comments/31",
            json!({ "id": 31, "issue_url": "https://api.github.com/repos/acme/app/issues/8" }),
        )
        .await;
        assert!(matches!(
            gh.delete_comment(&rr(), 7, "31").await,
            Err(Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn unapprove_dismisses_the_own_latest_approval() {
        let server = MockServer::start().await;
        ok_json(&server, "GET", "/user", json!({ "login": "me", "id": 1 })).await;
        ok_json(
            &server,
            "GET",
            &format!("{PR}/reviews"),
            json!([
                { "id": 50, "user": { "login": "me", "id": 1 }, "state": "APPROVED" },
                { "id": 51, "user": { "login": "other", "id": 2 }, "state": "APPROVED" },
                { "id": 52, "user": { "login": "me", "id": 1 }, "state": "COMMENTED" }
            ]),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path(format!("{PR}/reviews/50/dismissals")))
            .and(body_partial_json(json!({ "event": "DISMISS" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let gh = Github::with_base("tok".into(), server.uri());
        gh.unapprove(&rr(), 7).await.unwrap();
        // No change request of mine to withdraw.
        match gh.withdraw_request_changes(&rr(), 7).await {
            Err(Error::Conflict(m)) => assert!(m.contains("no change request"), "{m}"),
            other => panic!("expected Conflict, got {other:?}"),
        }
        // GitHub refusing the dismissal reads as "not supported for this account".
        Mock::given(method("PUT"))
            .and(path(format!("{PR}/reviews/50/dismissals")))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(json!({ "message": "Must have admin rights" })),
            )
            .mount(&server)
            .await;
        match gh.unapprove(&rr(), 7).await {
            Err(Error::Conflict(m)) => {
                assert!(
                    m.contains("not supported on GitHub for this account"),
                    "{m}"
                )
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn only_the_current_opinion_is_dismissed() {
        let me = Me::from_keys(vec!["login:me".into()]).unwrap();
        let rv = |id: u64, who: &str, state: &str| json!({ "id": id, "user": { "login": who }, "state": state });
        let approved_then_changes = [rv(1, "me", "APPROVED"), rv(2, "me", "CHANGES_REQUESTED")];
        assert_eq!(
            own_review_to_dismiss(&approved_then_changes, &me, "APPROVED"),
            None
        );
        assert_eq!(
            own_review_to_dismiss(&approved_then_changes, &me, "CHANGES_REQUESTED"),
            Some(2)
        );
        let others = [rv(3, "you", "APPROVED")];
        assert_eq!(own_review_to_dismiss(&others, &me, "APPROVED"), None);
    }

    #[test]
    fn quote_reply_quotes_the_first_lines_trimmed() {
        let long = "x".repeat(250);
        let body = quote_reply("ann", &format!("one\n\n  two  \nthree\n{long}"), "ok");
        assert!(
            body.starts_with("> @ann wrote:\n> one\n> two\n> three\n> …\n\nok"),
            "{body}"
        );
        let short = quote_reply("bo", &long, "ok");
        assert!(
            short.contains(&format!("> {}…\n", "x".repeat(200))),
            "{short}"
        );
    }
}

// ---------------------------------------------------------------------------
// GitLab
// ---------------------------------------------------------------------------

mod gitlab {
    use super::*;
    use crate::providers::gitlab::Gitlab;

    const MR: &str = "/api/v4/projects/acme%2Fapp/merge_requests/7";
    const DISC: &str = "0123456789abcdef0123456789abcdef01234567";

    #[tokio::test]
    async fn me_by_user_id_never_by_name() {
        let server = MockServer::start().await;
        ok_json(
            &server,
            "GET",
            "/api/v4/user",
            json!({ "id": 5, "username": "me" }),
        )
        .await;
        ok_json(
            &server,
            "GET",
            MR,
            json!({ "iid": 7, "title": "MR", "state": "opened",
                    "author": { "id": 5, "name": "Display Name" } }),
        )
        .await;
        ok_json(
            &server,
            "GET",
            &format!("{MR}/discussions"),
            json!([{ "id": DISC, "notes": [
                { "id": 100, "body": "mine", "author": { "id": 5, "name": "X" },
                  "created_at": "2026-09-01T10:00:00Z" },
                { "id": 101, "body": "theirs", "author": { "id": 6, "name": "Display Name" },
                  "created_at": "2026-09-01T11:00:00Z" }
            ] }]),
        )
        .await;
        ok_json(
            &server,
            "GET",
            &format!("{MR}/approvals"),
            json!({ "approved_by": [
                { "user": { "id": 5, "name": "Display Name" } },
                { "user": { "id": 6, "name": "Display Name" } }
            ] }),
        )
        .await;
        let gl = Gitlab::new("tok".into(), Some(server.uri()));
        let d = gl.get_pr(&rr(), 7).await.unwrap();
        assert!(d.summary.is_mine);
        let me: Vec<bool> = d.reviewers.iter().map(|r| r.is_me).collect();
        assert_eq!(me, [true, false], "same name, different id");
        assert!(d.comments[0].can_edit);
        assert!(!d.comments[0].replies[0].can_edit);
    }

    #[tokio::test]
    async fn unapprove_and_note_edit_delete_request_shape() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("{MR}/unapprove")))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .mount(&server)
            .await;
        // A thread head's id is its discussion id: the note is read back.
        ok_json(
            &server,
            "GET",
            &format!("{MR}/discussions/{DISC}"),
            json!({ "id": DISC, "notes": [
                { "id": 99, "system": true, "body": "changed the description" },
                { "id": 100, "body": "old" }
            ] }),
        )
        .await;
        Mock::given(method("PUT"))
            .and(path(format!("{MR}/notes/100")))
            .and(body_partial_json(json!({ "body": "new" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": 100, "body": "new", "author": { "id": 5 }, "created_at": "2026-09-01T10:00:00Z"
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(format!("{MR}/notes/101")))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let gl = Gitlab::new("tok".into(), Some(server.uri()));
        gl.unapprove(&rr(), 7).await.unwrap();
        let c = gl.edit_comment(&rr(), 7, DISC, "new").await.unwrap();
        assert_eq!(
            (c.id.as_str(), c.body.as_str(), c.can_edit),
            (DISC, "new", true)
        );
        gl.delete_comment(&rr(), 7, "101").await.unwrap();
        assert_eq!(hits(&server, "DELETE", &format!("{MR}/notes/101")).await, 1);
        assert!(gl.withdraw_request_changes(&rr(), 7).await.is_err());
    }
}
