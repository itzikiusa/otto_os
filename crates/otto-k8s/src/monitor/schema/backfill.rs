//! Legacy history recovery. A view is a completed-tier checkpoint, installed
//! only after atomic replacement. Staging is disposable; raw history is not.

use super::*;
use std::collections::HashSet;

const JOURNAL: &str = "k8s_backfill_v2";

const LIMITS: &str = "\nSETTINGS max_threads = 1, max_insert_threads = 1, max_memory_usage = 402653184, \
    max_block_size = 8192, max_insert_block_size = 65536, \
    min_insert_block_size_rows = 65536, min_insert_block_size_bytes = 16777216, optimize_on_insert = 1";

fn targets() -> Vec<(&'static str, &'static str)> {
    ROLLUPS
        .iter()
        .map(|r| (r.table, r.view))
        .chain([(LATEST_TABLE, LATEST_VIEW), (PODS_TABLE, PODS_VIEW)])
        .collect()
}

pub(super) fn statements(cluster_id: &str, date: &str, age: i64, retention: u32) -> Vec<String> {
    let filter = format!(
        "\nFROM k8s_samples\nWHERE cluster_id = {} AND sample_date = {}",
        sql_str(cluster_id),
        sql_str(date)
    );
    let mut out = Vec::new();
    for r in ROLLUPS {
        if age <= i64::from(keep_days(r.keep_days, retention)) {
            out.push(format!(
                "INSERT INTO {}\nSELECT {} AS t, cluster_id, namespace, workload, metric, pod, cityHash64(labels) AS series, labels, \
                 value AS v_min, value AS v_max, value AS v_sum, toUInt64(1) AS n, (ts, value) AS v_last{filter}{LIMITS}",
                r.table, bucket_expr("ts", r.grain)
            ));
        }
    }
    if age <= i64::from(LATEST_KEEP_DAYS) {
        out.push(format!("INSERT INTO {LATEST_TABLE}\nSELECT cluster_id, namespace, workload, metric, pod, cityHash64(labels) AS series, labels, ts AS last_ts, value AS last_value{filter}{LIMITS}"));
    }
    out.push(format!("INSERT INTO {PODS_TABLE}\nSELECT {} AS t, cluster_id, namespace, workload, pod, toUInt64(1) AS n, ts AS last{filter}{LIMITS}", bucket_expr("ts", 3600)));
    out
}

/// Returns whether initialization performed migration work. Already completed
/// tiers are never exchanged again, including after a lost acknowledgement.
pub(super) async fn migrate(sink: &dyn MonitorSink, retention: u32) -> Result<bool> {
    let rows = sink.query_rows(
        "SELECT name FROM system.tables WHERE database = currentDatabase() AND startsWith(name, 'k8s_')"
    ).await?;
    let mut present: HashSet<&str> = rows
        .iter()
        .filter_map(|r| r.get("name").and_then(|v| v.as_str()))
        .collect();
    let targets = targets();
    // If the final MV succeeded but its response/drop failed, clean the old
    // target even on the all-views-present fast path.
    for (table, view) in &targets {
        let stage = format!("{table}_backfill");
        if present.contains(view) && present.contains(stage.as_str()) {
            sink.exec(&format!("DROP TABLE IF EXISTS {stage} SYNC"))
                .await?;
        }
    }
    if targets.iter().all(|(_, view)| present.contains(view)) {
        if present.contains(JOURNAL) {
            sink.exec(&format!("DROP TABLE IF EXISTS {JOURNAL} SYNC"))
                .await?;
        }
        return Ok(false);
    }
    if !present.contains(JOURNAL) {
        // Old recovery truncated all tiers before each retry, even if some
        // views already existed. A legacy partial view set is NOT a safe
        // checkpoint. Remove it before establishing the new protocol marker.
        for (_, view) in &targets {
            if present.remove(view) {
                sink.exec(&format!("DROP VIEW IF EXISTS {view} SYNC"))
                    .await?;
            }
        }
        sink.exec(&format!(
            "CREATE TABLE IF NOT EXISTS {JOURNAL} (version UInt8) ENGINE = TinyLog"
        ))
        .await?;
    }
    tracing::info!("k8s monitor: migrating unfinished rollup tiers");
    let parts = sink.query_rows(backfill_partitions_sql()).await?;
    let today = chrono::Utc::now().date_naive();
    let view_sql = views_sql();
    for (table, view) in targets {
        if present.contains(view) {
            continue;
        }
        let stage = format!("{table}_backfill");
        // Synchronous drop waits for previous users of this staging UUID;
        // don't reuse a table that an interrupted INSERT could still write.
        sink.exec(&format!("DROP TABLE IF EXISTS {stage} SYNC"))
            .await?;
        sink.exec(&format!("CREATE TABLE {stage} AS {table}"))
            .await?;
        let prefix = format!("INSERT INTO {table}\n");
        for part in &parts {
            let cluster = part
                .get("cluster_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let date = part.get("d").and_then(|v| v.as_str()).unwrap_or("");
            let Ok(day) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
                continue;
            };
            for sql in statements(cluster, date, (today - day).num_days(), retention) {
                if let Some(select) = sql.strip_prefix(&prefix) {
                    sink.exec(&format!("INSERT INTO {stage}\n{select}")).await?;
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
        sink.exec(&format!("EXCHANGE TABLES {stage} AND {table}"))
            .await?;
        let view_prefix = format!("CREATE MATERIALIZED VIEW IF NOT EXISTS {view} TO ");
        let create_view = view_sql
            .split(';')
            .map(str::trim)
            .find(|s| s.starts_with(&view_prefix))
            .expect("every backfill target has a view");
        sink.exec(create_view).await?;
        sink.exec(&format!("DROP TABLE IF EXISTS {stage} SYNC"))
            .await?;
        tracing::info!(tier = table, "k8s monitor: rollup tier migration complete");
    }
    sink.exec(&format!("DROP TABLE IF EXISTS {JOURNAL} SYNC"))
        .await?;
    Ok(true)
}
