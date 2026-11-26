//! `post_review_comments` against a wiremock GitLab MR — test-plan.md §5.13,
//! DR-004: the PR number from `prs.json`, inline / info / summary, dedupe,
//! `max_inline`, the local-review and failure cases.
use super::super::script::tests::NodeFixture;
use crate::workflow_engine::*;
use otto_core::domain::{CommentSeverity, CommentState};
use otto_core::workflows::NodeRunState;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MR: &str = "/api/v4/projects/team%2Fapp/merge_requests/7";

struct Post {
    f: NodeFixture,
    forge: MockServer,
    repo_id: Id,
    review_id: Id,
}

async fn post_fixture(posting: u16) -> Post {
    let f = NodeFixture::new().await;
    let main = f.dir.path().join("main");
    std::fs::create_dir_all(&main).unwrap();
    for args in [
        &["init", "-q"][..],
        &[
            "remote",
            "add",
            "origin",
            "ssh://git@gitlab.ssh.example/team/app.git",
        ][..],
    ] {
        #[allow(clippy::disallowed_methods)]
        let ok = otto_git::hardened_std_command()
            .args(args)
            .current_dir(&main)
            .status()
            .unwrap();
        assert!(ok.success());
    }
    let forge = MockServer::start().await;
    otto_core::secrets::put_async(&f.ctx.secrets, "gl-post", "glpat-test")
        .await
        .unwrap();
    let account = f
        .ctx
        .git_store
        .create_account(otto_state::NewGitAccount {
            user_id: f.user.id.clone(),
            provider: otto_core::domain::GitProviderKind::Gitlab,
            label: "gitlab".into(),
            username: "owner".into(),
            token_ref: "gl-post".into(),
            api_base_url: Some(forge.uri()),
            namespace: None,
            token_expires_at: None,
        })
        .await
        .unwrap();
    let repo = f
        .ctx
        .git_store
        .create_repo(otto_state::NewRepo {
            workspace_id: f.ws.id.clone(),
            name: "app".into(),
            path: main.to_string_lossy().into(),
            remote_url: Some("ssh://git@gitlab.ssh.example/team/app.git".into()),
            provider: Some(otto_core::domain::GitProviderKind::Gitlab),
            git_account_id: Some(account.id.clone()),
        })
        .await
        .unwrap();
    // A workflow review: stored as a local review (PR 0).
    let review = f
        .ctx
        .reviews_store
        .create_review(&repo.id, 0)
        .await
        .unwrap();
    let mr = json!({"iid": 7, "title": "t", "state": "opened", "source_branch": "f", "target_branch": "develop",
                    "updated_at": "2026-11-01T00:00:00Z", "web_url": "https://gitlab.example/team/app/-/merge_requests/7",
                    "diff_refs": {"base_sha": "a", "start_sha": "a", "head_sha": "b"}});
    let existing = json!([{"id": "d0", "notes": [{"id": 1, "body": "Already there\nold text", "author": {"name": "x"},
        "created_at": "2026-11-01T00:00:00Z", "position": {"new_path": "d.rs", "new_line": 9}}]}]);
    for (p, body) in [
        (MR.to_string(), mr),
        (format!("{MR}/discussions"), existing),
        (format!("{MR}/approvals"), json!({})),
        ("/api/v4/user".to_string(), json!({"id": 1})),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&forge)
            .await;
    }
    let note = json!({"id": 11, "body": "x", "author": {"name": "bot"}, "created_at": "2026-11-01T00:00:00Z"});
    for p in [format!("{MR}/discussions"), format!("{MR}/notes")] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(posting)
                    .set_body_json(json!({"id": "d1", "notes": [note.clone()], "body": "x"})),
            )
            .mount(&forge)
            .await;
    }
    Post {
        f,
        forge,
        repo_id: repo.id,
        review_id: review.id,
    }
}

impl Post {
    async fn draft(
        &self,
        sev: CommentSeverity,
        path: Option<&str>,
        line: Option<u32>,
        body: &str,
    ) -> Id {
        self.f
            .ctx
            .reviews_store
            .add_comment(&self.review_id, path, line, sev, body)
            .await
            .unwrap()
            .id
    }

    /// The run so far: a finished review step (and a "Gotchas" step file).
    async fn seed_run(&self, outcome: &str, with_pr: bool) {
        let review = NodeRunState {
            node_id: "n5".into(),
            status: otto_core::workflows::NodeStatus::Success,
            output: Some(
                json!({"review_id": self.review_id, "repo_id": self.repo_id, "passed": true,
                                "score": 91, "score_text": "91", "threshold": 80, "outcome": outcome}),
            ),
            step_no: Some(5),
            ..Default::default()
        };
        let intro = NodeRunState {
            node_id: "n13".into(),
            status: otto_core::workflows::NodeStatus::Success,
            step_no: Some(6),
            step_file: Some("step6-draft-pr-summary".into()),
            ..Default::default()
        };
        self.f
            .files
            .write_named("step6-draft-pr-summary.md", "## Gotchas\nWatch the cache.");
        self.f
            .files
            .write_named("go-standards-evals.md", "| gate | ok |");
        sqlx::query("UPDATE workflow_runs SET nodes_json = ? WHERE id = ?")
            .bind(serde_json::to_string(&vec![review, intro]).unwrap())
            .bind(&self.f.run_id)
            .execute(&self.f.ctx.pool)
            .await
            .unwrap();
        if with_pr {
            self.f.files.write_named("prs.json", &json!([{"repo_id": self.repo_id, "repo_path": "/x", "number": 7,
                "head": "f", "base": "develop", "url": "https://gitlab.example/team/app/-/merge_requests/7"}]).to_string());
        }
    }

    async fn run(&self, params: Value) -> Result<(Value, Vec<String>)> {
        self.f
            .run(json!({"id": "n6", "kind": "post_review_comments", "name": "Post", "params": params}), json!({}))
            .await
    }

    async fn posts(&self, suffix: &str) -> Vec<String> {
        self.forge
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path().ends_with(suffix))
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }
}

#[tokio::test]
async fn posts_inline_info_and_summary_once_with_the_run_pr_number() {
    let p = post_fixture(201).await;
    let bug = p
        .draft(
            CommentSeverity::Bug,
            Some("a.rs"),
            Some(3),
            "Null deref\ndetails",
        )
        .await;
    p.draft(CommentSeverity::Warn, Some("b.rs"), Some(5), "Slow loop")
        .await;
    p.draft(CommentSeverity::Info, Some("c.rs"), Some(1), "Naming nit")
        .await;
    p.draft(
        CommentSeverity::Bug,
        Some("d.rs"),
        Some(9),
        "Already there\nnew wording",
    )
    .await;
    p.seed_run("passed", true).await;
    let params = json!({"review_from": "n5", "summary_intro_from": "n13", "summary_files": ["go-standards-evals.md"]});
    let (out, _) = p.run(params.clone()).await.unwrap();
    assert_eq!(
        (out["posted"].as_u64(), out["duplicates"].as_u64()),
        (Some(3), Some(1)),
        "{out}"
    );
    assert_eq!(
        out["pr"]["number"], 7,
        "the PR from prs.json, never the review's PR 0"
    );
    assert_eq!(out["passed"], true, "the review's output is forwarded");
    let inline = p.posts("/discussions").await;
    assert_eq!(inline.len(), 2, "bug + warn inline, the duplicate skipped");
    let notes = p.posts("/notes").await;
    assert_eq!(notes.len(), 2, "one combined info comment + one summary");
    assert!(notes[0].contains("Naming nit"), "{}", notes[0]);
    let summary = &notes[1];
    for part in [
        "Gotchas",
        "score 91/100",
        "pass",
        "merge_requests/7",
        "| gate | ok |",
        "1 info",
    ] {
        assert!(summary.contains(part), "summary misses {part}: {summary}");
    }
    let c = p.f.ctx.reviews_store.get_comment(&bug).await.unwrap();
    assert_eq!((c.state, c.posted), (CommentState::Approved, true));
    assert_eq!(out["deliverables"][0]["detail"], "3 posted");
    // Again (a resumed step): nothing is posted twice.
    let (out, _) = p.run(params).await.unwrap();
    assert_eq!(out["posted"], 0);
    assert_eq!(
        (
            p.posts("/discussions").await.len(),
            p.posts("/notes").await.len()
        ),
        (2, 2)
    );
}

#[tokio::test]
async fn max_inline_moves_the_rest_into_one_comment_and_info_can_be_skipped() {
    let p = post_fixture(201).await;
    for (i, f) in ["a.rs", "b.rs", "c.rs"].iter().enumerate() {
        p.draft(
            CommentSeverity::Bug,
            Some(f),
            Some(i as u32 + 1),
            &format!("bug {i}"),
        )
        .await;
    }
    p.draft(CommentSeverity::Info, Some("x.rs"), Some(1), "info")
        .await;
    p.seed_run("passed", true).await;
    let (out, _) = p
        .run(json!({"max_inline": 1, "info": "skip", "summary": false}))
        .await
        .unwrap();
    assert_eq!(
        (out["posted"].as_u64(), out["overflow"].as_u64()),
        (Some(2), Some(2)),
        "{out}"
    );
    assert_eq!(p.posts("/discussions").await.len(), 1);
    let notes = p.posts("/notes").await;
    assert_eq!(
        notes.len(),
        1,
        "one More findings comment, no info, no summary"
    );
    assert!(
        notes[0].contains("More findings (2)") && !notes[0].contains("info"),
        "{}",
        notes[0]
    );
}

#[tokio::test]
async fn local_review_posts_nothing_and_incomplete_review_is_an_error() {
    let p = post_fixture(201).await;
    p.draft(CommentSeverity::Bug, Some("a.rs"), Some(3), "bug")
        .await;
    p.seed_run("passed", false).await;
    let (out, _) = p.run(json!({})).await.unwrap();
    assert_eq!(out["deliverables"][0]["detail"], "no PR (local review)");
    assert!(
        p.forge
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "no forge call"
    );
    p.seed_run("incomplete", true).await;
    let err = p.run(json!({})).await.expect_err("incomplete");
    assert!(
        err.to_string().contains("nothing to post")
            && err.to_string().contains("no PR comment was posted"),
        "{err}"
    );
}

#[tokio::test]
async fn every_post_failing_is_an_error_and_releases_the_claims() {
    let p = post_fixture(500).await;
    let id = p
        .draft(CommentSeverity::Bug, Some("a.rs"), Some(3), "bug")
        .await;
    p.seed_run("passed", true).await;
    let err = p
        .run(json!({"summary": false}))
        .await
        .expect_err("all failed");
    assert!(err.to_string().contains("every post failed"), "{err}");
    let c = p.f.ctx.reviews_store.get_comment(&id).await.unwrap();
    assert!(
        !c.posted && c.state == CommentState::Draft,
        "a later run can post it"
    );
    let _ = body_string_contains("x");
}

#[tokio::test]
async fn a_posted_summary_keeps_the_step_ok_when_inline_posts_fail_and_a_clean_pr_shows_only_the_summary(
) {
    // Inline posts fail (5xx, never retried), the summary note posts.
    let p = post_fixture(201).await;
    Mock::given(method("POST"))
        .and(path(format!("{MR}/discussions")))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&p.forge)
        .await;
    p.draft(CommentSeverity::Bug, Some("a.rs"), Some(3), "bug")
        .await;
    p.seed_run("passed", true).await;
    let (out, logs) = p.run(json!({})).await.expect("the summary posted");
    assert_eq!(
        (out["posted"].as_u64(), out["failed"].as_u64()),
        (Some(0), Some(1)),
        "{out}"
    );
    let states: Vec<(&str, &str)> = out["deliverables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["label"].as_str().unwrap(), d["state"].as_str().unwrap()))
        .collect();
    assert_eq!(
        states,
        [("PR comments", "failed"), ("PR summary comment", "done")],
        "{out}"
    );
    assert!(logs.iter().any(|l| l.starts_with("⚠")), "{logs:?}");

    // No findings at all: the summary is the only deliverable.
    let p = post_fixture(201).await;
    p.seed_run("passed", true).await;
    let (out, _) = p.run(json!({})).await.unwrap();
    let labels: Vec<&str> = out["deliverables"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["label"].as_str())
        .collect();
    assert_eq!(labels, ["PR summary comment"], "{out}");
}

#[tokio::test]
async fn summary_files_outside_the_bare_names_are_never_posted() {
    let p = post_fixture(201).await;
    p.seed_run("passed", true).await;
    let dir = p.f.files.dir_str().unwrap();
    std::fs::create_dir_all(format!("{dir}/attachments")).unwrap();
    std::fs::write(format!("{dir}/attachments/internal.txt"), "INTERNAL").unwrap();
    p.run(json!({"summary_files": ["attachments/internal.txt", "go-standards-evals.md"]}))
        .await
        .unwrap();
    let notes = p.posts("/notes").await;
    let summary = notes.last().expect("a summary");
    assert!(summary.contains("| gate | ok |"), "{summary}");
    assert!(!summary.contains("INTERNAL"), "{summary}");
}

/// CR1-052 (4): a bare summary-file name that is a symlink out of the run
/// dir is never read into the PR summary.
#[cfg(unix)]
#[tokio::test]
async fn a_bare_summary_file_name_that_links_out_of_the_run_dir_is_never_posted() {
    let p = post_fixture(201).await;
    p.seed_run("passed", true).await;
    let dir = p.f.files.dir_str().unwrap();
    let secret = p.f.dir.path().join("secret.md");
    std::fs::write(&secret, "OUTSIDE").unwrap();
    std::os::unix::fs::symlink(&secret, format!("{dir}/leak.md")).unwrap();
    p.run(json!({"summary_files": ["leak.md", "go-standards-evals.md"]}))
        .await
        .unwrap();
    let notes = p.posts("/notes").await;
    let summary = notes.last().expect("a summary");
    assert!(summary.contains("| gate | ok |"), "{summary}");
    assert!(!summary.contains("OUTSIDE"), "{summary}");
}

/// CR1-052 (4): one inline post failing and one landing is a partial
/// delivery, not done and not failed.
#[tokio::test]
async fn one_inline_post_failing_of_two_is_a_partial_delivery() {
    let p = post_fixture(201).await;
    Mock::given(method("POST"))
        .and(path(format!("{MR}/discussions")))
        .and(body_string_contains("bug A"))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&p.forge)
        .await;
    p.draft(CommentSeverity::Bug, Some("a.rs"), Some(3), "bug A")
        .await;
    p.draft(CommentSeverity::Bug, Some("b.rs"), Some(4), "bug B")
        .await;
    p.seed_run("passed", true).await;
    let (out, _) = p.run(json!({"summary": false})).await.unwrap();
    assert_eq!(
        (out["posted"].as_u64(), out["failed"].as_u64()),
        (Some(1), Some(1)),
        "{out}"
    );
    let d = &out["deliverables"][0];
    assert_eq!(
        (d["label"].as_str(), d["state"].as_str()),
        (Some("PR comments"), Some("partial")),
        "{out}"
    );
}
