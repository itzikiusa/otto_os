//! The k8s-monitor SQL against a REAL embedded ClickHouse: DDL, the NDJSON
//! the collector writes, every dashboard query builder, and the health
//! aggregate. Skips when no `clickhouse` binary is installed (CI); on a dev
//! box it catches what the fake-sink router tests cannot — ClickHouse syntax
//! (the doubled `FORMAT JSONEachRow` that shipped once), type mismatches,
//! and Map-column access.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use otto_k8s::monitor::classify::{Class, Classified, ContainerSnap, PodSnap, Snapshot};
use otto_k8s::monitor::collector::{events_ndjson, samples_ndjson, status_samples};
use otto_k8s::monitor::parse::Sample;
use otto_k8s::monitor::{health, queries, schema};
use otto_k8s::{BoxFut, MonitorSink};
use otto_usage::{ClickHouse, UsageConfig, UsageEngine};
use serde_json::Value;

struct EngineSink(Arc<UsageEngine>);
impl MonitorSink for EngineSink {
    fn available(&self) -> bool {
        self.0.available()
    }
    fn exec<'a>(&'a self, sql: &'a str) -> BoxFut<'a, otto_core::Result<()>> {
        Box::pin(async move { self.0.exec_sql(sql).await })
    }
    fn insert_ndjson<'a>(&'a self, table: &'a str, ndjson: &'a str) -> BoxFut<'a, otto_core::Result<()>> {
        Box::pin(async move { self.0.insert_ndjson(table, ndjson).await })
    }
    fn query_rows<'a>(&'a self, sql: &'a str) -> BoxFut<'a, otto_core::Result<Vec<Value>>> {
        Box::pin(async move { self.0.query_rows(sql).await })
    }
}

fn pod(name: &str, version: &str) -> PodSnap {
    let mut containers = BTreeMap::new();
    containers.insert(
        "app".to_string(),
        ContainerSnap {
            restarts: 1,
            ..ContainerSnap::default()
        },
    );
    PodSnap {
        namespace: "shop".into(),
        name: name.into(),
        phase: "Running".into(),
        ready: true,
        workload_kind: "deployment".into(),
        workload: "web".into(),
        containers,
        mem_limit: 512 * 1024 * 1024,
        cpu_request: 100,
        created: "2026-09-01T00:00:00Z".into(),
        version: version.into(),
        ..PodSnap::default()
    }
}

fn s(metric: &str, labels: &[(&str, &str)], value: f64) -> Sample {
    Sample {
        metric: metric.into(),
        labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        value,
    }
}

#[tokio::test]
async fn every_query_builder_runs_on_a_real_clickhouse() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary on this machine");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let engine = UsageEngine::start(
        UsageConfig {
            enabled: true,
            retention_days: 7,
            metrics_interval_secs: 3600,
            clickhouse_path: None,
        },
        tmp.path().to_path_buf(),
    )
    .await;
    assert!(engine.wait_ready(Duration::from_secs(45)).await, "clickhouse did not come up");
    let sink = EngineSink(engine.clone());

    // DDL (idempotent) + TTL alter + purge statements.
    sink.exec(&schema::schema_sql(7)).await.unwrap();
    sink.exec(&schema::schema_sql(7)).await.unwrap();
    sink.exec(&schema::alter_ttl_sql(14)).await.unwrap();
    for q in schema::purge_cluster_sql("nobody", Some("2020-01-01")) {
        sink.exec(&q).await.unwrap();
    }

    // Samples exactly as the collector writes them: status series, a JSON
    // probe with a version label, prometheus counters + histogram buckets.
    let cid = "c1";
    let now = Utc::now();
    let p1 = pod("web-a", "1.0.0");
    let p2 = pod("web-b", "1.0.1");
    let mut version_a = BTreeMap::new();
    version_a.insert("version".to_string(), "1.0.0".to_string());
    let mut version_b = BTreeMap::new();
    version_b.insert("version".to_string(), "1.0.1".to_string());
    let mut nd = String::new();
    for (p, labels) in [(&p1, &version_a), (&p2, &version_b)] {
        nd.push_str(&samples_ndjson(cid, now, p, "app", &status_samples(p, now), &BTreeMap::new()));
        nd.push_str(&samples_ndjson(cid, now, p, "app", &[s("mem_sys_bytes", &[], 200.0)], labels));
        nd.push_str(&samples_ndjson(
            cid,
            now - chrono::Duration::minutes(5),
            p,
            "app",
            &[
                s("http_requests_total", &[("code", "200")], 100.0),
                s("http_requests_total", &[("code", "500")], 1.0),
                s("http_request_duration_seconds_bucket", &[("le", "0.1")], 50.0),
                s("http_request_duration_seconds_bucket", &[("le", "+Inf")], 60.0),
            ],
            labels,
        ));
        nd.push_str(&samples_ndjson(
            cid,
            now,
            p,
            "app",
            &[
                s("http_requests_total", &[("code", "200")], 400.0),
                s("http_requests_total", &[("code", "500")], 4.0),
                s("http_request_duration_seconds_bucket", &[("le", "0.1")], 300.0),
                s("http_request_duration_seconds_bucket", &[("le", "+Inf")], 360.0),
            ],
            labels,
        ));
    }
    sink.insert_ndjson("k8s_samples", &nd).await.unwrap();

    let ev = vec![
        Classified {
            kind: "restart",
            class: Class::Oom,
            namespace: "shop".into(),
            workload_kind: "deployment".into(),
            workload: "web".into(),
            pod: "web-a".into(),
            container: "app".into(),
            reason: "OOMKilled".into(),
            exit_code: 137,
            planned_by: String::new(),
            prev_restarts: 0,
            next_restarts: 1,
            at: now.to_rfc3339(),
        },
        Classified {
            kind: "version",
            class: Class::Planned,
            namespace: "shop".into(),
            workload_kind: "deployment".into(),
            workload: "web".into(),
            pod: String::new(),
            container: String::new(),
            reason: "1.0.0 → 1.0.1".into(),
            exit_code: 0,
            planned_by: "rollout".into(),
            prev_restarts: 0,
            next_restarts: 1,
            at: now.to_rfc3339(),
        },
    ];
    sink.insert_ndjson("k8s_events", &events_ndjson(cid, now, &ev, &[], &Snapshot::new()))
        .await
        .unwrap();

    let cids = vec![cid.to_string()];
    let win = chrono::Duration::hours(1);
    let secs = win.num_seconds();

    let mem = sink.query_rows(&queries::latest_memory_sql(&cids, None, 900)).await.unwrap();
    assert_eq!(mem.len(), 2, "{mem:?}");
    sink.query_rows(&queries::memory_between_sql(&cids, Some("shop"), secs + 900, secs - 900)).await.unwrap();

    let counts = sink.query_rows(&queries::restart_counts_sql(&cids, None, win)).await.unwrap();
    assert!(counts.iter().any(|r| r["class"] == "oom" && r["n"].as_u64() == Some(1)), "{counts:?}");

    let rates = sink.query_rows(&queries::request_rates_sql(&cids, None, secs, 0)).await.unwrap();
    assert_eq!(rates.len(), 1, "{rates:?}");
    let rps = rates[0]["rps"].as_f64().unwrap();
    assert!(rps > 0.0);
    assert!(rates[0]["err_rps"].as_f64().unwrap() > 0.0);
    sink.query_rows(&queries::request_rates_sql(&cids, None, secs + 86_400, secs)).await.unwrap();

    let buckets = sink.query_rows(&queries::latency_buckets_sql(&cids, None, secs, 0)).await.unwrap();
    let pairs: Vec<(String, f64)> = buckets
        .iter()
        .map(|r| (r["le"].as_str().unwrap().to_string(), r["delta"].as_f64().unwrap()))
        .collect();
    assert!(queries::p95_from_buckets(&pairs).is_some(), "{buckets:?}");
    sink.query_rows(&queries::latency_avg_sql(&cids, None, secs, 0)).await.unwrap();

    let versions = sink.query_rows(&queries::versions_sql(&cids, None, 900)).await.unwrap();
    assert_eq!(versions.len(), 2, "{versions:?}");

    let series = sink
        .query_rows(&queries::series_sql(cid, "http_requests_total", Some("web"), None, win, 60, true))
        .await
        .unwrap();
    assert!(!series.is_empty());
    let gauge = sink
        .query_rows(&queries::series_sql(cid, "mem_sys_bytes", None, Some("web-a"), win, 60, false))
        .await
        .unwrap();
    assert_eq!(gauge.len(), 1);
    let spark = sink
        .query_rows(&queries::workload_spark_sql(cid, None, &queries::MEMORY_GAUGES, win, 60, false))
        .await
        .unwrap();
    assert!(!spark.is_empty());

    let events = sink.query_rows(&queries::events_sql(cid, win, None, None, 100)).await.unwrap();
    assert_eq!(events.len(), 2, "{events:?}");
    let only_version = sink.query_rows(&queries::events_sql(cid, win, Some("version"), None, 100)).await.unwrap();
    assert_eq!(only_version.len(), 1);
    assert_eq!(only_version[0]["reason"], "1.0.0 → 1.0.1");

    // The aggregate the workloads tab + digest are built from.
    let mut snap = Snapshot::new();
    snap.insert("shop/web-a".into(), p1);
    snap.insert("shop/web-b".into(), p2);
    let stats = health::workload_stats(&sink, cid, &snap, None, win).await.unwrap();
    assert_eq!(stats.len(), 1);
    let web = &stats[0];
    assert_eq!(web.pods, 2);
    assert_eq!(web.mem_bytes, 400.0);
    assert_eq!(web.restarts.oom, 1);
    assert!(web.rps > 0.0);
    assert!(web.err_pct > 0.0);
    assert_eq!(web.latency_kind, "p95");
    assert_eq!(web.versions.len(), 2, "drift: {:?}", web.versions);

    engine.shutdown().await;
}
