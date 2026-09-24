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

/// A `Fetch.requestPaused` event, reduced to what the guard needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Paused {
    pub request_id: String,
    pub url: String,
    pub method: String,
    /// `None` when the engine omitted it (treated as a document — fail closed).
    pub resource_type: Option<String>,
    pub frame_id: Option<String>,
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
        })
    }

    pub fn is_document(&self) -> bool {
        self.resource_type
            .as_deref()
            .is_none_or(|t| t == "Document")
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
    /// An agent-driven outward document request: screenshot + approval first.
    HoldOutward,
}

/// `main_frame` = the request belongs to a live session's top-level frame.
pub fn decide(allowed: bool, req: &Paused, main_frame: bool, agent_drives: bool) -> Decision {
    if !allowed {
        return Decision::Fail {
            document: req.is_document(),
        };
    }
    if agent_drives && main_frame && req.is_document() && is_outward_method(&req.method) {
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
}
