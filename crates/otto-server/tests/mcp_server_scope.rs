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
    assert!(body.to_string().contains("not available"), "{body}");
    let (_, body) = d
        .send("POST", &p.alice, "/mcp/gateway/invoke", Some(invoke("wb")))
        .await;
    assert!(!body.to_string().contains("not available"), "{body}");

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

// ---------------------------------------------------------------------------
// Review round 2: each item failed before its fix.
// ---------------------------------------------------------------------------

async fn discovered(d: &Daemon, id: &str) {
    otto_state::McpToolsRepo::new(d.pool.clone())
        .upsert_discovered(
            &id.to_string(),
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
}

/// M-001: Scratch (every user's implicit Editor workspace) is not part of
/// "all": a user with no real membership — or a viewer — can't reach an
/// all-workspaces server through it, and it can't be picked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scratch_never_reaches_a_shared_server() {
    let d = boot().await;
    let p = people(&d).await;
    let zed = d.user("zed", false, &[]).await;
    let id = create_shared(&d, &p, json!({"all_workspaces": true})).await;
    d.enforce(&id, &[("zed", &["discover", "invoke", "approve"])])
        .await;
    discovered(&d, &id).await;
    let (st, _) = d
        .send("GET", &zed, &format!("/mcp/servers/{id}"), None)
        .await;
    assert!(
        st == 403 || st == 404,
        "zero-workspace user reached it: {st}"
    );
    let (st, body) = d
        .send("GET", &zed, "/workspaces/scratch/mcp/servers", None)
        .await;
    assert!(st != 200 || !body.to_string().contains(&id), "{st} {body}");
    let call = json!({"server_id": id, "tool": "t", "workspace_id": "scratch", "dry_run": true});
    let (st, body) = d
        .send("POST", &p.alice, "/mcp/gateway/invoke", Some(call))
        .await;
    assert_eq!(st, 403, "{body}");
    let (st, _) = d
        .send(
            "PATCH",
            &p.alice,
            &format!("/mcp/servers/{id}"),
            Some(json!({"all_workspaces": false, "shared_workspace_ids": ["scratch"]})),
        )
        .await;
    assert_eq!(st, 400);
}

/// M-002: a server-bound approval raised in wa can't be decided by an editor
/// of a workspace the server is merely shared with.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_shared_workspace_member_cannot_decide_the_homes_approvals() {
    let d = boot().await;
    let p = people(&d).await;
    let id = create_shared(&d, &p, json!({"shared_workspace_ids": ["wb"]})).await;
    d.enforce(&id, &[("bob", &["discover", "invoke", "approve"])])
        .await;
    let card = otto_state::McpApprovalRepo::new(d.pool.clone())
        .create(otto_state::NewApproval {
            workspace_id: Some("wa".into()),
            kind: "tool_call".into(),
            server_id: Some(id.clone()),
            server_name: Some("gh".into()),
            tool: Some("t".into()),
            title: "Call t".into(),
            detail: None,
            args_redacted_json: "{}".into(),
            args_hash: None,
            risk_label: None,
            requested_by: Some("alice".into()),
            requested_by_kind: Some("user".into()),
            requested_by_session_id: None,
            expires_at: None,
        })
        .await
        .unwrap();
    let path = format!("/mcp/approvals/{}/decide", card.id);
    let (st, body) = d
        .send("POST", &p.bob, &path, Some(json!({"approved": true})))
        .await;
    assert_eq!(st, 403, "{body}");
    let still = otto_state::McpApprovalRepo::new(d.pool.clone())
        .get(&card.id)
        .await
        .unwrap();
    assert_eq!(still.status, "pending");
}

/// M-003 / M-006: the UI test-invoke runs in the workspace the caller names
/// (its allowlist applies), which must reach the server and hold the caller;
/// it defaults to the home, where a shared-only member has no role. A call
/// for a workspace the server no longer reaches is refused by the governed
/// pipeline itself (the gateway's post-approval retry runs the same call).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_runs_in_its_workspace_and_only_where_the_server_reaches() {
    let d = boot().await;
    let p = people(&d).await;
    let id = create_shared(&d, &p, json!({"shared_workspace_ids": ["wb"]})).await;
    d.enforce(
        &id,
        &[
            ("bob", &["discover", "invoke"]),
            ("carol", &["discover", "invoke"]),
        ],
    )
    .await;
    discovered(&d, &id).await;
    let deny = json!({"entries": [{"server_id": id, "tool_name": "t", "mode": "deny"}]});
    let (st, body) = d
        .send("PUT", &p.bob, "/workspaces/wb/mcp/allowlist", Some(deny))
        .await;
    assert_eq!(st, 204, "{body}");
    let path = format!("/mcp/servers/{id}/tools/t/invoke");
    let (st, body) = d
        .send(
            "POST",
            &p.bob,
            &path,
            Some(json!({"workspace_id": "wb", "dry_run": true})),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["decision"], "denied", "wb's allowlist applies: {body}");
    let (st, body) = d
        .send("POST", &p.bob, &path, Some(json!({"dry_run": true})))
        .await;
    assert_eq!(st, 403, "bob has no role in the home: {body}");
    let (st, body) = d
        .send(
            "POST",
            &p.carol,
            &path,
            Some(json!({"workspace_id": "wc", "dry_run": true})),
        )
        .await;
    assert!(st == 403 || st == 404, "{st} {body}");
    // Narrowed while a call for wb was pending: the next call there is refused.
    let (st, _) = d
        .send(
            "PATCH",
            &p.alice,
            &format!("/mcp/servers/{id}"),
            Some(json!({"shared_workspace_ids": []})),
        )
        .await;
    assert_eq!(st, 200);
    let call = json!({"server_id": id, "tool": "t", "workspace_id": "wb", "dry_run": true});
    let (st, body) = d
        .send("POST", &p.alice, "/mcp/gateway/invoke", Some(call))
        .await;
    assert_eq!(st, 403, "{body}");
    assert!(body.to_string().contains("not available"), "{body}");
}

/// M-007 / M-013: the policy preview needs discover on the server and a role
/// in the workspace it previews; the allowlist needs discover; a shared-only
/// member gets no OAuth host or error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previews_allowlists_and_oauth_status_follow_the_same_rules() {
    let d = boot().await;
    let p = people(&d).await;
    let id = create_shared(&d, &p, json!({"shared_workspace_ids": ["wb"]})).await;
    d.enforce(&id, &[("bob", &["discover", "invoke"])]).await;
    let eval = |ws: &str| json!({"server_id": id, "tool": "t", "workspace_id": ws});
    let (st, body) = d
        .send("POST", &p.bob, "/mcp/policies/evaluate", Some(eval("wb")))
        .await;
    assert_eq!(st, 200, "{body}");
    let (st, _) = d
        .send("POST", &p.bob, "/mcp/policies/evaluate", Some(eval("wa")))
        .await;
    assert_eq!(st, 403, "another workspace's outcome");
    let (st, body) = d
        .send("GET", &p.bob, &format!("/mcp/servers/{id}/oauth"), None)
        .await;
    assert_eq!(st, 200, "{body}");
    assert!(
        body.get("auth_host").is_none() && body.get("error").is_none(),
        "{body}"
    );
    // Without discover, neither the preview nor the allowlist sees it.
    d.enforce(&id, &[]).await;
    let (st, _) = d
        .send("POST", &p.bob, "/mcp/policies/evaluate", Some(eval("wb")))
        .await;
    assert!(st == 403 || st == 404, "{st}");
    let allow = json!({"entries": [{"server_id": id, "mode": "allow"}]});
    let (st, _) = d
        .send("PUT", &p.bob, "/workspaces/wb/mcp/allowlist", Some(allow))
        .await;
    assert_eq!(st, 400);
}

/// M-008: the access pages answer a shared-workspace member for using the
/// server (discover / invoke), never for configuring it or its access.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn access_capabilities_match_what_a_shared_member_may_do() {
    let d = boot().await;
    let p = people(&d).await;
    let id = create_shared(&d, &p, json!({"shared_workspace_ids": ["wb"]})).await;
    d.enforce(
        &id,
        &[
            ("bob", &["discover", "invoke", "configure"]),
            ("carol", &["discover", "invoke"]),
        ],
    )
    .await;
    let (st, caps) = d
        .send(
            "GET",
            &p.bob,
            &format!("/access/mcp_server/{id}/capabilities"),
            None,
        )
        .await;
    assert_eq!(st, 200, "{caps}");
    assert_eq!(caps["operations"]["invoke"]["allowed"], true, "{caps}");
    assert_eq!(
        caps["operations"]["configure"]["allowed"], false,
        "settings stay home: {caps}"
    );
    let (st, _) = d
        .send("GET", &p.bob, &format!("/access/mcp_server/{id}"), None)
        .await;
    assert!(st == 403 || st == 404, "manage access is home-only: {st}");
    let (st, _) = d
        .send(
            "GET",
            &p.carol,
            &format!("/access/mcp_server/{id}/capabilities"),
            None,
        )
        .await;
    assert_eq!(st, 404);
}

/// M-011 + decision 1: a PATCH validates everything before it writes;
/// "all" clears the chosen links; a direct (native) server is never shared.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scope_patch_is_all_or_nothing_and_never_shares_a_native_server() {
    let d = boot().await;
    let p = people(&d).await;
    let id = create_shared(&d, &p, json!({"shared_workspace_ids": ["wb"]})).await;
    let path = format!("/mcp/servers/{id}");
    let (st, _) = d
        .send(
            "PATCH",
            &p.alice,
            &path,
            Some(json!({"description": "x", "shared_workspace_ids": ["nope"]})),
        )
        .await;
    assert_eq!(st, 400);
    let own = d.listed(&p.alice, "wa", &id).await.unwrap();
    assert!(own["description"].is_null(), "nothing written: {own}");
    let other = json!({"name": "other", "command": "true"});
    assert_eq!(
        d.send("POST", &p.alice, "/workspaces/wa/mcp/servers", Some(other))
            .await
            .0,
        200
    );
    let (st, _) = d
        .send(
            "PATCH",
            &p.alice,
            &path,
            Some(json!({"name": "other", "all_workspaces": true})),
        )
        .await;
    assert_eq!(st, 409);
    assert_eq!(
        d.listed(&p.alice, "wa", &id).await.unwrap()["all_workspaces"],
        false
    );
    let (st, body) = d
        .send(
            "PATCH",
            &p.alice,
            &path,
            Some(json!({"all_workspaces": true})),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["shared_workspace_ids"], json!([]));
    let (st, body) = d
        .send(
            "PATCH",
            &p.alice,
            &path,
            Some(json!({"all_workspaces": false})),
        )
        .await;
    assert_eq!(st, 200);
    assert_eq!(
        body["shared_workspace_ids"],
        json!([]),
        "old links are not revived"
    );
    // Native: can't be shared, and a shared server can't go native.
    let (st, _) = d
        .send(
            "PATCH",
            &p.alice,
            &path,
            Some(json!({"all_workspaces": true})),
        )
        .await;
    assert_eq!(st, 200);
    let (st, body) = d
        .send("PATCH", &p.alice, &path, Some(json!({"runs": "native"})))
        .await;
    assert_eq!(st, 400, "{body}");
    let (st, _) = d
        .send(
            "PATCH",
            &p.alice,
            &path,
            Some(json!({"all_workspaces": false})),
        )
        .await;
    assert_eq!(st, 200);
    let (st, body) = d
        .send("PATCH", &p.alice, &path, Some(json!({"runs": "native"})))
        .await;
    assert_eq!(st, 200, "{body}");
    let (st, body) = d
        .send(
            "PATCH",
            &p.alice,
            &path,
            Some(json!({"shared_workspace_ids": ["wb"]})),
        )
        .await;
    assert_eq!(st, 400, "{body}");
    assert!(body.to_string().contains("run it through Otto"), "{body}");
}
