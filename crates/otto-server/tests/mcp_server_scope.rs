//! Where an MCP server is available (`otto_state::mcp_scope`), end to end
//! through the REAL daemon router: one server row shared with chosen
//! workspaces or with all of them (including one created afterwards), and the
//! access rules around it —
//! - a member of a workspace it is shared with can use it, but never sees its
//!   configuration (command / env / where else it is shared);
//! - a non-member of every workspace it reaches cannot see it, even when its
//!   access policy names them;
//! - its settings, and where it is available, stay with the home workspace;
//!   changing where it is available is the owner's alone;
//! - an Enforced access policy still decides who may use it.

use chrono::Utc;
use otto_core::access::{AccessActor, AccessMode, AccessPolicy, AccessRule};
use otto_core::access::{ResourceKind, RuleEffect, SubjectKind};
use otto_core::domain::{Capability, Feature};
use otto_rbac::AuthRepo;
use otto_state::DbPool;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

struct Daemon {
    base: String,
    http: reqwest::Client,
    pool: DbPool,
    _tmp: tempfile::TempDir,
}

impl Daemon {
    async fn send(
        &self,
        method: &str,
        token: &str,
        path: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let mut req = self
            .http
            .request(m, format!("{}/api/v1{path}", self.base))
            .bearer_auth(token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn user(&self, id: &str, root: bool, member_of: &[(&str, &str)]) -> String {
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, 'x', ?, ?, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(id)
        .bind(root as i64)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .unwrap();
        for (ws, role) in member_of {
            self.member(ws, id, role).await;
        }
        otto_state::GrantsRepo::new(self.pool.clone())
            .set_grants(id, &[(Feature::Mcp, Capability::Admin)])
            .await
            .unwrap();
        AuthRepo::new(self.pool.clone())
            .issue_api_token(&id.to_string(), Some("ui"))
            .await
            .unwrap()
            .0
    }

    async fn workspace(&self, id: &str) {
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
             VALUES (?, ?, '/tmp', '{}', 0, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn member(&self, ws: &str, user: &str, role: &str) {
        sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, ?)")
            .bind(ws)
            .bind(user)
            .bind(role)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    /// The servers `token`'s user sees in workspace `ws` (by id).
    async fn listed(&self, token: &str, ws: &str, id: &str) -> Option<Value> {
        let (st, body) = self
            .send("GET", token, &format!("/workspaces/{ws}/mcp/servers"), None)
            .await;
        assert_eq!(st, 200, "list {ws}: {body}");
        body.as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .cloned()
    }

    /// Replace the server's Enforced policy: alice keeps everything, plus
    /// `rules` (user, operations).
    async fn enforce(&self, id: &str, rules: &[(&str, &[&str])]) {
        let access = otto_state::ResourceAccessRepo::new(self.pool.clone());
        let current = access
            .get_policy(ResourceKind::McpServer, &id.to_string())
            .await
            .unwrap();
        let mut all = vec![(
            "alice",
            otto_core::access::operations_for(ResourceKind::McpServer),
        )];
        all.extend(rules.iter().map(|(u, ops)| (*u, *ops)));
        let rules = all
            .into_iter()
            .map(|(user, ops)| AccessRule {
                id: otto_core::new_id(),
                subject_kind: SubjectKind::User,
                subject_id: user.into(),
                effect: RuleEffect::Allow,
                operations: ops.iter().map(|o| o.to_string()).collect(),
                children: None,
                grantable_operations: vec![],
                credential_connection_id: None,
            })
            .collect();
        let policy = AccessPolicy {
            kind: ResourceKind::McpServer,
            resource_id: id.into(),
            mode: AccessMode::Enforced,
            revision: current.revision,
            rules,
        };
        let actor = AccessActor {
            real_user_id: "alice".into(),
            effective_user_id: None,
        };
        access
            .put_policy(&policy, current.revision, &actor)
            .await
            .unwrap();
    }
}

async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let opts = SqliteConnectOptions::new()
        .filename(tmp.path().join("otto.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let pool: DbPool = pool.into();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut ctx = otto_server::ServerCtx::for_tests(&pool, tmp.path()).await;
    ctx.base_url = base.clone();
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let d = Daemon {
        base,
        http: reqwest::Client::new(),
        pool,
        _tmp: tmp,
    };
    for ws in ["wa", "wb", "wc"] {
        d.workspace(ws).await;
    }
    d
}

struct People {
    alice: String,
    bob: String,
    carol: String,
    dave: String,
}

/// alice: root (the owner), admin everywhere. bob: editor of wb only. carol:
/// editor of wc only. dave: editor of the home wa (may configure, not root).
async fn people(d: &Daemon) -> People {
    let admin_all: &[(&str, &str)] = &[("wa", "admin"), ("wb", "admin"), ("wc", "admin")];
    People {
        alice: d.user("alice", true, admin_all).await,
        bob: d.user("bob", false, &[("wb", "editor")]).await,
        carol: d.user("carol", false, &[("wc", "editor")]).await,
        dave: d.user("dave", false, &[("wa", "editor")]).await,
    }
}

async fn create_shared(d: &Daemon, p: &People, body: Value) -> String {
    let mut req = json!({"name": "gh", "command": "true", "env": {"PLAIN": "v"}, "enabled": true});
    req.as_object_mut()
        .unwrap()
        .extend(body.as_object().unwrap().clone());
    let (st, s) = d
        .send("POST", &p.alice, "/workspaces/wa/mcp/servers", Some(req))
        .await;
    assert_eq!(st, 200, "create: {s}");
    s["id"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shared_server_reaches_its_workspaces_members_and_no_one_else() {
    let d = boot().await;
    let p = people(&d).await;
    let id = create_shared(&d, &p, json!({"shared_workspace_ids": ["wb"]})).await;
    // Even a policy that names carol (not in wa or wb) never shows it to her.
    d.enforce(
        &id,
        &[
            ("bob", &["discover", "invoke"]),
            ("carol", &["discover", "invoke"]),
            ("dave", &["discover", "configure"]),
        ],
    )
    .await;

    let own = d.listed(&p.alice, "wa", &id).await.expect("home lists it");
    assert_eq!(own["shared_workspace_ids"], json!(["wb"]));
    assert_eq!(own["all_workspaces"], json!(false));

    // bob (member of wb): uses it, never sees its configuration.
    let seen = d.listed(&p.bob, "wb", &id).await.expect("shared into wb");
    assert_eq!(seen["workspace_id"], "wa", "the home stays visible");
    assert_eq!(seen["command"], "", "{seen}");
    assert_eq!(seen["env"], json!({}), "{seen}");
    assert_eq!(seen["shared_workspace_ids"], json!([]), "{seen}");
    let (st, body) = d
        .send("GET", &p.bob, &format!("/mcp/servers/{id}"), None)
        .await;
    assert_eq!(st, 200, "{body}");

    // carol (member of neither): cannot see it anywhere.
    assert!(d.listed(&p.carol, "wc", &id).await.is_none());
    let (st, _) = d
        .send("GET", &p.carol, &format!("/mcp/servers/{id}"), None)
        .await;
    assert!(st == 403 || st == 404, "non-member read: {st}");

    // Settings stay with the home: bob may not edit it from wb; dave (home
    // editor allowed to configure) may, but not change where it's available.
    let path = format!("/mcp/servers/{id}");
    let (st, _) = d
        .send("PATCH", &p.bob, &path, Some(json!({"description": "b"})))
        .await;
    assert_eq!(st, 403);
    let (st, _) = d
        .send(
            "PATCH",
            &p.bob,
            &path,
            Some(json!({"all_workspaces": true})),
        )
        .await;
    assert_eq!(st, 403);
    let (st, body) = d
        .send(
            "PATCH",
            &p.dave,
            &path,
            Some(json!({"all_workspaces": true})),
        )
        .await;
    assert_eq!(st, 403, "{body}");
    assert!(body.to_string().contains("only the owner"), "{body}");
    let (st, body) = d
        .send(
            "PATCH",
            &p.dave,
            &path,
            Some(json!({"shared_workspace_ids": []})),
        )
        .await;
    assert_eq!(st, 403, "{body}");
    let (st, body) = d
        .send("PATCH", &p.dave, &path, Some(json!({"description": "d"})))
        .await;
    assert_eq!(st, 200, "{body}");
    assert!(
        !d.listed(&p.alice, "wa", &id).await.unwrap()["all_workspaces"]
            .as_bool()
            .unwrap()
    );

    // An Enforced policy still decides: without his rule bob no longer sees it.
    d.enforce(&id, &[("carol", &["discover", "invoke"])]).await;
    assert!(d.listed(&p.bob, "wb", &id).await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_workspaces_includes_a_later_one_until_narrowed() {
    let d = boot().await;
    let p = people(&d).await;
    let id = create_shared(&d, &p, json!({"all_workspaces": true})).await;
    d.enforce(&id, &[("carol", &["discover", "invoke"])]).await;
    assert!(d.listed(&p.carol, "wc", &id).await.is_some());
    // A workspace created afterwards has it with no further step.
    d.workspace("wd").await;
    d.member("wd", "alice", "admin").await;
    assert!(d.listed(&p.alice, "wd", &id).await.is_some());
    otto_state::McpToolsRepo::new(d.pool.clone())
        .upsert_discovered(
            &id,
            &[otto_state::DiscoveredTool {
                name: "t".into(),
                title: None,
                description: None,
                input_schema: json!({"type": "object"}),
                annotations: json!({}),
                risk_label: "read".into(),
                injection_risk: "low".into(),
                mutating: false,
                supports_dry_run: false,
            }],
        )
        .await
        .unwrap();
    let gateway = |ws: &'static str| {
        let d = &d;
        let t = p.alice.clone();
        async move {
            let (st, body) = d
                .send(
                    "GET",
                    &t,
                    &format!("/mcp/gateway/tools?workspace_id={ws}"),
                    None,
                )
                .await;
            assert_eq!(st, 200, "{body}");
            body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "mcp__gh__t")
        }
    };
    assert!(gateway("wd").await, "a session in wd gets its tools");

    // Narrowed to "wa, wb": gone from wc and wd, sessions there included.
    let (st, body) = d
        .send(
            "PATCH",
            &p.alice,
            &format!("/mcp/servers/{id}"),
            Some(json!({"all_workspaces": false, "shared_workspace_ids": ["wb"]})),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["shared_workspace_ids"], json!(["wb"]));
    assert!(d.listed(&p.carol, "wc", &id).await.is_none());
    assert!(d.listed(&p.alice, "wd", &id).await.is_none());
    assert!(!gateway("wd").await);
    assert!(gateway("wb").await);
    let invoke =
        |ws: &str| json!({"server_id": id, "tool": "t", "workspace_id": ws, "dry_run": true});
    let (st, body) = d
        .send("POST", &p.alice, "/mcp/gateway/invoke", Some(invoke("wd")))
        .await;
    assert_eq!(st, 403, "{body}");
    assert!(body.to_string().contains("workspace mismatch"), "{body}");
    let (_, body) = d
        .send("POST", &p.alice, "/mcp/gateway/invoke", Some(invoke("wb")))
        .await;
    assert!(!body.to_string().contains("workspace mismatch"), "{body}");

    // An unknown workspace is refused and changes nothing.
    let (st, _) = d
        .send(
            "PATCH",
            &p.alice,
            &format!("/mcp/servers/{id}"),
            Some(json!({"shared_workspace_ids": ["nope"]})),
        )
        .await;
    assert_eq!(st, 400);
    assert!(d.listed(&p.alice, "wb", &id).await.is_some());
}

/// Under a Legacy policy workspace roles decide: an editor of a workspace it
/// is shared with uses it (still without its configuration); others don't.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_access_follows_the_shared_workspaces_roles() {
    let d = boot().await;
    let p = people(&d).await;
    let id = create_shared(&d, &p, json!({"shared_workspace_ids": ["wb"]})).await;
    let access = otto_state::ResourceAccessRepo::new(d.pool.clone());
    let mut legacy = access
        .get_policy(ResourceKind::McpServer, &id)
        .await
        .unwrap();
    let rev = legacy.revision;
    legacy.mode = AccessMode::Legacy;
    legacy.rules.clear();
    let actor = AccessActor {
        real_user_id: "alice".into(),
        effective_user_id: None,
    };
    access.put_policy(&legacy, rev, &actor).await.unwrap();
    let seen = d.listed(&p.bob, "wb", &id).await.expect("bob in wb");
    assert_eq!(seen["command"], "");
    let (st, _) = d
        .send("GET", &p.bob, &format!("/mcp/servers/{id}/tools"), None)
        .await;
    assert_eq!(st, 200);
    assert!(d.listed(&p.carol, "wc", &id).await.is_none());
    let (st, _) = d
        .send("GET", &p.carol, &format!("/mcp/servers/{id}/tools"), None)
        .await;
    assert_eq!(st, 403);
    // A home editor still sees the configuration.
    assert_eq!(
        d.listed(&p.dave, "wa", &id).await.unwrap()["command"],
        "true"
    );
}
