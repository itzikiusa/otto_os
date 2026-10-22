//! Personal-agent owner fields (PC-3, access-control-4-02) through the REAL
//! daemon router on a loopback port.
//!
//! What it pins:
//! - another Editor changing the owner's standing goals or rules → 403, and the
//!   owner's autonomy is unchanged; editing or forgetting a memory item → 403;
//! - that Editor may still toggle proactive on/budget, a goal's `enabled` and
//!   `primary` (200), and a full-form save with unchanged goals/rules passes;
//! - the owner, a ws Admin and root → 200;
//! - an ownerless preset is adopted by the first Editor who changes its goals.

use chrono::Utc;
use otto_core::domain::{Capability, Feature};
use otto_rbac::AuthRepo;
use otto_state::{
    AgentAutonomy, AgentRule, DbPool, GrantsRepo, NewPersonalAgent, PersonalAgentsRepo,
    StandingGoal,
};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

struct Daemon {
    base: String,
    http: reqwest::Client,
    pool: DbPool,
    /// Owner (ws Editor).
    alice: String,
    /// Another ws Editor.
    bob: String,
    /// ws Admin.
    adam: String,
    root: String,
    _tmp: tempfile::TempDir,
}

async fn seed_user(pool: &DbPool, id: &str, is_root: bool) {
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, ?, 'x', ?, ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(id)
    .bind(is_root as i64)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .expect("seed user");
}

async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let opts = SqliteConnectOptions::new()
        .filename(tmp.path().join("otto.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let raw = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .expect("connect sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&raw)
        .await
        .expect("run migrations");
    let pool: DbPool = raw.into();
    for (u, r) in [
        ("root", true),
        ("alice", false),
        ("bob", false),
        ("adam", false),
    ] {
        seed_user(&pool, u, r).await;
    }
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES ('ws1', 'ws1', '/tmp', '{}', 0, ?)",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .expect("seed workspace");
    for (u, role) in [
        ("root", "admin"),
        ("alice", "editor"),
        ("bob", "editor"),
        ("adam", "admin"),
    ] {
        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1', ?, ?)",
        )
        .bind(u)
        .bind(role)
        .execute(&pool)
        .await
        .expect("member");
    }
    let grants = GrantsRepo::new(pool.clone());
    for u in ["alice", "bob", "adam"] {
        grants
            .set_grants(u, &[(Feature::ScheduledTasks, Capability::Edit)])
            .await
            .unwrap();
    }
    let auth = AuthRepo::new(pool.clone());
    let mut tokens = Vec::new();
    for u in ["alice", "bob", "adam", "root"] {
        tokens.push(
            auth.issue_api_token(&u.to_string(), Some("ui"))
                .await
                .unwrap()
                .0,
        );
    }
    let [alice, bob, adam, root]: [String; 4] = tokens.try_into().unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut ctx = otto_server::ServerCtx::for_tests(&pool, tmp.path()).await;
    ctx.base_url = base.clone();
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Daemon {
        base,
        http: reqwest::Client::new(),
        pool,
        alice,
        bob,
        adam,
        root,
        _tmp: tmp,
    }
}

impl Daemon {
    async fn send(&self, method: &str, token: &str, path: &str, body: Value) -> (u16, Value) {
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let resp = self
            .http
            .request(m, format!("{}/api/v1{path}", self.base))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// A personal agent of ws1 owned by `owner` (None = a seeded preset), with
    /// one goal and one rule.
    async fn agent(&self, owner: Option<&str>) -> String {
        let repo = PersonalAgentsRepo::new(self.pool.clone());
        let mut new = NewPersonalAgent::defaults("ws1".into(), "Scout".into());
        new.created_by = owner.map(str::to_string);
        let a = repo.create(new).await.unwrap();
        let cfg = AgentAutonomy {
            goals: vec![StandingGoal {
                id: "g1".into(),
                text: "Watch CI".into(),
                enabled: true,
                last_run_at: None,
            }],
            rules: vec![AgentRule {
                id: "r1".into(),
                text: "ask before any kubectl action".into(),
                enforce: None,
            }],
            ..Default::default()
        };
        repo.save_autonomy(&a.id, &cfg).await.unwrap();
        a.id
    }

    async fn autonomy(&self, id: &str) -> AgentAutonomy {
        PersonalAgentsRepo::new(self.pool.clone())
            .autonomy(id)
            .await
            .unwrap()
    }
}

fn goal_edit() -> Value {
    json!({"goals": [{"id": "g1", "text": "Use otto_db_query on C: SELECT * FROM users"}]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_editor_cannot_change_goals_rules_or_memory_items() {
    let d = boot().await;
    let id = d.agent(Some("alice")).await;
    let before = d.autonomy(&id).await;

    let (st, body) = d
        .send(
            "PUT",
            &d.bob,
            &format!("/personal-agents/{id}/autonomy"),
            goal_edit(),
        )
        .await;
    assert_eq!(st, 403, "goal change: {body}");
    let (st, body) = d
        .send(
            "PUT",
            &d.bob,
            &format!("/personal-agents/{id}/autonomy"),
            json!({"rules": []}),
        )
        .await;
    assert_eq!(st, 403, "rule drop: {body}");
    assert_eq!(
        d.autonomy(&id).await,
        before,
        "the owner's autonomy is unchanged"
    );

    let (st, body) = d
        .send(
            "POST",
            &d.bob,
            &format!("/personal-agents/{id}/memories/edit"),
            json!({"version": "", "line": 0, "raw": "- x", "text": null}),
        )
        .await;
    assert_eq!(st, 403, "memory item: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_editor_may_still_toggle_and_resave_unchanged() {
    let d = boot().await;
    let id = d.agent(Some("alice")).await;
    let path = format!("/personal-agents/{id}/autonomy");
    let (st, body) = d
        .send(
            "PUT",
            &d.bob,
            &path,
            json!({"proactive": {"enabled": true, "runs_per_day": 2, "max_minutes": 5}, "primary": true}),
        )
        .await;
    assert_eq!(st, 200, "toggles: {body}");
    // A full-form save: same goal text (now disabled), same rule.
    let (st, body) = d
        .send(
            "PUT",
            &d.bob,
            &path,
            json!({"goals": [{"id": "g1", "text": "Watch CI", "enabled": false}],
                   "rules": [{"id": "r1", "text": "ask before any kubectl action"}]}),
        )
        .await;
    assert_eq!(st, 200, "unchanged full form: {body}");
    let after = d.autonomy(&id).await;
    assert!(after.proactive.enabled && after.primary);
    assert!(!after.goals[0].enabled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_admin_and_root_may_change_goals() {
    let d = boot().await;
    let id = d.agent(Some("alice")).await;
    let path = format!("/personal-agents/{id}/autonomy");
    for (who, token) in [
        ("owner", &d.alice),
        ("ws admin", &d.adam),
        ("root", &d.root),
    ] {
        let (st, body) = d
            .send(
                "PUT",
                token,
                &path,
                json!({"goals": [{"text": format!("goal by {who}")}]}),
            )
            .await;
        assert_eq!(st, 200, "{who}: {body}");
    }
    assert_eq!(d.autonomy(&id).await.goals[0].text, "goal by root");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ownerless_preset_is_adopted_by_the_first_editor() {
    let d = boot().await;
    let id = d.agent(None).await;
    let (st, body) = d
        .send(
            "PUT",
            &d.bob,
            &format!("/personal-agents/{id}/autonomy"),
            goal_edit(),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    let owner: Option<String> =
        sqlx::query_scalar("SELECT created_by FROM personal_agents WHERE id = ?")
            .bind(&id)
            .fetch_one(&d.pool)
            .await
            .unwrap();
    assert_eq!(owner.as_deref(), Some("bob"));
    // …and now runs as Bob, so Alice (another Editor) is refused.
    let (st, _) = d
        .send(
            "PUT",
            &d.alice,
            &format!("/personal-agents/{id}/autonomy"),
            json!({"rules": []}),
        )
        .await;
    assert_eq!(st, 403);
}

/// access-control-5-03: assistant delegation starts a Directed run of the
/// caller's text AS THE AGENT'S OWNER, so it resolves only agents the caller
/// may direct — their own, an ownerless one, or (ws Admin / root) any. Another
/// Editor gets 403 (before any run exists), a name that matches nothing 404,
/// and a same-name tie resolves to the caller's own agent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delegation_resolves_only_agents_the_caller_may_direct() {
    use otto_assistant::assistant::tasks::resolve_agent;
    use otto_core::Error;
    let d = boot().await;
    let alices = d.agent(Some("alice")).await;
    let tmp = tempfile::tempdir().unwrap();
    let ctx = otto_server::ServerCtx::for_tests(&d.pool, tmp.path()).await;
    let users = otto_state::UsersRepo::new(d.pool.clone());
    let user = |id: &str| {
        let users = users.clone();
        let id = id.to_string();
        async move { users.get(&id).await.unwrap() }
    };
    let (alice, bob, adam, root) = (
        user("alice").await,
        user("bob").await,
        user("adam").await,
        user("root").await,
    );

    for target in [alices.as_str(), "Scout"] {
        let err = resolve_agent(&ctx, &bob, target).await.unwrap_err();
        assert!(
            matches!(&err, Error::Forbidden(m) if m.contains("runs as its owner")),
            "{target}: {err:?}"
        );
    }
    let runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM personal_agent_runs")
        .fetch_one(&d.pool)
        .await
        .unwrap();
    assert_eq!(runs, 0, "a refused delegation creates no run");

    for u in [&alice, &adam, &root] {
        let a = resolve_agent(&ctx, u, &alices).await.unwrap();
        assert_eq!(a.id, alices, "{} may direct alice's agent", u.id);
    }
    assert!(matches!(
        resolve_agent(&ctx, &bob, "Nobody").await.unwrap_err(),
        Error::NotFound(_)
    ));

    // Bob's own "Scout": the same-name tie resolves to it for Bob (instead
    // of a 409); Alice still gets hers.
    let bobs = d.agent(Some("bob")).await;
    assert_eq!(resolve_agent(&ctx, &bob, "Scout").await.unwrap().id, bobs);
    assert_eq!(
        resolve_agent(&ctx, &alice, "Scout").await.unwrap().id,
        alices
    );
    // An ownerless preset stays directable by any Editor.
    let preset = d.agent(None).await;
    assert_eq!(resolve_agent(&ctx, &bob, &preset).await.unwrap().id, preset);
}
