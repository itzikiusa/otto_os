//! The database-change review routes enforce each step's authority over HTTP
//! (test-quality-6-02). The repo only knows approver ≠ author; who may
//! approve, reject, execute or read an old revision is decided by the route
//! handlers in `routes/database_changes.rs`, so these drive them end to end:
//! three users on one Prod connection policy — `author` (submit), `reviewer`
//! (submit + approve) and `peer` (submit) — with root as the executor. Every
//! refusal below happens before any driver call (the executor gate refuses
//! before network, and nothing here reaches it).
use otto_core::access::{AccessActor, AccessRule, ResourceKind, RuleEffect, SubjectKind};
use otto_core::domain::{Capability, ConnectionKind, Environment, Feature};
use otto_server::ServerCtx;
use otto_state::{ConnectionsRepo, GrantsRepo, NewConnection, ResourceAccessRepo, UsersRepo};
use serde_json::{json, Value};

/// A Prod MySQL connection owned by `root`, whose policy grants each
/// `(user, operations)` pair on the `shop` database.
async fn prod_connection(
    pool: &otto_state::DbPool,
    root: &otto_core::domain::User,
    name: &str,
    grants: &[(&str, &[&str])],
) -> String {
    let conn = ConnectionsRepo::new(pool.clone())
        .create(NewConnection {
            workspace_id: None,
            name: name.into(),
            kind: ConnectionKind::Mysql,
            params: json!({"host": "127.0.0.1", "port": 1}),
            secret_ref: Some("fixture-secret".into()),
            first_command: None,
            section_id: None,
            environment: Environment::Prod,
            read_only: false,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let repo = ResourceAccessRepo::new(pool.clone());
    let mut policy = repo
        .get_policy(ResourceKind::Connection, &conn.id)
        .await
        .unwrap();
    policy.rules = grants
        .iter()
        .map(|(user, ops)| AccessRule {
            id: format!("grant-{user}"),
            subject_kind: SubjectKind::User,
            subject_id: (*user).to_string(),
            effect: RuleEffect::Allow,
            operations: ops.iter().map(|o| (*o).to_string()).collect(),
            children: Some(vec!["shop".into()]),
            grantable_operations: vec![],
            credential_connection_id: None,
        })
        .collect();
    let actor = AccessActor {
        real_user_id: root.id.clone(),
        effective_user_id: None,
    };
    repo.put_policy(&policy, policy.revision, &actor)
        .await
        .unwrap();
    conn.id
}

struct Api {
    origin: String,
    client: reqwest::Client,
}

impl Api {
    async fn post(&self, token: &str, path: &str, body: Value) -> (u16, Value) {
        let r = self
            .client
            .post(format!("{}{path}", self.origin))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }
    async fn get(&self, token: &str, path: &str) -> (u16, Value) {
        let r = self
            .client
            .get(format!("{}{path}", self.origin))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn each_review_step_is_refused_to_a_user_without_its_authority() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    ctx.secrets.put("fixture-secret", "pw").unwrap();
    let users = UsersRepo::new(pool.clone());
    let root = users.create("root", "unused", "Root", true).await.unwrap();
    let author = users
        .create("author", "unused", "Author", false)
        .await
        .unwrap();
    let reviewer = users
        .create("reviewer", "unused", "Reviewer", false)
        .await
        .unwrap();
    let peer = users.create("peer", "unused", "Peer", false).await.unwrap();
    let grants = GrantsRepo::new(pool.clone());
    for u in [&author, &reviewer, &peer] {
        grants
            .set_grants(&u.id, &[(Feature::Database, Capability::View)])
            .await
            .unwrap();
    }
    let auth = otto_rbac::AuthRepo::new(pool.clone());
    let root_t = auth.issue(&root.id).await.unwrap();
    let author_t = auth.issue(&author.id).await.unwrap();
    let reviewer_t = auth.issue(&reviewer.id).await.unwrap();
    let peer_t = auth.issue(&peer.id).await.unwrap();

    let submit: &[&str] = &["discover", "change_submit"];
    let approve: &[&str] = &["discover", "change_submit", "change_approve"];
    let conn = prod_connection(
        &pool,
        &root,
        "Prod",
        &[
            (author.id.as_str(), submit),
            (reviewer.id.as_str(), approve),
            (peer.id.as_str(), submit),
        ],
    )
    .await;
    // A second connection `peer` cannot discover at all.
    let hidden = prod_connection(
        &pool,
        &root,
        "Hidden",
        &[
            (author.id.as_str(), submit),
            (reviewer.id.as_str(), approve),
        ],
    )
    .await;

    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let api = Api {
        origin,
        client: reqwest::Client::new(),
    };

    // The author drafts, validates (root executes) and submits.
    let target = |c: &str| json!([{"connection_id": c, "node": "shop"}]);
    let (s, change) = api
        .post(
            &author_t,
            "/database-changes",
            json!({"title": "Add column", "script": "ALTER TABLE shop.t ADD c INT",
                   "targets": target(&conn)}),
        )
        .await;
    assert_eq!(s, 200, "{change}");
    let id = change["id"].as_str().unwrap().to_string();
    let (s, change) = api
        .post(
            &author_t,
            &format!("/database-changes/{id}/validate"),
            json!({"revision": change["revision"], "executor_id": root.id}),
        )
        .await;
    assert_eq!(s, 200, "validate: {change}");
    let (s, change) = api
        .post(
            &author_t,
            &format!("/database-changes/{id}/submit"),
            json!({"revision": change["revision"]}),
        )
        .await;
    assert_eq!(s, 200, "submit: {change}");
    assert_eq!(change["status"], "awaiting_review");
    let rev = change["revision"].clone();

    // 1. `peer` holds change_submit only: it may see the change, never decide it.
    let (s, body) = api
        .post(
            &peer_t,
            &format!("/database-changes/{id}/approve"),
            json!({"revision": rev}),
        )
        .await;
    assert_eq!(s, 403, "peer approve: {body}");
    let (s, body) = api
        .post(
            &peer_t,
            &format!("/database-changes/{id}/reject"),
            json!({"revision": rev, "note": "looks risky"}),
        )
        .await;
    assert_eq!(s, 403, "peer reject: {body}");
    // Only the author may cancel a change that is not running.
    let (s, body) = api
        .post(
            &peer_t,
            &format!("/database-changes/{id}/cancel"),
            json!({"revision": rev}),
        )
        .await;
    assert_eq!(s, 403, "peer cancel of the author's change: {body}");
    let (s, detail) = api.get(&peer_t, &format!("/database-changes/{id}")).await;
    assert_eq!(s, 200, "{detail}");
    assert_eq!(detail["change"]["status"], "awaiting_review");

    // 2. The reviewer holds change_approve.
    let (s, change) = api
        .post(
            &reviewer_t,
            &format!("/database-changes/{id}/approve"),
            json!({"revision": rev, "note": "ok"}),
        )
        .await;
    assert_eq!(s, 200, "reviewer approve: {change}");
    assert_eq!(change["status"], "approved");

    // 3. The approver loses approval authority (the Database page, so the
    //    connection policy — and the approval's snapshot — stay unchanged):
    //    execute is refused at the approver re-check, before any claim.
    grants.set_grants(&reviewer.id, &[]).await.unwrap();
    let (s, body) = api
        .post(
            &root_t,
            &format!("/database-changes/{id}/execute"),
            json!({"revision": change["revision"]}),
        )
        .await;
    assert_eq!(s, 403, "execute after the approver lost authority: {body}");
    let (_, detail) = api.get(&root_t, &format!("/database-changes/{id}")).await;
    assert_eq!(detail["change"]["status"], "approved", "{detail}");
    assert_eq!(detail["attempts"], json!([]), "no attempt row: {detail}");

    // 4. History never shows `peer` a revision that targeted a connection it
    //    cannot discover, though the current revision is visible to it.
    let (s, moved) = api
        .post(
            &author_t,
            "/database-changes",
            json!({"title": "Move", "script": "ALTER TABLE shop.t ADD d INT",
                   "targets": target(&hidden)}),
        )
        .await;
    assert_eq!(s, 200, "{moved}");
    let moved_id = moved["id"].as_str().unwrap().to_string();
    let r = api
        .client
        .put(format!("{}/database-changes/{moved_id}", api.origin))
        .bearer_auth(&author_t)
        .json(&json!({"revision": moved["revision"], "title": "Move",
                      "script": "ALTER TABLE shop.t ADD d INT", "targets": target(&conn)}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "revise onto the shared connection");
    let names_hidden = |detail: &Value| {
        detail["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["data"].to_string().contains(&hidden))
    };
    let (s, mine) = api
        .get(&author_t, &format!("/database-changes/{moved_id}"))
        .await;
    assert_eq!(s, 200);
    assert!(names_hidden(&mine), "the author sees revision 1: {mine}");
    let (s, theirs) = api
        .get(&peer_t, &format!("/database-changes/{moved_id}"))
        .await;
    assert_eq!(s, 200, "{theirs}");
    assert!(
        !names_hidden(&theirs),
        "peer must not read revision 1's hidden target: {theirs}"
    );

    // 5. A running change is stopped only with change_execute. The state is
    //    set in the table (a real claim would start the driver task); the
    //    route decides on `status` alone. The reviewer gets its Database
    //    grant back, so a 403 below is the policy's, not the page's.
    grants
        .set_grants(&reviewer.id, &[(Feature::Database, Capability::View)])
        .await
        .unwrap();
    let set_status = |status: &'static str| {
        let (pool, id) = (pool.clone(), id.clone());
        async move {
            sqlx::query("UPDATE database_changes SET status = ? WHERE id = ?")
                .bind(status)
                .bind(&id)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    set_status("running").await;
    let (_, detail) = api.get(&root_t, &format!("/database-changes/{id}")).await;
    let rev = detail["change"]["revision"].clone();
    for (who, token) in [("peer", &peer_t), ("reviewer", &reviewer_t)] {
        let (s, body) = api
            .post(
                token,
                &format!("/database-changes/{id}/cancel"),
                json!({"revision": rev}),
            )
            .await;
        assert_eq!(s, 403, "{who} cancel of a running change: {body}");
    }
    let (_, detail) = api.get(&root_t, &format!("/database-changes/{id}")).await;
    assert_eq!(detail["change"]["status"], "running", "{detail}");
    assert_eq!(
        detail["change"]["cancellation_requested"], false,
        "{detail}"
    );

    // 6. Settling an unknown attempt needs change_execute too.
    set_status("outcome_unknown").await;
    sqlx::query(
        "INSERT INTO database_change_attempts(id,change_id,connection_id,node,script,state,\
         executor_id,content_hash,policy_revision,connection_fingerprint,ordinal) \
         VALUES('attempt-1',?,?,'shop','ALTER TABLE shop.t ADD c INT','outcome_unknown',?,'h',1,'f',0)",
    )
    .bind(&id)
    .bind(&conn)
    .bind(&root.id)
    .execute(&pool)
    .await
    .unwrap();
    let (s, body) = api
        .post(
            &peer_t,
            &format!("/database-changes/{id}/reconcile"),
            json!({"revision": rev, "attempt_id": "attempt-1", "outcome": "succeeded",
                   "note": "checked the table by hand"}),
        )
        .await;
    assert_eq!(s, 403, "peer reconcile: {body}");
    let (_, detail) = api.get(&root_t, &format!("/database-changes/{id}")).await;
    assert_eq!(detail["change"]["status"], "outcome_unknown", "{detail}");
    assert_eq!(
        detail["attempts"][0]["state"], "outcome_unknown",
        "{detail}"
    );
    server.abort();
}
