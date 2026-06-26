//! Outbound MCP client — Otto connecting *out* to a registered MCP server.
//!
//! Two transports:
//! - **stdio**: spawn `command args` with `env` (incl. keychain-resolved secrets),
//!   speak newline-delimited JSON-RPC 2.0 over the child's stdin/stdout. One
//!   short-lived child per operation (discovery / health / invoke) keeps the
//!   client stateless and robust; results are cached in `mcp_tools`.
//! - **http** (Streamable HTTP): one operation = `POST initialize` (capture
//!   `Mcp-Session-Id`) → `POST <op>` (carry the session id). The reqwest client is
//!   built with the **SSRF-validated IP pinned** (`.resolve`) so a DNS rebind can't
//!   redirect the connect — and the configured auth header — to loopback/metadata
//!   (design §14 F9). `otto_netguard::redirect_policy()` guards redirect hops.
//!
//! Hard caps mirror the inward server: 20 s per op, 1 MiB body. Redaction/row-cap
//! of results happens in the service layer.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const PROTOCOL_VERSION: &str = "2024-11-05";
const OP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Result of a `tools/call`.
pub struct CallResult {
    pub content: Value, // the raw MCP `result` object ({content, isError})
    pub is_error: bool,
    pub bytes: usize,
}

/// Transport configuration for one server.
pub enum Transport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

pub struct McpClient {
    transport: Transport,
}

impl McpClient {
    pub fn new(transport: Transport) -> Self {
        Self { transport }
    }

    /// `tools/list` → the advertised tool objects.
    pub async fn list_tools(&self) -> Result<Vec<Value>, String> {
        let result = self
            .op(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}), 2)
            .await?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(tools)
    }

    /// `tools/call` for a named tool.
    pub async fn call_tool(&self, name: &str, args: &Value) -> Result<CallResult, String> {
        let req = json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params": {"name": name, "arguments": args}
        });
        let result = self.op(req, 3).await?;
        let bytes = serde_json::to_vec(&result).map(|v| v.len()).unwrap_or(0);
        let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
        Ok(CallResult { content: result, is_error, bytes })
    }

    /// Health probe = an `initialize` round-trip (no extra op).
    pub async fn health(&self) -> Result<(), String> {
        match &self.transport {
            Transport::Stdio { .. } => {
                self.stdio_op(None).await.map(|_| ())
            }
            Transport::Http { .. } => self.http_op(None).await.map(|_| ()),
        }
    }

    /// Run one post-initialize op (request with `id`), returning its `result`.
    async fn op(&self, request: Value, id: i64) -> Result<Value, String> {
        match &self.transport {
            Transport::Stdio { .. } => self.stdio_op(Some((request, id))).await,
            Transport::Http { .. } => self.http_op(Some((request, id))).await,
        }
    }

    // ---- stdio ------------------------------------------------------------

    async fn stdio_op(&self, op: Option<(Value, i64)>) -> Result<Value, String> {
        let Transport::Stdio { command, args, env } = &self.transport else {
            return Err("not a stdio transport".into());
        };
        let fut = async {
            let mut child = Command::new(command)
                .args(args)
                .envs(env)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("spawn '{command}': {e}"))?;

            let mut stdin = child.stdin.take().ok_or("no child stdin")?;
            let stdout = child.stdout.take().ok_or("no child stdout")?;
            let mut reader = BufReader::new(stdout);

            // 1. initialize, wait for its response.
            let init = json!({
                "jsonrpc":"2.0","id":1,"method":"initialize",
                "params":{"protocolVersion":PROTOCOL_VERSION,"capabilities":{},
                          "clientInfo":{"name":"otto-control-plane","version":"0.1.0"}}
            });
            write_line(&mut stdin, &init).await?;
            let _ = read_until_id(&mut reader, 1).await?;

            // 2. initialized notification.
            write_line(&mut stdin, &json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await?;

            // 3. the op (if any), wait for its response.
            let result = match op {
                Some((req, id)) => {
                    write_line(&mut stdin, &req).await?;
                    read_until_id(&mut reader, id).await?
                }
                None => json!({}), // health: initialize was enough
            };

            // Close stdin so the child exits; reap it.
            drop(stdin);
            let _ = child.start_kill();
            let _ = child.wait().await;
            Ok::<Value, String>(result)
        };
        tokio::time::timeout(OP_TIMEOUT, fut)
            .await
            .map_err(|_| format!("stdio op timed out after {}s", OP_TIMEOUT.as_secs()))?
    }

    // ---- http (Streamable HTTP) ------------------------------------------

    async fn http_op(&self, op: Option<(Value, i64)>) -> Result<Value, String> {
        let Transport::Http { url, headers } = &self.transport else {
            return Err("not an http transport".into());
        };
        // SSRF: validate, then PIN the vetted IP so the actual connect can't be
        // DNS-rebound to an internal/metadata address (which would also exfiltrate
        // the auth header). Reject non-public resolutions.
        otto_netguard::check_url(url).await?;
        let parsed = reqwest::Url::parse(url).map_err(|e| format!("bad url: {e}"))?;
        let host = parsed.host_str().ok_or("url has no host")?.to_string();
        let port = parsed
            .port_or_known_default()
            .ok_or("url has no port")?;
        let addrs = tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| format!("dns: {e}"))?;
        let addr = addrs
            .into_iter()
            .find(|a| !otto_netguard::is_blocked_ip(a.ip()))
            .ok_or("host resolves only to blocked addresses")?;

        let mut builder = reqwest::Client::builder()
            .timeout(OP_TIMEOUT)
            .redirect(otto_netguard::redirect_policy())
            .resolve(&host, addr);
        let _ = &mut builder;
        let client = builder.build().map_err(|e| format!("http client: {e}"))?;

        let send = |body: Value, session: Option<String>| {
            let mut rb = client
                .post(url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream");
            for (k, v) in headers {
                rb = rb.header(k.as_str(), v.as_str());
            }
            if let Some(s) = session {
                rb = rb.header("Mcp-Session-Id", s);
            }
            rb.json(&body).send()
        };

        // 1. initialize.
        let init = json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":PROTOCOL_VERSION,"capabilities":{},
                      "clientInfo":{"name":"otto-control-plane","version":"0.1.0"}}
        });
        let resp = send(init, None).await.map_err(|e| format!("initialize: {e}"))?;
        let session_id = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let _ = parse_http_message(resp).await?; // ensure initialize succeeded

        let result = match op {
            None => json!({}),
            Some((mut req, _id)) => {
                // notifications/initialized first (best-effort).
                let _ = send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}), session_id.clone())
                    .await;
                req["id"] = json!(2);
                let resp = send(req, session_id).await.map_err(|e| format!("op: {e}"))?;
                let msg = parse_http_message(resp).await?;
                msg.get("result").cloned().ok_or_else(|| {
                    msg.get("error")
                        .map(|e| format!("server error: {e}"))
                        .unwrap_or_else(|| "no result in response".into())
                })?
            }
        };
        Ok(result)
    }
}

/// Read the JSON-RPC message from an HTTP response, honoring content negotiation
/// (a JSON body, or an SSE `text/event-stream` whose `data:` lines carry it).
/// Body is size-capped.
async fn parse_http_message(resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(format!("response too large ({} bytes)", bytes.len()));
    }
    let text = String::from_utf8_lossy(&bytes);
    if !status.is_success() {
        let snippet: String = text.chars().take(300).collect();
        return Err(format!("http {status}: {snippet}"));
    }
    if ct.contains("text/event-stream") {
        // Concatenate `data:` lines and parse the last complete JSON object.
        let mut data = String::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim());
            }
        }
        if data.is_empty() {
            return Err("empty SSE stream".into());
        }
        serde_json::from_str(&data).map_err(|e| format!("parse sse json: {e}"))
    } else {
        serde_json::from_str(&text).map_err(|e| format!("parse json: {e}"))
    }
}

async fn write_line<W: AsyncWriteExt + Unpin>(w: &mut W, v: &Value) -> Result<(), String> {
    let mut buf = serde_json::to_vec(v).map_err(|e| format!("encode: {e}"))?;
    buf.push(b'\n');
    w.write_all(&buf).await.map_err(|e| format!("write: {e}"))?;
    w.flush().await.map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

/// Read newline-delimited JSON-RPC lines until one carries `id == want_id`, return
/// its `result` (or surface its `error`). Notifications / other ids are skipped.
async fn read_until_id<R: AsyncBufReadExt + Unpin>(
    reader: &mut R,
    want_id: i64,
) -> Result<Value, String> {
    let mut line = String::new();
    let mut total = 0usize;
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await.map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return Err("server closed before responding".into());
        }
        total += n;
        if total > MAX_BODY_BYTES {
            return Err("response exceeded size cap".into());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue, // tolerate stray non-JSON noise on stdout
        };
        let id_matches = msg.get("id").and_then(Value::as_i64) == Some(want_id);
        if !id_matches {
            continue;
        }
        if let Some(err) = msg.get("error") {
            return Err(format!("server error: {err}"));
        }
        return Ok(msg.get("result").cloned().unwrap_or(json!({})));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Drives the stdio client against a tiny shell MCP server so the framing +
    // initialize→list/call sequence is exercised end to end (no external deps).
    fn echo_server_script() -> String {
        // A minimal MCP stdio server in awk-free pure sh: respond to initialize,
        // tools/list, tools/call by id. Reads line-by-line.
        r#"
while IFS= read -r line; do
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"1"}}}\n' ;;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"echo","inputSchema":{"type":"object"}}]}}\n' ;;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"ok"}],"isError":false}}\n' ;;
    *'"notifications/initialized"'*) : ;;
  esac
done
"#
        .to_string()
    }

    fn stdio_client() -> McpClient {
        let mut env = BTreeMap::new();
        env.insert("LC_ALL".into(), "C".into());
        McpClient::new(Transport::Stdio {
            command: "sh".into(),
            args: vec!["-c".into(), echo_server_script()],
            env,
        })
    }

    #[tokio::test]
    async fn stdio_health_initializes() {
        assert!(stdio_client().health().await.is_ok());
    }

    #[tokio::test]
    async fn stdio_list_tools() {
        let tools = stdio_client().list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], json!("echo"));
    }

    #[tokio::test]
    async fn stdio_call_tool() {
        let r = stdio_client().call_tool("echo", &json!({"x":1})).await.unwrap();
        assert!(!r.is_error);
        assert_eq!(r.content["content"][0]["text"], json!("ok"));
    }
}
