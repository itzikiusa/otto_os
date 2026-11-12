//! [`super::caller_scope`] against a real (in-memory) state DB: a personal
//! agent on the structured runtime (a Threads thread carrying
//! `meta.personal_agent`) gets exactly its owner's opt-in, never `All`.

use otto_core::agent_runtime::RuntimeProvider;
use otto_core::auth::AuthContext;
use otto_core::domain::User;
use otto_state::{
    AgentRuntimeRepo, DbPool, NewPersonalAgent, NewRuntimeSession, PersonalAgentsRepo, UsersRepo,
    WorkspacesRepo,
};
use serde_json::{json, Value};

use super::caller_scope;
use crate::slack_user_tokens::AsMe;

struct World {
    pool: DbPool,
    user: User,
    ws: String,
}

async fn world() -> World {
    let pool = otto_state::db::test_pool().await;
    let user = UsersRepo::new(pool.clone())
        .create("dana", "x", "Dana", false)
        .await
        .unwrap();
    let ws = WorkspacesRepo::new(pool.clone())
        .create("W", "/tmp", &user.id)
        .await
        .unwrap()
        .id;
    World { pool, user, ws }
}

/// A Threads thread of `w.user` with `meta` (no CLI is spawned: a row only).
async fn thread(w: &World, id: &str, meta: Option<Value>) {
    AgentRuntimeRepo::new(w.pool.clone())
        .create(NewRuntimeSession {
            id: id.into(),
            workspace_id: w.ws.clone(),
            provider: RuntimeProvider::Claude,
            title: "t".into(),
            cwd: "/tmp".into(),
            model: None,
            access_mode: Default::default(),
            created_by: w.user.id.clone(),
            parent_id: None,
            meta,
        })
        .await
        .unwrap();
}

/// `w.user`'s credential as bound to agent session `sid`.
fn bound(w: &World, sid: &str) -> AuthContext {
    let mut a = AuthContext::automation(&w.user, "test");
    a.mcp_session_id = Some(sid.into());
    a
}

#[tokio::test]
async fn a_personal_agent_thread_gets_only_its_owners_opt_in() {
    let w = world().await;
    let repo = PersonalAgentsRepo::new(w.pool.clone());
    let agent = repo
        .create(NewPersonalAgent {
            created_by: Some(w.user.id.clone()),
            ..NewPersonalAgent::defaults(w.ws.clone(), "Scout".into())
        })
        .await
        .unwrap();
    // A chat / run thread carries the agent in its server-owned meta.
    thread(
        &w,
        "thr-agent",
        Some(json!({"source": "personal_agent", "personal_agent": agent.id})),
    )
    .await;

    // Default: no team at all — never every team the person connected.
    let got = caller_scope(&w.pool, &bound(&w, "thr-agent"))
        .await
        .unwrap();
    assert!(matches!(&got, AsMe::Teams(t) if t.is_empty()), "{got:?}");

    // Exactly the teams its owner ticked.
    let mut cfg = repo.autonomy(&agent.id).await.unwrap();
    cfg.slack_as_me_teams = vec!["T2".into()];
    repo.save_autonomy(&agent.id, &cfg).await.unwrap();
    let got = caller_scope(&w.pool, &bound(&w, "thr-agent"))
        .await
        .unwrap();
    assert!(
        matches!(&got, AsMe::Teams(t) if t == &["T2".to_string()]),
        "{got:?}"
    );

    // The same through an Otto-issued API token's session binding.
    let mut managed = AuthContext::automation(&w.user, "test");
    managed.managed_session_id = Some("thr-agent".into());
    let got = caller_scope(&w.pool, &managed).await.unwrap();
    assert!(
        matches!(&got, AsMe::Teams(t) if t == &["T2".to_string()]),
        "{got:?}"
    );
}

#[tokio::test]
async fn a_plain_thread_acts_for_its_person_and_an_unknown_session_gets_nothing() {
    let w = world().await;
    // A Threads thread the person runs directly: no personal agent.
    thread(&w, "thr-plain", None).await;
    let got = caller_scope(&w.pool, &bound(&w, "thr-plain"))
        .await
        .unwrap();
    assert!(matches!(got, AsMe::All), "{got:?}");
    thread(&w, "thr-other-meta", Some(json!({"origin": "threads"}))).await;
    let got = caller_scope(&w.pool, &bound(&w, "thr-other-meta"))
        .await
        .unwrap();
    assert!(matches!(got, AsMe::All), "{got:?}");

    // A binding that resolves to no session row and no thread: nothing.
    let got = caller_scope(&w.pool, &bound(&w, "no-such-session"))
        .await
        .unwrap();
    assert!(matches!(&got, AsMe::Teams(t) if t.is_empty()), "{got:?}");

    // A person's own credential: every team.
    let person = AuthContext::automation(&w.user, "test");
    assert!(matches!(
        caller_scope(&w.pool, &person).await.unwrap(),
        AsMe::All
    ));
}
