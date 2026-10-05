//! Explicit async operation scopes. No global subscriber and no argument capture.
use crate::{SpanRecord, TelemetryService};
use std::{future::Future, sync::Arc, time::Instant};

#[derive(Clone)]
struct Context {
    service: Arc<TelemetryService>,
    trace_id: String,
    span_id: String,
    component: String,
}
tokio::task_local! { static CURRENT: Context; }

pub async fn scope<T>(
    service: Arc<TelemetryService>,
    parent: &SpanRecord,
    work: impl Future<Output = T>,
) -> T {
    let context = Context {
        service,
        trace_id: parent.trace_id.clone(),
        span_id: parent.span_id.clone(),
        component: parent.component.clone(),
    };
    CURRENT.scope(context, work).await
}

fn child(name: &'static str) -> Option<(Context, SpanRecord)> {
    let ctx = CURRENT
        .try_with(Clone::clone)
        .ok()
        .filter(|c| c.service.enabled())?;
    let mut span = SpanRecord::new(name, &ctx.component, 0.0);
    span.trace_id = ctx.trace_id.clone();
    span.parent_span_id = Some(ctx.span_id.clone());
    Some((ctx, span))
}

/// Measure an opaque phase without assuming its return value means success.
pub async fn measure<T>(name: &'static str, work: impl Future<Output = T>) -> T {
    let Some((ctx, mut span)) = child(name) else {
        return work.await;
    };
    let nested = Context {
        span_id: span.span_id.clone(),
        ..ctx.clone()
    };
    let started = Instant::now();
    let value = CURRENT.scope(nested, work).await;
    span.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    span.status = "unset".into();
    ctx.service.record(span);
    value
}

/// Result-aware phases record failure status, never the error message or inputs.
pub async fn measure_result<T, E>(
    name: &'static str,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let Some((ctx, mut span)) = child(name) else {
        return work.await;
    };
    let nested = Context {
        span_id: span.span_id.clone(),
        ..ctx.clone()
    };
    let started = Instant::now();
    let result = CURRENT.scope(nested, work).await;
    span.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    span.status = if result.is_ok() { "ok" } else { "error" }.into();
    ctx.service.record(span);
    result
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn uninstrumented_work_keeps_value_and_error() {
        assert_eq!(super::measure("phase", async { 7 }).await, 7);
        assert_eq!(
            super::measure_result("phase", async { Err::<(), _>("same error") }).await,
            Err("same error")
        );
    }
}
