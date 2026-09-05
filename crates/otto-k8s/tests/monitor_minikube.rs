//! Gated end-to-end check of one monitoring cycle against a REAL cluster —
//! the current kubeconfig context (intended for minikube). Runs only with
//! `OTTO_K8S_E2E=1`; otherwise it is a no-op so `cargo test` stays hermetic.
//!
//! Env: `OTTO_K8S_E2E_NS` (namespace, default `default`), `OTTO_K8S_E2E_PORT`
//! + `OTTO_K8S_E2E_PATH` (optional health probe on every pod, e.g. `8080` +
//! `/`). Samples land in an in-memory sink; nothing is written anywhere.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use otto_connections::Spawner;
use otto_core::auth::BoxFuture;
use otto_core::domain::{Connection, Session};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_k8s::monitor::classify::Snapshot;
use otto_k8s::monitor::collector::run_cycle;
use otto_k8s::monitor::probes::{MonitorConfig, Probe, ProbeFormat};
use otto_k8s::{BoxFut, K8sCtx, MonitorSink};
use otto_state::{K8sClustersRepo, NewK8sCluster, SqlitePool};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tokio::sync::broadcast;

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

struct NoSpawn;
impl Spawner for NoSpawn {
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
        _ws: &'a Id,
        _user: &'a Id,
        _provider: &'a str,
        _spec: otto_pty::CommandSpec,
        _title: String,
        _meta: Option<serde_json::Value>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async { Err(Error::Internal("not used".into())) })
    }
}

#[derive(Default)]
struct MemSink {
    inserts: Mutex<Vec<(String, String)>>,
}
impl MonitorSink for MemSink {
    fn available(&self) -> bool {
        true
    }
    fn exec<'a>(&'a self, _sql: &'a str) -> BoxFut<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn insert_ndjson<'a>(&'a self, table: &'a str, ndjson: &'a str) -> BoxFut<'a, Result<()>> {
        self.inserts.lock().unwrap().push((table.to_string(), ndjson.to_string()));
        Box::pin(async { Ok(()) })
    }
    fn query_rows<'a>(&'a self, _sql: &'a str) -> BoxFut<'a, Result<Vec<serde_json::Value>>> {
        Box::pin(async { Ok(vec![]) })
    }
}

#[derive(Clone)]
struct Ctx {
    pool: SqlitePool,
    secrets: Arc<dyn SecretStore>,
    events: broadcast::Sender<Event>,
    data_dir: Arc<tempfile::TempDir>,
    spawner: Arc<dyn Spawner>,
    sink: Arc<MemSink>,
}
impl K8sCtx for Ctx {
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
    fn monitor_sink(&self) -> Option<Arc<dyn MonitorSink>> {
        Some(self.sink.clone())
    }
}

#[tokio::test]
async fn one_cycle_against_the_current_context() {
    if std::env::var("OTTO_K8S_E2E").ok().as_deref() != Some("1") {
        eprintln!("OTTO_K8S_E2E is not set; skipping the minikube cycle test");
        return;
    }
    let context = String::from_utf8(
        Command::new("kubectl")
            .args(["config", "current-context"])
            .output()
            .expect("kubectl on PATH")
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    assert!(!context.is_empty(), "no current kubeconfig context");
    let ns = std::env::var("OTTO_K8S_E2E_NS").unwrap_or_else(|_| "default".into());

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().in_memory(true).foreign_keys(true))
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations").run(&pool).await.unwrap();
    let (events, _) = broadcast::channel(16);
    let ctx = Ctx {
        pool: pool.clone(),
        secrets: Arc::new(NullSecrets),
        events,
        data_dir: Arc::new(tempfile::tempdir().unwrap()),
        spawner: Arc::new(NoSpawn),
        sink: Arc::new(MemSink::default()),
    };
    let cluster = K8sClustersRepo::new(pool)
        .create(NewK8sCluster {
            id: otto_core::new_id(),
            name: "e2e".into(),
            source: otto_state::K8sClusterSource::Kubeconfig,
            kubeconfig_path: None,
            context_name: context,
            default_namespace: Some(ns.clone()),
            aws_account_id: None,
            environment: otto_core::domain::Environment::Dev,
            color: None,
            params: serde_json::json!({}),
            created_by: None,
        })
        .await
        .unwrap();

    let mut cfg = MonitorConfig {
        enabled: true,
        ..MonitorConfig::default()
    };
    if let Ok(port) = std::env::var("OTTO_K8S_E2E_PORT") {
        cfg.probes.push(Probe {
            name: "health".into(),
            port: port.parse().ok(),
            path: std::env::var("OTTO_K8S_E2E_PATH").unwrap_or_else(|_| "/".into()),
            format: ProbeFormat::Health,
            mappings: vec![],
            include: vec![],
            exclude: vec![],
            timeout_ms: 3000,
        });
    }

    let out = run_cycle(&ctx, &cluster, &cfg, &Snapshot::new(), None, ctx.sink.as_ref()).await;
    assert!(!out.unreachable, "cluster unreachable: {}", out.status.last_error);
    assert!(out.status.pods_seen > 0, "no pods in namespace {ns}");
    let inserts = ctx.sink.inserts.lock().unwrap().clone();
    let samples: Vec<&str> = inserts
        .iter()
        .filter(|(t, _)| t == "k8s_samples")
        .flat_map(|(_, nd)| nd.lines())
        .collect();
    assert!(samples.iter().any(|l| l.contains("\"metric\":\"restarts_total\"")));
    if !cfg.probes.is_empty() {
        assert!(
            samples.iter().any(|l| l.contains("\"metric\":\"up\"")),
            "health probe produced no samples: {}",
            out.status.last_error
        );
    }
    eprintln!(
        "cycle ok: pods_seen={} scraped={} failed={} transport={} metrics_server={} ms={}",
        out.status.pods_seen,
        out.status.pods_scraped,
        out.status.pods_failed,
        out.status.transport_used,
        out.status.metrics_server,
        out.status.cycle_ms
    );
}
