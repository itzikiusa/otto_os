//! Wiremock coverage for the second review pass: Bitbucket identity learnt
//! from the caller's own writes (and a refused `/user` not re-asked),
//! Bitbucket merge checks from branch restrictions (memoised per repo),
//! reactions on GitHub / GitLab comments, GitLab outdated notes, and the
//! GitHub / GitLab merge-check mappings.

use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{GitProvider, RemoteRef};
use crate::providers::client::enable_cache_for_tests;
use otto_core::api::{PrReaction, ReactionKey};

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

async fn status(server: &MockServer, verb: &str, at: &str, code: u16) {
    Mock::given(method(verb))
        .and(path(at))
        .respond_with(
            ResponseTemplate::new(code).set_body_json(json!({"error": {"message": "nope"}})),
        )
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

fn rx(key: ReactionKey, count: u32, me: bool) -> PrReaction {
    PrReaction { key, count, me }
}

// ---------------------------------------------------------------------------
// Bitbucket: identity fallback + merge checks
// ---------------------------------------------------------------------------

mod bitbucket {
    use super::*;
    use crate::providers::bitbucket::{token_owner, Bitbucket};
    use crate::providers::identity::is_negative_for_tests;

    const REPO: &str = "/repositories/acme/app";

    fn pr_path(n: u64) -> String {
        format!("{REPO}/pullrequests/{n}")
    }

    /// PR `n` into `dest`: one approval (`{u-2}`), one change request
    /// (`{u-3}`), the caller (`{u-me}`) a silent reviewer.
    async fn mount_pr(server: &MockServer, n: u64, dest: &str) {
        ok_json(
            server,
            "GET",
            &pr_path(n),
            json!({
                "id": n, "title": "PR", "state": "OPEN", "task_count": 1,
                "author": { "display_name": "Dev", "uuid": "{u-2}" },
                "source": { "branch": { "name": "feat" } },
                "destination": { "branch": { "name": dest } },
                "updated_on": "2026-09-01T10:00:00Z",
                "participants": [
                    { "user": { "display_name": "Me", "uuid": "{u-me}" },
                      "role": "REVIEWER", "approved": false, "state": null },
                    { "user": { "display_name": "Ann", "uuid": "{u-2}" },
                      "role": "REVIEWER", "approved": true, "state": "approved" },
                    { "user": { "display_name": "Bob", "uuid": "{u-3}" },
                      "role": "REVIEWER", "approved": false, "state": "changes_requested" }
                ]
            }),
        )
        .await;
        ok_json(
            server,
            "GET",
            &format!("{}/comments", pr_path(n)),
            json!({ "values": [] }),
        )
        .await;
        ok_json(
            server,
            "GET",
            &format!("{}/statuses", pr_path(n)),
            json!({ "values": [ { "state": "SUCCESSFUL", "key": "ci" } ] }),
        )
        .await;
        ok_json(
            server,
            "GET",
            &format!("{}/tasks", pr_path(n)),
            json!({ "values": [ { "state": "UNRESOLVED" }, { "state": "RESOLVED" } ] }),
        )
        .await;
    }

    fn restriction(
        kind: &str,
        value: Option<u64>,
        matcher: serde_json::Value,
    ) -> serde_json::Value {
        let mut v = json!({ "kind": kind, "value": value });
        for (k, val) in matcher.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    }

    async fn mount_rules(server: &MockServer) {
        let glob = |p: &str| json!({ "branch_match_kind": "glob", "pattern": p });
        ok_json(
            server,
            "GET",
            &format!("{REPO}/branch-restrictions"),
            json!({ "values": [
                restriction("require_approvals_to_merge", Some(2), glob("main")),
                // A looser `*` rule of the same kind: the strictest wins.
                restriction("require_approvals_to_merge", Some(1), glob("*")),
                restriction("require_no_changes_requested", None, glob("*")),
                restriction("require_tasks_to_be_completed", None, glob("release/*")),
                restriction("require_passing_builds_to_merge", Some(1),
                    json!({ "branch_match_kind": "branching_model", "branch_type": "production" })),
                // Not a merge check: ignored.
                restriction("push", None, glob("main")),
            ] }),
        )
        .await;
        ok_json(
            server,
            "GET",
            &format!("{REPO}/branching-model"),
            json!({
                "development": { "branch": { "name": "develop" } },
                "production": { "branch": { "name": "main" } },
                "branch_types": [ { "kind": "release", "prefix": "release/" } ]
            }),
        )
        .await;
    }

    fn checks(d: &otto_core::api::PrDetail) -> Vec<(&str, &str, bool, Option<&str>)> {
        d.merge_checks
            .iter()
            .map(|c| {
                (
                    c.key.as_str(),
                    c.label.as_str(),
                    c.passed,
                    c.detail.as_deref(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn restrictions_matched_by_branch_drive_the_checks() {
        let server = MockServer::start().await;
        mount_pr(&server, 7, "main").await;
        mount_rules(&server).await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        let d = bb.get_pr(&rr(), 7).await.unwrap();
        assert_eq!(
            checks(&d),
            [
                ("approvals", "At least 2 approvals", false, Some("1 of 2")),
                (
                    "no_changes_requested",
                    "No changes requested",
                    false,
                    Some("1 reviewer requested changes")
                ),
                (
                    "builds",
                    "At least 1 successful build",
                    true,
                    Some("1 of 1 successful")
                ),
            ],
            "`release/*` does not cover main; the production model rule does"
        );
        assert_eq!(
            d.task_count,
            Some(otto_core::api::TaskCount {
                open: 1,
                resolved: 1
            })
        );
        assert!(!d.reactions_supported);
    }

    #[tokio::test]
    async fn a_glob_rule_covers_its_branches_only() {
        let server = MockServer::start().await;
        mount_pr(&server, 7, "release/2.1").await;
        mount_rules(&server).await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        let d = bb.get_pr(&rr(), 7).await.unwrap();
        let keys: Vec<_> = checks(&d).iter().map(|c| (c.0, c.2)).collect();
        assert_eq!(
            keys,
            [
                ("approvals", true),
                ("no_changes_requested", false),
                ("tasks", false)
            ],
            "the `*` approval rule (1) applies, `main` and production do not"
        );
        assert_eq!(d.merge_checks[2].detail.as_deref(), Some("1 open task"));
    }

    #[tokio::test]
    async fn unreadable_restrictions_mean_no_checks_and_the_pr_still_loads() {
        let server = MockServer::start().await;
        enable_cache_for_tests(&server.uri());
        mount_pr(&server, 7, "main").await;
        mount_pr(&server, 8, "main").await;
        status(&server, "GET", &format!("{REPO}/branch-restrictions"), 403).await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        let d = bb.get_pr(&rr(), 7).await.expect("PR loads");
        assert!(d.merge_checks.is_empty(), "no rule is invented");
        let first = hits(&server, "GET", &format!("{REPO}/branch-restrictions")).await;
        bb.get_pr(&rr(), 8).await.unwrap();
        assert_eq!(
            hits(&server, "GET", &format!("{REPO}/branch-restrictions")).await,
            first,
            "the refusal is memoised for the repo"
        );
    }

    #[tokio::test]
    async fn rules_are_read_once_per_repo() {
        let server = MockServer::start().await;
        enable_cache_for_tests(&server.uri());
        mount_pr(&server, 7, "main").await;
        mount_pr(&server, 8, "main").await;
        mount_rules(&server).await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        let a = bb.get_pr(&rr(), 7).await.unwrap();
        let b = bb.get_pr(&rr(), 8).await.unwrap();
        assert_eq!(a.merge_checks.len(), 3);
        assert_eq!(b.merge_checks, a.merge_checks);
        assert_eq!(
            hits(&server, "GET", &format!("{REPO}/branch-restrictions")).await,
            1,
            "the second PR is a memo hit"
        );
        assert_eq!(
            hits(&server, "GET", &format!("{REPO}/branching-model")).await,
            1
        );
    }

    #[tokio::test]
    async fn a_refused_identity_is_not_asked_again_on_every_pr_open() {
        let server = MockServer::start().await;
        enable_cache_for_tests(&server.uri());
        mount_pr(&server, 7, "main").await;
        mount_pr(&server, 8, "main").await;
        status(&server, "GET", "/user", 403).await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        bb.get_pr(&rr(), 7).await.unwrap();
        let first = hits(&server, "GET", "/user").await;
        assert!(first >= 1);
        assert!(is_negative_for_tests(&bb.identity_key()));
        bb.get_pr(&rr(), 8).await.unwrap();
        assert_eq!(hits(&server, "GET", "/user").await, first);
    }

    #[tokio::test]
    async fn a_network_failure_of_the_identity_is_retried() {
        let server = MockServer::start().await;
        enable_cache_for_tests(&server.uri());
        mount_pr(&server, 7, "main").await;
        mount_pr(&server, 8, "main").await;
        status(&server, "GET", "/user", 503).await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        bb.get_pr(&rr(), 7).await.unwrap();
        let first = hits(&server, "GET", "/user").await;
        assert!(!is_negative_for_tests(&bb.identity_key()));
        bb.get_pr(&rr(), 8).await.unwrap();
        assert!(hits(&server, "GET", "/user").await > first);
    }

    /// Without Account: read, `is_me` is learnt from the approve response
    /// (the caller's participant object).
    #[tokio::test]
    async fn approving_teaches_who_you_are() {
        let server = MockServer::start().await;
        enable_cache_for_tests(&server.uri());
        mount_pr(&server, 7, "main").await;
        status(&server, "GET", "/user", 403).await;
        ok_json(
            &server,
            "POST",
            &format!("{}/approve", pr_path(7)),
            json!({ "type": "participant", "approved": true,
                    "user": { "display_name": "Me", "uuid": "{U-ME}", "account_id": "acc-me" } }),
        )
        .await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        let before = bb.get_pr(&rr(), 7).await.unwrap();
        assert!(before.reviewers.iter().all(|r| !r.is_me));
        bb.approve(&rr(), 7).await.unwrap();
        let after = bb.get_pr(&rr(), 7).await.unwrap();
        let me: Vec<_> = after
            .reviewers
            .iter()
            .map(|r| (r.name.as_str(), r.is_me))
            .collect();
        assert_eq!(me, [("Me", true), ("Ann", false), ("Bob", false)]);
    }

    /// A posted comment's author is the caller too.
    #[tokio::test]
    async fn commenting_teaches_who_you_are() {
        let server = MockServer::start().await;
        enable_cache_for_tests(&server.uri());
        status(&server, "GET", "/user", 401).await;
        ok_json(
            &server,
            "POST",
            &format!("{}/comments", pr_path(7)),
            json!({ "id": 9, "content": { "raw": "hi" }, "created_on": "2026-09-01T10:00:00Z",
                    "user": { "display_name": "Me", "uuid": "{u-me}" } }),
        )
        .await;
        let bb = Bitbucket::with_base("u".into(), "tok".into(), server.uri());
        assert!(bb.me().await.is_none());
        bb.comment(
            &rr(),
            7,
            &otto_core::api::NewPrCommentReq {
                body: "hi".into(),
                path: None,
                line: None,
                in_reply_to: None,
                side: None,
                old_line: None,
                commit_id: None,
            },
        )
        .await
        .unwrap();
        let me = bb.me().await.expect("learnt from the comment");
        assert!(me.owns(&[crate::providers::identity::key("uuid", "{u-me}")]));
    }

    #[tokio::test]
    async fn reactions_are_a_409_on_bitbucket() {
        let bb = Bitbucket::with_base("u".into(), "tok".into(), "http://127.0.0.1:9".into());
        match bb
            .set_reaction(&rr(), 7, "1", ReactionKey::Heart, true)
            .await
        {
            Err(otto_core::Error::Conflict(m)) => assert_eq!(m, "Bitbucket Cloud has no reactions"),
            other => panic!("expected 409, got {other:?}"),
        }
    }

    #[test]
    fn token_owner_names_the_account_not_the_deprecated_username() {
        assert_eq!(
            token_owner(&json!({ "display_name": "Itzik A.", "nickname": "itzik",
                                 "account_id": "5b10", "username": "old" })),
            "Itzik A. (itzik) · account 5b10"
        );
        assert_eq!(
            token_owner(&json!({ "display_name": "kay", "nickname": "Kay" })),
            "kay"
        );
        assert_eq!(token_owner(&json!({ "username": "legacy" })), "legacy");
    }
}

// ---------------------------------------------------------------------------
// GitHub: reactions + merge checks
// ---------------------------------------------------------------------------

mod github {
    use super::*;
    use crate::providers::github::{merge_checks_from, Github};

    const PR: &str = "/repos/acme/app/pulls/7";

    async fn mount_detail(server: &MockServer) {
        ok_json(
            server,
            "GET",
            PR,
            json!({ "number": 7, "state": "open", "user": { "login": "dev", "id": 2 },
                    "head": { "ref": "feat", "sha": "deadbeef" }, "base": { "ref": "main" },
                    "mergeable": true, "mergeable_state": "blocked",
                    "updated_at": "2026-09-01T10:00:00Z" }),
        )
        .await;
        ok_json(
            server,
            "GET",
            "/repos/acme/app/issues/7/comments",
            json!([
                { "id": 11, "body": "nice", "user": { "login": "dev", "id": 2 },
                  "created_at": "2026-09-01T10:00:00Z",
                  "reactions": { "total_count": 3, "+1": 2, "-1": 0, "heart": 1, "eyes": 0 } },
                { "id": 12, "body": "quiet", "user": { "login": "dev", "id": 2 },
                  "created_at": "2026-09-01T11:00:00Z",
                  "reactions": { "total_count": 0, "+1": 0 } }
            ]),
        )
        .await;
        ok_json(server, "GET", &format!("{PR}/comments"), json!([])).await;
        ok_json(
            server,
            "GET",
            &format!("{PR}/reviews"),
            json!([{ "id": 5, "user": { "login": "rev", "id": 3 }, "state": "COMMENTED",
                     "body": "summary", "submitted_at": "2026-09-01T12:00:00Z" }]),
        )
        .await;
        ok_json(
            server,
            "GET",
            "/repos/acme/app/issues/comments/11/reactions",
            json!([
                { "id": 1, "content": "+1", "user": { "login": "ME", "id": 1 } },
                { "id": 2, "content": "+1", "user": { "login": "x", "id": 9 } },
                { "id": 3, "content": "heart", "user": { "login": "x", "id": 9 } }
            ]),
        )
        .await;
    }

    #[tokio::test]
    async fn counts_come_from_the_rollup_and_me_from_the_list() {
        let server = MockServer::start().await;
        mount_detail(&server).await;
        ok_json(&server, "GET", "/user", json!({ "login": "me", "id": 1 })).await;
        let d = Github::with_base("tok".into(), server.uri())
            .get_pr(&rr(), 7)
            .await
            .unwrap();
        let by_id = |id: &str| d.comments.iter().find(|c| c.id == id).unwrap();
        assert_eq!(
            by_id("11").reactions,
            [
                rx(ReactionKey::ThumbsUp, 2, true),
                rx(ReactionKey::Heart, 1, false)
            ]
        );
        assert!(by_id("11").can_react);
        assert!(by_id("12").reactions.is_empty());
        assert!(!by_id("review-5").can_react, "a review summary takes none");
        assert!(d.reactions_supported);
        assert_eq!(
            hits(
                &server,
                "GET",
                "/repos/acme/app/issues/comments/12/reactions"
            )
            .await,
            0,
            "no list read for a comment without reactions"
        );
    }

    #[tokio::test]
    async fn without_an_identity_the_counts_stay_and_nothing_more_is_read() {
        let server = MockServer::start().await;
        mount_detail(&server).await;
        status(&server, "GET", "/user", 403).await;
        let d = Github::with_base("tok".into(), server.uri())
            .get_pr(&rr(), 7)
            .await
            .expect("the PR loads");
        let c = d.comments.iter().find(|c| c.id == "11").unwrap();
        assert_eq!(
            c.reactions,
            [
                rx(ReactionKey::ThumbsUp, 2, false),
                rx(ReactionKey::Heart, 1, false)
            ]
        );
        assert_eq!(
            hits(
                &server,
                "GET",
                "/repos/acme/app/issues/comments/11/reactions"
            )
            .await,
            0
        );
    }

    #[tokio::test]
    async fn adding_posts_the_content_on_the_review_comment_endpoint() {
        let server = MockServer::start().await;
        ok_json(
            &server,
            "GET",
            "/repos/acme/app/pulls/comments/21",
            json!({ "id": 21, "pull_request_url": "https://api.github.com/repos/acme/app/pulls/7" }),
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/app/pulls/comments/21/reactions"))
            .and(body_partial_json(json!({ "content": "rocket" })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 70 })))
            .expect(1)
            .mount(&server)
            .await;
        ok_json(
            &server,
            "GET",
            "/repos/acme/app/pulls/comments/21/reactions",
            json!([{ "id": 70, "content": "rocket", "user": { "login": "me", "id": 1 } }]),
        )
        .await;
        ok_json(&server, "GET", "/user", json!({ "login": "me", "id": 1 })).await;
        let got = Github::with_base("tok".into(), server.uri())
            .set_reaction(&rr(), 7, "21", ReactionKey::Rocket, true)
            .await
            .unwrap();
        assert_eq!(got, [rx(ReactionKey::Rocket, 1, true)]);
    }

    #[tokio::test]
    async fn removing_deletes_only_your_own_reaction_by_id() {
        let server = MockServer::start().await;
        status(&server, "GET", "/repos/acme/app/pulls/comments/11", 404).await;
        ok_json(
            &server,
            "GET",
            "/repos/acme/app/issues/comments/11",
            json!({ "id": 11, "issue_url": "https://api.github.com/repos/acme/app/issues/7" }),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/issues/comments/11/reactions"))
            .and(query_param("content", "+1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "id": 55, "content": "+1", "user": { "login": "me", "id": 1 } },
                { "id": 56, "content": "+1", "user": { "login": "x", "id": 9 } }
            ])))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/issues/comments/11/reactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "id": 56, "content": "+1", "user": { "login": "x", "id": 9 } }
            ])))
            .with_priority(2)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/repos/acme/app/issues/comments/11/reactions/55"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        ok_json(&server, "GET", "/user", json!({ "login": "me", "id": 1 })).await;
        let got = Github::with_base("tok".into(), server.uri())
            .set_reaction(&rr(), 7, "11", ReactionKey::ThumbsUp, false)
            .await
            .unwrap();
        assert_eq!(got, [rx(ReactionKey::ThumbsUp, 1, false)]);
        assert_eq!(
            hits(
                &server,
                "DELETE",
                "/repos/acme/app/issues/comments/11/reactions/56"
            )
            .await,
            0,
            "someone else's reaction is never touched"
        );
    }

    #[tokio::test]
    async fn a_review_summary_takes_no_reaction() {
        let gh = Github::with_base("tok".into(), "http://127.0.0.1:9".into());
        let err = gh
            .set_reaction(&rr(), 7, "review-5", ReactionKey::Heart, true)
            .await
            .expect_err("refused");
        assert!(matches!(err, otto_core::Error::Invalid(_)), "{err}");
    }

    #[test]
    fn mergeable_state_maps_to_honest_checks() {
        let pr = |state: &str, mergeable: serde_json::Value| json!({ "state": "open", "mergeable": mergeable, "mergeable_state": state });
        let keys = |v: serde_json::Value| -> Vec<(String, bool)> {
            merge_checks_from(&v)
                .into_iter()
                .map(|c| (c.key, c.passed))
                .collect()
        };
        let k = |s: &str, b: bool| (s.to_string(), b);
        assert_eq!(
            keys(pr("blocked", json!(true))),
            [k("conflicts", true), k("branch_protection", false)]
        );
        assert_eq!(
            keys(pr("clean", json!(true))),
            [k("conflicts", true), k("branch_protection", true)]
        );
        assert_eq!(keys(pr("dirty", json!(null))), [k("conflicts", false)]);
        assert_eq!(
            keys(pr("dirty", json!(false))),
            [k("conflicts", false)],
            "never two conflict rows"
        );
        assert_eq!(keys(pr("behind", json!(null))), [k("up_to_date", false)]);
        assert_eq!(
            keys(pr("unstable", json!(true))),
            [
                k("conflicts", true),
                k("branch_protection", true),
                k("checks", false)
            ]
        );
        assert!(keys(pr("unknown", json!(null))).is_empty());
        assert!(keys(json!({ "state": "closed", "mergeable_state": "clean" })).is_empty());
    }
}

// ---------------------------------------------------------------------------
// GitLab: award emoji, outdated notes, merge checks
// ---------------------------------------------------------------------------

mod gitlab {
    use super::*;
    use crate::providers::gitlab::{merge_checks_from, Gitlab};

    const MR: &str = "/api/v4/projects/acme%2Fapp/merge_requests/7";
    const DISC: &str = "0123456789abcdef0123456789abcdef01234567";

    async fn mount_mr(server: &MockServer) {
        ok_json(server, "GET", "/api/v4/user", json!({ "id": 5 })).await;
        ok_json(
            server,
            "GET",
            MR,
            json!({ "iid": 7, "title": "MR", "state": "opened",
                    "author": { "id": 6, "name": "Dev" },
                    "diff_refs": { "base_sha": "b0", "start_sha": "s0", "head_sha": "new" },
                    "detailed_merge_status": "not_approved",
                    "blocking_discussions_resolved": true }),
        )
        .await;
        let pos = |head: &str, line: u64| json!({ "new_path": "a.rs", "old_path": "a.rs", "new_line": line, "head_sha": head });
        ok_json(
            server,
            "GET",
            &format!("{MR}/discussions"),
            json!([{ "id": DISC, "notes": [
                { "id": 100, "body": "old anchor", "author": { "id": 6, "name": "Dev" },
                  "created_at": "2026-09-01T10:00:00Z", "position": pos("old", 3) },
                { "id": 101, "body": "current", "author": { "id": 5, "name": "Me" },
                  "created_at": "2026-09-01T11:00:00Z", "position": pos("new", 4) }
            ] }]),
        )
        .await;
        ok_json(
            server,
            "GET",
            &format!("{MR}/notes/100/award_emoji"),
            json!([
                { "id": 1, "name": "thumbsup", "user": { "id": 5 } },
                { "id": 2, "name": "tada", "user": { "id": 6 } },
                { "id": 3, "name": "smile", "user": { "id": 6 } }
            ]),
        )
        .await;
        ok_json(
            server,
            "GET",
            &format!("{MR}/notes/101/award_emoji"),
            json!([]),
        )
        .await;
    }

    #[tokio::test]
    async fn awards_map_to_reactions_and_old_anchors_are_outdated() {
        let server = MockServer::start().await;
        mount_mr(&server).await;
        let d = Gitlab::new("tok".into(), Some(server.uri()))
            .get_pr(&rr(), 7)
            .await
            .unwrap();
        let head = &d.comments[0];
        assert_eq!(head.id, DISC);
        assert_eq!(
            head.reactions,
            [
                rx(ReactionKey::ThumbsUp, 1, true),
                rx(ReactionKey::Hooray, 1, false)
            ],
            "unknown award names are left out"
        );
        assert!(head.can_react && d.reactions_supported);
        assert!(head.outdated, "anchored to an older head");
        assert!(!head.replies[0].outdated);
        assert!(head.replies[0].reactions.is_empty());
        let checks: Vec<_> = d
            .merge_checks
            .iter()
            .map(|c| (c.key.as_str(), c.passed))
            .collect();
        assert_eq!(checks, [("discussions", true), ("approvals", false)]);
    }

    #[tokio::test]
    async fn award_and_unaward_on_a_thread_head_use_its_note() {
        let server = MockServer::start().await;
        ok_json(
            &server,
            "GET",
            &format!("{MR}/discussions/{DISC}"),
            json!({ "id": DISC, "notes": [ { "id": 100, "body": "x" } ] }),
        )
        .await;
        ok_json(&server, "GET", "/api/v4/user", json!({ "id": 5 })).await;
        Mock::given(method("POST"))
            .and(path(format!("{MR}/notes/100/award_emoji")))
            .and(body_partial_json(json!({ "name": "eyes" })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 9 })))
            .expect(1)
            .mount(&server)
            .await;
        ok_json(
            &server,
            "GET",
            &format!("{MR}/notes/100/award_emoji"),
            json!([
                { "id": 9, "name": "eyes", "user": { "id": 5 } },
                { "id": 8, "name": "eyes", "user": { "id": 6 } }
            ]),
        )
        .await;
        Mock::given(method("DELETE"))
            .and(path(format!("{MR}/notes/100/award_emoji/9")))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let gl = Gitlab::new("tok".into(), Some(server.uri()));
        let added = gl
            .set_reaction(&rr(), 7, DISC, ReactionKey::Eyes, true)
            .await
            .unwrap();
        assert_eq!(added, [rx(ReactionKey::Eyes, 2, true)]);
        gl.set_reaction(&rr(), 7, DISC, ReactionKey::Eyes, false)
            .await
            .unwrap();
        assert_eq!(
            hits(&server, "DELETE", &format!("{MR}/notes/100/award_emoji/8")).await,
            0,
            "someone else's award is never removed"
        );
    }

    #[test]
    fn detailed_merge_status_maps_to_checks() {
        let keys = |v: serde_json::Value| -> Vec<(String, bool)> {
            merge_checks_from(&v)
                .into_iter()
                .map(|c| (c.key, c.passed))
                .collect()
        };
        let k = |s: &str, b: bool| (s.to_string(), b);
        assert_eq!(
            keys(
                json!({ "state": "opened", "detailed_merge_status": "mergeable",
                         "blocking_discussions_resolved": true })
            ),
            [k("discussions", true), k("mergeable", true)]
        );
        assert_eq!(
            keys(json!({ "state": "opened", "detailed_merge_status": "discussions_not_resolved" })),
            [k("discussions", false)]
        );
        assert_eq!(
            keys(json!({ "state": "opened", "detailed_merge_status": "conflict" })),
            [k("conflicts", false)]
        );
        assert!(keys(json!({ "state": "opened", "detailed_merge_status": "checking" })).is_empty());
        assert!(
            keys(json!({ "state": "merged", "detailed_merge_status": "mergeable" })).is_empty()
        );
    }
}
