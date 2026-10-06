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
    async fn merge_cleanup_never_deletes_a_fork_namesake() {
        for owner in ["contributor", "acme"] {
            let server = MockServer::start().await;
            let gh = Github::with_base("tok".into(), server.uri());
            Mock::given(method("GET"))
                .and(path("/repos/acme/app/pulls/7"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "head": {"ref": "feature", "repo": {"full_name": format!("{owner}/app")}}
                })))
                .mount(&server)
                .await;
            Mock::given(method("PUT"))
                .and(path("/repos/acme/app/pulls/7/merge"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            Mock::given(method("DELETE"))
                .respond_with(ResponseTemplate::new(204))
                .mount(&server)
                .await;
            gh.merge(&rr(), 7, otto_core::api::MergeStrategy::Merge, true)
                .await
                .unwrap();
            let deletes = server
                .received_requests()
                .await
                .unwrap()
                .into_iter()
                .filter(|r| r.method.as_str() == "DELETE")
                .count();
            assert_eq!(
                deletes,
                usize::from(owner == "acme"),
                "cleanup of {owner}/app"
            );
        }
    }

    /// Past GitHub's `.diff` limits (406) the PR diff degrades to the
    /// per-file listing instead of failing: counts + status for every file,
    /// hunks where GitHub sent a patch, `too_large` where it didn't.
    #[tokio::test]
    async fn pr_diff_too_large_falls_back_to_the_files_listing() {
        use otto_core::api::FileChangeStatus;
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/9"))
            .respond_with(ResponseTemplate::new(406).set_body_json(json!({
                "message": "Sorry, the diff exceeded the maximum number of files (300).",
                "errors": [{"resource": "PullRequest", "field": "diff", "code": "too_large"}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/9/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"filename": "src/a.rs", "status": "modified", "additions": 1, "deletions": 1,
                 "patch": "@@ -1,2 +1,2 @@\n keep\n-old\n+new"},
                {"filename": "moved.rs", "previous_filename": "was.rs", "status": "renamed",
                 "additions": 0, "deletions": 0},
                {"filename": "huge.lock", "status": "modified", "additions": 90000, "deletions": 3},
                {"filename": "img.png", "status": "modified", "additions": 0, "deletions": 0}
            ])))
            .mount(&server)
            .await;
        let d = gh.get_pr_diff(&rr(), 9).await.unwrap();
        assert_eq!(d.files.len(), 4);
        let a = &d.files[0];
        assert_eq!((a.added, a.deleted), (Some(1), Some(1)));
        assert_eq!(a.hunks[0].lines.len(), 3);
        assert_eq!(a.language.as_deref(), Some("rust"));
        let m = &d.files[1];
        assert_eq!(m.status, Some(FileChangeStatus::Renamed));
        assert_eq!(m.old_path.as_deref(), Some("was.rs"));
        assert_eq!(m.too_large, None);
        let h = &d.files[2];
        assert_eq!(h.too_large, Some(true));
        assert_eq!(h.hunks_omitted, Some(true));
        assert_eq!(h.added, Some(90000));
        assert!(d.files[3].is_binary);
        assert_eq!(d.total_added, Some(90001));
    }

    #[tokio::test]
    async fn pr_diff_other_errors_still_fail() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/9"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
            .mount(&server)
            .await;
        assert!(gh.get_pr_diff(&rr(), 9).await.is_err());
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
                        format!(
                            r#"<{}/repos/acme/app/pulls?page=2>; rel="next""#,
                            server.uri()
                        )
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
        assert_eq!(
            bodies,
            vec!["first", "second"],
            "both pages must be present"
        );
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

    /// `/changes` past GitLab's own limits sets `overflow` and drops files:
    /// the diff must say it is partial.
    #[tokio::test]
    async fn mr_changes_overflow_marks_the_diff_truncated() {
        let server = MockServer::start().await;
        let gl = Gitlab::new("tok".into(), Some(server.uri()));
        Mock::given(method("GET"))
            .and(path_regex(
                r"^/api/v4/projects/.+/merge_requests/5/changes$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "overflow": true,
                "changes": [{
                    "old_path": "a.rs", "new_path": "a.rs",
                    "diff": "@@ -1 +1 @@\n-x\n+y\n",
                }],
            })))
            .mount(&server)
            .await;
        let d = gl.get_pr_diff(&rr(), 5).await.unwrap();
        assert_eq!(d.truncated, Some(true));
        assert_eq!((d.files[0].added, d.files[0].deleted), (Some(1), Some(1)));
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
            .and(path_regex(
                r"^/api/v4/projects/.+/merge_requests/7/discussions$",
            ))
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

    /// Summary-first: the file list comes from `diffstat` (paged by `next`),
    /// never the unified diff — every status maps, a rename keeps its origin,
    /// counts are Bitbucket's `lines_added/removed`.
    #[tokio::test]
    async fn pr_diff_summary_reads_diffstat_pages_not_the_diff() {
        let server = MockServer::start().await;
        let bb = Bitbucket::with_base("user".into(), "tok".into(), server.uri());
        Mock::given(method("GET"))
            .and(path("/repositories/acme/app/pullrequests/7/diffstat"))
            .and(query_param("pagelen", "500"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "values": [
                    {"status": "modified", "lines_added": 3, "lines_removed": 1,
                     "old": {"path": "src/a.rs"}, "new": {"path": "src/a.rs"}},
                    {"status": "renamed", "lines_added": 0, "lines_removed": 0,
                     "old": {"path": "old/b.ts"}, "new": {"path": "new/b.ts"}},
                ],
                "next": format!("{}/repositories/acme/app/pullrequests/7/diffstat?page=2", server.uri()),
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repositories/acme/app/pullrequests/7/diffstat"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "values": [
                    {"status": "added", "lines_added": 5, "lines_removed": 0,
                     "old": null, "new": {"path": "c.md"}},
                    {"status": "removed", "lines_added": 0, "lines_removed": 9,
                     "old": {"path": "gone.py"}, "new": null},
                ],
            })))
            .mount(&server)
            .await;
        // The whole diff must NOT be downloaded for a summary.
        Mock::given(method("GET"))
            .and(path("/repositories/acme/app/pullrequests/7/diff"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .expect(0)
            .mount(&server)
            .await;

        let d = bb
            .get_pr_diff_summary(&rr(), 7)
            .await
            .unwrap()
            .expect("supported");
        use otto_core::api::FileChangeStatus as S;
        let rows: Vec<_> = d
            .files
            .iter()
            .map(|f| {
                (
                    f.path.as_str(),
                    f.old_path.as_deref(),
                    f.status,
                    f.added,
                    f.deleted,
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("src/a.rs", None, Some(S::Modified), Some(3), Some(1)),
                (
                    "new/b.ts",
                    Some("old/b.ts"),
                    Some(S::Renamed),
                    Some(0),
                    Some(0)
                ),
                ("c.md", None, Some(S::Added), Some(5), Some(0)),
                ("gone.py", None, Some(S::Deleted), Some(0), Some(9)),
            ]
        );
        assert!(d
            .files
            .iter()
            .all(|f| f.hunks.is_empty() && f.hunks_omitted == Some(true)));
        assert_eq!((d.total_added, d.total_deleted), (Some(8), Some(10)));
        assert_eq!(d.truncated, None);
    }

    /// Opening one file asks Bitbucket for that file only (`?path=`, repeated
    /// for a rename's origin, URL-encoded).
    #[tokio::test]
    async fn pr_file_diff_sends_the_path_filter() {
        let server = MockServer::start().await;
        let bb = Bitbucket::with_base("user".into(), "tok".into(), server.uri());
        Mock::given(method("GET"))
            .and(path("/repositories/acme/app/pullrequests/7/diff"))
            .and(query_param("path", "new dir/b&c.ts"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "diff --git a/old/b.ts b/new dir/b&c.ts\nsimilarity index 90%\nrename from old/b.ts\nrename to new dir/b&c.ts\n--- a/old/b.ts\n+++ b/new dir/b&c.ts\n@@ -1 +1 @@\n-x\n+y\n",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let d = bb
            .get_pr_file_diff(&rr(), 7, "new dir/b&c.ts", Some("old/b.ts"))
            .await
            .unwrap()
            .expect("supported");
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].hunks.len(), 1);
        let reqs = server.received_requests().await.unwrap();
        let q = reqs[0].url.query().unwrap_or("").to_string();
        assert!(q.contains("path=old%2Fb.ts"), "rename origin sent too: {q}");
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
            .and(path(
                "/repositories/acme/app/pullrequests/7/request-changes",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        bb.request_changes(&rr(), 7, Some("please fix"))
            .await
            .unwrap();

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
            .and(path(
                "/repositories/acme/app/pullrequests/7/request-changes",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        // Whitespace-only is "no body" — an empty comment helps nobody.
        bb.request_changes(&rr(), 7, Some("   ")).await.unwrap();
        bb.request_changes(&rr(), 7, None).await.unwrap();

        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 2);
        assert!(reqs
            .iter()
            .all(|r| r.url.path().ends_with("/request-changes")));
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

    /// S2-305: the GitLab client never follows a redirect to another origin,
    /// so its custom `PRIVATE-TOKEN` header cannot leak to an SSO host.
    #[tokio::test]
    async fn gitlab_token_is_not_sent_across_a_cross_origin_redirect() {
        let forge = MockServer::start().await;
        let sso = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/o%2Fr/merge_requests/1"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/login", sso.uri()).as_str()),
            )
            .mount(&forge)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&sso)
            .await;
        let gl = crate::providers::gitlab::Gitlab::new("glpat-secret".into(), Some(forge.uri()));
        let r = RemoteRef {
            owner: "o".into(),
            repo: "r".into(),
        };
        assert!(gl.get_pr(&r, 1).await.is_err());
        assert!(
            sso.received_requests().await.unwrap().is_empty(),
            "the cross-origin hop must not be followed"
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

// ---------------------------------------------------------------------------
// Inline-comment anchors: every forge gets the diff SIDE, not just a number.
// ---------------------------------------------------------------------------

mod inline_anchors {
    use otto_core::api::{NewPrCommentReq, PrCommentSide};
    use serde_json::json;

    fn req(line: u32, side: Option<PrCommentSide>, old_line: Option<u32>) -> NewPrCommentReq {
        NewPrCommentReq {
            body: "nit".into(),
            path: Some("src/a.rs".into()),
            line: Some(line),
            in_reply_to: None,
            side,
            old_line,
            commit_id: None,
        }
    }

    #[test]
    fn github_deleted_line_goes_left_with_the_old_number() {
        let b = crate::providers::github::inline_comment_body(
            &req(12, Some(PrCommentSide::Old), Some(12)),
            "src/a.rs",
            12,
            "abc",
        );
        assert_eq!(b["side"], "LEFT");
        assert_eq!(b["line"], 12);
        assert_eq!(b["commit_id"], "abc");
        // Absent side stays RIGHT — old MCP/agent callers keep working.
        let b = crate::providers::github::inline_comment_body(&req(3, None, None), "p", 3, "abc");
        assert_eq!(b["side"], "RIGHT");
    }

    #[test]
    fn gitlab_sends_old_line_for_deletions_and_both_for_context() {
        let mr = json!({"diff_refs": {"base_sha": "b", "start_sha": "s", "head_sha": "h"}});
        let del = crate::providers::gitlab::text_position(
            &req(7, Some(PrCommentSide::Old), Some(7)),
            "src/a.rs",
            7,
            &mr,
        );
        assert_eq!(del["old_line"], 7);
        assert!(del.get("new_line").is_none());
        let ctx = crate::providers::gitlab::text_position(
            &req(9, Some(PrCommentSide::New), Some(8)),
            "src/a.rs",
            9,
            &mr,
        );
        assert_eq!(ctx["new_line"], 9);
        assert_eq!(ctx["old_line"], 8);
        let add = crate::providers::gitlab::text_position(&req(4, None, None), "src/a.rs", 4, &mr);
        assert_eq!(add["new_line"], 4);
        assert!(add.get("old_line").is_none());
        assert_eq!(add["head_sha"], "h");
    }

    #[test]
    fn bitbucket_deleted_line_uses_from() {
        let del = crate::providers::bitbucket::inline_anchor(
            &req(5, Some(PrCommentSide::Old), Some(5)),
            "src/a.rs",
        );
        assert_eq!(del["from"], 5);
        assert!(del.get("to").is_none());
        let add = crate::providers::bitbucket::inline_anchor(&req(6, None, None), "src/a.rs");
        assert_eq!(add["to"], 6);
    }
}

mod github_read_back {
    use super::*;
    use crate::providers::github::Github;
    use otto_core::api::PrCommentSide;

    /// A LEFT-side comment reads back as `side: old`; an outdated one
    /// (`line: null`) keeps its original line and is flagged outdated.
    #[tokio::test]
    async fn get_pr_reads_side_and_outdated() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "number": 3, "title": "t", "state": "open", "user": {"login": "d"},
                "head": {"ref": "f", "sha": "h1"}, "base": {"ref": "main"},
                "updated_at": "2026-09-01T10:00:00Z", "html_url": "u",
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/3/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": 1, "user": {"login": "a"}, "body": "x", "path": "f.rs",
                 "line": 10, "side": "LEFT", "created_at": "2026-09-01T10:00:00Z"},
                {"id": 2, "user": {"login": "a"}, "body": "y", "path": "f.rs",
                 "line": null, "original_line": 4, "original_side": "RIGHT",
                 "created_at": "2026-09-01T10:00:00Z"},
            ])))
            .mount(&server)
            .await;
        for p in [
            "/repos/acme/app/issues/3/comments",
            "/repos/acme/app/pulls/3/reviews",
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
                .mount(&server)
                .await;
        }
        let d = gh.get_pr(&rr(), 3).await.unwrap();
        let c1 = d.comments.iter().find(|c| c.id == "1").unwrap();
        assert_eq!(
            (c1.line, c1.side, c1.outdated),
            (Some(10), Some(PrCommentSide::Old), false)
        );
        let c2 = d.comments.iter().find(|c| c.id == "2").unwrap();
        assert_eq!(
            (c2.line, c2.side, c2.outdated),
            (Some(4), Some(PrCommentSide::New), true)
        );
    }
}

// ---------------------------------------------------------------------------
// Cached provider reads (perf G1/G10): a remount within the TTL makes no
// request, a stale entry revalidates with If-None-Match (304 = cached body),
// and our own write clears the repo's entries.
// ---------------------------------------------------------------------------

mod cached_reads {
    use super::*;
    use crate::providers::client::{enable_cache_for_tests, expire_cached_for_tests};
    use crate::providers::github::Github;
    use wiremock::matchers::header;

    fn pr(number: u64) -> serde_json::Value {
        json!({
            "number": number, "title": format!("PR {number}"), "state": "open",
            "user": { "login": "dev" }, "head": { "ref": "feat", "sha": "deadbeef" },
            "base": { "ref": "main" }, "updated_at": "2026-09-01T10:00:00Z",
            "html_url": format!("https://github.com/acme/app/pull/{number}"),
        })
    }

    async fn list_gets(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method.as_str() == "GET" && r.url.path() == "/repos/acme/app/pulls")
            .count()
    }

    #[tokio::test]
    async fn second_list_within_ttl_makes_no_request_and_304_serves_cache() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        enable_cache_for_tests(&server.uri());
        // A conditional GET carrying the stored ETag is answered 304.
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .and(header("if-none-match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"v1\"")
                    .set_body_json(json!([pr(1), pr(2)])),
            )
            .mount(&server)
            .await;

        let a = gh.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap();
        let b = gh.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap();
        assert_eq!(a.items.len(), 2);
        assert_eq!(b.items.len(), 2);
        assert_eq!(
            list_gets(&server).await,
            1,
            "a repeat within the TTL is free"
        );

        expire_cached_for_tests(&server.uri());
        let c = gh.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap();
        assert_eq!(c.items.len(), 2, "the 304 serves the cached body");
        let reqs = server.received_requests().await.unwrap();
        let last = reqs.last().unwrap();
        assert_eq!(
            last.headers
                .get("if-none-match")
                .and_then(|v| v.to_str().ok()),
            Some("\"v1\""),
            "a stale entry revalidates with its ETag"
        );
        assert_eq!(list_gets(&server).await, 2);
    }

    #[tokio::test]
    async fn own_write_invalidates_the_repo_reads() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        enable_cache_for_tests(&server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([pr(1)])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/app/pulls/1/reviews"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        gh.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap();
        gh.approve(&rr(), 1).await.unwrap();
        gh.list_prs(&rr(), PrState::Open, 1, 50).await.unwrap();
        assert_eq!(
            list_gets(&server).await,
            2,
            "the approve cleared the cached list — no 15 s of stale state"
        );
    }

    #[tokio::test]
    async fn pr_detail_reopen_within_ttl_costs_no_rest_request() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        enable_cache_for_tests(&server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pr(7)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(
                r"^/repos/acme/app/(pulls|issues)/7/(comments|reviews)$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/repos/acme/app/commits/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        gh.get_pr(&rr(), 7).await.unwrap();
        let first = server.received_requests().await.unwrap().len();
        assert!(first >= 4, "detail + 3 lists (+ CI): {first}");
        gh.get_pr(&rr(), 7).await.unwrap();
        let second = server.received_requests().await.unwrap().len();
        assert_eq!(second, first, "a re-open within the TTL hits the cache");
    }

    #[tokio::test]
    async fn concurrent_cold_reads_share_one_request() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        enable_cache_for_tests(&server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(150))
                    .set_body_json(json!([pr(1)])),
            )
            .mount(&server)
            .await;
        let r = rr();
        let reads = (0..4).map(|_| gh.list_prs(&r, PrState::Open, 1, 50));
        for r in futures_util::future::join_all(reads).await {
            assert_eq!(r.unwrap().items.len(), 1);
        }
        assert_eq!(
            list_gets(&server).await,
            1,
            "four windows opening the list on a cold cache cost one GET"
        );
    }

    /// A read that left BEFORE our write must neither answer a read issued
    /// after it (single-flight join) nor put its pre-write body back into the
    /// cache the write cleared — the posted comment would stay hidden for the
    /// whole TTL.
    #[tokio::test]
    async fn read_in_flight_across_a_write_neither_serves_nor_stores_stale() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        enable_cache_for_tests(&server.uri());
        // The first GET (pre-write state) is slow; every later one is fresh.
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(400))
                    .set_body_json(json!([pr(1)])),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([pr(1), pr(2)])))
            .with_priority(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/app/pulls/1/reviews"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        let r = rr();
        let (stale, fresh) = tokio::join!(gh.list_prs(&r, PrState::Open, 1, 50), async {
            // Let the slow read reach the server before the write.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            gh.approve(&r, 1).await.unwrap();
            gh.list_prs(&r, PrState::Open, 1, 50).await
        });
        assert_eq!(stale.unwrap().items.len(), 1, "the slow read's own answer");
        assert_eq!(
            fresh.unwrap().items.len(),
            2,
            "a read after the write must not join the pre-write request"
        );
        assert_eq!(list_gets(&server).await, 2);

        let again = gh.list_prs(&r, PrState::Open, 1, 50).await.unwrap();
        assert_eq!(
            again.items.len(),
            2,
            "the pre-write body, landing last, must not overwrite the cache"
        );
        assert_eq!(list_gets(&server).await, 2, "served from the fresh entry");
    }

    /// The same race on the GraphQL review-thread memo: a probe in flight
    /// while a thread is resolved must not re-memoise the unresolved state.
    #[tokio::test]
    async fn thread_probe_in_flight_across_a_resolve_is_not_memoised() {
        use wiremock::matchers::body_string_contains;
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        enable_cache_for_tests(&server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pr(7)))
            .mount(&server)
            .await;
        // One inline comment, so the GraphQL resolution probe runs.
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 11, "body": "nit", "user": { "login": "rev" },
                "path": "a.rs", "line": 3, "created_at": "2026-09-01T10:00:00Z",
            }])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(
                r"^/repos/acme/app/(issues/7/comments|pulls/7/reviews)$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/repos/acme/app/commits/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(body_string_contains("resolveReviewThread"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {} })))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(600))
                    .set_body_json(json!({
                        "data": { "repository": { "pullRequest": { "reviewThreads": {
                            "nodes": [] } } } }
                    })),
            )
            .with_priority(2)
            .mount(&server)
            .await;
        let probes = || async {
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| {
                    r.url.path() == "/graphql"
                        && !String::from_utf8_lossy(&r.body).contains("resolveReviewThread")
                })
                .count()
        };

        let r = rr();
        let (probe, resolve) = tokio::join!(gh.get_pr(&r, 7), async {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            gh.resolve_pr_thread(&r, 7, "T1", true).await
        });
        probe.unwrap();
        resolve.unwrap();
        assert_eq!(probes().await, 1);
        gh.get_pr(&r, 7).await.unwrap();
        assert_eq!(
            probes().await,
            2,
            "the probe that straddled the resolve did not memoise its answer"
        );
    }

    #[tokio::test]
    async fn review_thread_probe_is_memoised_until_a_resolve() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        enable_cache_for_tests(&server.uri());
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pr(7)))
            .mount(&server)
            .await;
        // One inline comment, so the GraphQL resolution probe runs.
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "id": 11, "body": "nit", "user": { "login": "rev" },
                "path": "a.rs", "line": 3, "created_at": "2026-09-01T10:00:00Z",
            }])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(
                r"^/repos/acme/app/(issues/7/comments|pulls/7/reviews)$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/repos/acme/app/commits/.+"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "repository": { "pullRequest": { "reviewThreads": { "nodes": [
                    { "id": "T1", "isResolved": false,
                      "comments": { "nodes": [ { "databaseId": 11 } ] } }
                ] } } } }
            })))
            .mount(&server)
            .await;
        let posts = || async {
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/graphql")
                .count()
        };

        gh.get_pr(&rr(), 7).await.unwrap();
        assert_eq!(posts().await, 1);
        gh.get_pr(&rr(), 7).await.unwrap();
        assert_eq!(
            posts().await,
            1,
            "a re-open within the TTL makes no GraphQL POST"
        );

        gh.resolve_pr_thread(&rr(), 7, "T1", true).await.unwrap();
        assert_eq!(posts().await, 2, "the resolve mutation");
        gh.get_pr(&rr(), 7).await.unwrap();
        assert_eq!(posts().await, 3, "the resolve dropped the memo");
    }
}

// ---------------------------------------------------------------------------
// S15-10: merges pinned to the head the user reviewed
// ---------------------------------------------------------------------------

mod merge_pin {
    use super::*;
    use crate::providers::bitbucket::Bitbucket;
    use crate::providers::github::Github;
    use crate::providers::gitlab::Gitlab;
    use otto_core::api::MergeStrategy;
    use otto_core::Error;
    use wiremock::matchers::body_partial_json;

    #[test]
    fn check_head_is_prefix_aware_with_a_floor() {
        use crate::providers::check_head;
        let full = "0123456789abcdef0123456789abcdef01234567";
        assert!(check_head(Some(full), full).is_ok());
        assert!(
            check_head(Some("0123456789ab"), full).is_ok(),
            "bitbucket short"
        );
        assert!(
            check_head(Some(full), "0123456789AB").is_ok(),
            "case-insensitive"
        );
        assert!(matches!(
            check_head(Some("fedcba987654"), full),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(check_head(None, full), Err(Error::Conflict(_))));
        assert!(
            check_head(Some(full), "").is_err(),
            "an empty pin never matches"
        );
        assert!(
            check_head(Some(full), "0123").is_err(),
            "below the 7-char floor"
        );
    }

    /// GitHub gets the pin as the merge body's `sha`; its 409 "Head branch was
    /// modified" surfaces as the shared "PR changed — re-check" conflict.
    #[tokio::test]
    async fn github_forwards_sha_and_maps_head_moved() {
        let server = MockServer::start().await;
        let gh = Github::with_base("tok".into(), server.uri());
        Mock::given(method("PUT"))
            .and(path("/repos/acme/app/pulls/7/merge"))
            .and(body_partial_json(
                json!({"merge_method": "squash", "sha": "abc1234def"}),
            ))
            .respond_with(ResponseTemplate::new(409).set_body_json(
                json!({"message": "Head branch was modified. Review and try the merge again."}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let err = gh
            .merge_pinned(&rr(), 7, MergeStrategy::Squash, false, Some("abc1234def"))
            .await
            .unwrap_err();
        match err {
            Error::Conflict(m) => assert!(m.contains("PR changed"), "{m}"),
            other => panic!("expected 409, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn gitlab_forwards_sha_in_the_merge_body() {
        let server = MockServer::start().await;
        let gl = Gitlab::new("tok".into(), Some(server.uri()));
        Mock::given(method("PUT"))
            .and(path_regex(r"^/api/v4/projects/.+/merge_requests/5/merge$"))
            .and(body_partial_json(json!({"sha": "abc1234def"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        gl.merge_pinned(&rr(), 5, MergeStrategy::Merge, false, Some("abc1234def"))
            .await
            .unwrap();
    }

    /// S2-306: a pinned Rebase merge checks the reviewed head on an UNCACHED
    /// read, waits for the async rebase, then pins `/merge` to the head the
    /// rebase produced; a head that moved is refused before any rebase.
    #[tokio::test]
    async fn gitlab_rebase_checks_uncached_and_pins_the_rebased_head() {
        let reviewed = "abc1234def0123456789abcdef0123456789abcd";
        let rebased = "9999999999999999999999999999999999999999";
        let server = MockServer::start().await;
        let gl = Gitlab::new("tok".into(), Some(server.uri()));
        let mr = || path_regex(r"^/api/v4/projects/.+/merge_requests/5$");
        // 1st read: the pin check (pre-rebase head). 2nd: still rebasing.
        // Then: done, at the rebased head.
        Mock::given(method("GET"))
            .and(mr())
            .and(query_param("include_rebase_in_progress", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sha": reviewed})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(mr())
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"sha": reviewed, "rebase_in_progress": true})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(mr())
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"sha": rebased, "rebase_in_progress": false})),
            )
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(r"^/api/v4/projects/.+/merge_requests/5/rebase$"))
            .respond_with(ResponseTemplate::new(202).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(r"^/api/v4/projects/.+/merge_requests/5/merge$"))
            .and(body_partial_json(json!({"sha": rebased})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        gl.merge_pinned(&rr(), 5, MergeStrategy::Rebase, false, Some(reviewed))
            .await
            .unwrap();

        // A head that moved since review: refused, nothing is rebased.
        let server = MockServer::start().await;
        let gl = Gitlab::new("tok".into(), Some(server.uri()));
        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v4/projects/.+/merge_requests/5$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sha": rebased})))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(0)
            .mount(&server)
            .await;
        let res = gl
            .merge_pinned(&rr(), 5, MergeStrategy::Rebase, false, Some(reviewed))
            .await;
        assert!(matches!(res, Err(Error::Conflict(_))), "{res:?}");
    }

    /// Bitbucket has no forge-side pin: a moved head (abbreviated hash) is
    /// refused BEFORE the merge call; a matching prefix merges.
    #[tokio::test]
    async fn bitbucket_compares_the_abbreviated_head_before_merging() {
        for (head, merges) in [("fedcba987654", false), ("abc1234def01", true)] {
            let server = MockServer::start().await;
            let bb = Bitbucket::with_base("user".into(), "tok".into(), server.uri());
            Mock::given(method("GET"))
                .and(path("/repositories/acme/app/pullrequests/9"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"id": 9, "source": {"commit": {"hash": head}}})),
                )
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/repositories/acme/app/pullrequests/9/merge"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .expect(u64::from(merges))
                .mount(&server)
                .await;
            let full = "abc1234def0123456789abcdef0123456789abcd";
            let res = bb
                .merge_pinned(&rr(), 9, MergeStrategy::Merge, false, Some(full))
                .await;
            if merges {
                res.unwrap();
            } else {
                assert!(matches!(res, Err(Error::Conflict(_))), "{res:?}");
            }
        }
    }
}
