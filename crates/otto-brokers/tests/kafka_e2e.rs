//! End-to-end test of the Message Brokers REST surface against a real Redpanda,
//! driving the actual axum router (`otto_brokers::api_router`) + service +
//! rdkafka driver. Gated on `OTTO_BROKERS_E2E=1` and `#[ignore]`.
//!
//!   docker compose -f dev/brokers/docker-compose.yml up -d   # wait for healthy
//!   OTTO_BROKERS_E2E=1 cargo test -p otto-brokers --test kafka_e2e -- --ignored --nocapture
//!
//! Overridable via env: OTTO_BROKERS_E2E_BOOTSTRAP (127.0.0.1:19092),
//! OTTO_BROKERS_E2E_SR (http://127.0.0.1:18081),
//! OTTO_BROKERS_E2E_METRICS (http://127.0.0.1:19644/public_metrics).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::{Extension, Router};
use futures_util::future::BoxFuture;
use http_body_util::BodyExt;
use otto_brokers::{api_router, BrokersCtx, BrokersService};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::secrets::SecretStore;
use otto_core::{new_id, Id, Result};
use serde_json::{json, Value};
use tower::ServiceExt;

fn enabled() -> bool {
    std::env::var("OTTO_BROKERS_E2E").is_ok()
}
fn bootstrap() -> String {
    std::env::var("OTTO_BROKERS_E2E_BOOTSTRAP").unwrap_or_else(|_| "127.0.0.1:19092".into())
}
fn sr_url() -> String {
    std::env::var("OTTO_BROKERS_E2E_SR").unwrap_or_else(|_| "http://127.0.0.1:18081".into())
}
fn metrics_url() -> String {
    std::env::var("OTTO_BROKERS_E2E_METRICS")
        .unwrap_or_else(|_| "http://127.0.0.1:19644/public_metrics".into())
}

// --- in-memory secret store ------------------------------------------------
#[derive(Default)]
struct MemSecrets(Mutex<HashMap<String, String>>);
impl SecretStore for MemSecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

// --- permissive role checker (auth/role coverage is unit-tested elsewhere) --
struct AllowAll;
impl RoleChecker for AllowAll {
    fn check<'a>(
        &'a self,
        _u: &'a User,
        _ws: &'a Id,
        _m: WorkspaceRole,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
struct TestCtx {
    brokers: Arc<BrokersService>,
    roles: Arc<dyn RoleChecker>,
}
impl BrokersCtx for TestCtx {
    fn brokers(&self) -> &Arc<BrokersService> {
        &self.brokers
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

fn root_user(id: &Id) -> User {
    User {
        id: id.clone(),
        username: "e2e".into(),
        display_name: "E2E".into(),
        is_root: true,
        disabled: false,
        created_at: chrono::Utc::now(),
    }
}

async fn build_app() -> (Router, Id) {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("e2e.db");
    let pool = otto_state::open(&db).await.expect("open db");
    // Leak the tempdir so the file outlives the test pool.
    std::mem::forget(tmp);

    let user_id = new_id();
    let ws_id = new_id();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?,?,?,?,1,?)")
        .bind(&user_id).bind("e2e").bind("x").bind("E2E").bind(&now)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?,?,?,?)")
        .bind(&ws_id)
        .bind("e2e-ws")
        .bind("/tmp/e2e")
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();

    let svc = Arc::new(BrokersService::new(
        otto_state::BrokerClustersRepo::new(pool.clone()),
        otto_state::BrokerClusterSectionsRepo::new(pool),
        Arc::new(MemSecrets::default()),
        None,
    ));
    let ctx = TestCtx {
        brokers: svc,
        roles: Arc::new(AllowAll),
    };
    let app = api_router::<TestCtx>()
        .layer(Extension(AuthUser(root_user(&user_id))))
        .with_state(ctx);
    (app, ws_id)
}

async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(match body {
            Some(b) => Body::from(b.to_string()),
            None => Body::empty(),
        })
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

#[tokio::test]
#[ignore]
async fn full_surface_against_redpanda() {
    if !enabled() {
        eprintln!("skipping: set OTTO_BROKERS_E2E=1 (and start dev/brokers/docker-compose.yml)");
        return;
    }
    let (app, ws) = build_app().await;
    let topic = format!(
        "otto-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    // 1. Create the cluster (with metrics + schema-registry wired).
    let (st, cluster) = call(
        &app,
        "POST",
        &format!("/workspaces/{ws}/brokers/clusters"),
        Some(json!({
            "name": "redpanda-e2e",
            "bootstrap_servers": bootstrap(),
            "security_protocol": "plaintext",
            "schema_registry_url": sr_url(),
            "metrics_url": metrics_url(),
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "create cluster: {cluster}");
    let id = cluster["id"].as_str().unwrap().to_string();

    // 2. Test connection.
    let (st, test) = call(
        &app,
        "POST",
        &format!("/brokers/clusters/{id}/test"),
        Some(json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(test["ok"], json!(true), "test: {test}");
    assert!(test["broker_count"].as_i64().unwrap() >= 1);

    // 3. Overview.
    let (st, ov) = call(
        &app,
        "GET",
        &format!("/brokers/clusters/{id}/overview"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        !ov["brokers"].as_array().unwrap().is_empty(),
        "overview: {ov}"
    );

    // 4. Create a topic (3 partitions).
    let (st, _t) = call(
        &app,
        "POST",
        &format!("/brokers/clusters/{id}/topics"),
        Some(json!({ "name": topic, "partitions": 3, "replication_factor": 1 })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "create topic");

    // 5. Produce three JSON messages.
    for i in 0..3 {
        let (st, r) = call(
            &app,
            "POST",
            &format!("/brokers/clusters/{id}/topics/{topic}/produce"),
            Some(json!({ "key": format!("k{i}"), "value": format!("{{\"n\":{i}}}") })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "produce {i}: {r}");
    }

    // 6. Consume from the beginning — expect our 3 messages, JSON-decoded.
    let (st, consumed) = call(
        &app,
        "POST",
        &format!("/brokers/clusters/{id}/topics/{topic}/consume"),
        Some(json!({ "start": { "type": "beginning" }, "limit": 50, "decode": "auto" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let msgs = consumed["messages"].as_array().unwrap();
    assert!(
        msgs.len() >= 3,
        "expected >=3 messages, got {}: {consumed}",
        msgs.len()
    );
    assert!(
        msgs.iter().any(|m| m["value"]["format"] == json!("json")),
        "expected a JSON-decoded value"
    );
    assert!(msgs.iter().any(|m| m["key"]["text"].as_str() == Some("k0")));

    // 7. Topic detail: partitions + message count.
    let (st, detail) = call(
        &app,
        "GET",
        &format!("/brokers/clusters/{id}/topics/{topic}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(detail["partitions"].as_array().unwrap().len(), 3);
    assert!(detail["message_count"].as_i64().unwrap() >= 3);

    // 8. Configs.
    let (st, configs) = call(
        &app,
        "GET",
        &format!("/brokers/clusters/{id}/topics/{topic}/configs"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(!configs.as_array().unwrap().is_empty());

    // 9. Consumer groups (we consumed without committing → may be empty, but the
    //    endpoint must succeed and return an array).
    let (st, groups) = call(&app, "GET", &format!("/brokers/clusters/{id}/groups"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(groups.is_array());

    // 10. Metrics — total >= 3; Prometheus scrape of Redpanda should populate
    //     per-broker resource metrics.
    let (st, metrics) = call(
        &app,
        "GET",
        &format!("/brokers/clusters/{id}/metrics"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        metrics["total_messages"].as_i64().unwrap() >= 3,
        "metrics: {metrics}"
    );
    assert_eq!(
        metrics["prometheus_available"],
        json!(true),
        "metrics: {metrics}"
    );
    assert!(!metrics["brokers"].as_array().unwrap().is_empty());

    // 11. Schema registry subjects (none registered yet → empty array, 200).
    let (st, subjects) = call(
        &app,
        "GET",
        &format!("/brokers/clusters/{id}/schema-registry/subjects"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "subjects: {subjects}");
    assert!(subjects.is_array());

    // 12. Delete the topic, then the cluster.
    let (st, _) = call(
        &app,
        "DELETE",
        &format!("/brokers/clusters/{id}/topics/{topic}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = call(&app, "DELETE", &format!("/brokers/clusters/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    eprintln!("✓ Message Brokers E2E passed against Redpanda");
}
