use super::*;
use otto_core::domain::{Capability, Feature};
use otto_state::WorkflowsRepo;

#[tokio::test]
async fn search_respects_each_sources_feature_grant() {
    let pool = otto_state::db::test_pool().await;
    let dir = tempfile::tempdir().unwrap();
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('reader','reader','unused',0,'2026-10-08T00:00:00Z')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES('ws','Fixture',?,'2026-10-08T00:00:00Z')")
        .bind(dir.path().to_str().unwrap()).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workspace_members(workspace_id,user_id,role) VALUES('ws','reader','viewer')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let user = otto_state::UsersRepo::new(pool.clone())
        .get(&"reader".into())
        .await
        .unwrap();
    let grants = otto_state::GrantsRepo::new(pool.clone());
    grants
        .set_grants(
            &user.id,
            &[
                (Feature::Agents, Capability::View),
                (Feature::Workflows, Capability::None),
            ],
        )
        .await
        .unwrap();
    let wf = WorkflowsRepo::new(pool.clone())
        .create(
            &"ws".into(),
            "Sensitive workflow",
            "",
            "",
            &Default::default(),
            &user.id,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('other','other','unused',0,'2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
    let memories = MemoriesRepo::new(pool.clone());
    let mut ids = vec![];
    for (title, visibility, creator) in [
        ("Sensitive shared", "shared", "other"),
        ("Sensitive private", "private", "other"),
        ("Sensitive owned", "private", "reader"),
    ] {
        let new = serde_json::from_value(serde_json::json!({"scope":"workspace","kind":"note","title":title,"body":title,"source_kind":"manual","visibility":visibility})).unwrap();
        ids.push(memories.create("ws", creator, new).await.unwrap().id);
    }
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let read = |user| {
        search(
            Path("ws".into()),
            State(ctx.clone()),
            CurrentUser(user),
            Query(SearchParams {
                q: "Sensitive".into(),
            }),
        )
    };
    let hidden = read(user.clone()).await.unwrap().0;
    assert!(
        !hidden.iter().any(|h| h.id == wf.id),
        "Agents View cannot reveal denied Workflows metadata"
    );
    assert!(
        hidden.iter().all(|h| !ids.contains(&h.id)),
        "denied Product source hides even owned/shared memory titles"
    );
    grants
        .set_grants(
            &user.id,
            &[
                (Feature::Agents, Capability::View),
                (Feature::Workflows, Capability::View),
                (Feature::Product, Capability::View),
            ],
        )
        .await
        .unwrap();
    let visible = read(user.clone()).await.unwrap().0;
    assert!(visible.iter().any(|h| h.id == wf.id));
    assert!(
        visible.iter().any(|h| h.id == ids[0]),
        "shared memory is visible"
    );
    assert!(
        !visible.iter().any(|h| h.id == ids[1]),
        "foreign private memory title is hidden"
    );
    assert!(
        visible.iter().any(|h| h.id == ids[2]),
        "owned private memory remains visible"
    );
    let mut root = user;
    root.is_root = true;
    let root_rows = read(root).await.unwrap().0;
    assert!(root_rows.iter().any(|h| h.id == wf.id));
    assert!(
        root_rows.iter().any(|h| h.id == ids[1]),
        "root retains existing visibility bypass"
    );
}
