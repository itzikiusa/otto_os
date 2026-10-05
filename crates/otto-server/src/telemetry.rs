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
    if route.contains("/telemetry/") || route.ends_with("/health") {
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
    span.attributes.insert("http.route".into(), route.into());
    let started = Instant::now();
    let mut response =
        otto_telemetry::context::scope(Arc::clone(service), &span, next.run(req)).await;
    span.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    span.status = if response.status().as_u16() < 400 {
        "ok"
    } else {
        "error"
    }
    .into();
    span.attributes.insert(
        "http.response.status_code".into(),
        response.status().as_u16().into(),
    );
    if let Ok(value) = format!("00-{}-{}-01", span.trace_id, span.span_id).parse() {
        response.headers_mut().insert("traceparent", value);
    }
    service.record(span);
    response
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
