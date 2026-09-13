//! Export credential tests use an isolated FileStore, never the user's Keychain.
use otto_connections::ConnectionsService;
use otto_core::{
    domain::{ConnectionKind, Environment},
    secrets::SecretStore,
};
use otto_state::{ConnectionSectionsRepo, ConnectionsRepo, NewConnection};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

struct Fixture {
    dir: PathBuf,
    pool: sqlx::SqlitePool,
    secrets: Arc<otto_keychain::FileStore>,
    service: ConnectionsService,
    id: String,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
async fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!("otto-export-test-{}", otto_core::new_id()));
    std::fs::create_dir(&dir).unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    for (id, root) in [("root", 1), ("member", 0)] {
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,disabled,created_at) VALUES(?,?, '',?, ?,0,?)").bind(id).bind(id).bind(id).bind(root).bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
    }
    let repo = ConnectionsRepo::new(pool.clone());
    let connection = repo
        .create(NewConnection {
            workspace_id: None,
            name: "Fixture".into(),
            kind: ConnectionKind::Mysql,
            params: json!({"host":"fixture.invalid","user":"alice"}),
            secret_ref: Some("test-password".into()),
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: "root".into(),
        })
        .await
        .unwrap();
    let secrets = Arc::new(otto_keychain::FileStore::new(&dir));
    secrets.put("test-password", "fixture-secret").unwrap();
    let service = ConnectionsService::new(
        repo,
        ConnectionSectionsRepo::new(pool.clone()),
        secrets.clone(),
    );
    Fixture {
        dir,
        pool,
        secrets,
        service,
        id: connection.id,
    }
}
#[tokio::test]
async fn default_export_does_not_read_secret_store_and_explicit_export_fails_closed() {
    let f = fixture().await;
    // An invalid file proves the default did not attempt a credential read.
    std::fs::write(f.dir.join("secrets.json"), "not valid secret JSON").unwrap();
    let profile = f
        .service
        .export_profile(&f.id, &"root".into(), false)
        .await
        .unwrap();
    assert!(profile.password.is_none());
    let error = f
        .service
        .export_profile(&f.id, &"root".into(), true)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("test-password"));
    assert!(!error.contains("not valid secret JSON"));
    std::fs::write(f.dir.join("secrets.json"), "{}").unwrap();
    assert!(f
        .service
        .export_profile(&f.id, &"root".into(), true)
        .await
        .is_err());
}
#[tokio::test]
async fn only_current_active_root_can_read_export_passwords() {
    let f = fixture().await;
    assert!(f
        .service
        .export_profile(&f.id, &"member".into(), true)
        .await
        .is_err());
    let profile = f
        .service
        .export_profile(&f.id, &"root".into(), true)
        .await
        .unwrap();
    assert_eq!(profile.password.as_deref(), Some("fixture-secret"));
    sqlx::query("UPDATE users SET disabled=1 WHERE id='root'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f
        .service
        .export_profile(&f.id, &"root".into(), true)
        .await
        .is_err());
    assert_eq!(
        f.secrets.get("test-password").unwrap().as_deref(),
        Some("fixture-secret")
    );
}
