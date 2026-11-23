//! Owner execution on the connection's own account skips native-grant
//! inspection. An alternate credential profile, and a non-owner whose rule
//! falls back to that same account, still hit `native_grants`. Redis has no
//! native ceiling, so those two paths fail closed without a network call.
//! Policy activation and approved-change preflight use the same owner exemption.
use super::*;
use otto_core::access::*;
use otto_core::domain::{Capability, ConnectionKind, Environment, Feature};
use otto_state::resource_access::ResourceAccessRepo;
use otto_state::{DbPool, GrantsRepo, UsersRepo};

struct NoSecrets;
impl SecretStore for NoSecrets {
    fn put(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn get(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn delete(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

fn allow(id: &str, subject: &Id, credential: Option<Id>) -> AccessRule {
    AccessRule {
        id: id.into(),
        subject_kind: SubjectKind::User,
        subject_id: subject.clone(),
        effect: RuleEffect::Allow,
        operations: vec!["discover".into(), "db_query".into()],
        children: None,
        grantable_operations: vec![],
        credential_connection_id: credential,
    }
}

struct Fixture {
    pool: DbPool,
    service: DbViewerService,
    root: Id,
    owner: Id,
    editor: Id,
    conn: Id,
    profile: Id,
}

impl Fixture {
    async fn new() -> Self {
        let pool = DbPool::from(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    sqlx::sqlite::SqliteConnectOptions::new()
                        .in_memory(true)
                        .foreign_keys(true),
                )
                .await
                .unwrap(),
        );
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let users = UsersRepo::new(pool.clone());
        let root = users.create("root", "", "Root", true).await.unwrap();
        let owner = users.create("owner", "", "Owner", false).await.unwrap();
        let editor = users.create("editor", "", "Editor", false).await.unwrap();
        let grants = GrantsRepo::new(pool.clone());
        for user in [&owner, &editor] {
            grants
                .set_grants(&user.id, &[(Feature::Database, Capability::View)])
                .await
                .unwrap();
        }
        let connections = otto_state::ConnectionsRepo::new(pool.clone());
        let params = |user: &str| serde_json::json!({"host": "127.0.0.1", "port": 1, "user": user});
        let conn = connections
            .create(otto_state::NewConnection {
                workspace_id: None,
                name: "logical".into(),
                kind: ConnectionKind::Redis,
                params: params("admin"),
                secret_ref: None,
                first_command: None,
                section_id: None,
                environment: Environment::Dev,
                read_only: false,
                created_by: owner.id.clone(),
            })
            .await
            .unwrap();
        let profile = connections
            .create(otto_state::NewConnection {
                workspace_id: None,
                name: "restricted".into(),
                kind: ConnectionKind::Redis,
                params: params("restricted"),
                secret_ref: None,
                first_command: None,
                section_id: None,
                environment: Environment::Dev,
                read_only: false,
                created_by: owner.id.clone(),
            })
            .await
            .unwrap();
        let service = DbViewerService::new(
            connections,
            Arc::new(NoSecrets),
            otto_state::DbExplorerRepo::new(pool.clone()),
        );
        let fixture = Self {
            pool,
            service,
            root: root.id,
            owner: owner.id,
            editor: editor.id,
            conn: conn.id,
            profile: profile.id,
        };
        fixture
            .set_rules(vec![
                allow("owner", &fixture.owner, None),
                allow("editor", &fixture.editor, None),
            ])
            .await;
        fixture
    }

    async fn set_rules(&self, rules: Vec<AccessRule>) {
        let repo = ResourceAccessRepo::new(self.pool.clone());
        let current = repo
            .get_policy(ResourceKind::Connection, &self.conn)
            .await
            .unwrap();
        repo.put_policy(
            &AccessPolicy {
                kind: ResourceKind::Connection,
                resource_id: self.conn.clone(),
                mode: AccessMode::Enforced,
                revision: current.revision,
                rules,
            },
            current.revision,
            &AccessActor {
                real_user_id: self.root.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
    }

    async fn resolved(&self, user: &Id) -> Resolved {
        let snap = self.service.access_snapshot(&self.conn).await.unwrap();
        self.service
            .resolve_snap(&snap, &self.conn, user, None, "db_query", SecretRead::Fresh)
            .await
            .unwrap()
    }

    async fn verify(&self, user: &Id, resolved: &Resolved) -> Result<()> {
        let snap = self.service.access_snapshot(&self.conn).await.unwrap();
        self.service
            .verify_native_snap(&snap, &self.conn, user, resolved)
            .await
    }
}

fn scope_refusal(err: Error) -> String {
    match err {
        Error::Forbidden(message) => message,
        other => panic!("expected native_scope_required, got {other}"),
    }
}

#[tokio::test]
async fn owner_same_connection_skips_native_grants_alternate_and_non_owner_do_not() {
    let fixture = Fixture::new().await;

    let owner = fixture.resolved(&fixture.owner).await;
    assert_eq!(owner.profile_id, fixture.conn);
    assert_eq!(
        owner.config.params.get("__read_only_execution"),
        Some(&Value::Bool(true))
    );
    fixture.verify(&fixture.owner, &owner).await.unwrap();
    assert_eq!(
        owner.config.params.get("__read_only_execution"),
        Some(&Value::Bool(true)),
        "the owner bypass must leave enforced reads read-only"
    );

    let editor = fixture.resolved(&fixture.editor).await;
    assert_eq!(
        editor.profile_id, fixture.conn,
        "a rule with no credential profile uses the shared account"
    );
    let editor_err = fixture.verify(&fixture.editor, &editor).await.unwrap_err();
    assert!(
        scope_refusal(editor_err).contains("native_scope_required"),
        "a non-owner on the shared account stays on the native ceiling"
    );

    let root = fixture.resolved(&fixture.root).await;
    fixture.verify(&fixture.root, &root).await.unwrap();

    fixture
        .set_rules(vec![
            allow("owner", &fixture.owner, Some(fixture.profile.clone())),
            allow("editor", &fixture.editor, None),
        ])
        .await;
    let alternate = fixture.resolved(&fixture.owner).await;
    assert_eq!(alternate.profile_id, fixture.profile);
    let alternate_err = fixture
        .verify(&fixture.owner, &alternate)
        .await
        .unwrap_err();
    assert!(
        scope_refusal(alternate_err).contains("native_scope_required"),
        "the owner's alternate profile still validates native grants"
    );
    fixture.verify(&fixture.root, &root).await.unwrap();
}

fn change_allow(id: &str, subject: &Id, credential: Option<Id>) -> AccessRule {
    let mut rule = allow(id, subject, credential);
    rule.operations
        .extend(["db_data".to_string(), "change_execute".to_string()]);
    rule
}

async fn put_rules(fixture: &Fixture, conn: &Id, rules: Vec<AccessRule>) {
    let repo = ResourceAccessRepo::new(fixture.pool.clone());
    let current = repo
        .get_policy(ResourceKind::Connection, conn)
        .await
        .unwrap();
    repo.put_policy(
        &AccessPolicy {
            kind: ResourceKind::Connection,
            resource_id: conn.clone(),
            mode: AccessMode::Enforced,
            revision: current.revision,
            rules,
        },
        current.revision,
        &AccessActor {
            real_user_id: fixture.root.clone(),
            effective_user_id: None,
        },
    )
    .await
    .unwrap();
}

async fn mysql_owned_by(fixture: &Fixture, name: &str, read_only: bool) -> Id {
    otto_state::ConnectionsRepo::new(fixture.pool.clone())
        .create(otto_state::NewConnection {
            workspace_id: None,
            name: name.into(),
            kind: ConnectionKind::Mysql,
            params: serde_json::json!({"host": "127.0.0.1", "port": 1, "user": "admin"}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only,
            created_by: fixture.owner.clone(),
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn owner_same_account_skips_activation_and_approved_preflight_native_grants() {
    let fixture = Fixture::new().await;
    let owner_only = vec![allow("owner", &fixture.owner, None)];
    fixture.set_rules(owner_only.clone()).await;
    let policy = ResourceAccessRepo::new(fixture.pool.clone())
        .get_policy(ResourceKind::Connection, &fixture.conn)
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .service
            .validate_access_policy(&policy, &fixture.owner)
            .await
            .unwrap_err(),
        Error::Forbidden(message) if message.contains("root must verify")
    ));
    fixture
        .service
        .validate_access_policy(&policy, &fixture.root)
        .await
        .unwrap();

    fixture
        .set_rules(vec![
            allow("owner", &fixture.owner, None),
            allow("editor", &fixture.editor, None),
        ])
        .await;
    let with_editor = ResourceAccessRepo::new(fixture.pool.clone())
        .get_policy(ResourceKind::Connection, &fixture.conn)
        .await
        .unwrap();
    let editor_err = fixture
        .service
        .validate_access_policy(&with_editor, &fixture.root)
        .await
        .unwrap_err();
    assert!(
        scope_refusal(editor_err).contains("native_scope_required"),
        "a non-owner on the shared account still blocks activation"
    );

    fixture
        .set_rules(vec![allow(
            "owner",
            &fixture.owner,
            Some(fixture.profile.clone()),
        )])
        .await;
    let alternate = ResourceAccessRepo::new(fixture.pool.clone())
        .get_policy(ResourceKind::Connection, &fixture.conn)
        .await
        .unwrap();
    let alternate_err = fixture
        .service
        .validate_access_policy(&alternate, &fixture.root)
        .await
        .unwrap_err();
    assert!(
        scope_refusal(alternate_err).contains("native_scope_required"),
        "the owner's alternate profile still blocks activation"
    );

    let mysql = mysql_owned_by(&fixture, "mysql-logical", false).await;
    put_rules(
        &fixture,
        &mysql,
        vec![
            change_allow("owner", &fixture.owner, None),
            change_allow("editor", &fixture.editor, None),
        ],
    )
    .await;
    let script = "UPDATE shop.orders SET total = 1";
    fixture
        .service
        .preflight_approved_change(&mysql, &fixture.owner, Some("shop"), script)
        .await
        .unwrap();
    let missing_target = fixture
        .service
        .preflight_approved_change(&mysql, &fixture.owner, None, script)
        .await
        .unwrap_err();
    assert!(
        scope_refusal(missing_target).contains("explicit database target"),
        "the owner still names one approved database"
    );

    let readonly = mysql_owned_by(&fixture, "mysql-readonly", true).await;
    put_rules(
        &fixture,
        &readonly,
        vec![change_allow("owner", &fixture.owner, None)],
    )
    .await;
    let readonly_err = fixture
        .service
        .preflight_approved_change(&readonly, &fixture.owner, Some("shop"), script)
        .await
        .unwrap_err();
    assert!(
        scope_refusal(readonly_err).contains("read-only"),
        "a read-only connection still refuses the owner"
    );

    let editor_err = fixture
        .service
        .preflight_approved_change(&mysql, &fixture.editor, Some("shop"), script)
        .await
        .unwrap_err();
    let editor_msg = editor_err.to_string();
    assert!(
        editor_msg.contains("native_scope_required")
            || editor_msg.contains("connect")
            || editor_msg.contains("Connection")
            || editor_msg.contains("refused"),
        "a non-owner still reaches native grant verification: {editor_msg}"
    );
}

#[tokio::test]
async fn local_infile_grant_tracks_host_permission_for_incremental_scripts() {
    let fixture = Fixture::new().await;
    let mut resolved = fixture.resolved(&fixture.root).await;
    resolved.config.engine = Engine::Mysql;
    let statement = "LOAD DATA LOCAL INFILE '/tmp/otto-policy-fixture.csv' INTO TABLE t";
    fixture
        .service
        .prepare_local_infile(
            &fixture.conn,
            &fixture.root,
            statement,
            &mut resolved.config,
        )
        .await
        .unwrap();
    assert_eq!(
        resolved.config.params["__mysql_local_infile_authorized"],
        true
    );
    // No local-file capability or extra authority read for an ordinary query.
    fixture
        .service
        .prepare_local_infile(
            &fixture.conn,
            &fixture.root,
            "SELECT 1",
            &mut resolved.config,
        )
        .await
        .unwrap();
    assert!(resolved
        .config
        .params
        .get("__mysql_local_infile_authorized")
        .is_none());
    // Byte detection remains independent of later SQL-mode changes.
    fixture.service.prepare_local_infile(
        &fixture.conn, &fixture.root,
        r"SET sql_mode='NO_BACKSLASH_ESCAPES'; SELECT 'ends\'; LOAD DATA LOCAL INFILE '/tmp/f' INTO TABLE t",
        &mut resolved.config,
    ).await.unwrap();
    assert_eq!(
        resolved.config.params["__mysql_local_infile_authorized"],
        true
    );
    // Even an injected flag cannot give a connection owner host-file access.
    resolved.config.params["__mysql_local_infile_authorized"] = Value::Bool(true);
    assert!(matches!(
        fixture
            .service
            .prepare_local_infile(
                &fixture.conn,
                &fixture.owner,
                statement,
                &mut resolved.config,
            )
            .await,
        Err(Error::Forbidden(_))
    ));
    assert!(resolved
        .config
        .params
        .get("__mysql_local_infile_authorized")
        .is_none());
}

/// Root and the owner on the connection's own account fall back to the lexer
/// for SQL forms restricted SQL can't prove (process lists, `KILL`); an editor,
/// and the owner through an alternate credential profile, are still refused.
#[tokio::test]
async fn unrestricted_callers_run_unparsed_sql_forms_editors_do_not() {
    let fixture = Fixture::new().await;
    let conn = mysql_owned_by(&fixture, "mysql", false).await;
    put_rules(
        &fixture,
        &conn,
        vec![
            allow("owner", &fixture.owner, None),
            allow("editor", &fixture.editor, None),
        ],
    )
    .await;
    let snap = fixture.service.access_snapshot(&conn).await.unwrap();
    let run = |user: Id, sql: &'static str| {
        let (service, snap) = (&fixture.service, &snap);
        async move {
            let req = QueryRequest {
                statement: sql.into(),
                ..QueryRequest::default()
            };
            service.execution_access_snap(snap, &user, &req).await
        }
    };
    run(fixture.owner.clone(), "SHOW FULL PROCESSLIST")
        .await
        .unwrap();
    for sql in ["SHOW FULL PROCESSLIST", "KILL 42"] {
        run(fixture.root.clone(), sql).await.unwrap();
        assert!(matches!(
            run(fixture.editor.clone(), sql).await.unwrap_err(),
            Error::Forbidden(_)
        ));
    }
    put_rules(
        &fixture,
        &conn,
        vec![allow(
            "owner",
            &fixture.owner,
            Some(fixture.profile.clone()),
        )],
    )
    .await;
    let snap = fixture.service.access_snapshot(&conn).await.unwrap();
    let req = QueryRequest {
        statement: "SHOW FULL PROCESSLIST".into(),
        ..QueryRequest::default()
    };
    assert!(matches!(
        fixture
            .service
            .execution_access_snap(&snap, &fixture.owner, &req)
            .await
            .unwrap_err(),
        Error::Forbidden(_)
    ));
}

#[tokio::test]
async fn mongosh_access_grant_is_root_only_and_strips_forged_flags() {
    use otto_core::auth::{with_request_credential, RequestCredential};

    let fixture = Fixture::new().await;
    let mut resolved = fixture.resolved(&fixture.root).await;
    resolved.config.engine = Engine::Mongodb;
    let script = "const x = 1; print(x);";
    let human_root = RequestCredential {
        agent: false,
        root_withheld: false,
    };

    // A signed-in human root user is permitted to invoke mongosh scripts.
    with_request_credential(
        human_root,
        fixture
            .service
            .prepare_mongosh_access(&fixture.root, script, &mut resolved.config),
    )
    .await
    .unwrap();
    assert_eq!(resolved.config.params["__mongosh_authorized"], true);

    // Ordinary native MongoDB reads do not get the mongosh flag.
    with_request_credential(
        human_root,
        fixture.service.prepare_mongosh_access(
            &fixture.root,
            "db.users.find({})",
            &mut resolved.config,
        ),
    )
    .await
    .unwrap();
    assert!(resolved.config.params.get("__mongosh_authorized").is_none());

    // Even a pre-injected flag is stripped and non-root human callers are refused.
    resolved.config.params["__mongosh_authorized"] = Value::Bool(true);
    assert!(matches!(
        with_request_credential(
            human_root,
            fixture
                .service
                .prepare_mongosh_access(&fixture.editor, script, &mut resolved.config),
        )
        .await,
        Err(Error::Forbidden(_))
    ));
    assert!(resolved.config.params.get("__mongosh_authorized").is_none());

    // An agent session owned by root (with root withheld) is refused.
    resolved.config.params["__mongosh_authorized"] = Value::Bool(true);
    let agent_withheld = RequestCredential {
        agent: true,
        root_withheld: true,
    };
    assert!(matches!(
        with_request_credential(
            agent_withheld,
            fixture
                .service
                .prepare_mongosh_access(&fixture.root, script, &mut resolved.config),
        )
        .await,
        Err(Error::Forbidden(_))
    ));
    assert!(resolved.config.params.get("__mongosh_authorized").is_none());

    // Even an agent credential with root_withheld=false (or outside a request) is refused.
    let agent_unwithheld = RequestCredential {
        agent: true,
        root_withheld: false,
    };
    assert!(matches!(
        with_request_credential(
            agent_unwithheld,
            fixture
                .service
                .prepare_mongosh_access(&fixture.root, script, &mut resolved.config),
        )
        .await,
        Err(Error::Forbidden(_))
    ));
    assert!(matches!(
        fixture
            .service
            .prepare_mongosh_access(&fixture.root, script, &mut resolved.config)
            .await,
        Err(Error::Forbidden(_))
    ));

    // Carrying the request credential across tokio::spawn preserves both allow and deny.
    let svc = fixture.service.clone();
    let root_id = fixture.root.clone();
    let mut cfg_ok = resolved.config.clone();
    let ok = with_request_credential(human_root, async move {
        tokio::spawn(otto_core::auth::carry_request_credential(async move {
            svc.prepare_mongosh_access(&root_id, script, &mut cfg_ok)
                .await
                .map(|()| cfg_ok.params["__mongosh_authorized"].clone())
        }))
        .await
        .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(ok, Value::Bool(true));

    let svc = fixture.service.clone();
    let root_id = fixture.root.clone();
    let mut cfg_deny = resolved.config.clone();
    let denied = with_request_credential(agent_withheld, async move {
        tokio::spawn(otto_core::auth::carry_request_credential(async move {
            svc.prepare_mongosh_access(&root_id, script, &mut cfg_deny)
                .await
        }))
        .await
        .unwrap()
    })
    .await;
    assert!(matches!(denied, Err(Error::Forbidden(_))));
}

#[tokio::test]
async fn enforced_prod_connection_allows_catalog_reads_but_blocks_writes() {
    let fixture = Fixture::new().await;
    let conn = mysql_owned_by(&fixture, "mysql-prod", false).await;
    sqlx::query("UPDATE connections SET environment = 'prod' WHERE id = ?")
        .bind(&conn)
        .execute(&fixture.pool)
        .await
        .unwrap();
    put_rules(&fixture, &conn, vec![allow("root", &fixture.root, None)]).await;
    let snap = fixture.service.access_snapshot(&conn).await.unwrap();

    for sql in [
        "SHOW TABLES",
        "DESCRIBE orders",
        "SELECT COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_NAME = 'orders'",
    ] {
        let req = QueryRequest {
            statement: sql.into(),
            ..QueryRequest::default()
        };
        fixture
            .service
            .execution_access_snap(&snap, &fixture.root, &req)
            .await
            .unwrap_or_else(|e| panic!("catalog read '{sql}' should pass on prod: {e}"));
    }

    let write = QueryRequest {
        statement: "DELETE FROM orders WHERE id = 1".into(),
        ..QueryRequest::default()
    };
    let err = fixture
        .service
        .execution_access_snap(&snap, &fixture.root, &write)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("review_required"), "unexpected error: {err}");
}
