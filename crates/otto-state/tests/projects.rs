use otto_core::{domain::SessionKind, Error};
use otto_state::{
    projects::{ProjectInput, ProjectsRepo},
    sessions::{NewSession, SessionsRepo},
    swarm::{NewProject, SwarmRepo},
};
use serde_json::json;
use sqlx::SqlitePool;

async fn fixture() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,created_at) VALUES ('user','user','x','User','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
    for ws in ["one", "two"] {
        sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES (?,?,'/tmp','2026-01-01T00:00:00Z')").bind(ws).bind(ws).execute(&pool).await.unwrap();
    }
    pool
}
fn input() -> ProjectInput {
    ProjectInput {
        name: "Shared release".into(),
        goal_md: "Ship the release".into(),
        instructions_md: "Run focused tests".into(),
        references: vec!["vault://guide".into()],
        memory_md: "Use UTC".into(),
        ..Default::default()
    }
}
#[tokio::test]
async fn projects_share_existing_swarm_identity_and_preserve_execution_fields() {
    let pool = fixture().await;
    let swarm = SwarmRepo::new(pool.clone());
    let old = swarm
        .create_project(NewProject {
            swarm_id: "team".into(),
            workspace_id: "one".into(),
            name: "Existing".into(),
            description: "old".into(),
            repo_path: Some("/repo".into()),
            goal_md: Some("goal".into()),
            story_id: None,
            order_idx: 4,
            created_by: "user".into(),
        })
        .await
        .unwrap();
    let repo = ProjectsRepo::new(pool);
    let shared = repo.get(&old.id).await.unwrap();
    assert_eq!(shared.swarm_id.as_deref(), Some("team"));
    let updated = repo
        .update(&old.id, input(), shared.context_version)
        .await
        .unwrap();
    assert_eq!(updated.id, old.id);
    let still_swarm = swarm.get_project(&old.id).await.unwrap();
    assert_eq!(still_swarm.swarm_id, "team");
    assert_eq!(still_swarm.order_idx, 4);
    let standalone = repo
        .create(&"one".into(), &"user".into(), input())
        .await
        .unwrap();
    assert_eq!(standalone.swarm_id, None);
    assert_eq!(repo.list(&"one".into()).await.unwrap().len(), 2);
    assert_eq!(swarm.list_projects(&"team".into()).await.unwrap().len(), 1);
    assert!(matches!(
        repo.update(&old.id, input(), shared.context_version).await,
        Err(Error::Conflict(_))
    ));
}
#[tokio::test]
async fn project_context_is_workspace_checked_and_membership_is_provider_independent() {
    let pool = fixture().await;
    let projects = ProjectsRepo::new(pool.clone());
    let project = projects
        .create(&"one".into(), &"user".into(), input())
        .await
        .unwrap();
    let sessions = SessionsRepo::new(pool);
    let meta = json!({"project_id": project.id});
    let context = sessions
        .project_context(&"one".into(), &meta)
        .await
        .unwrap()
        .unwrap();
    for expected in [
        "Shared release",
        "Ship the release",
        "Run focused tests",
        "vault://guide",
        "Use UTC",
    ] {
        assert!(context.contains(expected));
    }
    assert!(matches!(
        sessions.project_context(&"two".into(), &meta).await,
        Err(Error::Forbidden(_))
    ));
    assert!(matches!(
        sessions
            .project_context(&"one".into(), &json!({"project_id": 5}))
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(sessions
        .project_context(&"one".into(), &json!({}))
        .await
        .unwrap()
        .is_none());
    for provider in ["claude", "codex", "shell"] {
        let session = sessions
            .create(NewSession {
                workspace_id: "one".into(),
                kind: SessionKind::Agent,
                provider: provider.into(),
                title: provider.into(),
                cwd: "/tmp".into(),
                provider_session_id: None,
                connection_id: None,
                created_by: "user".into(),
                meta: meta.clone(),
            })
            .await
            .unwrap();
        assert_eq!(session.meta["project_id"], project.id);
    }
    assert_eq!(
        projects
            .sessions(&project.id, &"one".into(), 100)
            .await
            .unwrap()
            .len(),
        3
    );
    let member = projects
        .sessions(&project.id, &"one".into(), 100)
        .await
        .unwrap()
        .remove(0);
    let foreign = projects
        .create(&"two".into(), &"user".into(), input())
        .await
        .unwrap();
    assert!(matches!(
        sessions
            .replace_meta_keys(&member.id, &json!({"project_id": foreign.id}))
            .await,
        Err(Error::Forbidden(_))
    ));
    sessions
        .replace_meta_keys(&member.id, &json!({"project_id": null}))
        .await
        .unwrap();
    assert_eq!(
        projects
            .sessions(&project.id, &"one".into(), 100)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn migration_preserves_legacy_project_and_backfills_swarm_membership() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .filter(|m| m.version < 134)
    {
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    sqlx::raw_sql("INSERT INTO users(id,username,password_hash,created_at) VALUES ('user','user','x','2026-01-01T00:00:00Z');
        INSERT INTO workspaces(id,name,root_path,created_at) VALUES ('one','one','/tmp','2026-01-01T00:00:00Z');
        INSERT INTO swarm_projects(id,swarm_id,workspace_id,name,goal_md,story_id,integration_branch,created_by,created_at,updated_at) VALUES ('project','team','one','Legacy','Preserved goal','story','integration/keep','user','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z');
        INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at,meta_json) VALUES ('session','one','agent','codex','Worker','idle','/tmp','user','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','{\"source\":\"swarm\",\"keep\":true}');
        INSERT INTO swarm_runs(id,swarm_id,workspace_id,project_id,agent_id,session_id,kind,trigger,status,enqueued_at) VALUES ('run','team','one','project','agent','session','task','manual','done','2026-01-01T00:00:00Z');")
        .execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/0134_common_projects.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let old = SwarmRepo::new(pool.clone())
        .get_project(&"project".into())
        .await
        .unwrap();
    assert_eq!(old.swarm_id, "team");
    assert_eq!(old.goal_md.as_deref(), Some("Preserved goal"));
    assert_eq!(old.story_id.as_deref(), Some("story"));
    assert_eq!(old.integration_branch.as_deref(), Some("integration/keep"));
    let session = SessionsRepo::new(pool.clone())
        .get(&"session".into())
        .await
        .unwrap();
    assert_eq!(session.meta["project_id"], "project");
    assert_eq!(session.meta["keep"], true);
    let projects = ProjectsRepo::new(pool.clone());
    let common = projects.get(&"project".into()).await.unwrap();
    assert_eq!(common.instructions_md, "");
    assert_eq!(common.context_version, 1);
    let free = projects
        .create(&"one".into(), &"user".into(), input())
        .await
        .unwrap();
    assert!(matches!(
        SwarmRepo::new(pool).get_project(&free.id).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn metadata_rejects_missing_projects_without_overwriting_existing_membership() {
    let pool = fixture().await;
    let projects = ProjectsRepo::new(pool.clone());
    let project = projects
        .create(&"one".into(), &"user".into(), input())
        .await
        .unwrap();
    let sessions = SessionsRepo::new(pool);
    let member = sessions
        .create(NewSession {
            workspace_id: "one".into(),
            kind: SessionKind::Connection,
            provider: "ssh".into(),
            title: "Supporting work".into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "user".into(),
            meta: json!({"project_id": project.id, "keep": "yes"}),
        })
        .await
        .unwrap();
    assert!(matches!(
        sessions
            .replace_meta_keys(&member.id, &json!({"project_id": "missing"}))
            .await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(
        sessions.get(&member.id).await.unwrap().meta["project_id"],
        project.id
    );
    assert_eq!(
        projects
            .sessions(&project.id, &"one".into(), 100)
            .await
            .unwrap()
            .len(),
        1
    );
    let updated = projects
        .update(
            &project.id,
            ProjectInput {
                instructions_md: "Changed instructions".into(),
                ..input()
            },
            project.context_version,
        )
        .await
        .unwrap();
    let fresh = sessions
        .project_context(&"one".into(), &member.meta)
        .await
        .unwrap()
        .unwrap();
    assert!(fresh.contains("Changed instructions"));
    assert!(fresh.contains(&format!("Context version: {}", updated.context_version)));
}

#[tokio::test]
async fn oversized_curated_context_is_rejected_before_it_becomes_launch_arguments() {
    let pool = fixture().await;
    let projects = ProjectsRepo::new(pool);
    let oversized = ProjectInput {
        goal_md: "g".repeat(30_000),
        instructions_md: "i".repeat(30_000),
        memory_md: "m".repeat(30_000),
        ..input()
    };
    assert!(matches!(
        projects
            .create(&"one".into(), &"user".into(), oversized)
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(projects.list(&"one".into()).await.unwrap().is_empty());
}

#[tokio::test]
async fn project_session_paging_filters_owner_before_limit_and_orders_ties() {
    let pool = fixture().await;
    sqlx::query("INSERT INTO users(id,username,password_hash,created_at) VALUES ('other','other','x','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
    let projects = ProjectsRepo::new(pool.clone());
    let project = projects
        .create(&"one".into(), &"user".into(), input())
        .await
        .unwrap();
    for (id, owner) in [("a", "user"), ("b", "user"), ("c", "other"), ("d", "other")] {
        sqlx::query("INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at,meta_json) VALUES (?,'one','agent','codex','Worker','idle','/tmp',?,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z',?)")
            .bind(id).bind(owner).bind(json!({"project_id":project.id}).to_string()).execute(&pool).await.unwrap();
    }
    let first = projects
        .session_page(&project.id, &"one".into(), Some("user"), None, 1)
        .await
        .unwrap();
    assert_eq!(first.items[0].id, "b");
    assert_eq!(first.next.as_ref().unwrap().0, "2026-01-01T00:00:00Z");
    let second = projects
        .session_page(
            &project.id,
            &"one".into(),
            Some("user"),
            Some(("2026-01-01T00:00:00Z", "b")),
            1,
        )
        .await
        .unwrap();
    assert_eq!(second.items[0].id, "a");
    let all = projects
        .session_page(&project.id, &"one".into(), None, None, 10)
        .await
        .unwrap();
    assert_eq!(all.items.len(), 4);
}
