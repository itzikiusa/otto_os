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
        match schema::ensure(&sink, 14).await {
            Err(error) if error.to_string().contains("injected migration interruption") => {}
            Err(error) => return Err(error),
            Ok(()) => return Err(otto_core::Error::Internal("interruption was not exercised".into())),
        }
        let partial = sink
            .query_rows("SELECT sum(n) AS n FROM k8s_samples_1m")
            .await?;
        // The retry must clear the partial first tier before rebuilding it.
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
