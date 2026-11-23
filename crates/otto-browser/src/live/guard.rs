//! Per-request decisions for the session-long `Fetch` interception pump: the
//! SSRF verdict (via `crate::cdp::request_allowed`, the same rule the reader
//! engine uses), a short-lived per-origin verdict cache, and the
//! "outward action" hold for agent-driven form submits. Pure where possible.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

/// How long an origin's verdict is reused (bounded so a DNS change is picked
/// up; the guard proxy re-vets every connection anyway).
pub const VERDICT_TTL: Duration = Duration::from_secs(60);
const VERDICT_CAP: usize = 1024;

/// Maximum characters included in a redacted `body_preview` on approval cards.
pub const MAX_BODY_PREVIEW_CHARS: usize = 512;

/// A `Fetch.requestPaused` event, reduced to what the guard needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Paused {
    pub request_id: String,
    pub url: String,
    pub method: String,
    /// `None` when the engine omitted it (treated as a document — fail closed).
    pub resource_type: Option<String>,
    pub frame_id: Option<String>,
    /// Optional raw request body (`request.postData`) for preview on held SPA mutations.
    pub post_data: Option<String>,
}

impl Paused {
    pub fn parse(params: &Value) -> Option<Self> {
        Some(Self {
            request_id: params.get("requestId")?.as_str()?.to_string(),
            url: params
                .pointer("/request/url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            method: params
                .pointer("/request/method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .to_ascii_uppercase(),
            resource_type: params
                .get("resourceType")
                .and_then(Value::as_str)
                .map(str::to_string),
            frame_id: params
                .get("frameId")
                .and_then(Value::as_str)
                .map(str::to_string),
            post_data: params
                .pointer("/request/postData")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    pub fn is_document(&self) -> bool {
        self.resource_type
            .as_deref()
            .is_none_or(|t| t == "Document")
    }

    pub fn is_spa_request(&self) -> bool {
        matches!(self.resource_type.as_deref(), Some("Fetch" | "XHR"))
    }
}

/// True when `url` targets a known analytics, telemetry, or RUM beacon host/path
/// that should not interrupt an agent with SPA mutation approval prompts.
pub fn is_beacon_or_analytics_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    const BEACON_HOST_SUFFIXES: &[&str] = &[
        "google-analytics.com",
        "analytics.google.com",
        "doubleclick.net",
        "googletagmanager.com",
        "segment.io",
        "segment.com",
        "sentry.io",
        "datadoghq.com",
        "datadoghq.eu",
        "amplitude.com",
        "mixpanel.com",
        "posthog.com",
        "plausible.io",
        "fullstory.com",
        "hotjar.com",
        "intercom.io",
        "clarity.ms",
        "newrelic.com",
        "nr-data.net",
        "bugsnag.com",
        "rollbar.com",
        "heapanalytics.com",
    ];
    if BEACON_HOST_SUFFIXES
        .iter()
        .any(|sfx| host == *sfx || host.ends_with(&format!(".{sfx}")))
    {
        return true;
    }
    let path = parsed.path().to_ascii_lowercase();
    const BEACON_PATH_MARKERS: &[&str] = &[
        "/beacon",
        "/telemetry",
        "/analytics",
        "/collect",
        "/rum",
        "/cdn-cgi/rum",
        "/_vercel/insights",
        "/j/collect",
        "/g/collect",
    ];
    BEACON_PATH_MARKERS
        .iter()
        .any(|m| path == *m || path.starts_with(&format!("{m}/")) || path.ends_with(m))
}

/// Redact non-JSON request bodies (form-encoded, truncated JSON, plain text)
/// without breaking `Bearer <tok>` or PEM blocks while still catching secrets
/// embedded inside `a=1&token=ghp_...` or truncated `"token":"ghp_...`.
fn redact_non_json_body(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut start = 0;
    for (idx, ch) in input.char_indices() {
        if matches!(ch, '&' | '"') {
            if idx > start {
                out.push_str(&otto_core::redact::redact_text(&input[start..idx]).value);
            }
            out.push(ch);
            start = idx + ch.len_utf8();
        }
    }
    if start < input.len() {
        out.push_str(&otto_core::redact::redact_text(&input[start..]).value);
    }
    out
}

/// Format a redacted, single-line-collapsed, length-capped preview of a
/// `Fetch`/`XHR` request body for human approval cards.
pub fn format_body_preview(post_data: Option<&str>) -> Option<String> {
    let raw = post_data?.trim();
    if raw.is_empty() {
        return None;
    }
    // Bound input before redaction so multi-megabyte uploads don't waste CPU.
    let bounded: String = raw.chars().take(MAX_BODY_PREVIEW_CHARS * 4).collect();
    let redacted = if let Ok(val) = serde_json::from_str::<serde_json::Value>(&bounded) {
        let cleaned = otto_core::redact::redact_json(&val).value;
        serde_json::to_string(&cleaned).unwrap_or_else(|_| redact_non_json_body(&bounded))
    } else {
        redact_non_json_body(&bounded)
    };
    let collapsed: String = redacted
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return None;
    }
    let char_count = trimmed.chars().count();
    if char_count > MAX_BODY_PREVIEW_CHARS {
        let mut out: String = trimmed.chars().take(MAX_BODY_PREVIEW_CHARS).collect();
        out.push('…');
        Some(out)
    } else {
        Some(trimmed.to_string())
    }
}

/// A state-changing method (a form submit is a POST document request).
pub fn is_outward_method(method: &str) -> bool {
    !matches!(method, "GET" | "HEAD" | "OPTIONS")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Continue,
    /// Refused by the SSRF guard; `document` = a navigation / redirect hop
    /// (surfaced to viewers as a `blocked` frame).
    Fail {
        document: bool,
    },
    /// An agent-driven outward request: screenshot + approval first.
    HoldOutward,
}

/// `main_frame` = the request belongs to a live session's top-level frame.
pub fn decide(allowed: bool, req: &Paused, main_frame: bool, agent_drives: bool) -> Decision {
    decide_with_spa_guard(allowed, req, main_frame, agent_drives, false)
}

/// Decision helper supporting opt-in SPA `Fetch`/`XHR` outward mutation gating.
pub fn decide_with_spa_guard(
    allowed: bool,
    req: &Paused,
    main_frame: bool,
    agent_drives: bool,
    guard_spa_mutations: bool,
) -> Decision {
    if !allowed {
        return Decision::Fail {
            document: req.is_document(),
        };
    }
    if agent_drives && main_frame && req.is_document() && is_outward_method(&req.method) {
        return Decision::HoldOutward;
    }
    if guard_spa_mutations
        && agent_drives
        && main_frame
        && req.is_spa_request()
        && is_outward_method(&req.method)
        && !is_beacon_or_analytics_url(&req.url)
    {
        return Decision::HoldOutward;
    }
    Decision::Continue
}

/// Cache key: `scheme://host:port`.
pub fn origin_key(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) => format!(
            "{}://{}:{}",
            u.scheme(),
            u.host_str().unwrap_or(""),
            u.port_or_known_default().unwrap_or(0)
        ),
        Err(_) => url.to_string(),
    }
}

/// `scheme://host[:port]` for display (approval cards, audit).
pub fn display_origin(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) => u.origin().ascii_serialization(),
        Err(_) => String::new(),
    }
}

/// Host only — never the path/query (may carry tokens).
pub fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

#[derive(Debug, Default)]
pub struct VerdictCache {
    map: HashMap<String, (bool, Instant)>,
}

impl VerdictCache {
    pub fn get(&self, key: &str, now: Instant) -> Option<bool> {
        self.map
            .get(key)
            .filter(|(_, at)| now.saturating_duration_since(*at) < VERDICT_TTL)
            .map(|(v, _)| *v)
    }

    pub fn insert(&mut self, key: String, verdict: bool, now: Instant) {
        if self.map.len() >= VERDICT_CAP {
            self.map.clear();
        }
        self.map.insert(key, (verdict, now));
    }
}

/// Vet one URL through the netguard rule, consulting/updating `cache`.
pub async fn vet(url: &str, cache: &std::sync::Mutex<VerdictCache>) -> bool {
    let key = origin_key(url);
    let now = Instant::now();
    if let Some(v) = cache.lock().ok().and_then(|c| c.get(&key, now)) {
        return v;
    }
    let v = crate::cdp::request_allowed(url).await;
    if let Ok(mut c) = cache.lock() {
        c.insert(key, v, now);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn paused(method: &str, rt: Option<&str>) -> Paused {
        let mut p = json!({
            "requestId": "r1",
            "frameId": "F",
            "request": {"url": "https://example.com/x?token=1", "method": method}
        });
        if let Some(rt) = rt {
            p["resourceType"] = json!(rt);
        }
        Paused::parse(&p).unwrap()
    }

    #[test]
    fn parses_paused_events() {
        let p = paused("post", Some("Document"));
        assert_eq!(p.request_id, "r1");
        assert_eq!(p.method, "POST");
        assert_eq!(p.frame_id.as_deref(), Some("F"));
        assert!(Paused::parse(&json!({"request": {}})).is_none());
    }

    #[test]
    fn blocked_requests_fail_and_documents_are_flagged() {
        assert_eq!(
            decide(false, &paused("GET", Some("Document")), true, false),
            Decision::Fail { document: true }
        );
        assert_eq!(
            decide(false, &paused("GET", Some("Image")), true, false),
            Decision::Fail { document: false }
        );
        // No resourceType → treated as a document (fail closed).
        assert_eq!(
            decide(false, &paused("GET", None), false, false),
            Decision::Fail { document: true }
        );
    }

    #[test]
    fn only_agent_driven_outward_main_frame_documents_are_held() {
        let post_doc = paused("POST", Some("Document"));
        assert_eq!(decide(true, &post_doc, true, true), Decision::HoldOutward);
        // A human submitting is their own action.
        assert_eq!(decide(true, &post_doc, true, false), Decision::Continue);
        // Sub-frame / XHR posts are not gated (analytics beacons etc.).
        assert_eq!(decide(true, &post_doc, false, true), Decision::Continue);
        assert_eq!(
            decide(true, &paused("POST", Some("XHR")), true, true),
            Decision::Continue
        );
        assert_eq!(
            decide(true, &paused("GET", Some("Document")), true, true),
            Decision::Continue
        );
    }

    #[test]
    fn origins_and_hosts_never_leak_paths() {
        assert_eq!(origin_key("https://a.example/x?y"), "https://a.example:443");
        assert_eq!(
            display_origin("https://a.example:8443/x?y"),
            "https://a.example:8443"
        );
        assert_eq!(host_of("https://a.example/secret?token=1"), "a.example");
        assert_eq!(host_of("nonsense"), "");
    }

    #[test]
    fn verdict_cache_expires() {
        let t0 = Instant::now();
        let mut c = VerdictCache::default();
        c.insert("k".into(), true, t0);
        assert_eq!(c.get("k", t0 + Duration::from_secs(5)), Some(true));
        assert_eq!(c.get("k", t0 + VERDICT_TTL + Duration::from_secs(1)), None);
    }

    #[tokio::test]
    async fn vetting_refuses_internal_targets_through_the_cache() {
        let cache = std::sync::Mutex::new(VerdictCache::default());
        assert!(!vet("http://127.0.0.1:7700/api/v1/sessions", &cache).await);
        assert!(!vet("http://169.254.169.254/latest/meta-data/", &cache).await);
        assert!(!vet("file:///etc/passwd", &cache).await);
        assert!(vet("https://8.8.8.8/", &cache).await);
        assert!(vet("data:text/plain,hi", &cache).await);
        // Cached verdict still refuses.
        assert!(!vet("http://127.0.0.1:7700/other", &cache).await);
    }

    #[test]
    fn spa_mutation_guard_holds_non_idempotent_fetch_xhr_and_skips_beacons() {
        let mut fetch_post = paused("POST", Some("Fetch"));
        fetch_post.url = "https://api.github.com/repos/o/r/pulls/1/merge".into();
        let mut xhr_delete = paused("DELETE", Some("XHR"));
        xhr_delete.url = "https://jira.example/rest/api/3/issue/ENG-1".into();
        let mut fetch_get = paused("GET", Some("Fetch"));
        fetch_get.url = "https://api.github.com/repos/o/r".into();
        let mut analytics = paused("POST", Some("Fetch"));
        analytics.url = "https://www.google-analytics.com/g/collect?v=2".into();
        let mut beacon_path = paused("POST", Some("XHR"));
        beacon_path.url = "https://app.example/telemetry".into();

        // Off by default -> Continue.
        assert_eq!(
            decide_with_spa_guard(true, &fetch_post, true, true, false),
            Decision::Continue
        );
        // Enabled + agent drives -> holds Fetch/XHR mutations, skips GET & beacons.
        assert_eq!(
            decide_with_spa_guard(true, &fetch_post, true, true, true),
            Decision::HoldOutward
        );
        assert_eq!(
            decide_with_spa_guard(true, &xhr_delete, true, true, true),
            Decision::HoldOutward
        );
        assert_eq!(
            decide_with_spa_guard(true, &fetch_get, true, true, true),
            Decision::Continue
        );
        assert_eq!(
            decide_with_spa_guard(true, &analytics, true, true, true),
            Decision::Continue
        );
        assert_eq!(
            decide_with_spa_guard(true, &beacon_path, true, true, true),
            Decision::Continue
        );
        // Human driving -> never held.
        assert_eq!(
            decide_with_spa_guard(true, &fetch_post, true, false, true),
            Decision::Continue
        );
    }

    #[test]
    fn body_preview_redacts_secrets_and_truncates() {
        let secret = format!("ghp_{}", "Z".repeat(36));
        let body = format!(r#"{{"merge_method":"squash","token":"{secret}"}}"#);
        let preview = format_body_preview(Some(&body)).unwrap();
        assert!(!preview.contains(&secret), "secret leaked: {preview}");
        assert!(preview.contains("squash"));

        let huge = "a".repeat(MAX_BODY_PREVIEW_CHARS + 200);
        let capped = format_body_preview(Some(&huge)).unwrap();
        assert!(capped.ends_with('…'));
        assert_eq!(capped.chars().count(), MAX_BODY_PREVIEW_CHARS + 1);
    }
}
