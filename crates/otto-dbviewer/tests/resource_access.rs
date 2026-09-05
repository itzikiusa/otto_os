//! Resource/credential boundaries. Native cases use only explicitly provided
//! disposable loopback fixture ports; no developer or production profile is read.
use otto_core::access::*;
use otto_core::domain::{Capability, ConnectionKind, Environment, Feature, User};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_dbviewer::{DbViewerService, QueryRequest};
use otto_state::resource_access::ResourceAccessRepo;
use otto_state::{
    ConnectionsRepo, DbExplorerRepo, GrantsRepo, NewConnection, SqlitePool, UsersRepo,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::sync::Arc;

struct FixtureSecrets;
impl SecretStore for FixtureSecrets {
    fn put(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn get(&self, _: &str) -> Result<Option<String>> {
        Ok(Some("otto_fixture_only".into()))
    }
    fn delete(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

struct Fixture {
    pool: SqlitePool,
    service: DbViewerService,
    root: User,
    reader: User,
    conn: Id,
    profile: Id,
    group: Id,
}
impl Fixture {
    async fn new(kind: ConnectionKind, port: u16) -> Self {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let users = UsersRepo::new(pool.clone());
        let root = users.create("root", "fixture", "Root", true).await.unwrap();
        let reader = users
            .create("reader", "fixture", "Reader", false)
            .await
            .unwrap();
        GrantsRepo::new(pool.clone())
            .set_grants(&reader.id, &[(Feature::Database, Capability::View)])
            .await
            .unwrap();
        let db = if kind == ConnectionKind::Postgres {
            "resourcefixture"
        } else {
            "shop"
        };
        let native_root = if kind == ConnectionKind::Postgres {
            "postgres"
        } else {
            "root"
        };
        let conns = ConnectionsRepo::new(pool.clone());
        let mut ids = Vec::new();
        for native_user in [native_root, "otto_reader"] {
            ids.push(conns.create(NewConnection {
                workspace_id: None, name: native_user.into(), kind,
                params: serde_json::json!({"host":"127.0.0.1", "port":port, "user":native_user, "db":db}),
                secret_ref: Some("fixture-secret".into()), first_command: None, section_id: None,
                environment: Environment::Dev, read_only: false, created_by: root.id.clone(),
            }).await.unwrap().id);
        }
        let conn = ids.remove(0);
        let profile = ids.remove(0);
        let repo = ResourceAccessRepo::new(pool.clone());
        let actor = AccessActor {
            real_user_id: root.id.clone(),
            effective_user_id: None,
        };
        let group = repo.create_group("readers", None, &actor).await.unwrap();
        repo.add_group_member(&group.id, &reader.id, &actor)
            .await
            .unwrap();
        let current = repo
            .get_policy(ResourceKind::Connection, &conn)
            .await
            .unwrap();
        repo.put_policy(
            &AccessPolicy {
                kind: ResourceKind::Connection,
                resource_id: conn.clone(),
                mode: AccessMode::Enforced,
                revision: current.revision,
                rules: vec![AccessRule {
                    id: "reader-grant".into(),
                    subject_kind: SubjectKind::Group,
                    subject_id: group.id.clone(),
                    effect: RuleEffect::Allow,
                    operations: ["discover", "db_browse", "db_query", "db_export"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                    children: Some(vec!["shop".into()]),
                    grantable_operations: vec![],
                    credential_connection_id: Some(profile.clone()),
                }],
            },
            current.revision,
            &actor,
        )
        .await
        .unwrap();
        let service = DbViewerService::new(
            conns,
            Arc::new(FixtureSecrets),
            DbExplorerRepo::new(pool.clone()),
        );
        Self {
            pool,
            service,
            root,
            reader,
            conn,
            profile,
            group: group.id,
        }
    }
    fn req(&self, sql: &str) -> QueryRequest {
        QueryRequest {
            statement: sql.into(),
            node: Some("shop".into()),
            confirm_write: true,
            ..Default::default()
        }
    }
    async fn revoke(&self) {
        ResourceAccessRepo::new(self.pool.clone())
            .remove_group_member(
                &self.group,
                &self.reader.id,
                &AccessActor {
                    real_user_id: self.root.id.clone(),
                    effective_user_id: None,
                },
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn grants_block_writes_exports_and_hidden_ids_before_network_and_revocation_is_live() {
    let f = Fixture::new(ConnectionKind::Mysql, 1).await;
    f.service
        .authorize(&f.conn, &f.reader.id, Some("shop"), "db_query")
        .await
        .unwrap();
    for sql in [
        "UPDATE shop.orders SET total=0",
        "SELECT 1; DROP TABLE shop.orders",
        "WITH gone AS (DELETE FROM shop.orders RETURNING *) SELECT * FROM gone",
        "SELECT lo_create(0)",
    ] {
        let error = f
            .service
            .run(&f.conn, &f.reader.id, &f.req(sql))
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Forbidden(_)), "{sql}: {error:?}");
    }
    assert!(f
        .service
        .schema_children(&f.conn, &f.reader.id, "db:hidden", None)
        .await
        .is_err());
    // Source credential profiles are not visible resources through the logical grant.
    assert!(matches!(
        f.service
            .authorize(&f.profile, &f.reader.id, None, "discover")
            .await
            .unwrap_err(),
        Error::NotFound(_)
    ));
    let repo = ResourceAccessRepo::new(f.pool.clone());
    let mut policy = repo
        .get_policy(ResourceKind::Connection, &f.conn)
        .await
        .unwrap();
    let revision = policy.revision;
    policy.rules.push(AccessRule {
        id: "deny-export".into(),
        subject_kind: SubjectKind::User,
        subject_id: f.reader.id.clone(),
        effect: RuleEffect::Deny,
        operations: vec!["db_export".into()],
        children: None,
        grantable_operations: vec![],
        credential_connection_id: None,
    });
    repo.put_policy(
        &policy,
        revision,
        &AccessActor {
            real_user_id: f.root.id.clone(),
            effective_user_id: None,
        },
    )
    .await
    .unwrap();
    assert!(f
        .service
        .guard_export(&f.conn, &f.reader.id, "SELECT 1", Some("db:shop"))
        .await
        .is_err());
    f.revoke().await;
    assert!(matches!(
        f.service
            .authorize(&f.conn, &f.reader.id, Some("shop"), "db_query")
            .await
            .unwrap_err(),
        Error::NotFound(_)
    ));
}

async fn native_fixture(kind: ConnectionKind, env: &str) {
    let port: u16 = std::env::var(env)
        .expect("explicit disposable fixture port required")
        .parse()
        .unwrap();
    let f = Fixture::new(kind, port).await;
    let candidate = ResourceAccessRepo::new(f.pool.clone())
        .get_policy(ResourceKind::Connection, &f.conn)
        .await
        .unwrap();
    f.service
        .validate_access_policy(&candidate, &f.root.id)
        .await
        .unwrap();
    let mut broad = candidate.clone();
    broad.rules[0].credential_connection_id = None;
    assert!(
        f.service
            .validate_access_policy(&broad, &f.root.id)
            .await
            .is_err(),
        "overprivileged primary credential must fail activation readiness"
    );
    let result = f
        .service
        .run(
            &f.conn,
            &f.reader.id,
            &f.req("SELECT total FROM shop.orders WHERE id=1"),
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0][0], serde_json::json!(42));
    let nodes = f.service.schema_root(&f.conn, &f.reader.id).await.unwrap();
    assert_eq!(
        nodes.iter().map(|n| n.label.as_str()).collect::<Vec<_>>(),
        vec!["shop"]
    );
    assert!(f
        .service
        .run(
            &f.conn,
            &f.reader.id,
            &f.req("SELECT * FROM hidden.secrets")
        )
        .await
        .is_err());
    // Native direct driver query also fails: Otto's parser is not the boundary.
    let config = otto_dbviewer::ResolvedConfig {
        engine: if kind == ConnectionKind::Mysql {
            otto_dbviewer::Engine::Mysql
        } else {
            otto_dbviewer::Engine::Postgres
        },
        host: "127.0.0.1".into(),
        port,
        user: Some("otto_reader".into()),
        password: Some("otto_fixture_only".into()),
        database: Some(
            if kind == ConnectionKind::Mysql {
                "shop"
            } else {
                "resourcefixture"
            }
            .into(),
        ),
        tls: Default::default(),
        params: serde_json::json!({}),
    };
    let driver = otto_dbviewer::Registry::new().get(config.engine);
    assert!(driver
        .run(&config, &f.req("UPDATE shop.orders SET total=0"))
        .await
        .is_err());
    assert!(driver
        .run(&config, &f.req("SELECT * FROM hidden.secrets"))
        .await
        .is_err());
    // Warm pooled credentials cannot continue after membership revocation.
    f.revoke().await;
    assert!(f
        .service
        .run(
            &f.conn,
            &f.reader.id,
            &f.req("SELECT total FROM shop.orders")
        )
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires explicitly provisioned disposable MySQL fixture"]
async fn mysql_native_credentials_isolate_database_and_writes() {
    native_fixture(ConnectionKind::Mysql, "OTTO_RESOURCE_MYSQL_PORT").await;
}
#[tokio::test]
#[ignore = "requires explicitly provisioned disposable PostgreSQL fixture"]
async fn postgres_native_credentials_isolate_schema_and_writes() {
    native_fixture(ConnectionKind::Postgres, "OTTO_RESOURCE_PG_PORT").await;
}

async fn approved_native_fixture(kind: ConnectionKind, env: &str, partial:bool) {
    use otto_state::database_changes::{
        ChangeInput, ChangeTarget, DatabaseChangesRepo, TargetSnapshot,
    };
    let port = std::env::var(env)
        .expect("explicit fixture port")
        .parse()
        .unwrap();
    let f = Fixture::new(kind, port).await;
    let repo = DatabaseChangesRepo::new(f.pool.clone());
    let table = format!("change_{}", otto_core::new_id().replace('-', ""));
    let create = format!("CREATE TABLE shop.{table} (id INT)");
    let script=if partial{format!("{create}; {create}")}else{create};
    let target = ChangeTarget {
        connection_id: f.conn.clone(),
        node: "shop".into(),
    };
    f.service
        .preflight_approved_change(&f.conn, &f.root.id, Some("shop"), &script)
        .await
        .unwrap();
    let fingerprint = f
        .service
        .change_target_fingerprint(&f.conn, &f.root.id, Some("shop"))
        .await
        .unwrap();
    let policy = ResourceAccessRepo::new(f.pool.clone())
        .get_policy(ResourceKind::Connection, &f.conn)
        .await
        .unwrap();
    let snapshots = vec![TargetSnapshot {
        target: target.clone(),
        environment: "dev".into(),
        policy_revision: policy.revision,
        connection_fingerprint: fingerprint,
    }];
    // Different real author and approver through the production persistence API.
    let change = repo
        .create(
            &ChangeInput {
                title: "Fixture schema change".into(),
                description: String::new(),
                script: script.clone(),
                targets: vec![target],
            },
            &f.reader.id,
            &f.reader.id,
        )
        .await
        .unwrap();
    let change = repo
        .validate(&change, &f.root.id, &snapshots, &f.reader.id, &f.reader.id)
        .await
        .unwrap();
    let change = repo
        .transition(&change, "awaiting_review", &f.reader.id, &f.reader.id, "")
        .await
        .unwrap();
    let change = repo
        .transition(
            &change,
            "approved",
            &f.root.id,
            &f.root.id,
            "fixture reviewed",
        )
        .await
        .unwrap();
    let attempts = repo
        .claim(&change, &snapshots, &f.root.id, &f.root.id)
        .await
        .unwrap();
    repo.start_attempt(&attempts[0].id).await.unwrap();
    // A stale/tampered artifact cannot execute even though it is marked running.
    sqlx::query("UPDATE database_change_attempts SET script='DROP TABLE shop.orders' WHERE id=?")
        .bind(&attempts[0].id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f
        .service
        .execute_approved_change(&change.id, &attempts[0].id, &f.root.id)
        .await
        .is_err());
    sqlx::query("UPDATE database_change_attempts SET script=? WHERE id=?")
        .bind(&script)
        .bind(&attempts[0].id)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE database_change_attempts SET content_hash='stale' WHERE id=?")
        .bind(&attempts[0].id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f
        .service
        .execute_approved_change(&change.id, &attempts[0].id, &f.root.id)
        .await
        .is_err());
    sqlx::query("UPDATE database_change_attempts SET content_hash=? WHERE id=?")
        .bind(&change.content_hash)
        .bind(&attempts[0].id)
        .execute(&f.pool)
        .await
        .unwrap();
    let result=f.service
        .execute_approved_change(&change.id, &attempts[0].id, &f.root.id)
        .await
        .unwrap();
    if partial {
        assert!(!result.errored,"first DDL succeeded");
        assert_eq!(result.more_results.len(),1);
        assert!(result.more_results[0].errored,"second duplicate DDL must surface an error inside Ok(batch)");
        repo.finish_attempt_progress(&attempts[0].id,false,Some(1)).await.unwrap();
        assert_eq!(repo.finish(&change.id,&f.root.id,&f.root.id).await.unwrap().status,"outcome_unknown");
        let locked:i64=sqlx::query_scalar("SELECT count(*) FROM database_change_attempts WHERE connection_id=? AND state='outcome_unknown'").bind(&f.conn).fetch_one(&f.pool).await.unwrap();assert_eq!(locked,1);
        let native=f.service.run(&f.conn,&f.root.id,&f.req(&format!("SELECT count(*) FROM shop.{table}"))).await.unwrap();assert_eq!(native.rows[0][0],serde_json::json!(0),"first CREATE actually committed");
        assert!(f.service.execute_approved_change(&change.id,&attempts[0].id,&f.root.id).await.is_err(),"unknown attempts never replay");
        return;
    }
    repo.finish_attempt(&attempts[0].id, true).await.unwrap();
    assert_eq!(
        repo.finish(&change.id, &f.root.id, &f.root.id)
            .await
            .unwrap()
            .status,
        "succeeded"
    );
    assert!(
        f.service
            .execute_approved_change(&change.id, &attempts[0].id, &f.root.id)
            .await
            .is_err(),
        "finished attempts cannot replay"
    );
    let result = f
        .service
        .run(
            &f.conn,
            &f.root.id,
            &f.req(&format!("SELECT count(*) FROM shop.{table}")),
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0][0], serde_json::json!(0));
}

#[tokio::test]
#[ignore = "requires explicitly provisioned disposable MySQL fixture"]
async fn mysql_reviewed_artifact_executes_once_and_rejects_mismatch() {
    approved_native_fixture(ConnectionKind::Mysql, "OTTO_RESOURCE_MYSQL_PORT",false).await;
}
#[tokio::test]
#[ignore = "requires explicitly provisioned disposable PostgreSQL fixture"]
async fn postgres_reviewed_artifact_executes_once_and_rejects_mismatch() {
    approved_native_fixture(ConnectionKind::Postgres, "OTTO_RESOURCE_PG_PORT",false).await;
}

#[tokio::test]
#[ignore = "requires explicitly provisioned disposable PostgreSQL fixture"]
async fn governed_root_reads_cannot_call_a_mutating_builtin_overload() {
    let port:u16=std::env::var("OTTO_RESOURCE_PG_PORT").unwrap().parse().unwrap();
    let f=Fixture::new(ConnectionKind::Postgres,port).await;
    let native=sqlx::PgPool::connect(&format!("postgres://postgres:otto_fixture_only@127.0.0.1:{port}/resourcefixture")).await.unwrap();
    // Fixture-only schema keeps this malicious overload away from other tests.
    let schema=format!("readonly_{}",otto_core::new_id().to_ascii_lowercase());
    sqlx::raw_sql(&format!("CREATE SCHEMA {schema}; CREATE TABLE {schema}.effects (n integer); CREATE FUNCTION {schema}.lower(integer) RETURNS integer LANGUAGE plpgsql AS $$ BEGIN INSERT INTO {schema}.effects VALUES ($1); RETURN $1; END $$; REVOKE ALL ON FUNCTION {schema}.lower(integer) FROM PUBLIC;")).execute(&native).await.unwrap();
    sqlx::query("UPDATE connections SET environment='prod',params_json=json_set(params_json,'$.__read_only_execution',0) WHERE id=?").bind(&f.conn).execute(&f.pool).await.unwrap();
    for statement in ["SELECT lower(1)","SELECT 1; SELECT lower(1)"] {
        let req=QueryRequest{statement:statement.into(),node:Some(schema.clone()),confirm_write:true,..Default::default()};
        let result=f.service.run(&f.conn,&f.root.id,&req).await;
        let error=result.expect_err("native read-only must reject hidden writes for root").to_string();
        assert!(error.contains("read-only transaction"),"must reach the native readonly gate, got: {error}");
    }
    // Streaming exports use the same native protection, including early errors.
    let export_error=f.service.export_to_writer(&f.conn,&f.root.id,"SELECT lower(1)",Some(&schema),otto_dbviewer::export::ExportFormat::Csv,None,Box::new(Vec::<u8>::new())).await.expect_err("export must refuse hidden writes").to_string();
    assert!(export_error.contains("read-only transaction"),"export must reach native readonly gate, got: {export_error}");
    let count:i64=sqlx::query_scalar(&format!("SELECT count(*) FROM {schema}.effects")).fetch_one(&native).await.unwrap();assert_eq!(count,0);
    // Failed read-only execution returns a clean connection for another read.
    let result=f.service.run(&f.conn,&f.root.id,&QueryRequest{statement:"SELECT 1".into(),node:Some(schema.clone()),..Default::default()}).await.unwrap();assert_eq!(result.rows[0][0],serde_json::json!(1));
    sqlx::raw_sql(&format!("DROP SCHEMA {schema} CASCADE")).execute(&native).await.unwrap();
    native.close().await;
}

#[tokio::test]
#[ignore = "requires explicitly provisioned disposable MySQL fixture"]
async fn mysql_partial_reviewed_batch_retains_unknown_lock() {approved_native_fixture(ConnectionKind::Mysql,"OTTO_RESOURCE_MYSQL_PORT",true).await;}
#[tokio::test]
#[ignore = "requires explicitly provisioned disposable PostgreSQL fixture"]
async fn postgres_partial_reviewed_batch_retains_unknown_lock() {approved_native_fixture(ConnectionKind::Postgres,"OTTO_RESOURCE_PG_PORT",true).await;}

#[tokio::test]
async fn governed_readonly_profile_rejects_confirmed_root_write_before_network() {
    let f=Fixture::new(ConnectionKind::Mysql,9).await;
    sqlx::query("UPDATE connections SET read_only=1 WHERE id=?").bind(&f.conn).execute(&f.pool).await.unwrap();
    let result=f.service.run(&f.conn,&f.root.id,&QueryRequest{statement:"UPDATE shop.orders SET total=0".into(),node:Some("shop".into()),confirm_write:true,..Default::default()}).await;
    assert!(matches!(result,Err(Error::Forbidden(ref reason)) if reason.contains("read-only connection")));
}
