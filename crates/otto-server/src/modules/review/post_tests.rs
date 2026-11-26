//! `post_one` with a workflow review (stored as PR 0) and the run's real PR
//! number: the forge is a wiremock GitLab, the review of MR `!7`.
use super::*;
use otto_core::domain::{CommentSeverity, GitProviderKind};
use otto_state::{NewGitAccount, NewRepo};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MR: &str = "/api/v4/projects/team%2Fapp/merge_requests/7";

struct Rig {
    ctx: ServerCtx,
    user: otto_core::domain::User,
    review_id: Id,
    checkout: std::path::PathBuf,
    gitlab: MockServer,
    _tmp: tempfile::TempDir,
}

async fn rig() -> Rig {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = otto_state::db::test_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let user = otto_state::UsersRepo::new(pool.clone())
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("Post", tmp.path().to_str().unwrap(), &user.id)
        .await
        .unwrap();
    let gitlab = MockServer::start().await;
    ctx.secrets.put("gl-post-one", "glpat-test").unwrap();
    let account = ctx
        .git_store
        .create_account(NewGitAccount {
            user_id: user.id.clone(),
            provider: GitProviderKind::Gitlab,
            label: "gitlab".into(),
            username: "owner".into(),
            token_ref: "gl-post-one".into(),
            api_base_url: Some(gitlab.uri()),
            namespace: None,
            token_expires_at: None,
        })
        .await
        .unwrap();
    let remote = "ssh://git@gitlab.ssh.example/team/app.git";
    let checkout = tmp.path().join("repo");
    std::fs::create_dir_all(&checkout).unwrap();
    for args in [
        &["init", "-q"][..],
        &["remote", "add", "origin", remote][..],
    ] {
        let out = std::process::Command::new("git")
            .current_dir(&checkout)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let repo = ctx
        .git_store
        .create_repo(NewRepo {
            workspace_id: ws.id.clone(),
            name: "app".into(),
            path: checkout.to_string_lossy().into_owned(),
            remote_url: Some(remote.into()),
            provider: Some(GitProviderKind::Gitlab),
            git_account_id: Some(account.id.clone()),
        })
        .await
        .unwrap();
    // A workflow review: stored as the local sentinel, its PR comes from the run.
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    Rig {
        ctx,
        user,
        review_id: review.id,
        checkout,
        gitlab,
        _tmp: tmp,
    }
}

impl Rig {
    async fn comment(&self) -> Id {
        self.ctx
            .reviews_store
            .add_comment(
                &self.review_id,
                None,
                None,
                CommentSeverity::Warn,
                "unchecked error",
            )
            .await
            .unwrap()
            .id
    }
    async fn post(&self, id: &Id) -> PostOutcome {
        post_one(&self.ctx, &self.user, id, 7, false).await.unwrap()
    }
    async fn posts(&self) -> usize {
        let all = self.gitlab.received_requests().await.unwrap_or_default();
        all.iter().filter(|r| r.method.as_str() == "POST").count()
    }
}

fn note(status: u16) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(serde_json::json!({"id": 11, "body": "x"}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_workflow_review_posts_to_the_runs_pr_once_without_learning() {
    let r = rig().await;
    Mock::given(method("POST"))
        .and(path(format!("{MR}/notes")))
        .respond_with(note(201))
        .mount(&r.gitlab)
        .await;
    let id = r.comment().await;
    let PostOutcome::Posted(c) = r.post(&id).await else {
        panic!("posted to PR 7 although the review is stored as PR 0");
    };
    assert!(c.posted && c.state == CommentState::Approved);
    assert!(matches!(r.post(&id).await, PostOutcome::AlreadyPosted(_)));
    assert_eq!(r.posts().await, 1, "never re-posted");
    // An automated post is not a person's approval: no review file appended.
    assert!(!r.checkout.join(".otto").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_post_returns_the_forge_error_and_can_be_posted_later() {
    let r = rig().await;
    let refused = Mock::given(method("POST"))
        .and(path(format!("{MR}/notes")))
        .respond_with(note(500))
        .mount_as_scoped(&r.gitlab)
        .await;
    let id = r.comment().await;
    let PostOutcome::Failed(c, why) = r.post(&id).await else {
        panic!("a 500 is not posted");
    };
    assert!(!c.posted && c.state == CommentState::Approved);
    assert!(
        !why.trim().is_empty(),
        "the forge's error text reaches the caller"
    );
    drop(refused);
    Mock::given(method("POST"))
        .and(path(format!("{MR}/notes")))
        .respond_with(note(201))
        .mount(&r.gitlab)
        .await;
    assert!(
        matches!(r.post(&id).await, PostOutcome::Posted(_)),
        "the claim was released"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_declined_comment_is_never_posted() {
    let r = rig().await;
    let id = r.comment().await;
    r.ctx
        .reviews_store
        .set_comment_state(&id, CommentState::Declined, false)
        .await
        .unwrap();
    assert!(matches!(r.post(&id).await, PostOutcome::Declined));
    assert_eq!(r.posts().await, 0);
}
