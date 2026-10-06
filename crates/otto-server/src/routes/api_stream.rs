//! Streaming API-client transports: a single WebSocket the UI opens to the
//! daemon (`GET /ws/api-client/stream`, bearer in the `otto-bearer`
//! subprotocol only — no `?token=`) which then bridges to an
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

use crate::state::ServerCtx;
use otto_core::{
    api::ExecuteApiReq,
    domain::{Capability, Feature, WorkspaceRole},
    Id,
};
use otto_state::GrantsRepo;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct StreamQuery {
    workspace_id: Id,
}

#[derive(Deserialize)]
struct OpenSpec {
    action: String,
    kind: String,
    request: ExecuteApiReq,
}

/// Root-level WS router: authenticate AND authorize before upgrading.
pub fn ws_router(ctx: ServerCtx) -> Router {
    Router::new()
        .route("/ws/api-client/stream", get(stream_ws))
        .with_state(ctx)
}

async fn stream_ws(
    ws: WebSocketUpgrade,
    Query(q): Query<StreamQuery>,
    State(ctx): State<ServerCtx>,
    headers: axum::http::HeaderMap,
) -> Response {
    // The bearer travels only in the `otto-bearer` subprotocol (token out of
    // the URL — S11-11); the legacy `?token=` fallback is gone (S11-312).
    let ws = ws.protocols([crate::ws_events::BEARER_SUBPROTOCOL]);
    let token = match crate::ws_events::token_from_subprotocol(&headers) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "missing token").into_response(),
    };
    let auth = match ctx.authenticator.authenticate(&token).await {
        Ok(auth) => auth,
        Err(_) => return (StatusCode::UNAUTHORIZED, "invalid token").into_response(),
    };
    // Share / MCP-restricted tokens and read-only agent sessions never stream.
    if let Err(e) = crate::feature_guard::root_route_gate(
        crate::feature_guard::RootRoute::ApiStream,
        &auth,
        Some(&ctx.pool),
    )
    .await
    {
        return (StatusCode::FORBIDDEN, e.to_string()).into_response();
    }
    let user = &auth.effective_user;
    if auth.scope.is_some()
        || auth.mcp_only
        || ctx
            .roles
            .check(user, &q.workspace_id, WorkspaceRole::Editor)
            .await
            .is_err()
        || !matches!(GrantsRepo::new(ctx.pool.clone()).capability_of(user, Feature::ApiClient).await,
            Ok(cap) if cap >= Capability::Edit)
    {
        return (
            StatusCode::FORBIDDEN,
            "API Client edit access is required in this workspace",
        )
            .into_response();
    }
    let actor = user.id.clone();
    // A managed agent credential may open streams but can never confirm a
    // new-host secret send (see `prepare_stream`).
    let confirm_allowed = auth.managed_session_id.is_none();
    ws.max_message_size(1024 * 1024)
        .max_frame_size(1024 * 1024)
        .on_upgrade(move |socket| serve(socket, ctx, q.workspace_id, actor, confirm_allowed))
}

async fn send_json(socket: &mut WebSocket, v: Value) -> Result<(), axum::Error> {
    socket.send(Message::Text(v.to_string().into())).await
}

fn is_close_action(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| {
            v.get("action")
                .and_then(|a| a.as_str())
                .map(|s| s == "close")
        })
        .unwrap_or(false)
}

async fn serve(mut socket: WebSocket, ctx: ServerCtx, wid: Id, actor: Id, confirm_allowed: bool) {
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
            let _ = send_json(
                &mut socket,
                json!({"type":"error","message":format!("bad open spec: {e}")}),
            )
            .await;
            return;
        }
    };
    if spec.action != "open" {
        let _ = send_json(
            &mut socket,
            json!({"type":"error","message":"first frame must be an 'open'"}),
        )
        .await;
        return;
    }
    match spec.kind.as_str() {
        "sse" => serve_sse(socket, spec, &ctx, &wid, &actor, confirm_allowed).await,
        "websocket" | "ws" => {
            serve_websocket(socket, spec, &ctx, &wid, &actor, confirm_allowed).await
        }
        other => {
            let _ = send_json(
                &mut socket,
                json!({"type":"error","message":format!("unknown kind: {other}")}),
            )
            .await;
        }
    }
}

// ── SSE upstream ────────────────────────────────────────────────────────────

async fn serve_sse(
    mut socket: WebSocket,
    spec: OpenSpec,
    ctx: &ServerCtx,
    wid: &Id,
    actor: &Id,
    confirm_allowed: bool,
) {
    // The request's timeout bounds the wait for the response HEAD only — the
    // event stream itself may run for as long as the user keeps it open.
    let head_timeout = super::api_client::effective_timeout(spec.request.timeout_ms);
    let connecting = async {
        let req =
            super::api_client::prepare_stream(ctx, wid, &spec.request, actor, confirm_allowed)
                .await?;
        match tokio::time::timeout(
            head_timeout,
            req.header("Accept", "text/event-stream").send(),
        )
        .await
        {
            Ok(sent) => sent.map_err(|e| e.without_url().to_string()),
            Err(_) => Err(format!(
                "no response from the server within {} ms",
                head_timeout.as_millis()
            )),
        }
    };
    let result = tokio::select! {
        result = connecting => result,
        _ = socket.recv() => return,
    };
    let resp = match result {
        Ok(resp) => resp,
        Err(message) => {
            let _ = send_json(&mut socket, json!({"type":"error","message":message})).await;
            return;
        }
    };
    let status = resp.status();
    if send_json(
        &mut socket,
        json!({"type":"open","detail":format!("{status} — streaming events")}),
    )
    .await
    .is_err()
    {
        return;
    }

    let mut stream = resp.bytes_stream();
    let mut framer = SseFramer::default();
    loop {
        tokio::select! {
            chunk = stream.next() => match chunk {
                Some(Ok(bytes)) => {
                    if framer.pending() + bytes.len() > 1024 * 1024 {
                        let _ = send_json(&mut socket, json!({"type":"error","message":"SSE event exceeds 1 MiB"})).await;
                        return;
                    }
                    for block in framer.push(&bytes) {
                        if let Some(ev) = parse_sse_event(&block) {
                            if send_json(&mut socket, ev).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    let _ = send_json(&mut socket, json!({"type":"error","message":e.without_url().to_string()})).await;
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
    let _ = send_json(
        &mut socket,
        json!({"type":"closed","detail":"stream ended"}),
    )
    .await;
    let _ = socket.send(Message::Close(None)).await;
}

/// Splits an SSE byte stream into complete event blocks. Bytes are buffered
/// RAW and only a whole block (ending in a blank line) is decoded, so a
/// multi-byte UTF-8 character split across two network chunks (emoji / CJK in
/// LLM token streams) arrives intact instead of as two U+FFFD.
#[derive(Default)]
struct SseFramer {
    buf: Vec<u8>,
    /// `buf[..scanned]` is already CRLF-folded and holds no `\n\n` start
    /// (except possibly at its last byte) — each push folds and scans only the
    /// new bytes (perf F10: it re-folded and rescanned the whole pending
    /// buffer per chunk, O(pending²) for a large event).
    scanned: usize,
}

impl SseFramer {
    /// Bytes held for the event still in progress.
    fn pending(&self) -> usize {
        self.buf.len()
    }

    /// Append a chunk; return every block it completed (CRLF folded to LF).
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        // CRLF → LF over the new bytes only: everything before is already
        // folded, except a trailing lone CR, which may meet its LF now.
        let fold_from = if self.buf.last() == Some(&b'\r') {
            self.buf.len() - 1
        } else {
            self.buf.len()
        };
        let mut tail = self.buf.split_off(fold_from);
        tail.extend_from_slice(bytes);
        for (i, b) in tail.iter().enumerate() {
            if *b == b'\r' && tail.get(i + 1) == Some(&b'\n') {
                continue;
            }
            self.buf.push(*b);
        }
        // A `\n\n` can start one byte before the unscanned / refolded region.
        let mut from = self.scanned.min(fold_from).saturating_sub(1);
        let mut start = 0;
        let mut blocks = Vec::new();
        while let Some(rel) = self.buf[from..].windows(2).position(|w| w == b"\n\n") {
            let end = from + rel + 2;
            blocks.push(String::from_utf8_lossy(&self.buf[start..end]).into_owned());
            start = end;
            from = end;
        }
        self.buf.drain(..start);
        self.scanned = self.buf.len();
        blocks
    }
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

// ── WebSocket upstream ──────────────────────────────────────────────────────

async fn serve_websocket(
    mut socket: WebSocket,
    spec: OpenSpec,
    ctx: &ServerCtx,
    wid: &Id,
    actor: &Id,
    confirm_allowed: bool,
) {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::header::{HeaderName, HeaderValue};
    use tokio_tungstenite::tungstenite::Message as TMsg;

    let connecting = async {
        if spec.request.verify_ssl == Some(false)
            || spec.request.ssh_connection_id.is_some()
            || !spec.request.body.is_empty()
            || spec.request.method.to_uppercase() != "GET"
        {
            return Err("WebSocket supports GET headers, query and auth; TLS verification must be enabled, SSH and request bodies are unavailable".to_string());
        }
        let prepared =
            super::api_client::prepare_stream(ctx, wid, &spec.request, actor, confirm_allowed)
                .await?
                .build()
                .map_err(|e| e.without_url().to_string())?;
        let mut request = prepared
            .url()
            .as_str()
            .into_client_request()
            .map_err(|e| e.to_string())?;
        for (name, value) in prepared.headers() {
            let name =
                HeaderName::from_bytes(name.as_str().as_bytes()).map_err(|e| e.to_string())?;
            let value = HeaderValue::from_bytes(value.as_bytes()).map_err(|e| e.to_string())?;
            request.headers_mut().insert(name, value);
        }
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(1024 * 1024))
            .max_frame_size(Some(1024 * 1024));
        // SSRF guard, DNS-rebinding safe: resolve + vet the host ONCE and dial
        // exactly the vetted address (tungstenite's own connect would resolve
        // the name again, after `prepare_stream`'s pre-flight check). The
        // workspace allow-local opt-in skips the vetting, as for HTTP.
        let target = prepared.url().clone();
        let port = target.port_or_known_default().unwrap_or(80);
        let addrs: Vec<std::net::SocketAddr> =
            if super::api_client::workspace_allows_local(ctx, wid).await {
                let host = target
                    .host_str()
                    .unwrap_or("")
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string();
                let resolved: Vec<std::net::SocketAddr> =
                    tokio::net::lookup_host((host.as_str(), port))
                        .await
                        .map_err(|e| format!("dns resolution failed for {host}: {e}"))?
                        .collect();
                resolved
            } else {
                otto_netguard::resolve_checked(target.as_str()).await?.1
            };
        let socket = tokio::net::TcpStream::connect(addrs.as_slice())
            .await
            .map_err(|_| "WebSocket connection failed; check the server URL".to_string())?;
        let _ = socket.set_nodelay(true);
        tokio_tungstenite::client_async_tls_with_config(request, socket, Some(config), None).await.map_err(|_| "WebSocket handshake failed; check the server URL, TLS certificate and authorization".to_string())
    };
    let result = tokio::select! {
        result = tokio::time::timeout(super::api_client::effective_timeout(spec.request.timeout_ms), connecting) =>
            result.unwrap_or_else(|_| Err("WebSocket connection timed out".into())),
        _ = socket.recv() => return,
    };
    let (upstream, _resp) = match result {
        Ok(upstream) => upstream,
        Err(message) => {
            let _ = send_json(&mut socket, json!({"type":"error","message":message})).await;
            return;
        }
    };
    if send_json(&mut socket, json!({"type":"open","detail":"connected"}))
        .await
        .is_err()
    {
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
    let _ = send_json(
        &mut socket,
        json!({"type":"closed","detail":"disconnected"}),
    )
    .await;
    let _ = socket.send(Message::Close(None)).await;
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
    fn framer_keeps_utf8_split_across_chunks() {
        let text = "data: héllo 🌍\n\n".as_bytes();
        // Cut inside the 4-byte emoji and inside the 2-byte é.
        let cut_a = text.iter().position(|b| *b == 0xC3).unwrap() + 1;
        let cut_b = text.len() - 4;
        let mut framer = SseFramer::default();
        assert!(framer.push(&text[..cut_a]).is_empty());
        assert!(framer.push(&text[cut_a..cut_b]).is_empty());
        let blocks = framer.push(&text[cut_b..]);
        assert_eq!(blocks.len(), 1);
        let ev = parse_sse_event(&blocks[0]).expect("event");
        assert_eq!(ev["data"], "héllo 🌍");
        assert_eq!(framer.pending(), 0);
    }

    #[test]
    fn framer_folds_crlf_even_when_split() {
        let mut framer = SseFramer::default();
        assert!(framer.push(b"data: a\r").is_empty());
        assert!(framer.push(b"\n\r").is_empty());
        let blocks = framer.push(b"\ndata: b\r\n\r\n");
        assert_eq!(
            blocks,
            vec!["data: a\n\n".to_string(), "data: b\n\n".to_string()]
        );
    }

    /// perf F10: any chunking (down to one byte at a time) frames exactly
    /// like one big push, and a large pending event is scanned linearly.
    #[test]
    fn framer_is_chunking_invariant() {
        let stream = b"data: a\r\n\r\nevent: x\ndata: \xF0\x9F\x8C\x8D\n\n:c\r\n\r\ndata: tail";
        let mut whole = SseFramer::default();
        let expected = whole.push(stream);
        for size in [1usize, 2, 3, 5, 7] {
            let mut f = SseFramer::default();
            let mut got = Vec::new();
            for chunk in stream.chunks(size) {
                got.extend(f.push(chunk));
            }
            assert_eq!(got, expected, "chunk size {size}");
            assert_eq!(f.pending(), whole.pending());
        }
        let mut big = SseFramer::default();
        let line = vec![b'x'; 1024];
        for _ in 0..1024 {
            assert!(big.push(&line).is_empty());
        }
        assert_eq!(big.push(b"\n\n").len(), 1);
        assert_eq!(big.pending(), 0);
    }

    #[test]
    fn empty_block_yields_nothing() {
        assert!(parse_sse_event(":\n\n").is_none());
    }
}
