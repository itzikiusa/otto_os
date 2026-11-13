//! "Act as me in Slack" through the REAL daemon router (auth middleware +
//! feature guard + route classes + the governed invoke): who may manage a
//! person's Slack user tokens, which credentials may act in which of their
//! Slack teams, and the personal-agent opt-in. Nothing here reaches Slack —
//! every call stops at a gate, or answers from the Keychain index.
//!
//! Harness trimmed from `personal_agent_policy.rs`.

use chrono::Utc;
use otto_rbac::AuthRepo;
use otto_server::ServerCtx;
use otto_state::{DbPool, NewPersonalAgent, NewSession, PersonalAgentsRepo, SessionsRepo};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

async fn file_pool(dir: &std::path::Path) -> DbPool {
    let opts = SqliteConnectOptions::new()
        .filename(dir.join("otto.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .expect("connect sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

struct Daemon {
    base: String,
    human: String,
    http: reqwest::Client,
    pool: DbPool,
    ctx: ServerCtx,
    tmp: tempfile::TempDir,
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

    /// An agent session of alice in ws1 with `meta`, and its managed token.
    async fn agent_session(&self, meta: Value) -> String {
        let s = SessionsRepo::new(self.pool.clone())
            .create(NewSession {
                workspace_id: "ws1".into(),
                kind: otto_core::domain::SessionKind::Agent,
                provider: "claude".into(),
                title: "Agent".into(),
                cwd: self.tmp.path().to_string_lossy().to_string(),
                provider_session_id: None,
                connection_id: None,
                created_by: "alice".into(),
                meta,
            })
            .await
            .unwrap();
        AuthRepo::new(self.pool.clone())
            .issue_session_api_token(&"alice".to_string(), &s.id)
            .await
            .unwrap()
            .0
    }

    async fn invoke(&self, token: &str, tool: &str, args: Value) -> Value {
        let (st, body) = self
            .send(
                "POST",
                token,
                "/mcp/otto-tools/invoke",
                Some(json!({"tool": format!("otto.{tool}"), "arguments": args})),
            )
            .await;
        assert_eq!(st, 200, "invoke {tool}: {body}");
        body
    }

    async fn teams(&self, token: &str) -> Value {
        let (st, body) = self
            .send(
                "GET",
                token,
                "/workspaces/ws1/integrations/slack/teams",
                None,
            )
            .await;
        assert_eq!(st, 200, "{body}");
        body
    }
}

async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let pool = file_pool(tmp.path()).await;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES ('alice', 'alice', 'x', 'alice', 1, ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES ('ws1', 'ws', '/tmp', '{}', 0, ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1', 'alice', 'admin')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (human, _) = AuthRepo::new(pool.clone())
        .issue_api_token(&"alice".to_string(), Some("ui"))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
    ctx.base_url = base.clone();
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let d = Daemon {
        base,
        human,
        http: reqwest::Client::new(),
        pool,
        ctx,
        tmp,
    };
    // Two connected Slack teams for alice, as `POST /integrations/slack/me`
    // leaves them (that route needs a live `auth.test`).
    let index = json!([
        {"team_id": "T1", "team": "Acme", "url": "https://acme.slack.com/", "user_id": "U1", "user": "alice", "added_at": ""},
        {"team_id": "T2", "team": "Globex", "url": "https://globex.slack.com/", "user_id": "U7", "user": "alice", "added_at": ""}
    ]);
    d.ctx
        .secrets
        .put("slack-user-alice", &index.to_string())
        .unwrap();
    for t in ["T1", "T2"] {
        d.ctx
            .secrets
            .put(&format!("slack-user-alice-{t}"), &format!("xoxp-{t}"))
            .unwrap();
    }
    let (st, body) = d
        .send(
            "PATCH",
            &d.human.clone(),
            "/mcp/otto-server",
            Some(json!({"enabled": true, "tools": ["slack_my_workspaces", "slack_post_message"]})),
        )
        .await;
    assert_eq!(st, 200, "enable tools: {body}");
    d
}

fn team_ids(v: &Value) -> Vec<String> {
    v["teams"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t["team_id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The person manages their tokens; no agent credential can, and nothing
/// returned carries a token.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_person_manages_their_slack_tokens() {
    let d = boot().await;
    let (st, list) = d
        .send("GET", &d.human, "/integrations/slack/me", None)
        .await;
    assert_eq!(st, 200, "{list}");
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert!(!list.to_string().contains("xoxp"), "{list}");
    assert!(list[0]["missing_scopes"].is_array());

    let agent = d.agent_session(json!({})).await;
    for (m, path, body) in [
        (
            "POST",
            "/integrations/slack/me",
            Some(json!({"token": "xoxp-evil"})),
        ),
        (
            "PUT",
            "/integrations/slack/me/T1",
            Some(json!({"token": "xoxp-evil"})),
        ),
        ("DELETE", "/integrations/slack/me/T1", None),
        ("POST", "/integrations/slack/me/T1/test", None),
    ] {
        let (st, body) = d.send(m, &agent, path, body).await;
        assert_eq!(st, 403, "{m} {path} by an agent: {body}");
    }
    // A bot token is refused before Slack is asked.
    let (st, _) = d
        .send(
            "POST",
            &d.human,
            "/integrations/slack/me",
            Some(json!({"token": "xoxb-1"})),
        )
        .await;
    assert_eq!(st, 400);
    let (st, _) = d
        .send("DELETE", &d.human, "/integrations/slack/me/T2", None)
        .await;
    assert_eq!(st, 204);
    assert!(d.ctx.secrets.get("slack-user-alice-T2").unwrap().is_none());
}

/// A person and a session they run act in every team; a personal agent in
/// none until its owner allows specific teams — set only by the owner, from
/// a person's credential.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_personal_agent_acts_as_its_owner_only_where_allowed() {
    let d = boot().await;
    assert_eq!(team_ids(&d.teams(&d.human).await), ["T1", "T2"]);
    let terminal = d.agent_session(json!({})).await;
    assert_eq!(team_ids(&d.teams(&terminal).await), ["T1", "T2"]);

    let repo = PersonalAgentsRepo::new(d.pool.clone());
    let mut new = NewPersonalAgent::defaults("ws1".into(), "Digest".into());
    new.created_by = Some("alice".into());
    let agent = repo.create(new).await.unwrap().id;
    let agent_tok = d.agent_session(json!({"personal_agent": agent})).await;
    let t = d.teams(&agent_tok).await;
    assert_eq!(team_ids(&t), Vec::<String>::new());
    assert_eq!(t["as_me_restricted"], true);

    let grant = format!("/personal-agents/{agent}/slack-as-me");
    // The agent cannot grant itself.
    let (st, _) = d
        .send("PUT", &agent_tok, &grant, Some(json!({"teams": ["T1"]})))
        .await;
    assert_eq!(st, 403);
    // Only the owner's own connected teams.
    let (st, _) = d
        .send("PUT", &d.human, &grant, Some(json!({"teams": ["T9"]})))
        .await;
    assert_eq!(st, 400);
    let (st, body) = d
        .send(
            "PUT",
            &d.human,
            &grant,
            Some(json!({"teams": ["T2", "T2"]})),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["teams"], json!(["T2"]));
    assert_eq!(body["is_owner"], true);
    assert_eq!(body["available"].as_array().unwrap().len(), 2);
    assert_eq!(team_ids(&d.teams(&agent_tok).await), ["T2"]);

    // Through the governed tool, bound to the agent's own session.
    let env = d
        .invoke(
            &agent_tok,
            "slack_my_workspaces",
            json!({"workspace_id": "ws1"}),
        )
        .await;
    assert_eq!(env["executed"], true, "{env}");
    assert_eq!(team_ids(&env["content"]), ["T2"], "{env}");
}

/// A post as the person is approval-gated, and the card says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn posting_as_the_person_waits_for_an_approval() {
    let d = boot().await;
    let terminal = d.agent_session(json!({})).await;
    let env = d
        .invoke(
            &terminal,
            "slack_post_message",
            json!({"workspace_id": "ws1", "as": "user", "team": "Acme",
                   "channel": "#daily", "text": "Today: 3 PRs to review"}),
        )
        .await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    assert_eq!(env["executed"], false, "{env}");
    let (_, pending) = d
        .send("GET", &d.human, "/mcp/approvals?status=pending", None)
        .await;
    let card = pending.to_string();
    assert!(
        card.contains("AS YOU") && card.contains("Today: 3 PRs"),
        "{card}"
    );
}

/// An agent's own token never writes AS THE PERSON over REST — post, react
/// and mark are refused before Slack is asked, naming the governed tool. As
/// the bot it reaches the bot path as before (here: no bot token, 400).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_token_cannot_write_as_the_person_over_rest() {
    let d = boot().await;
    let terminal = d.agent_session(json!({})).await;
    let base = "/workspaces/ws1/integrations/slack";
    for (path, body, tool) in [
        (
            "messages",
            json!({"as": "user", "team": "T1", "channel": "C0123ABCD", "text": "hi"}),
            "otto.slack_post_message",
        ),
        (
            "messages",
            json!({"as": "me", "team": "T1", "channel": "C0123ABCD", "text": "hi"}),
            "otto.slack_post_message",
        ),
        (
            "reactions",
            json!({"as": "user", "team": "T1", "channel": "C0123ABCD",
                   "ts": "1730800000.000200", "name": "eyes"}),
            "otto.slack_react",
        ),
        (
            "mark",
            json!({"team": "T1", "channel": "C0123ABCD", "ts": "1730800000.000200"}),
            "otto.slack_mark_unread",
        ),
    ] {
        let (st, resp) = d
            .send(
                "POST",
                &terminal,
                &format!("{base}/{path}"),
                Some(body.clone()),
            )
            .await;
        assert_eq!(st, 403, "{path} {body}: {resp}");
        assert!(
            resp["message"].as_str().unwrap_or_default().contains(tool),
            "{resp}"
        );
    }
    let (st, resp) = d
        .send(
            "POST",
            &terminal,
            &format!("{base}/messages"),
            Some(json!({"channel": "C0123ABCD", "text": "hi"})),
        )
        .await;
    assert_eq!(st, 400, "as the bot: {resp}");
    assert!(
        resp["message"]
            .as_str()
            .unwrap_or_default()
            .contains("bot token"),
        "{resp}"
    );
}

/// The approved governed post replays AS THE PERSON (every team), so the
/// requesting personal agent's own grant is checked BEFORE the card is
/// filed: a team it was not allowed, or none named, is refused outright.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_governed_post_as_the_person_honours_the_agents_team_grant() {
    let d = boot().await;
    let repo = PersonalAgentsRepo::new(d.pool.clone());
    let mut new = NewPersonalAgent::defaults("ws1".into(), "Digest".into());
    new.created_by = Some("alice".into());
    let agent = repo.create(new).await.unwrap().id;
    let mut cfg = repo.autonomy(&agent).await.unwrap();
    cfg.slack_as_me_teams = vec!["T2".into()];
    repo.save_autonomy(&agent, &cfg).await.unwrap();
    let agent_tok = d.agent_session(json!({"personal_agent": agent})).await;
    let post = |team: Option<&str>| {
        let mut a = json!({"workspace_id": "ws1", "as": "user", "channel": "#daily",
                           "text": "Today: 3 PRs to review"});
        if let Some(t) = team {
            a["team"] = json!(t);
        }
        a
    };
    for team in [Some("Acme"), Some("T1"), None] {
        let env = d.invoke(&agent_tok, "slack_post_message", post(team)).await;
        assert_eq!(env["decision"], "denied", "{team:?}: {env}");
        assert_eq!(env["executed"], false, "{env}");
    }
    let (_, pending) = d
        .send("GET", &d.human, "/mcp/approvals?status=pending", None)
        .await;
    assert_eq!(pending.as_array().map(Vec::len), Some(0), "{pending}");

    let env = d
        .invoke(&agent_tok, "slack_post_message", post(Some("Globex")))
        .await;
    assert_eq!(env["decision"], "pending_approval", "{env}");
    let (_, pending) = d
        .send("GET", &d.human, "/mcp/approvals?status=pending", None)
        .await;
    let card = pending.to_string();
    assert!(
        card.contains("Globex") && card.contains("#daily") && card.contains("Today: 3 PRs"),
        "{card}"
    );
}
