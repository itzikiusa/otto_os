use super::*;

async fn fixture() -> otto_state::DbPool {
    let pool = otto_state::db::test_pool().await;
    sqlx::query("INSERT INTO users(id,username,password_hash,is_root,created_at) VALUES('alice','alice','unused',1,'2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES('wa','wa','/tmp','2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
    for (id, status) in [("live", "running"), ("persisted", "exited")] {
        sqlx::query("INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at) VALUES(?,'wa','agent','claude','Owned assist',?,'/tmp','alice','2026-10-08T00:00:00Z','2026-10-08T00:00:00Z')")
            .bind(id).bind(status).execute(&pool).await.unwrap();
    }
    pool
}

#[tokio::test]
async fn owner_can_resume_live_and_persisted_sessions() {
    let pool = fixture().await;
    for id in ["live", "persisted"] {
        require_owned_resume(&pool, "wa", "alice", Some(&id.into()))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn absent_binding_retains_fresh_session_fallback() {
    let pool = fixture().await;
    require_owned_resume(&pool, "wa", "alice", None)
        .await
        .unwrap();
    require_owned_resume(&pool, "wa", "alice", Some(&"missing".into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn foreign_user_cannot_resume_live_or_persisted_session() {
    let pool = fixture().await;
    for id in ["live", "persisted"] {
        assert!(matches!(
            require_owned_resume(&pool, "wa", "bob", Some(&id.into())).await,
            Err(Error::Forbidden(_))
        ));
    }
}

#[tokio::test]
async fn foreign_workspace_cannot_resume_live_or_persisted_session() {
    let pool = fixture().await;
    for id in ["live", "persisted"] {
        assert!(matches!(
            require_owned_resume(&pool, "wb", "alice", Some(&id.into())).await,
            Err(Error::Forbidden(_))
        ));
    }
}
