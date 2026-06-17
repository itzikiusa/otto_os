//! Streaming API-client transports: a single WebSocket the UI opens to the
//! daemon (`GET /ws/api-client/stream?token=…`) which then bridges to an
//! upstream **SSE** (`text/event-stream`) or **WebSocket** endpoint. Running
//! the upstream connection in the daemon (like the HTTP proxy) dodges webview
//! CORS/CSP and keeps secrets server-side.
//!
//! Wire protocol (JSON text frames):
//!   UI → daemon : {action:"open", kind:"sse"|"websocket", url, method?, headers?, body?}
//!                 {action:"send", data}            (websocket only)
//!                 {action:"close"}
//!   daemon → UI : {type:"open", detail}
//!                 {type:"event", event, data, id}  (sse)
//!                 {type:"message", dir:"in"|"out", data, binary}  (websocket)
//!                 {type:"error", message}
//!                 {type:"closed", detail}

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};

use otto_core::auth::TokenAuthenticator;

#[derive(Clone)]
struct StreamState {
    auth: Arc<dyn TokenAuthenticator>,
}

#[derive(Deserialize)]
struct StreamQuery {
    token: Option<String>,
}

#[derive(Deserialize, Default)]
struct KV {
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: String,
}

#[derive(Deserialize)]
struct OpenSpec {
    #[serde(default)]
    action: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    url: String,
    #[serde(default = "default_get")]
    method: String,
    #[serde(default)]
    headers: Vec<KV>,
    #[serde(default)]
    body: String,
}

fn default_get() -> String {
    "GET".to_string()
}

/// Root-level WS router (self-authenticates via `?token=`).
pub fn ws_router(authenticator: Arc<dyn TokenAuthenticator>) -> Router {
    Router::new()
        .route("/ws/api-client/stream", get(stream_ws))
        .with_state(StreamState { auth: authenticator })
}

async fn stream_ws(
    ws: WebSocketUpgrade,
    Query(q): Query<StreamQuery>,
    State(st): State<StreamState>,
) -> Response {
    let token = match q.token {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "missing token").into_response(),
    };
    if st.auth.authenticate(&token).await.is_err() {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }
    ws.on_upgrade(serve)
}

async fn send_json(socket: &mut WebSocket, v: Value) -> Result<(), axum::Error> {
    socket.send(Message::Text(v.to_string().into())).await
}

fn is_close_action(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("action").and_then(|a| a.as_str()).map(|s| s == "close"))
        .unwrap_or(false)
}

async fn serve(mut socket: WebSocket) {
    // First UI frame carries the open spec.
    let first = loop {
        match socket.recv().await {
            Some(Ok(Message::Text(t))) => break t.as_str().to_string(),
            Some(Ok(Message::Close(_))) | None => return,
            Some(Ok(_)) => continue,
            Some(Err(_)) => return,
        }
    };
    let spec: OpenSpec = match serde_json::from_str(&first) {
        Ok(s) => s,
        Err(e) => {
            let _ = send_json(&mut socket, json!({"type":"error","message":format!("bad open spec: {e}")})).await;
            return;
        }
    };
    if spec.action != "open" {
        let _ = send_json(&mut socket, json!({"type":"error","message":"first frame must be an 'open'"})).await;
        return;
    }
    match spec.kind.as_str() {
        "sse" => serve_sse(socket, spec).await,
        "websocket" | "ws" => serve_websocket(socket, spec).await,
        other => {
            let _ = send_json(&mut socket, json!({"type":"error","message":format!("unknown kind: {other}")})).await;
        }
    }
}

// ── SSE upstream ────────────────────────────────────────────────────────────

async fn serve_sse(mut socket: WebSocket, spec: OpenSpec) {
    let client = match reqwest::Client::builder().user_agent("Otto-ApiClient/1.0").build() {
        Ok(c) => c,
        Err(e) => {
            let _ = send_json(&mut socket, json!({"type":"error","message":e.to_string()})).await;
            return;
        }
    };
    let method = reqwest::Method::from_bytes(spec.method.to_uppercase().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut req = client.request(method, &spec.url).header("Accept", "text/event-stream");
    for h in &spec.headers {
        if !h.key.trim().is_empty() {
            req = req.header(&h.key, &h.value);
        }
    }
    if !spec.body.is_empty() {
        req = req.body(spec.body.clone());
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = send_json(&mut socket, json!({"type":"error","message":e.to_string()})).await;
            return;
        }
    };
    let status = resp.status();
    if send_json(&mut socket, json!({"type":"open","detail":format!("{status} — streaming events")})).await.is_err() {
        return;
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    loop {
        tokio::select! {
            chunk = stream.next() => match chunk {
                Some(Ok(bytes)) => {
                    buf.push_str(&String::from_utf8_lossy(&bytes));
                    buf = buf.replace("\r\n", "\n");
                    while let Some(idx) = buf.find("\n\n") {
                        let block: String = buf.drain(..idx + 2).collect();
                        if let Some(ev) = parse_sse_event(&block) {
                            if send_json(&mut socket, ev).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    let _ = send_json(&mut socket, json!({"type":"error","message":e.to_string()})).await;
                    break;
                }
                None => break,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Text(t))) => { if is_close_action(t.as_str()) { break; } }
                Some(Ok(Message::Close(_))) | None => return,
                _ => {}
            },
        }
    }
    let _ = send_json(&mut socket, json!({"type":"closed","detail":"stream ended"})).await;
    let _ = socket.send(Message::Close(None)).await;
}

fn parse_sse_event(block: &str) -> Option<Value> {
    let mut event = String::new();
    let mut data: Vec<String> = Vec::new();
    let mut id = String::new();
    for line in block.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => event = value.to_string(),
            "data" => data.push(value.to_string()),
            "id" => id = value.to_string(),
            _ => {}
        }
    }
    if data.is_empty() && event.is_empty() && id.is_empty() {
        return None;
    }
    Some(json!({
        "type": "event",
        "event": if event.is_empty() { "message".to_string() } else { event },
        "data": data.join("\n"),
        "id": id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_event_with_multiline_data() {
        let block = "event: update\nid: 42\ndata: line1\ndata: line2\n\n";
        let ev = parse_sse_event(block).expect("event");
        assert_eq!(ev["event"], "update");
        assert_eq!(ev["id"], "42");
        assert_eq!(ev["data"], "line1\nline2");
    }

    #[test]
    fn defaults_event_name_to_message_and_skips_comments() {
        let block = ": keep-alive comment\ndata: hello\n\n";
        let ev = parse_sse_event(block).expect("event");
        assert_eq!(ev["event"], "message");
        assert_eq!(ev["data"], "hello");
    }

    #[test]
    fn empty_block_yields_nothing() {
        assert!(parse_sse_event(":\n\n").is_none());
    }
}

// ── WebSocket upstream ──────────────────────────────────────────────────────

async fn serve_websocket(mut socket: WebSocket, spec: OpenSpec) {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::header::{HeaderName, HeaderValue};
    use tokio_tungstenite::tungstenite::Message as TMsg;

    let mut request = match spec.url.as_str().into_client_request() {
        Ok(r) => r,
        Err(e) => {
            let _ = send_json(&mut socket, json!({"type":"error","message":format!("bad ws url: {e}")})).await;
            return;
        }
    };
    for h in &spec.headers {
        if h.key.trim().is_empty() {
            continue;
        }
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(h.key.as_bytes()),
            HeaderValue::from_str(&h.value),
        ) {
            request.headers_mut().insert(name, val);
        }
    }

    let (upstream, _resp) = match tokio_tungstenite::connect_async(request).await {
        Ok(x) => x,
        Err(e) => {
            let _ = send_json(&mut socket, json!({"type":"error","message":format!("connect failed: {e}")})).await;
            return;
        }
    };
    if send_json(&mut socket, json!({"type":"open","detail":"connected"})).await.is_err() {
        return;
    }
    let (mut up_tx, mut up_rx) = upstream.split();

    loop {
        tokio::select! {
            up = up_rx.next() => match up {
                Some(Ok(TMsg::Text(t))) => {
                    if send_json(&mut socket, json!({"type":"message","dir":"in","data":t.as_str(),"binary":false})).await.is_err() { break; }
                }
                Some(Ok(TMsg::Binary(b))) => {
                    if send_json(&mut socket, json!({"type":"message","dir":"in","data":B64.encode(&b),"binary":true})).await.is_err() { break; }
                }
                Some(Ok(TMsg::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    let _ = send_json(&mut socket, json!({"type":"error","message":e.to_string()})).await;
                    break;
                }
            },
            cli = socket.recv() => match cli {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
                    match v.get("action").and_then(|a| a.as_str()) {
                        Some("send") => {
                            let data = v.get("data").and_then(|d| d.as_str()).unwrap_or("").to_string();
                            if up_tx.send(TMsg::Text(data.clone().into())).await.is_err() {
                                let _ = send_json(&mut socket, json!({"type":"error","message":"upstream send failed"})).await;
                                break;
                            }
                            let _ = send_json(&mut socket, json!({"type":"message","dir":"out","data":data,"binary":false})).await;
                        }
                        Some("close") => break,
                        _ => {}
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                _ => {}
            },
        }
    }
    let _ = up_tx.send(TMsg::Close(None)).await;
    let _ = send_json(&mut socket, json!({"type":"closed","detail":"disconnected"})).await;
    let _ = socket.send(Message::Close(None)).await;
}
