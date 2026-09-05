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

#[derive(Clone)]
struct TestCtx {
    pool: SqlitePool,
    secrets: Arc<dyn SecretStore>,
    events: broadcast::Sender<Event>,
    data_dir: Arc<tempfile::TempDir>,
    spawner: Arc<dyn Spawner>,
    recorder: Arc<RecordingSpawner>,
}

impl TestCtx {
    async fn new() -> (Self, User) {
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
        None
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
    let last = argv_log()
        .into_iter()
        .rev()
        .find(|l| l.contains(" version "))
        .unwrap();
    assert_eq!(
        last,
        format!(
            "--kubeconfig {} --context kind-kind version -o json --request-timeout=8s",
            c["kubeconfig_path"].as_str().unwrap()
        )
    );
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
