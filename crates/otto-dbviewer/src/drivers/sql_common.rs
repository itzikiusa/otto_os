//! Helpers the SQL drivers (MySQL, Postgres) share, in one place instead of a
//! byte-identical copy per driver (or one driver reaching into the other's
//! internals for them): the bulk schema-graph assembler, pool-acquire error
//! wording and f64 → JSON shaping.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use crate::types::{self, NodeKind};

// --- Bulk schema graph ---------------------------------------------------------

/// One diagrammable relation from a bulk catalog read, in tree order.
pub(crate) struct BulkRel {
    /// The lazy tree's node id for this object (`db:<s>/table:<t>` …).
    pub id: String,
    pub name: String,
    pub kind: NodeKind,
}

/// One column row from a bulk catalog read (`primary_key` may be pre-set by
/// engines that report it per column, e.g. MySQL's `column_key = 'PRI'`).
pub(crate) struct BulkCol {
    pub table: String,
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
}

/// One FK column pair from a bulk catalog read, rows ordered by
/// (table, constraint, key position).
pub(crate) struct BulkFk {
    pub table: String,
    pub name: String,
    pub column: String,
    pub ref_schema: Option<String>,
    pub ref_table: String,
    pub ref_column: String,
}

/// Assemble a [`types::SchemaGraph`] from set-based catalog rows — the same
/// shape the per-object walk builds: the first `max_tables` relations (in the
/// given order) with PK/FK column flags, the FK edges out of them (a missing
/// ref schema defaults to this schema), `truncated` when relations were cut.
pub(crate) fn assemble_bulk_graph(
    schema: &str,
    rels: Vec<BulkRel>,
    cols: Vec<BulkCol>,
    pks: &[(String, String)],
    fks: Vec<BulkFk>,
    max_tables: usize,
) -> types::SchemaGraph {
    use std::collections::HashSet;
    let truncated = rels.len() > max_tables;
    let kept: Vec<BulkRel> = rels.into_iter().take(max_tables).collect();
    let kept_names: HashSet<&str> = kept.iter().map(|r| r.name.as_str()).collect();
    let pk: HashSet<(&str, &str)> = pks.iter().map(|(t, c)| (t.as_str(), c.as_str())).collect();
    let fk_cols: HashSet<(&str, &str)> = fks
        .iter()
        .map(|f| (f.table.as_str(), f.column.as_str()))
        .collect();
    let mut by_table: HashMap<&str, Vec<types::GraphColumn>> = HashMap::new();
    for c in &cols {
        if !kept_names.contains(c.table.as_str()) {
            continue;
        }
        by_table
            .entry(c.table.as_str())
            .or_default()
            .push(types::GraphColumn {
                name: c.name.clone(),
                data_type: c.data_type.clone(),
                nullable: c.nullable,
                primary_key: c.primary_key || pk.contains(&(c.table.as_str(), c.name.as_str())),
                foreign_key: fk_cols.contains(&(c.table.as_str(), c.name.as_str())),
            });
    }
    // Group FK rows into edges (rows arrive ordered by table, constraint, pos).
    let mut edges: Vec<types::GraphEdge> = Vec::new();
    for f in &fks {
        if !kept_names.contains(f.table.as_str()) {
            continue;
        }
        match edges.last_mut() {
            Some(e) if e.from_table == f.table && e.name == f.name => {
                e.from_columns.push(f.column.clone());
                e.to_columns.push(f.ref_column.clone());
            }
            _ => edges.push(types::GraphEdge {
                name: f.name.clone(),
                from_table: f.table.clone(),
                from_columns: vec![f.column.clone()],
                to_schema: f.ref_schema.clone().unwrap_or_else(|| schema.to_string()),
                to_table: f.ref_table.clone(),
                to_columns: vec![f.ref_column.clone()],
            }),
        }
    }
    let tables = kept
        .into_iter()
        .map(|r| types::GraphTable {
            columns: by_table.remove(r.name.as_str()).unwrap_or_default(),
            id: r.id,
            schema: schema.to_string(),
            name: r.name,
            kind: r.kind,
        })
        .collect();
    types::SchemaGraph {
        schema: schema.to_string(),
        tables,
        edges,
        relationships: true,
        truncated,
    }
}

// --- Pool acquire -------------------------------------------------------------

/// `PoolTimedOut` means every pooled session stayed busy for the driver's
/// acquire timeout: say so (with that driver's pool size) instead of sqlx's
/// generic text.
pub(crate) fn acquire_error(
    e: sqlx::Error,
    max_connections: u32,
    acquire_timeout: Duration,
) -> otto_core::Error {
    match e {
        sqlx::Error::PoolTimedOut => types::upstream(format!(
            "connection busy: all {max_connections} sessions to this server are in use \
             (waited {}s); stop a running query or retry",
            acquire_timeout.as_secs()
        )),
        other => types::upstream(other),
    }
}

// --- Cell shaping -------------------------------------------------------------

/// Pure shaping of an optional f64 (Null if absent or non-finite).
pub(crate) fn float_to_json(v: Option<f64>) -> Value {
    match v {
        Some(n) => serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_error_names_the_pool_size_and_wait() {
        let msg = acquire_error(sqlx::Error::PoolTimedOut, 6, Duration::from_secs(10)).to_string();
        assert!(msg.contains("all 6 sessions"), "{msg}");
        assert!(msg.contains("waited 10s"), "{msg}");
    }

    #[test]
    fn float_to_json_drops_non_finite() {
        assert_eq!(float_to_json(Some(1.5)), serde_json::json!(1.5));
        assert_eq!(float_to_json(Some(f64::INFINITY)), Value::Null);
        assert_eq!(float_to_json(None), Value::Null);
    }
}
