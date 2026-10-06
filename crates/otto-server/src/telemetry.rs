//! Application timings: route templates only, with W3C parent propagation.
use axum::{
    extract::{MatchedPath, Request, State},
    middleware::Next,
    response::Response,
};
pub use otto_telemetry::context::measure;
use otto_telemetry::{SpanRecord, TelemetryService};
use std::{sync::Arc, time::Instant};

/// Accept only version 00, nonzero trace/span IDs and valid trace flags.
pub fn parent(value: &str) -> Option<(String, String)> {
    let parts: Vec<_> = value.split('-').collect();
    if parts.len() != 4
        || parts[0] != "00"
        || parts[3].len() != 2
        || !parts[3]
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return None;
    }
    let id = |s: &str, n: usize| {
        s.len() == n
            && s.bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            && s.bytes().any(|c| c != b'0')
    };
    (id(parts[1], 32) && id(parts[2], 16)).then(|| (parts[1].into(), parts[2].into()))
}

/// Component labels come only from the router's static template, never a URL ID.
fn component(route: &str) -> &str {
    let mut parts = route
        .trim_start_matches("/api/v1/")
        .trim_start_matches('/')
        .split('/');
    let first = parts.next().unwrap_or("server");
    let name = if first == "workspaces" {
        let _workspace_parameter = parts.next();
        parts.next().unwrap_or(first)
    } else {
        first
    };
    match name {
        "sessions" => "agents",
        "repos" => "git",
        "connections" | "db" => "database",
        "k8s" => "kubernetes",
        "api-client" => "api",
        "" => "server",
        value if value.starts_with('{') => "server",
        value => value,
    }
}

/// Routes whose wall time is not request latency (S9-06), so they are never
/// timed as operations: telemetry's own reads and health probes; LONG-POLLS
/// (`GET /sessions/{id}/wait` parks up to ~14 min for `wait_session` — it
/// would dominate `total_ms`/p95, top the slow-operation suggestions, and past
/// 1 h fail span validation as a silent drop); and WebSocket upgrades (the
/// span would only time the 101, not the socket's life).
fn untimed(route: &str, headers: &axum::http::HeaderMap) -> bool {
    const LONG_POLLS: &[&str] = &["/api/v1/sessions/{id}/wait"];
    route.contains("/telemetry/")
        || route.ends_with("/health")
        || LONG_POLLS.contains(&route)
        || route.contains("/ws/")
        || headers
            .get(axum::http::header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

pub async fn middleware(
    State(service): State<Option<Arc<TelemetryService>>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(service) = service.as_ref().filter(|s| s.enabled()) else {
        return next.run(req).await;
    };
    let Some(route) = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
    else {
        return next.run(req).await;
    };
    if untimed(&route, req.headers()) {
        return next.run(req).await;
    }
    let component = component(&route);
    let mut name = format!(
        "http.{}.{}",
        req.method().as_str().to_ascii_lowercase(),
        route
            .trim_start_matches("/api/v1/")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '.'
            })
            .collect::<String>()
    );
    if name.len() > 96 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut hash);
        name.truncate(78);
        name.push_str(&format!(".{:016x}", hash.finish()));
    }
    let mut span = SpanRecord::new(name, component, 0.0);
    span.kind = "server".into();
    if let Some((trace, parent)) = req
        .headers()
        .get("traceparent")
        .and_then(|v| v.to_str().ok())
        .and_then(parent)
    {
        span.trace_id = trace;
        span.parent_span_id = Some(parent);
    }
    span.attributes
        .insert("http.request.method".into(), req.method().as_str().into());
    span.attributes
        .insert("http.route".into(), route.clone().into());
    let traceparent = format!("00-{}-{}-01", span.trace_id, span.span_id);
    // A client that disconnects drops this future mid-request; the guard
    // still records the span (status `cancelled`, 499) so aborted slow
    // requests are not invisible in the latency data.
    let mut guard = RequestSpan {
        service: Arc::clone(service),
        span: Some(span),
        started: Instant::now(),
    };
    let parent = guard.span.clone().expect("span present until finish");
    let mut response =
        otto_telemetry::context::scope(Arc::clone(service), &parent, next.run(req)).await;
    guard.finish(response.status().as_u16());
    if let Ok(value) = traceparent.parse() {
        response.headers_mut().insert("traceparent", value);
    }
    // The static route template (never a concrete URL) lets the UI name its
    // client span per endpoint instead of one opaque `http.client` bucket.
    if let Ok(value) = route.parse() {
        response.headers_mut().insert(ROUTE_HEADER, value);
    }
    response
}

/// Response header carrying the matched route TEMPLATE (`/api/v1/repos/{id}/fetch`).
pub const ROUTE_HEADER: &str = "x-otto-route";

/// Records the request span exactly once: on completion with the response
/// status, or on drop (client abort / cancelled future) as `cancelled`.
struct RequestSpan {
    service: Arc<TelemetryService>,
    span: Option<SpanRecord>,
    started: Instant,
}
impl RequestSpan {
    fn finish(&mut self, status: u16) {
        let Some(mut span) = self.span.take() else {
            return;
        };
        span.duration_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        span.status = match status {
            499 => "cancelled",
            s if s < 400 => "ok",
            _ => "error",
        }
        .into();
        span.attributes
            .insert("http.response.status_code".into(), status.into());
        self.service.record(span);
    }
}
impl Drop for RequestSpan {
    fn drop(&mut self) {
        // 499: the de-facto "client closed request" code.
        self.finish(499);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_rejects_zero_ids_extra_data_and_versions() {
        let good = "00-1234567890abcdef1234567890abcdef-1234567890abcdef-01";
        assert!(parent(good).is_some());
        assert!(parent(&good.replace("00-", "01-")).is_none());
        assert!(parent(&format!("{good}-extra")).is_none());
        assert!(parent("00-00000000000000000000000000000000-1234567890abcdef-01").is_none());
    }
    #[tokio::test]
    async fn aborted_request_is_recorded_as_cancelled() {
        let temp = tempfile::tempdir().unwrap();
        let usage = otto_usage::UsageEngine::start(
            otto_usage::UsageConfig {
                enabled: false,
                ..Default::default()
            },
            temp.path().join("usage"),
        )
        .await;
        let service =
            TelemetryService::start(usage.clone(), temp.path().to_path_buf(), Default::default())
                .await;
        service.enable_ingestion_for_tests();
        {
            let _guard = RequestSpan {
                service: Arc::clone(&service),
                span: Some(SpanRecord::new("http.get.repos", "git", 0.0)),
                started: Instant::now(),
            };
            // Dropped without finish(): the client went away mid-request.
        }
        let mut done = RequestSpan {
            service: Arc::clone(&service),
            span: Some(SpanRecord::new("http.get.repos", "git", 0.0)),
            started: Instant::now(),
        };
        done.finish(200);
        drop(done);
        let statuses: Vec<_> = service
            .queued_spans_for_tests()
            .into_iter()
            .map(|s| (s.status, s.attributes["http.response.status_code"].clone()))
            .collect();
        assert_eq!(
            statuses,
            [
                ("cancelled".to_string(), serde_json::json!(499)),
                ("ok".to_string(), serde_json::json!(200))
            ],
            "abort recorded once, completed request recorded once"
        );
        service.shutdown().await;
        usage.shutdown().await;
    }
    #[test]
    fn long_polls_and_upgrades_are_not_timed() {
        let none = axum::http::HeaderMap::new();
        assert!(untimed("/api/v1/sessions/{id}/wait", &none));
        assert!(untimed("/api/v1/ws/events", &none));
        assert!(untimed("/api/v1/telemetry/overview", &none));
        assert!(untimed("/health", &none));
        let mut upgrade = axum::http::HeaderMap::new();
        upgrade.insert(axum::http::header::UPGRADE, "websocket".parse().unwrap());
        assert!(untimed("/api/v1/rooms/{id}", &upgrade));
        assert!(!untimed("/api/v1/sessions/{id}", &none));
        assert!(!untimed("/api/v1/repos/{id}/fetch", &none));
    }
    #[test]
    fn workspace_routes_keep_the_feature_component() {
        assert_eq!(
            component("/api/v1/workspaces/{wid}/vault/notes/{id}"),
            "vault"
        );
        assert_eq!(component("/api/v1/workspaces/{wid}/sessions"), "agents");
        assert_eq!(component("/api/v1/repos/{rid}/status"), "git");
        assert_eq!(component("/api/v1/workspaces"), "workspaces");
        assert_eq!(component("/api/v1/{unknown}"), "server");
    }
}
