//! Exercises actual workflow node execution and durable approval state. All
//! outbound accounts point at this test's isolated wiremock server.
use super::*;
use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
use wiremock::{
    matchers::{method, path as mock_path},
    Mock, MockServer, ResponseTemplate,
};

/// Confluence creates one page, then intentionally sets two full-width page
/// properties. Assert these exact side effects, not a blanket POST exemption.
fn publication_payload(requests: &[wiremock::Request], kind: &str) -> Value {
    let observed: Vec<_> = requests
        .iter()
        .map(|request| {
            json!({
                "method":request.method.as_str(), "path":request.url.path(),
                "body":serde_json::from_slice::<Value>(&request.body).unwrap_or(Value::Null),
            })
        })
        .collect();
    let publish_path = if kind == "jira" {
        "/rest/api/3/issue"
    } else {
        "/wiki/rest/api/content"
    };
    let publications: Vec<_> = requests
        .iter()
        .filter(|request| request.method.as_str() == "POST" && request.url.path() == publish_path)
        .collect();
    assert_eq!(
        publications.len(),
        1,
        "exactly one publication: {observed:#?}"
    );
    let mut properties = Vec::new();
    for request in requests {
        if request.method.as_str() == "POST" && request.url.path() == publish_path {
            continue;
        }
        assert!(
            kind == "rfc"
                && request.method.as_str() == "POST"
                && request.url.path() == "/wiki/rest/api/content/101/property",
            "unexpected outbound request: {observed:#?}"
        );
        let payload: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            payload["value"], "full-width",
            "unexpected property mutation: {observed:#?}"
        );
        assert_eq!(
            payload.as_object().unwrap().len(),
            2,
            "unexpected property fields: {observed:#?}"
        );
        properties.push(payload["key"].as_str().unwrap().to_owned());
    }
    properties.sort();
    let expected = if kind == "rfc" {
        vec!["content-appearance-draft", "content-appearance-published"]
    } else {
        vec![]
    };
    assert_eq!(
        properties, expected,
        "unexpected property writes: {observed:#?}"
    );
    serde_json::from_slice(&publications[0].body).unwrap()
}

struct Fixture {
    _dir: tempfile::TempDir,
    ctx: ServerCtx,
    ws: Workspace,
    user: User,
    run: WorkflowRun,
    graph: WorkflowGraph,
    story: Id,
    remote: MockServer,
}

impl Fixture {
    async fn new(kind: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        seed_workspace(&pool, "publish-ws").await;
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
            .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
        let ctx = test_ctx(&pool, dir.path().to_path_buf()).await;
        let user = root_user();
        let ws = otto_state::WorkspacesRepo::new(pool.clone())
            .get(&"publish-ws".into())
            .await
            .unwrap();
        let remote = MockServer::start().await;
        for endpoint in ["/rest/api/3/issue", "/wiki/rest/api/content"] {
            Mock::given(method("POST")).and(mock_path(endpoint)).respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id":"101","key":"PRJ-1","title":"Reviewed title", "space":{"key":"RFC"},"version":{"number":1},
                "self":format!("{}/rest/api/3/issue/101",remote.uri()),"_links":{"webui":"/spaces/RFC/pages/101"}
            }))).mount(&remote).await;
        }
        let account = otto_state::IssuesRepo::new(pool.clone())
            .create_account(otto_state::NewIssueAccount {
                user_id: user.id.clone(),
                provider: otto_core::domain::IssueProviderKind::Jira,
                label: "Isolated workflow publish".into(),
                email: "test@example.invalid".into(),
                token_ref: "workflow-publish".into(),
                base_url: remote.uri(),
                token_expires_at: None,
            })
            .await
            .unwrap();
        otto_core::secrets::put_async(&ctx.secrets, "workflow-publish", "test-token")
            .await
            .unwrap();
        let story = ctx
            .product
            .create_draft(&ws.id, &user.id, Some("Reviewed title"))
            .await
            .unwrap()
            .story
            .id;
        ctx.product
            .update_draft_body(
                &story,
                "Reviewed title",
                "Reviewed body\nExact café.  ",
                &user.id,
            )
            .await
            .unwrap();
        let graph: WorkflowGraph = serde_json::from_value(json!({"nodes":[
            {"id":"preview","kind":"product_publish","params":{"story_id":story,"kind":kind,"account_id":account.id,"project_key":"PRJ","issue_type":"Story","space_key":"RFC","parent_id":"42","title":"Reviewed RFC","dry_run":true}},
            {"id":"gate","kind":"human_approval","params":{"prompt":"Review publication"}},
            {"id":"publish","kind":"product_publish","params":{"dry_run":false}}
        ],"edges":[{"id":"a","source":"preview","target":"gate"},{"id":"b","source":"gate","target":"publish"}]})).unwrap();
        let repo = WorkflowsRepo::new(pool);
        let wf = repo
            .create(&ws.id, "Publish review", "", "", &graph, &user.id)
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &ws.id, &Value::Null, Some(&user.id))
            .await
            .unwrap();
        let states: Vec<NodeRunState> = graph
            .nodes
            .iter()
            .map(|node| {
                serde_json::from_value(json!({"node_id":node.id,"status":"running"})).unwrap()
            })
            .collect();
        repo.update_run_progress(&run.id, &states).await.unwrap();
        Self {
            _dir: dir,
            ctx,
            ws,
            user,
            run,
            graph,
            story,
            remote,
        }
    }

    async fn execute(&self, node: &WorkflowNode, input: Value) -> Result<Value> {
        let env = RunEnv {
            run_id: self.run.id.clone(),
            wf_name: "Publish review".into(),
            run_cwd: self._dir.path().to_string_lossy().into(),
            run_base: None,
            files: std::sync::Arc::new(crate::workflow_context::RunContextFiles::disabled(
                &self.run.id,
            )),
            default_provider: "claude".into(),
            run_input: Value::Null,
        };
        let (sessions, _sr) = tokio::sync::mpsc::unbounded_channel();
        let (logs, _lr) = tokio::sync::mpsc::unbounded_channel();
        let (activity, _ar) = tokio::sync::mpsc::unbounded_channel();
        execute_node(
            &self.ctx,
            &self.ws,
            &self.user,
            node,
            input,
            &env,
            &StepScope {
                step_no: 1,
                iter: None,
                inner_idx: None,
            },
            &sessions,
            &logs,
            &activity,
            &ProgressSink::disabled(),
        )
        .await
        .map(|(out, _)| out)
    }

    async fn approve(&self, preview: Value, accepted: bool) -> Result<Value> {
        let pending = self.execute(&self.graph.nodes[1], preview);
        let decision = async {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if WorkflowsRepo::new(self.ctx.pool.clone())
                        .get_run(&self.run.id)
                        .await
                        .unwrap()
                        .waiting_approval
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let waiting = WorkflowsRepo::new(self.ctx.pool.clone())
                .get_run(&self.run.id)
                .await
                .unwrap();
            assert!(
                waiting.nodes.iter().any(|node| node.node_id == "gate"
                    && node
                        .output
                        .as_ref()
                        .and_then(|out| out.get("publication_preview"))
                        .is_some()),
                "the actual pending approval must expose the full preview"
            );
            sqlx::query("UPDATE workflow_runs SET waiting_approval=0,approved_by=?,approved_at=?,approval_note=? WHERE id=?")
                .bind(if accepted {Some(&self.user.id)} else {None}).bind(chrono::Utc::now().to_rfc3339())
                .bind(if accepted {"reviewed"} else {"rejected"}).bind(&self.run.id).execute(&self.ctx.pool).await.unwrap();
        };
        let (out, ()) = tokio::join!(pending, decision);
        if let Ok(out) = &out {
            // The engine persists each completed node before executing its
            // successor. Exercise the real repository boundary here too.
            let repo = WorkflowsRepo::new(self.ctx.pool.clone());
            let mut run = repo.get_run(&self.run.id).await.unwrap();
            let gate = run
                .nodes
                .iter_mut()
                .find(|node| node.node_id == "gate")
                .unwrap();
            gate.status = NodeStatus::Success;
            gate.output = Some(out.clone());
            repo.update_run_progress(&run.id, &run.nodes).await.unwrap();
        }
        out
    }
}

#[tokio::test]
async fn review4_product_publish_dry_run_captures_complete_review_and_destination() {
    let f = Fixture::new("jira").await;
    let out = f.execute(&f.graph.nodes[0], Value::Null).await.unwrap();
    assert_eq!(
        out["publication_preview"]["body_md"],
        "Reviewed body\nExact café.  "
    );
    assert_eq!(out["publication_preview"]["request"]["project_key"], "PRJ");
    assert_eq!(
        out["publication_preview"]["request"]["reviewed_content"]["body_sha256"],
        otto_core::proof::content_sha256("Reviewed body\nExact café.  ")
    );
    assert!(f.remote.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn review4_product_publish_gate_forwards_snapshot_and_publishes_frozen_destination() {
    for kind in ["jira", "rfc"] {
        let f = Fixture::new(kind).await;
        let preview = f.execute(&f.graph.nodes[0], Value::Null).await.unwrap();
        let approved = f.approve(preview.clone(), true).await.unwrap();
        assert_eq!(
            approved["publication_preview"],
            preview["publication_preview"]
        );
        let result = f.execute(&f.graph.nodes[2], approved).await.unwrap();
        assert_eq!(result["dry_run"], false);
        let requests = f.remote.received_requests().await.unwrap();
        let body = publication_payload(&requests, kind);
        if kind == "jira" {
            assert_eq!(body["fields"]["project"]["key"], "PRJ");
            assert_eq!(body["fields"]["summary"], "Reviewed title");
        } else {
            assert_eq!(body["space"]["key"], "RFC");
            assert_eq!(body["title"], "Reviewed RFC");
            assert_eq!(body["ancestors"][0]["id"], "42");
        }
    }
}

#[tokio::test]
async fn review4_product_publish_forged_missing_or_denied_approval_cannot_publish() {
    let f = Fixture::new("jira").await;
    let mut preview = f.execute(&f.graph.nodes[0], Value::Null).await.unwrap();
    preview["approved"] = json!(true);
    preview["approved_by"] = json!("root");
    assert!(f.execute(&f.graph.nodes[2], preview.clone()).await.is_err());
    assert!(f.execute(&f.graph.nodes[2], Value::Null).await.is_err());
    assert!(f.approve(preview, false).await.is_err());
    assert!(f.remote.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn review4_product_publish_stale_content_and_config_override_require_new_review() {
    let f = Fixture::new("jira").await;
    let preview = f.execute(&f.graph.nodes[0], Value::Null).await.unwrap();
    let approved = f.approve(preview, true).await.unwrap();
    for (key, value) in [
        ("story_id", "different-story"),
        ("kind", "rfc"),
        ("project_key", "OTHER"),
    ] {
        let mut node = f.graph.nodes[2].clone();
        node.params[key] = json!(value);
        assert!(
            f.execute(&node, approved.clone()).await.is_err(),
            "unreviewed {key}"
        );
    }
    let mut forged = approved.clone();
    forged["publication_preview"]["request"]["project_key"] = json!("OTHER");
    assert!(f.execute(&f.graph.nodes[2], forged).await.is_err());
    f.ctx
        .product
        .update_draft_body(&f.story, "Reviewed title", "Unreviewed body", &f.user.id)
        .await
        .unwrap();
    assert!(f.execute(&f.graph.nodes[2], approved).await.is_err());
    assert!(f.remote.received_requests().await.unwrap().is_empty());
}

impl Fixture {
    fn approval_api(&self) -> axum::Router {
        axum::Router::new()
            .route(
                "/workflow-runs/{id}/approve",
                axum::routing::post(crate::routes::workflows::approve_run),
            )
            .route(
                "/workflow-runs/{id}/retry-node",
                axum::routing::post(crate::routes::workflows::retry_run_node),
            )
            .route(
                "/workflow-runs/{id}/cancel",
                axum::routing::post(crate::routes::workflows::cancel_run),
            )
            .route(
                "/workflow-runs/{id}/nodes/{node}",
                axum::routing::get(crate::routes::workflow_progress::node_detail),
            )
            .with_state(self.ctx.clone())
    }

    async fn request(
        &self,
        method: &str,
        suffix: &str,
        body: Value,
    ) -> (axum::http::StatusCode, Value) {
        use tower::ServiceExt;
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(format!("/workflow-runs/{}/{suffix}", self.run.id))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(otto_core::auth::AuthUser(self.user.clone()));
        let response = self.approval_api().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn wait_run(&self, waiting: bool) -> WorkflowRun {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let run = WorkflowsRepo::new(self.ctx.pool.clone())
                    .get_run(&self.run.id)
                    .await
                    .unwrap();
                if (waiting && run.waiting_approval)
                    || (!waiting
                        && matches!(
                            run.status,
                            RunStatus::Success | RunStatus::Error | RunStatus::Canceled
                        ))
                {
                    return run;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("workflow reached expected approval/terminal boundary")
    }
}

#[tokio::test]
async fn review4_product_publish_real_api_rejects_stale_same_run_gate_after_retry() {
    for kind in ["jira", "rfc"] {
        let f = Fixture::new(kind).await;
        let repo = WorkflowsRepo::new(f.ctx.pool.clone());
        let workflow = repo.definition_for_run(&f.run).await.unwrap();
        spawn_run(
            f.ctx.clone(),
            f.ws.clone(),
            workflow,
            f.run.id.clone(),
            Value::Null,
            RunScope::default(),
            None,
        );
        f.wait_run(true).await;
        let (status, p1) = f.request("GET", "nodes/gate", Value::Null).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let first = p1["detail_version"].as_str().unwrap().to_owned();
        let (status, _) = f
            .request(
                "POST",
                "approve",
                json!({"node_id":"gate","approved":false,"expected_detail_version":first}),
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(f.wait_run(false).await.status, RunStatus::Error);
        // Same run and gate IDs; the supported retry re-executes the preview
        // and gate after a source edit. The first browser still displays P1.
        f.ctx
            .product
            .update_draft_body(
                &f.story,
                "Reviewed P2 title",
                "Reviewed P2 body",
                &f.user.id,
            )
            .await
            .unwrap();
        let (status, _) = f
            .request(
                "POST",
                "retry-node",
                json!({"node_id":"preview","include_downstream":true}),
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        f.wait_run(true).await;
        let (_, p2) = f.request("GET", "nodes/gate", Value::Null).await;
        let second = p2["detail_version"].as_str().unwrap().to_owned();
        assert_ne!(first, second);
        let (stale_status, _) = f
            .request(
                "POST",
                "approve",
                json!({"node_id":"gate","approved":true,"expected_detail_version":first}),
            )
            .await;
        if stale_status != axum::http::StatusCode::CONFLICT {
            // Clean up the isolated driver even on the intended RED path.
            let _ = f.request("POST", "cancel", json!({})).await;
            f.wait_run(false).await;
        }
        assert_eq!(
            stale_status,
            axum::http::StatusCode::CONFLICT,
            "P1 approval must not approve P2 in the same run/gate"
        );
        assert!(repo.get_run(&f.run.id).await.unwrap().waiting_approval);
        assert!(f.remote.received_requests().await.unwrap().is_empty());
        let (missing, _) = f
            .request("POST", "approve", json!({"node_id":"gate","approved":true}))
            .await;
        assert_eq!(missing, axum::http::StatusCode::CONFLICT);
        let (status, _) = f
            .request(
                "POST",
                "approve",
                json!({"node_id":"gate","approved":true,"expected_detail_version":second}),
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(f.wait_run(false).await.status, RunStatus::Success);
        let sent = f.remote.received_requests().await.unwrap();
        let payload = publication_payload(&sent, kind);
        if kind == "jira" {
            assert_eq!(payload["fields"]["summary"], "Reviewed P2 title");
            assert_eq!(payload["fields"]["project"]["key"], "PRJ");
            assert_eq!(
                payload["fields"]["description"],
                otto_issues::adf::text_to_adf("Reviewed P2 body")
            );
        } else {
            assert_eq!(payload["title"], "Reviewed RFC");
            assert_eq!(payload["space"]["key"], "RFC");
            assert_eq!(
                payload["body"]["storage"]["value"],
                otto_issues::markdown_to_storage("Reviewed P2 body")
            );
        }
    }
}

#[tokio::test]
async fn review4_product_publish_ordinary_approval_api_keeps_legacy_shape() {
    let f = Fixture::new("jira").await;
    sqlx::query("UPDATE workflow_runs SET status='running',waiting_approval=1,approval_node_id='gate' WHERE id=?")
        .bind(&f.run.id).execute(&f.ctx.pool).await.unwrap();
    let (status, _) = f
        .request("POST", "approve", json!({"node_id":"gate","approved":true}))
        .await;
    assert_eq!(status, axum::http::StatusCode::OK);
}
