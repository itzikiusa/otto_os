use crate::{OperationSummary, Suggestion, TelemetryConfig};
use sha2::{Digest, Sha256};
pub(crate) fn id(kind: &str, component: &str, name: &str) -> String {
    hex::encode(Sha256::digest(
        format!("{kind}\0{component}\0{name}").as_bytes(),
    ))[..24]
        .to_string()
}
pub(crate) fn rank(
    operations: &[OperationSummary],
    c: &TelemetryConfig,
    now: i64,
) -> Vec<Suggestion> {
    let mut candidates: Vec<_> = operations
        .iter()
        .filter(|op| op.count >= u64::from(c.min_samples) && op.p95_ms >= c.slow_threshold_ms)
        .collect();
    candidates.sort_by(|a, b| {
        b.total_ms
            .total_cmp(&a.total_ms)
            .then_with(|| b.p95_ms.total_cmp(&a.p95_ms))
            .then_with(|| a.name.cmp(&b.name))
    });
    candidates.into_iter().take(c.suggestion_limit as usize).map(|op|Suggestion {
        id:id("latency",&op.component,&op.name),kind:"latency".into(),component:op.component.clone(),name:op.name.clone(),
        count:op.count,p50_ms:op.p50_ms,p95_ms:op.p95_ms,max_ms:op.max_ms,total_ms:op.total_ms,threshold_ms:c.slow_threshold_ms,
        window_hours:c.analysis_window_hours,observed_at:now,
        action:format!("Open the slow trace for {} and compare queue, request and child-operation durations. Measure CPU with a native profile before attributing this latency to computation.",op.name),
        trace_id:op.trace_id.clone(),dismissed:false,peak_cpu_percent:None,peak_rss_mb:None,
    }).collect()
}
