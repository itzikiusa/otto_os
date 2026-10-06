//! Review-comment state machine over real HTTP (S15-07): a DECLINED comment is
//! never approvable — a decline that lands while "Post all" walks its snapshot
//! must not be posted anyway. Restoring it to draft is the way back.
use otto_core::domain::CommentSeverity;
use otto_server::ServerCtx;
use otto_state::{DbPool, NewRepo};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

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

#[tokio::test]
async fn approving_a_declined_comment_is_a_conflict_until_restored() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let owner = users
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&owner.id)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("Review states", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let checkout = tmp.path().join("repo");
    std::fs::create_dir_all(&checkout).unwrap();
    let repo = ctx
        .git_store
        .create_repo(NewRepo {
            workspace_id: ws.id.clone(),
            name: "repo".into(),
            path: checkout.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    // A LOCAL review (pr #0): approving never calls a forge.
    let review = ctx.reviews_store.create_review(&repo.id, 0).await.unwrap();
    let comment = ctx
        .reviews_store
        .add_comment(
            &review.id,
            Some("a.rs"),
            Some(3),
            CommentSeverity::Info,
            "nit",
        )
        .await
        .unwrap();

    // The review/run routes live in the module routers, not the core router.
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let url = |tail: &str| format!("{origin}/pr-review-comments/{}{tail}", comment.id);

    let declined = client
        .post(url("/decline"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(declined.status(), 200);

    let approve = client
        .post(url("/approve"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(approve.status(), 409, "approving a declined comment");
    let still = ctx.reviews_store.get_comment(&comment.id).await.unwrap();
    assert_eq!(still.state, otto_core::domain::CommentState::Declined);
    assert!(!still.posted);

    let restored = client
        .patch(url(""))
        .bearer_auth(&token)
        .json(&json!({"restore_draft": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(restored.status(), 200);
    let approved: Value = client
        .post(url("/approve"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(approved["state"], "approved");
    assert_eq!(approved["posted"], false, "a local review never posts");

    // S15-302: the retry guard found the copy a failed attempt created on the
    // PR — record it posted (nothing is sent) so it stops being postable.
    let mark = |c: &reqwest::Client| {
        c.patch(url(""))
            .bearer_auth(&token)
            .json(&json!({"mark_posted": true}))
            .send()
    };
    let marked: Value = mark(&client).await.unwrap().json().await.unwrap();
    assert_eq!(marked["state"], "approved");
    assert_eq!(marked["posted"], true);
    assert_eq!(
        mark(&client).await.unwrap().status(),
        409,
        "an already-posted comment can't be marked again"
    );
    let draft = ctx
        .reviews_store
        .add_comment(&review.id, None, None, CommentSeverity::Info, "draft")
        .await
        .unwrap();
    let draft_mark = client
        .patch(format!("{origin}/pr-review-comments/{}", draft.id))
        .bearer_auth(&token)
        .json(&json!({"mark_posted": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(draft_mark.status(), 409, "a draft was never attempted");
    assert!(
        !ctx.reviews_store
            .get_comment(&draft.id)
            .await
            .unwrap()
            .posted
    );
    server.abort();
}
