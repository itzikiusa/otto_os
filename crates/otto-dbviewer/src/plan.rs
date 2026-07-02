//! Pure normalizers that distill each engine's native EXPLAIN output into the
//! engine-agnostic [`PlanNode`] tree the UI renders. Kept side-effect-free (JSON
//! in → tree out) so they're exhaustively unit-tested against canned fixtures
//! without a live database. The drivers run the actual `EXPLAIN` and hand the
//! parsed JSON (or plain-text lines) here.
//!
//! Warnings flag the costly access patterns the UI badges in red: full scans
//! (`access_type: ALL` on MySQL / `Seq Scan` on Postgres / `COLLSCAN` on Mongo),
//! and MySQL's `Using filesort` / `Using temporary`. ClickHouse's
//! `ReadFromMergeTree` is deliberately NOT flagged (it isn't a full-scan smell).

use serde_json::Value;

use crate::types::PlanNode;

/// Cap a detail string so a giant predicate can't blow up the plan tree.
fn short(s: &str) -> String {
    const MAX: usize = 160;
    if s.chars().count() > MAX {
        let kept: String = s.chars().take(MAX - 1).collect();
        format!("{kept}…")
    } else {
        s.to_string()
    }
}

/// Recursively test whether `key` appears anywhere in `v` with a `true` value —
/// used to surface MySQL's `using_filesort` / `using_temporary_table` flags,
/// which can sit at the query-block, ordering, or grouping level.
fn json_flag_true(v: &Value, key: &str) -> bool {
    match v {
        Value::Object(m) => {
            if m.get(key).and_then(Value::as_bool) == Some(true) {
                return true;
            }
            m.values().any(|x| json_flag_true(x, key))
        }
        Value::Array(a) => a.iter().any(|x| json_flag_true(x, key)),
        _ => false,
    }
}

// --- MySQL (EXPLAIN FORMAT=JSON) --------------------------------------------

/// Normalize MySQL `EXPLAIN FORMAT=JSON`. The plan is a `query_block` that nests
/// `table` entries (directly, in `nested_loop`, or under ordering/grouping ops).
/// We flatten the table accesses into children of a `query_block` root and hoist
/// the `using_filesort` / `using_temporary_table` flags to root warnings.
pub fn from_mysql_json(v: &Value) -> PlanNode {
    let qb = v.get("query_block").unwrap_or(v);
    let mut root = PlanNode::op("query_block");
    if json_flag_true(v, "using_filesort") {
        root.warnings.push("Using filesort".into());
    }
    if json_flag_true(v, "using_temporary_table") {
        root.warnings.push("Using temporary".into());
    }
    collect_mysql_tables(qb, &mut root.children);
    root
}

fn collect_mysql_tables(v: &Value, out: &mut Vec<PlanNode>) {
    match v {
        Value::Object(m) => {
            if let Some(name) = m.get("table_name").and_then(Value::as_str) {
                let access = m.get("access_type").and_then(Value::as_str).unwrap_or("");
                // MySQL 8.3+/9.x replaced the classic `query_block`/`table` shape
                // with `query_plan`/`inputs` where each node has a descriptive
                // `operation` (e.g. "Table scan on orders") and `access_type` is
                // "table"/"index"/"ref" (not the classic "ALL"/"ref"/"eq_ref").
                let operation = m.get("operation").and_then(Value::as_str).unwrap_or("");
                let mut n = PlanNode::op(if !access.is_empty() {
                    access
                } else if !operation.is_empty() {
                    operation
                } else {
                    "table"
                });
                n.object = Some(name.to_string());
                n.est_rows = m
                    .get("rows_produced_per_join")
                    .or_else(|| m.get("rows_examined_per_scan"))
                    .or_else(|| m.get("estimated_rows")) // new format
                    .and_then(Value::as_f64);
                n.detail = m
                    .get("attached_condition")
                    .or_else(|| m.get("condition")) // new format
                    .and_then(Value::as_str)
                    .map(short)
                    .or_else(|| {
                        m.get("key")
                            .and_then(Value::as_str)
                            .map(|k| format!("key: {k}"))
                    });
                // Full table scan: classic `access_type: ALL`, new `access_type:
                // table`, or a "Table scan on …" operation label.
                if access.eq_ignore_ascii_case("ALL")
                    || access.eq_ignore_ascii_case("table")
                    || operation.to_ascii_lowercase().starts_with("table scan")
                {
                    n.warnings.push("full table scan".into());
                }
                out.push(n);
                // A table entry can still wrap a materialized subquery with its own
                // tables — recurse into its non-scalar members to catch those.
                for (k, val) in m {
                    if k != "table_name" {
                        collect_mysql_tables(val, out);
                    }
                }
            } else {
                for val in m.values() {
                    collect_mysql_tables(val, out);
                }
            }
        }
        Value::Array(a) => {
            for val in a {
                collect_mysql_tables(val, out);
            }
        }
        _ => {}
    }
}

// --- PostgreSQL (EXPLAIN (FORMAT JSON)) -------------------------------------

/// Normalize Postgres `EXPLAIN (FORMAT JSON)` — a `[{ "Plan": { … } }]` array
/// whose nodes recurse through `Plans`. `Seq Scan` is flagged as a full scan.
pub fn from_pg_json(v: &Value) -> PlanNode {
    let plan = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|x| x.get("Plan"))
        .or_else(|| v.get("Plan"))
        .unwrap_or(v);
    pg_node(plan)
}

fn pg_node(v: &Value) -> PlanNode {
    let op = v.get("Node Type").and_then(Value::as_str).unwrap_or("Plan").to_string();
    let mut n = PlanNode::op(&op);
    n.object = v
        .get("Relation Name")
        .or_else(|| v.get("Index Name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    n.est_rows = v.get("Plan Rows").and_then(Value::as_f64);
    n.detail = v
        .get("Index Cond")
        .or_else(|| v.get("Filter"))
        .or_else(|| v.get("Hash Cond"))
        .or_else(|| v.get("Join Type"))
        .and_then(Value::as_str)
        .map(short);
    if op.eq_ignore_ascii_case("Seq Scan") {
        n.warnings.push("sequential scan (full table)".into());
    }
    if let Some(kids) = v.get("Plans").and_then(Value::as_array) {
        n.children = kids.iter().map(pg_node).collect();
    }
    n
}

// --- ClickHouse (EXPLAIN json=1, plain-text fallback) -----------------------

/// Normalize ClickHouse `EXPLAIN json=1` — a `[{ "Plan": { "Node Type", "Plans" } }]`
/// tree (same nesting as Postgres, different node types). No full-scan warning:
/// `ReadFromMergeTree` is normal for MergeTree tables, not a smell (spec §5.2).
pub fn from_clickhouse_json(v: &Value) -> PlanNode {
    let plan = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|x| x.get("Plan"))
        .or_else(|| v.get("Plan"))
        .unwrap_or(v);
    ch_node(plan)
}

fn ch_node(v: &Value) -> PlanNode {
    let op = v.get("Node Type").and_then(Value::as_str).unwrap_or("Plan").to_string();
    let mut n = PlanNode::op(&op);
    n.object = v.get("Description").and_then(Value::as_str).map(str::to_string);
    if let Some(kids) = v.get("Plans").and_then(Value::as_array) {
        n.children = kids.iter().map(ch_node).collect();
    }
    n
}

/// Fallback for when `EXPLAIN json=1` isn't available: wrap the plain-text
/// `EXPLAIN` output as one root node with a child per non-blank line.
pub fn from_clickhouse_text(lines: &[String]) -> PlanNode {
    let mut root = PlanNode::op("EXPLAIN");
    root.children = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(PlanNode::op)
        .collect();
    root
}

// --- MongoDB (explain queryPlanner) -----------------------------------------

/// Normalize a Mongo `explain` (queryPlanner) document. Walks `winningPlan`
/// (Mongo 7+ nests the classic plan under `queryPlan`) via `inputStage(s)`.
/// `COLLSCAN` is flagged as a collection scan.
pub fn from_mongo_queryplanner(v: &Value) -> PlanNode {
    let wp = v
        .pointer("/queryPlanner/winningPlan")
        .unwrap_or(v);
    // Mongo 7+ slot-based execution nests the classic stage tree under `queryPlan`.
    let wp = wp.get("queryPlan").unwrap_or(wp);
    mongo_stage(wp)
}

fn mongo_stage(v: &Value) -> PlanNode {
    let stage = v.get("stage").and_then(Value::as_str).unwrap_or("stage").to_string();
    let mut n = PlanNode::op(&stage);
    n.object = v.get("indexName").and_then(Value::as_str).map(str::to_string);
    if n.object.is_none() {
        if let Some(kp) = v.get("keyPattern") {
            n.detail = Some(short(&kp.to_string()));
        } else if let Some(filter) = v.get("filter") {
            n.detail = Some(short(&filter.to_string()));
        }
    }
    if stage.eq_ignore_ascii_case("COLLSCAN") {
        n.warnings.push("collection scan (COLLSCAN)".into());
    }
    if let Some(input) = v.get("inputStage") {
        n.children.push(mongo_stage(input));
    }
    if let Some(arr) = v.get("inputStages").and_then(Value::as_array) {
        for s in arr {
            n.children.push(mongo_stage(s));
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mysql_full_scan_and_filesort_flagged() {
        // A single-table SELECT with a full scan + ORDER BY filesort.
        let v = json!({
            "query_block": {
                "select_id": 1,
                "ordering_operation": {
                    "using_filesort": true,
                    "table": {
                        "table_name": "orders",
                        "access_type": "ALL",
                        "rows_examined_per_scan": 1000,
                        "rows_produced_per_join": 1000,
                        "attached_condition": "(orders.total > 10)"
                    }
                }
            }
        });
        let root = from_mysql_json(&v);
        assert_eq!(root.op, "query_block");
        assert!(root.warnings.contains(&"Using filesort".to_string()));
        assert_eq!(root.children.len(), 1);
        let t = &root.children[0];
        assert_eq!(t.op, "ALL");
        assert_eq!(t.object.as_deref(), Some("orders"));
        assert_eq!(t.est_rows, Some(1000.0));
        assert!(t.warnings.contains(&"full table scan".to_string()));
        assert_eq!(t.detail.as_deref(), Some("(orders.total > 10)"));
    }

    #[test]
    fn mysql_new_9x_format_table_scan_flagged() {
        // MySQL 8.3+/9.x EXPLAIN FORMAT=JSON: query_plan/inputs, descriptive
        // `operation`, `access_type: "table"`, `estimated_rows`.
        let v = json!({
            "query_plan": {
                "operation": "Filter: (orders.total_cents > 0)",
                "access_type": "filter",
                "inputs": [{
                    "operation": "Table scan on orders",
                    "table_name": "orders",
                    "access_type": "table",
                    "estimated_rows": 4.0,
                    "condition": "(orders.total_cents > 0)"
                }]
            }
        });
        let root = from_mysql_json(&v);
        let t = root
            .children
            .iter()
            .find(|c| c.object.as_deref() == Some("orders"))
            .expect("orders node");
        assert_eq!(t.est_rows, Some(4.0));
        assert!(
            t.warnings.contains(&"full table scan".to_string()),
            "new-format access_type=table must flag a full scan"
        );
        assert_eq!(t.detail.as_deref(), Some("(orders.total_cents > 0)"));
    }

    #[test]
    fn mysql_indexed_ref_has_no_full_scan_warning() {
        let v = json!({
            "query_block": {
                "table": { "table_name": "customers", "access_type": "eq_ref", "key": "PRIMARY",
                           "rows_examined_per_scan": 1 }
            }
        });
        let root = from_mysql_json(&v);
        let t = &root.children[0];
        assert_eq!(t.op, "eq_ref");
        assert!(t.warnings.is_empty());
        assert_eq!(t.detail.as_deref(), Some("key: PRIMARY"));
    }

    #[test]
    fn postgres_seq_scan_flagged_and_children_walked() {
        let v = json!([{
            "Plan": {
                "Node Type": "Aggregate",
                "Plan Rows": 1,
                "Plans": [{
                    "Node Type": "Seq Scan",
                    "Relation Name": "orders",
                    "Plan Rows": 5000,
                    "Filter": "(status = 'paid')"
                }]
            }
        }]);
        let root = from_pg_json(&v);
        assert_eq!(root.op, "Aggregate");
        assert_eq!(root.children.len(), 1);
        let scan = &root.children[0];
        assert_eq!(scan.op, "Seq Scan");
        assert_eq!(scan.object.as_deref(), Some("orders"));
        assert_eq!(scan.est_rows, Some(5000.0));
        assert!(scan.warnings.contains(&"sequential scan (full table)".to_string()));
    }

    #[test]
    fn postgres_index_scan_not_flagged() {
        let v = json!([{ "Plan": {
            "Node Type": "Index Scan", "Relation Name": "customers", "Index Name": "customers_pkey",
            "Plan Rows": 1, "Index Cond": "(id = 1)"
        }}]);
        let root = from_pg_json(&v);
        assert_eq!(root.op, "Index Scan");
        assert_eq!(root.object.as_deref(), Some("customers"));
        assert!(root.warnings.is_empty());
        assert_eq!(root.detail.as_deref(), Some("(id = 1)"));
    }

    #[test]
    fn clickhouse_json_tree_no_full_scan_warning() {
        let v = json!([{ "Plan": {
            "Node Type": "Expression",
            "Plans": [{ "Node Type": "ReadFromMergeTree", "Description": "analytics.events" }]
        }}]);
        let root = from_clickhouse_json(&v);
        assert_eq!(root.op, "Expression");
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].op, "ReadFromMergeTree");
        // ReadFromMergeTree is normal — never flagged.
        assert!(root.children[0].warnings.is_empty());
    }

    #[test]
    fn clickhouse_text_fallback_is_one_node_per_line() {
        let lines = vec![
            "Expression (Projection)".to_string(),
            "  ReadFromMergeTree (analytics.events)".to_string(),
            "  ".to_string(), // blank → dropped
        ];
        let root = from_clickhouse_text(&lines);
        assert_eq!(root.op, "EXPLAIN");
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].op, "Expression (Projection)");
    }

    #[test]
    fn mongo_collscan_flagged() {
        let v = json!({
            "queryPlanner": { "winningPlan": { "stage": "COLLSCAN", "filter": { "status": "paid" } } }
        });
        let root = from_mongo_queryplanner(&v);
        assert_eq!(root.op, "COLLSCAN");
        assert!(root.warnings.contains(&"collection scan (COLLSCAN)".to_string()));
    }

    #[test]
    fn mongo_ixscan_walks_input_stage_no_warning() {
        let v = json!({
            "queryPlanner": { "winningPlan": {
                "stage": "FETCH",
                "inputStage": { "stage": "IXSCAN", "indexName": "status_1", "keyPattern": { "status": 1 } }
            }}
        });
        let root = from_mongo_queryplanner(&v);
        assert_eq!(root.op, "FETCH");
        assert_eq!(root.children.len(), 1);
        let ix = &root.children[0];
        assert_eq!(ix.op, "IXSCAN");
        assert_eq!(ix.object.as_deref(), Some("status_1"));
        assert!(root.warnings.is_empty() && ix.warnings.is_empty());
    }

    #[test]
    fn mongo7_query_plan_nesting_is_unwrapped() {
        let v = json!({
            "queryPlanner": { "winningPlan": { "queryPlan": { "stage": "COLLSCAN" } } }
        });
        let root = from_mongo_queryplanner(&v);
        assert_eq!(root.op, "COLLSCAN");
        assert!(!root.warnings.is_empty());
    }
}
