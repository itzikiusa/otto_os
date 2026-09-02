//! Athena — catalog browsing / history / results / cancel (View), execute
//! (Edit) (§2.5). Results are converted to the DB Explorer `QueryResult`
//! shape so `ResultsGrid` renders them unchanged.

use otto_core::{Error, Result};
use otto_state::AwsAccountRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::AwsService;

/// Fan-out cap for per-workgroup `get-work-group` calls.
const WORKGROUP_DETAIL_CAP: usize = 20;
/// `batch-get-query-execution` accepts at most 50 ids.
const HISTORY_BATCH: usize = 50;

// ---------------------------------------------------------------------------
// DB Explorer `QueryResult` (identical serde shape to
// `otto_dbviewer::types::{QueryResult, Column, QueryStats}` — duplicated here
// so this crate does not pull the DB drivers in).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_hint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryStats {
    pub duration_ms: u64,
    pub row_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_read: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueryResult {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    pub stats: QueryStats,
    #[serde(default)]
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Workgroup {
    pub name: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_location: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkgroupsResp {
    pub workgroups: Vec<Workgroup>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabasesResp {
    pub databases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TableColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub col_type: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    #[serde(rename = "type")]
    pub table_type: String,
    pub columns: Vec<TableColumn>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TablesResp {
    pub tables: Vec<Table>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Execution {
    pub id: String,
    pub query: String,
    pub state: String,
    pub submitted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_scanned_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryResp {
    pub executions: Vec<Execution>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueryReq {
    pub sql: String,
    pub database: Option<String>,
    pub workgroup: Option<String>,
    pub output_location: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryStartedResp {
    pub query_execution_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct ExecStats {
    pub data_scanned_bytes: u64,
    pub execution_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AthenaQueryStatus {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub stats: ExecStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<QueryResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CatalogQuery {
    pub catalog: Option<String>,
    pub database: Option<String>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HistoryQuery {
    pub workgroup: Option<String>,
    pub max: Option<usize>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct StatusQuery {
    pub token: Option<String>,
    pub max: Option<u32>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RegionQuery {
    pub region: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure
// ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

pub fn normalize_workgroups(v: &Value) -> Vec<Workgroup> {
    v.get("WorkGroups")
        .and_then(|w| w.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|w| {
                    Some(Workgroup {
                        name: s(w, "Name")?,
                        state: s(w, "State").unwrap_or_else(|| "ENABLED".into()),
                        output_location: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `get-work-group` → its configured output location, if any.
pub fn workgroup_output_location(v: &Value) -> Option<String> {
    v.get("WorkGroup")?
        .get("Configuration")?
        .get("ResultConfiguration")?
        .get("OutputLocation")?
        .as_str()
        .map(str::to_string)
}

pub fn normalize_databases(v: &Value) -> Vec<String> {
    v.get("DatabaseList")
        .and_then(|d| d.as_array())
        .map(|arr| arr.iter().filter_map(|d| s(d, "Name")).collect())
        .unwrap_or_default()
}

/// `list-table-metadata` → tables with columns + partition keys (partition
/// keys are appended so the tree shows every queryable column).
pub fn normalize_tables(v: &Value) -> Vec<Table> {
    let cols = |arr: Option<&Value>| -> Vec<TableColumn> {
        arr.and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        Some(TableColumn {
                            name: s(c, "Name")?,
                            col_type: s(c, "Type").unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    v.get("TableMetadataList")
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let mut columns = cols(t.get("Columns"));
                    columns.extend(cols(t.get("PartitionKeys")));
                    Some(Table {
                        name: s(t, "Name")?,
                        table_type: s(t, "TableType").unwrap_or_else(|| "EXTERNAL_TABLE".into()),
                        columns,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn exec_from(e: &Value) -> Option<Execution> {
    let status = e.get("Status").cloned().unwrap_or(Value::Null);
    let stats = e.get("Statistics").cloned().unwrap_or(Value::Null);
    Some(Execution {
        id: s(e, "QueryExecutionId")?,
        query: s(e, "Query").unwrap_or_default(),
        state: s(&status, "State").unwrap_or_else(|| "UNKNOWN".into()),
        submitted_at: s(&status, "SubmissionDateTime"),
        completed_at: s(&status, "CompletionDateTime"),
        data_scanned_bytes: stats.get("DataScannedInBytes").and_then(|x| x.as_u64()),
        execution_ms: stats
            .get("TotalExecutionTimeInMillis")
            .and_then(|x| x.as_u64()),
    })
}

/// `batch-get-query-execution` → history rows (the API returns them in the
/// order of the ids passed, which `list-query-executions` gives newest-first).
pub fn normalize_executions(v: &Value) -> Vec<Execution> {
    v.get("QueryExecutions")
        .and_then(|q| q.as_array())
        .map(|arr| arr.iter().filter_map(exec_from).collect())
        .unwrap_or_default()
}

/// `get-query-execution` → state / reason / stats.
pub fn normalize_status(v: &Value) -> AthenaQueryStatus {
    let e = v.get("QueryExecution").cloned().unwrap_or(Value::Null);
    let status = e.get("Status").cloned().unwrap_or(Value::Null);
    let stats = e.get("Statistics").cloned().unwrap_or(Value::Null);
    AthenaQueryStatus {
        state: s(&status, "State").unwrap_or_else(|| "UNKNOWN".into()),
        reason: s(&status, "StateChangeReason"),
        stats: ExecStats {
            data_scanned_bytes: stats
                .get("DataScannedInBytes")
                .and_then(|x| x.as_u64())
                .unwrap_or(0),
            execution_ms: stats
                .get("TotalExecutionTimeInMillis")
                .and_then(|x| x.as_u64())
                .unwrap_or(0),
        },
        result: None,
        next_token: None,
    }
}

/// `get-query-results` → `QueryResult`. `first_page` drops the header row
/// Athena emits as row 0 of the first page. Cells stay strings (Athena
/// returns everything as `VarCharValue`); missing cells are `null`.
pub fn results_to_query_result(
    v: &Value,
    first_page: bool,
    duration_ms: u64,
    bytes_read: u64,
) -> (QueryResult, Option<String>) {
    let rs = v.get("ResultSet").cloned().unwrap_or(Value::Null);
    let columns: Vec<Column> = rs
        .get("ResultSetMetadata")
        .and_then(|m| m.get("ColumnInfo"))
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .map(|c| Column {
                    name: s(c, "Name").or_else(|| s(c, "Label")).unwrap_or_default(),
                    type_hint: s(c, "Type"),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut rows: Vec<Vec<Value>> = rs
        .get("Rows")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| {
                    r.get("Data")
                        .and_then(|d| d.as_array())
                        .map(|cells| {
                            cells
                                .iter()
                                .map(|c| c.get("VarCharValue").cloned().unwrap_or(Value::Null))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    if first_page && !rows.is_empty() {
        // Header row: every cell equals the column name.
        let is_header = rows[0].len() == columns.len()
            && rows[0]
                .iter()
                .zip(&columns)
                .all(|(cell, col)| cell.as_str() == Some(col.name.as_str()));
        if is_header {
            rows.remove(0);
        }
    }
    let next_token = s(v, "NextToken");
    let row_count = rows.len();
    (
        QueryResult {
            columns,
            rows,
            stats: QueryStats {
                duration_ms,
                row_count,
                bytes_read: Some(bytes_read),
            },
            truncated: next_token.is_some(),
        },
        next_token,
    )
}

fn validate_ident(kind: &str, v: &str) -> Result<()> {
    if v.is_empty()
        || v.len() > 256
        || !v
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@' | ':' | '/'))
    {
        return Err(Error::Invalid(format!("invalid {kind} '{v}'")));
    }
    Ok(())
}

pub fn validate_qid(q: &str) -> Result<()> {
    if q.is_empty() || q.len() > 64 || !q.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(Error::Invalid(format!("invalid query execution id '{q}'")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

pub async fn workgroups(
    svc: &AwsService,
    a: &AwsAccountRow,
    region: Option<&str>,
) -> Result<WorkgroupsResp> {
    let v = svc
        .run_json(a, region, &["athena", "list-work-groups"])
        .await?;
    let mut wgs = normalize_workgroups(&v);
    for wg in wgs.iter_mut().take(WORKGROUP_DETAIL_CAP) {
        if let Ok(d) = svc
            .run_json(
                a,
                region,
                &["athena", "get-work-group", "--work-group", &wg.name],
            )
            .await
        {
            wg.output_location = workgroup_output_location(&d);
        }
    }
    Ok(WorkgroupsResp { workgroups: wgs })
}

pub async fn databases(
    svc: &AwsService,
    a: &AwsAccountRow,
    q: &CatalogQuery,
) -> Result<DatabasesResp> {
    let catalog = q.catalog.clone().unwrap_or_else(|| "AwsDataCatalog".into());
    validate_ident("catalog", &catalog)?;
    let v = svc
        .run_json(
            a,
            q.region.as_deref(),
            &["athena", "list-databases", "--catalog-name", &catalog],
        )
        .await?;
    Ok(DatabasesResp {
        databases: normalize_databases(&v),
    })
}

pub async fn tables(svc: &AwsService, a: &AwsAccountRow, q: &CatalogQuery) -> Result<TablesResp> {
    let catalog = q.catalog.clone().unwrap_or_else(|| "AwsDataCatalog".into());
    let database = q
        .database
        .clone()
        .ok_or_else(|| Error::Invalid("database is required".into()))?;
    validate_ident("catalog", &catalog)?;
    validate_ident("database", &database)?;
    let v = svc
        .run_json(
            a,
            q.region.as_deref(),
            &[
                "athena",
                "list-table-metadata",
                "--catalog-name",
                &catalog,
                "--database-name",
                &database,
            ],
        )
        .await?;
    Ok(TablesResp {
        tables: normalize_tables(&v),
    })
}

pub async fn history(svc: &AwsService, a: &AwsAccountRow, q: &HistoryQuery) -> Result<HistoryResp> {
    let max = q.max.unwrap_or(50).clamp(1, HISTORY_BATCH).to_string();
    let mut args = vec![
        "athena",
        "list-query-executions",
        "--max-items",
        max.as_str(),
    ];
    if let Some(w) = q.workgroup.as_deref().filter(|s| !s.is_empty()) {
        validate_ident("workgroup", w)?;
        args.extend(["--work-group", w]);
    }
    let v = svc.run_json(a, q.region.as_deref(), &args).await?;
    let ids: Vec<String> = v
        .get("QueryExecutionIds")
        .and_then(|i| i.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Ok(HistoryResp { executions: vec![] });
    }
    let mut args: Vec<&str> = vec![
        "athena",
        "batch-get-query-execution",
        "--query-execution-ids",
    ];
    args.extend(ids.iter().take(HISTORY_BATCH).map(String::as_str));
    let v = svc.run_json(a, q.region.as_deref(), &args).await?;
    Ok(HistoryResp {
        executions: normalize_executions(&v),
    })
}

pub async fn start_query(
    svc: &AwsService,
    a: &AwsAccountRow,
    req: &QueryReq,
    region: Option<&str>,
) -> Result<QueryStartedResp> {
    let sql = req.sql.trim();
    if sql.is_empty() || sql.len() > 256 * 1024 {
        return Err(Error::Invalid("sql must be 1 byte .. 256 KiB".into()));
    }
    let workgroup = req
        .workgroup
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(w) = workgroup {
        validate_ident("workgroup", w)?;
    }
    let output_location = req
        .output_location
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(o) = output_location {
        if !o.starts_with("s3://") {
            return Err(Error::Invalid(
                "output_location must be an s3:// URI".into(),
            ));
        }
    } else {
        // Athena refuses to start without a result location; give the hint
        // up front instead of surfacing the raw InvalidRequestException.
        let wg = workgroup.unwrap_or("primary");
        let d = svc
            .run_json(a, region, &["athena", "get-work-group", "--work-group", wg])
            .await?;
        if workgroup_output_location(&d).is_none() {
            return Err(Error::Invalid(format!(
                "workgroup '{wg}' has no query result location — pass output_location (s3://bucket/prefix/) or configure one on the workgroup"
            )));
        }
    }
    let ctx;
    let rc;
    let mut args: Vec<&str> = vec!["athena", "start-query-execution", "--query-string", sql];
    if let Some(db) = req
        .database
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        validate_ident("database", db)?;
        ctx = format!("Database={db}");
        args.extend(["--query-execution-context", ctx.as_str()]);
    }
    if let Some(w) = workgroup {
        args.extend(["--work-group", w]);
    }
    if let Some(o) = output_location {
        rc = format!("OutputLocation={o}");
        args.extend(["--result-configuration", rc.as_str()]);
    }
    let v = svc.run_json(a, region, &args).await?;
    let query_execution_id = v
        .get("QueryExecutionId")
        .and_then(|q| q.as_str())
        .ok_or_else(|| {
            Error::Upstream("start-query-execution returned no QueryExecutionId".into())
        })?
        .to_string();
    Ok(QueryStartedResp { query_execution_id })
}

pub async fn status(
    svc: &AwsService,
    a: &AwsAccountRow,
    qid: &str,
    q: &StatusQuery,
) -> Result<AthenaQueryStatus> {
    validate_qid(qid)?;
    let region = q.region.as_deref();
    let v = svc
        .run_json(
            a,
            region,
            &["athena", "get-query-execution", "--query-execution-id", qid],
        )
        .await?;
    let mut st = normalize_status(&v);
    if st.state == "SUCCEEDED" {
        let max = q.max.unwrap_or(1000).clamp(1, 1000).to_string();
        let mut args = vec![
            "athena",
            "get-query-results",
            "--query-execution-id",
            qid,
            "--max-items",
            max.as_str(),
        ];
        let token = q.token.as_deref().filter(|t| !t.is_empty());
        if let Some(t) = token {
            args.extend(["--starting-token", t]);
        }
        let r = svc.run_json(a, region, &args).await?;
        let (result, next) = results_to_query_result(
            &r,
            token.is_none(),
            st.stats.execution_ms,
            st.stats.data_scanned_bytes,
        );
        st.result = Some(result);
        st.next_token = next;
    }
    Ok(st)
}

pub async fn cancel(
    svc: &AwsService,
    a: &AwsAccountRow,
    qid: &str,
    region: Option<&str>,
) -> Result<()> {
    validate_qid(qid)?;
    svc.run(
        a,
        region,
        &[
            "athena",
            "stop-query-execution",
            "--query-execution-id",
            qid,
        ],
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workgroups_and_output_location() {
        let v: Value = serde_json::from_str(
            r#"{"WorkGroups": [{"Name": "primary", "State": "ENABLED", "CreationTime": "2020-01-01T00:00:00+00:00"}, {"Name": "analytics", "State": "DISABLED"}]}"#,
        )
        .unwrap();
        let w = normalize_workgroups(&v);
        assert_eq!(w[0].name, "primary");
        assert_eq!(w[1].state, "DISABLED");
        let d: Value = serde_json::from_str(
            r#"{"WorkGroup": {"Name": "primary", "State": "ENABLED", "Configuration": {"ResultConfiguration": {"OutputLocation": "s3://athena-results/primary/"}, "EnforceWorkGroupConfiguration": false}}}"#,
        )
        .unwrap();
        assert_eq!(
            workgroup_output_location(&d).as_deref(),
            Some("s3://athena-results/primary/")
        );
        assert!(workgroup_output_location(
            &serde_json::json!({"WorkGroup": {"Configuration": {}}})
        )
        .is_none());
    }

    #[test]
    fn databases_and_tables() {
        let v: Value = serde_json::from_str(
            r#"{"DatabaseList": [{"Name": "default"}, {"Name": "logs", "Description": "x"}]}"#,
        )
        .unwrap();
        assert_eq!(normalize_databases(&v), vec!["default", "logs"]);
        let t: Value = serde_json::from_str(
            r#"{"TableMetadataList": [{"Name": "events", "CreateTime": "2024-01-01T00:00:00+00:00", "TableType": "EXTERNAL_TABLE", "Columns": [{"Name": "id", "Type": "bigint"}, {"Name": "payload", "Type": "string"}], "PartitionKeys": [{"Name": "dt", "Type": "string"}], "Parameters": {"EXTERNAL": "TRUE"}}]}"#,
        )
        .unwrap();
        let tables = normalize_tables(&t);
        assert_eq!(tables[0].name, "events");
        assert_eq!(tables[0].columns.len(), 3, "partition keys appended");
        assert_eq!(tables[0].columns[2].name, "dt");
        let j = serde_json::to_value(&tables[0]).unwrap();
        assert_eq!(j["type"], "EXTERNAL_TABLE");
        assert_eq!(j["columns"][0]["type"], "bigint");
    }

    #[test]
    fn history_and_status_normalize() {
        let v: Value = serde_json::from_str(
            r#"{"QueryExecutions": [{"QueryExecutionId": "q1", "Query": "select 1", "StatementType": "DML", "Status": {"State": "SUCCEEDED", "SubmissionDateTime": "2024-06-01T10:00:00+00:00", "CompletionDateTime": "2024-06-01T10:00:02+00:00"}, "Statistics": {"EngineExecutionTimeInMillis": 1500, "DataScannedInBytes": 2048, "TotalExecutionTimeInMillis": 2100}}], "UnprocessedQueryExecutionIds": []}"#,
        )
        .unwrap();
        let ex = normalize_executions(&v);
        assert_eq!(ex[0].id, "q1");
        assert_eq!(ex[0].state, "SUCCEEDED");
        assert_eq!(ex[0].data_scanned_bytes, Some(2048));
        assert_eq!(ex[0].execution_ms, Some(2100));

        let st: Value = serde_json::from_str(
            r#"{"QueryExecution": {"QueryExecutionId": "q2", "Status": {"State": "FAILED", "StateChangeReason": "SYNTAX_ERROR: line 1:8: Column 'x' cannot be resolved", "AthenaError": {"ErrorCategory": 2}}, "Statistics": {"TotalExecutionTimeInMillis": 300, "DataScannedInBytes": 0}}}"#,
        )
        .unwrap();
        let s = normalize_status(&st);
        assert_eq!(s.state, "FAILED");
        assert!(s.reason.unwrap().starts_with("SYNTAX_ERROR"));
        assert_eq!(s.stats.execution_ms, 300);
    }

    #[test]
    fn results_convert_to_query_result_and_drop_header() {
        let v: Value = serde_json::from_str(
            r#"{"UpdateCount": 0, "ResultSet": {"Rows": [
                {"Data": [{"VarCharValue": "id"}, {"VarCharValue": "name"}]},
                {"Data": [{"VarCharValue": "1"}, {"VarCharValue": "alice"}]},
                {"Data": [{"VarCharValue": "2"}, {}]}
              ], "ResultSetMetadata": {"ColumnInfo": [
                {"CatalogName": "hive", "Name": "id", "Label": "id", "Type": "bigint", "Precision": 19, "Nullable": "UNKNOWN"},
                {"CatalogName": "hive", "Name": "name", "Label": "name", "Type": "varchar", "Precision": 2147483647, "Nullable": "UNKNOWN"}
              ]}}, "NextToken": "tok2"}"#,
        )
        .unwrap();
        let (r, next) = results_to_query_result(&v, true, 2100, 2048);
        assert_eq!(r.columns.len(), 2);
        assert_eq!(r.columns[0].type_hint.as_deref(), Some("bigint"));
        assert_eq!(r.rows.len(), 2, "header dropped");
        assert_eq!(r.rows[0], vec![Value::from("1"), Value::from("alice")]);
        assert_eq!(r.rows[1][1], Value::Null);
        assert_eq!(r.stats.row_count, 2);
        assert_eq!(r.stats.bytes_read, Some(2048));
        assert!(r.truncated);
        assert_eq!(next.as_deref(), Some("tok2"));
        // Serialized shape matches the DB Explorer contract keys.
        let j = serde_json::to_value(&r).unwrap();
        assert!(j["columns"][0].get("type_hint").is_some());
        assert_eq!(j["stats"]["duration_ms"], 2100);

        // Second page: header not dropped even if the first cell matches.
        let page2: Value = serde_json::json!({"ResultSet": {"Rows": [{"Data": [{"VarCharValue": "id"}, {"VarCharValue": "name"}]}], "ResultSetMetadata": {"ColumnInfo": [{"Name": "id", "Type": "bigint"}, {"Name": "name", "Type": "varchar"}]}}});
        let (r2, next2) = results_to_query_result(&page2, false, 0, 0);
        assert_eq!(r2.rows.len(), 1);
        assert!(next2.is_none() && !r2.truncated);
    }

    #[test]
    fn validators() {
        assert!(validate_qid("6b4f1b8e-1c2d-4e5f-8a9b-0c1d2e3f4a5b").is_ok());
        assert!(validate_qid("x y").is_err());
        assert!(validate_ident("database", "my_db-1").is_ok());
        assert!(validate_ident("database", "db; drop").is_err());
    }
}
