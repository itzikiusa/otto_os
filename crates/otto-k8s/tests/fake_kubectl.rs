//! Router-level tests for the Kubernetes console driven through a **fake
//! `kubectl`** — a shell script placed first on `PATH` that records every argv
//! it receives and answers with fixture JSON. This exercises the real axum
//! router, the real SQLite repo (in-memory, migrated), the argv building and
//! the normalisation end-to-end without a cluster.
//!
//! Auth is NOT under test here (the policy table lives in otto-server); every
//! request carries a root `AuthUser` extension, exactly like the connections
//! harness in `crates/otto-connections/tests/global_grants.rs`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Extension;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_connections::Spawner;
use otto_core::auth::{AuthUser, BoxFuture};
use otto_core::domain::{Connection, Session, User};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_k8s::K8sCtx;
use otto_state::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::broadcast;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Fake kubectl
// ---------------------------------------------------------------------------

struct Fake {
    /// Directory holding the `kubectl` script (prepended to PATH).
    bin: PathBuf,
    /// Every invocation's argv, one per line.
    log: PathBuf,
}

/// One fake per test binary: PATH is process-global, so the script is written
/// once and every test reads its own slice of the shared argv log.
fn fake() -> &'static Fake {
    static FAKE: OnceLock<Fake> = OnceLock::new();
    FAKE.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("otto-k8s-fake-{}", otto_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
        let log = dir.join("argv.log");
        let script = format!(
            r#"#!/bin/sh
echo "$@" >> "{log}"
case " $* " in
  *" -n forbidden "*)
    echo 'Error from server (Forbidden): pods is forbidden: User "dev" cannot list resource "pods" in API group "" in the namespace "forbidden"' >&2
    exit 1 ;;
  *" version "*)
    echo '{{"clientVersion":{{"gitVersion":"v1.30.0"}},"serverVersion":{{"gitVersion":"v1.29.5"}}}}' ;;
  *" config view "*)
    case " $* " in
      *"broken.yaml"*) echo 'error: error loading config file: yaml: line 1: did not find expected key' >&2; exit 1 ;;
      *) cat "{fx}/config_view.json" ;;
    esac ;;
  *" get --raw /apis/metrics.k8s.io/v1beta1/namespaces/"*"/pods "*|*" get --raw /apis/metrics.k8s.io/v1beta1/pods "*)
    cat "{fx}/pod_metrics.json" ;;
  *" get --raw /apis/metrics.k8s.io/v1beta1 "*)
    echo '{{"kind":"APIResourceList"}}' ;;
  *" api-resources "*)
    printf 'applications.argoproj.io\nrollouts.argoproj.io\n' ;;
  *" get namespaces "*) echo '{{"items":[{{"metadata":{{"name":"shop"}},"status":{{"phase":"Active"}}}},{{"metadata":{{"name":"other"}},"status":{{"phase":"Active"}}}}]}}' ;;
  *" get pods "*) cat "{fx}/pods.json" ;;
  *" get secrets "*) cat "{fx}/secret_list.json" ;;
  *" get deployments "*) cat "{fx}/deployments.json" ;;
  *" rollout restart "*) echo "deployment.apps/web restarted" ;;
  *" rollout status "*) echo "Waiting for deployment \"web\" rollout to finish: 1 of 3 updated replicas are available..." >&2; exit 1 ;;
  *) echo '{{}}' ;;
esac
"#,
            log = log.display(),
            fx = fixtures.display()
        );
        let path = dir.join("kubectl");
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let old = std::env::var("PATH").unwrap_or_default();
        // Edition 2021: set_var is safe; tests in this binary all want the fake.
        std::env::set_var("PATH", format!("{}:{old}", dir.display()));
        Fake { bin: dir, log }
    })
}

fn argv_log() -> Vec<String> {
    std::fs::read_to_string(&fake().log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

async fn mem_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(true),
        )
        .await
        .expect("in-memory pool");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn seed_root(pool: &SqlitePool) -> User {
    let id = otto_core::new_id();
    let now_ts = Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
         VALUES (?, 'root', 'hash', 'root', 1, 0, ?)",
    )
    .bind(&id)
    .bind(&now_ts)
    .execute(pool)
    .await
    .expect("seed user");
    User {
        id,
        username: "root".into(),
        display_name: "root".into(),
        is_root: true,
        disabled: false,
        created_at: Utc::now(),
    }
}

struct NullSecrets;
impl SecretStore for NullSecrets {
    fn put(&self, _k: &str, _v: &str) -> Result<()> {
        Ok(())
    }
    fn get(&self, _k: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn delete(&self, _k: &str) -> Result<()> {
        Ok(())
    }
}

/// What the k8s routes handed to `spawn_command`: (provider, spec, title, meta).
type SpawnRecord = (
    String,
    otto_pty::CommandSpec,
    String,
    Option<serde_json::Value>,
);

/// Records the spawn request instead of opening a PTY.
#[derive(Default)]
struct RecordingSpawner {
    last: Mutex<Option<SpawnRecord>>,
}
impl Spawner for RecordingSpawner {
    fn spawn_connection<'a>(
        &'a self,
        _ws: &'a Id,
        _user: &'a Id,
        _conn: &'a Connection,
        _spec: otto_pty::CommandSpec,
        _first: Option<String>,
        _title: Option<String>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async { Err(Error::Internal("not used".into())) })
    }
    fn spawn_command<'a>(
        &'a self,
        ws_id: &'a Id,
        user_id: &'a Id,
        provider: &'a str,
        spec: otto_pty::CommandSpec,
        title: String,
        meta: Option<serde_json::Value>,
    ) -> BoxFuture<'a, Result<Session>> {
        *self.last.lock().unwrap() = Some((provider.to_string(), spec, title.clone(), meta));
        let ws = ws_id.clone();
        let uid = user_id.clone();
        Box::pin(async move {
            Ok(Session {
                id: "sess-1".into(),
                workspace_id: ws,
                kind: otto_core::domain::SessionKind::Connection,
                provider: "k8s".into(),
                title,
                status: otto_core::domain::SessionStatus::Running,
                cwd: String::new(),
                provider_session_id: None,
                connection_id: None,
                created_by: uid,
                created_at: Utc::now(),
                last_active_at: Utc::now(),
                archived: false,
                meta: serde_json::Value::Null,
            })
        })
    }
}

/// In-memory `MonitorSink`: records every statement, answers queries with
/// canned rows keyed by a substring of the SQL.
#[derive(Default)]
struct FakeSink {
    available: bool,
    execs: Mutex<Vec<String>>,
    inserts: Mutex<Vec<(String, String)>>,
    /// `(sql substring, rows)` — first match wins; no match ⇒ empty.
    canned: Mutex<Vec<(String, Vec<serde_json::Value>)>>,
}

impl otto_k8s::MonitorSink for FakeSink {
    fn available(&self) -> bool {
        self.available
    }
    fn exec<'a>(&'a self, sql: &'a str) -> otto_k8s::BoxFut<'a, Result<()>> {
        self.execs.lock().unwrap().push(sql.to_string());
        Box::pin(async { Ok(()) })
    }
    fn insert_ndjson<'a>(&'a self, table: &'a str, ndjson: &'a str) -> otto_k8s::BoxFut<'a, Result<()>> {
        self.inserts.lock().unwrap().push((table.to_string(), ndjson.to_string()));
        Box::pin(async { Ok(()) })
    }
    fn query_rows<'a>(&'a self, sql: &'a str) -> otto_k8s::BoxFut<'a, Result<Vec<serde_json::Value>>> {
        let rows = self
            .canned
            .lock()
            .unwrap()
            .iter()
            .find(|(needle, _)| sql.contains(needle.as_str()))
            .map(|(_, rows)| rows.clone())
            .unwrap_or_default();
        Box::pin(async move { Ok(rows) })
    }
}

#[derive(Clone)]
struct TestCtx {
    pool: SqlitePool,
    secrets: Arc<dyn SecretStore>,
    events: broadcast::Sender<Event>,
    data_dir: Arc<tempfile::TempDir>,
    spawner: Arc<dyn Spawner>,
    recorder: Arc<RecordingSpawner>,
    sink: Arc<FakeSink>,
}

impl TestCtx {
    async fn new() -> (Self, User) {
        Self::with_sink(true).await
    }

    async fn with_sink(available: bool) -> (Self, User) {
        let _ = fake();
        let pool = mem_pool().await;
        let user = seed_root(&pool).await;
        let (events, _) = broadcast::channel(64);
        let recorder = Arc::new(RecordingSpawner::default());
        (
            Self {
                pool,
                secrets: Arc::new(NullSecrets),
                events,
                data_dir: Arc::new(tempfile::tempdir().unwrap()),
                spawner: recorder.clone(),
                recorder,
                sink: Arc::new(FakeSink {
                    available,
                    ..FakeSink::default()
                }),
            },
            user,
        )
    }
}

impl K8sCtx for TestCtx {
    fn pool(&self) -> SqlitePool {
        self.pool.clone()
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
    fn events(&self) -> &broadcast::Sender<Event> {
        &self.events
    }
    fn data_dir(&self) -> &Path {
        self.data_dir.path()
    }
    fn spawner(&self) -> &Arc<dyn Spawner> {
        &self.spawner
    }
    fn monitor_sink(&self) -> Option<Arc<dyn otto_k8s::MonitorSink>> {
        Some(self.sink.clone())
    }
}

async fn call(
    ctx: &TestCtx,
    user: &User,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value, String) {
    let app = otto_k8s::api_router::<TestCtx>()
        .layer(Extension(AuthUser(user.clone())))
        .with_state(ctx.clone());
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = app
        .oneshot(req.body(body).unwrap())
        .await
        .expect("router response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json, text)
}

fn kubeconfig_file(ctx: &TestCtx) -> String {
    let p = ctx.data_dir.path().join("user-kube.yaml");
    std::fs::write(&p, "apiVersion: v1\nkind: Config\n").unwrap();
    p.to_string_lossy().to_string()
}

async fn create_cluster(ctx: &TestCtx, user: &User) -> serde_json::Value {
    let (st, body, text) = call(
        ctx,
        user,
        "POST",
        "/k8s/clusters",
        Some(serde_json::json!({
            "name": "kind",
            "source": "kubeconfig",
            "kubeconfig_path": kubeconfig_file(ctx),
            "context_name": "kind-kind",
            "default_namespace": "shop",
            "environment": "staging",
            "color": "#0af"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{text}");
    body
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_reports_the_fake_kubectl() {
    let (ctx, user) = TestCtx::new().await;
    let (st, body, _) = call(&ctx, &user, "GET", "/k8s/status", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["kubectl"]["installed"], true);
    assert_eq!(body["kubectl"]["version"], "v1.30.0");
    assert_eq!(
        body["kubectl"]["path"].as_str().unwrap(),
        fake().bin.join("kubectl").to_string_lossy()
    );
    assert_eq!(body["install"]["kubectl"]["state"], "idle");
    assert_eq!(body["install"]["k9s"]["tool"], "k9s");
}

#[tokio::test]
async fn cluster_crud_roundtrip() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap().to_string();
    assert_eq!(c["source"], "kubeconfig");
    assert_eq!(c["context_name"], "kind-kind");
    assert_eq!(c["environment"], "staging");
    assert_eq!(c["color"], "#0af");
    assert_eq!(c["created_by"], user.id);

    // Validation: missing context / missing file / wrong source.
    let (st, _, _) = call(
        &ctx,
        &user,
        "POST",
        "/k8s/clusters",
        Some(serde_json::json!({"name": "x", "context_name": " "})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _, t) = call(&ctx, &user, "POST", "/k8s/clusters", Some(serde_json::json!({"name": "x", "context_name": "c", "kubeconfig_path": "/nope/kube.yaml"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{t}");
    let (st, _, _) = call(
        &ctx,
        &user,
        "POST",
        "/k8s/clusters",
        Some(serde_json::json!({"name": "x", "context_name": "c", "source": "eks"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    let (st, list, _) = call(&ctx, &user, "GET", "/k8s/clusters", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (st, got, _) = call(&ctx, &user, "GET", &format!("/k8s/clusters/{id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(got["name"], "kind");

    let (st, up, t) = call(
        &ctx,
        &user,
        "PATCH",
        &format!("/k8s/clusters/{id}"),
        Some(serde_json::json!({"name": "renamed", "default_namespace": "", "environment": "prod", "color": ""})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!(up["name"], "renamed");
    assert!(up.get("default_namespace").is_none());
    assert!(up.get("color").is_none());
    assert_eq!(up["environment"], "prod");
    assert_eq!(up["context_name"], "kind-kind", "untouched fields are kept");

    let (st, _, _) = call(&ctx, &user, "DELETE", &format!("/k8s/clusters/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _, _) = call(&ctx, &user, "GET", &format!("/k8s/clusters/{id}"), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    // The user's own kubeconfig file is never touched by a delete.
    assert!(ctx.data_dir.path().join("user-kube.yaml").is_file());
}

#[tokio::test]
async fn import_writes_a_private_kubeconfig_and_validates_it() {
    let (ctx, user) = TestCtx::new().await;
    let yaml = "apiVersion: v1\nkind: Config\ncurrent-context: kind-kind\n";
    let (st, c, t) = call(
        &ctx,
        &user,
        "POST",
        "/k8s/clusters/import",
        Some(serde_json::json!({"name": "pasted", "kubeconfig_yaml": yaml})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{t}");
    assert_eq!(c["source"], "imported");
    assert_eq!(
        c["context_name"], "kind-kind",
        "defaults to the file's current-context"
    );
    assert!(c.get("default_namespace").is_none());
    let path = PathBuf::from(c["kubeconfig_path"].as_str().unwrap());
    assert_eq!(
        path,
        ctx.data_dir
            .path()
            .join("kube")
            .join(format!("{}.yaml", c["id"].as_str().unwrap()))
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), yaml.trim());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    // kubectl was pointed at the new file, never at ~/.kube/config.
    let last = argv_log()
        .into_iter()
        .rev()
        .find(|l| l.contains("config view"))
        .unwrap();
    assert!(
        last.starts_with(&format!(
            "--kubeconfig {} config view -o json",
            path.display()
        )),
        "{last}"
    );

    // Explicit context + its namespace from the file.
    let (st, c2, t) = call(
        &ctx,
        &user,
        "POST",
        "/k8s/clusters/import",
        Some(serde_json::json!({"name": "prod", "kubeconfig_yaml": yaml, "context_name": "prod"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{t}");
    assert_eq!(c2["context_name"], "prod");
    assert_eq!(c2["default_namespace"], "shop");

    // Unknown context ⇒ 400 and the file is cleaned up.
    let before = std::fs::read_dir(ctx.data_dir.path().join("kube"))
        .unwrap()
        .count();
    let (st, _, t) = call(
        &ctx,
        &user,
        "POST",
        "/k8s/clusters/import",
        Some(serde_json::json!({"name": "bad", "kubeconfig_yaml": yaml, "context_name": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(t.contains("not found in the pasted kubeconfig"), "{t}");
    assert_eq!(
        std::fs::read_dir(ctx.data_dir.path().join("kube"))
            .unwrap()
            .count(),
        before
    );
    let (st, _, _) = call(
        &ctx,
        &user,
        "POST",
        "/k8s/clusters/import",
        Some(serde_json::json!({"name": "e", "kubeconfig_yaml": "  "})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // DELETE of an imported cluster removes the Otto-owned file.
    let (st, _, _) = call(
        &ctx,
        &user,
        "DELETE",
        &format!("/k8s/clusters/{}", c["id"].as_str().unwrap()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(!path.exists());
    // PATCHing the managed path is refused.
    let (st, _, _) = call(
        &ctx,
        &user,
        "PATCH",
        &format!("/k8s/clusters/{}", c2["id"].as_str().unwrap()),
        Some(serde_json::json!({"kubeconfig_path": "/tmp/x"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_endpoint_uses_base_args_and_reports_server_version() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, body, t) = call(
        &ctx,
        &user,
        "POST",
        &format!("/k8s/clusters/{id}/test"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["server_version"], "v1.29.5");
    assert!(body["message"].as_str().unwrap().contains("kind-kind"));
    // Other tests and capability probes append to the same process-wide log.
    // Assert this fixture's exact test command, independently of their order.
    let expected = format!(
        "--kubeconfig {} --context kind-kind version -o json --request-timeout=8s",
        c["kubeconfig_path"].as_str().unwrap()
    );
    assert!(argv_log().contains(&expected), "missing test command: {expected}");
    let (_, got, _) = call(&ctx, &user, "GET", &format!("/k8s/clusters/{id}"), None).await;
    assert!(
        got.get("last_used_at").is_some(),
        "a successful test bumps last_used_at"
    );
}

#[tokio::test]
async fn capabilities_probe_is_cached_until_refresh() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    // The argv log is shared by every test in this binary, so count only the
    // probes issued against THIS cluster's kubeconfig.
    let mine = format!("--kubeconfig {} ", c["kubeconfig_path"].as_str().unwrap());
    let probes = || {
        argv_log()
            .iter()
            .filter(|l| l.starts_with(&mine) && l.contains("api-resources"))
            .count()
    };
    let (st, caps, t) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/capabilities"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!(caps["server_version"], "v1.29.5");
    assert_eq!(caps["metrics_server"], true);
    assert_eq!(caps["argo_rollouts"], true);
    assert_eq!(caps["argocd"], true);
    assert_eq!(probes(), 1);
    let (_, again, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/capabilities"),
        None,
    )
    .await;
    assert_eq!(again["checked_at"], caps["checked_at"], "served from cache");
    assert_eq!(probes(), 1);
    let (_, fresh, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/capabilities?refresh=true"),
        None,
    )
    .await;
    assert_eq!(probes(), 2);
    assert_eq!(fresh["metrics_server"], true);
    let (_, got, _) = call(&ctx, &user, "GET", &format!("/k8s/clusters/{id}"), None).await;
    assert_eq!(
        got["capabilities"]["argocd"], true,
        "cache is visible on the cluster row"
    );
}

#[tokio::test]
async fn resources_pods_are_normalised_and_merged_with_metrics() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, body, t) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/resources?kind=pods&ns=shop&label=app%3Dweb"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!(body["kind"], "pods");
    assert_eq!(body["has_metrics"], true);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 7);
    let web = items
        .iter()
        .find(|r| r["name"] == "web-5d4c-abcde")
        .unwrap();
    assert_eq!(web["status"], "Running");
    assert_eq!(web["health"], "ok");
    assert_eq!(web["ready"], "2/2");
    assert_eq!(web["cpu"], 252);
    let crash = items.iter().find(|r| r["name"] == "worker-crash").unwrap();
    assert_eq!(crash["status"], "CrashLoopBackOff");
    assert_eq!(crash["health"], "bad");
    // Tests share one argv log (PATH is process-global), so match the exact
    // line for THIS cluster's kubeconfig instead of "the last get pods".
    let want = format!("--kubeconfig {} --context kind-kind --request-timeout 20s get pods -o json -n shop -l app=web", c["kubeconfig_path"].as_str().unwrap());
    assert!(argv_log().iter().any(|l| l == &want), "missing {want}");
    assert!(argv_log()
        .iter()
        .any(|l| l.ends_with("get --raw /apis/metrics.k8s.io/v1beta1/namespaces/shop/pods")));

    // Free-text filter + all namespaces.
    let (_, body, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/resources?kind=pods&q=crash"),
        None,
    )
    .await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert!(argv_log()
        .iter()
        .any(|l| l.ends_with("get pods -o json -A")));

    // Unknown kind ⇒ 400.
    let (st, _, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/resources?kind=widgets"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // Unknown cluster ⇒ 404.
    let (st, _, _) = call(
        &ctx,
        &user,
        "GET",
        "/k8s/clusters/nope/resources?kind=pods",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn forbidden_namespace_maps_to_403() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, body, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/resources?kind=pods&ns=forbidden"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "forbidden");
    assert!(
        body["message"].as_str().unwrap().contains("cluster RBAC:"),
        "{body}"
    );
}

#[tokio::test]
async fn secrets_rows_carry_no_values() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, _, text) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/resources?kind=secrets&ns=shop"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(text.contains("\"keys\":\"password,username\""), "{text}");
    assert!(
        !text.contains("aHVudGVyMg=="),
        "secret values must never be serialised"
    );
}

#[tokio::test]
async fn actions_run_kubectl_and_are_audited() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, body, t) = call(
        &ctx,
        &user,
        "POST",
        &format!("/k8s/clusters/{id}/actions"),
        Some(serde_json::json!({"action": "restart", "kind": "deployments", "ns": "shop", "name": "web"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{t}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["output"], "deployment.apps/web restarted");
    assert!(argv_log()
        .iter()
        .any(|l| l.ends_with("rollout restart deployments/web -n shop")));

    // Destructive without the typed confirm ⇒ 400, still audited.
    let (st, _, t) = call(
        &ctx,
        &user,
        "POST",
        &format!("/k8s/clusters/{id}/actions"),
        Some(serde_json::json!({"action": "delete_pod", "kind": "pods", "ns": "shop", "name": "web-1"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(t.contains("confirm_name"), "{t}");

    // rollout_status tolerates kubectl's non-zero "not finished yet".
    let (st, body, _) = call(
        &ctx,
        &user,
        "POST",
        &format!("/k8s/clusters/{id}/actions"),
        Some(serde_json::json!({"action": "rollout_status", "kind": "deployments", "ns": "shop", "name": "web"})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["ok"], false);
    assert!(body["output"]
        .as_str()
        .unwrap()
        .contains("Waiting for deployment"));

    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, detail FROM audit_log WHERE action LIKE 'k8s.action.%' ORDER BY ts, id",
    )
    .fetch_all(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0].0, "k8s.action.restart");
    let d: serde_json::Value = serde_json::from_str(&rows[0].1).unwrap();
    assert_eq!(d["ok"], true);
    assert_eq!(d["name"], "web");
    assert_eq!(rows[1].0, "k8s.action.delete_pod");
    let d: serde_json::Value = serde_json::from_str(&rows[1].1).unwrap();
    assert_eq!(d["ok"], false);
    assert!(d["error"]
        .as_str()
        .unwrap()
        .contains("confirmation required"));
}

#[tokio::test]
async fn exec_spawns_a_k8s_session_with_the_contract_argv() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, body, t) = call(
        &ctx,
        &user,
        "POST",
        &format!("/k8s/clusters/{id}/exec"),
        Some(serde_json::json!({"workspace_id": "ws1", "ns": "shop", "pod": "web-1", "container": "web"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{t}");
    assert_eq!(body["id"], "sess-1");
    let (provider, spec, title, meta) = ctx.recorder.last.lock().unwrap().clone().unwrap();
    assert_eq!(provider, "k8s");
    assert_eq!(title, "web-1 · shop");
    assert_eq!(meta.unwrap()["k8s"]["pod"], "web-1");
    assert_eq!(
        spec.program,
        fake().bin.join("kubectl").to_string_lossy().as_ref()
    );
    assert_eq!(
        spec.args,
        vec![
            "--kubeconfig",
            c["kubeconfig_path"].as_str().unwrap(),
            "--context",
            "kind-kind",
            "-n",
            "shop",
            "exec",
            "-it",
            "web-1",
            "-c",
            "web",
            "--",
            "sh",
            "-c",
            otto_k8s::sessions::DEFAULT_SHELL
        ]
    );
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE action = 'k8s.exec'")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // k9s is not on the fake PATH ⇒ 400 "not installed".
    let (st, _, t) = call(
        &ctx,
        &user,
        "POST",
        &format!("/k8s/clusters/{id}/k9s"),
        Some(serde_json::json!({"workspace_id": "ws1"})),
    )
    .await;
    if install_has_real_k9s() {
        assert_eq!(st, StatusCode::CREATED, "{t}");
    } else {
        assert_eq!(st, StatusCode::BAD_REQUEST, "{t}");
        assert!(t.contains("k9s not installed"), "{t}");
    }
}

/// The k9s branch depends on the developer machine (brew may have k9s).
fn install_has_real_k9s() -> bool {
    otto_k8s::install::locate(otto_k8s::Tool::K9s, Path::new("/nonexistent")).is_some()
}

#[tokio::test]
async fn logs_endpoint_returns_text_and_streams_when_following() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, _, text) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/pods/shop/web-1/logs?tail=50&container=web"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(text.trim(), "{}", "fake kubectl echoes {{}} for logs");
    assert!(argv_log()
        .iter()
        .any(|l| l.ends_with("--request-timeout 20s logs web-1 -n shop -c web --tail=50")));
    let (st, _, text) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/pods/shop/web-1/logs?follow=true&timestamps=true"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(text.trim(), "{}");
    let follow = argv_log()
        .into_iter()
        .rev()
        .find(|l| l.contains(" logs "))
        .unwrap();
    assert!(
        follow.ends_with("--context kind-kind logs web-1 -n shop --tail=500 --timestamps -f"),
        "{follow}"
    );
    assert!(
        !follow.contains("--request-timeout"),
        "streams carry no request timeout"
    );
}

// ---------------------------------------------------------------------------
// Monitoring routes
// ---------------------------------------------------------------------------

fn monitor_cfg(enabled: bool, interval: u32) -> serde_json::Value {
    serde_json::json!({
        "enabled": enabled,
        "interval_secs": interval,
        "namespaces": [],
        "probes": [{
            "name": "info", "port": 9000, "path": "/actuator/info", "format": "json",
            "mappings": [{"field": "memory_stats.sys", "metric": "mem_sys_bytes", "unit": "bytes_human"}]
        }, {
            "name": "health", "port": 9000, "path": "/actuator/health", "format": "health"
        }],
        "exclusions": [{"kind": "pod", "match": "old-*"}],
        "transport": "auto",
        "concurrency": 4,
        "retention_days": 7
    })
}

#[tokio::test]
async fn monitor_put_validates_and_persists() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();

    let (st, body, text) = call(&ctx, &user, "GET", &format!("/k8s/clusters/{id}/monitor"), None).await;
    assert_eq!(st, StatusCode::OK, "{text}");
    assert_eq!(body["config"]["enabled"], false);
    assert!(body["status"].is_null());
    assert!(body["presets"].as_array().unwrap().len() >= 3);

    let (st, body, _) = call(
        &ctx,
        &user,
        "PUT",
        &format!("/k8s/clusters/{id}/monitor"),
        Some(monitor_cfg(true, 5)),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid");
    assert!(body["message"].as_str().unwrap().contains("interval_secs"));

    let (st, body, text) = call(
        &ctx,
        &user,
        "PUT",
        &format!("/k8s/clusters/{id}/monitor"),
        Some(monitor_cfg(true, 60)),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{text}");
    assert_eq!(body["config"]["enabled"], true);
    assert_eq!(body["config"]["probes"][0]["mappings"][0]["unit"], "bytes_human");
    assert_eq!(body["config"]["exclusions"][0]["kind"], "pod");

    let (st, body, _) = call(&ctx, &user, "GET", &format!("/k8s/clusters/{id}/monitor"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["config"]["enabled"], true);
    assert_eq!(body["config"]["retention_days"], 7);
}

#[tokio::test]
async fn monitor_enabling_without_sink_is_409() {
    let (ctx, user) = TestCtx::with_sink(false).await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, body, _) = call(
        &ctx,
        &user,
        "PUT",
        &format!("/k8s/clusters/{id}/monitor"),
        Some(monitor_cfg(true, 60)),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict");
    // Saving it disabled is fine without ClickHouse.
    let (st, _, text) = call(
        &ctx,
        &user,
        "PUT",
        &format!("/k8s/clusters/{id}/monitor"),
        Some(monitor_cfg(false, 60)),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{text}");
}

#[tokio::test]
async fn monitor_overview_rejects_bad_window_and_lists_clusters() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let (st, _, _) = call(&ctx, &user, "GET", "/k8s/monitor/overview?window=1y", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, body, text) = call(&ctx, &user, "GET", "/k8s/monitor/overview?window=6h", None).await;
    assert_eq!(st, StatusCode::OK, "{text}");
    let rows = body.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["cluster"]["id"], c["id"]);
    assert_eq!(rows[0]["enabled"], false);
    assert_eq!(rows[0]["health"], "off");
}

#[tokio::test]
async fn monitor_series_rejects_bad_ident() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, body, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/monitor/series?metric=x;drop&window=1h"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid");
    let (st, body, text) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/monitor/series?metric=http_requests_total&workload=web&window=1h"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{text}");
    assert_eq!(body["kind"], "rate");
    assert!(body["points"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn monitor_run_now_sweeps_pods_and_writes_samples() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, _, text) = call(
        &ctx,
        &user,
        "PUT",
        &format!("/k8s/clusters/{id}/monitor"),
        Some(monitor_cfg(true, 60)),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{text}");

    let (st, body, text) = call(&ctx, &user, "POST", &format!("/k8s/clusters/{id}/monitor/run"), None).await;
    assert_eq!(st, StatusCode::OK, "{text}");
    assert_eq!(body["pods_seen"], 7, "{body}");
    assert!(body["last_ok_at"].is_string(), "{body}");
    assert_eq!(body["metrics_server"], "ok");
    // The fake kubectl answers every unknown argv with `{}` and exit 0, so the
    // proxy probe "succeeds" and the auto transport picks it.
    assert_eq!(body["transport_used"], "proxy");
    assert!(body.get("snapshot").is_none(), "snapshot is never serialised");

    let inserts = ctx.sink.inserts.lock().unwrap().clone();
    let samples: Vec<&str> = inserts
        .iter()
        .filter(|(t, _)| t == "k8s_samples")
        .flat_map(|(_, nd)| nd.lines())
        .collect();
    assert!(samples.iter().any(|l| l.contains("\"metric\":\"restarts_total\"")));
    assert!(samples.iter().any(|l| l.contains("\"metric\":\"cpu_millis\"")), "metrics-server merged");
    assert!(samples.iter().any(|l| l.contains("\"metric\":\"up\"")), "health probe");
    assert!(samples.iter().all(|l| l.contains(&format!("\"cluster_id\":\"{id}\""))));
    // Excluded pod (old-*) is swept but never scraped.
    assert!(!samples.iter().any(|l| l.contains("old-terminating") && l.contains("\"metric\":\"up\"")));
    let execs = ctx.sink.execs.lock().unwrap().clone();
    assert!(execs.iter().any(|q| q.contains("CREATE TABLE IF NOT EXISTS k8s_samples")));

    let (st, body, _) = call(&ctx, &user, "GET", &format!("/k8s/clusters/{id}/monitor"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["status"]["pods_seen"], 7);

    // Second run diffs against the stored snapshot: no restarts, no churn.
    let (st, _, _) = call(&ctx, &user, "POST", &format!("/k8s/clusters/{id}/monitor/run"), None).await;
    assert_eq!(st, StatusCode::OK);
    let inserts = ctx.sink.inserts.lock().unwrap().clone();
    let churn = inserts
        .iter()
        .filter(|(t, _)| t == "k8s_events")
        .flat_map(|(_, nd)| nd.lines())
        .filter(|l| l.contains("\"kind\":\"churn\"") || l.contains("\"kind\":\"restart\""))
        .count();
    assert_eq!(churn, 0);
}

#[tokio::test]
async fn monitor_test_probes_reports_per_probe_parse() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, _, _) = call(
        &ctx,
        &user,
        "PUT",
        &format!("/k8s/clusters/{id}/monitor"),
        Some(monitor_cfg(false, 60)),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, body, text) = call(
        &ctx,
        &user,
        "POST",
        &format!("/k8s/clusters/{id}/monitor/test"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{text}");
    assert_eq!(body["namespace"], "shop");
    assert_eq!(body["transport"], "proxy");
    assert_eq!(body["metrics_server"], "ok");
    let probes = body["probes"].as_array().unwrap();
    assert_eq!(probes.len(), 2);
    let info = probes.iter().find(|p| p["name"] == "info").unwrap();
    assert_eq!(info["ok"], true);
    assert_eq!(info["parse_errors"], 1, "mapping path missing in the `{{}}` body");
    let health = probes.iter().find(|p| p["name"] == "health").unwrap();
    assert!(health["samples"].as_array().unwrap().iter().any(|s| s["metric"] == "up" && s["value"] == 1.0));

    let (st, _, _) = call(
        &ctx,
        &user,
        "POST",
        &format!("/k8s/clusters/{id}/monitor/test"),
        Some(serde_json::json!({"pod": "nope"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn monitor_events_parse_detail_and_filter() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    ctx.sink.canned.lock().unwrap().push((
        "FROM k8s_events".into(),
        vec![serde_json::json!({
            "ts": "2026-09-05 08:00:00.000", "namespace": "shop", "workload": "frb", "pod": "frb-1", "container": "frb",
            "kind": "restart", "class": "oom", "reason": "OOMKilled", "exit_code": 137,
            "detail": "{\"planned_by\":\"\",\"prev_restarts\":0,\"next_restarts\":1}", "actor": ""
        })],
    ));
    let (st, body, text) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/monitor/events?window=24h&class=oom"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{text}");
    assert_eq!(body[0]["class"], "oom");
    assert_eq!(body[0]["detail"]["next_restarts"], 1, "detail JSON is parsed");
    let (st, _, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/monitor/events?class=bad%20class"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn monitor_health_digest_reports_disabled_then_stats() {
    let (ctx, user) = TestCtx::new().await;
    let c = create_cluster(&ctx, &user).await;
    let id = c["id"].as_str().unwrap();
    let (st, body, _) = call(&ctx, &user, "GET", &format!("/k8s/clusters/{id}/monitor/health"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["collector"]["enabled"], false);

    let (st, _, _) = call(
        &ctx,
        &user,
        "PUT",
        &format!("/k8s/clusters/{id}/monitor"),
        Some(monitor_cfg(true, 60)),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = call(&ctx, &user, "POST", &format!("/k8s/clusters/{id}/monitor/run"), None).await;
    assert_eq!(st, StatusCode::OK);
    ctx.sink.canned.lock().unwrap().push((
        "argMax(value, ts) AS mem".into(),
        vec![serde_json::json!({"cluster_id": id, "namespace": "shop", "workload": "web-5d4c-abcde", "pod": "web-5d4c-abcde", "mem": 900.0, "metric": "mem_sys_bytes"})],
    ));
    let (st, body, text) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/monitor/health?window=1h"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{text}");
    assert_eq!(body["collector"]["enabled"], true);
    assert_eq!(body["pods"]["total"], 7);
    assert!(body["thresholds"]["mem_pct"].is_number());
    assert!(body["restarts"]["oom"].is_array());

    let (st, body, text) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/monitor/workloads?window=1h"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{text}");
    let wls = body["workloads"].as_array().unwrap();
    let web = wls.iter().find(|w| w["workload"] == "web-5d4c-abcde").expect("web workload from snapshot");
    assert_eq!(web["mem_bytes"], 900.0);
    assert!(web["spark"]["mem"].is_array());
}

#[tokio::test]
async fn new_clusters_are_private_and_guessed_namespace_reads_are_denied() {
    let (ctx, owner) = TestCtx::new().await;
    let cluster = create_cluster(&ctx, &owner).await;
    let id = cluster["id"].as_str().unwrap();
    let mut outsider = owner.clone();
    outsider.id = otto_core::new_id();
    outsider.is_root = false;
    let (st, list, _) = call(&ctx, &outsider, "GET", "/k8s/clusters", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list, serde_json::json!([]), "ungranted cluster leaked");
    for suffix in [
        "",
        "/resources?kind=pods&ns=shop",
        "/metrics?ns=shop",
        "/pods/shop/web/logs",
    ] {
        let (st, _, _) = call(
            &ctx,
            &outsider,
            "GET",
            &format!("/k8s/clusters/{id}{suffix}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{suffix}");
    }
}

#[tokio::test]
async fn namespace_workloads_do_not_grant_secrets_metrics_exec_or_mutation() {
    use otto_core::access::{AccessActor, AccessRule, ResourceKind, RuleEffect, SubjectKind};
    let (ctx, owner) = TestCtx::new().await;
    let cluster = create_cluster(&ctx, &owner).await;
    let id = cluster["id"].as_str().unwrap().to_string();
    let mut user = owner.clone();
    user.id = otto_core::new_id();
    user.is_root = false;
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at) VALUES (?, 'limited', 'x', 'limited', 0, 0, ?)")
        .bind(&user.id).bind(Utc::now().to_rfc3339()).execute(&ctx.pool).await.unwrap();
    sqlx::query("INSERT INTO user_feature_grants (user_id, feature, capability) VALUES (?, 'kubernetes', 'view')")
        .bind(&user.id).execute(&ctx.pool).await.unwrap();
    let repo = otto_state::resource_access::ResourceAccessRepo::new(ctx.pool.clone());
    let mut policy = repo
        .get_policy(ResourceKind::K8sCluster, &id)
        .await
        .unwrap();
    for (operations, children) in [
        (vec!["discover"], None),
        (
            vec!["workloads_view", "resources_view", "logs"],
            Some(vec!["namespace:shop"]),
        ),
    ] {
        policy.rules.push(AccessRule {
            id: otto_core::new_id(),
            subject_kind: SubjectKind::User,
            subject_id: user.id.clone(),
            effect: RuleEffect::Allow,
            operations: operations.into_iter().map(str::to_string).collect(),
            children: children.map(|c| c.into_iter().map(str::to_string).collect()),
            credential_connection_id: None,
            grantable_operations: vec![],
        });
    }
    let actor = AccessActor {
        real_user_id: owner.id.clone(),
        effective_user_id: None,
    };
    repo.put_policy(&policy, policy.revision, &actor)
        .await
        .unwrap();
    let (st, _, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/resources?kind=deployments&ns=shop"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    for suffix in [
        "/resources?kind=pods",
        "/resources?kind=pods&ns=other",
        "/resources?kind=secrets&ns=shop",
        "/resource?kind=secret&ns=shop&name=secret",
        "/metrics?ns=shop",
        "/pods/other/web/logs",
    ] {
        let (st, _, _) = call(
            &ctx,
            &user,
            "GET",
            &format!("/k8s/clusters/{id}{suffix}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{suffix}");
    }
    for (suffix, body) in [
        (
            "/exec",
            serde_json::json!({"workspace_id":"ws", "ns":"shop", "pod":"web"}),
        ),
        (
            "/k9s",
            serde_json::json!({"workspace_id":"ws", "ns":"shop"}),
        ),
        (
            "/actions",
            serde_json::json!({"action":"restart", "kind":"deployment", "ns":"shop", "name":"web"}),
        ),
    ] {
        let (st, _, _) = call(
            &ctx,
            &user,
            "POST",
            &format!("/k8s/clusters/{id}{suffix}"),
            Some(body),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{suffix}");
    }
    let (st, namespaces, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/k8s/clusters/{id}/namespaces"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(namespaces["namespaces"].as_array().unwrap().len(), 1);
    assert_eq!(namespaces["namespaces"][0]["name"], "shop");
    // A body held open across policy revocation must stop before releasing
    // another log chunk, even though the original HTTP request was allowed.
    use futures_util::StreamExt;
    let body = Body::from_stream(futures_util::stream::iter(vec![
        Ok::<_, std::io::Error>("first"),
        Ok("second"),
    ]));
    let mut stream = otto_k8s::access::guard_body(
        body,
        ctx.pool.clone(),
        user.clone(),
        id.clone(),
        "shop".into(),
    )
    .into_data_stream();
    assert_eq!(stream.next().await.unwrap().unwrap(), "first");
    let mut revoked = repo
        .get_policy(ResourceKind::K8sCluster, &id)
        .await
        .unwrap();
    revoked.rules.retain(|rule| rule.subject_id != user.id);
    repo.put_policy(&revoked, revoked.revision, &actor)
        .await
        .unwrap();
    assert!(
        stream.next().await.is_none(),
        "revoked logs still emitted a chunk"
    );
    assert!(
        ctx.recorder.last.lock().unwrap().is_none(),
        "denied session must never spawn"
    );
}

#[tokio::test]
async fn delegated_cluster_admin_cannot_attach_repoint_or_read_hidden_configuration() {
    use otto_core::access::{AccessActor, AccessRule, ResourceKind, RuleEffect, SubjectKind};
    let (ctx, root) = TestCtx::new().await;
    let cluster = create_cluster(&ctx, &root).await;
    let id = cluster["id"].as_str().unwrap().to_string();
    let mut user = root.clone();
    user.is_root = false;
    user.id = otto_core::new_id();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at) VALUES (?, 'delegated', 'x', 'delegated', 0, 0, ?)").bind(&user.id).bind(Utc::now().to_rfc3339()).execute(&ctx.pool).await.unwrap();
    sqlx::query("INSERT INTO user_feature_grants (user_id, feature, capability) VALUES (?, 'kubernetes', 'admin')").bind(&user.id).execute(&ctx.pool).await.unwrap();
    let repo = otto_state::resource_access::ResourceAccessRepo::new(ctx.pool.clone());
    let actor = AccessActor {
        real_user_id: root.id.clone(),
        effective_user_id: None,
    };
    let mut policy = repo
        .get_policy(ResourceKind::K8sCluster, &id)
        .await
        .unwrap();
    policy.rules.push(AccessRule {
        id: otto_core::new_id(),
        subject_kind: SubjectKind::User,
        subject_id: user.id.clone(),
        effect: RuleEffect::Allow,
        operations: vec![
            "discover".into(),
            "configure".into(),
            "workloads_view".into(),
            "exec".into(),
        ],
        children: None,
        credential_connection_id: None,
        grantable_operations: vec![],
    });
    repo.put_policy(&policy, policy.revision, &actor)
        .await
        .unwrap();
    for patch in [
        serde_json::json!({"context_name":"hidden-admin"}),
        serde_json::json!({"kubeconfig_path":"/tmp/hidden.yaml"}),
    ] {
        let (st, _, _) = call(
            &ctx,
            &user,
            "PATCH",
            &format!("/k8s/clusters/{id}"),
            Some(patch),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }
    let (st, _, _) = call(
        &ctx,
        &user,
        "POST",
        "/k8s/clusters",
        Some(serde_json::json!({"name":"alias", "context_name":"hidden-admin"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = call(
        &ctx,
        &user,
        "POST",
        "/k8s/clusters/import",
        Some(serde_json::json!({"name":"alias", "kubeconfig_yaml":"apiVersion: v1"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, renamed, _) = call(&ctx, &user, "PATCH", &format!("/k8s/clusters/{id}"), Some(serde_json::json!({"name":"renamed", "context_name":cluster["context_name"], "kubeconfig_path":cluster["kubeconfig_path"]}))).await;
    assert_eq!(st, StatusCode::OK, "{renamed}");
    let mut policy = repo
        .get_policy(ResourceKind::K8sCluster, &id)
        .await
        .unwrap();
    policy
        .rules
        .last_mut()
        .unwrap()
        .operations
        .retain(|op| op != "configure");
    repo.put_policy(&policy, policy.revision, &actor)
        .await
        .unwrap();
    for path in [format!("/k8s/clusters/{id}"), "/k8s/clusters".into()] {
        let (st, body, _) = call(&ctx, &user, "GET", &path, None).await;
        assert_eq!(st, StatusCode::OK);
        let body = if body.is_array() { &body[0] } else { &body };
        assert!(body["kubeconfig_path"].is_null());
        assert_eq!(body["context_name"], "");
        assert!(body["default_namespace"].is_null());
        assert_eq!(body["params"], serde_json::json!({}));
    }
    let (st, probe, _) = call(&ctx, &user, "POST", &format!("/k8s/clusters/{id}/test"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(probe["message"], "Connection succeeded");
    sqlx::query("DELETE FROM user_feature_grants WHERE user_id = ?")
        .bind(&user.id)
        .execute(&ctx.pool)
        .await
        .unwrap();
    let (st, _, _) = call(
        &ctx,
        &user,
        "POST",
        &format!("/k8s/clusters/{id}/exec"),
        Some(serde_json::json!({"workspace_id":"w", "ns":"shop", "pod":"web"})),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(ctx.recorder.last.lock().unwrap().is_none());
}
