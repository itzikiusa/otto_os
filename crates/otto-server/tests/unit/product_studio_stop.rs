use super::*;
use axum::{
    extract::{Path, State},
    Json,
};
use otto_product::http::CurrentUser;

#[tokio::test]
async fn product_stop_preserves_editor_boundary_and_root_access() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = crate::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let owner = users
        .create("stop-owner", "x", "Owner", false)
        .await
        .unwrap();
    let viewer = users
        .create("stop-viewer", "x", "Viewer", false)
        .await
        .unwrap();
    let root = users.create("stop-root", "x", "Root", true).await.unwrap();
    let outsider = users
        .create("stop-other", "x", "Other", false)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("Stop fixture", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    ctx.workspaces
        .set_member(&ws.id, &viewer.id, otto_core::domain::WorkspaceRole::Viewer)
        .await
        .unwrap();
    let story = ctx
        .product
        .create_draft(&ws.id, &owner.id, Some("Stop fixture"))
        .await
        .unwrap();
    let chat = otto_product::chat::create_chat(
        Path(story.story.id.clone()),
        State(ctx.clone()),
        CurrentUser(owner.clone()),
        None,
    )
    .await
    .unwrap()
    .0;
    let thread = otto_product::refine::create_thread(
        Path(story.story.id),
        State(ctx.clone()),
        CurrentUser(owner.clone()),
        None,
    )
    .await
    .unwrap()
    .0;
    for (user, allowed) in [
        (owner, true),
        (root, true),
        (viewer, false),
        (outsider, false),
    ] {
        let req = || {
            Json(otto_product::chat::StopTurnReq {
                turn_id: "not-running".into(),
            })
        };
        let chat_result = otto_product::chat::stop_turn(
            Path(chat.id.clone()),
            State(ctx.clone()),
            CurrentUser(user.clone()),
            req(),
        )
        .await;
        let refine_result = otto_product::refine::stop_turn(
            Path(thread.id.clone()),
            State(ctx.clone()),
            CurrentUser(user),
            req(),
        )
        .await;
        if allowed {
            assert!(!chat_result.unwrap().0.stopped);
            assert!(!refine_result.unwrap().0.stopped);
        } else {
            assert!(matches!(
                chat_result.unwrap_err().0,
                otto_core::Error::Forbidden(_)
            ));
            assert!(matches!(
                refine_result.unwrap_err().0,
                otto_core::Error::Forbidden(_)
            ));
        }
    }
}
