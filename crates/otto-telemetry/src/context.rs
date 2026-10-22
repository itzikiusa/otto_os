//! Explicit async operation scopes. No global subscriber and no argument capture.
use crate::{SpanRecord, SpanSink};
use std::{future::Future, sync::Arc, time::Instant};

#[derive(Clone)]
struct Context {
    service: Arc<dyn SpanSink>,
    trace_id: String,
    span_id: String,
    component: String,
}
tokio::task_local! { static CURRENT: Context; }

pub async fn scope<T>(
    service: Arc<dyn SpanSink>,
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
    use super::*;
    use std::sync::Mutex;

    /// A sink that keeps what it is handed.
    struct Collect(Mutex<Vec<SpanRecord>>, bool);
    impl SpanSink for Collect {
        fn enabled(&self) -> bool {
            self.1
        }
        fn record(&self, span: SpanRecord) {
            self.0.lock().unwrap().push(span);
        }
    }

    #[tokio::test]
    async fn scoped_phases_reach_the_sink_as_children() {
        let sink = Arc::new(Collect(Mutex::new(Vec::new()), true));
        let parent = SpanRecord::new("request", "server", 0.0);
        let out = scope(sink.clone(), &parent, async {
            measure_result("phase", async { Err::<(), _>("boom") }).await
        })
        .await;
        assert_eq!(out, Err("boom"));
        let spans = sink.0.lock().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "phase");
        assert_eq!(spans[0].status, "error");
        assert_eq!(spans[0].trace_id, parent.trace_id);
        assert_eq!(
            spans[0].parent_span_id.as_deref(),
            Some(parent.span_id.as_str())
        );
    }

    #[tokio::test]
    async fn a_disabled_sink_records_nothing() {
        let sink = Arc::new(Collect(Mutex::new(Vec::new()), false));
        let parent = SpanRecord::new("request", "server", 0.0);
        scope(sink.clone(), &parent, measure("phase", async {})).await;
        assert!(sink.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn uninstrumented_work_keeps_value_and_error() {
        assert_eq!(super::measure("phase", async { 7 }).await, 7);
        assert_eq!(
            super::measure_result("phase", async { Err::<(), _>("same error") }).await,
            Err("same error")
        );
    }
}
