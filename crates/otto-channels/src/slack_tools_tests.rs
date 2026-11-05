//! Route tests: the real channels router (with the Slack routes) served on
//! loopback, against the mock Slack of `slack_api_tests`, with an in-memory
//! secret store and an allow-all role checker.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use otto_core::auth::{AuthContext, AuthUser, BoxFuture, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::secrets::SecretStore;
use otto_state::{IntegrationsRepo, WorkspacesRepo};
use serde_json::{json, Value};

use crate::slack_api::tests::mock;
use crate::slack_user_tokens::AsMe;

#[derive(Default)]
struct MemSecrets(Mutex<HashMap<String, String>>);
impl SecretStore for MemSecrets {
    fn put(&self, k: &str, v: &str) -> otto_core::Result<()> {
        self.0.lock().unwrap().insert(k.into(), v.into());
        Ok(())
    }
    fn get(&self, k: &str) -> otto_core::Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(k).cloned())
    }
    fn delete(&self, k: &str) -> otto_core::Result<()> {
        self.0.lock().unwrap().remove(k);
        Ok(())
    }
}

struct AllowAll;
impl RoleChecker for AllowAll {
    fn check<'a>(
        &'a self,
        _u: &'a User,
        _w: &'a otto_core::Id,
        _m: WorkspaceRole,
    ) -> BoxFuture<'a, otto_core::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
struct Ctx {
    integrations: IntegrationsRepo,
    workspaces: WorkspacesRepo,
    secrets: Arc<dyn SecretStore>,
    roles: Arc<dyn RoleChecker>,
    base: String,
    as_me: AsMe,
}

impl crate::http::ChannelsCtx for Ctx {
    fn integrations(&self) -> &IntegrationsRepo {
        &self.integrations
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
    fn slack_as_me<'a>(&'a self, _a: &'a AuthContext) -> BoxFuture<'a, otto_core::Result<AsMe>> {
        let v = self.as_me.clone();
        Box::pin(async move { Ok(v) })
    }
    fn slack_api_base(&self) -> String {
        self.base.clone()
    }
}

fn user(id: &str) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    }
}

/// The router served as Otto user `uid`; returns its `/api/v1` base and the
/// shared secret store.
async fn serve(slack_base: &str, uid: &str, as_me: AsMe, secrets: Arc<dyn SecretStore>) -> String {
    let dir = std::env::temp_dir().join(format!("otto-slack-tools-{}", otto_core::new_id()));
    let pool = otto_state::open(&dir.join("otto.db")).await.unwrap();
    let ctx = Ctx {
        integrations: IntegrationsRepo::new(pool.clone()),
        workspaces: WorkspacesRepo::new(pool),
        secrets,
        roles: Arc::new(AllowAll),
        base: slack_base.to_string(),
        as_me,
    };
    let u = user(uid);
    let app = crate::http::router::<Ctx>()
        .with_state(ctx)
        .layer(axum::Extension(AuthContext::automation(&u, "test")))
        .layer(axum::Extension(AuthUser(u)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

fn auth_test(team_id: &str, team: &str) -> Value {
    json!({"ok": true, "user_id": "U1", "user": "dana", "team": team,
        "team_id": team_id, "url": format!("https://{}.slack.com/", team.to_lowercase())})
}

async fn send(method: reqwest::Method, url: String, body: Option<Value>) -> (u16, Value) {
    let c = reqwest::Client::new();
    let mut r = c.request(method, url);
    if let Some(b) = body {
        r = r.json(&b);
    }
    let resp = r.send().await.unwrap();
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_person_connects_teams_and_a_duplicate_team_is_refused() {
    let (slack, _) = mock::serve(vec![("auth.test", auth_test("T1", "Acme"))], "search:read").await;
    let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
    let api = serve(&slack, "u1", AsMe::All, secrets.clone()).await;
    let me = format!("{api}/integrations/slack/me");

    let (st, body) = send(
        reqwest::Method::POST,
        me.clone(),
        Some(json!({"token": "xoxb-bot"})),
    )
    .await;
    assert_eq!(st, 400, "{body}");
    let (st, body) = send(
        reqwest::Method::POST,
        me.clone(),
        Some(json!({"token": "xoxp-1"})),
    )
    .await;
    assert_eq!(st, 201, "{body}");
    assert_eq!(body["team_id"], "T1");
    assert!(body["missing_scopes"]
        .as_array()
        .unwrap()
        .contains(&json!("im:history")));
    assert!(body.get("token").is_none());
    // Stored per Otto user + team; the index holds no token.
    assert_eq!(
        secrets.get("slack-user-u1-T1").unwrap().as_deref(),
        Some("xoxp-1")
    );
    assert!(!secrets
        .get("slack-user-u1")
        .unwrap()
        .unwrap()
        .contains("xoxp"));

    let (st, _) = send(
        reqwest::Method::POST,
        me.clone(),
        Some(json!({"token": "xoxp-2"})),
    )
    .await;
    assert_eq!(st, 409);
    let (st, _) = send(
        reqwest::Method::PUT,
        format!("{me}/T1"),
        Some(json!({"token": "xoxp-2"})),
    )
    .await;
    assert_eq!(st, 200);
    assert_eq!(
        secrets.get("slack-user-u1-T1").unwrap().as_deref(),
        Some("xoxp-2")
    );
    // A replace with another team's token is refused.
    let (st, _) = send(
        reqwest::Method::PUT,
        format!("{me}/T9"),
        Some(json!({"token": "xoxp-3"})),
    )
    .await;
    assert_eq!(st, 400);
    let (st, body) = send(reqwest::Method::POST, format!("{me}/T1/test"), None).await;
    assert_eq!((st, body["ok"].clone()), (200, json!(true)), "{body}");
    let (_, list) = send(reqwest::Method::GET, me.clone(), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (st, _) = send(reqwest::Method::DELETE, format!("{me}/T1"), None).await;
    assert_eq!(st, 204);
    assert!(secrets.get("slack-user-u1-T1").unwrap().is_none());
}

#[tokio::test]
async fn as_me_posts_with_the_callers_own_team_token_and_marks_unread() {
    let (slack, calls) = mock::serve(
        vec![
            ("users.list", json!({"ok": true, "members": []})),
            (
                "chat.postMessage",
                json!({"ok": true, "channel": "C9", "ts": "1730800000.000200"}),
            ),
            (
                "chat.getPermalink",
                json!({"ok": true, "permalink": "https://p"}),
            ),
            ("conversations.mark", json!({"ok": true})),
        ],
        "",
    )
    .await;
    let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
    for t in ["T1", "T2"] {
        secrets
            .put(&format!("slack-user-u1-{t}"), &format!("xoxp-{t}"))
            .unwrap();
    }
    let index = json!([
        {"team_id": "T1", "team": "Acme", "url": "https://acme.slack.com/", "user_id": "U1", "user": "dana", "added_at": ""},
        {"team_id": "T2", "team": "Globex", "url": "https://globex.slack.com/", "user_id": "U5", "user": "dana", "added_at": ""}
    ]);
    secrets.put("slack-user-u1", &index.to_string()).unwrap();
    let api = serve(&slack, "u1", AsMe::All, secrets.clone()).await;
    let url = format!("{api}/workspaces/ws1/integrations/slack/messages");

    // Two teams connected: `team` is required, and the error lists them.
    let (st, body) = send(
        reqwest::Method::POST,
        url.clone(),
        Some(json!({"as": "user", "channel": "C0123ABCD", "text": "hi"})),
    )
    .await;
    assert_eq!(st, 400);
    assert!(
        body["message"].as_str().unwrap().contains("Globex (T2)"),
        "{body}"
    );

    let (st, body) = send(
        reqwest::Method::POST,
        url,
        Some(json!({"as": "user", "team": "globex", "channel": "C0123ABCD", "text": "Daily summary"})),
    )
    .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(
        (body["as"].clone(), body["team_id"].clone()),
        (json!("user"), json!("T2"))
    );
    let bearer = calls
        .lock()
        .unwrap()
        .iter()
        .find(|c| c.0 == "chat.postMessage")
        .map(|c| c.2.clone())
        .unwrap();
    assert_eq!(bearer, "xoxp-T2");

    let (st, body) = send(
        reqwest::Method::POST,
        format!("{api}/workspaces/ws1/integrations/slack/mark"),
        Some(json!({"team": "T2", "channel": "C0123ABCD", "ts": "1730800000.000200"})),
    )
    .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["cursor"], "1730800000.000199");
    assert_eq!(
        mock::params(&calls, "conversations.mark")["ts"],
        "1730800000.000199"
    );
}

#[tokio::test]
async fn a_personal_agent_without_the_opt_in_cannot_act_as_the_person() {
    let (slack, calls) = mock::serve(vec![], "").await;
    let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
    secrets.put("slack-user-u1-T1", "xoxp-T1").unwrap();
    secrets
        .put(
            "slack-user-u1",
            &json!([{"team_id": "T1", "team": "Acme", "url": "", "user_id": "U1", "user": "d", "added_at": ""}])
                .to_string(),
        )
        .unwrap();
    let api = serve(&slack, "u1", AsMe::Teams(vec![]), secrets).await;
    let (st, body) = send(
        reqwest::Method::GET,
        format!("{api}/workspaces/ws1/integrations/slack/search?query=deploy"),
        None,
    )
    .await;
    assert_eq!(st, 403, "{body}");
    assert!(body["message"].as_str().unwrap().contains("Can act as me"));
    // Its team listing is empty too, and Slack was never called.
    let (_, teams) = send(
        reqwest::Method::GET,
        format!("{api}/workspaces/ws1/integrations/slack/teams"),
        None,
    )
    .await;
    assert_eq!(teams["teams"], json!([]));
    assert_eq!(teams["as_me_restricted"], json!(true));
    assert!(mock::methods(&calls).is_empty());
}

#[tokio::test]
async fn bot_reads_need_the_workspace_bot_token_and_validate_input() {
    let (slack, calls) = mock::serve(
        vec![
            ("users.list", json!({"ok": true, "members": []})),
            (
                "conversations.history",
                json!({"ok": true, "messages": [{"ts": "1.000000", "text": "x"}]}),
            ),
        ],
        "",
    )
    .await;
    let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
    let api = serve(&slack, "u1", AsMe::All, secrets.clone()).await;
    let read = format!("{api}/workspaces/ws1/integrations/slack/messages?channel=C0123ABCD");
    let (st, body) = send(reqwest::Method::GET, read.clone(), None).await;
    assert_eq!(st, 400);
    assert!(body["message"].as_str().unwrap().contains("bot token"));
    secrets.put("chan-bot-ws1-slack", "xoxb-ws1").unwrap();
    let (st, body) = send(reqwest::Method::GET, format!("{read}&oldest=2h"), None).await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["as"], "bot");
    assert_eq!(body["messages"][0]["ts"], "1.000000");
    assert!(mock::params(&calls, "conversations.history")["oldest"].ends_with(".000000"));
    let (st, _) = send(reqwest::Method::GET, format!("{read}&thread_ts=abc"), None).await;
    assert_eq!(st, 400);
    let (st, _) = send(reqwest::Method::GET, format!("{read}&as=admin"), None).await;
    assert_eq!(st, 400);
}

#[test]
fn the_digest_keeps_only_recent_hits_and_flags_my_own() {
    use crate::slack_api::Hit;
    let hit = |ts: &str, user: &str| Hit {
        channel_id: "D1".into(),
        channel_name: "Grok".into(),
        kind: "im",
        ts: ts.into(),
        time: String::new(),
        user: Some(user.into()),
        user_name: None,
        text: String::new(),
        permalink: None,
        from_me: false,
    };
    let (m, d) = super::build_digest(
        "100.000000",
        "U1",
        vec![hit("50.000000", "U2"), hit("150.000000", "U2")],
        vec![
            hit("120.000000", "U1"),
            hit("130.000000", "U2"),
            hit("130.000000", "U2"),
        ],
        10,
    );
    assert_eq!(m.len(), 1);
    assert_eq!(d.len(), 2, "deduped");
    assert_eq!(d[0].ts, "130.000000", "newest first");
    assert!(d[1].from_me && !d[0].from_me);
}
