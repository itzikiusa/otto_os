//! Mongo schema metadata must survive enforced browsing without exposing values.
use super::*;
use otto_core::access::*;
use otto_core::domain::{Capability, ConnectionKind, Environment, Feature};
use std::sync::atomic::{AtomicUsize, Ordering};

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

struct MongoMetadata {
    reads: AtomicUsize,
}

#[async_trait::async_trait]
impl Driver for MongoMetadata {
    fn engine(&self) -> Engine {
        Engine::Mongodb
    }
    fn capabilities(&self) -> Capabilities {
        crate::drivers::mongodb::MongoDriver::default().capabilities()
    }
    async fn test(&self, _: &ResolvedConfig) -> Result<TestResult> {
        unreachable!()
    }
    async fn schema_root(&self, _: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
        unreachable!()
    }
    async fn schema_graph_bulk(
        &self,
        _: &ResolvedConfig,
        schema: &str,
        _: usize,
    ) -> Result<Option<SchemaGraph>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        assert_eq!(schema, "shop");
        // Production Mongo's optimized graph samples TOP-LEVEL names/types only.
        // A completion repair in the generic walk cannot fix this normal path.
        Ok(Some(SchemaGraph {
            schema: schema.into(),
            tables: ["profiles", "other"]
                .into_iter()
                .map(|name| crate::types::GraphTable {
                    id: format!("db:shop/coll:{name}"),
                    schema: schema.into(),
                    name: name.into(),
                    kind: NodeKind::Collection,
                    columns: vec![crate::types::GraphColumn {
                        name: "address".into(),
                        data_type: "object".into(),
                        nullable: true,
                        primary_key: false,
                        foreign_key: false,
                    }],
                })
                .collect(),
            edges: vec![],
            relationships: false,
            truncated: false,
        }))
    }
    async fn schema_children(
        &self,
        _: &ResolvedConfig,
        path: &NodePath,
        _: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        assert_eq!(path.get("db"), Some("shop"));
        Ok(vec![SchemaNode::new(
            "db:shop/coll:profiles",
            "profiles",
            NodeKind::Collection,
        )])
    }
    async fn object_detail(&self, _: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        assert_eq!(path.get("db"), Some("shop"));
        assert_eq!(path.get("coll"), Some("profiles"));
        let mut detail = ObjectDetail::new("profiles", NodeKind::Collection);
        detail.indexes = vec![crate::types::IndexDef {
            name: "address.city_1".into(),
            columns: vec!["address.city".into()],
            unique: false,
            method: None,
            definition: None,
        }];
        detail.extra = serde_json::json!({
            "sampled_fields": {"_id": "objectId", "address": "object"},
            "sampled_paths": ["_id", "address", "address.city"],
            "sampled_path_types": {"_id": "objectId", "address": "object", "address.city": "string"},
            "sample": {"address": {"city": "PRIVATE_SAMPLE_VALUE"}},
            "validator": {"private": "PRIVATE_VALIDATOR_VALUE"},
            "unrecognized": "PRIVATE_EXTRA_VALUE"
        });
        Ok(detail)
    }
    async fn run(&self, _: &ResolvedConfig, _: &QueryRequest) -> Result<QueryResult> {
        unreachable!()
    }
    async fn completion(
        &self,
        _: &ResolvedConfig,
        _: &CompletionContext,
    ) -> Result<CompletionResponse> {
        unreachable!("enforced completion must use the authorized graph")
    }
    fn assemble_completion(
        &self,
        snap: &crate::complete::SchemaSnapshot,
        ctx: &CompletionContext,
    ) -> Vec<crate::types::CompletionItem> {
        crate::drivers::mongodb::MongoDriver::default().assemble_completion(snap, ctx)
    }
}

async fn fixture() -> (DbViewerService, Id, Id, Arc<MongoMetadata>) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let users = otto_state::UsersRepo::new(pool.clone());
    let root = users.create("root", "", "Root", true).await.unwrap();
    let reader = users.create("reader", "", "Reader", false).await.unwrap();
    otto_state::GrantsRepo::new(pool.clone())
        .set_grants(&reader.id, &[(Feature::Database, Capability::View)])
        .await
        .unwrap();
    let connections = ConnectionsRepo::new(pool.clone());
    let conn = connections
        .create(otto_state::NewConnection {
            workspace_id: None,
            name: "scoped mongo".into(),
            kind: ConnectionKind::Mongodb,
            params: serde_json::json!({"host":"127.0.0.1","port":1,"database":"shop"}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let policies = otto_state::resource_access::ResourceAccessRepo::new(pool.clone());
    let mut policy = policies
        .get_policy(ResourceKind::Connection, &conn.id)
        .await
        .unwrap();
    policy.rules = vec![AccessRule {
        id: "shop-only".into(),
        subject_kind: SubjectKind::User,
        subject_id: reader.id.clone(),
        effect: RuleEffect::Allow,
        operations: vec!["discover".into(), "db_browse".into()],
        children: Some(vec!["shop".into()]),
        grantable_operations: vec![],
        credential_connection_id: None,
    }];
    policies
        .put_policy(
            &policy,
            policy.revision,
            &AccessActor {
                real_user_id: root.id,
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
    let mut service =
        DbViewerService::new(connections, Arc::new(NoSecrets), DbExplorerRepo::new(pool));
    let driver = Arc::new(MongoMetadata {
        reads: AtomicUsize::new(0),
    });
    service
        .registry
        .set_for_test(Engine::Mongodb, driver.clone());
    (service, conn.id, reader.id, driver)
}

#[tokio::test]
async fn enforced_mongo_detail_keeps_schema_metadata_but_not_sampled_values() {
    let (service, conn, reader, driver) = fixture().await;
    let detail = service
        .object_detail(&conn, &reader, "db:shop/coll:profiles", false)
        .await
        .unwrap();
    assert_eq!(
        detail.extra,
        serde_json::json!({
            "sampled_fields": {"_id": "objectId", "address": "object"},
            "sampled_paths": ["_id", "address", "address.city"],
            "sampled_path_types": {"_id": "objectId", "address": "object", "address.city": "string"}
        })
    );
    let reads = driver.reads.load(Ordering::SeqCst);
    assert!(service
        .object_detail(&conn, &reader, "db:hidden/coll:profiles", false)
        .await
        .is_err());
    assert_eq!(
        driver.reads.load(Ordering::SeqCst),
        reads,
        "denied schema must not reach driver"
    );
}

#[tokio::test]
async fn enforced_mongo_completion_contains_nested_fields_only_from_authorized_schema() {
    let (service, conn, reader, driver) = fixture().await;
    let ctx = CompletionContext {
        prefix: "db.profiles.find({ address.ci".into(),
        database: Some("shop".into()),
        ..Default::default()
    };
    let response = service.completion(&conn, &reader, &ctx).await.unwrap();
    let city = response
        .items
        .iter()
        .find(|item| item.label == "address.city")
        .expect("authorized Mongo nested path must complete");
    assert_eq!(city.insert_text.as_deref(), Some("\"address.city\""));
    assert_eq!(city.score, Some(crate::complete::score::MONGO_INDEX_FIELD));
    assert!(!serde_json::to_string(&response)
        .unwrap()
        .contains("PRIVATE_"));
    assert_eq!(
        driver.reads.load(Ordering::SeqCst),
        2,
        "one bulk graph plus only the selected collection's detail"
    );
    let again = service.completion(&conn, &reader, &ctx).await.unwrap();
    assert!(again.items.iter().any(|item| item.label == "address.city"));
    assert_eq!(
        driver.reads.load(Ordering::SeqCst),
        2,
        "typing must reuse the enriched snapshot"
    );
    let sql = service
        .completion(
            &conn,
            &reader,
            &CompletionContext {
                prefix: "SELECT * FROM profiles WHERE address.ci".into(),
                ..ctx.clone()
            },
        )
        .await
        .unwrap();
    let sql_city = sql
        .items
        .iter()
        .find(|item| item.label == "address.city")
        .expect("the Mongo SQL dialect must reuse the same nested metadata");
    assert_eq!(
        sql_city.insert_text, None,
        "SQL dotted paths remain unquoted"
    );
    assert_eq!(driver.reads.load(Ordering::SeqCst), 2);
    for i in 0..20 {
        service
            .completion(
                &conn,
                &reader,
                &CompletionContext {
                    prefix: format!("db.not_a_collection_{i}.find({{ address.ci"),
                    ..ctx.clone()
                },
            )
            .await
            .unwrap();
    }
    assert_eq!(
        driver.reads.load(Ordering::SeqCst),
        2,
        "unknown typed names must not read metadata"
    );
    assert_eq!(
        service.enforced_completions.snapshot_count(),
        1,
        "field slots must share one base graph snapshot"
    );
    // Even after warming both graph and completion caches, a different database
    // cannot borrow the visible collection's metadata or trigger a driver read.
    let reads = driver.reads.load(Ordering::SeqCst);
    let denied = CompletionContext {
        database: Some("hidden".into()),
        ..ctx
    };
    assert!(service.completion(&conn, &reader, &denied).await.is_err());
    assert_eq!(driver.reads.load(Ordering::SeqCst), reads);
}

#[tokio::test]
async fn enforced_mongo_fields_rebuild_on_policy_change_and_refresh() {
    let (service, conn, reader, driver) = fixture().await;
    let ctx = CompletionContext {
        prefix: "db.profiles.find({ address.ci".into(),
        database: Some("shop".into()),
        ..Default::default()
    };
    service.completion(&conn, &reader, &ctx).await.unwrap();
    assert_eq!(driver.reads.load(Ordering::SeqCst), 2);
    let policies = otto_state::resource_access::ResourceAccessRepo::new(service.connections.pool());
    let policy = policies
        .get_policy(ResourceKind::Connection, &conn)
        .await
        .unwrap();
    let owner = service.connections.get(&conn).await.unwrap().created_by;
    // A policy revision must not reuse the previously scoped nested fields,
    // even while its unchanged authorized base graph remains fresh.
    policies
        .put_policy(
            &policy,
            policy.revision,
            &AccessActor {
                real_user_id: owner.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
    let updated = service.completion(&conn, &reader, &ctx).await.unwrap();
    assert!(updated
        .items
        .iter()
        .any(|item| item.label == "address.city"));
    assert_eq!(driver.reads.load(Ordering::SeqCst), 3);
    assert_eq!(service.enforced_completions.snapshot_count(), 1);
    service
        .refresh_completion_cache(&conn, &reader)
        .await
        .unwrap();
    service.completion(&conn, &reader, &ctx).await.unwrap();
    assert_eq!(
        driver.reads.load(Ordering::SeqCst),
        5,
        "refresh rebuilds graph and fields"
    );
    let mut revoked = policies
        .get_policy(ResourceKind::Connection, &conn)
        .await
        .unwrap();
    revoked.rules.clear();
    policies
        .put_policy(
            &revoked,
            revoked.revision,
            &AccessActor {
                real_user_id: owner,
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
    assert!(service.completion(&conn, &reader, &ctx).await.is_err());
    assert_eq!(
        driver.reads.load(Ordering::SeqCst),
        5,
        "revocation must refuse before reading or returning cached fields"
    );
}
