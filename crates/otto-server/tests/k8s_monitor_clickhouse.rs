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
use otto_k8s::monitor::queries::{Level, Span, Tier, WSpan, WideMetric};
use otto_k8s::monitor::{fleet, health, queries, schema, wide};
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
    if no_clickhouse() {
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
    // The wide rows the collector writes for the same two cycles.
    let mut snap_pods = Snapshot::new();
    snap_pods.insert("shop/web-a".into(), p1.clone());
    snap_pods.insert("shop/web-b".into(), p2.clone());
    for (at, scale) in [(now - chrono::Duration::minutes(5), 1.0), (now, 4.0)] {
        let mut by_pod = std::collections::HashMap::new();
        for key in ["shop/web-a", "shop/web-b"] {
            by_pod.insert(
                key.to_string(),
                vec![
                    s("mem_sys_bytes", &[], 200.0),
                    s("http_requests_total", &[("code", "200")], 100.0 * scale),
                    s("http_requests_total", &[("code", "500")], scale),
                    s(
                        "http_request_duration_seconds_bucket",
                        &[("le", "0.1")],
                        50.0 * scale,
                    ),
                    s(
                        "http_request_duration_seconds_bucket",
                        &[("le", "+Inf")],
                        60.0 * scale,
                    ),
                ],
            );
        }
        sink.insert_ndjson(
            schema::POD_CYCLE_TABLE,
            &wide::cycle_ndjson("c1-builders", at, &snap_pods, &by_pod)
                .replace("\"c1-builders\"", "\"c1\""),
        )
        .await
        .unwrap();
    }

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

/// No `clickhouse` binary: skip on a dev box, FAIL where the ratchets are
/// required (CI sets `OTTO_REQUIRE_CLICKHOUSE=1` after installing the pinned
/// binary — a vacuous pass there hid every query-builder regression, perf K7).
fn no_clickhouse() -> bool {
    if ClickHouse::locate(None).is_some() {
        return false;
    }
    let required = std::env::var("OTTO_REQUIRE_CLICKHOUSE")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    assert!(
        !required,
        "OTTO_REQUIRE_CLICKHOUSE is set but no `clickhouse` binary was found"
    );
    eprintln!("SKIP: no `clickhouse` binary on this machine");
    true
}

async fn start_engine(tmp: &tempfile::TempDir) -> Option<Arc<UsageEngine>> {
    if no_clickhouse() {
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
        let mut cases: Vec<(&str, String, String)> = vec![
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

// ---------------------------------------------------------------------------
// Wide tiers (one row per pod per cycle, counters as increments)
// ---------------------------------------------------------------------------

/// Independent reference: the reset-aware increment of a counter between two
/// consecutive samples (first sample = 0).
fn inc(prev: Option<f64>, v: f64) -> f64 {
    match prev {
        None => 0.0,
        Some(p) if v >= p => v - p,
        Some(_) => v,
    }
}

/// One fixture pod: (namespace, workload, pod, j).
const WIDE_PODS: [(&str, &str, &str, f64); 4] = [
    ("shop", "web", "web-a", 1.0),
    ("shop", "web", "web-b", 2.0),
    ("shop", "api", "api-a", 3.0),
    // Same workload NAME in another namespace (F1): never summed with shop/web.
    ("cbo", "web", "web-c", 4.0),
];
/// web-b's process restarts at this cycle: its counters start over (reset).
const RESET_AT: u32 = 200;

/// Counter values of one pod at cycle `i`.
fn counters_at(pod: &str, j: f64, i: u32) -> (f64, f64, f64, f64, f64, f64) {
    let n = if pod == "web-b" && i >= RESET_AT {
        f64::from(i - RESET_AT) * 0.5
    } else {
        f64::from(i)
    };
    (
        n * 7.0 * j,           // 200s
        (n / 9.0).floor() * j, // 503s
        n * 5.0,               // le 0.1
        n * 7.0 * j,           // le +Inf
        n * 0.25 * j,          // latency sum
        n * 7.0 * j,           // latency count
    )
}

/// Memory samples of one pod at cycle `i`: web-a is a two-container pod
/// (metrics-server, summed per pod — F2); the rest expose `mem_sys_bytes`.
fn memory_at(pod: &str, j: f64, i: u32) -> (Vec<Sample>, f64) {
    let base = 1000.0 * j + f64::from(i % 17) * 3.0;
    if pod == "web-a" {
        let smp = otto_k8s::monitor::collector::metrics_server_samples(
            [(10, base as i64), (5, 64)].into_iter(),
        );
        (smp, (base as i64 + 64) as f64)
    } else {
        (vec![s("mem_sys_bytes", &[], base)], base)
    }
}

#[derive(Default, Clone, Debug)]
struct Ref {
    req: f64,
    err: f64,
    lat_sum: f64,
    lat_cnt: f64,
    le01: f64,
    inf: f64,
    mem_sum: f64,
    mem_n: f64,
    mem_max: f64,
    /// (ts, value) of the latest cycle's memory total.
    mem_last: (i64, f64),
}

fn close(what: &str, a: f64, b: f64) {
    let tol = 1e-6 * a.abs().max(b.abs()).max(1.0);
    assert!((a - b).abs() <= tol, "{what}: wide {a} vs reference {b}");
}

#[tokio::test]
async fn wide_tiers_match_a_raw_increment_reference() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(engine) = start_engine(&tmp).await else {
        return;
    };
    let sink = EngineSink(engine.clone());
    schema::ensure(&sink, 14).await.unwrap();
    let cid = "wide-eq";

    // 20 s cycles over the last ~3 hours, from an hour boundary.
    let now = Utc::now().timestamp();
    let start = (now / 3600 - 3) * 3600;
    let mut snap = Snapshot::new();
    for (ns, wl, name, _) in WIDE_PODS {
        let mut p = pod(name, "1.0.0");
        p.namespace = ns.into();
        p.workload = wl.into();
        snap.insert(format!("{ns}/{name}"), p);
    }
    // cycle ts → per-pod reference increments + memory.
    let mut cycles: Vec<(i64, BTreeMap<&str, Ref>)> = Vec::new();
    let mut prev: BTreeMap<&str, (f64, f64, f64, f64, f64, f64)> = BTreeMap::new();
    let mut nd = String::new();
    let mut i = 0u32;
    let mut t = start;
    while t <= now {
        let at = chrono::DateTime::from_timestamp(t, 0).unwrap();
        let mut by_pod = std::collections::HashMap::new();
        let mut refs = BTreeMap::new();
        for (ns, _, name, j) in WIDE_PODS {
            let c = counters_at(name, j, i);
            let (mem_samples, mem_total) = memory_at(name, j, i);
            let mut smp = mem_samples;
            smp.extend([
                s("http_requests_total", &[("code", "200")], c.0),
                s("http_requests_total", &[("code", "503")], c.1),
                s(
                    "http_request_duration_seconds_bucket",
                    &[("le", "0.1")],
                    c.2,
                ),
                s(
                    "http_request_duration_seconds_bucket",
                    &[("le", "+Inf")],
                    c.3,
                ),
                s("http_request_duration_seconds_sum", &[], c.4),
                s("http_request_duration_seconds_count", &[], c.5),
            ]);
            let p = prev.get(name);
            let r = Ref {
                req: inc(p.map(|x| x.0), c.0) + inc(p.map(|x| x.1), c.1),
                err: inc(p.map(|x| x.1), c.1),
                le01: inc(p.map(|x| x.2), c.2),
                inf: inc(p.map(|x| x.3), c.3),
                lat_sum: inc(p.map(|x| x.4), c.4),
                lat_cnt: inc(p.map(|x| x.5), c.5),
                mem_sum: mem_total,
                mem_n: 1.0,
                mem_max: mem_total,
                mem_last: (t, mem_total),
            };
            prev.insert(name, c);
            refs.insert(name, r);
            // Raw rows exactly as written (the per-series tiers' input).
            nd.push_str(&samples_ndjson(
                cid,
                at,
                &snap[&format!("{ns}/{name}")],
                "app",
                &smp,
                &BTreeMap::new(),
            ));
            by_pod.insert(format!("{ns}/{name}"), smp);
        }
        // One insert per cycle, as the collector does.
        sink.insert_ndjson(
            schema::POD_CYCLE_TABLE,
            &wide::cycle_ndjson(cid, at, &snap, &by_pod),
        )
        .await
        .unwrap();
        cycles.push((t, refs));
        t += 20;
        i += 1;
    }
    sink.insert_ndjson("k8s_samples", &nd).await.unwrap();

    // Reference totals per (namespace, workload[, pod]) over [from, to).
    let reference = |from: i64, to: i64, by_pod: bool| -> BTreeMap<(String, String, String), Ref> {
        let mut out: BTreeMap<(String, String, String), Ref> = BTreeMap::new();
        for (ts, refs) in &cycles {
            if *ts < from || *ts >= to {
                continue;
            }
            // A workload's memory per cycle is the sum of its pods.
            let mut cycle_wl: BTreeMap<(String, String, String), f64> = BTreeMap::new();
            for (ns, wl, name, _) in WIDE_PODS {
                let r = &refs[name];
                let k = (
                    ns.to_string(),
                    wl.to_string(),
                    if by_pod {
                        name.to_string()
                    } else {
                        String::new()
                    },
                );
                let e = out.entry(k.clone()).or_default();
                e.req += r.req;
                e.err += r.err;
                e.lat_sum += r.lat_sum;
                e.lat_cnt += r.lat_cnt;
                e.le01 += r.le01;
                e.inf += r.inf;
                e.mem_sum += r.mem_sum;
                e.mem_n += 1.0;
                e.mem_max = e.mem_max.max(r.mem_max);
                *cycle_wl.entry(k).or_default() += r.mem_sum;
            }
            for (k, total) in cycle_wl {
                out.get_mut(&k).unwrap().mem_last = (*ts, total);
            }
        }
        out
    };

    let filter = format!(" AND cluster_id = '{cid}'");
    for tier in schema::WIDE_TIERS {
        let span = WSpan::between(tier, start, None, now);
        for (src, by_pod) in [(Level::Workload, false), (Level::Pod, true)] {
            let got = queries::wide_totals(&sink, &[span], now, src, src, &filter)
                .await
                .unwrap();
            let want = reference(start, now + 1, by_pod);
            assert_eq!(got.len(), want.len(), "{} rows", span.table(src));
            for ((_, ns, wl, pod), t) in &got {
                let w = &want[&(ns.clone(), wl.clone(), pod.clone())];
                let what = format!("{} {ns}/{wl}/{pod}", span.table(src));
                close(&format!("{what} req"), t.req, w.req);
                close(&format!("{what} err"), t.err, w.err);
                close(&format!("{what} lat_sum"), t.lat_sum, w.lat_sum);
                close(&format!("{what} lat_cnt"), t.lat_cnt, w.lat_cnt);
                close(&format!("{what} le 0.1"), t.hist["0.1"], w.le01);
                close(&format!("{what} le +Inf"), t.hist["+Inf"], w.inf);
                close(&format!("{what} mem_sum"), t.mem_sum, w.mem_sum);
                close(&format!("{what} mem_n"), t.mem_n, w.mem_n);
                close(&format!("{what} mem_max"), t.mem_max, w.mem_max);
                close(&format!("{what} mem_last"), t.mem_last, w.mem_last.1);
            }
        }
        // F1: the two `web` workloads stay apart.
        let got = queries::wide_totals(
            &sink,
            &[span],
            now,
            Level::Workload,
            Level::Workload,
            &filter,
        )
        .await
        .unwrap();
        assert!(got.keys().any(|k| k.1 == "cbo" && k.2 == "web"));
        assert!(got.keys().any(|k| k.1 == "shop" && k.2 == "web"));
    }
    // F2: web-a's memory is BOTH containers.
    let (_, web_a_mem) = memory_at("web-a", 1.0, 0);
    assert!(web_a_mem > 1000.0 + 64.0 - 1.0);

    // Stitched totals (1m head + 5m middle + 1m edge) == one minute-tier read.
    let parts = WSpan::plan_total(now, 3600, 0);
    assert!(parts.len() >= 2, "{parts:?}");
    let single = WSpan::between(schema::WIDE_1M, parts[0].from, None, now);
    let a = queries::wide_totals(
        &sink,
        &parts,
        now,
        Level::Workload,
        Level::Workload,
        &filter,
    )
    .await
    .unwrap();
    let b = queries::wide_totals(
        &sink,
        &[single],
        now,
        Level::Workload,
        Level::Workload,
        &filter,
    )
    .await
    .unwrap();
    assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
    for (k, x) in &a {
        let y = &b[k];
        close("stitched req", x.req, y.req);
        close("stitched hist", x.hist["+Inf"], y.hist["+Inf"]);
        close("stitched mem_sum", x.mem_sum, y.mem_sum);
        close("stitched mem_last", x.mem_last, y.mem_last);
    }
    // Served again from the closed-span cache for the closed parts: same.
    let again = queries::wide_totals(
        &sink,
        &parts,
        now,
        Level::Workload,
        Level::Workload,
        &filter,
    )
    .await
    .unwrap();
    assert_eq!(again, a);

    // Charts: per-bucket wide series equal the reference per bucket.
    for tier in schema::WIDE_TIERS {
        let step = (tier.grain.max(60) * 2) as u32;
        let span = WSpan::between(tier, start, None, now);
        let rows = sink
            .query_rows(&queries::wide_spark_in(&span, cid, Some("shop"), step))
            .await
            .unwrap();
        assert!(!rows.is_empty());
        for r in &rows {
            let ts = chrono::DateTime::parse_from_rfc3339(r["t"].as_str().unwrap())
                .unwrap()
                .timestamp();
            let want = reference(ts, ts + i64::from(step), false);
            let w = &want[&(
                "shop".to_string(),
                r["workload"].as_str().unwrap().to_string(),
                String::new(),
            )];
            let cyc = cycles
                .iter()
                .filter(|(t, _)| *t >= ts && *t < ts + i64::from(step))
                .count() as f64;
            close(
                "spark rps",
                r["rps"].as_f64().unwrap(),
                w.req / f64::from(step),
            );
            close("spark mem", r["mem"].as_f64().unwrap(), w.mem_sum / cyc);
        }
        for (metric, src, g) in [
            (
                WideMetric::Rps,
                Level::Workload,
                "concat(namespace, '/', workload)",
            ),
            (
                WideMetric::Err,
                Level::Pod,
                "concat(namespace, '/', workload, '/', pod)",
            ),
            (
                WideMetric::Latency,
                Level::Workload,
                "concat(namespace, '/', workload)",
            ),
        ] {
            let rows = sink
                .query_rows(&queries::wide_series_in(
                    &span, src, g, &filter, metric, step,
                ))
                .await
                .unwrap();
            assert!(!rows.is_empty());
            for r in &rows {
                let ts = chrono::DateTime::parse_from_rfc3339(r["t"].as_str().unwrap())
                    .unwrap()
                    .timestamp();
                let key: Vec<&str> = r["g"].as_str().unwrap().split('/').collect();
                let want = reference(ts, ts + i64::from(step), src == Level::Pod);
                let w = &want[&(
                    key[0].to_string(),
                    key[1].to_string(),
                    key.get(2).map(|p| p.to_string()).unwrap_or_default(),
                )];
                let v = r["v"].as_f64().unwrap();
                match metric {
                    WideMetric::Rps => close("series rps", v, w.req / f64::from(step)),
                    WideMetric::Err if w.req > 0.0 => close("series err", v, 100.0 * w.err / w.req),
                    WideMetric::Latency if w.lat_cnt > 0.0 => {
                        close("series latency", v, 1000.0 * w.lat_sum / w.lat_cnt)
                    }
                    _ => {}
                }
            }
        }
    }

    // The reset (#2): web-b's counters restarted mid-window. The per-series
    // max − min under-counts it; the increments match the reference.
    let web_b =
        fleet::FleetFilter::parse(Some(cid), Some("shop"), Some("web"), Some("web-b")).unwrap();
    let legacy = Span::between(Tier::Rollup(schema::ROLLUP_1H), start, None, now);
    let old = sink
        .query_rows(&fleet::rates_in(&legacy, &web_b, fleet::Group::Pod))
        .await
        .unwrap();
    let want = reference(start, now + 1, true)
        [&("shop".to_string(), "web".to_string(), "web-b".to_string())]
        .req;
    let old_req = old[0]["rps"].as_f64().unwrap() * legacy.secs as f64;
    let new = queries::wide_totals(
        &sink,
        &[WSpan::between(schema::WIDE_1H, start, None, now)],
        now,
        Level::Pod,
        Level::Pod,
        &web_b.sql(),
    )
    .await
    .unwrap();
    let new_req = new.values().next().unwrap().req;
    close("web-b increments", new_req, want);
    assert!(
        old_req < 0.9 * want,
        "max − min should under-count the reset: {old_req} vs {want}"
    );

    // The health aggregate end to end on the wide tiers.
    let stats = health::workload_stats(&sink, cid, &snap, None, chrono::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(stats.len(), 3, "shop/web, shop/api, cbo/web");
    assert!(stats.iter().all(|s| s.rps > 0.0 && s.latency_kind == "p95"));
    assert!(
        stats.iter().all(|s| s.rps_baseline > 0.0),
        "baseline from the hour tier"
    );
    engine.shutdown().await;
}

// ---------------------------------------------------------------------------
// Rows read per refresh: the per-series tiers (before) vs the wide tiers
// ---------------------------------------------------------------------------

/// Rows ClickHouse plans to read for `sql` (`EXPLAIN ESTIMATE`, summed over
/// every table the query touches) — granule-exact, like `read_rows`.
async fn rows_read(sink: &EngineSink, sql: &str) -> u64 {
    sink.query_rows(&format!("EXPLAIN ESTIMATE {sql}"))
        .await
        .unwrap()
        .iter()
        .map(|r| {
            r["rows"]
                .as_u64()
                .or_else(|| r["rows"].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(0)
        })
        .sum()
}

async fn rows_read_all(sink: &EngineSink, sqls: &[String]) -> u64 {
    let mut n = 0;
    for q in sqls {
        let r = rows_read(sink, q).await;
        // Per-query breakdown: rows + the tables it reads.
        let tables: Vec<&str> = q
            .split("FROM ")
            .skip(1)
            .filter_map(|t| t.split_whitespace().next())
            .filter(|t| t.starts_with("k8s_"))
            .collect();
        eprintln!("    {r:>9} rows  {}", tables.join(" + "));
        n += r;
    }
    n
}

/// The pre-wide `versions_sql` (removed: versions come from the snapshot).
fn old_versions_sql(cid: &str) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, pod, argMax(labels['version'], last_ts) AS version
         FROM k8s_latest
         WHERE cluster_id IN ('{cid}') AND labels['version'] != '' AND last_ts >= now() - INTERVAL 900 SECOND
         GROUP BY cluster_id, namespace, workload, pod"
    )
}

/// `[(metric, labels, base, rate)]` per pod: memory, request counters by
/// code, an 11-bucket histogram by code, sum/count by code, status series.
/// `(metric, labels, base, rate)` — a generated series.
type SeriesSpec = (String, Vec<(String, String)>, f64, f64);

fn measure_series() -> Vec<SeriesSpec> {
    let mut v: Vec<SeriesSpec> = Vec::new();
    let l = |kv: &[(&str, &str)]| -> Vec<(String, String)> {
        kv.iter()
            .map(|(k, x)| (k.to_string(), x.to_string()))
            .collect()
    };
    v.push(("mem_working_set_bytes".into(), vec![], 1e8, 0.0));
    for (code, rate) in [("200", 50.0), ("500", 1.0)] {
        v.push((
            "http_requests_total".into(),
            l(&[("code", code)]),
            0.0,
            rate,
        ));
        for (k, le) in [
            "0.005", "0.01", "0.025", "0.05", "0.1", "0.25", "0.5", "1", "2.5", "5", "+Inf",
        ]
        .iter()
        .enumerate()
        {
            v.push((
                "http_request_duration_seconds_bucket".into(),
                l(&[("code", code), ("le", le)]),
                0.0,
                rate * (k as f64 + 1.0) / 11.0,
            ));
        }
        v.push((
            "http_request_duration_seconds_sum".into(),
            l(&[("code", code)]),
            0.0,
            rate * 0.05,
        ));
        v.push((
            "http_request_duration_seconds_count".into(),
            l(&[("code", code)]),
            0.0,
            rate,
        ));
    }
    for m in [
        "restarts_total",
        "ready",
        "phase_running",
        "mem_limit_bytes",
        "cpu_request_millis",
        "pod_age_seconds",
    ] {
        v.push((m.into(), vec![], 1.0, 0.0));
    }
    v
}

#[tokio::test]
async fn rows_read_per_refresh_before_and_after() {
    let tmp = tempfile::tempdir().unwrap();
    let Some(engine) = start_engine(&tmp).await else {
        return;
    };
    let sink = EngineSink(engine.clone());
    schema::ensure(&sink, 14).await.unwrap();
    // Scale: OTTO_K8S_MEASURE_PODS pods (3 per workload), 60 s cycles, 26 h.
    let pods: u64 = std::env::var("OTTO_K8S_MEASURE_PODS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60)
        .max(3)
        / 3
        * 3;
    // Every wide table carries the time index (granule pruning on `t`).
    assert_eq!(
        sink.query_rows(&schema::wide_t_index_present_sql())
            .await
            .unwrap()
            .len(),
        schema::wide_tables().len()
    );
    let cid = "m1";
    let now = Utc::now().timestamp();
    let start = (now - 26 * 3600) / 60 * 60;
    let cycles = ((now - start) / 60) as u64 + 1;
    let series = measure_series();

    // Raw rows (the per-series tiers' input), generated inside ClickHouse.
    let arr = series
        .iter()
        .map(|(m, kv, base, rate)| {
            let ks = kv.iter().map(|(k, _)| format!("'{k}'")).collect::<Vec<_>>().join(", ");
            let vs = kv.iter().map(|(_, x)| format!("'{x}'")).collect::<Vec<_>>().join(", ");
            format!("('{m}', CAST([{ks}] AS Array(String)), CAST([{vs}] AS Array(String)), toFloat64({base}), toFloat64({rate}))")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let t0 = std::time::Instant::now();
    sink.exec(&format!(
        "INSERT INTO k8s_samples (ts, cluster_id, namespace, workload_kind, workload, pod, container, metric, labels, value)
         SELECT toDateTime64({start} + 60 * c, 3), '{cid}', 'shop', 'deployment', concat('wl', toString(intDiv(p, 3))),
                concat('wl', toString(intDiv(p, 3)), '-', toString(p % 3)), 'app',
                s.1, mapFromArrays(s.2, s.3), s.4 + s.5 * c
         FROM (SELECT number % {pods} AS p, intDiv(number, {pods}) AS c FROM numbers({total}))
         ARRAY JOIN [{arr}] AS s
         SETTINGS max_block_size = 4096, min_insert_block_size_rows = 200000, max_threads = 2",
        total = pods * cycles,
    ))
    .await
    .unwrap();
    // The wide rows the collector would write for the same samples: constant
    // increments (0 on the first cycle), memory 1e8 per pod. Blocks hold
    // whole cycles (max_block_size is a multiple of the pod count).
    let hist_rate = |code_rate: f64| -> String {
        let les = [
            "0.005", "0.01", "0.025", "0.05", "0.1", "0.25", "0.5", "1", "2.5", "5", "+Inf",
        ];
        let ks = les
            .iter()
            .map(|l| format!("'{l}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let vs = (0..les.len())
            .map(|k| format!("{}", code_rate * (k as f64 + 1.0) / 11.0))
            .collect::<Vec<_>>()
            .join(", ");
        format!("mapFromArrays([{ks}], [{vs}])")
    };
    sink.exec(&format!(
        "INSERT INTO k8s_pod_cycle
         SELECT toDateTime({start} + 60 * c), '{cid}', 'shop', concat('wl', toString(intDiv(p, 3))),
                concat('wl', toString(intDiv(p, 3)), '-', toString(p % 3)),
                1e8, 1, if(c = 0, 0, 51), if(c = 0, 0, 1), if(c = 0, 0, 2.55), if(c = 0, 0, 51),
                if(c = 0, mapFromArrays(CAST([] AS Array(String)), CAST([] AS Array(Float64))), {h})
         FROM (SELECT number % {pods} AS p, intDiv(number, {pods}) AS c FROM numbers({total}))
         SETTINGS max_block_size = {block}, max_threads = 1",
        h = hist_rate(51.0),
        total = pods * cycles,
        block = pods * 50,
    ))
    .await
    .unwrap();
    let mut tables: Vec<&str> = schema::ROLLUPS.iter().map(|r| r.table).collect();
    tables.push("k8s_latest");
    tables.extend(schema::wide_tables());
    for t in &tables {
        sink.exec(&format!("OPTIMIZE TABLE {t} FINAL"))
            .await
            .unwrap();
    }
    let gen_secs = t0.elapsed().as_secs_f64();
    // Sanity: every cycle folded exactly once into the workload hour tier.
    assert_eq!(
        count(
            &sink,
            &format!("SELECT toUInt64(sum(n)) AS n FROM k8s_wl_1h WHERE cluster_id = '{cid}'")
        )
        .await,
        cycles * (pods / 3)
    );

    let cids = vec![cid.to_string()];
    let all = fleet::FleetFilter::parse(Some(cid), None, None, None).unwrap();
    let day = chrono::Duration::hours(24);
    let hour = chrono::Duration::hours(1);
    let filter = format!(" AND cluster_id = '{cid}'");
    let wl = Level::Workload;

    // Workloads tab / health digest, window = 1 h.
    let spark_step = queries::chart_step(180, 3600);
    let before_wl = vec![
        queries::latest_memory_sql(&cids, None, 900),
        queries::memory_between_sql(&cids, None, 3600 + 900, 3600 - 900),
        queries::request_rates_sql(&cids, None, 3600, 0),
        queries::request_rates_sql(&cids, None, 3600 + 86_400, 3600),
        queries::latency_buckets_sql(&cids, None, 3600, 0),
        queries::latency_buckets_sql(&cids, None, 3600 + 86_400, 3600),
        old_versions_sql(cid),
        queries::workload_spark_sql(cid, None, &queries::MEMORY_GAUGES, hour, spark_step, false),
        queries::workload_spark_sql(
            cid,
            None,
            &queries::REQUEST_COUNTERS,
            hour,
            spark_step,
            true,
        ),
    ];
    let mut after_wl = vec![queries::latest_memory_sql(&cids, None, 900)];
    let mut after_wl_open = after_wl.clone();
    for parts in [
        WSpan::plan_total(now, 3600, 0),
        WSpan::plan_total(now, 3600 + 86_400, 3600),
        WSpan::plan_total(now, 3600 + 900, 3600 - 900),
    ] {
        for p in parts {
            let q = queries::wide_totals_in(&p, wl, wl, &filter);
            if !p.closed(now) {
                after_wl_open.push(q.clone());
            }
            after_wl.push(q);
        }
    }
    let wstep = queries::wide_step(180, 3600);
    let spark = queries::wide_spark_in(&WSpan::plan(now, 3600, 0, Some(wstep)), cid, None, wstep);
    after_wl.push(spark.clone());
    after_wl_open.push(spark);

    // Fleet table (24 h, by workload) and one fleet chart (24 h rps).
    let before_table = vec![
        fleet::memory_sql(&all, day, fleet::Group::Workload),
        fleet::rates_sql(&all, day, fleet::Group::Workload),
        fleet::latency_buckets_sql(&all, day, fleet::Group::Workload),
        fleet::latency_avg_sql(&all, day, fleet::Group::Workload),
    ];
    let after_table: Vec<String> = WSpan::plan_total(now, 86_400, 0)
        .iter()
        .map(|p| queries::wide_totals_in(p, wl, wl, &all.sql()))
        .collect();
    let before_chart = vec![fleet::series_sql(
        &all,
        day,
        fleet::SeriesMetric::Rps,
        fleet::SeriesBy::Workload,
        1500,
    )];
    let after_chart = vec![fleet::wide_series_sql(
        &all,
        day,
        fleet::SeriesMetric::Rps,
        fleet::SeriesBy::Workload,
        queries::wide_step(1440, 86_400),
    )];

    let mut report = Vec::new();
    for (what, before, after) in [
        ("workloads/health 1h", &before_wl, &after_wl),
        (
            "workloads/health 1h (closed spans cached)",
            &before_wl,
            &after_wl_open,
        ),
        ("fleet table 24h", &before_table, &after_table),
        ("fleet chart rps 24h", &before_chart, &after_chart),
    ] {
        eprintln!("  {what} — before:");
        let b = rows_read_all(&sink, before).await;
        eprintln!("  {what} — after:");
        let a = rows_read_all(&sink, after).await;
        // Every new query runs (and is valid SQL), not just EXPLAINs.
        for q in after {
            sink.query_rows(q).await.unwrap();
        }
        report.push(format!(
            "{what}: before {b} rows, after {a} rows ({:.0}x)",
            b as f64 / a.max(1) as f64
        ));
        assert!(a * 10 <= b, "{what}: {a} vs {b}");
    }
    eprintln!(
        "ROWS READ ({pods} pods, {} workloads, 60 s cycles, 26 h; fixture {gen_secs:.1}s):\n  {}",
        pods / 3,
        report.join("\n  ")
    );
    engine.shutdown().await;
}
