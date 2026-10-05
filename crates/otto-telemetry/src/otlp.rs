//! OTLP/HTTP JSON encoding. All content reaches here through typed allowlists.
use crate::{now_nanos, ResourcePoint, SpanRecord};
use serde_json::{json, Value};
fn resource() -> Value {
    json!({"attributes":[{"key":"service.name","value":{"stringValue":"otto"}}]})
}
fn attributes(s: &SpanRecord) -> Vec<Value> {
    let mut out = vec![json!({"key":"otto.component","value":{"stringValue":s.component}})];
    out.extend(s.attributes.iter().map(|(k,v)|json!({"key":k,"value":if let Some(s)=v.as_str(){json!({"stringValue":s})}else{json!({"doubleValue":v.as_f64().unwrap_or(0.0)})}})));
    out
}
pub(crate) fn traces(spans: &[SpanRecord]) -> Value {
    json!({"resourceSpans":[{"resource":resource(),"scopeSpans":[{"scope":{"name":"otto.telemetry","version":"1"},"spans":spans.iter().map(|s|json!({
        "traceId":s.trace_id,"spanId":s.span_id,"parentSpanId":s.parent_span_id.as_deref().unwrap_or(""),"name":s.name,
        "kind":match s.kind.as_str(){"server"=>2,"client"=>3,"producer"=>4,"consumer"=>5,_=>1},
        "startTimeUnixNano":s.start_unix_nano.to_string(),"endTimeUnixNano":s.start_unix_nano.saturating_add((s.duration_ms*1e6)as u64).to_string(),
        "attributes":attributes(s),"status":{"code":if s.status=="error"{2}else if s.status=="ok"{1}else{0}}
    })).collect::<Vec<_>>()}]}]})
}
pub(crate) fn errors(spans: &[SpanRecord]) -> Value {
    json!({"resourceLogs":[{"resource":resource(),"scopeLogs":[{"scope":{"name":"otto.telemetry"},"logRecords":spans.iter().filter(|s|s.status=="error").map(|s|json!({"timeUnixNano":s.start_unix_nano.to_string(),"severityNumber":17,"severityText":"ERROR","body":{"stringValue":"operation.error"},"traceId":s.trace_id,"spanId":s.span_id,"attributes":attributes(s)})).collect::<Vec<_>>()}]}]})
}
pub(crate) fn metrics(points: &[ResourcePoint]) -> Value {
    let mut metrics = Vec::new();
    for point in points {
        for (name, unit, value) in [
            ("otto.process.cpu", "%", point.cpu_percent),
            ("otto.process.rss", "MiBy", point.rss_mb),
            ("otto.host.load", "1", point.host_load),
        ] {
            if let Some(value) = value {
                metrics.push(json!({"name":name,"unit":unit,"gauge":{"dataPoints":[{"timeUnixNano":((point.timestamp as u64)*1_000_000_000).to_string(),"asDouble":value,"attributes":[{"key":"otto.process","value":{"stringValue":point.process}}]}]}}));
            }
        }
    }
    json!({"resourceMetrics":[{"resource":resource(),"scopeMetrics":[{"scope":{"name":"otto.resources"},"metrics":metrics}]}]})
}
pub(crate) fn spike(point: &ResourcePoint) -> Value {
    json!({"resourceLogs":[{"resource":resource(),"scopeLogs":[{"scope":{"name":"otto.resources"},"logRecords":[{"timeUnixNano":now_nanos().to_string(),"severityNumber":13,"severityText":"WARN","body":{"stringValue":"resource.spike"},"attributes":[{"key":"otto.process","value":{"stringValue":point.process}},{"key":"otto.cpu.percent","value":{"doubleValue":point.cpu_percent.unwrap_or_default()}},{"key":"otto.rss.mb","value":{"doubleValue":point.rss_mb.unwrap_or_default()}}]}]}]}]})
}

pub(crate) fn profile(profile: &crate::NativeProfile) -> Value {
    json!({"resourceLogs":[{"resource":resource(),"scopeLogs":[{"scope":{"name":"otto.profiles"},"logRecords":[{"timeUnixNano":now_nanos().to_string(),"severityNumber":9,"severityText":"INFO","body":{"stringValue":"profile.captured"},"attributes":[{"key":"otto.profile.format","value":{"stringValue":profile.format}},{"key":"otto.profile.duration_seconds","value":{"intValue":profile.duration_seconds.to_string()}},{"key":"otto.profile.frame_count","value":{"intValue":profile.frames.len().to_string()}}]}]}]}]})
}
