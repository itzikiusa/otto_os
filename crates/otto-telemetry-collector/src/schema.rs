//! Owned telemetry rollups. Upstream owns raw OTLP columns, all other stores are untouched.
use crate::TelemetryConfig;
use anyhow::Result;
use otto_usage::ClickHouse;
pub(crate) async fn ensure(ch: &ClickHouse, c: &TelemetryConfig) -> Result<()> {
    ch.exec(&format!("CREATE TABLE IF NOT EXISTS otto_telemetry.operation_minutes (minute DateTime, component LowCardinality(String), name LowCardinality(String), calls AggregateFunction(count), errors AggregateFunction(sum, UInt64), duration AggregateFunction(sum, Float64), maximum AggregateFunction(max, Float64), quantiles AggregateFunction(quantilesTDigest(0.5,0.95), Float64), trace AggregateFunction(argMax, String, UInt64)) ENGINE=AggregatingMergeTree ORDER BY (component,name,minute) PARTITION BY toDate(minute) TTL minute + INTERVAL {} DAY",c.traces_days)).await?;
    ch.exec("CREATE MATERIALIZED VIEW IF NOT EXISTS otto_telemetry.operation_minutes_mv TO otto_telemetry.operation_minutes AS SELECT toStartOfMinute(Timestamp) minute, SpanAttributes['otto.component'] component, SpanName name, countState() calls, sumState(toUInt64(StatusCode='Error')) errors, sumState(toFloat64(Duration)/1e6) duration, maxState(toFloat64(Duration)/1e6) maximum, quantilesTDigestState(0.5,0.95)(toFloat64(Duration)/1e6) quantiles, argMaxState(TraceId,Duration) trace FROM otto_telemetry.otel_traces GROUP BY minute,component,name").await?;
    ch.exec(&format!("CREATE TABLE IF NOT EXISTS otto_telemetry.resource_minutes (minute DateTime, process LowCardinality(String), metric LowCardinality(String), value AggregateFunction(max, Float64)) ENGINE=AggregatingMergeTree ORDER BY (metric,process,minute) PARTITION BY toDate(minute) TTL minute + INTERVAL {} DAY",c.metrics_days)).await?;
    ch.exec("CREATE MATERIALIZED VIEW IF NOT EXISTS otto_telemetry.resource_minutes_mv TO otto_telemetry.resource_minutes AS SELECT toStartOfMinute(TimeUnix) minute, Attributes['otto.process'] process, MetricName metric, maxState(Value) value FROM otto_telemetry.otel_metrics_gauge GROUP BY minute,process,metric").await?;
    ch.exec(&format!("CREATE TABLE IF NOT EXISTS otto_telemetry.spike_minutes (minute DateTime, process LowCardinality(String), calls AggregateFunction(count), cpu AggregateFunction(max, Float64), rss AggregateFunction(max, Float64)) ENGINE=AggregatingMergeTree ORDER BY (process,minute) PARTITION BY toDate(minute) TTL minute + INTERVAL {} DAY",c.logs_days)).await?;
    ch.exec("CREATE MATERIALIZED VIEW IF NOT EXISTS otto_telemetry.spike_minutes_mv TO otto_telemetry.spike_minutes AS SELECT toStartOfMinute(Timestamp) minute, LogAttributes['otto.process'] process,countState() calls,maxState(toFloat64OrZero(LogAttributes['otto.cpu.percent'])) cpu,maxState(toFloat64OrZero(LogAttributes['otto.rss.mb'])) rss FROM otto_telemetry.otel_logs WHERE Body='resource.spike' GROUP BY minute,process").await?;
    for (table, column, days) in [
        ("otel_traces", "Timestamp", c.traces_days),
        ("otel_traces_trace_id_ts", "Start", c.traces_days),
        ("otel_logs", "Timestamp", c.logs_days),
        ("otel_metrics_gauge", "TimeUnix", c.metrics_days),
        ("otel_metrics_sum", "TimeUnix", c.metrics_days),
        ("otel_metrics_histogram", "TimeUnix", c.metrics_days),
        (
            "otel_metrics_exponential_histogram",
            "TimeUnix",
            c.metrics_days,
        ),
        ("otel_metrics_summary", "TimeUnix", c.metrics_days),
        ("operation_minutes", "minute", c.traces_days),
        ("resource_minutes", "minute", c.metrics_days),
        ("spike_minutes", "minute", c.logs_days),
    ] {
        // DateTime64 TTL must be cast to DateTime. Existing telemetry retention changes apply in place.
        ch.exec(&format!("ALTER TABLE otto_telemetry.{table} MODIFY TTL toDateTime({column}) + INTERVAL {days} DAY SETTINGS materialize_ttl_after_modify=0")).await?;
    }
    Ok(())
}
pub(crate) fn operations(hours: u32) -> String {
    format!("SELECT component,name,countMerge(calls) count,sumMerge(errors) errors,sumMerge(duration) total_ms,maxMerge(maximum) max_ms,quantilesTDigestMerge(0.5,0.95)(quantiles)[1] p50_ms,quantilesTDigestMerge(0.5,0.95)(quantiles)[2] p95_ms,argMaxMerge(trace) trace_id FROM otto_telemetry.operation_minutes WHERE minute >= now()-INTERVAL {hours} HOUR GROUP BY component,name ORDER BY total_ms DESC LIMIT 1000 SETTINGS max_execution_time=5,max_memory_usage=134217728")
}
pub(crate) fn resources(hours: u32) -> String {
    // At most 1440 time buckets per process even across a long retention window.
    let interval = (hours * 60).div_ceil(1440).max(1);
    format!("SELECT toUnixTimestamp(toStartOfInterval(minute,INTERVAL {interval} MINUTE)) timestamp,process,metric,maxMerge(value) value FROM otto_telemetry.resource_minutes WHERE minute>=now()-INTERVAL {hours} HOUR GROUP BY timestamp,process,metric ORDER BY timestamp LIMIT 23040 SETTINGS max_execution_time=5,max_memory_usage=134217728")
}

pub(crate) fn slow_operations(c: &TelemetryConfig) -> String {
    // Fetch every slow candidate: ranking by self time happens afterwards, so
    // limiting by inclusive total here would drop the real root causes.
    operations(c.analysis_window_hours).replace(
        "ORDER BY total_ms DESC LIMIT 1000",
        &format!(
            "HAVING count >= {} AND p95_ms >= {} ORDER BY total_ms DESC LIMIT 500",
            c.min_samples, c.slow_threshold_ms
        ),
    )
}

/// Exclusive time per operation: each span's duration minus the summed
/// durations of its direct children in the same trace (clamped at zero for
/// concurrent children). Bounded by the analysis window and memory caps.
pub(crate) fn self_times(c: &TelemetryConfig) -> String {
    let hours = c.analysis_window_hours;
    format!("SELECT p.SpanAttributes['otto.component'] component,p.SpanName name,sum(greatest(toFloat64(p.Duration)-toFloat64(k.child),0))/1e6 self_ms FROM otto_telemetry.otel_traces p LEFT JOIN (SELECT TraceId,ParentSpanId,sum(Duration) child FROM otto_telemetry.otel_traces WHERE Timestamp>=now()-INTERVAL {hours} HOUR AND ParentSpanId!='' GROUP BY TraceId,ParentSpanId) k ON k.TraceId=p.TraceId AND k.ParentSpanId=p.SpanId WHERE p.Timestamp>=now()-INTERVAL {hours} HOUR GROUP BY component,name LIMIT 10000 SETTINGS max_execution_time=10,max_memory_usage=268435456,join_use_nulls=0")
}

/// Bound the raw-span scan with the exporter's TraceId → time-range lookup
/// table, so a drill-down reads one trace's time slice instead of scanning
/// every TraceId in the retention window.
pub(crate) fn trace(id: &str, days: u32) -> String {
    format!("SELECT TraceId,SpanId,ParentSpanId,SpanName,SpanKind,SpanAttributes,StatusCode,toUnixTimestamp64Nano(Timestamp) started,Duration FROM otto_telemetry.otel_traces WHERE TraceId='{id}' AND Timestamp>=(SELECT min(Start) FROM otto_telemetry.otel_traces_trace_id_ts WHERE TraceId='{id}') AND Timestamp<=(SELECT max(End)+1 FROM otto_telemetry.otel_traces_trace_id_ts WHERE TraceId='{id}') AND Timestamp>=now()-INTERVAL {days} DAY ORDER BY Timestamp LIMIT 1000 SETTINGS max_execution_time=5,max_memory_usage=67108864")
}
