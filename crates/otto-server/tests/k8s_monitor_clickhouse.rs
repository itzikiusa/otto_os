//! The k8s-monitor SQL against a REAL embedded ClickHouse: DDL, the NDJSON
//! the collector writes, every dashboard query builder, and the health
//! aggregate. Skips when no `clickhouse` binary is installed (CI); on a dev
//! box it catches what the fake-sink router tests cannot — ClickHouse syntax
//! (the doubled `FORMAT JSONEachRow` that shipped once), type mismatches,
//! and Map-column access — plus the rollup tiers: the one-time backfill of
//! an old raw-only install, and that every dashboard query answers the same
//! from a rollup as from raw rows over the same range.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use otto_k8s::monitor::classify::{Class, Classified, ContainerSnap, PodSnap, Snapshot};
use otto_k8s::monitor::collector::{events_ndjson, samples_ndjson, status_samples};
use otto_k8s::monitor::parse::Sample;
use otto_k8s::monitor::queries::{Span, Tier};
use otto_k8s::monitor::{fleet, health, queries, schema};
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
    fn insert_ndjson<'a>(
        &'a self,
        table: &'a str,
        ndjson: &'a str,
    ) -> BoxFut<'a, otto_core::Result<()>> {
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
        labels: labels
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
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
    assert!(
        engine.wait_ready(Duration::from_secs(45)).await,
        "clickhouse did not come up"
    );
    let sink = EngineSink(engine.clone());

    // DDL + views (idempotent) + TTL alter + purge statements.
    schema::ensure(&sink, 7).await.unwrap();
    schema::ensure(&sink, 7).await.unwrap();
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
        nd.push_str(&samples_ndjson(
            cid,
            now,
            p,
            "app",
            &status_samples(p, now),
            &BTreeMap::new(),
        ));
        nd.push_str(&samples_ndjson(
            cid,
            now,
            p,
            "app",
            &[s("mem_sys_bytes", &[], 200.0)],
            labels,
        ));
        nd.push_str(&samples_ndjson(
            cid,
            now - chrono::Duration::minutes(5),
            p,
            "app",
            &[
                s("http_requests_total", &[("code", "200")], 100.0),
                s("http_requests_total", &[("code", "500")], 1.0),
                s(
                    "http_request_duration_seconds_bucket",
                    &[("le", "0.1")],
                    50.0,
                ),
                s(
                    "http_request_duration_seconds_bucket",
                    &[("le", "+Inf")],
                    60.0,
                ),
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
                s(
                    "http_request_duration_seconds_bucket",
                    &[("le", "0.1")],
                    300.0,
                ),
                s(
                    "http_request_duration_seconds_bucket",
                    &[("le", "+Inf")],
                    360.0,
                ),
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
    sink.insert_ndjson(
        "k8s_events",
        &events_ndjson(cid, now, &ev, &[], &Snapshot::new()),
    )
    .await
    .unwrap();

    let cids = vec![cid.to_string()];
    let win = chrono::Duration::hours(1);
    let secs = win.num_seconds();

    let mem = sink
        .query_rows(&queries::latest_memory_sql(&cids, None, 900))
        .await
        .unwrap();
    assert_eq!(mem.len(), 2, "{mem:?}");
    sink.query_rows(&queries::memory_between_sql(
        &cids,
        Some("shop"),
        secs + 900,
        secs - 900,
    ))
    .await
    .unwrap();

    let counts = sink
        .query_rows(&queries::restart_counts_sql(&cids, None, win))
        .await
        .unwrap();
    assert!(
        counts
            .iter()
            .any(|r| r["class"] == "oom" && r["n"].as_u64() == Some(1)),
        "{counts:?}"
    );

    let rates = sink
        .query_rows(&queries::request_rates_sql(&cids, None, secs, 0))
        .await
        .unwrap();
    assert_eq!(rates.len(), 1, "{rates:?}");
    let rps = rates[0]["rps"].as_f64().unwrap();
    assert!(rps > 0.0);
    assert!(rates[0]["err_rps"].as_f64().unwrap() > 0.0);
    sink.query_rows(&queries::request_rates_sql(
        &cids,
        None,
        secs + 86_400,
        secs,
    ))
    .await
    .unwrap();

    let buckets = sink
        .query_rows(&queries::latency_buckets_sql(&cids, None, secs, 0))
        .await
        .unwrap();
    let pairs: Vec<(String, f64)> = buckets
        .iter()
        .map(|r| {
            (
                r["le"].as_str().unwrap().to_string(),
                r["delta"].as_f64().unwrap(),
            )
        })
        .collect();
    assert!(queries::p95_from_buckets(&pairs).is_some(), "{buckets:?}");
    sink.query_rows(&queries::latency_avg_sql(&cids, None, secs, 0))
        .await
        .unwrap();

    let versions = sink
        .query_rows(&queries::versions_sql(&cids, None, 900))
        .await
        .unwrap();
    assert_eq!(versions.len(), 2, "{versions:?}");

    let series = sink
        .query_rows(&queries::series_sql(
            cid,
            "http_requests_total",
            Some("web"),
            None,
            win,
            60,
            true,
        ))
        .await
        .unwrap();
    assert!(!series.is_empty());
    let gauge = sink
        .query_rows(&queries::series_sql(
            cid,
            "mem_sys_bytes",
            None,
            Some("web-a"),
            win,
            60,
            false,
        ))
        .await
        .unwrap();
    assert_eq!(gauge.len(), 1);
    let spark = sink
        .query_rows(&queries::workload_spark_sql(
            cid,
            None,
            &queries::MEMORY_GAUGES,
            win,
            60,
            false,
        ))
        .await
        .unwrap();
    assert!(!spark.is_empty());

    let events = sink
        .query_rows(&queries::events_sql(cid, win, None, None, 100))
        .await
        .unwrap();
    assert_eq!(events.len(), 2, "{events:?}");
    let only_version = sink
        .query_rows(&queries::events_sql(cid, win, Some("version"), None, 100))
        .await
        .unwrap();
    assert_eq!(only_version.len(), 1);
    assert_eq!(only_version[0]["reason"], "1.0.0 → 1.0.1");

    // The aggregate the workloads tab + digest are built from.
    let mut snap = Snapshot::new();
    snap.insert("shop/web-a".into(), p1);
    snap.insert("shop/web-b".into(), p2);
    let stats = health::workload_stats(&sink, cid, &snap, None, win)
        .await
        .unwrap();
    assert_eq!(stats.len(), 1);
    let web = &stats[0];
    assert_eq!(web.pods, 2);
    assert_eq!(web.mem_bytes, 400.0);
    assert_eq!(web.restarts.oom, 1);
    assert!(web.rps > 0.0);
    assert!(web.err_pct > 0.0);
    assert_eq!(web.latency_kind, "p95");
    assert_eq!(web.versions.len(), 2, "drift: {:?}", web.versions);

    // The same stats with the raw table emptied: everything above came from
    // the rollups + last-value table (a 1 h window never reads raw).
    sink.exec("TRUNCATE TABLE k8s_samples").await.unwrap();
    let from_rollups = health::workload_stats(&sink, cid, &snap, None, win)
        .await
        .unwrap();
    assert_eq!(from_rollups[0].mem_bytes, 400.0);
    assert_eq!(from_rollups[0].rps, web.rps);
    assert_eq!(from_rollups[0].latency_ms, web.latency_ms);
    assert_eq!(from_rollups[0].versions.len(), 2);

    engine.shutdown().await;
}

async fn start_engine(tmp: &tempfile::TempDir) -> Option<Arc<UsageEngine>> {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary on this machine");
        return None;
    }
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
    assert!(
        engine.wait_ready(Duration::from_secs(45)).await,
        "clickhouse did not come up"
    );
    Some(engine)
}

async fn count(sink: &EngineSink, sql: &str) -> u64 {
    sink.query_rows(sql).await.unwrap()[0]["n"]
        .as_u64()
        .unwrap()
}

/// The schema an install had before the rollups: raw + events only.
const OLD_RAW_DDL: &str = "CREATE TABLE IF NOT EXISTS k8s_samples (
    ts DateTime64(3), sample_date Date DEFAULT toDate(ts), cluster_id LowCardinality(String),
    namespace LowCardinality(String), workload_kind LowCardinality(String), workload LowCardinality(String),
    pod String, container LowCardinality(String), metric LowCardinality(String),
    labels Map(LowCardinality(String), String), value Float64
) ENGINE = MergeTree PARTITION BY (cluster_id, sample_date)
ORDER BY (cluster_id, namespace, workload, metric, pod, ts) TTL sample_date + INTERVAL 14 DAY";

#[tokio::test]
async fn old_raw_install_is_backfilled_once_then_fed_by_views() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(engine) = start_engine(&tmp).await else {
        return;
    };
    let sink = EngineSink(engine.clone());
    sink.exec(OLD_RAW_DDL).await.unwrap();

    // Raw history: today, yesterday and 5 days ago (two clusters).
    let p = pod("web-a", "1.0.0");
    let now = Utc::now();
    let mut nd = String::new();
    for (cid, age) in [("c1", 0i64), ("c1", 1), ("c1", 5), ("c2", 0)] {
        let at = now - chrono::Duration::days(age);
        for k in 0..3 {
            nd.push_str(&samples_ndjson(
                cid,
                at - chrono::Duration::seconds(20 * k),
                &p,
                "app",
                &[s("mem_working_set_bytes", &[], 100.0 + k as f64)],
                &BTreeMap::new(),
            ));
        }
    }
    sink.insert_ndjson("k8s_samples", &nd).await.unwrap();
    assert_eq!(
        count(&sink, "SELECT count() AS n FROM k8s_samples").await,
        12
    );

    schema::ensure(&sink, 14).await.unwrap();
    // Every raw row is in the hour tier exactly once; the 5-day-old day is
    // past the 1-minute keep but inside the 5-minute one.
    let h1 = "SELECT sum(n) AS n FROM k8s_samples_1h";
    assert_eq!(count(&sink, h1).await, 12);
    assert_eq!(
        count(&sink, "SELECT sum(n) AS n FROM k8s_samples_5m").await,
        12
    );
    assert_eq!(
        count(&sink, "SELECT sum(n) AS n FROM k8s_samples_1m").await,
        9
    );
    assert_eq!(
        count(&sink, "SELECT sum(n) AS n FROM k8s_pods_1h").await,
        12
    );
    assert_eq!(
        count(&sink, "SELECT count() AS n FROM k8s_latest FINAL").await,
        2,
        "one series per cluster"
    );
    // Raw days past the raw keep were dropped right after the backfill.
    assert_eq!(
        count(&sink, "SELECT count() AS n FROM k8s_samples").await,
        9
    );
    let views = count(&sink, &schema::views_present_sql()).await;
    assert_eq!(views as usize, schema::views().len());

    // Idempotent: a second start neither re-runs the backfill nor doubles.
    schema::ensure(&sink, 14).await.unwrap();
    assert_eq!(count(&sink, h1).await, 12);

    // New inserts flow through the views.
    sink.insert_ndjson(
        "k8s_samples",
        &samples_ndjson(
            "c1",
            now + chrono::Duration::seconds(1),
            &p,
            "app",
            &[s("mem_working_set_bytes", &[], 500.0)],
            &BTreeMap::new(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(count(&sink, h1).await, 13);
    let latest = sink
        .query_rows(&queries::latest_memory_sql(&["c1".into()], None, 900))
        .await
        .unwrap();
    assert_eq!(latest[0]["mem"].as_f64(), Some(500.0));

    // TTL alters and purges cover the new tables without rewriting parts.
    sink.exec(&schema::alter_ttl_sql(30)).await.unwrap();
    for q in schema::purge_cluster_sql("c2", None) {
        sink.exec(&q).await.unwrap();
    }
    assert_eq!(
        count(
            &sink,
            "SELECT count() AS n FROM k8s_samples_1h WHERE cluster_id = 'c2'"
        )
        .await,
        0
    );
    engine.shutdown().await;
}

/// Compare two JSONEachRow result sets: same rows (order-insensitive),
/// numbers within a relative 1e-9.
fn assert_same_rows(what: &str, a: &[Value], b: &[Value]) {
    fn key(v: &Value) -> String {
        let mut parts: Vec<String> = v
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, x)| !x.is_number())
            .map(|(k, x)| format!("{k}={x}"))
            .collect();
        parts.sort();
        parts.join("|")
    }
    assert_eq!(
        a.len(),
        b.len(),
        "{what}: row count\nraw: {a:?}\nrollup: {b:?}"
    );
    let mut a: Vec<&Value> = a.iter().collect();
    let mut b: Vec<&Value> = b.iter().collect();
    a.sort_by_key(|v| key(v));
    b.sort_by_key(|v| key(v));
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(key(x), key(y), "{what}: row identity");
        for (k, xv) in x.as_object().unwrap() {
            let yv = &y[k];
            if let (Some(p), Some(q)) = (xv.as_f64(), yv.as_f64()) {
                let tol = 1e-9 * p.abs().max(q.abs()).max(1.0);
                assert!(
                    (p - q).abs() <= tol,
                    "{what}: {k} raw {p} vs rollup {q} ({x} / {y})"
                );
            }
        }
    }
}

#[tokio::test]
async fn every_rollup_tier_matches_raw_on_the_same_range() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(engine) = start_engine(&tmp).await else {
        return;
    };
    let sink = EngineSink(engine.clone());
    schema::ensure(&sink, 14).await.unwrap();

    // Fixture: 3 pods × 2 workloads, 20 s samples over the last ~3 hours,
    // starting on an hour boundary so one range is aligned for every tier.
    let now = Utc::now().timestamp();
    let start = (now / 3600 - 3) * 3600;
    let pods = [("web", "web-a"), ("web", "web-b"), ("api", "api-a")];
    let mut nd = String::new();
    let mut t = start;
    let mut i = 0u32;
    while t <= now {
        let at = chrono::DateTime::from_timestamp(t, 0).unwrap();
        for (j, (wl, name)) in pods.iter().enumerate() {
            let mut p = pod(name, "1.0.0");
            p.workload = (*wl).into();
            let j = j as f64 + 1.0;
            let n = f64::from(i);
            let gauge = if *name == "api-a" {
                "mem_sys_bytes"
            } else {
                "mem_working_set_bytes"
            };
            nd.push_str(&samples_ndjson(
                "c1",
                at,
                &p,
                "app",
                &[
                    s(gauge, &[], 1000.0 * j + f64::from(i % 17) * 3.0),
                    s("http_requests_total", &[("code", "200")], n * 7.0 * j),
                    s(
                        "http_requests_total",
                        &[("code", "503")],
                        (n / 9.0).floor() * j,
                    ),
                    s(
                        "http_requests_total",
                        &[("code", "200"), ("method", "GET"), ("path", "/api/x")],
                        n * 2.0,
                    ),
                    s(
                        "http_request_duration_seconds_bucket",
                        &[("le", "0.1")],
                        n * 5.0,
                    ),
                    s(
                        "http_request_duration_seconds_bucket",
                        &[("le", "0.5")],
                        n * 6.0 * j,
                    ),
                    s(
                        "http_request_duration_seconds_bucket",
                        &[("le", "+Inf")],
                        n * 7.0 * j,
                    ),
                    s("http_request_duration_seconds_sum", &[], n * 0.25 * j),
                    s("http_request_duration_seconds_count", &[], n * 7.0 * j),
                ],
                &BTreeMap::new(),
            ));
        }
        t += 20;
        i += 1;
    }
    sink.insert_ndjson("k8s_samples", &nd).await.unwrap();

    let cids = vec!["c1".to_string()];
    let all = fleet::FleetFilter::parse(Some("c1"), None, None, None).unwrap();
    let window = chrono::Duration::seconds(now - start);
    for tier in [
        Tier::Rollup(schema::ROLLUP_1M),
        Tier::Rollup(schema::ROLLUP_5M),
        Tier::Rollup(schema::ROLLUP_1H),
    ] {
        let raw = Span::between(Tier::Raw, start, None, now);
        let roll = Span::between(tier, start, None, now);
        let step = tier.grain().max(60) as u32 * 2;
        // A baseline-shaped range with an upper bound too.
        let raw_b = Span::between(Tier::Raw, start, Some(start + 7200), now);
        let roll_b = Span::between(tier, start, Some(start + 7200), now);
        let cases: Vec<(&str, String, String)> = vec![
            (
                "rates",
                queries::request_rates_in(&raw, &cids, None),
                queries::request_rates_in(&roll, &cids, None),
            ),
            (
                "rates-bounded",
                queries::request_rates_in(&raw_b, &cids, Some("shop")),
                queries::request_rates_in(&roll_b, &cids, Some("shop")),
            ),
            (
                "buckets",
                queries::latency_buckets_in(&raw, &cids, None),
                queries::latency_buckets_in(&roll, &cids, None),
            ),
            (
                "latency-avg",
                queries::latency_avg_in(&raw, &cids, None),
                queries::latency_avg_in(&roll, &cids, None),
            ),
            (
                "memory-between",
                queries::memory_between_in(&raw_b, &cids, None),
                queries::memory_between_in(&roll_b, &cids, None),
            ),
            (
                "series-counter",
                queries::series_in(
                    &raw,
                    "c1",
                    "http_requests_total",
                    Some("web"),
                    None,
                    step,
                    true,
                ),
                queries::series_in(
                    &roll,
                    "c1",
                    "http_requests_total",
                    Some("web"),
                    None,
                    step,
                    true,
                ),
            ),
            (
                "series-gauge",
                queries::series_in(
                    &raw,
                    "c1",
                    "mem_working_set_bytes",
                    None,
                    Some("web-a"),
                    step,
                    false,
                ),
                queries::series_in(
                    &roll,
                    "c1",
                    "mem_working_set_bytes",
                    None,
                    Some("web-a"),
                    step,
                    false,
                ),
            ),
            (
                "spark",
                queries::workload_spark_in(
                    &raw,
                    "c1",
                    None,
                    &queries::REQUEST_COUNTERS,
                    step,
                    true,
                ),
                queries::workload_spark_in(
                    &roll,
                    "c1",
                    None,
                    &queries::REQUEST_COUNTERS,
                    step,
                    true,
                ),
            ),
            (
                "fleet-memory",
                fleet::memory_in(&raw, &all, fleet::Group::Pod),
                fleet::memory_in(&roll, &all, fleet::Group::Pod),
            ),
            (
                "fleet-rates",
                fleet::rates_in(&raw, &all, fleet::Group::Workload),
                fleet::rates_in(&roll, &all, fleet::Group::Workload),
            ),
            (
                "fleet-buckets",
                fleet::latency_buckets_in(&raw, &all, fleet::Group::Pod),
                fleet::latency_buckets_in(&roll, &all, fleet::Group::Pod),
            ),
            (
                "fleet-latency-avg",
                fleet::latency_avg_in(&raw, &all, fleet::Group::Workload),
                fleet::latency_avg_in(&roll, &all, fleet::Group::Workload),
            ),
            (
                "fleet-requests",
                fleet::requests_in(&raw, &all),
                fleet::requests_in(&roll, &all),
            ),
        ];
        let mut cases = cases;
        for m in [
            fleet::SeriesMetric::Mem,
            fleet::SeriesMetric::Rps,
            fleet::SeriesMetric::Err,
            fleet::SeriesMetric::Latency,
        ] {
            cases.push((
                m.as_str(),
                fleet::series_in(&raw, window, &all, m, fleet::SeriesBy::Workload, step),
                fleet::series_in(&roll, window, &all, m, fleet::SeriesBy::Workload, step),
            ));
        }
        for (what, raw_sql, roll_sql) in cases {
            assert!(
                roll_sql.contains(tier.table()),
                "{what} reads {}",
                tier.table()
            );
            assert!(
                !roll_sql.contains("FROM k8s_samples WHERE"),
                "{what} must not read raw"
            );
            let a = sink.query_rows(&raw_sql).await.unwrap();
            let b = sink.query_rows(&roll_sql).await.unwrap();
            assert!(!a.is_empty(), "{what}: fixture produced no rows");
            assert_same_rows(&format!("{what} @ {}", tier.table()), &a, &b);
        }
    }

    // The planner sends real dashboard windows to rollups: the default 1 h
    // workloads tab, the 24 h fleet table and the 7 d views never touch raw.
    for (back, until) in [(3600, 0), (86_400, 0), (7 * 86_400, 0), (25 * 3600, 3600)] {
        let span = Span::plan(now, back, until, None);
        assert!(!span.is_raw(), "{back}s window planned on raw");
    }
    engine.shutdown().await;
}
