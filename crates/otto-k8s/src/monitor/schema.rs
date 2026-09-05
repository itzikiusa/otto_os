//! ClickHouse DDL for the monitoring tables (spec "Storage"). Both tables are
//! partitioned per cluster + day so per-cluster purges are cheap and the TTL
//! follows the largest configured retention (`alter_ttl_sql`); clusters that
//! ask for less than the table TTL are trimmed by `purge_cluster_sql` at the
//! end of each cycle.

/// Escape a string literal for ClickHouse (`'`, `\`, newline).
pub fn sql_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' => o.push_str("\\'"),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            _ => o.push(ch),
        }
    }
    o.push('\'');
    o
}

/// `CREATE TABLE IF NOT EXISTS` for `k8s_samples` + `k8s_events` with a
/// `{retention_days}` TTL. Idempotent — run at every collector start.
pub fn schema_sql(retention_days: u32) -> String {
    let ttl = retention_days.max(1);
    format!(
        "CREATE TABLE IF NOT EXISTS k8s_samples (
    ts            DateTime64(3),
    sample_date   Date DEFAULT toDate(ts),
    cluster_id    LowCardinality(String),
    namespace     LowCardinality(String),
    workload_kind LowCardinality(String),
    workload      LowCardinality(String),
    pod           String,
    container     LowCardinality(String),
    metric        LowCardinality(String),
    labels        Map(LowCardinality(String), String),
    value         Float64
) ENGINE = MergeTree
PARTITION BY (cluster_id, sample_date)
ORDER BY (cluster_id, namespace, workload, metric, pod, ts)
TTL sample_date + INTERVAL {ttl} DAY;

CREATE TABLE IF NOT EXISTS k8s_events (
    ts          DateTime64(3),
    event_date  Date DEFAULT toDate(ts),
    cluster_id  LowCardinality(String),
    namespace   LowCardinality(String),
    workload    LowCardinality(String),
    pod         String,
    container   LowCardinality(String),
    kind        LowCardinality(String),
    class       LowCardinality(String),
    reason      String,
    exit_code   Int32,
    detail      String,
    actor       String
) ENGINE = MergeTree
PARTITION BY (cluster_id, event_date)
ORDER BY (cluster_id, ts)
TTL event_date + INTERVAL {ttl} DAY;"
    )
}

/// `ALTER TABLE … MODIFY TTL` for both tables.
pub fn alter_ttl_sql(retention_days: u32) -> String {
    let ttl = retention_days.max(1);
    format!(
        "ALTER TABLE k8s_samples MODIFY TTL sample_date + INTERVAL {ttl} DAY;
ALTER TABLE k8s_events MODIFY TTL event_date + INTERVAL {ttl} DAY;"
    )
}

/// `DELETE` statements for one cluster (both tables); `before_date`
/// (`YYYY-MM-DD`) limits to rows older than that day, `None` = everything
/// (cluster removed).
pub fn purge_cluster_sql(cluster_id: &str, before_date: Option<&str>) -> Vec<String> {
    let cid = sql_str(cluster_id);
    let mk = |table: &str, date_col: &str| {
        let mut q = format!("DELETE FROM {table} WHERE cluster_id = {cid}");
        if let Some(d) = before_date {
            q.push_str(&format!(" AND {date_col} < {}", sql_str(d)));
        }
        q
    };
    vec![mk("k8s_samples", "sample_date"), mk("k8s_events", "event_date")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_has_both_tables_and_ttl() {
        let s = schema_sql(14);
        assert!(s.contains("CREATE TABLE IF NOT EXISTS k8s_samples"));
        assert!(s.contains("CREATE TABLE IF NOT EXISTS k8s_events"));
        assert_eq!(s.matches("INTERVAL 14 DAY").count(), 2);
        assert!(s.contains("labels        Map(LowCardinality(String), String)"));
    }

    #[test]
    fn purge_targets_one_cluster_only() {
        let v = purge_cluster_sql("c'1", Some("2026-09-01"));
        assert_eq!(v.len(), 2);
        assert!(v[0].contains("cluster_id = 'c\\'1'"));
        assert!(v[0].contains("sample_date < '2026-09-01'"));
        let all = purge_cluster_sql("c1", None);
        assert!(!all[0].contains("sample_date <"));
    }

    #[test]
    fn retention_floor_is_one_day() {
        assert!(alter_ttl_sql(0).contains("INTERVAL 1 DAY"));
    }

    #[test]
    fn sql_str_escapes() {
        assert_eq!(sql_str("a'b\\c\nd"), "'a\\'b\\\\c\\nd'");
    }
}
