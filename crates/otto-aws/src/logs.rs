//! CloudWatch Logs — log groups, streams, filtered events (tail = poll forward
//! from the newest timestamp seen) and Logs Insights queries. Read-only: every
//! call is a `logs describe-*` / `filter-log-events` / `start-query` /
//! `get-query-results` / `stop-query` (an Insights query reads, it never
//! writes). Insights results reuse Athena's DB-Explorer `QueryResult` shape so
//! the UI renders them with the same `ResultsGrid`.

use otto_core::{Error, Result};
use otto_state::AwsAccountRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::AwsService;
use crate::athena::{Column, QueryResult, QueryStats};

/// Default / hard cap of items per page.
pub const PAGE_DEFAULT: u32 = 200;
pub const PAGE_CAP: u32 = 1000;
/// Insights `--limit` cap (the service's own maximum is 10 000).
pub const INSIGHTS_LIMIT_CAP: u32 = 10_000;
/// Insights range cap: a query over more than 30 days is almost always a typo
/// and scans (bills) a lot.
pub const INSIGHTS_MAX_RANGE_MS: i64 = 31 * 24 * 3600 * 1000;
/// At most this many log groups per Insights query (service limit is 50).
pub const INSIGHTS_MAX_GROUPS: usize = 50;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogGroup {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupsResp {
    pub groups: Vec<LogGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogStream {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_event_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamsResp {
    pub streams: Vec<LogStream>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogEvent {
    /// Unique per event — the UI dedupes tail polls on it.
    pub id: String,
    pub stream: String,
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_time: Option<i64>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventsResp {
    pub events: Vec<LogEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GroupsQuery {
    pub prefix: Option<String>,
    pub token: Option<String>,
    pub max: Option<u32>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct StreamsQuery {
    pub group: String,
    pub prefix: Option<String>,
    pub token: Option<String>,
    pub max: Option<u32>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct EventsQuery {
    pub group: String,
    /// Comma-separated stream names (max 100); empty = every stream.
    pub streams: Option<String>,
    /// CloudWatch filter pattern (`ERROR`, `{ $.level = "error" }`, …).
    pub pattern: Option<String>,
    /// Epoch milliseconds, inclusive.
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub token: Option<String>,
    pub max: Option<u32>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InsightsReq {
    pub groups: Vec<String>,
    pub query: String,
    /// Epoch milliseconds.
    pub start: i64,
    pub end: i64,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsightsStartedResp {
    pub query_id: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RegionQuery {
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsightsResultsResp {
    /// `Scheduled` | `Running` | `Complete` | `Failed` | `Cancelled` | `Timeout` | `Unknown`.
    pub status: String,
    /// True once the status is terminal (stop polling).
    pub done: bool,
    pub result: QueryResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records_matched: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records_scanned: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_scanned: Option<f64>,
}

// ---------------------------------------------------------------------------
// Validation (pure)
// ---------------------------------------------------------------------------

/// Log group names: 1–512 chars of `[A-Za-z0-9._\-/#]`.
pub fn validate_group(g: &str) -> Result<()> {
    if g.is_empty()
        || g.len() > 512
        || !g
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '#'))
    {
        return Err(Error::Invalid(format!(
            "invalid log group name '{g}' — pick one from the log group list"
        )));
    }
    Ok(())
}

fn no_control(label: &str, v: &str, max: usize) -> Result<()> {
    if v.len() > max || v.chars().any(|c| c.is_control() && c != '\t') {
        return Err(Error::Invalid(format!(
            "{label} must be at most {max} chars without control characters"
        )));
    }
    Ok(())
}

/// Split + validate the `streams=` list (stream names may contain anything but
/// `:` and `*`; the API takes at most 100).
pub fn parse_streams(raw: Option<&str>) -> Result<Vec<String>> {
    let list: Vec<String> = raw
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if list.len() > 100 {
        return Err(Error::Invalid(
            "at most 100 log streams per request — narrow the selection".into(),
        ));
    }
    for s in &list {
        if s.len() > 512 || s.contains(':') || s.contains('*') || s.chars().any(char::is_control) {
            return Err(Error::Invalid(format!("invalid log stream name '{s}'")));
        }
    }
    Ok(list)
}

pub fn validate_insights(req: &InsightsReq) -> Result<()> {
    if req.groups.is_empty() {
        return Err(Error::Invalid(
            "pick at least one log group for the Insights query".into(),
        ));
    }
    if req.groups.len() > INSIGHTS_MAX_GROUPS {
        return Err(Error::Invalid(format!(
            "Insights queries take at most {INSIGHTS_MAX_GROUPS} log groups"
        )));
    }
    for g in &req.groups {
        validate_group(g)?;
    }
    if req.query.trim().is_empty() {
        return Err(Error::Invalid(
            "the Insights query is empty — e.g. `fields @timestamp, @message | sort @timestamp desc | limit 100`".into(),
        ));
    }
    no_control("query", &req.query.replace(['\n', '\r'], " "), 10_000)?;
    if req.end <= req.start {
        return Err(Error::Invalid(
            "the time range is empty — the end must be after the start".into(),
        ));
    }
    if req.end - req.start > INSIGHTS_MAX_RANGE_MS {
        return Err(Error::Invalid(
            "the Insights time range is capped at 31 days — narrow it (every scanned byte is billed)"
                .into(),
        ));
    }
    Ok(())
}

pub fn validate_query_id(q: &str) -> Result<()> {
    if q.is_empty() || q.len() > 256 || !q.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(Error::Invalid(format!("invalid Insights query id '{q}'")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Normalizers (pure)
// ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}
fn i(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(|x| x.as_i64())
}
fn next_token(v: &Value) -> Option<String> {
    s(v, "NextToken")
        .or_else(|| s(v, "nextToken"))
        .filter(|t| !t.is_empty())
}

pub fn normalize_groups(v: &Value) -> GroupsResp {
    let groups = v
        .get("logGroups")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|g| {
                    Some(LogGroup {
                        name: s(g, "logGroupName")?,
                        arn: s(g, "arn"),
                        created_ms: i(g, "creationTime"),
                        retention_days: i(g, "retentionInDays"),
                        stored_bytes: g.get("storedBytes").and_then(|x| x.as_u64()),
                        class: s(g, "logGroupClass"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    GroupsResp {
        groups,
        next_token: next_token(v),
    }
}

pub fn normalize_streams(v: &Value) -> StreamsResp {
    let streams = v
        .get("logStreams")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|st| {
                    Some(LogStream {
                        name: s(st, "logStreamName")?,
                        created_ms: i(st, "creationTime"),
                        first_event_ms: i(st, "firstEventTimestamp"),
                        last_event_ms: i(st, "lastEventTimestamp"),
                        stored_bytes: st.get("storedBytes").and_then(|x| x.as_u64()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    StreamsResp {
        streams,
        next_token: next_token(v),
    }
}

pub fn normalize_events(v: &Value) -> EventsResp {
    let events = v
        .get("events")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let stream = s(e, "logStreamName").unwrap_or_default();
                    let timestamp = i(e, "timestamp")?;
                    let id = s(e, "eventId").unwrap_or_else(|| format!("{stream}:{timestamp}"));
                    Some(LogEvent {
                        id,
                        stream,
                        timestamp,
                        ingestion_time: i(e, "ingestionTime"),
                        message: s(e, "message").unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    EventsResp {
        events,
        next_token: next_token(v),
    }
}

/// `get-query-results` → `QueryResult`. Columns are the union of field names
/// in first-seen order, minus the internal `@ptr` pointer.
pub fn normalize_insights(v: &Value) -> InsightsResultsResp {
    let status = s(v, "status").unwrap_or_else(|| "Unknown".into());
    let done = matches!(
        status.as_str(),
        "Complete" | "Failed" | "Cancelled" | "Timeout"
    );
    let raw_rows: Vec<&Vec<Value>> = v
        .get("results")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|r| r.as_array()).collect())
        .unwrap_or_default();
    let mut columns: Vec<String> = Vec::new();
    for row in &raw_rows {
        for cell in row.iter() {
            if let Some(f) = cell.get("field").and_then(|f| f.as_str()) {
                if f != "@ptr" && !columns.iter().any(|c| c == f) {
                    columns.push(f.to_string());
                }
            }
        }
    }
    let rows: Vec<Vec<Value>> = raw_rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|c| {
                    row.iter()
                        .find(|cell| cell.get("field").and_then(|f| f.as_str()) == Some(c))
                        .and_then(|cell| cell.get("value").cloned())
                        .unwrap_or(Value::Null)
                })
                .collect()
        })
        .collect();
    let stat = |k: &str| {
        v.get("statistics")
            .and_then(|st| st.get(k))
            .and_then(|x| x.as_f64())
    };
    let bytes_scanned = stat("bytesScanned");
    InsightsResultsResp {
        status,
        done,
        result: QueryResult {
            columns: columns
                .into_iter()
                .map(|name| Column {
                    name,
                    type_hint: None,
                })
                .collect(),
            stats: QueryStats {
                duration_ms: 0,
                row_count: rows.len(),
                bytes_read: bytes_scanned.map(|b| b as u64),
            },
            rows,
            truncated: false,
        },
        records_matched: stat("recordsMatched"),
        records_scanned: stat("recordsScanned"),
        bytes_scanned,
    }
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

fn page(max: Option<u32>) -> String {
    max.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_CAP).to_string()
}

pub async fn list_groups(
    svc: &AwsService,
    a: &AwsAccountRow,
    q: &GroupsQuery,
) -> Result<GroupsResp> {
    let max = page(q.max);
    let mut args: Vec<&str> = vec!["logs", "describe-log-groups", "--max-items", &max];
    let prefix = q.prefix.as_deref().map(str::trim).unwrap_or("");
    if !prefix.is_empty() {
        validate_group(prefix)?;
        args.extend(["--log-group-name-prefix", prefix]);
    }
    if let Some(t) = q.token.as_deref().filter(|t| !t.is_empty()) {
        args.extend(["--starting-token", t]);
    }
    let v = svc.run_json(a, q.region.as_deref(), &args).await?;
    Ok(normalize_groups(&v))
}

pub async fn list_streams(
    svc: &AwsService,
    a: &AwsAccountRow,
    q: &StreamsQuery,
) -> Result<StreamsResp> {
    validate_group(&q.group)?;
    let max = page(q.max);
    let mut args: Vec<&str> = vec![
        "logs",
        "describe-log-streams",
        "--log-group-name",
        &q.group,
        "--max-items",
        &max,
    ];
    let prefix = q.prefix.as_deref().map(str::trim).unwrap_or("");
    if prefix.is_empty() {
        // Newest first — the stream a user wants is almost always the latest.
        args.extend(["--order-by", "LastEventTime", "--descending"]);
    } else {
        // The API refuses a name prefix combined with LastEventTime ordering.
        no_control("stream prefix", prefix, 512)?;
        args.extend(["--log-stream-name-prefix", prefix]);
    }
    if let Some(t) = q.token.as_deref().filter(|t| !t.is_empty()) {
        args.extend(["--starting-token", t]);
    }
    let v = svc.run_json(a, q.region.as_deref(), &args).await?;
    Ok(normalize_streams(&v))
}

pub async fn filter_events(
    svc: &AwsService,
    a: &AwsAccountRow,
    q: &EventsQuery,
) -> Result<EventsResp> {
    validate_group(&q.group)?;
    let streams = parse_streams(q.streams.as_deref())?;
    let max = page(q.max);
    let start = q.start.map(|s| s.to_string());
    let end = q.end.map(|e| e.to_string());
    if let (Some(s), Some(e)) = (q.start, q.end) {
        if e < s {
            return Err(Error::Invalid(
                "the time range is empty — the end must be after the start".into(),
            ));
        }
    }
    let pattern = q.pattern.as_deref().map(str::trim).unwrap_or("");
    if !pattern.is_empty() {
        no_control("filter pattern", pattern, 1024)?;
    }
    let token = q.token.as_deref().filter(|t| !t.is_empty());
    // F2d: the tail ticks every 2–10 s — sign the call in-process (one pooled
    // HTTPS round trip) instead of starting a Python child each tick. A CLI
    // page token can only be resumed by the CLI, and vice versa.
    if token.is_none_or(|t| crate::native::native_token(t).is_some()) {
        if let Some(t) = svc.native_target(a, q.region.as_deref()).await? {
            let mut input = serde_json::Map::new();
            input.insert("logGroupName".into(), q.group.clone().into());
            if !streams.is_empty() {
                input.insert("logStreamNames".into(), streams.clone().into());
            }
            if !pattern.is_empty() {
                input.insert("filterPattern".into(), pattern.into());
            }
            if let Some(s) = q.start {
                input.insert("startTime".into(), s.into());
            }
            if let Some(e) = q.end {
                input.insert("endTime".into(), e.into());
            }
            if let Some(tok) = token.and_then(crate::native::native_token) {
                input.insert("nextToken".into(), tok.into());
            }
            let n = max.parse::<usize>().unwrap_or(1);
            let v = crate::native::filter_log_events(&t, input, n).await?;
            svc.touch(a).await;
            return Ok(normalize_events(&v));
        }
    }
    if token.is_some_and(|t| crate::native::native_token(t).is_some()) {
        return Err(crate::native::foreign_token_error());
    }
    let mut args: Vec<&str> = vec![
        "logs",
        "filter-log-events",
        "--log-group-name",
        &q.group,
        "--max-items",
        &max,
    ];
    if !streams.is_empty() {
        args.push("--log-stream-names");
        args.extend(streams.iter().map(String::as_str));
    }
    if !pattern.is_empty() {
        args.extend(["--filter-pattern", pattern]);
    }
    if let Some(s) = start.as_deref() {
        args.extend(["--start-time", s]);
    }
    if let Some(e) = end.as_deref() {
        args.extend(["--end-time", e]);
    }
    if let Some(t) = token {
        args.extend(["--starting-token", t]);
    }
    let v = svc.run_json(a, q.region.as_deref(), &args).await?;
    Ok(normalize_events(&v))
}

pub async fn start_insights(
    svc: &AwsService,
    a: &AwsAccountRow,
    req: &InsightsReq,
    region: Option<&str>,
) -> Result<InsightsStartedResp> {
    validate_insights(req)?;
    // `start-query` takes epoch SECONDS.
    let start = (req.start / 1000).to_string();
    let end = (req.end / 1000).max(req.start / 1000 + 1).to_string();
    let limit = req
        .limit
        .unwrap_or(1000)
        .clamp(1, INSIGHTS_LIMIT_CAP)
        .to_string();
    let mut args: Vec<&str> = vec!["logs", "start-query", "--log-group-names"];
    args.extend(req.groups.iter().map(String::as_str));
    args.extend([
        "--start-time",
        &start,
        "--end-time",
        &end,
        "--query-string",
        &req.query,
        "--limit",
        &limit,
    ]);
    let v = svc.run_json(a, region, &args).await?;
    let query_id = s(&v, "queryId")
        .ok_or_else(|| Error::Upstream("logs start-query returned no queryId".into()))?;
    Ok(InsightsStartedResp { query_id })
}

pub async fn insights_results(
    svc: &AwsService,
    a: &AwsAccountRow,
    qid: &str,
    region: Option<&str>,
) -> Result<InsightsResultsResp> {
    validate_query_id(qid)?;
    // F2d: the Insights status poll — in-process when static creds exist.
    if let Some(t) = svc.native_target(a, region).await? {
        let v = crate::native::json_call(
            &t,
            "logs",
            "Logs_20140328",
            "GetQueryResults",
            &serde_json::json!({ "queryId": qid }),
        )
        .await?;
        svc.touch(a).await;
        return Ok(normalize_insights(&v));
    }
    let v = svc
        .run_json(a, region, &["logs", "get-query-results", "--query-id", qid])
        .await?;
    Ok(normalize_insights(&v))
}

pub async fn stop_insights(
    svc: &AwsService,
    a: &AwsAccountRow,
    qid: &str,
    region: Option<&str>,
) -> Result<()> {
    validate_query_id(qid)?;
    svc.run(a, region, &["logs", "stop-query", "--query-id", qid])
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_and_streams_normalize() {
        let v: Value = serde_json::from_str(r#"{"logGroups":[{"logGroupName":"/aws/eks/prod/cluster","creationTime":1700000000000,"retentionInDays":30,"storedBytes":1234,"arn":"arn:aws:logs:eu-west-1:1:log-group:/aws/eks/prod/cluster:*","logGroupClass":"STANDARD"},{"nope":1}],"NextToken":"tok"}"#).unwrap();
        let g = normalize_groups(&v);
        assert_eq!(g.groups.len(), 1);
        assert_eq!(g.groups[0].name, "/aws/eks/prod/cluster");
        assert_eq!(g.groups[0].retention_days, Some(30));
        assert_eq!(g.next_token.as_deref(), Some("tok"));
        let v: Value = serde_json::from_str(r#"{"logStreams":[{"logStreamName":"kube-apiserver-abc","lastEventTimestamp":5,"firstEventTimestamp":1}]}"#).unwrap();
        let st = normalize_streams(&v);
        assert_eq!(st.streams[0].last_event_ms, Some(5));
        assert!(st.next_token.is_none());
    }

    #[test]
    fn events_normalize_and_fallback_id() {
        let v: Value = serde_json::from_str(r#"{"events":[{"logStreamName":"s1","timestamp":10,"message":"hello\n","ingestionTime":11,"eventId":"e1"},{"logStreamName":"s2","timestamp":12,"message":"x"},{"message":"no ts"}],"NextToken":""}"#).unwrap();
        let e = normalize_events(&v);
        assert_eq!(e.events.len(), 2);
        assert_eq!(e.events[0].id, "e1");
        assert_eq!(e.events[1].id, "s2:12");
        assert!(e.next_token.is_none(), "an empty token means no more pages");
    }

    #[test]
    fn insights_results_drop_ptr_and_align_columns() {
        let v: Value = serde_json::from_str(r#"{"status":"Complete","results":[[{"field":"@timestamp","value":"2026-10-03 09:00:00.000"},{"field":"@message","value":"boom"},{"field":"@ptr","value":"xx"}],[{"field":"@message","value":"ok"},{"field":"level","value":"info"}]],"statistics":{"recordsMatched":2.0,"recordsScanned":10.0,"bytesScanned":512.0}}"#).unwrap();
        let r = normalize_insights(&v);
        assert!(r.done);
        let cols: Vec<&str> = r.result.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(cols, ["@timestamp", "@message", "level"]);
        assert_eq!(r.result.rows[1][0], Value::Null);
        assert_eq!(r.result.rows[1][1], "ok");
        assert_eq!(r.result.stats.bytes_read, Some(512));
        assert_eq!(r.records_matched, Some(2.0));
        let running = normalize_insights(&serde_json::json!({"status":"Running","results":[]}));
        assert!(!running.done);
        assert!(running.result.columns.is_empty());
    }

    #[test]
    fn validation_explains_what_to_fix() {
        assert!(validate_group("/aws/lambda/my-fn").is_ok());
        assert!(validate_group("bad group").is_err());
        assert!(validate_group("").is_err());
        assert_eq!(
            parse_streams(Some("a, b,,c")).unwrap(),
            vec!["a".to_string(), "b".into(), "c".into()]
        );
        assert!(parse_streams(Some("a:b")).is_err());
        let many = (0..101)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_streams(Some(&many)).is_err());
        let ok = InsightsReq {
            groups: vec!["/g".into()],
            query: "fields @message".into(),
            start: 0,
            end: 60_000,
            limit: None,
        };
        assert!(validate_insights(&ok).is_ok());
        let mut bad = ok.clone();
        bad.groups.clear();
        assert!(validate_insights(&bad)
            .unwrap_err()
            .to_string()
            .contains("at least one log group"));
        let mut bad = ok.clone();
        bad.end = bad.start;
        assert!(validate_insights(&bad).is_err());
        let mut bad = ok.clone();
        bad.end = INSIGHTS_MAX_RANGE_MS + 1;
        assert!(validate_insights(&bad)
            .unwrap_err()
            .to_string()
            .contains("31 days"));
        assert!(validate_query_id("12ab-34cd").is_ok());
        assert!(validate_query_id("x y").is_err());
    }
}
