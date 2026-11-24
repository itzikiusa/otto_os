//! The legacy MCP **HTTP+SSE** transport (protocol 2024-11-05), for servers
//! that never moved to Streamable HTTP: a long-lived `GET` event stream whose
//! first `endpoint` event names where to `POST` messages; every response (and
//! any server→client request) then arrives on that stream. One reader task
//! per session routes responses to their waiting request by id and answers a
//! server's requests (`ping` → `{}`, anything else -32601, as Streamable
//! HTTP does). The endpoint must be on the stream's own origin — the POSTs
//! carry the configured headers (Keychain secrets), so a server cannot point
//! them anywhere else. The session ends when it is dropped (the reader is
//! aborted) or the stream closes.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;

/// The longest a session's event stream may stay open (pooled sessions are
/// reaped idle long before; this only bounds a forgotten one).
const STREAM_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
/// Cap on one event's data (a response body), as the other transports cap a body.
const MAX_EVENT_BYTES: usize = 1024 * 1024;

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>;

pub(crate) struct SseSession {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    headers: BTreeMap<String, String>,
    pending: Pending,
    closed: Arc<AtomicBool>,
    /// The reader task, aborted on drop (also when `open` fails or is cancelled).
    _reader: tokio::task::JoinSet<()>,
}

/// Why a request failed — the caller decides whether a fresh retry is safe.
pub(crate) enum SseError {
    /// The message never reached the server.
    NotDelivered(String),
    /// The server may have seen it.
    Failed(String),
}

impl SseSession {
    /// Open the event stream at `url` and wait for its `endpoint` event.
    pub(crate) async fn open(
        client: reqwest::Client,
        url: &str,
        headers: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let base = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
        let mut rb = client
            .get(base.clone())
            .header("Accept", "text/event-stream")
            .timeout(STREAM_LIFETIME);
        for (k, v) in headers {
            rb = rb.header(k.as_str(), v.as_str());
        }
        let resp = rb
            .send()
            .await
            .map_err(|e| format!("open event stream: {e}"))?;
        let status = resp.status();
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !status.is_success() || !ct.contains("text/event-stream") {
            return Err(format!("open event stream: http {status} ({ct})"));
        }
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (endpoint_tx, endpoint_rx) = oneshot::channel::<String>();
        let mut reader = tokio::task::JoinSet::new();
        reader.spawn(read_stream(
            resp,
            client.clone(),
            base.clone(),
            headers.clone(),
            pending.clone(),
            closed.clone(),
            endpoint_tx,
        ));
        let raw = endpoint_rx
            .await
            .map_err(|_| "the event stream closed before naming its endpoint".to_string())?;
        let endpoint = endpoint_url(&base, &raw)?;
        Ok(Self {
            client,
            endpoint,
            headers: headers.clone(),
            pending,
            closed,
            _reader: reader,
        })
    }

    /// Send request `msg` (carrying `id`) and wait for its response on the stream.
    pub(crate) async fn request(&self, msg: Value, id: i64) -> Result<Value, SseError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(SseError::NotDelivered("event stream closed".into()));
        }
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let sent = post(&self.client, &self.endpoint, &self.headers, &msg).await;
        if let Err(e) = sent {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(e);
        }
        rx.await
            .map_err(|_| SseError::Failed("event stream closed before the response".into()))
    }

    /// A notification (no response expected).
    pub(crate) async fn notify(&self, msg: Value) {
        let _ = post(&self.client, &self.endpoint, &self.headers, &msg).await;
    }
}

async fn post(
    client: &reqwest::Client,
    endpoint: &reqwest::Url,
    headers: &BTreeMap<String, String>,
    msg: &Value,
) -> Result<(), SseError> {
    let mut rb = client
        .post(endpoint.clone())
        .header("Content-Type", "application/json")
        .json(msg);
    for (k, v) in headers {
        rb = rb.header(k.as_str(), v.as_str());
    }
    let resp = rb.send().await.map_err(|e| {
        if e.is_connect() {
            SseError::NotDelivered(format!("post: {e}"))
        } else {
            SseError::Failed(format!("post: {e}"))
        }
    })?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else if status.is_client_error() {
        // Refused (an expired session…): the message was not run.
        Err(SseError::NotDelivered(format!("post: http {status}")))
    } else {
        // A 5xx may come after the server ran it: never re-sent.
        Err(SseError::Failed(format!("post: http {status}")))
    }
}

/// The `endpoint` event's URI, resolved against the stream's URL. It must stay
/// on that origin (scheme, host, port). Pure — unit-tested.
pub(crate) fn endpoint_url(base: &reqwest::Url, raw: &str) -> Result<reqwest::Url, String> {
    let url = base
        .join(raw.trim())
        .map_err(|e| format!("bad endpoint event: {e}"))?;
    let same = url.scheme() == base.scheme()
        && url.host_str() == base.host_str()
        && url.port_or_known_default() == base.port_or_known_default();
    if same {
        Ok(url)
    } else {
        Err("the server named an endpoint on another origin; refused".into())
    }
}

/// One SSE event: its `event:` name (default `message`) and joined `data:`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Event {
    pub name: String,
    pub data: String,
}

/// Incremental `text/event-stream` line parser keeping event names.
#[derive(Default)]
pub(crate) struct EventParser {
    pending: Vec<u8>,
    name: Option<String>,
    data: Option<String>,
}

impl EventParser {
    pub(crate) fn feed(&mut self, chunk: &[u8], out: &mut Vec<Event>) -> Result<(), String> {
        self.pending.extend_from_slice(chunk);
        while let Some(i) = self.pending.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.pending.drain(..=i).collect();
            let line = String::from_utf8_lossy(&raw[..raw.len() - 1]).into_owned();
            self.line(line.strip_suffix('\r').unwrap_or(&line), out);
        }
        let held = self.pending.len() + self.data.as_ref().map_or(0, String::len);
        if held > MAX_EVENT_BYTES {
            return Err("an event exceeded the size cap".into());
        }
        Ok(())
    }

    fn line(&mut self, line: &str, out: &mut Vec<Event>) {
        if line.is_empty() {
            if let Some(data) = self.data.take() {
                out.push(Event {
                    name: self.name.take().unwrap_or_else(|| "message".into()),
                    data,
                });
            }
            self.name = None;
            return;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.name = Some(value.to_string()),
            "data" => match &mut self.data {
                Some(d) => {
                    d.push('\n');
                    d.push_str(value);
                }
                None => self.data = Some(value.to_string()),
            },
            _ => {}
        }
    }
}

async fn read_stream(
    mut resp: reqwest::Response,
    client: reqwest::Client,
    base: reqwest::Url,
    headers: BTreeMap<String, String>,
    pending: Pending,
    closed: Arc<AtomicBool>,
    endpoint_tx: oneshot::Sender<String>,
) {
    let mut endpoint_tx = Some(endpoint_tx);
    let mut endpoint: Option<reqwest::Url> = None;
    let mut parser = EventParser::default();
    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            _ => break,
        };
        let mut events = Vec::new();
        if parser.feed(&chunk, &mut events).is_err() {
            break;
        }
        for ev in events {
            if ev.name == "endpoint" {
                endpoint = endpoint_url(&base, &ev.data).ok();
                if let Some(tx) = endpoint_tx.take() {
                    let _ = tx.send(ev.data);
                }
                continue;
            }
            let Ok(msg) = serde_json::from_str::<Value>(&ev.data) else {
                continue;
            };
            if let Some(reply) = crate::client::server_request_answer(&msg) {
                if let Some(url) = endpoint.clone() {
                    let (client, headers) = (client.clone(), headers.clone());
                    tokio::spawn(async move {
                        let _ = post(&client, &url, &headers, &reply).await;
                    });
                }
                continue;
            }
            if crate::client::is_response(&msg) {
                let id = msg.get("id").and_then(Value::as_i64);
                let waiter = id.and_then(|id| {
                    pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&id)
                });
                if let Some(tx) = waiter {
                    let _ = tx.send(msg);
                }
            }
        }
    }
    closed.store(true, Ordering::Release);
    // Waiters see the stream end (their sender drops).
    pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_keep_their_names_and_join_data_lines() {
        let mut p = EventParser::default();
        let mut out = Vec::new();
        p.feed(
            b"event: endpoint\ndata: /messages?session=1\n\n: ping\n\ndata: {\"a\":\n",
            &mut out,
        )
        .unwrap();
        p.feed(b"data: 1}\n\n", &mut out).unwrap();
        assert_eq!(
            out,
            vec![
                Event {
                    name: "endpoint".into(),
                    data: "/messages?session=1".into()
                },
                Event {
                    name: "message".into(),
                    data: "{\"a\":\n1}".into()
                },
            ]
        );
    }

    #[test]
    fn the_endpoint_stays_on_the_streams_origin() {
        let base = reqwest::Url::parse("https://mcp.example.com/sse").unwrap();
        assert_eq!(
            endpoint_url(&base, "/messages?s=1").unwrap().as_str(),
            "https://mcp.example.com/messages?s=1"
        );
        assert!(endpoint_url(&base, "https://mcp.example.com:443/m").is_ok());
        assert!(endpoint_url(&base, "https://evil.example.net/m").is_err());
        assert!(endpoint_url(&base, "http://mcp.example.com/m").is_err());
        assert!(endpoint_url(&base, "https://mcp.example.com:8443/m").is_err());
    }

    /// A one-connection-at-a-time server answering every request `head`.
    async fn answering(head: String) -> (tokio::task::JoinHandle<Option<usize>>, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/sse", l.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.ok()?;
            let mut buf = [0u8; 4096];
            s.read(&mut buf).await.ok().filter(|n| *n > 0)?;
            s.write_all(head.as_bytes()).await.ok()?;
            // What the client does next: 0 once it closes the connection.
            tokio::time::timeout(Duration::from_secs(5), s.read(&mut buf))
                .await
                .ok()?
                .ok()
        });
        (server, url)
    }

    /// Only a refused POST (4xx: an expired session) may be re-sent on a
    /// fresh session; after a 5xx the server may have run a mutating call.
    #[tokio::test]
    async fn a_server_error_on_post_is_not_retried_as_undelivered() {
        for (status, undelivered) in [(404, true), (400, true), (500, false), (502, false)] {
            let head =
                format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let (_server, url) = answering(head).await;
            let url = reqwest::Url::parse(&url).unwrap();
            let r = post(
                &reqwest::Client::new(),
                &url,
                &BTreeMap::new(),
                &Value::Null,
            )
            .await;
            assert_eq!(
                matches!(r, Err(SseError::NotDelivered(_))),
                undelivered,
                "{status}"
            );
        }
    }

    /// A refused open (an endpoint on another origin) lets its stream go
    /// instead of reading it for a day.
    #[tokio::test]
    async fn a_refused_open_closes_its_event_stream() {
        let (server, url) = answering("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\nevent: endpoint\ndata: http://evil.example/m\n\n".into()).await;
        let err = SseSession::open(reqwest::Client::new(), &url, &BTreeMap::new())
            .await
            .err()
            .unwrap();
        assert!(err.contains("another origin"), "{err}");
        assert_eq!(server.await.unwrap(), Some(0), "the stream stayed open");
    }
}
