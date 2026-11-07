//! Graph definitions must be validated before persistence, and survive a real
//! create/detail roundtrip without launching provider sessions.
use axum::{body::Body, http::Request, Router};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower::ServiceExt;

async fn call(app: &Router, token: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn graph_create_rejects_invalid_dependencies_and_roundtrips_research_report() {
    let tmp = tempfile::tempdir().unwrap();
    let pool: otto_state::DbPool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(true),
        )
        .await
        .unwrap()
        .into();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let ctx = otto_server::ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let owner = otto_state::UsersRepo::new(pool.clone())
        .create("graph-owner", "unused", "Graph owner", true)
        .await
        .unwrap();
    let token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&owner.id)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("Graph test", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let (api, root) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api, root);
    let path = format!("/api/v1/workspaces/{}/goal-loops", ws.id);
    let mut config = serde_json::to_value(otto_core::domain::GoalLoopConfig::default()).unwrap();
    config["graph"] = json!({"nodes":[
        {"id":"inspect","title":"Inspect contracts","instructions":"Trace current values and contracts.","depends_on":[],"criterion_ids":["contract"]},
        {"id":"implement","title":"Implement change","instructions":"Change only the inspected contract.","depends_on":["inspect"],"criterion_ids":["behavior"]}
    ]});
    let input = json!({"name":"Graph goal","repo_path":tmp.path(),"autostart":false,
        "definition":{"title":"Graph goal","acceptance_criteria":[
            {"id":"contract","text":"Contract recorded","verify":"Inspect a concrete contract trace","verify_kind":"agent"},
            {"id":"behavior","text":"Behavior verified","verify":"Check changed critical behavior","verify_kind":"agent"}
        ]},"config":config,"limits":otto_core::domain::GoalLoopLimits::default()});
    for defect in [
        "cycle",
        "missing dependency",
        "missing criterion",
        "uncovered criterion",
    ] {
        let mut bad = input.clone();
        match defect {
            "cycle" => bad["config"]["graph"]["nodes"][0]["depends_on"] = json!(["implement"]),
            "missing dependency" => {
                bad["config"]["graph"]["nodes"][1]["depends_on"] = json!(["absent"])
            }
            "missing criterion" => {
                bad["config"]["graph"]["nodes"][1]["criterion_ids"] = json!(["absent"])
            }
            _ => bad["config"]["graph"]["nodes"][1]["criterion_ids"] = json!(["contract"]),
        }
        let (status, body) = call(&app, &token, "POST", &path, bad).await;
        assert_eq!(status, 400, "{defect}: {body}");
        assert!(ctx
            .goal_loops_repo
            .list_by_workspace(&ws.id)
            .await
            .unwrap()
            .is_empty());
    }
    let (status, created) = call(&app, &token, "POST", &path, input.clone()).await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["status"], "draft");
    let detail_path = format!("/api/v1/goal-loops/{}", created["id"].as_str().unwrap());
    let (status, detail) = call(&app, &token, "GET", &detail_path, Value::Null).await;
    assert_eq!(status, 200, "{detail}");
    assert_eq!(detail["loop"]["config"]["graph"], input["config"]["graph"]);

    let mut research = input;
    research["config"]["mode"] = json!("research");
    research["config"]["graph"]["nodes"][1]["criterion_ids"] =
        json!(["behavior", "otto-research-report"]);
    let (status, created) = call(&app, &token, "POST", &path, research).await;
    assert_eq!(status, 200, "{created}");
    let criteria = created["definition"]["acceptance_criteria"]
        .as_array()
        .unwrap();
    let report = criteria
        .iter()
        .find(|c| c["id"] == "otto-research-report")
        .unwrap();
    assert_eq!(report["verify_kind"], "command");
    assert_eq!(report["verify_cmd"], "test -s findings.md");
}
