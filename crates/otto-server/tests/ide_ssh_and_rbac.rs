//! Integration coverage for IDE local-root RBAC, remote SSH connection
//! resolution (`ServerCtx::resolve_ssh_connection`), and `/ws/lsp?connection_id=...`
//! pre-upgrade and WebSocket upgrade guards.

use chrono::Utc;
use futures_util::StreamExt;
use otto_core::access::{AccessActor, AccessRule, ResourceKind, RuleEffect, SubjectKind};
use otto_core::domain::{
    Capability, ConnectionKind, Environment, Feature, User, SCRATCH_WORKSPACE_ID,
};
use otto_core::Error;
use otto_ide::types::IdeTargetSpec;
use otto_ide::IdeCtx;
use otto_rbac::AuthRepo;
use otto_server::lsp::ide_local_root_authorized;
use otto_server::ServerCtx;
use otto_state::{
    ConnectionsRepo, DbPool, GrantsRepo, NewConnection, ResourceAccessRepo, UsersRepo,
};
use serde_json::{json, Value};
use sqlx::Row;

async fn seed_user(pool: &DbPool, id: &str, is_root: bool) -> User {
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, 'x', ?, ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .bind(i64::from(is_root))
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed user");
    UsersRepo::new(pool.clone())
        .get(&id.to_string())
        .await
        .unwrap()
}

async fn seed_workspace(pool: &DbPool, id: &str, root: &std::path::Path) {
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES (?, ?, ?, '{}', 0, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(root.to_string_lossy().into_owned())
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed workspace");
}

async fn add_member(pool: &DbPool, ws_id: &str, user_id: &str, role: &str) {
    sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)")
        .bind(ws_id)
        .bind(user_id)
        .bind(role)
        .execute(pool)
        .await
        .expect("insert workspace member");
}

fn local_target_with_extras(root: &std::path::Path, extras: &[&std::path::Path]) -> IdeTargetSpec {
    let mut spec = IdeTargetSpec::local(root.to_string_lossy());
    spec.additional_roots = extras
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    spec
}

async fn grant_conn_ops(
    pool: &DbPool,
    actor_id: &str,
    conn_id: &str,
    users: &[&str],
    ops: &[&str],
) {
    let access_repo = ResourceAccessRepo::new(pool.clone());
    let cid = conn_id.to_string();
    let mut policy = access_repo
        .get_policy(ResourceKind::Connection, &cid)
        .await
        .unwrap();
    policy.rules = users
        .iter()
        .enumerate()
        .map(|(idx, uid)| AccessRule {
            id: format!("rule-{idx}"),
            subject_kind: SubjectKind::User,
            subject_id: (*uid).to_string(),
            effect: RuleEffect::Allow,
            operations: ops.iter().map(|s| (*s).to_string()).collect(),
            children: None,
            grantable_operations: vec![],
            credential_connection_id: None,
        })
        .collect();
    access_repo
        .put_policy(
            &policy,
            policy.revision,
            &AccessActor {
                real_user_id: actor_id.to_string(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
}

async fn seed_ide_preferences(
    pool: &DbPool,
    owner_id: &str,
    ws_root: &std::path::Path,
    extra_root: &std::path::Path,
) {
    let ws_str = ws_root.to_string_lossy().into_owned();
    let extra_str = extra_root.to_string_lossy().into_owned();
    let roots_json = json!([ws_str, extra_str]).to_string();

    for (table, overrides) in [
        (
            "ide_project_environments",
            json!({
                "project_key": format!("local:{}", ws_root.display()),
                "additional_roots_json": roots_json,
                "extra_roots_json": roots_json,
                "env_vars_json": "[]",
            }),
        ),
        (
            "ide_named_projects",
            json!({
                "id": "saved-proj-1",
                "name": "Saved Project",
                "root_path": ws_root.to_string_lossy(),
                "root": ws_root.to_string_lossy(),
                "additional_roots_json": roots_json,
                "extra_roots_json": roots_json,
                "roots_json": roots_json,
                "shared": 1,
                "is_shared": 1,
            }),
        ),
    ] {
        let pragma = sqlx::query(sqlx::AssertSqlSafe(format!(
            "PRAGMA table_info(\"{table}\")"
        )))
        .fetch_all(pool)
        .await
        .unwrap();
        let map = overrides.as_object().unwrap();
        let mut cols = Vec::new();
        let mut vals: Vec<Value> = Vec::new();
        for col in pragma {
            let name: String = col.get("name");
            let notnull: i64 = col.get("notnull");
            let pk: i64 = col.get("pk");
            let has_default = col
                .try_get::<Option<String>, _>("dflt_value")
                .unwrap()
                .is_some();
            if let Some(v) = map.get(&name) {
                cols.push(name);
                vals.push(v.clone());
            } else if matches!(name.as_str(), "created_by" | "owner_id" | "user_id") {
                cols.push(name);
                vals.push(json!(owner_id));
            } else if name == "workspace_id" {
                cols.push(name);
                vals.push(json!("ws1"));
            } else if (notnull != 0 || pk > 0) && !has_default {
                let col_type: String = col.get("type");
                let v = if col_type.to_ascii_uppercase().contains("INT") {
                    json!(1)
                } else if name.ends_with("_json") {
                    json!("{}")
                } else {
                    json!(format!("{table}-{name}"))
                };
                cols.push(name);
                vals.push(v);
            }
        }
        let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
            "INSERT OR REPLACE INTO \"{table}\" ("
        ));
        qb.push(
            cols.iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(","),
        )
        .push(") VALUES (");
        let mut sep = qb.separated(",");
        for v in &vals {
            match v {
                Value::Number(n) => {
                    sep.push_bind(n.as_i64().unwrap());
                }
                Value::String(s) => {
                    sep.push_bind(s.as_str());
                }
                _ => {
                    sep.push_bind(v.to_string());
                }
            }
        }
        qb.push(")");
        qb.build().execute(pool).await.unwrap();
    }
}

#[tokio::test]
async fn ide_local_root_rbac_enforces_live_workspace_roles_and_revocation() {
    let tmp = tempfile::tempdir().unwrap();
    let ws1_root = tmp.path().join("ws1");
    let ws2_viewer_root = tmp.path().join("ws2-viewer");
    let ungranted_extra = tmp.path().join("outside-extra");
    let scratch_root = tmp.path().join("scratch-ws");
    std::fs::create_dir_all(&ws1_root).unwrap();
    std::fs::create_dir_all(&ws2_viewer_root).unwrap();
    std::fs::create_dir_all(&ungranted_extra).unwrap();
    std::fs::create_dir_all(&scratch_root).unwrap();

    let pool = otto_server::test_support::mem_pool().await;
    let _root = seed_user(&pool, "root", true).await;
    let viewer = seed_user(&pool, "viewer", false).await;
    let editor = seed_user(&pool, "editor", false).await;

    seed_workspace(&pool, "ws1", &ws1_root).await;
    seed_workspace(&pool, "ws2", &ws2_viewer_root).await;
    sqlx::query(
        "INSERT OR REPLACE INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES (?, 'Scratch', ?, '{}', 0, ?)",
    )
    .bind(SCRATCH_WORKSPACE_ID)
    .bind(scratch_root.to_string_lossy().into_owned())
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .expect("seed scratch workspace");
    add_member(&pool, "ws1", "viewer", "viewer").await;
    add_member(&pool, "ws1", "editor", "editor").await;
    // `editor` is only a Viewer on `ws2`.
    add_member(&pool, "ws2", "editor", "viewer").await;

    // Seed saved IDE projects and cached environment `additional_roots` for
    // `viewer` pointing at both `ws1_root` and `ungranted_extra`.
    seed_ide_preferences(&pool, &viewer.id, &ws1_root, &ungranted_extra).await;

    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    ctx.ide.environments.ensure_hydrated(&pool).await;
    let _ = ctx.ide.projects.ensure_hydrated(&pool).await;

    // (a) Workspace Viewer gets `true` for read on `ws1`, `false` / Forbidden
    // when `write = true`, and `false` on `ungranted_extra` even though both
    // are attached to a saved project and `additional_roots`.
    assert!(ide_local_root_authorized(&ctx, &viewer, &ws1_root, false)
        .await
        .unwrap());
    assert!(
        !ide_local_root_authorized(&ctx, &viewer, &ws1_root, true)
            .await
            .unwrap(),
        "workspace viewer must never get write access via saved project or additional_roots"
    );
    assert!(
        !ide_local_root_authorized(&ctx, &viewer, &ungranted_extra, false)
            .await
            .unwrap(),
        "saved project / additional_roots must not grant read access outside live workspaces"
    );
    assert!(
        !ide_local_root_authorized(&ctx, &viewer, &scratch_root, false)
            .await
            .unwrap(),
        "SCRATCH_WORKSPACE_ID must never grant implicit local root access to non-member non-root users"
    );
    let viewer_target = local_target_with_extras(&ws1_root, &[&ws1_root]);
    assert!(matches!(
        ctx.authorize_local_target(&viewer, &viewer_target, true)
            .await,
        Err(Error::Forbidden(_))
    ));

    // (b) Workspace Editor gets `true` when `write = true` on `ws1`, while an
    // additional root in a Viewer-only workspace allows reads (`write = false`)
    // and denies writes (`write = true`), and an ungranted extra root is
    // always denied.
    assert!(ide_local_root_authorized(&ctx, &editor, &ws1_root, true)
        .await
        .unwrap());
    let editor_ws1_only = IdeTargetSpec::local(ws1_root.to_string_lossy());
    ctx.authorize_local_target(&editor, &editor_ws1_only, true)
        .await
        .expect("workspace editor may write inside ws1");

    let editor_with_viewer_extra = local_target_with_extras(&ws1_root, &[&ws2_viewer_root]);
    ctx.authorize_local_target(&editor, &editor_with_viewer_extra, false)
        .await
        .expect("editor on ws1 + viewer on ws2 may read across both roots");
    assert!(
        matches!(
            ctx.authorize_local_target(&editor, &editor_with_viewer_extra, true)
                .await,
            Err(Error::Forbidden(_))
        ),
        "write = true must reject an additional_root where caller is only a Viewer"
    );

    let editor_with_ungranted_extra = local_target_with_extras(&ws1_root, &[&ungranted_extra]);
    assert!(matches!(
        ctx.authorize_local_target(&editor, &editor_with_ungranted_extra, false)
            .await,
        Err(Error::Forbidden(_))
    ));

    // (c) Revoking workspace membership immediately denies both read and write
    // access despite saved projects and hydrated `additional_roots`.
    seed_ide_preferences(&pool, &editor.id, &ws1_root, &ungranted_extra).await;
    ctx.ide.environments.ensure_hydrated(&pool).await;
    let _ = ctx.ide.projects.ensure_hydrated(&pool).await;

    sqlx::query("DELETE FROM workspace_members WHERE workspace_id = 'ws1' AND user_id = ?")
        .bind(&editor.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM workspace_members WHERE workspace_id = 'ws1' AND user_id = ?")
        .bind(&viewer.id)
        .execute(&pool)
        .await
        .unwrap();

    for user in [&editor, &viewer] {
        assert!(
            !ide_local_root_authorized(&ctx, user, &ws1_root, false)
                .await
                .unwrap(),
            "revoked member {} must lose read access immediately",
            user.id
        );
        assert!(
            !ide_local_root_authorized(&ctx, user, &ws1_root, true)
                .await
                .unwrap(),
            "revoked member {} must lose write access immediately",
            user.id
        );
        assert!(matches!(
            ctx.authorize_local_target(user, &editor_ws1_only, false)
                .await,
            Err(Error::Forbidden(_))
        ));
    }
}

#[tokio::test]
async fn resolve_ssh_connection_validates_kind_rbac_policy_and_ssh_params() {
    let tmp = tempfile::tempdir().unwrap();
    let ws1_root = tmp.path().join("ws1");
    std::fs::create_dir_all(&ws1_root).unwrap();

    let pool = otto_server::test_support::mem_pool().await;
    let root = seed_user(&pool, "root", true).await;
    let editor = seed_user(&pool, "editor", false).await;
    let viewer = seed_user(&pool, "viewer", false).await;
    let outsider = seed_user(&pool, "outsider", false).await;

    seed_workspace(&pool, "ws1", &ws1_root).await;
    add_member(&pool, "ws1", "editor", "editor").await;
    add_member(&pool, "ws1", "viewer", "viewer").await;

    let grants = GrantsRepo::new(pool.clone());
    for u in [&editor.id, &viewer.id, &outsider.id] {
        grants
            .set_grants(
                u,
                &[
                    (Feature::Agents, Capability::Edit),
                    (Feature::Connections, Capability::Edit),
                ],
            )
            .await
            .unwrap();
    }

    let conns = ConnectionsRepo::new(pool.clone());
    let valid_ssh = conns
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: "Dev SSH".into(),
            kind: ConnectionKind::Ssh,
            params: json!({
                "host": "devbox.internal",
                "port": 2222,
                "user": "deploy",
                "identity_file": "/keys/id_ed25519",
                "jump": "bastion.internal"
            }),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();

    let mysql_conn = conns
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: "Dev MySQL".into(),
            kind: ConnectionKind::Mysql,
            params: json!({"host": "127.0.0.1", "port": 3306}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();

    let bad_ssh = conns
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: "Injected SSH".into(),
            kind: ConnectionKind::Ssh,
            params: json!({
                "host": "-oProxyCommand=sh",
                "port": 22,
                "user": "deploy"
            }),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();

    let prod_ssh = conns
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: "Prod SSH".into(),
            kind: ConnectionKind::Ssh,
            params: json!({
                "host": "prod.internal",
                "port": 22,
                "user": "ops"
            }),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Prod,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();

    grant_conn_ops(
        &pool,
        &root.id,
        &valid_ssh.id,
        &[&editor.id, &viewer.id],
        &["discover", "shell"],
    )
    .await;
    grant_conn_ops(
        &pool,
        &root.id,
        &mysql_conn.id,
        &[&editor.id],
        &["discover", "shell"],
    )
    .await;
    grant_conn_ops(
        &pool,
        &root.id,
        &bad_ssh.id,
        &[&editor.id],
        &["discover", "shell"],
    )
    .await;

    // Grant `editor` only `discover` + `sftp_read` (not `shell`) on `prod_ssh` initially.
    let access_repo = ResourceAccessRepo::new(pool.clone());
    let mut policy = access_repo
        .get_policy(ResourceKind::Connection, &prod_ssh.id)
        .await
        .unwrap();
    policy.rules = vec![AccessRule {
        id: "read-only-rule".into(),
        subject_kind: SubjectKind::User,
        subject_id: editor.id.clone(),
        effect: RuleEffect::Allow,
        operations: vec!["discover".into(), "sftp_read".into()],
        children: None,
        grantable_operations: vec![],
        credential_connection_id: None,
    }];
    let actor = AccessActor {
        real_user_id: root.id.clone(),
        effective_user_id: None,
    };
    policy = access_repo
        .put_policy(&policy, policy.revision, &actor)
        .await
        .unwrap();

    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;

    // 1. Valid SSH connection resolves cleanly for workspace Editor.
    let resolved = ctx
        .resolve_ssh_connection(&editor, &valid_ssh.id)
        .await
        .expect("valid SSH connection should resolve");
    assert_eq!(resolved.connection_id, valid_ssh.id);
    assert_eq!(resolved.name, "Dev SSH");
    assert_eq!(resolved.host, "devbox.internal");
    assert_eq!(resolved.port, Some(2222));
    assert_eq!(resolved.user, "deploy");
    assert_eq!(resolved.identity_file.as_deref(), Some("/keys/id_ed25519"));
    assert_eq!(resolved.proxy_jump.as_deref(), Some("bastion.internal"));

    // 2. Non-SSH DB connection is rejected with Invalid (400).
    assert!(matches!(
        ctx.resolve_ssh_connection(&editor, &mysql_conn.id).await,
        Err(Error::Invalid(_))
    ));

    // 3. Missing connection ID is rejected with NotFound (404).
    assert!(matches!(
        ctx.resolve_ssh_connection(&editor, &"missing-conn-id".to_string())
            .await,
        Err(Error::NotFound(_))
    ));

    // 4. Workspace Viewer and non-member are rejected.
    assert!(matches!(
        ctx.resolve_ssh_connection(&viewer, &valid_ssh.id).await,
        Err(Error::Forbidden(_))
    ));
    assert!(matches!(
        ctx.resolve_ssh_connection(&outsider, &valid_ssh.id).await,
        Err(Error::Forbidden(_) | Error::NotFound(_))
    ));

    // 5. Option-like SSH parameters (`-oProxyCommand=...`) are rejected.
    assert!(matches!(
        ctx.resolve_ssh_connection(&editor, &bad_ssh.id).await,
        Err(Error::Invalid(_))
    ));

    // 6. Enforced Prod SSH policy without `"shell"` operation denies, then
    // succeeds once `"shell"` is granted.
    assert!(matches!(
        ctx.resolve_ssh_connection(&editor, &prod_ssh.id).await,
        Err(Error::Forbidden(_))
    ));
    policy.rules[0].operations = vec!["discover".into(), "shell".into()];
    access_repo
        .put_policy(&policy, policy.revision, &actor)
        .await
        .unwrap();
    let prod_resolved = ctx
        .resolve_ssh_connection(&editor, &prod_ssh.id)
        .await
        .expect("prod SSH with shell grant should resolve");
    assert_eq!(prod_resolved.host, "prod.internal");
    assert_eq!(prod_resolved.user, "ops");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ws_lsp_remote_ssh_guards_and_upgrade_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let ws1_root = tmp.path().join("ws1");
    std::fs::create_dir_all(&ws1_root).unwrap();

    let pool = otto_server::test_support::mem_pool().await;
    let root = seed_user(&pool, "root", true).await;
    let editor = seed_user(&pool, "editor", false).await;
    let viewer = seed_user(&pool, "viewer", false).await;
    let outsider = seed_user(&pool, "outsider", false).await;
    let no_agents = seed_user(&pool, "no_agents", false).await;

    seed_workspace(&pool, "ws1", &ws1_root).await;
    add_member(&pool, "ws1", "editor", "editor").await;
    add_member(&pool, "ws1", "viewer", "viewer").await;
    add_member(&pool, "ws1", "no_agents", "editor").await;

    let grants = GrantsRepo::new(pool.clone());
    for u in [&editor.id, &viewer.id, &outsider.id] {
        grants
            .set_grants(
                u,
                &[
                    (Feature::Agents, Capability::Edit),
                    (Feature::Connections, Capability::Edit),
                ],
            )
            .await
            .unwrap();
    }
    grants
        .set_grants(&no_agents.id, &[(Feature::Connections, Capability::Edit)])
        .await
        .unwrap();

    let conns = ConnectionsRepo::new(pool.clone());
    let ssh_conn = conns
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: "Remote Dev".into(),
            kind: ConnectionKind::Ssh,
            // Port 1 on loopback refuses immediately when the upgraded WS tries
            // to spawn `ssh`, without touching external networks.
            params: json!({"host": "127.0.0.1", "port": 1, "user": "dev"}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();

    let mysql_conn = conns
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: "DB".into(),
            kind: ConnectionKind::Mysql,
            params: json!({"host": "127.0.0.1", "port": 3306}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();

    let prod_ssh = conns
        .create(NewConnection {
            workspace_id: Some("ws1".into()),
            name: "Prod Locked SSH".into(),
            kind: ConnectionKind::Ssh,
            params: json!({"host": "127.0.0.1", "port": 1, "user": "ops"}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Prod,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();

    grant_conn_ops(
        &pool,
        &root.id,
        &ssh_conn.id,
        &[&editor.id, &viewer.id, &no_agents.id],
        &["discover", "shell"],
    )
    .await;
    grant_conn_ops(
        &pool,
        &root.id,
        &mysql_conn.id,
        &[&editor.id],
        &["discover", "shell"],
    )
    .await;
    grant_conn_ops(
        &pool,
        &root.id,
        &prod_ssh.id,
        &[&editor.id],
        &["discover", "sftp_read"],
    )
    .await;

    let auth = AuthRepo::new(pool.clone());
    let editor_tok = auth
        .issue_api_token(&editor.id, Some("ui"))
        .await
        .unwrap()
        .0;
    let viewer_tok = auth
        .issue_api_token(&viewer.id, Some("ui"))
        .await
        .unwrap()
        .0;
    let outsider_tok = auth
        .issue_api_token(&outsider.id, Some("ui"))
        .await
        .unwrap()
        .0;
    let no_agents_tok = auth
        .issue_api_token(&no_agents.id, Some("ui"))
        .await
        .unwrap()
        .0;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let mut ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    ctx.base_url = base.clone();
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let client = reqwest::Client::new();
    let probe = |token: &str, lang: &str, remote_root: &str, conn_id: &str| {
        let client = client.clone();
        let url = format!("{base}/ws/lsp");
        let token = token.to_string();
        let lang = lang.to_string();
        let remote_root = remote_root.to_string();
        let conn_id = conn_id.to_string();
        async move {
            let resp = client
                .get(url)
                .query(&[
                    ("lang", lang.as_str()),
                    ("root", remote_root.as_str()),
                    ("connection_id", conn_id.as_str()),
                ])
                .header("Connection", "Upgrade")
                .header("Upgrade", "websocket")
                .header("Sec-WebSocket-Version", "13")
                .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
                .header("Sec-WebSocket-Protocol", format!("otto-bearer, {token}"))
                .send()
                .await
                .unwrap();
            let status = resp.status().as_u16();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            (status, body)
        }
    };

    // Relative remote root is rejected with 400 before SSH lookup.
    let (st, body) = probe(&editor_tok, "rust", "relative/repo", &ssh_conn.id).await;
    assert_eq!(st, 400, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("absolute path"),
        "{body}"
    );

    // Missing connection → 404.
    let (st, _) = probe(&editor_tok, "rust", "/srv/repo", "no-such-conn").await;
    assert_eq!(st, 404);

    // Non-SSH connection → 400.
    let (st, _) = probe(&editor_tok, "rust", "/srv/repo", &mysql_conn.id).await;
    assert_eq!(st, 400);

    // Viewer role on workspace → 403.
    let (st, _) = probe(&viewer_tok, "rust", "/srv/repo", &ssh_conn.id).await;
    assert_eq!(st, 403);

    // Non-member of workspace → 403 or 404.
    let (st, _) = probe(&outsider_tok, "rust", "/srv/repo", &ssh_conn.id).await;
    assert!(matches!(st, 403 | 404), "outsider got {st}");

    // Non-root user without `Feature::Agents` grant is blocked at `RootRoute::Lsp` → 403.
    let (st, _) = probe(&no_agents_tok, "rust", "/srv/repo", &ssh_conn.id).await;
    assert_eq!(st, 403);

    // Enforced Prod SSH connection without shell grant → 403.
    let (st, _) = probe(&editor_tok, "rust", "/srv/repo", &prod_ssh.id).await;
    assert_eq!(st, 403);

    // Valid SSH connection + Editor passes SSH auth & resolution and reaches
    // language resolution (`unknown language` 400 proves SSH guard passed).
    let (st, body) = probe(&editor_tok, "otto-test-none", "/srv/repo", &ssh_conn.id).await;
    assert_eq!(st, 400, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown language"),
        "{body}"
    );

    // Valid SSH connection + known remote language (`rust`) upgrades to 101 WebSocket.
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let ws_url = format!(
        "ws://{addr}/ws/lsp?lang=rust&root=/srv/repo&connection_id={}",
        ssh_conn.id
    );
    let mut req = ws_url.into_client_request().unwrap();
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        format!("otto-bearer, {editor_tok}").parse().unwrap(),
    );
    let (mut ws_stream, resp) = tokio_tungstenite::connect_async(req)
        .await
        .expect("authorized SSH /ws/lsp request should upgrade to WebSocket");
    assert_eq!(resp.status().as_u16(), 101);

    let first_msg = tokio::time::timeout(std::time::Duration::from_secs(3), ws_stream.next())
        .await
        .expect("receive initial WS LSP frame")
        .expect("WS stream open")
        .expect("WS text frame");
    let attached_json: Value = serde_json::from_str(&first_msg.into_text().unwrap()).unwrap();
    assert_eq!(attached_json["method"], "otto/lspAttached");
    assert_eq!(attached_json["params"]["lang"], "rust");
    assert_eq!(attached_json["params"]["root"], "/srv/repo");
}
