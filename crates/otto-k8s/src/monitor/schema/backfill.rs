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

/// Tables whose history is keyed by a bucket column `t` (every tier but
/// `k8s_latest`, which only holds the newest value per series).
fn has_history(table: &str) -> bool {
    ROLLUPS.iter().any(|r| r.table == table) || table == PODS_TABLE
}

fn count_of(v: &serde_json::Value, k: &str) -> u64 {
    v.get(k)
        .and_then(|x| {
            x.as_u64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
}

fn date_of(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// How a missing view's target is recovered (S6-10).
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Rebuild from raw in staging, then EXCHANGE over the target: raw covers
    /// every day the target holds (or it is empty).
    Rebuild,
    /// The target holds days raw no longer has (raw keeps ~2 days, the hour
    /// tier 90): never replace it. Only raw days after the target's newest
    /// day (`after`, `YYYY-MM-DD`) are appended.
    TopUp { after: String },
}

/// Decide [`Plan`] for `table` from its own day span and raw's first day.
async fn plan_for(sink: &dyn MonitorSink, table: &str) -> Result<Plan> {
    if !has_history(table) {
        return Ok(Plan::Rebuild);
    }
    let target = sink
        .query_rows(&format!(
            "SELECT count() AS n, toString(min(toDate(t))) AS lo, toString(max(toDate(t))) AS hi FROM {table}"
        ))
        .await?;
    let Some(target) = target.first().filter(|r| count_of(r, "n") > 0) else {
        return Ok(Plan::Rebuild);
    };
    let raw = sink
        .query_rows("SELECT count() AS n, toString(min(sample_date)) AS lo FROM k8s_samples")
        .await?;
    let raw_lo = raw
        .first()
        .filter(|r| count_of(r, "n") > 0)
        .map(|r| date_of(r, "lo"));
    let (lo, hi) = (date_of(target, "lo"), date_of(target, "hi"));
    // ISO dates compare lexically; no raw at all also means "keep".
    let predates_raw = raw_lo.is_none_or(|raw_lo| lo < raw_lo);
    Ok(if predates_raw {
        Plan::TopUp { after: hi }
    } else {
        Plan::Rebuild
    })
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
        let prefix = format!("INSERT INTO {table}\n");
        let view_prefix = format!("CREATE MATERIALIZED VIEW IF NOT EXISTS {view} TO ");
        let create_view = view_sql
            .split(';')
            .map(str::trim)
            .find(|s| s.starts_with(&view_prefix))
            .expect("every backfill target has a view");
        if let Plan::TopUp { after } = plan_for(sink, table).await? {
            // S6-10: a tier added by a release, or one view lost to a failed
            // CREATE, must not trade 90 days of hour rollups for a rebuild
            // from 2 days of raw. Append only the raw days the target lacks.
            tracing::warn!(
                tier = table,
                after = %after,
                "k8s monitor: target holds history older than raw; topping up instead of rebuilding"
            );
            for part in &parts {
                let cluster = part
                    .get("cluster_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let date = part.get("d").and_then(|v| v.as_str()).unwrap_or("");
                let Ok(day) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
                    continue;
                };
                if date <= after.as_str() {
                    continue;
                }
                for sql in statements(cluster, date, (today - day).num_days(), retention) {
                    if sql.starts_with(&prefix) {
                        sink.exec(&sql).await?;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
            sink.exec(create_view).await?;
            tracing::info!(tier = table, "k8s monitor: rollup tier view restored");
            continue;
        }
        let stage = format!("{table}_backfill");
        // Synchronous drop waits for previous users of this staging UUID;
        // don't reuse a table that an interrupted INSERT could still write.
        sink.exec(&format!("DROP TABLE IF EXISTS {stage} SYNC"))
            .await?;
        sink.exec(&format!("CREATE TABLE {stage} AS {table}"))
            .await?;
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
        sink.exec(create_view).await?;
        sink.exec(&format!("DROP TABLE IF EXISTS {stage} SYNC"))
            .await?;
        tracing::info!(tier = table, "k8s monitor: rollup tier migration complete");
    }
    sink.exec(&format!("DROP TABLE IF EXISTS {JOURNAL} SYNC"))
        .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BoxFut;
    use std::sync::Mutex;

    /// Scripted sink: `system.tables` lists `tables`; per-table stats come
    /// from `stats` (substring → row); every exec is recorded.
    struct Scripted {
        tables: Vec<&'static str>,
        stats: Vec<(&'static str, serde_json::Value)>,
        execs: Mutex<Vec<String>>,
    }

    impl MonitorSink for Scripted {
        fn available(&self) -> bool {
            true
        }
        fn exec<'a>(&'a self, sql: &'a str) -> BoxFut<'a, Result<()>> {
            self.execs.lock().unwrap().push(sql.to_string());
            Box::pin(async { Ok(()) })
        }
        fn insert_ndjson<'a>(&'a self, _t: &'a str, _d: &'a str) -> BoxFut<'a, Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn query_rows<'a>(&'a self, sql: &'a str) -> BoxFut<'a, Result<Vec<serde_json::Value>>> {
            let rows = if sql.contains("system.tables") {
                self.tables
                    .iter()
                    .map(|t| serde_json::json!({ "name": t }))
                    .collect()
            } else if sql.contains("cluster_id") && sql.contains(" AS d") {
                // backfill partitions: raw keeps today + yesterday.
                let today = chrono::Utc::now().date_naive();
                [today - chrono::Duration::days(1), today]
                    .iter()
                    .map(|d| serde_json::json!({"cluster_id": "c1", "d": d.format("%Y-%m-%d").to_string()}))
                    .collect()
            } else {
                self.stats
                    .iter()
                    .find(|(needle, _)| sql.contains(needle))
                    .map(|(_, r)| vec![r.clone()])
                    .unwrap_or_default()
            };
            Box::pin(async move { Ok(rows) })
        }
    }

    #[tokio::test]
    async fn a_missing_hour_view_never_replaces_history_older_than_raw() {
        // S6-10: the hour view is missing (a failed CREATE on upgrade),
        // there is no journal, and raw only goes back to yesterday while
        // the hour tier holds 90 days.
        let today = chrono::Utc::now().date_naive();
        let yesterday = (today - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let old = (today - chrono::Duration::days(85))
            .format("%Y-%m-%d")
            .to_string();
        let mut tables: Vec<&'static str> = targets()
            .iter()
            .flat_map(|(t, v)| [*t, *v])
            .filter(|n| *n != ROLLUP_1H.view)
            .collect();
        tables.push("k8s_samples");
        let sink = Scripted {
            tables,
            stats: vec![
                (
                    "FROM k8s_samples_1h",
                    serde_json::json!({"n": "2000", "lo": old, "hi": yesterday}),
                ),
                ("FROM k8s_samples\n", serde_json::json!({})),
                (
                    "min(sample_date)",
                    serde_json::json!({"n": "500", "lo": yesterday}),
                ),
            ],
            execs: Mutex::new(Vec::new()),
        };
        assert!(migrate(&sink, 90).await.unwrap());
        let execs = sink.execs.lock().unwrap().clone();
        assert!(
            !execs
                .iter()
                .any(|e| e.contains("EXCHANGE TABLES k8s_samples_1h_backfill")),
            "the hour tier was replaced: {execs:#?}"
        );
        assert!(!execs
            .iter()
            .any(|e| e.contains("CREATE TABLE k8s_samples_1h_backfill")));
        // Only days after the tier's newest day are appended — today, not
        // yesterday (already folded).
        let topups: Vec<&String> = execs
            .iter()
            .filter(|e| e.starts_with("INSERT INTO k8s_samples_1h\n"))
            .collect();
        assert_eq!(topups.len(), 1, "{topups:#?}");
        assert!(topups[0].contains(&today.format("%Y-%m-%d").to_string()));
        assert!(execs
            .iter()
            .any(|e| e.starts_with("CREATE MATERIALIZED VIEW IF NOT EXISTS k8s_samples_1h_mv TO")));
        // A tier raw fully covers (empty here) is still rebuilt atomically.
        assert!(execs
            .iter()
            .any(|e| e == "EXCHANGE TABLES k8s_samples_1m_backfill AND k8s_samples_1m"));
    }
}
