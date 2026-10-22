use crate::{OperationSummary, Suggestion, TelemetryConfig};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub(crate) fn id(kind: &str, component: &str, name: &str) -> String {
    hex::encode(Sha256::digest(
        format!("{kind}\0{component}\0{name}").as_bytes(),
    ))[..24]
        .to_string()
}
/// Rank slow operations by EXCLUSIVE (self) time so one root cause is not
/// suggested once per ancestor: an HTTP handler that only waits on a git
/// fetch has little self time. `self_ms` is keyed by (component, name); an
/// operation without a measured self time falls back to its inclusive total.
/// Candidates whose slowest trace was already claimed by a higher-ranked
/// candidate are collapsed into it.
pub(crate) fn rank(
    operations: &[OperationSummary],
    self_ms: &HashMap<(String, String), f64>,
    c: &TelemetryConfig,
    now: i64,
) -> Vec<Suggestion> {
    let own = |op: &OperationSummary| {
        self_ms
            .get(&(op.component.clone(), op.name.clone()))
            .copied()
    };
    let mut candidates: Vec<_> = operations
        .iter()
        .filter(|op| op.count >= u64::from(c.min_samples) && op.p95_ms >= c.slow_threshold_ms)
        .map(|op| (op, own(op)))
        .collect();
    candidates.sort_by(|(a, a_self), (b, b_self)| {
        b_self
            .unwrap_or(b.total_ms)
            .total_cmp(&a_self.unwrap_or(a.total_ms))
            .then_with(|| b.p95_ms.total_cmp(&a.p95_ms))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut claimed = HashSet::new();
    candidates
        .into_iter()
        .filter(|(op, _)| op.trace_id.is_empty() || claimed.insert(op.trace_id.clone()))
        .take(c.suggestion_limit as usize)
        .map(|(op, self_ms)| Suggestion {
        id:id("latency",&op.component,&op.name),kind:"latency".into(),component:op.component.clone(),name:op.name.clone(),
        count:op.count,p50_ms:op.p50_ms,p95_ms:op.p95_ms,max_ms:op.max_ms,total_ms:op.total_ms,self_ms,threshold_ms:c.slow_threshold_ms,
        window_hours:c.analysis_window_hours,observed_at:now,
        action:format!("Open the slow trace for {} and compare queue, request and child-operation durations. Measure CPU with a native profile before attributing this latency to computation.",op.name),
        trace_id:op.trace_id.clone(),dismissed:false,peak_cpu_percent:None,peak_rss_mb:None,
    }).collect()
}
