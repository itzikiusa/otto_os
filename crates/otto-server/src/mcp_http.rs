//! The outward "Otto as an MCP server" — **Streamable HTTP transport**.
//!
//! This is the piece that makes Otto's `otto.*` tools reachable **over HTTP, not
//! only locally**: an external MCP client (Claude Code, Cursor, mcp-remote, …)
//! points directly at `POST {base}/api/v1/mcp/http` with an
//! `Authorization: Bearer <mcp-token>` header
//! and speaks newline-free JSON-RPC 2.0 (one request/response per HTTP round
//! trip — the MCP "Streamable HTTP" transport). No local `ottod mcp-server`
//! subprocess is required; over the loopback listener this works for same-machine
//! clients, and over the opt-in TLS `network_listener` it works for remote ones.
//!
//! Authentication and authorization are the daemon's normal chokepoints: the
//! bearer token is resolved by `auth_middleware` into an [`AuthContext`], the
//! `feature_guard` confines a `kind='mcp'` token to exactly this route (+ the
//! legacy invoke/status routes), and the **per-token [`McpScope`]** carried on
//! that context decides which tools `tools/list` shows and `tools/call` runs —
//! so different tokens (and different users) get different accesses through the
//! one transport. Every `tools/call` funnels through
//! [`crate::mcp_outward::governed_invoke`], the same governed choke point the
//! stdio bridge uses, so enable/approval/audit are identical on both paths.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::auth::CurrentAuthContext;
use crate::mcp_outward::{governed_invoke, mcp_tools_list};
use crate::state::ServerCtx;

/// The protocol version we advertise when a client does not request one. We echo
/// the client's requested `protocolVersion` when present (maximising
/// compatibility across client versions), falling back to this otherwise.
const DEFAULT_PROTOCOL_VERSION: &str = "2025-03-26";

/// `GET /api/v1/mcp/http` — we do not offer a standalone server→client SSE
/// stream (every response rides its request's POST), so per the MCP spec we
/// answer `405 Method Not Allowed`. Returning a clean 405 (rather than a 403)
/// keeps well-behaved Streamable-HTTP clients from treating the endpoint as
/// broken when they probe for an event stream.
pub async fn mcp_http_get() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(json!({
            "jsonrpc": "2.0",
            "error": { "code": -32000, "message": "this MCP server does not offer a standalone SSE stream; POST JSON-RPC requests instead" },
            "id": Value::Null
        })),
    )
        .into_response()
}

/// `POST /api/v1/mcp/http` — handle one JSON-RPC message (or a batch). The caller
/// is already authenticated (`auth_middleware`) and confined (`feature_guard`);
/// the [`AuthContext`] carries the per-token scope used for `tools/list` and
/// `tools/call`.
pub async fn mcp_http_post(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(body): Json<Value>,
) -> Response {
    // A JSON-RPC batch is an array of messages; a single message is an object.
    if let Some(arr) = body.as_array() {
        let mut out = Vec::new();
        for msg in arr {
            if let Some(resp) = handle_one(&ctx, &auth, msg).await {
                out.push(resp);
            }
        }
        // A batch of pure notifications yields no responses → 202 Accepted.
        if out.is_empty() {
            return StatusCode::ACCEPTED.into_response();
        }
        return Json(Value::Array(out)).into_response();
    }
    match handle_one(&ctx, &auth, &body).await {
        Some(resp) => Json(resp).into_response(),
        // A notification (no `id`) gets no body — 202 Accepted per the transport.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Handle a single JSON-RPC message. Returns `Some(response)` for a request and
/// `None` for a notification (no `id`).
async fn handle_one(ctx: &ServerCtx, auth: &otto_core::auth::AuthContext, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = msg.get("id").cloned();
    let is_notification = id.is_none();
    match method {
        "initialize" => {
            let requested = msg
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL_VERSION)
                .to_string();
            Some(rpc_ok(
                id,
                json!({
                    "protocolVersion": requested,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "otto", "version": env!("CARGO_PKG_VERSION") }
                }),
            ))
        }
        "notifications/initialized" | "initialized" => None,
        "ping" => Some(rpc_ok(id, json!({}))),
        "tools/list" => {
            let tools = mcp_tools_list(ctx, auth.mcp_scope.as_ref()).await;
            Some(rpc_ok(id, json!({ "tools": tools })))
        }
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // Route through the SAME governed choke point as the stdio bridge:
            // per-token scope → global enable → approval → execute → audit.
            let result = match governed_invoke(ctx, auth, &name, &args, false, None).await {
                Ok(envelope) => {
                    let is_error = envelope.get("decision").and_then(Value::as_str)
                        == Some("denied")
                        || envelope.get("decision").and_then(Value::as_str) == Some("error")
                        || envelope.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    tool_result(&envelope, is_error)
                }
                Err(e) => tool_result(&json!({ "error": e.0.to_string() }), true),
            };
            Some(rpc_ok(id, result))
        }
        _ if is_notification => None,
        _ => Some(rpc_err(id, -32601, format!("method not found: {method}"))),
    }
}

/// Wrap a governed envelope as an MCP tool result (text content + isError).
fn tool_result(value: &Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    json!({ "content": [ { "type": "text", "text": text } ], "isError": is_error })
}

fn rpc_ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn rpc_err(id: Option<Value>, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": { "code": code, "message": message.into() } })
}
