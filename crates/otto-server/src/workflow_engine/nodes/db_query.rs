//! The `db_query` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall {
        ctx, user, node, ..
    } = call;
    let p = &node.params;
    let conn_id: Id = p
        .get("connection_id")
        .and_then(Value::as_str)
        .ok_or_else(|| otto_core::Error::Invalid("db_query: missing connection_id".into()))?
        .to_string();
    let stmt = p
        .get("statement")
        .and_then(Value::as_str)
        .ok_or_else(|| otto_core::Error::Invalid("db_query: missing statement".into()))?
        .to_string();
    let max_rows = p.get("max_rows").and_then(Value::as_u64).unwrap_or(100) as usize;
    let req = QueryRequest {
        statement: stmt.clone(),
        max_rows: Some(max_rows),
        // Deliberately leave confirm_write = false (default): the
        // workflow engine must never silently issue writes. A graph that
        // genuinely needs a write can set the param explicitly.
        confirm_write: false,
        ..Default::default()
    };
    let result = ctx
        .db_explorer
        .run(&conn_id, &user.id, &req)
        .await
        .map_err(|e| otto_core::Error::Upstream(format!("db_query: {e}")))?;
    let rows_returned = result.rows.len();
    let out = json!({
        "columns": result.columns,
        "rows": result.rows,
        "rows_returned": rows_returned,
        "truncated": result.truncated,
    });
    Ok((
        out,
        vec![format!("db_query: {rows_returned} rows returned")],
    ))
}
