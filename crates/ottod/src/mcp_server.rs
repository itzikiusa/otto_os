//! `ottod mcp-server` — the OUTWARD "Otto as an MCP server".
//!
//! An external agent launches this over stdio with an `OTTO_API_TOKEN` that is a
//! restricted `kind='mcp'` token (minted in the MCP Control Plane UI). It speaks
//! newline-delimited JSON-RPC 2.0 and exposes the eight `otto.*` tools. Every
//! `tools/call` is forwarded to `POST /api/v1/mcp/otto-tools/invoke`, which is the
//! ONLY route the restricted token may reach — the control plane governs (enabled?
//! allowlisted? dangerous→approval?), audits, and executes each call. `tools/list`
//! reflects the currently-enabled tool set from `GET /api/v1/mcp/otto-server`.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::config::Config;

const PROTOCOL_VERSION: &str = "2024-11-05";
const CALL_TIMEOUT: Duration = Duration::from_secs(35);

struct Ctx {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Ctx {
    async fn get_json(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!("daemon returned {status}: {}", body.chars().take(300).collect::<String>()));
        }
        serde_json::from_str(&body).map_err(|e| format!("parse json: {e}"))
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!("daemon returned {status}: {}", text.chars().take(300).collect::<String>()));
        }
        serde_json::from_str(&text).map_err(|e| format!("parse json: {e}"))
    }

    /// The MCP `tools/list`: the eight specs filtered to the enabled set.
    async fn tools_list(&self) -> Value {
        // The set of currently-enabled tools (best-effort; on error list all).
        let enabled: Option<Vec<String>> = self
            .get_json("/mcp/otto-server")
            .await
            .ok()
            .and_then(|v| {
                v.get("tools").and_then(Value::as_array).map(|tools| {
                    tools
                        .iter()
                        .filter(|t| t.get("enabled").and_then(Value::as_bool).unwrap_or(false))
                        .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
            });
        let specs = otto_server::mcp_outward::otto_tool_specs();
        let tools: Vec<Value> = specs
            .into_iter()
            .filter(|s| {
                let name = s.get("name").and_then(Value::as_str).unwrap_or("");
                enabled.as_ref().map(|e| e.iter().any(|n| n == name)).unwrap_or(true)
            })
            .map(|s| {
                json!({
                    "name": s["name"],
                    "description": s["description"],
                    "inputSchema": s["inputSchema"],
                })
            })
            .collect();
        json!({ "tools": tools })
    }

    /// The MCP `tools/call`: forward to the governed choke point.
    async fn tools_call(&self, name: &str, args: &Value) -> Value {
        let res = self
            .post_json(
                "/mcp/otto-tools/invoke",
                &json!({ "tool": name, "arguments": args }),
            )
            .await;
        match res {
            Ok(v) => {
                let executed = v.get("executed").and_then(Value::as_bool).unwrap_or(false);
                let decision = v.get("decision").and_then(Value::as_str).unwrap_or("");
                let is_error = decision == "denied" || decision == "error" || v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                // Surface the whole governed envelope to the agent so it can see a
                // denial reason / pending-approval id / the tool content.
                let _ = executed;
                tool_result(&v, is_error)
            }
            Err(e) => tool_result(&json!({ "error": e }), true),
        }
    }
}

fn tool_result(value: &Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    json!({ "content": [ { "type": "text", "text": text } ], "isError": is_error })
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}
fn rpc_err(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

async fn handle(ctx: &Ctx, msg: Value) -> Option<Value> {
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    let is_notification = id.is_none();
    match method {
        "initialize" => Some(rpc_ok(
            id.unwrap_or(Value::Null),
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "otto", "version": env!("CARGO_PKG_VERSION") }
            }),
        )),
        "notifications/initialized" | "initialized" => None,
        "ping" => Some(rpc_ok(id.unwrap_or(Value::Null), json!({}))),
        "tools/list" => Some(rpc_ok(id.unwrap_or(Value::Null), ctx.tools_list().await)),
        "tools/call" => {
            let id = id.unwrap_or(Value::Null);
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Some(rpc_ok(id, ctx.tools_call(&name, &args).await))
        }
        _ if is_notification => None,
        _ => Some(rpc_err(id.unwrap_or(Value::Null), -32601, format!("method not found: {method}"))),
    }
}

pub async fn run() -> Result<(), String> {
    let token = std::env::var("OTTO_API_TOKEN").unwrap_or_default();
    if token.is_empty() {
        return Err("OTTO_API_TOKEN not set (mint a restricted MCP token in the MCP Control Plane → Otto Server tab)".into());
    }
    let base = std::env::var("OTTO_MCP_BASE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", Config::load().port));

    let http = reqwest::Client::builder()
        .timeout(CALL_TIMEOUT)
        .build()
        .map_err(|e| format!("build http client: {e}"))?;
    let ctx = Ctx { http, base, token };

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await.map_err(|e| format!("read stdin: {e}"))?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                write_line(&mut stdout, &rpc_err(Value::Null, -32700, format!("parse error: {e}"))).await?;
                continue;
            }
        };
        if let Some(resp) = handle(&ctx, msg).await {
            write_line(&mut stdout, &resp).await?;
        }
    }
    Ok(())
}

async fn write_line(stdout: &mut tokio::io::Stdout, value: &Value) -> Result<(), String> {
    let mut buf = serde_json::to_vec(value).map_err(|e| format!("encode: {e}"))?;
    buf.push(b'\n');
    stdout.write_all(&buf).await.map_err(|e| format!("write: {e}"))?;
    stdout.flush().await.map_err(|e| format!("flush: {e}"))?;
    Ok(())
}
