//! Skip and Optimize routes (addendum §2.6 test 5, DR-110, test-plan O7).
use super::*;
use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
use otto_core::workflows::{NodeRunState, Workflow};
use serde_json::Value;
use tower::ServiceExt;

const CHECKOUT_RECIPE: &str = "Read repos.json. For each entry: git fetch, then \
    git checkout --detach origin/<branch>, take git merge-base origin/<base> HEAD, and \
    write source, base and base_branch back into repos.json.";

struct Fx {
    _tmp: tempfile::TempDir,
    ctx: ServerCtx,
    app: axum::Router,
    wf: Workflow,
}

async fn fixture() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "opt-ws").await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
        .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
    let ctx = test_ctx(&pool, tmp.path().join("data")).await;
    // Built edges: a `"source"` literal trips otto-core's session-source guard.
    let edge = |id: &str, s: &str, t: &str| json!({"id": id, "source": s, "target": t});
    let graph: WorkflowGraph = serde_json::from_value(json!({
        "nodes": [
            {"id": "n1", "kind": "manual_trigger", "name": "Start"},
            {"id": "n3", "kind": "agent_prompt", "name": "Checkout PR Branch",
             "params": {"prompt": CHECKOUT_RECIPE, "provider": "claude"}},
            {"id": "n4", "kind": "log", "name": "Done"}
        ],
        "edges": [edge("e1", "n1", "n3"), edge("e2", "n3", "n4")]
    }))
    .unwrap();
    let wf = WorkflowsRepo::new(pool)
        .create(
            &"opt-ws".into(),
            "PR Reviewer",
            "",
            "",
            &graph,
            &"root".into(),
        )
        .await
        .unwrap();
    let app = axum::Router::new().merge(routes()).with_state(ctx.clone());
    Fx {
        _tmp: tmp,
        ctx,
        app,
        wf,
    }
}

impl Fx {
    async fn post(&self, path: &str, body: Value, human: bool) -> (StatusCode, Value) {
        let agent = (!human).then(|| "agent-session".into());
        self.send(path, body, root_user(), agent, None).await
    }
    async fn send(
        &self,
        path: &str,
        body: Value,
        user: otto_core::domain::User,
        mcp_session_id: Option<String>,
        managed_session_id: Option<String>,
    ) -> (StatusCode, Value) {
        let mut req = axum::http::Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        req.extensions_mut()
            .insert(otto_core::auth::AuthUser(user.clone()));
        req.extensions_mut().insert(otto_core::auth::AuthContext {
            real_user: user.clone(),
            effective_user: user,
            scope: None,
            mcp_only: false,
            mcp_scope: None,
            mcp_internal: false,
            mcp_session_id,
            managed_session_id,
            device_id: None,
        });
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    async fn stored(&self) -> Workflow {
        WorkflowsRepo::new(self.ctx.pool.clone())
            .get(&self.wf.id)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn optimize_drafts_and_a_person_applies_with_a_version_check() {
    let f = fixture().await;
    let path = format!("/workflows/{}/optimize", f.wf.id);
    let (st, draft) = f.post(&path, json!({}), false).await;
    assert_eq!(st, StatusCode::OK, "{draft}");
    let id = "git_checkout:n3";
    assert!(
        draft["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == id),
        "{draft}"
    );
    let n3 = |g: &Value| {
        g["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == "n3")
            .unwrap()["kind"]
            .clone()
    };
    assert_eq!(n3(&draft["draft_graph"]), "git_checkout");

    let apply = format!("{path}/apply");
    let v = f.wf.version;
    let (st, _) = f
        .post(
            &apply,
            json!({"base_version": v, "change_ids": [id]}),
            false,
        )
        .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "an agent credential never rewrites a graph"
    );
    let (st, _) = f
        .post(
            &apply,
            json!({"base_version": v - 1, "change_ids": [id]}),
            true,
        )
        .await;
    assert_eq!(st, StatusCode::CONFLICT, "a stale base version");
    let (st, body) = f
        .post(
            &apply,
            json!({"base_version": v, "change_ids": ["parallel:n99"]}),
            true,
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(f.stored().await.version, v, "nothing saved yet");

    // A forged graph in the body is ignored: the graph is re-derived.
    let body = json!({"base_version": v, "change_ids": [id], "graph": {"nodes": [], "edges": []}});
    let (st, saved) = f.post(&apply, body, true).await;
    assert_eq!(st, StatusCode::OK, "{saved}");
    let wf = f.stored().await;
    assert_eq!(wf.version, v + 1);
    assert_eq!(
        serde_json::to_value(&wf.graph).unwrap(),
        draft["draft_graph"]
    );
    let note: String = sqlx::query_scalar(
        "SELECT note FROM workflow_versions WHERE workflow_id = ? ORDER BY version DESC LIMIT 1",
    )
    .bind(&wf.id)
    .fetch_one(&f.ctx.pool)
    .await
    .unwrap();
    assert!(note.starts_with("Optimized by "), "{note}");
    let (st, _) = f
        .post(&apply, json!({"base_version": v, "change_ids": [id]}), true)
        .await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "a second apply on the same base loses"
    );
}

#[tokio::test]
async fn skip_needs_a_running_step_that_is_not_the_approval_gate() {
    let f = fixture().await;
    let repo = WorkflowsRepo::new(f.ctx.pool.clone());
    let run = repo
        .create_run(&f.wf.id, &f.wf.workspace_id, &json!({}), None)
        .await
        .unwrap();
    let running = NodeRunState {
        node_id: "n3".into(),
        status: NodeStatus::Running,
        ..Default::default()
    };
    sqlx::query("UPDATE workflow_runs SET status = 'running', nodes_json = ? WHERE id = ?")
        .bind(serde_json::to_string(&vec![running]).unwrap())
        .bind(&run.id)
        .execute(&f.ctx.pool)
        .await
        .unwrap();
    let skip = |n: &str| format!("/workflow-runs/{}/nodes/{n}/skip", run.id);
    let (st, _) = f.post(&skip("n4"), json!({}), true).await;
    assert_eq!(st, StatusCode::CONFLICT, "n4 is not running");
    let (st, body) = f.post(&skip("n3"), json!({}), true).await;
    assert_eq!((st, body["ok"].as_bool()), (StatusCode::OK, Some(true)));
    let key = crate::workflow_engine::skip_marker_key(&run.id, "n3");
    assert!(f.ctx.wf_skip_current.lock().unwrap().contains(&key));
    sqlx::query(
        "UPDATE workflow_runs SET waiting_approval = 1, approval_node_id = 'n3' WHERE id = ?",
    )
    .bind(&run.id)
    .execute(&f.ctx.pool)
    .await
    .unwrap();
    let (st, _) = f.post(&skip("n3"), json!({}), true).await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "the approval gate is never skipped"
    );
}

/// A workspace Viewer of opt-ws (not root).
async fn viewer(f: &Fx) -> otto_core::domain::User {
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('vera','vera','x','Vera',0,?)")
        .bind(chrono::Utc::now().to_rfc3339()).execute(&f.ctx.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workspace_members(workspace_id,user_id,role) VALUES('opt-ws','vera','viewer')",
    )
    .execute(&f.ctx.pool)
    .await
    .unwrap();
    otto_core::domain::User {
        id: "vera".into(),
        username: "vera".into(),
        display_name: "Vera".into(),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    }
}

/// CR1-052 (2): a native agent session's credential and a workspace Viewer
/// never apply a draft or skip a step; of two applies on one base, one wins.
#[tokio::test]
async fn apply_and_skip_refuse_agents_and_viewers_and_one_of_two_applies_wins() {
    let f = fixture().await;
    let apply = format!("/workflows/{}/optimize/apply", f.wf.id);
    let body = json!({"base_version": f.wf.version, "change_ids": ["git_checkout:n3"]});
    let managed = Some("native-agent".to_string());
    let (st, _) = f
        .send(&apply, body.clone(), root_user(), None, managed)
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "a managed agent session");
    let vera = viewer(&f).await;
    let (st, _) = f.send(&apply, body.clone(), vera.clone(), None, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "a Viewer");
    let repo = WorkflowsRepo::new(f.ctx.pool.clone());
    let run = repo
        .create_run(&f.wf.id, &f.wf.workspace_id, &json!({}), None)
        .await
        .unwrap();
    let running = NodeRunState {
        node_id: "n3".into(),
        status: NodeStatus::Running,
        ..Default::default()
    };
    sqlx::query("UPDATE workflow_runs SET status = 'running', nodes_json = ? WHERE id = ?")
        .bind(serde_json::to_string(&vec![running]).unwrap())
        .bind(&run.id)
        .execute(&f.ctx.pool)
        .await
        .unwrap();
    let skip = format!("/workflow-runs/{}/nodes/n3/skip", run.id);
    let (st, _) = f.send(&skip, json!({}), vera, None, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "a Viewer never skips");
    assert_eq!(f.stored().await.version, f.wf.version, "nothing saved");

    let (a, b) = tokio::join!(
        f.post(&apply, body.clone(), true),
        f.post(&apply, body, true)
    );
    let mut got = [a.0, b.0];
    got.sort();
    assert_eq!(got, [StatusCode::OK, StatusCode::CONFLICT], "{a:?} {b:?}");
    assert_eq!(f.stored().await.version, f.wf.version + 1);
}
