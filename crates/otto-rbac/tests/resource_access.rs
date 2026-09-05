use otto_core::access::{
    operations_for, AccessActor, AccessMode, AccessPolicy, AccessRule, ResourceKind, ResourceRef,
    RuleEffect, SubjectKind,
};
use otto_core::domain::User;
use otto_core::{Error, Id};
use otto_rbac::ResourceAccess;
use otto_state::{ResourceAccessRepo, UsersRepo};

async fn user(pool: &sqlx::SqlitePool, name: &str, root: bool) -> User {
    UsersRepo::new(pool.clone())
        .create(name, "hash", name, root)
        .await
        .unwrap()
}

/// Insert a real connection, then remove the trigger-created policy to model an
/// existing pre-0115 resource that must retain rollout-compatible Legacy mode.
async fn legacy_connection(pool: &sqlx::SqlitePool, id: &str, owner: &User) {
    sqlx::query(
        "INSERT INTO connections
         (id, name, kind, params_json, created_by, created_at)
         VALUES (?, ?, 'mysql', '{}', ?, '2026-09-05T00:00:00Z')",
    )
    .bind(id)
    .bind(id)
    .bind(&owner.id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "DELETE FROM resource_access_policies
         WHERE resource_kind = 'connection' AND resource_id = ?",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

fn actor(user: &User) -> AccessActor {
    AccessActor {
        real_user_id: user.id.clone(),
        effective_user_id: None,
    }
}

fn resource(id: &str, child: Option<&str>) -> ResourceRef {
    ResourceRef {
        kind: ResourceKind::Connection,
        id: id.into(),
        child: child.map(str::to_owned),
    }
}

fn rule(
    id: &str,
    subject_kind: SubjectKind,
    subject_id: &str,
    effect: RuleEffect,
    operations: &[&str],
    children: Option<&[&str]>,
) -> AccessRule {
    AccessRule {
        id: id.into(),
        subject_kind,
        subject_id: subject_id.into(),
        effect,
        operations: operations.iter().map(|v| (*v).to_owned()).collect(),
        children: children.map(|values| values.iter().map(|v| (*v).to_owned()).collect()),
        grantable_operations: Vec::new(),
        credential_connection_id: None,
    }
}

fn enforced(resource_id: &str, rules: Vec<AccessRule>) -> AccessPolicy {
    AccessPolicy {
        kind: ResourceKind::Connection,
        resource_id: resource_id.into(),
        mode: AccessMode::Enforced,
        revision: 0,
        rules,
    }
}

#[tokio::test]
async fn direct_denial_restricts_one_group_member_without_restricting_peers() {
    // Catches a missing deny-wins branch or accidental sharing across resources.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let access = ResourceAccess::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let alice = user(&pool, "alice", false).await;
    let peer = user(&pool, "peer", false).await;
    let bob = user(&pool, "bob", false).await;
    let outsider = user(&pool, "outsider", false).await;
    legacy_connection(&pool, "connection-a", &root).await;
    legacy_connection(&pool, "connection-b", &root).await;

    let dba = repo
        .create_group("DBA", Some("database administrators"), &actor(&root))
        .await
        .unwrap();
    repo.add_group_member(&dba.id, &alice.id, &actor(&root))
        .await
        .unwrap();
    repo.add_group_member(&dba.id, &peer.id, &actor(&root))
        .await
        .unwrap();

    repo.put_policy(
        &enforced(
            "connection-a",
            vec![
                rule(
                    "dba-a",
                    SubjectKind::Group,
                    &dba.id,
                    RuleEffect::Allow,
                    &["discover", "db_query", "db_schema"],
                    None,
                ),
                rule(
                    "alice-no-schema",
                    SubjectKind::User,
                    &alice.id,
                    RuleEffect::Deny,
                    &["db_schema"],
                    None,
                ),
            ],
        ),
        0,
        &actor(&root),
    )
    .await
    .unwrap();
    repo.put_policy(
        &enforced(
            "connection-b",
            vec![rule(
                "bob-b",
                SubjectKind::User,
                &bob.id,
                RuleEffect::Allow,
                &["discover", "db_query"],
                None,
            )],
        ),
        0,
        &actor(&root),
    )
    .await
    .unwrap();

    let alice_schema = access
        .evaluate(&alice, &resource("connection-a", None), "db_schema")
        .await
        .unwrap();
    assert!(!alice_schema.allowed);
    assert_eq!(alice_schema.matched_rule_ids, vec!["alice-no-schema"]);
    assert!(
        access
            .evaluate(&alice, &resource("connection-a", None), "db_query")
            .await
            .unwrap()
            .allowed
    );
    assert!(
        access
            .evaluate(&peer, &resource("connection-a", None), "db_schema")
            .await
            .unwrap()
            .allowed
    );
    assert!(
        access
            .evaluate(&bob, &resource("connection-b", None), "db_query")
            .await
            .unwrap()
            .allowed
    );
    assert!(
        !access
            .evaluate(&bob, &resource("connection-a", None), "db_query")
            .await
            .unwrap()
            .allowed
    );
    assert!(
        !access
            .evaluate(&outsider, &resource("connection-a", None), "discover")
            .await
            .unwrap()
            .allowed
    );
}

#[tokio::test]
async fn child_scope_is_exact_and_denies_win_across_multiple_groups() {
    // Catches treating named children as all children and allow overriding deny.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let access = ResourceAccess::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let alice = user(&pool, "alice", false).await;
    legacy_connection(&pool, "connection-a", &root).await;
    let readers = repo
        .create_group("readers", None, &actor(&root))
        .await
        .unwrap();
    let restricted = repo
        .create_group("restricted", None, &actor(&root))
        .await
        .unwrap();
    repo.add_group_member(&readers.id, &alice.id, &actor(&root))
        .await
        .unwrap();
    repo.add_group_member(&restricted.id, &alice.id, &actor(&root))
        .await
        .unwrap();

    repo.put_policy(
        &enforced(
            "connection-a",
            vec![
                rule(
                    "finance-readers",
                    SubjectKind::Group,
                    &readers.id,
                    RuleEffect::Allow,
                    &["discover", "db_query"],
                    Some(&["finance"]),
                ),
                rule(
                    "restricted-no-query",
                    SubjectKind::Group,
                    &restricted.id,
                    RuleEffect::Deny,
                    &["db_query"],
                    Some(&["finance"]),
                ),
                rule(
                    "alice-query",
                    SubjectKind::User,
                    &alice.id,
                    RuleEffect::Allow,
                    &["db_query"],
                    Some(&["finance"]),
                ),
            ],
        ),
        0,
        &actor(&root),
    )
    .await
    .unwrap();

    assert!(
        !access
            .evaluate(
                &alice,
                &resource("connection-a", Some("finance")),
                "db_query",
            )
            .await
            .unwrap()
            .allowed
    );
    assert!(
        !access
            .evaluate(
                &alice,
                &resource("connection-a", Some("operations")),
                "db_query",
            )
            .await
            .unwrap()
            .allowed
    );
    assert!(
        access
            .evaluate(&alice, &resource("connection-a", None), "discover")
            .await
            .unwrap()
            .allowed
    );
}

#[tokio::test]
async fn legacy_root_and_live_membership_have_distinct_behavior() {
    // Catches caching group membership and treating missing policy as enforced.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let access = ResourceAccess::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let alice = user(&pool, "alice", false).await;
    legacy_connection(&pool, "legacy-connection", &root).await;
    legacy_connection(&pool, "connection-a", &root).await;

    let missing = access
        .evaluate(&alice, &resource("legacy-connection", None), "db_query")
        .await
        .unwrap();
    assert!(missing.allowed);
    assert_eq!(missing.mode, AccessMode::Legacy);

    let group = repo
        .create_group("temporary", None, &actor(&root))
        .await
        .unwrap();
    repo.add_group_member(&group.id, &alice.id, &actor(&root))
        .await
        .unwrap();
    repo.put_policy(
        &enforced(
            "connection-a",
            vec![rule(
                "temporary-query",
                SubjectKind::Group,
                &group.id,
                RuleEffect::Allow,
                &["discover", "db_query"],
                None,
            )],
        ),
        0,
        &actor(&root),
    )
    .await
    .unwrap();

    assert!(
        access
            .evaluate(&alice, &resource("connection-a", None), "db_query")
            .await
            .unwrap()
            .allowed
    );
    repo.remove_group_member(&group.id, &alice.id, &actor(&root))
        .await
        .unwrap();
    assert!(
        !access
            .evaluate(&alice, &resource("connection-a", None), "db_query")
            .await
            .unwrap()
            .allowed
    );
    assert!(
        access
            .evaluate(&root, &resource("connection-a", None), "db_schema")
            .await
            .unwrap()
            .allowed
    );

    let disabled_root = UsersRepo::new(pool.clone())
        .update(&root.id, None, None, Some(true))
        .await
        .unwrap();
    assert!(
        !access
            .evaluate(&disabled_root, &resource("connection-a", None), "db_query",)
            .await
            .unwrap()
            .allowed
    );
}

#[tokio::test]
async fn policy_writes_compare_and_swap_and_validate_rules() {
    // Catches stale policy overwrite and accepting malformed scope/subjects/rules.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let alice = user(&pool, "alice", false).await;

    let first = repo
        .put_policy(
            &enforced(
                "connection-a",
                vec![rule(
                    "alice-query",
                    SubjectKind::User,
                    &alice.id,
                    RuleEffect::Allow,
                    &["db_query"],
                    None,
                )],
            ),
            0,
            &actor(&root),
        )
        .await
        .unwrap();
    assert_eq!(first.revision, 1);

    let stale = repo.put_policy(&first, 0, &actor(&root)).await.unwrap_err();
    assert!(matches!(stale, Error::Conflict(_)));

    let mut invalid = enforced(
        "connection-b",
        vec![rule(
            "empty-child",
            SubjectKind::User,
            &alice.id,
            RuleEffect::Allow,
            &["db_query"],
            Some(&[]),
        )],
    );
    assert!(matches!(
        repo.put_policy(&invalid, 0, &actor(&root)).await,
        Err(Error::Invalid(_))
    ));

    invalid.rules[0].children = None;
    invalid.rules[0].operations = vec!["not_an_operation".into()];
    assert!(matches!(
        repo.put_policy(&invalid, 0, &actor(&root)).await,
        Err(Error::Invalid(_))
    ));

    invalid.rules[0].operations = vec!["db_query".into()];
    invalid.rules.push(invalid.rules[0].clone());
    assert!(matches!(
        repo.put_policy(&invalid, 0, &actor(&root)).await,
        Err(Error::Invalid(_))
    ));

    invalid.rules.truncate(1);
    invalid.rules[0].subject_id = "missing-user".into();
    assert!(matches!(
        repo.put_policy(&invalid, 0, &actor(&root)).await,
        Err(Error::Invalid(_))
    ));

    invalid.rules[0].subject_id = alice.id.clone();
    invalid.rules[0].effect = RuleEffect::Deny;
    invalid.rules[0].grantable_operations = vec!["db_query".into()];
    assert!(matches!(
        repo.put_policy(&invalid, 0, &actor(&root)).await,
        Err(Error::Invalid(_))
    ));

    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE action = 'resource_access.policy_update'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit_count, 1);
}

#[tokio::test]
async fn roles_are_validated_presets_and_owner_initialization_is_enforced() {
    // Catches mutable-role binding behavior and incomplete owner initialization.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let access = ResourceAccess::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let owner = user(&pool, "owner", false).await;
    legacy_connection(&pool, "connection-new", &root).await;

    let role = repo
        .create_role(
            "database reader",
            Some("query-only preset"),
            ResourceKind::Connection,
            &["discover".into(), "db_query".into()],
            &["db_query".into()],
            &actor(&root),
        )
        .await
        .unwrap();
    assert_eq!(repo.list_roles().await.unwrap(), vec![role]);

    assert!(matches!(
        repo.create_role(
            "bad",
            None,
            ResourceKind::McpServer,
            &["db_query".into()],
            &[],
            &actor(&root),
        )
        .await,
        Err(Error::Invalid(_))
    ));

    let initialized = repo
        .initialize_owner_policy(
            ResourceKind::Connection,
            &"connection-new".to_owned(),
            &owner.id,
            &["discover".into(), "db_query".into(), "manage_access".into()],
            &["db_query".into()],
            &actor(&root),
        )
        .await
        .unwrap();
    assert_eq!(initialized.mode, AccessMode::Enforced);
    assert_eq!(initialized.revision, 1);
    assert!(
        access
            .evaluate(&owner, &resource("connection-new", None), "manage_access",)
            .await
            .unwrap()
            .allowed
    );
}

#[test]
fn operation_catalogues_cover_cloud_and_cluster_resources() {
    // Catches accepting an operation for the wrong resource family.
    assert!(operations_for(ResourceKind::AwsAccount).contains(&"s3_read"));
    assert!(operations_for(ResourceKind::AwsAccount).contains(&"ec2_terminate"));
    assert!(operations_for(ResourceKind::AwsAccount).contains(&"athena_query"));
    assert!(operations_for(ResourceKind::AwsAccount).contains(&"sqs_redrive"));
    assert!(operations_for(ResourceKind::AwsAccount).contains(&"eks_import"));
    assert!(!operations_for(ResourceKind::AwsAccount).contains(&"exec"));

    assert!(operations_for(ResourceKind::K8sCluster).contains(&"workloads_view"));
    assert!(operations_for(ResourceKind::K8sCluster).contains(&"exec"));
    assert!(operations_for(ResourceKind::K8sCluster).contains(&"k9s"));
    assert!(operations_for(ResourceKind::K8sCluster).contains(&"secrets_view"));
    assert!(operations_for(ResourceKind::K8sCluster).contains(&"delete"));
    assert!(!operations_for(ResourceKind::K8sCluster).contains(&"s3_read"));
}

#[tokio::test]
async fn raw_resource_insert_is_enforced_empty_before_owner_initialization() {
    // Catches a legacy-access window in create/import paths and stale same-id reuse.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let owner = user(&pool, "owner", false).await;
    let connection_id = "trigger-connection".to_owned();

    sqlx::query(
        "INSERT INTO connections
         (id, name, kind, params_json, created_by, created_at)
         VALUES (?, 'trigger', 'mysql', '{}', ?, '2026-09-05T00:00:00Z')",
    )
    .bind(&connection_id)
    .bind(&owner.id)
    .execute(&pool)
    .await
    .unwrap();

    let initial = repo
        .get_policy(ResourceKind::Connection, &connection_id)
        .await
        .unwrap();
    assert_eq!(initial.mode, AccessMode::Enforced);
    assert_eq!(initial.revision, 1);
    assert!(initial.rules.is_empty());

    let initialized = repo
        .initialize_owner_policy(
            ResourceKind::Connection,
            &connection_id,
            &owner.id,
            &["discover".into(), "db_query".into()],
            &[],
            &actor(&root),
        )
        .await
        .unwrap();
    assert_eq!(initialized.revision, 2);

    sqlx::query("DELETE FROM connections WHERE id = ?")
        .bind(&connection_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO connections
         (id, name, kind, params_json, created_by, created_at)
         VALUES (?, 'trigger-again', 'mysql', '{}', ?, '2026-09-05T00:01:00Z')",
    )
    .bind(&connection_id)
    .bind(&owner.id)
    .execute(&pool)
    .await
    .unwrap();
    let recreated = repo
        .get_policy(ResourceKind::Connection, &connection_id)
        .await
        .unwrap();
    assert_eq!(recreated.revision, 1);
    assert!(recreated.rules.is_empty());
}

#[tokio::test]
async fn failed_policy_audit_rolls_back_the_policy_head_and_version() {
    // Catches splitting the policy mutation from its mandatory audit record.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let alice = user(&pool, "alice", false).await;
    sqlx::query(
        "CREATE TRIGGER reject_resource_access_audit
         BEFORE INSERT ON audit_log
         WHEN NEW.action = 'resource_access.policy_update'
         BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END",
    )
    .execute(&pool)
    .await
    .unwrap();

    let result = repo
        .put_policy(
            &enforced(
                "connection-a",
                vec![rule(
                    "alice-query",
                    SubjectKind::User,
                    &alice.id,
                    RuleEffect::Allow,
                    &["db_query"],
                    None,
                )],
            ),
            0,
            &actor(&root),
        )
        .await;
    assert!(matches!(result, Err(Error::Internal(_))));
    let policy = repo
        .get_policy(ResourceKind::Connection, &"connection-a".into())
        .await
        .unwrap();
    assert_eq!(
        policy,
        AccessPolicy::legacy(ResourceKind::Connection, "connection-a".into())
    );
    let versions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM resource_access_policy_versions WHERE resource_id = 'connection-a'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(versions, 0);
}

#[tokio::test]
async fn group_and_role_editing_primitives_round_trip() {
    // Catches partial CRUD implementations that cannot support management routes.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let alice = user(&pool, "alice", false).await;
    let act = actor(&root);

    let group = repo.create_group("ops", None, &act).await.unwrap();
    repo.add_group_member(&group.id, &alice.id, &act)
        .await
        .unwrap();
    assert_eq!(
        repo.group_members(&group.id).await.unwrap(),
        vec![alice.id.clone()]
    );
    assert_eq!(
        repo.groups_for_user(&alice.id).await.unwrap(),
        vec![group.id.clone()]
    );
    let renamed = repo
        .update_group(&group.id, "operators", Some("on-call"), &act)
        .await
        .unwrap();
    assert_eq!(renamed.name, "operators");
    repo.delete_group(&group.id, &act).await.unwrap();
    assert!(matches!(
        repo.get_group(&group.id).await,
        Err(Error::NotFound(_))
    ));

    let role = repo
        .create_role(
            "reader",
            None,
            ResourceKind::Connection,
            &["discover".into(), "db_query".into()],
            &[],
            &act,
        )
        .await
        .unwrap();
    let updated = repo
        .update_role(
            &role.id,
            "reader-plus-export",
            None,
            ResourceKind::Connection,
            &["discover".into(), "db_query".into(), "db_export".into()],
            &[],
            &act,
        )
        .await
        .unwrap();
    assert!(updated.operations.contains(&"db_export".into()));
    repo.delete_role(&role.id, &act).await.unwrap();
    assert!(matches!(
        repo.get_role(&role.id).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn live_evaluation_distinguishes_deleted_resources_from_existing_legacy_resources() {
    // Catches policy deletion turning a missing resource into legacy-allowed access.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let access = ResourceAccess::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let alice = user(&pool, "alice", false).await;
    legacy_connection(&pool, "legacy-existing", &root).await;
    legacy_connection(&pool, "delete-me", &root).await;

    assert!(
        access
            .evaluate(&alice, &resource("legacy-existing", None), "db_query")
            .await
            .unwrap()
            .allowed
    );
    assert!(matches!(
        access
            .evaluate(&alice, &resource("never-existed", None), "db_query")
            .await,
        Err(Error::NotFound(_))
    ));

    repo.put_policy(
        &enforced(
            "delete-me",
            vec![rule(
                "alice-read",
                SubjectKind::User,
                &alice.id,
                RuleEffect::Allow,
                &["discover", "db_query"],
                None,
            )],
        ),
        0,
        &actor(&root),
    )
    .await
    .unwrap();
    assert!(
        access
            .evaluate(&alice, &resource("delete-me", None), "db_query")
            .await
            .unwrap()
            .allowed
    );
    sqlx::query("DELETE FROM connections WHERE id = 'delete-me'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        access
            .evaluate(&alice, &resource("delete-me", None), "db_query")
            .await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn child_derived_parent_discovery_requires_one_child_to_survive_denies() {
    // Catches flattening child allows at the parent while ignoring child denies.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let access = ResourceAccess::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let alice = user(&pool, "alice", false).await;
    legacy_connection(&pool, "connection-a", &root).await;
    let readers = repo
        .create_group("readers", None, &actor(&root))
        .await
        .unwrap();
    repo.add_group_member(&readers.id, &alice.id, &actor(&root))
        .await
        .unwrap();

    let mut policy = enforced(
        "connection-a",
        vec![
            rule(
                "group-discover-finance",
                SubjectKind::Group,
                &readers.id,
                RuleEffect::Allow,
                &["discover"],
                Some(&["finance"]),
            ),
            rule(
                "alice-deny-finance",
                SubjectKind::User,
                &alice.id,
                RuleEffect::Deny,
                &["discover"],
                Some(&["finance"]),
            ),
        ],
    );
    policy = repo.put_policy(&policy, 0, &actor(&root)).await.unwrap();
    assert!(
        !access
            .evaluate(&alice, &resource("connection-a", None), "discover")
            .await
            .unwrap()
            .allowed
    );

    policy.rules[0].children = Some(vec!["finance".into(), "hr".into()]);
    policy = repo.put_policy(&policy, 1, &actor(&root)).await.unwrap();
    assert!(
        access
            .evaluate(&alice, &resource("connection-a", None), "discover")
            .await
            .unwrap()
            .allowed
    );

    policy.rules[1].children = Some(vec!["finance".into(), "hr".into()]);
    policy = repo.put_policy(&policy, 2, &actor(&root)).await.unwrap();
    assert!(
        !access
            .evaluate(&alice, &resource("connection-a", None), "discover")
            .await
            .unwrap()
            .allowed
    );

    policy.rules.push(rule(
        "alice-parent-discover",
        SubjectKind::User,
        &alice.id,
        RuleEffect::Allow,
        &["discover"],
        None,
    ));
    repo.put_policy(&policy, 3, &actor(&root)).await.unwrap();
    assert!(
        access
            .evaluate(&alice, &resource("connection-a", None), "discover")
            .await
            .unwrap()
            .allowed
    );
}

#[tokio::test]
async fn non_discover_operations_require_discovery_at_the_same_effective_scope() {
    // Catches action grants bypassing the resource/child visibility prerequisite.
    let pool = otto_state::db::test_pool().await;
    let repo = ResourceAccessRepo::new(pool.clone());
    let access = ResourceAccess::new(pool.clone());
    let root = user(&pool, "root", true).await;
    let alice = user(&pool, "alice", false).await;
    legacy_connection(&pool, "connection-a", &root).await;

    let mut policy = enforced(
        "connection-a",
        vec![
            rule(
                "alice-finance-query",
                SubjectKind::User,
                &alice.id,
                RuleEffect::Allow,
                &["db_query"],
                Some(&["finance"]),
            ),
            rule(
                "alice-hr-discover",
                SubjectKind::User,
                &alice.id,
                RuleEffect::Allow,
                &["discover"],
                Some(&["hr"]),
            ),
        ],
    );
    policy = repo.put_policy(&policy, 0, &actor(&root)).await.unwrap();
    let blocked = access
        .evaluate(
            &alice,
            &resource("connection-a", Some("finance")),
            "db_query",
        )
        .await
        .unwrap();
    assert!(!blocked.allowed);
    assert_eq!(blocked.reason, "discover_required");

    policy.rules[1].children = Some(vec!["finance".into()]);
    policy = repo.put_policy(&policy, 1, &actor(&root)).await.unwrap();
    assert!(
        access
            .evaluate(
                &alice,
                &resource("connection-a", Some("finance")),
                "db_query",
            )
            .await
            .unwrap()
            .allowed
    );

    policy.rules[0].operations = vec!["db_schema".into()];
    policy.rules[0].children = None;
    policy = repo.put_policy(&policy, 2, &actor(&root)).await.unwrap();
    assert!(
        !access
            .evaluate(&alice, &resource("connection-a", None), "db_schema")
            .await
            .unwrap()
            .allowed
    );

    policy.rules[1].children = None;
    repo.put_policy(&policy, 3, &actor(&root)).await.unwrap();
    assert!(
        access
            .evaluate(&alice, &resource("connection-a", None), "db_schema")
            .await
            .unwrap()
            .allowed
    );
    assert!(
        access
            .evaluate(&root, &resource("connection-a", None), "db_schema")
            .await
            .unwrap()
            .allowed
    );
}

#[allow(dead_code)]
fn _ids_are_strings(id: Id) -> String {
    id
}
