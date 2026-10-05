//! A legacy raw-only install must migrate within the embedded server's
//! actual memory limit, including high-cardinality label maps. Kept separate
//! from dashboard query benchmarks: this exercises schema initialization.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_core::Result;
use otto_k8s::monitor::schema;
use otto_k8s::{BoxFut, MonitorSink};
use otto_usage::{ClickHouse, UsageConfig, UsageEngine};
use serde_json::Value;

struct Sink(Arc<UsageEngine>, AtomicBool);

impl MonitorSink for Sink {
    fn available(&self) -> bool {
        self.0.available()
    }

    fn exec<'a>(&'a self, sql: &'a str) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            if sql.starts_with("INSERT INTO k8s_samples_5m")
                && self.1.swap(false, Ordering::Relaxed)
            {
                return Err(otto_core::Error::Internal(
                    "injected migration interruption".into(),
                ));
            }
            self.0.exec_sql(sql).await.map_err(|error| {
                otto_core::Error::Internal(format!(
                    "{}: {error}",
                    sql.lines().next().unwrap_or("migration statement")
                ))
            })
        })
    }

    fn insert_ndjson<'a>(&'a self, table: &'a str, data: &'a str) -> BoxFut<'a, Result<()>> {
        Box::pin(async move { self.0.insert_ndjson(table, data).await })
    }

    fn query_rows<'a>(&'a self, sql: &'a str) -> BoxFut<'a, Result<Vec<Value>>> {
        Box::pin(async move { self.0.query_rows(sql).await })
    }
}

#[tokio::test]
#[ignore = "isolated ClickHouse migration scale regression; run with --ignored"]
async fn high_cardinality_backfill_fits_embedded_memory_limit() {
    if ClickHouse::locate(None).is_none() {
        assert!(
            std::env::var("OTTO_REQUIRE_CLICKHOUSE").unwrap_or_default() != "1",
            "ClickHouse binary required"
        );
        eprintln!("SKIP: ClickHouse binary unavailable");
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
    let ready = engine.wait_ready(Duration::from_secs(45)).await;
    if !ready {
        engine.shutdown().await;
        panic!("isolated ClickHouse did not become ready");
    }
    eprintln!(
        "BACKFILL_PERF pid={}",
        engine.clickhouse().unwrap().server_pid().unwrap()
    );
    let sink = Sink(engine.clone(), AtomicBool::new(true));
    let result = tokio::time::timeout(Duration::from_secs(300), async {
        sink.exec(&schema::schema_sql(14)).await?;
        // Many distinct series at the same instant: a time-window-only fix
        // cannot mask unbounded GROUP BY memory. Generate in SQL blocks so
        // the test driver does not retain the synthetic rows in Rust memory.
        for batch in 0..30 {
            sink.exec(&format!(
                "INSERT INTO k8s_samples (ts, cluster_id, namespace, workload_kind, workload, pod, container, metric, labels, value) \
                 SELECT toStartOfDay(now()) + INTERVAL 1 HOUR, 'migration-test', 'ns', 'Deployment', 'web', \
                 concat('pod-', toString(number % 600)), 'app', 'requests', \
                 map('path', concat(toString(number), repeat('x', 384))), 1 \
                 FROM numbers({}, 100000) SETTINGS max_threads = 1, max_block_size = 4096, \
                 min_insert_block_size_rows = 0, min_insert_block_size_bytes = 0",
                batch * 100_000
            ))
            .await?;
        }
        eprintln!("seeded 3 million distinct raw series; starting migration");
        let reading = Arc::new(AtomicBool::new(true));
        let mut foreground = tokio::task::JoinSet::new();
        for _ in 0..4 {
            let engine = engine.clone();
            let reading = reading.clone();
            foreground.spawn(async move {
                let mut queries = 0;
                let mut worst_ms = 0;
                while reading.load(Ordering::Relaxed) {
                    let started = std::time::Instant::now();
                    engine.query_rows("SELECT sum(n) FROM k8s_samples_1m SETTINGS max_threads=1, max_memory_usage=67108864").await?;
                    worst_ms = worst_ms.max(started.elapsed().as_millis());
                    queries += 1;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Ok::<_, otto_core::Error>((queries, worst_ms))
            });
        }

        match schema::ensure(&sink, 14).await {
            Err(error) if error.to_string().contains("injected migration interruption") => {}
            Err(error) => return Err(error),
            Ok(()) => return Err(otto_core::Error::Internal("interruption was not exercised".into())),
        }
        let partial = sink
            .query_rows("SELECT sum(n) AS n FROM k8s_samples_1m")
            .await?;
        // The retry must keep the completed first tier and finish the rest.
        schema::ensure(&sink, 14).await?;
        let first = sink
            .query_rows("SELECT sum(n) AS n, sum(v_sum) AS total FROM k8s_samples_1m")
            .await?;
        // Completed initialization must neither truncate nor double counts.
        schema::ensure(&sink, 14).await?;
        let second = sink
            .query_rows("SELECT sum(n) AS n, sum(v_sum) AS total FROM k8s_samples_1m")
            .await?;
        let views = sink.query_rows(&schema::views_present_sql()).await?;
        let tiers = sink.query_rows(
            "SELECT '5m' AS tier, sum(n) AS n FROM k8s_samples_5m \
             UNION ALL SELECT '1h', sum(n) FROM k8s_samples_1h \
             UNION ALL SELECT 'pods', sum(n) FROM k8s_pods_1h"
        ).await?;
        reading.store(false, Ordering::Relaxed);
        while let Some(result) = foreground.join_next().await {
            let (queries, worst_ms) = result.expect("foreground reader")?;
            eprintln!("BACKFILL_FOREGROUND queries={queries} worst_ms={worst_ms}");
        }
        // Agent-like concurrent readers after migration: bounded requests,
        // serial scenarios, no provider calls or user data. External resource
        // sampling correlates these markers with this isolated server PID.
        for readers in [1usize, 4, 8] {
            eprintln!("BACKFILL_READERS n={readers}");
            let started = std::time::Instant::now();
            let mut tasks = tokio::task::JoinSet::new();
            for _ in 0..readers {
                let engine = engine.clone();
                tasks.spawn(async move {
                    let until = tokio::time::Instant::now() + Duration::from_secs(3);
                    while tokio::time::Instant::now() < until {
                        let rows = engine.query_rows("SELECT sum(n) AS n FROM k8s_samples_1m SETTINGS max_threads=1, max_memory_usage=67108864").await?;
                        if rows[0]["n"].as_u64() != Some(3_000_000) {
                            return Err(otto_core::Error::Internal("concurrent reader count changed".into()));
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Ok::<_, otto_core::Error>(())
                });
            }
            while let Some(result) = tasks.join_next().await { result.expect("reader task")?; }
            eprintln!("BACKFILL_READERS_DONE n={readers} elapsed_ms={}", started.elapsed().as_millis());
        }
        eprintln!("BACKFILL_IDLE_START");
        tokio::time::sleep(Duration::from_secs(5)).await;
        Ok::<_, otto_core::Error>((partial, first, second, views, tiers))
    })
    .await;
    engine.shutdown().await;
    let (partial, first, second, views, tiers) = result
        .expect("migration timed out")
        .expect("migration failed");
    assert_eq!(partial[0]["n"].as_u64(), Some(3_000_000));
    assert_eq!(first[0]["n"].as_u64(), Some(3_000_000));
    assert_eq!(first[0]["total"].as_f64(), Some(3_000_000.0));
    assert_eq!(second, first, "completed migration must be idempotent");
    assert_eq!(views[0]["n"].as_u64(), Some(schema::views().len() as u64));
    assert_eq!(tiers.len(), 3);
    for row in tiers {
        assert_eq!(row["n"].as_u64(), Some(3_000_000), "tier {}", row["tier"]);
    }
}

#[tokio::test]
async fn backfill_retry_preserves_completed_tiers_and_new_inserts() {
    if ClickHouse::locate(None).is_none() {
        assert_ne!(
            std::env::var("OTTO_REQUIRE_CLICKHOUSE").unwrap_or_default(),
            "1"
        );
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
    assert!(engine.wait_ready(Duration::from_secs(45)).await);
    let sink = Sink(engine.clone(), AtomicBool::new(true));
    let result = tokio::spawn(async move {
        sink.exec(&schema::schema_sql(14)).await?;
        sink.exec("INSERT INTO k8s_samples (ts, cluster_id, namespace, workload, pod, metric, labels, value) SELECT now64(3), 'resume', 'ns', 'web', 'pod', 'requests', map('path', toString(number % 10)), toFloat64(number) FROM numbers(1000)").await?;
        let error = schema::ensure(&sink, 14).await.unwrap_err();
        assert!(error.to_string().contains("injected migration interruption"));
        let views = sink.query_rows(&schema::views_present_sql()).await?;
        assert_eq!(views[0]["n"].as_u64(), Some(1), "the completed first tier must be checkpointed before starting the next");
        let before = sink.query_rows("SELECT uuid FROM system.tables WHERE database=currentDatabase() AND name='k8s_samples_1m'").await?;
        schema::ensure(&sink, 14).await?;
        let after = sink.query_rows("SELECT uuid FROM system.tables WHERE database=currentDatabase() AND name='k8s_samples_1m'").await?;
        assert_eq!(before, after, "a completed target must not be rebuilt on retry");
        for tier in ["k8s_samples_1m", "k8s_samples_5m", "k8s_samples_1h", "k8s_pods_1h"] {
            let rows = sink.query_rows(&format!("SELECT sum(n) AS n FROM {tier}")).await?;
            assert_eq!(rows[0]["n"].as_u64(), Some(1000), "{tier}");
        }
        sink.exec("INSERT INTO k8s_samples (ts, cluster_id, namespace, workload, pod, metric, labels, value) VALUES (now64(3), 'resume', 'ns', 'web', 'pod', 'requests', map('path', '0'), 7)").await?;
        schema::ensure(&sink, 14).await?;
        let rows = sink.query_rows("SELECT sum(n) AS n, sum(v_sum) AS total FROM k8s_samples_1m").await?;
        assert_eq!(rows[0]["n"].as_u64(), Some(1001));
        assert_eq!(rows[0]["total"].as_f64(), Some(499507.0));
        Ok::<_, otto_core::Error>(())
    }).await;
    engine.shutdown().await;
    result.expect("retry assertion").expect("retry query");
}

struct InterruptedSink {
    engine: Arc<UsageEngine>,
    fault: std::sync::Mutex<Option<(&'static str, &'static str, usize)>>,
}

impl MonitorSink for InterruptedSink {
    fn available(&self) -> bool {
        self.engine.available()
    }
    fn exec<'a>(&'a self, sql: &'a str) -> BoxFut<'a, Result<()>> {
        Box::pin(async move {
            let mode = {
                let mut fault = self.fault.lock().unwrap();
                match fault.as_mut() {
                    Some((prefix, mode, skip)) if sql.starts_with(*prefix) => {
                        if *skip > 0 {
                            *skip -= 1;
                            None
                        } else {
                            let mode = *mode;
                            *fault = None;
                            Some(mode)
                        }
                    }
                    _ => None,
                }
            };
            if mode == Some("partial") {
                self.engine
                    .exec_sql(&sql.replace("\nSETTINGS", "\nLIMIT 11\nSETTINGS"))
                    .await?;
            } else if mode != Some("before") {
                self.engine.exec_sql(sql).await?;
            }
            if mode.is_some() {
                return Err(otto_core::Error::Internal(
                    "injected acknowledgement loss".into(),
                ));
            }
            Ok(())
        })
    }
    fn insert_ndjson<'a>(&'a self, table: &'a str, data: &'a str) -> BoxFut<'a, Result<()>> {
        Box::pin(async move { self.engine.insert_ndjson(table, data).await })
    }
    fn query_rows<'a>(&'a self, sql: &'a str) -> BoxFut<'a, Result<Vec<Value>>> {
        Box::pin(async move { self.engine.query_rows(sql).await })
    }
}

#[tokio::test]
async fn backfill_recovers_partial_writes_and_lost_acknowledgements() {
    if ClickHouse::locate(None).is_none() {
        assert_ne!(
            std::env::var("OTTO_REQUIRE_CLICKHOUSE").unwrap_or_default(),
            "1"
        );
        return;
    }
    for fault in [
        ("INSERT INTO k8s_samples_1m_backfill", "partial", 0),
        ("INSERT INTO k8s_samples_5m_backfill", "legacy", 0),
        ("EXCHANGE TABLES k8s_samples_1m_backfill", "before", 0),
        ("EXCHANGE TABLES k8s_samples_1m_backfill", "after", 0),
        (
            "CREATE MATERIALIZED VIEW IF NOT EXISTS k8s_samples_1m_mv TO",
            "after",
            0,
        ),
        (
            "DROP TABLE IF EXISTS k8s_pods_1h_backfill SYNC",
            "before",
            1,
        ),
    ] {
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
        assert!(engine.wait_ready(Duration::from_secs(45)).await);
        let sink = InterruptedSink {
            engine: engine.clone(),
            fault: std::sync::Mutex::new(Some(fault)),
        };
        // Capture assertion panics so the isolated child is stopped on every path.
        let result = tokio::spawn(async move {
            sink.exec(&schema::schema_sql(14)).await?;
            sink.exec("INSERT INTO k8s_samples (ts, cluster_id, namespace, workload, pod, metric, labels, value) SELECT toStartOfDay(now()) + toIntervalSecond(number % 120), 'recovery', 'ns', 'web', 'pod', 'requests', map('path', 'one'), toFloat64(number % 100) FROM numbers(70000)").await?;
            if fault.1 == "legacy" {
                // The old algorithm could leave a MV installed after it had
                // truncated that target on a later, failed retry.
                sink.exec(schema::views_sql().split(';').next().unwrap()).await?;
            }
            let error = schema::ensure(&sink, 14).await.unwrap_err();
            assert!(error.to_string().contains("injected acknowledgement loss"), "{error}");
            if fault.1 == "partial" || fault.1 == "before" && fault.2 == 0 {
                let rows = sink.query_rows("SELECT count() AS n FROM k8s_samples_1m").await?;
                assert_eq!(rows[0]["n"].as_u64(), Some(0), "failed staging must leave target untouched");
            }
            schema::ensure(&sink, 14).await?;
            for tier in ["k8s_samples_1m", "k8s_samples_5m", "k8s_samples_1h"] {
                let rows = sink.query_rows(&format!("SELECT sum(n) AS n, sum(v_sum) AS total, min(v_min) AS lo, max(v_max) AS hi, max(v_last).1 AS last_ts FROM {tier}")).await?;
                assert_eq!(rows[0]["n"].as_u64(), Some(70000), "{fault:?} {tier}");
                assert_eq!(rows[0]["total"].as_f64(), Some(3465000.0));
                assert_eq!(rows[0]["lo"].as_f64(), Some(0.0));
                assert_eq!(rows[0]["hi"].as_f64(), Some(99.0));
                let expected = sink.query_rows("SELECT max(ts) AS last_ts FROM k8s_samples").await?;
                assert_eq!(rows[0]["last_ts"], expected[0]["last_ts"]);
            }
            let latest = sink.query_rows("SELECT max(last_ts) AS ts, argMax(last_value, last_ts) AS value FROM k8s_latest").await?;
            let raw = sink.query_rows("SELECT max(ts) AS ts, argMax(value, ts) AS value FROM k8s_samples").await?;
            assert_eq!(latest[0]["ts"], raw[0]["ts"]);
            // Equal timestamps have no deterministic argMax tie-break, so
            // compare exact latest values using a strictly newer row below.
            let stages = sink.query_rows("SELECT name FROM system.tables WHERE database=currentDatabase() AND endsWith(name, '_backfill')").await?;
            assert!(stages.is_empty(), "orphan staging tables after {fault:?}: {stages:?}");
            sink.exec("INSERT INTO k8s_samples (ts, cluster_id, namespace, workload, pod, metric, labels, value) VALUES (now64(3), 'recovery', 'ns', 'web', 'pod', 'requests', map('path','one'), 101)").await?;
            let rows = sink.query_rows("SELECT sum(n) AS n, max(v_max) AS hi FROM k8s_samples_1m").await?;
            assert_eq!(rows[0]["n"].as_u64(), Some(70001));
            assert_eq!(rows[0]["hi"].as_f64(), Some(101.0));
            let latest = sink.query_rows("SELECT argMax(last_value, last_ts) AS value FROM k8s_latest").await?;
            assert_eq!(latest[0]["value"].as_f64(), Some(101.0));
            Ok::<_, otto_core::Error>(())
        }).await;
        engine.shutdown().await;
        result.expect("recovery assertion").expect("recovery query");
    }
}
