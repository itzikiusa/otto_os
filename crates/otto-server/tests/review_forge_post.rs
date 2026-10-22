//! Approving a PR review comment POSTS it to the forge (test-quality-4-02).
//! The real daemon router on a loopback port, a GitLab git account whose
//! `api_base_url` is a wiremock server, and a review of MR `!7`.
//!
//! What it pins (the composition in `review/routes.rs` `approve_comment` +
//! `post_review_comment`, not just its pure helpers):
//! - a 201 marks the comment posted after exactly ONE forge request, and a
//!   second approve sends nothing (the posted 0→1 claim);
//! - a 500 leaves it unposted (never "posted" on a failed call), releases the
//!   claim, and a later approve posts it;
//! - a rejected inline anchor (GitLab 400 on the positioned discussion) falls
//!   back to ONE general note that cites `a.rs:3`.
use otto_core::domain::{CommentSeverity, GitProviderKind, ReviewComment};
use otto_server::ServerCtx;
use otto_state::{DbPool, NewGitAccount, NewRepo};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MR: &str = "/api/v4/projects/team%2Fapp/merge_requests/7";

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

struct Forge {
    ctx: ServerCtx,
    review_id: String,
    origin: String,
    token: String,
    http: reqwest::Client,
    gitlab: MockServer,
    _tmp: tempfile::TempDir,
}

impl Forge {
    async fn comment(&self, path: Option<&str>, line: Option<u32>) -> ReviewComment {
        self.ctx
            .reviews_store
            .add_comment(&self.review_id, path, line, CommentSeverity::Info, "nit")
            .await
            .unwrap()
    }

    async fn approve(&self, comment: &ReviewComment) -> Value {
        let resp = self
            .http
            .post(format!(
                "{}/pr-review-comments/{}/approve",
                self.origin, comment.id
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        resp.json().await.unwrap()
    }

    async fn posts(&self) -> Vec<wiremock::Request> {
        self.gitlab
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.method.as_str() == "POST")
            .collect()
    }
}

async fn boot() -> Forge {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let owner = otto_state::UsersRepo::new(pool.clone())
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&owner.id)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("Forge post", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();

    let gitlab = MockServer::start().await;
    ctx.secrets.put("gl-forge-post", "glpat-test").unwrap();
    let account = ctx
        .git_store
        .create_account(NewGitAccount {
            user_id: owner.id.clone(),
            provider: GitProviderKind::Gitlab,
            label: "gitlab".into(),
            username: "owner".into(),
            token_ref: "gl-forge-post".into(),
            api_base_url: Some(gitlab.uri()),
            namespace: None,
            token_expires_at: None,
        })
        .await
        .unwrap();

    // The live origin is the authority for the destination (refresh_remote),
    // so the checkout carries the same remote the repo row records.
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
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
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
    let review = ctx.reviews_store.create_review(&repo.id, 7).await.unwrap();

    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/api/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Forge {
        ctx,
        review_id: review.id,
        origin,
        token,
        http: reqwest::Client::new(),
        gitlab,
        _tmp: tmp,
    }
}

fn note() -> ResponseTemplate {
    ResponseTemplate::new(201).set_body_json(json!({"id": 11, "body": "nit"}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_201_posts_once_and_a_second_approve_sends_nothing() {
    let f = boot().await;
    Mock::given(method("POST"))
        .and(path(format!("{MR}/notes")))
        .respond_with(note())
        .expect(1)
        .mount(&f.gitlab)
        .await;
    let c = f.comment(None, None).await;

    let first = f.approve(&c).await;
    assert_eq!(first["state"], "approved");
    assert_eq!(first["posted"], true);
    assert_eq!(f.posts().await.len(), 1);

    let again = f.approve(&c).await;
    assert_eq!(again["posted"], true);
    assert_eq!(
        f.posts().await.len(),
        1,
        "an already-posted comment is never re-sent"
    );
    f.gitlab.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_500_stays_unposted_and_a_later_approve_posts() {
    let f = boot().await;
    // Mount order is match priority: the 500 answers the first POST only.
    Mock::given(method("POST"))
        .and(path(format!("{MR}/notes")))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .expect(1)
        .mount(&f.gitlab)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{MR}/notes")))
        .respond_with(note())
        .expect(1)
        .mount(&f.gitlab)
        .await;
    let c = f.comment(None, None).await;

    let failed = f.approve(&c).await;
    assert_eq!(failed["state"], "approved");
    assert_eq!(
        failed["posted"], false,
        "a failed post is never recorded as posted"
    );
    assert_eq!(f.posts().await.len(), 1, "a write is not retried on a 5xx");
    let stored = f.ctx.reviews_store.get_comment(&c.id).await.unwrap();
    assert!(!stored.posted, "the claim was released");

    let retried = f.approve(&c).await;
    assert_eq!(retried["posted"], true);
    assert_eq!(f.posts().await.len(), 2);
    f.gitlab.verify().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_anchor_falls_back_to_a_general_note_citing_the_line() {
    let f = boot().await;
    Mock::given(method("GET"))
        .and(path(MR))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "iid": 7,
            "diff_refs": {"base_sha": "b", "start_sha": "s", "head_sha": "h"}
        })))
        .mount(&f.gitlab)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{MR}/discussions")))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"message": "400 Bad request - line_code is invalid"})),
        )
        .expect(1)
        .mount(&f.gitlab)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{MR}/notes")))
        .and(body_string_contains("a.rs:3"))
        .respond_with(note())
        .expect(1)
        .mount(&f.gitlab)
        .await;
    let c = f.comment(Some("a.rs"), Some(3)).await;

    let posted = f.approve(&c).await;
    assert_eq!(
        posted["posted"], true,
        "an approved finding is not silently dropped"
    );
    let posts = f.posts().await;
    assert_eq!(
        posts.len(),
        2,
        "the positioned discussion, then one general note"
    );
    assert!(posts[0].url.path().ends_with("/discussions"));
    assert!(posts[1].url.path().ends_with("/notes"));
    let body: Value = serde_json::from_slice(&posts[1].body).unwrap();
    assert!(
        body["body"].as_str().unwrap().contains("a.rs:3"),
        "the fallback cites the location: {body}"
    );
    f.gitlab.verify().await;
}
