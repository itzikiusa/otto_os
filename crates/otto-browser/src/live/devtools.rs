//! Opt-in DevTools inspection for the remote live browser: bounded Console +
//! Network event rings (with query-string redaction so tokens in URLs never
//! leak into inspector logs) and point-based element inspection.
//!
//! Pure data structures and CDP parsers live here so every edge case is
//! unit-tested without launching Chromium.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Maximum console entries retained per live session.
pub const MAX_CONSOLE_ENTRIES: usize = 200;
/// Maximum network entries retained per live session.
pub const MAX_NETWORK_ENTRIES: usize = 200;
/// Maximum text length of a single console message (chars).
const MAX_CONSOLE_TEXT_CHARS: usize = 2_000;
/// Maximum redacted URL length stored per network entry (chars).
const MAX_NETWORK_URL_CHARS: usize = 512;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsoleEntry {
    pub seq: u64,
    /// `"log"` | `"info"` | `"warning"` | `"error"` | `"debug"` | `"exception"`
    pub level: String,
    pub text: String,
    pub timestamp: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkEntry {
    pub request_id: String,
    pub method: String,
    /// Origin + path only (query string and fragment redacted).
    pub url: String,
    pub resource_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    pub failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    pub timestamp: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementStyles {
    pub display: String,
    pub position: String,
    pub color: String,
    pub background: String,
    pub font: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectedElement {
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default)]
    pub classes: Vec<String>,
    pub selector: String,
    pub text: String,
    pub outer_html: String,
    pub rect: ElementRect,
    pub styles: ElementStyles,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DevtoolsSnapshot {
    pub enabled: bool,
    pub console: Vec<ConsoleEntry>,
    pub network: Vec<NetworkEntry>,
}

/// Strip query string and fragment from a URL so tokens / session keys never
/// appear in inspector logs. Non-http(s) URLs keep only their scheme + short
/// preview.
pub fn redact_network_url(raw: &str) -> String {
    if let Ok(mut u) = reqwest::Url::parse(raw) {
        if matches!(u.scheme(), "http" | "https" | "ws" | "wss") {
            let had_query = u.query().is_some();
            u.set_query(None);
            u.set_fragment(None);
            let _ = u.set_username("");
            let _ = u.set_password(None);
            let mut s = u.to_string();
            if had_query {
                s.push_str("?…");
            }
            return s.chars().take(MAX_NETWORK_URL_CHARS).collect();
        }
        return format!("{}:…", u.scheme());
    }
    raw.split(&['?', '#'][..])
        .next()
        .unwrap_or("")
        .chars()
        .take(MAX_NETWORK_URL_CHARS)
        .collect()
}

fn format_remote_object(arg: &Value) -> String {
    if let Some(s) = arg.get("value").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(v) = arg.get("value") {
        if !v.is_null() {
            return v.to_string();
        }
    }
    if let Some(d) = arg.get("description").and_then(Value::as_str) {
        return d.to_string();
    }
    if let Some(u) = arg.get("unserializableValue").and_then(Value::as_str) {
        return u.to_string();
    }
    arg.get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn cap_chars(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

/// Parse a CDP `Runtime.consoleAPICalled` event.
pub fn parse_console_event(params: &Value, seq: u64) -> Option<ConsoleEntry> {
    let kind = params.get("type").and_then(Value::as_str).unwrap_or("log");
    let level = match kind {
        "warning" | "warn" => "warning",
        "error" | "assert" => "error",
        "info" => "info",
        "debug" | "trace" => "debug",
        _ => "log",
    };
    let args = params.get("args").and_then(Value::as_array)?;
    let joined = args
        .iter()
        .map(format_remote_object)
        .collect::<Vec<_>>()
        .join(" ");
    let source = params.pointer("/stackTrace/callFrames/0").and_then(|f| {
        let url = f.get("url").and_then(Value::as_str).unwrap_or("");
        if url.is_empty() {
            None
        } else {
            let line = f.get("lineNumber").and_then(Value::as_u64).unwrap_or(0) + 1;
            Some(format!("{}:{line}", redact_network_url(url)))
        }
    });
    Some(ConsoleEntry {
        seq,
        level: level.to_string(),
        text: cap_chars(&joined, MAX_CONSOLE_TEXT_CHARS),
        timestamp: params
            .get("timestamp")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        source,
    })
}

/// Parse a CDP `Runtime.exceptionThrown` event.
pub fn parse_exception_event(params: &Value, seq: u64) -> Option<ConsoleEntry> {
    let details = params.get("exceptionDetails")?;
    let desc = details
        .pointer("/exception/description")
        .and_then(Value::as_str)
        .or_else(|| details.get("text").and_then(Value::as_str))
        .unwrap_or("Uncaught exception");
    let source = details
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .map(|u| {
            let line = details
                .get("lineNumber")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            format!("{}:{line}", redact_network_url(u))
        });
    Some(ConsoleEntry {
        seq,
        level: "error".to_string(),
        text: cap_chars(desc, MAX_CONSOLE_TEXT_CHARS),
        timestamp: params
            .get("timestamp")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        source,
    })
}

/// Bounded in-memory DevTools event buffer per live session.
#[derive(Debug, Default)]
pub struct DevtoolsBuffer {
    pub enabled: bool,
    next_console_seq: u64,
    console: VecDeque<ConsoleEntry>,
    network: VecDeque<NetworkEntry>,
    /// `requestId` → start timestamp (seconds, from CDP `Network.requestWillBeSent`).
    in_flight: HashMap<String, f64>,
}

impl DevtoolsBuffer {
    pub fn snapshot(&self) -> DevtoolsSnapshot {
        DevtoolsSnapshot {
            enabled: self.enabled,
            console: self.console.iter().cloned().collect(),
            network: self.network.iter().cloned().collect(),
        }
    }

    pub fn clear(&mut self) {
        self.console.clear();
        self.network.clear();
        self.in_flight.clear();
    }

    pub fn push_console_event(&mut self, params: &Value) -> Option<ConsoleEntry> {
        if !self.enabled {
            return None;
        }
        let seq = self.next_console_seq + 1;
        let entry = parse_console_event(params, seq)?;
        self.next_console_seq = seq;
        if self.console.len() >= MAX_CONSOLE_ENTRIES {
            self.console.pop_front();
        }
        self.console.push_back(entry.clone());
        Some(entry)
    }

    pub fn push_exception_event(&mut self, params: &Value) -> Option<ConsoleEntry> {
        if !self.enabled {
            return None;
        }
        let seq = self.next_console_seq + 1;
        let entry = parse_exception_event(params, seq)?;
        self.next_console_seq = seq;
        if self.console.len() >= MAX_CONSOLE_ENTRIES {
            self.console.pop_front();
        }
        self.console.push_back(entry.clone());
        Some(entry)
    }

    pub fn on_request_will_be_sent(&mut self, params: &Value) -> Option<NetworkEntry> {
        if !self.enabled {
            return None;
        }
        let req_id = params.get("requestId")?.as_str()?.to_string();
        let method = params
            .pointer("/request/method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_ascii_uppercase();
        let url = redact_network_url(
            params
                .pointer("/request/url")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let resource_type = params
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("Other")
            .to_string();
        let ts = params
            .get("timestamp")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if self.in_flight.len() >= MAX_NETWORK_ENTRIES * 2 {
            self.in_flight.clear();
        }
        self.in_flight.insert(req_id.clone(), ts);
        let entry = NetworkEntry {
            request_id: req_id,
            method,
            url,
            resource_type,
            status: None,
            status_text: None,
            failed: false,
            error_text: None,
            duration_ms: None,
            timestamp: ts,
        };
        self.upsert_network(entry.clone());
        Some(entry)
    }

    pub fn on_response_received(&mut self, params: &Value) -> Option<NetworkEntry> {
        if !self.enabled {
            return None;
        }
        let req_id = params.get("requestId")?.as_str()?;
        let status = params
            .pointer("/response/status")
            .and_then(Value::as_u64)
            .map(|s| s.min(u16::MAX as u64) as u16);
        let status_text = params
            .pointer("/response/statusText")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| cap_chars(s, 80));
        let ts = params
            .get("timestamp")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let duration_ms = self
            .in_flight
            .remove(req_id)
            .map(|start| ((ts - start).max(0.0) * 1000.0).round());
        if let Some(existing) = self
            .network
            .iter_mut()
            .rev()
            .find(|e| e.request_id == req_id)
        {
            existing.status = status;
            existing.status_text = status_text;
            existing.duration_ms = duration_ms;
            return Some(existing.clone());
        }
        let entry = NetworkEntry {
            request_id: req_id.to_string(),
            method: "GET".into(),
            url: redact_network_url(
                params
                    .pointer("/response/url")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            ),
            resource_type: params
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("Other")
                .to_string(),
            status,
            status_text,
            failed: false,
            error_text: None,
            duration_ms,
            timestamp: ts,
        };
        self.upsert_network(entry.clone());
        Some(entry)
    }

    pub fn on_loading_failed(&mut self, params: &Value) -> Option<NetworkEntry> {
        if !self.enabled {
            return None;
        }
        let req_id = params.get("requestId")?.as_str()?;
        let err = params
            .get("errorText")
            .and_then(Value::as_str)
            .unwrap_or("Failed");
        let ts = params
            .get("timestamp")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let duration_ms = self
            .in_flight
            .remove(req_id)
            .map(|start| ((ts - start).max(0.0) * 1000.0).round());
        if let Some(existing) = self
            .network
            .iter_mut()
            .rev()
            .find(|e| e.request_id == req_id)
        {
            existing.failed = true;
            existing.error_text = Some(cap_chars(err, 200));
            existing.duration_ms = duration_ms;
            return Some(existing.clone());
        }
        None
    }

    fn upsert_network(&mut self, entry: NetworkEntry) {
        if let Some(existing) = self
            .network
            .iter_mut()
            .rev()
            .find(|e| e.request_id == entry.request_id)
        {
            *existing = entry;
            return;
        }
        if self.network.len() >= MAX_NETWORK_ENTRIES {
            self.network.pop_front();
        }
        self.network.push_back(entry);
    }
}

/// Safe, bounded JS evaluated via `Runtime.evaluate` to inspect the element at
/// viewport CSS coordinates `(x, y)`. Builds a stable CSS selector using the
/// same `#id` > `data-*` > `tag:nth-of-type` priority as `selector.ts`.
pub fn inspect_element_expr(x: f64, y: f64) -> String {
    let x = if x.is_finite() { x.max(0.0) } else { 0.0 };
    let y = if y.is_finite() { y.max(0.0) } else { 0.0 };
    format!(
        r#"(function(){{
  var el = document.elementFromPoint({x}, {y});
  if (!el || el.nodeType !== 1) return null;
  function escAttr(v){{ return String(v).replace(/(["\\])/g, '\\$1'); }}
  function buildSel(node){{
    var id = node.getAttribute && node.getAttribute('id');
    if (id && id.length <= 280) {{
      return (typeof CSS !== 'undefined' && CSS.escape) ? ('#' + CSS.escape(id)) : ('[id="' + escAttr(id) + '"]');
    }}
    var attrs = ['data-testid','data-test','data-id','data-qa'];
    for (var i = 0; i < attrs.length; i++) {{
      var val = node.getAttribute && node.getAttribute(attrs[i]);
      if (val && val.length <= 260) return '[' + attrs[i] + '="' + escAttr(val) + '"]';
    }}
    var steps = [], cur = node;
    while (cur && cur.nodeType === 1 && cur !== document.documentElement) {{
      var tag = cur.tagName.toLowerCase();
      var p = cur.parentElement;
      if (!p) {{ steps.unshift(tag); break; }}
      var sibs = 0, idx = 0;
      for (var c = p.firstElementChild; c; c = c.nextElementSibling) {{
        if (c.tagName === cur.tagName) {{ sibs++; if (c === cur) idx = sibs; }}
      }}
      steps.unshift(sibs > 1 ? (tag + ':nth-of-type(' + idx + ')') : tag);
      cur = p;
      if (steps.length >= 12) break;
    }}
    return steps.join(' > ').slice(0, 300);
  }}
  var r = el.getBoundingClientRect();
  var cs = getComputedStyle(el);
  var cls = typeof el.className === 'string' ? el.className.trim().split(/\s+/).filter(Boolean).slice(0, 12) : [];
  return {{
    tag: el.tagName.toLowerCase(),
    id: el.id ? String(el.id).slice(0, 120) : null,
    classes: cls,
    selector: buildSel(el),
    text: String(el.innerText || el.textContent || '').trim().slice(0, 500),
    outer_html: String(el.outerHTML || '').slice(0, 2000),
    rect: {{ x: Math.round(r.left * 10) / 10, y: Math.round(r.top * 10) / 10, width: Math.round(r.width * 10) / 10, height: Math.round(r.height * 10) / 10 }},
    styles: {{
      display: cs.display || '',
      position: cs.position || '',
      color: cs.color || '',
      background: cs.backgroundColor || '',
      font: (cs.fontSize || '') + ' ' + String(cs.fontFamily || '').split(',')[0]
    }}
  }};
}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_query_fragment_and_credentials_from_network_urls() {
        assert_eq!(
            redact_network_url("https://user:pass@api.example.com/v1/items?token=secret#top"),
            "https://api.example.com/v1/items?…"
        );
        assert_eq!(
            redact_network_url("https://example.com/app.js"),
            "https://example.com/app.js"
        );
        assert_eq!(redact_network_url("data:image/png;base64,AAAA"), "data:…");
    }

    #[test]
    fn parses_console_and_exception_events() {
        let mut buf = DevtoolsBuffer {
            enabled: true,
            ..Default::default()
        };
        let c = buf
            .push_console_event(&json!({
                "type": "warning",
                "timestamp": 12.5,
                "args": [
                    {"type": "string", "value": "deprecated"},
                    {"type": "number", "value": 42}
                ],
                "stackTrace": {"callFrames": [{"url": "https://ex.com/a.js?k=1", "lineNumber": 9}]}
            }))
            .unwrap();
        assert_eq!(c.seq, 1);
        assert_eq!(c.level, "warning");
        assert_eq!(c.text, "deprecated 42");
        assert_eq!(c.source.as_deref(), Some("https://ex.com/a.js?…:10"));

        let ex = buf
            .push_exception_event(&json!({
                "timestamp": 13.0,
                "exceptionDetails": {
                    "text": "Uncaught",
                    "url": "https://ex.com/b.js",
                    "lineNumber": 3,
                    "exception": {"description": "TypeError: x is not a function"}
                }
            }))
            .unwrap();
        assert_eq!(ex.seq, 2);
        assert_eq!(ex.level, "error");
        assert!(ex.text.contains("TypeError"));
        assert_eq!(ex.source.as_deref(), Some("https://ex.com/b.js:4"));
    }

    #[test]
    fn tracks_network_lifecycle_and_duration() {
        let mut buf = DevtoolsBuffer {
            enabled: true,
            ..Default::default()
        };
        let r1 = buf
            .on_request_will_be_sent(&json!({
                "requestId": "req-1",
                "type": "Fetch",
                "timestamp": 100.0,
                "request": {"method": "post", "url": "https://ex.com/api?key=123"}
            }))
            .unwrap();
        assert_eq!(r1.method, "POST");
        assert_eq!(r1.url, "https://ex.com/api?…");
        assert_eq!(r1.status, None);

        let r2 = buf
            .on_response_received(&json!({
                "requestId": "req-1",
                "timestamp": 100.25,
                "response": {"status": 201, "statusText": "Created"}
            }))
            .unwrap();
        assert_eq!(r2.status, Some(201));
        assert_eq!(r2.duration_ms, Some(250.0));
        assert_eq!(buf.snapshot().network.len(), 1);

        let _ = buf.on_request_will_be_sent(&json!({
            "requestId": "req-2",
            "type": "XHR",
            "timestamp": 101.0,
            "request": {"method": "GET", "url": "https://ex.com/fail"}
        }));
        let failed = buf
            .on_loading_failed(&json!({
                "requestId": "req-2",
                "timestamp": 101.1,
                "errorText": "net::ERR_FAILED"
            }))
            .unwrap();
        assert!(failed.failed);
        assert_eq!(failed.error_text.as_deref(), Some("net::ERR_FAILED"));
        assert_eq!(failed.duration_ms, Some(100.0));
    }

    #[test]
    fn ring_buffers_stay_bounded_and_disabled_buffer_drops_events() {
        let mut buf = DevtoolsBuffer::default();
        assert!(buf
            .push_console_event(&json!({"type": "log", "args": [{"value": "x"}]}))
            .is_none());
        buf.enabled = true;
        for i in 0..(MAX_CONSOLE_ENTRIES + 25) {
            buf.push_console_event(&json!({"type": "log", "args": [{"value": format!("m{i}")}]}));
        }
        let snap = buf.snapshot();
        assert_eq!(snap.console.len(), MAX_CONSOLE_ENTRIES);
        assert_eq!(snap.console.first().unwrap().seq, 26);
    }

    #[test]
    fn inspect_expr_sanitizes_non_finite_coords() {
        let js = inspect_element_expr(f64::NAN, -10.0);
        assert!(js.contains("document.elementFromPoint(0, 0)"));
    }
}
